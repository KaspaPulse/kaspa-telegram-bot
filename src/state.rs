use dashmap::{DashMap, DashSet};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

pub const MAX_ACCOUNTS_PER_WALLET: usize = 2;

#[derive(Serialize, Deserialize, Clone)]
pub struct WalletData {
    pub last_balance: f64,
    pub chat_ids: Vec<i64>,
}

pub struct ActiveTracker {
    pub messages: Vec<MessageRef>,
    pub daa_score: u64,
}

pub struct MessageRef {
    pub chat_id: i64,
    pub message_id: i32,
}

pub struct PendingAlert {
    pub daa_score: u64,
}

pub struct AppState {
    pub monitored_wallets: DashMap<String, WalletData>,
    pub active_trackers: DashMap<String, ActiveTracker>,
    pub pending_alerts: DashMap<String, PendingAlert>,
    pub processed_txids: DashSet<String>,
    pub is_monitoring: AtomicBool,

    // NATIVE ADMIN FEATURES
    pub admin_id: Option<i64>,
    pub start_time: Instant,
    pub rate_limits: DashMap<i64, Instant>,
    pub shutdown_token: CancellationToken,
}

impl AppState {
    pub fn new(shutdown_token: CancellationToken) -> Self {
        let admin_id = std::env::var("ADMIN_ID")
            .ok()
            .and_then(|v| v.trim().parse::<i64>().ok());
        let state = Self {
            monitored_wallets: DashMap::new(),
            active_trackers: DashMap::new(),
            pending_alerts: DashMap::new(),
            processed_txids: DashSet::new(),
            is_monitoring: AtomicBool::new(true),
            admin_id,
            start_time: Instant::now(),
            rate_limits: DashMap::new(),
            shutdown_token,
        };
        state.load_wallets();
        state
    }

    pub fn load_wallets(&self) {
        if let Ok(data) = std::fs::read_to_string("wallets.json") {
            if let Ok(parsed) = serde_json::from_str::<HashMap<String, WalletData>>(&data) {
                for (k, mut v) in parsed {
                    if v.chat_ids.is_empty() {
                        if let Some(aid) = self.admin_id {
                            v.chat_ids.push(aid);
                        }
                    }
                    self.monitored_wallets.insert(k, v);
                }
            }
        }
    }

    pub async fn save_wallets(&self) {
        let map: HashMap<String, WalletData> = self
            .monitored_wallets
            .iter()
            .map(|kv| (kv.key().clone(), kv.value().clone()))
            .collect();
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = tokio::fs::write("wallets.json", json).await;
        }
    }

    pub async fn save_trackers(&self) {}

    pub fn get_all_users(&self) -> Vec<i64> {
        let mut users = HashSet::new();
        for kv in self.monitored_wallets.iter() {
            for id in &kv.value().chat_ids {
                users.insert(*id);
            }
        }
        users.into_iter().collect()
    }
}
