use dashmap::{DashMap, DashSet};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::atomic::AtomicBool;

pub const WALLETS_FILE: &str = "wallets.json";
pub const TRACKERS_FILE: &str = "trackers.json";
pub const CONFIRMATIONS_NEEDED: u64 = 5000;
pub const MAX_ACCOUNTS_PER_WALLET: usize = 2;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WalletData {
    pub last_balance: f64,
    pub chat_ids: Vec<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessageRef {
    #[serde(rename = "chatId")]
    pub chat_id: i64,
    #[serde(rename = "messageId")]
    pub message_id: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ActiveTracker {
    pub messages: Vec<MessageRef>,
    #[serde(rename = "daaScore")]
    pub daa_score: u64,
}

#[derive(Clone, Debug)]
pub struct PendingAlert {
    pub daa_score: u64,
}

pub struct AppState {
    pub monitored_wallets: DashMap<String, WalletData>,
    pub active_trackers: DashMap<String, ActiveTracker>,
    pub processed_txids: DashSet<String>,
    pub pending_alerts: DashMap<String, PendingAlert>,
    pub is_monitoring: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            monitored_wallets: DashMap::new(),
            active_trackers: DashMap::new(),
            processed_txids: DashSet::new(),
            pending_alerts: DashMap::new(),
            is_monitoring: AtomicBool::new(true),
        }
    }

    pub fn load_data(&self, admin_id: Option<i64>) {
        if let Ok(data) = fs::read_to_string(WALLETS_FILE) {
            if let Ok(mut parsed) =
                serde_json::from_str::<std::collections::HashMap<String, WalletData>>(&data)
            {
                for (k, mut v) in parsed.drain() {
                    if v.chat_ids.is_empty() {
                        if let Some(id) = admin_id {
                            v.chat_ids.push(id);
                        }
                    }
                    self.monitored_wallets.insert(k, v);
                }
            }
        }
        if let Ok(data) = fs::read_to_string(TRACKERS_FILE) {
            if let Ok(parsed) =
                serde_json::from_str::<std::collections::HashMap<String, ActiveTracker>>(&data)
            {
                for (k, v) in parsed {
                    self.active_trackers.insert(k, v);
                }
            }
        }
    }

    pub fn save_wallets(&self) {
        let mut map = std::collections::HashMap::new();
        for kv in self.monitored_wallets.iter() {
            map.insert(kv.key().clone(), kv.value().clone());
        }
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = fs::write(WALLETS_FILE, json);
        }
    }

    pub fn save_trackers(&self) {
        let mut map = std::collections::HashMap::new();
        for kv in self.active_trackers.iter() {
            map.insert(kv.key().clone(), kv.value().clone());
        }
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = fs::write(TRACKERS_FILE, json);
        }
    }

    // [NEW] Memory Cleanup Logic matching bot.js processLiveTrackers()
    pub fn cleanup_old_trackers(&self, current_daa: u64) {
        let mut to_remove = Vec::new();
        for tracker in self.active_trackers.iter() {
            if current_daa > tracker.value().daa_score
                && (current_daa - tracker.value().daa_score) >= CONFIRMATIONS_NEEDED
            {
                to_remove.push(tracker.key().clone());
            }
        }
        let mut changed = false;
        for key in to_remove {
            self.active_trackers.remove(&key);
            log::info!(
                "[CLEANUP] TX {} reached confirmations. Removed from active memory.",
                key
            );
            changed = true;
        }
        if changed {
            self.save_trackers();
        }
    }

    pub fn get_all_users(&self) -> Vec<i64> {
        let mut users = std::collections::HashSet::new();
        for w in self.monitored_wallets.iter() {
            for id in &w.value().chat_ids {
                users.insert(*id);
            }
        }
        users.into_iter().collect()
    }
}
