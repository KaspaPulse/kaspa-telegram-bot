#![allow(deprecated)]
use std::sync::Arc;
use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use serde_json::{json, Value};
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::{mpsc, Semaphore};
use std::time::Duration;
use chrono::Utc;

use crate::state::AppState;
use crate::utils::helpers::format_short_wallet;
use crate::api::ApiManager;
use crate::utils::types::{Sompi, Kaspa};

#[derive(Debug, Clone)]
pub struct UtxoEvent {
    pub tx_id: String,
    pub address: String,
    pub amount: Sompi,
    pub daa_score: u64,
}

#[tracing::instrument(skip(state, bot, api))]
pub async fn start_kaspa_engine(state: Arc<AppState>, bot: Bot, api: Arc<ApiManager>) {
    let ws_url = std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
    let (tx_event, mut rx_event) = mpsc::channel::<UtxoEvent>(5000);
    let api_semaphore = Arc::new(Semaphore::new(10));

    let worker_state = state.clone();
    let worker_bot = bot.clone();
    let worker_api = api.clone();
    let worker_shutdown = state.shutdown_token.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = worker_shutdown.cancelled() => { break; }
                event_opt = rx_event.recv() => {
                    if let Some(event) = event_opt {
                        let sem_clone = api_semaphore.clone();
                        let s_clone = worker_state.clone();
                        let b_clone = worker_bot.clone();
                        let a_clone = worker_api.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(20)).await;
                            let _permit = match sem_clone.acquire().await { Ok(p) => p, Err(_) => return };
                            process_reward_event(event, s_clone, b_clone, a_clone).await;
                        });
                    }
                }
            }
        }
    });

    loop {
        if state.shutdown_token.is_cancelled() { return; }
        tracing::info!("[NODE] Connecting to wRPC at {}...", ws_url);

        let mut request = match ws_url.clone().into_client_request() {
            Ok(req) => req,
            Err(e) => { tracing::error!("❌ [NODE] Invalid URL: {}", e); tokio::time::sleep(Duration::from_secs(5)).await; continue; }
        };
        if let Ok(header_val) = "wrpc-json".parse() { request.headers_mut().insert("sec-websocket-protocol", header_val); }

        match connect_async(request).await {
            Ok((mut ws_stream, _)) => {
                tracing::info!("✅ [NODE] Connected!");
                
                let mut last_wallet_count = 0;
                let mut sub_interval = tokio::time::interval(Duration::from_secs(10));

                loop {
                    tokio::select! {
                        _ = sub_interval.tick() => {
                            let current_count = state.monitored_wallets.len();
                            if current_count != last_wallet_count {
                                last_wallet_count = current_count;
                                let addrs: Vec<String> = state.monitored_wallets.iter().map(|kv| kv.key().clone()).collect();
                                let sub_req = json!({"jsonrpc":"2.0","id":2,"method":"notifyUtxosChanged","params":{"addresses":addrs}});
                                let _ = ws_stream.send(Message::Text(sub_req.to_string())).await;
                                tracing::info!("🔄 [NODE] Subscription Sync: {} wallets", current_count);
                            }
                        }
                        _ = state.shutdown_token.cancelled() => {
                            let _ = ws_stream.close(None).await;
                            return;
                        }
                        msg = ws_stream.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                                        if let Some(method) = parsed.get("method").and_then(|m| m.as_str()) {
                                            if method == "utxosChangedNotification" {
                                                parse_utxos_and_queue(parsed.get("params").unwrap_or(&json!({})), &state, &tx_event).await;
                                            }
                                        }
                                    }
                                }
                                Some(Ok(Message::Close(_))) | None => break,
                                _ => {}
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::error!("❌ [NODE ERROR]: {}", e),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn parse_utxos_and_queue(payload: &Value, state: &Arc<AppState>, tx: &mpsc::Sender<UtxoEvent>) {
    if let Some(added) = payload.get("added").and_then(|v| v.as_array()) {
        for entry in added {
            let tx_id = match extract_tx_id(entry) { Some(id) => id, None => continue };
            let address = match extract_address(entry) { Some(addr) => addr, None => continue };
            
            if !state.monitored_wallets.contains_key(&address) { continue; }
            
            let amount = match extract_amount(entry) { Some(amt) => amt, None => {
                tracing::warn!("⚠️ [NODE] Received notification for {} but could not parse amount.", address);
                continue;
            }};
            
            let daa_score = extract_daa_score(entry).unwrap_or(0);
            
            if state.processed_txids.contains_key(&tx_id) { continue; }
            state.processed_txids.insert(tx_id.clone(), std::time::Instant::now());

            tracing::info!("💎 [EVENT] Valid Reward Detected: {} KAS for {}", Kaspa::from(amount), address);
            let _ = tx.try_send(UtxoEvent { tx_id, address, amount, daa_score });
        }
    }
}

async fn process_reward_event(event: UtxoEvent, state: Arc<AppState>, bot: Bot, api: Arc<ApiManager>) {
    let exact_reward: Kaspa = event.amount.into();
    let mut live_bal = match api.get_balance(&event.address).await {
        Ok(bal) if bal > 0.0 => Kaspa(bal),
        _ => Kaspa(0.0),
    };

    if let Some(mut wallet) = state.monitored_wallets.get_mut(&event.address) {
        if live_bal.0 == 0.0 { live_bal = Kaspa(wallet.last_balance + exact_reward.0); }
        wallet.last_balance = live_bal.0;
    }
    state.sync_wallet_to_db(&event.address).await;

    let dt_str = format!("{} UTC", Utc::now().format("%Y-%m-%d %H:%M:%S"));
    let short_wallet = format_short_wallet(&event.address);
    let final_text = format!("⚡ <b>Native Node Reward!</b> 💎\n━━━━━━━━━━━━━━━━━━\n<b>Time:</b> <code>{}</code>\n<b>Wallet:</b> <a href=\"https://kaspa.stream/addresses/{}\">{}</a>\n<b>Amount:</b> <code>+{} KAS</code>\n<b>Live Balance:</b> <code>{} KAS</code>\n━━━━━━━━━━━━━━━━━━\n<b>DAA Score:</b> <code>{}</code>", 
        dt_str, event.address, short_wallet, exact_reward, live_bal, event.daa_score);

    tracing::info!("[BOT OUT] Chat ID: Reward Notification for {}", event.address);
    let subscribers = match state.monitored_wallets.get(&event.address) { Some(w) => w.chat_ids.clone(), None => return };
    for chat_id in subscribers {
        let _ = bot.send_message(ChatId(chat_id), &final_text).parse_mode(ParseMode::Html).disable_web_page_preview(true).await;
    }
}

// Greedily extract fields (Supports String or Number, underscore or camelCase)
fn extract_tx_id(entry: &Value) -> Option<String> {
    let outpoint = entry.get("outpoint")?;
    outpoint.get("transactionId").or(outpoint.get("transaction_id")).and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn extract_address(entry: &Value) -> Option<String> {
    entry.get("address").and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn extract_amount(entry: &Value) -> Option<Sompi> {
    let utxo = entry.get("utxoEntry")?;
    let val = utxo.get("amount").or(utxo.get("amount_sompi"))?;
    if let Some(n) = val.as_u64() { Some(Sompi(n)) }
    else if let Some(s) = val.as_str() { s.parse::<u64>().ok().map(Sompi) }
    else { None }
}

fn extract_daa_score(entry: &Value) -> Option<u64> {
    let val = entry.get("utxoEntry")?.get("blockDaaScore")?;
    if let Some(n) = val.as_u64() { Some(n) }
    else if let Some(s) = val.as_str() { s.parse::<u64>().ok() }
    else { None }
}
