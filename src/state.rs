use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use chrono::Utc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use teloxide::types::ChatId;
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
    // --- Restored Enterprise Bot Fields ---
    pub admin_id: Option<ChatId>,
    pub start_time: Instant,
    pub shutdown_token: CancellationToken,
    pub rate_limits: DashMap<ChatId, Instant>,
}

impl AppState {
    pub fn new(shutdown_token: CancellationToken) -> Self {
        let monitored_wallets = DashMap::new();

        if let Ok(file_content) = std::fs::read_to_string("wallets.json") {
            if let Ok(parsed_data) = serde_json::from_str::<std::collections::HashMap<String, WalletData>>(&file_content) {
                for (wallet, data) in parsed_data {
                    monitored_wallets.insert(wallet, data);
                }
                log::info!("Persistence Engine: Loaded {} wallets.", monitored_wallets.len());
            }
        }

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

    // [FIX] Converted to async to satisfy .await calls in bot.rs and kaspa.rs
    pub async fn save_wallets(&self) {
        let mut export_map = std::collections::HashMap::new();
        for entry in self.monitored_wallets.iter() {
            export_map.insert(entry.key().clone(), entry.value().clone());
        }

        if let Ok(json_data) = serde_json::to_string_pretty(&export_map) {
            let _ = tokio::fs::write("wallets.json", json_data).await;
        }
    }

    pub fn prune_memory(&self) {
        let now = Utc::now().timestamp() as u64;
        let max_age: u64 = std::env::var("GC_RETENTION_HOURS").unwrap_or_else(|_| "24".to_string()).parse::<u64>().unwrap_or(24) * 3600;

        let initial_txs = self.processed_txids.len();
        self.processed_txids.retain(|_, &mut ts| now.saturating_sub(ts) < max_age);
        let pruned_txs = initial_txs - self.processed_txids.len();

        let initial_alerts = self.pending_alerts.len();
        self.pending_alerts.retain(|_, alert| now.saturating_sub(alert.timestamp) < max_age);
        let pruned_alerts = initial_alerts - self.pending_alerts.len();

        if pruned_txs > 0 || pruned_alerts > 0 {
            log::info!("Garbage Collector: Cleared {} TXIDs and {} Alerts from RAM.", pruned_txs, pruned_alerts);
        }
    }
}