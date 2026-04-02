#![allow(deprecated, unused_variables)]
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use crate::state::{AppState, PendingAlert};
use crate::utils::helpers::format_short_wallet;

#[allow(dead_code)]
async fn fetch_local_block(hash: &str, ws_url: &str) -> Option<String> {
    if let Ok(mut request) = ws_url.into_client_request() {
        request
            .headers_mut()
            .insert("sec-websocket-protocol", "wrpc-json".parse().unwrap());
        if let Ok((mut ws_stream, _)) = connect_async(request).await {
            let req = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getBlock",
                "params": { "hash": hash, "includeTransactions": false }
            });
            if ws_stream.send(Message::Text(req.to_string())).await.is_ok() {
                if let Some(Ok(Message::Text(res))) = ws_stream.next().await {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&res) {
                        if let Some(result) = parsed.get("result") {
                            if let Some(blues) = result
                                .get("block")
                                .and_then(|b| b.get("verboseData"))
                                .and_then(|v| v.get("mergeSetBluesHashes"))
                                .and_then(|h| h.as_array())
                            {
                                if !blues.is_empty() {
                                    return blues[0].as_str().map(|s| s.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

pub async fn start_kaspa_engine(state: Arc<AppState>, bot: Bot) {
    let ws_url = std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
    let http_client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    loop {
        if !state
            .is_monitoring
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        log::info!("[NODE] Connecting to Node02 wRPC at {}...", ws_url);

        let mut request = match ws_url.clone().into_client_request() {
            Ok(req) => req,
            Err(e) => {
                log::error!("❌ [NODE] Invalid URL: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        request
            .headers_mut()
            .insert("sec-websocket-protocol", "wrpc-json".parse().unwrap());

        match connect_async(request).await {
            Ok((mut ws_stream, _)) => {
                log::info!("✅ [NODE] Connected! Handshaking...");
                let addresses: Vec<String> = state
                    .monitored_wallets
                    .iter()
                    .map(|kv| kv.key().clone())
                    .collect();
                if !addresses.is_empty() {
                    let sub_req = json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "notifyUtxosChanged",
                        "params": { "addresses": addresses }
                    });
                    let _ = ws_stream.send(Message::Text(sub_req.to_string())).await;
                }

                while let Some(msg) = ws_stream.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                                if let Some(method) = parsed.get("method").and_then(|m| m.as_str())
                                {
                                    if method == "utxosChangedNotification" {
                                        if let Some(params) = parsed.get("params") {
                                            handle_utxos_changed(
                                                params,
                                                state.clone(),
                                                bot.clone(),
                                                http_client.clone(),
                                                ws_url.clone(),
                                            )
                                            .await;
                                        }
                                    }
                                } else if let Some(result) = parsed.get("result") {
                                    log::info!(
                                        "📥 [NODE ACK] Subscription Successful: {:?}",
                                        result
                                    );
                                }
                            }
                        }
                        Ok(Message::Close(c)) => {
                            log::warn!("⚠️ [NODE] Closed: {:?}", c);
                            break;
                        }
                        Err(e) => {
                            log::error!("❌ [NODE] WS Error: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                log::error!("❌ [NODE FATAL] Fail: {}", e);
            }
        }
        log::warn!("🔄 Reconnecting in 5s...");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn handle_utxos_changed(
    payload: &Value,
    state: Arc<AppState>,
    bot: Bot,
    http_client: Client,
    ws_url: String,
) {
    if let Some(added) = payload.get("added").and_then(|v| v.as_array()) {
        for entry in added {
            let tx_id = match extract_tx_id(entry) {
                Some(id) => id,
                None => continue,
            };
            let address = match extract_address(entry) {
                Some(addr) => addr,
                None => continue,
            };
            if !state.monitored_wallets.contains_key(&address) {
                continue;
            }
            let amount_sompi = match extract_amount(entry) {
                Some(amt) => amt,
                None => continue,
            };
            let daa_score = extract_daa_score(entry).unwrap_or(0);
            if state.processed_txids.contains(&tx_id) {
                continue;
            }
            state
                .pending_alerts
                .insert(tx_id.clone(), PendingAlert { daa_score });
            let state_clone = state.clone();
            let bot_clone = bot.clone();
            let http_clone = http_client.clone();
            let tx_id_clone = tx_id.clone();
            let address_clone = address.clone();
            let ws_clone = ws_url.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(20)).await;
                state_clone.processed_txids.insert(tx_id_clone.clone());
                let final_daa = match state_clone.pending_alerts.remove(&tx_id_clone) {
                    Some(alert) => alert.1.daa_score,
                    None => return,
                };
                let exact_reward = amount_sompi / 100_000_000.0;
                let mut live_bal = 0.0;
                if let Some(mut wallet) = state_clone.monitored_wallets.get_mut(&address_clone) {
                    live_bal = wallet.last_balance + exact_reward;
                    wallet.last_balance = live_bal;
                }
                state_clone.save_wallets();
                let dt_str = format!("{} UTC", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                let short_wallet = format_short_wallet(&address_clone);
                let build_msg = |b: &str, a: &str, m: &str| -> String {
                    format!("⚡ *Native Node Reward!* 💎\n━━━━━━━━━━━━━━━━━━\n*Time:* `{}`\n*Wallet:* [{}]({})\n*Amount:* `+{:.8} KAS`\n*Live Balance:* `{:.8} KAS`\n━━━━━━━━━━━━━━━━━━\n*Mined Block:* {}\n*Accepting Block:* {}\n*DAA Score:* `{}`", dt_str, short_wallet, format!("https://kaspa.stream/addresses/{}", address_clone), exact_reward, live_bal, m, a, final_daa)
                };
                let subscribers = match state_clone.monitored_wallets.get(&address_clone) {
                    Some(w) => w.chat_ids.clone(),
                    None => return,
                };
                for chat_id in subscribers {
                    let _ = bot_clone
                        .send_message(ChatId(chat_id), build_msg("...", "...", "..."))
                        .parse_mode(ParseMode::Markdown)
                        .await;
                }
            });
        }
    }
}

fn extract_tx_id(entry: &Value) -> Option<String> {
    entry
        .get("outpoint")
        .and_then(|o| o.get("transactionId"))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}
fn extract_address(entry: &Value) -> Option<String> {
    entry
        .get("address")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}
fn extract_amount(entry: &Value) -> Option<f64> {
    entry
        .get("utxoEntry")
        .and_then(|u| u.get("amount"))
        .and_then(|v| v.as_f64())
}
fn extract_daa_score(entry: &Value) -> Option<u64> {
    entry
        .get("utxoEntry")
        .and_then(|u| u.get("blockDaaScore"))
        .and_then(|v| v.as_u64())
}
