use futures_util::{StreamExt, SinkExt};
use std::sync::Arc;
use crate::state::{AppState, PendingAlert};
use tokio::time::{sleep, Duration};
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

pub async fn start_kaspa_listener(state: Arc<AppState>, bot: teloxide::Bot) {
    loop {
        sleep(Duration::from_secs(2)).await;

        if !state.is_monitoring.load(std::sync::atomic::Ordering::SeqCst) {
            sleep(Duration::from_secs(5)).await;
            continue;
        }

        let ws_url = std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
        
        let mut request = match ws_url.clone().into_client_request() {
            Ok(req) => req,
            Err(e) => {
                log::error!("[NODE] Invalid WebSocket URL: {}", e);
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        
        request.headers_mut().insert("sec-websocket-protocol", "wrpc-json".parse().unwrap());

        match tokio_tungstenite::connect_async(request).await {
            Ok((mut ws_stream, _)) => {
                log::info!("✅ [NODE] Connected! Initializing Subscriptions...");

                let wallets: Vec<String> = state.monitored_wallets.iter().map(|e| e.key().clone()).collect();
                if !wallets.is_empty() {
                    let sub_request = serde_json::json!({
                        "notifyUtxosChangedRequest": { "addresses": wallets }
                    });
                    
                    if let Err(e) = ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(sub_request.to_string())).await {
                        log::error!("[NODE] Failed to send subscription: {}", e);
                    } else {
                        log::info!("🔄 [NODE] Subscription Sent for {} wallets.", wallets.len());
                    }
                }

                while let Some(msg) = ws_stream.next().await {
                    match msg {
                        Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                                if let Some(notification) = json.get("utxosChangedNotification") {
                                    
                                    // 1. Process Removed (Orphan detection)
                                    if let Some(removed) = notification.get("removed").and_then(|r| r.as_array()) {
                                        for utxo in removed {
                                            if let Some(tx_id) = utxo.get("outpoint").and_then(|o| o.get("transactionId")).and_then(|t| t.as_str()) {
                                                if state.pending_alerts.remove(tx_id).is_some() {
                                                    log::info!("[SILENT ORPHAN] TX {} suppressed within buffer.", tx_id);
                                                }
                                                if state.active_trackers.remove(tx_id).is_some() {
                                                    log::info!("[ORPHAN] TX {} removed from DAG. Tracker removed.", tx_id);
                                                }
                                            }
                                        }
                                    }

                                    // 2. Process Added (New UTXO detection)
                                    if let Some(added) = notification.get("added").and_then(|a| a.as_array()) {
                                        for utxo in added {
                                            let address = match utxo.get("address") {
                                                Some(Value::String(s)) => s.clone(),
                                                Some(Value::Object(obj)) => {
                                                    let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("kaspa");
                                                    let payload = obj.get("payload").and_then(|v| v.as_str()).unwrap_or("");
                                                    format!("{}:{}", prefix, payload)
                                                },
                                                _ => continue,
                                            };

                                            let tx_id = match utxo.get("outpoint").and_then(|o| o.get("transactionId")).and_then(|t| t.as_str()) {
                                                Some(t) => t.to_string(),
                                                None => continue,
                                            };

                                            let amount_sompi: u64 = match utxo.get("utxoEntry").and_then(|e| e.get("amount")) {
                                                Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
                                                Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
                                                _ => 0,
                                            };

                                            let daa_score: u64 = match utxo.get("utxoEntry").and_then(|e| e.get("blockDaaScore")) {
                                                Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
                                                Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
                                                _ => 0,
                                            };

                                            if amount_sompi == 0 { continue; }

                                            if state.monitored_wallets.contains_key(&address) {
                                                let amount_kas = (amount_sompi as f64) / 100_000_000.0;
                                                
                                                if state.processed_txids.contains_key(&tx_id) { continue; }

                                                log::info!("[PENDING] TX {} queued for 20 seconds buffer.", tx_id);
                                                
                                                state.pending_alerts.insert(tx_id.clone(), PendingAlert { daa_score });
                                                
                                                let state_clone = state.clone();
                                                let bot_clone = bot.clone();
                                                
                                                tokio::spawn(async move {
                                                    crate::dag_buffer::process_reward(tx_id, address, amount_kas, daa_score, state_clone, bot_clone).await;
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => { break; }
                        Err(_) => { break; }
                        _ => {}
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
