use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, tungstenite::client::IntoClientRequest};
use futures_util::{StreamExt, SinkExt};

pub struct LocalNodeRpc;

impl LocalNodeRpc {
    pub async fn query(request_type: &str, params: serde_json::Value) -> Option<serde_json::Value> {
        let ws_url = std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
        
        // [FIX] Added .clone() so the URL isn't consumed, leaving it available for the log below
        let mut request = match ws_url.clone().into_client_request() {
            Ok(req) => req,
            Err(_) => return None,
        };
        request.headers_mut().insert("sec-websocket-protocol", "wrpc-json".parse().unwrap());

        if let Ok((mut ws_stream, _)) = connect_async(request).await {
            let mut payload_map = serde_json::Map::new();
            payload_map.insert(request_type.to_string(), params);
            let payload = serde_json::Value::Object(payload_map);

            if ws_stream.send(Message::Text(payload.to_string())).await.is_ok() {
                if let Ok(Some(Ok(Message::Text(msg)))) = tokio::time::timeout(std::time::Duration::from_secs(5), ws_stream.next()).await {
                    let _ = ws_stream.close(None).await;
                    
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&msg) {
                        let response_key = format!("{}Response", request_type.replace("Request", ""));
                        if let Some(res) = json.get(&response_key) {
                            if let Some(error) = res.get("error") {
                                log::error!("[LOCAL RPC] Node returned error: {:?}", error);
                                return None;
                            }
                            return Some(res.clone());
                        }
                    }
                }
            }
        } else {
            log::warn!("[LOCAL RPC] Failed to connect to node at {}", ws_url);
        }
        None
    }

    pub async fn get_supply() -> Option<(f64, f64)> {
        if let Some(res) = Self::query("getCoinSupplyRequest", serde_json::json!({})).await {
            let circ = res["circulatingSompi"].as_u64().unwrap_or(0) as f64 / 100_000_000.0;
            let max = res["maxSompi"].as_u64().unwrap_or(0) as f64 / 100_000_000.0;
            if circ > 0.0 {
                return Some((circ, max));
            }
        }
        None
    }

    pub async fn get_dag_info() -> Option<(String, u64, u64, f64)> {
        if let Some(res) = Self::query("getBlockDagInfoRequest", serde_json::json!({})).await {
            let network = res["networkName"].as_str().unwrap_or("Unknown").to_string();
            let blocks = res["blockCount"].as_u64().unwrap_or(0);
            let headers = res["headerCount"].as_u64().unwrap_or(0);
            let diff = res["difficulty"].as_f64().unwrap_or(0.0);
            return Some((network, blocks, headers, diff));
        }
        None
    }
}
