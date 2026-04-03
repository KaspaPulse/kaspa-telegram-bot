use futures_util::{StreamExt, SinkExt};
use std::sync::Arc;
use crate::state::AppState;
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

        let ws_url = std::env::var("NODE_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
        log::info!("[NODE] Connecting to WebSocket: {}", ws_url);

        let mut request = match ws_url.into_client_request() {
            Ok(req) => req,
            Err(e) => {
                log::error!("[NODE] Invalid WebSocket URL: {}", e);
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        
        // [CRITICAL FIX 1] The rusty-kaspa node WILL DROP the connection instantly if this header is missing!
        request.headers_mut().insert("sec-websocket-protocol", "wrpc-json".parse().unwrap());

        match tokio_tungstenite::connect_async(request).await {
            Ok((mut ws_stream, _)) => {
                log::info!("✅ [NODE] Connected! Initializing Subscriptions...");

                let wallets: Vec<String> = state.monitored_wallets.iter().map(|e| e.key().clone()).collect();
                if !wallets.is_empty() {
                    // [CRITICAL FIX 2] Exact workflow-rpc JSON payload
                    let sub_request = serde_json::json!({
                        "id": 1,
                        "method": "notifyUtxosChanged",
                        "params": { "addresses": wallets }
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
                                
                                if let Some(err) = json.get("error") {
                                    log::error!("[NODE] RPC Error from Server: {:?}", err);
                                    continue;
                                }

                                // [CRITICAL FIX 3] Exact workflow-rpc notification structure
                                if let Some(method) = json.get("method").and_then(|m| m.as_str()) {
                                    if method == "utxosChangedNotification" {
                                        if let Some(params) = json.get("params") {
                                            if let Some(added) = params.get("added").and_then(|a| a.as_array()) {
                                                for utxo in added {
                                                    // Robust Address Extraction
                                                    let address = match utxo.get("address") {
                                                        Some(Value::String(s)) => s.clone(),
                                                        Some(Value::Object(obj)) => {
                                                            let prefix = obj.get("prefix").and_then(|v| v.as_str()).unwrap_or("kaspa");
                                                            let payload = obj.get("payload").and_then(|v| v.as_str()).unwrap_or("");
                                                            format!("{}:{}", prefix, payload)
                                                        },
                                                        _ => continue,
                                                    };

                                                    // Robust Amount Extraction
                                                    let amount_sompi: u64 = match utxo.get("utxoEntry").and_then(|e| e.get("amount")) {
                                                        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
                                                        Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
                                                        _ => 0,
                                                    };

                                                    if amount_sompi == 0 { continue; }

                                                    if state.monitored_wallets.contains_key(&address) {
                                                        let amount_kas = (amount_sompi as f64) / 100_000_000.0;
                                                        let amount_str = amount_kas.to_string();
                                                        
                                                        log::info!("[NOTIFICATION] Match Found! Address: {}, Amount: {} KAS", address, amount_str);
                                                        
                                                        let state_clone = state.clone();
                                                        let addr_clone = address.clone();
                                                        let amt_clone = amount_str.clone();
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
                        Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                            log::warn!("[NODE] WebSocket Closed by Server. Handshake rejected or node restarted.");
                            break;
                        }
                        Err(e) => {
                            log::error!("[NODE] WebSocket Network Error: {}", e);
                            break;
                        }
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