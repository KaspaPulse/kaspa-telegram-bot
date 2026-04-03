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

pub async fn start_kaspa_engine(state: Arc<AppState>, bot: Bot, api: Arc<ApiManager>) {
    let ws_url = std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
    let (tx_event, mut rx_event) = mpsc::channel::<UtxoEvent>(5000);
    let api_semaphore = Arc::new(Semaphore::new(10));

    let worker_state = state.clone();
    let worker_bot = bot.clone();
    let worker_api = api.clone();

    // Background processing worker
    tokio::spawn(async move {
        while let Some(event) = rx_event.recv().await {
            let sem = api_semaphore.clone();
            let s = worker_state.clone();
            let b = worker_bot.clone();
            let a = worker_api.clone();
            tokio::spawn(async move {
                let _permit = sem.acquire().await.ok();
                process_reward_event(event, s, b, a).await;
            });
        }
    });

    loop {
        if state.shutdown_token.is_cancelled() { break; }
        tracing::info!("[NODE] Connecting to {}...", ws_url);

        let mut request = ws_url.clone().into_client_request().unwrap();
        request.headers_mut().insert("sec-websocket-protocol", "wrpc-json".parse().unwrap());

        match connect_async(request).await {
            Ok((mut ws_stream, _)) => {
                tracing::info!("✅ [NODE] Connected!");
                
                // Subscription Logic - Run ONCE per connection
                let addrs: Vec<String> = state.monitored_wallets.iter().map(|kv| kv.key().clone()).collect();
                if !addrs.is_empty() {
                    let sub_req = json!({
                        "jsonrpc": "1.0",
                        "id": 1,
                        "method": "notifyUtxosChangedRequest",
                        "params": { "addresses": addrs }
                    });
                    let _ = ws_stream.send(Message::Text(sub_req.to_string())).await;
                    tracing::info!("🔄 [NODE] Subscribed to {} wallets.", addrs.len());
                }

                // Listen Loop
                while let Some(msg) = ws_stream.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                                if let Some(method) = parsed.get("method").and_then(|m| m.as_str()) {
                                    if method == "utxosChangedNotification" {
                                        if let Some(params) = parsed.get("params") {
                                            parse_utxos_and_queue(params, &state, &tx_event).await;
                                        }
                                    }
                                }
                            }
                        }
                        _ => break, // Break to reconnect on error/close
                    }
                }
            }
            Err(_) => tokio::time::sleep(Duration::from_secs(5)).await,
        }
        tracing::warn!("⚠️ [NODE] Connection lost. Reconnecting in 5s...");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn parse_utxos_and_queue(payload: &Value, state: &Arc<AppState>, tx: &mpsc::Sender<UtxoEvent>) {
    if let Some(added) = payload.get("added").and_then(|v| v.as_array()) {
        for entry in added {
            let tx_id = entry.get("outpoint").and_then(|o| o.get("transactionId")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let address = entry.get("address").and_then(|a| a.as_str()).map(|s| s.to_string());
            
            if let (Some(id), Some(addr)) = (tx_id, address) {
                if state.monitored_wallets.contains_key(&addr) {
                    let amount = entry.get("utxoEntry").and_then(|u| u.get("amount")).and_then(|v| v.as_u64()).map(Sompi).unwrap_or(Sompi(0));
                    let daa = entry.get("utxoEntry").and_then(|u| u.get("blockDaaScore")).and_then(|v| v.as_u64()).unwrap_or(0);
                    let _ = tx.try_send(UtxoEvent { tx_id: id, address: addr, amount, daa_score: daa });
                }
            }
        }
    }
}

async fn process_reward_event(event: UtxoEvent, state: Arc<AppState>, bot: Bot, api: Arc<ApiManager>) {
    let kas: Kaspa = event.amount.into();
    let bal = api.get_balance(&event.address).await.unwrap_or(0.0);
    
    let msg = format!("⚡ <b>Native Node Reward!</b> 💎\n━━━━━━━━━━━━━━━━━━\n<b>Time:</b> <code>{} UTC</code>\n<b>Wallet:</b> <code>{}</code>\n<b>Amount:</b> <code>+{} KAS</code>\n<b>Balance:</b> <code>{} KAS</code>\n━━━━━━━━━━━━━━━━━━\n<b>TXID:</b> <a href=\"https://kaspa.stream/transactions/{}\">View Transaction</a>\n<b>DAA Score:</b> <code>{}</code>", 
        Utc::now().format("%Y-%m-%d %H:%M:%S"), format_short_wallet(&event.address), kas.0, bal, event.tx_id, event.daa_score);

    if let Some(wallet) = state.monitored_wallets.get(&event.address) {
        for &chat_id in &wallet.chat_ids {
            let _ = bot.send_message(ChatId(chat_id), &msg)
                .parse_mode(ParseMode::Html)
                .disable_web_page_preview(true)
                .await;
        }
    }
}
