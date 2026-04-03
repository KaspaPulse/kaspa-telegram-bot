use futures_util::StreamExt;
use futures_util::SinkExt;
use std::sync::Arc;
use crate::state::AppState;
use tokio::time::{sleep, Duration};

pub async fn start_kaspa_listener(state: Arc<AppState>, bot: teloxide::Bot) {
    loop {
        sleep(Duration::from_secs(2)).await;

        if !state.is_monitoring.load(std::sync::atomic::Ordering::SeqCst) {
            sleep(Duration::from_secs(5)).await;
            continue;
        }

        let ws_url = std::env::var("NODE_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:16110".to_string());
        log::info!("[NODE] Connecting to WebSocket: {}", ws_url);

        match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((mut ws_stream, _)) => {
                log::info!("[NODE] Connected successfully. Initializing Subscriptions...");

                let wallets: Vec<String> = state.monitored_wallets.iter().map(|e| e.key().clone()).collect();
                if !wallets.is_empty() {
                    let sub_request = serde_json::json!({
                        "notifyUtxosChangedRequest": { "addresses": wallets }
                    });
                    let _ = ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(sub_request.to_string())).await;
                    log::info!("[NODE] Subscribed to {} wallets.", wallets.len());
                }

                while let Some(msg) = ws_stream.next().await {
                    if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(ntf) = json.get("utxosChangedNotification") {
                                let added = ntf.get("added").and_then(|a| a.as_array());
                                if let Some(utxos) = added {
                                    for utxo in utxos {
                                        let address = utxo.get("address").and_then(|a| a.as_str()).unwrap_or("");
                                        let amount = utxo.get("utxoEntry").and_then(|e| e.get("amount")).and_then(|a| a.as_str()).unwrap_or("0");
                                        
                                        if state.monitored_wallets.contains_key(address) {
                                            log::info!("[NOTIFICATION] Match Found! Address: {}, Amount: {}", address, amount);
                                            let state_clone = state.clone();
                                            let addr_clone = address.to_string();
                                            let amt_clone = amount.to_string();
                                            
                                            // Clone the bot BEFORE moving it into the spawn block
                                            let bot_clone = bot.clone(); 
                                            
                                            tokio::spawn(async move {
                                                crate::dag_buffer::DagBuffer::handle_new_utxo(addr_clone, amt_clone, state_clone, bot_clone).await;
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("[NODE] Connection failed: {}. Retrying in 10s...", e);
                sleep(Duration::from_secs(10)).await;
            }
        }
    }
}