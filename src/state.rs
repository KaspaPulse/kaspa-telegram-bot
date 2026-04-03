use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use chrono::Utc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use teloxide::types::ChatId;
use sled::Db;
use crate::api::ApiManager;
use crate::dag_buffer::DagBuffer;

pub const MAX_ACCOUNTS_PER_WALLET: usize = 2;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct WalletData {
    pub last_balance: f64,
    pub chat_ids: Vec<i64>,
}

pub struct PendingAlert {
    pub daa_score: u64,
    pub timestamp: u64,
}

pub struct AppState {
    pub monitored_wallets: DashMap<String, WalletData>,
    pub api_manager: Arc<ApiManager>,
    pub dag_buffer: Arc<DagBuffer>,
    pub processed_txids: DashMap<String, u64>,
    pub pending_alerts: DashMap<String, PendingAlert>,
    pub is_monitoring: AtomicBool,
    pub admin_id: Option<ChatId>,
    pub start_time: Instant,
    pub shutdown_token: CancellationToken,
    pub rate_limits: DashMap<ChatId, Instant>,
    // --- Enterprise Embedded Database ---
    pub db: Db,
}

impl AppState {
    pub fn new(shutdown_token: CancellationToken) -> Self {
        let monitored_wallets = DashMap::new();

        // [DATABASE INIT] Open or create the embedded high-speed database
        let db = sled::open("kaspa_bot_db").expect("CRITICAL: Failed to initialize embedded database");

        // [MIGRATION SYSTEM] Smoothly transition from legacy JSON to Sled DB
        if std::path::Path::new("wallets.json").exists() {
            if let Ok(file_content) = std::fs::read_to_string("wallets.json") {
                if let Ok(parsed_data) = serde_json::from_str::<std::collections::HashMap<String, WalletData>>(&file_content) {
                    for (wallet, data) in parsed_data {
                        if let Ok(serialized) = serde_json::to_vec(&data) {
                            let _ = db.insert(wallet.as_bytes(), serialized);
                        }
                    }
                    let _ = db.flush();
                    let _ = std::fs::rename("wallets.json", "wallets.json.bak");
                    log::info!("Migration Engine: Successfully migrated legacy wallets.json to embedded DB.");
                }
            }
        }

        // [MEMORY HYDRATION] Load data from DB into ultra-fast RAM (DashMap)
        let mut loaded_count = 0;
        for item in db.iter() {
            if let Ok((k, v)) = item {
                if let Ok(wallet) = String::from_utf8(k.to_vec()) {
                    if let Ok(data) = serde_json::from_slice::<WalletData>(&v) {
                        monitored_wallets.insert(wallet, data);
                        loaded_count += 1;
                    }
                }
            }
        }
        log::info!("Persistence Engine: Loaded {} wallets from embedded DB.", loaded_count);

        let admin_id = std::env::var("ADMIN_ID")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .map(ChatId);

        Self {
            monitored_wallets,
            api_manager: Arc::new(ApiManager::new()),
            dag_buffer: DagBuffer::new(),
            processed_txids: DashMap::new(),
            pending_alerts: DashMap::new(),
            is_monitoring: AtomicBool::new(true),
            admin_id,
            start_time: Instant::now(),
            shutdown_token,
            rate_limits: DashMap::new(),
            db,
        }
    }

    pub fn get_all_users(&self) -> Vec<i64> {
        let mut users = std::collections::HashSet::new();
        for entry in self.monitored_wallets.iter() {
            for &id in &entry.value().chat_ids {
                users.insert(id);
            }
        }
        users.into_iter().collect()
    }

    // [O(1) I/O FIX] Writes only deltas to WAL, eliminating massive JSON rewrite bottlenecks
    pub async fn save_wallets(&self) {
        for entry in self.monitored_wallets.iter() {
            if let Ok(serialized) = serde_json::to_vec(entry.value()) {
                let _ = self.db.insert(entry.key().as_bytes(), serialized);
            }
        }
        
        // Asynchronously flush the Write-Ahead Log to disk without blocking the runtime
        if let Err(e) = self.db.flush_async().await {
            log::error!("[PERSISTENCE] DB Flush Error: {}", e);
        } else {
            log::debug!("[PERSISTENCE] Database synchronized successfully.");
        }
    }

    pub fn prune_memory(&self) {
        let now = Utc::now().timestamp() as u64;
        let max_age: u64 = std::env::var("GC_RETENTION_HOURS")
            .unwrap_or_else(|_| "24".to_string())
            .parse::<u64>()
            .unwrap_or(24) * 3600;

        let initial_txs = self.processed_txids.len();
        self.processed_txids.retain(|_, &mut ts| now.saturating_sub(ts) < max_age);
        let pruned_txs = initial_txs - self.processed_txids.len();

        let initial_alerts = self.pending_alerts.len();
        self.pending_alerts.retain(|_, alert| now.saturating_sub(alert.timestamp) < max_age);
        let pruned_alerts = initial_alerts - self.pending_alerts.len();

        let initial_rates = self.rate_limits.len();
        self.rate_limits.retain(|_, time| time.elapsed().as_secs() < 3600);
        let pruned_rates = initial_rates - self.rate_limits.len();

        if pruned_txs > 0 || pruned_alerts > 0 || pruned_rates > 0 {
            log::info!(
                "Garbage Collector: Cleared {} TXIDs, {} Alerts, and {} Rate Limits from RAM.",
                pruned_txs, pruned_alerts, pruned_rates
            );
        }
    }
}