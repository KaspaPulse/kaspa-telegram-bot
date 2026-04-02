use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use crate::api::ApiManager;
use crate::dag_buffer::DagBuffer;

pub const MAX_ACCOUNTS_PER_WALLET: usize = 2;

pub struct WalletData {
    pub last_balance: f64,
    pub chat_ids: Vec<i64>,
}

pub struct PendingAlert {
    pub daa_score: u64,
}

pub struct AppState {
    pub monitored_wallets: DashMap<String, WalletData>,
    pub api_manager: Arc<ApiManager>,
    pub dag_buffer: Arc<DagBuffer>,
    pub processed_txids: DashMap<String, ()>,
    pub pending_alerts: DashMap<String, PendingAlert>,
    pub is_monitoring: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            monitored_wallets: DashMap::new(),
            api_manager: Arc::new(ApiManager::new()),
            dag_buffer: DagBuffer::new(),
            processed_txids: DashMap::new(),
            pending_alerts: DashMap::new(),
            is_monitoring: AtomicBool::new(true),
        }
    }

    /// Helper for broadcast command
    pub fn get_all_users(&self) -> Vec<i64> {
        let mut users = std::collections::HashSet::new();
        for entry in self.monitored_wallets.iter() {
            for &id in &entry.value().chat_ids {
                users.insert(id);
            }
        }
        users.into_iter().collect()
    }

    pub fn save_wallets(&self) {
        log::info!("Saving wallet state to disk...");
        // In a real scenario, you would write monitored_wallets to wallets.json here
    }
}