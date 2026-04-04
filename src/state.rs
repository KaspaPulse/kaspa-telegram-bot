use std::collections::{HashSet, HashMap};
use std::sync::Arc;
use dashmap::DashMap;
use tokio::fs;
use tracing::info;

pub type SharedState = Arc<DashMap<String, HashSet<i64>>>;
pub type UtxoState = Arc<DashMap<String, HashSet<String>>>;

pub async fn save_state(state: &SharedState) {
    let map: HashMap<String, HashSet<i64>> = state.iter().map(|e| (e.key().clone(), e.value().clone())).collect();
    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = fs::write("wallets.json", json).await;
        info!("[SYSTEM] State saved persistently to wallets.json");
    }
}
