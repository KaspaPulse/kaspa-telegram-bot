use dashmap::{DashMap, DashSet};
use sqlx::{SqlitePool, Row};
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

pub const MAX_ACCOUNTS_PER_WALLET: usize = 2;

#[derive(Clone, Debug)]
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
    pub db_pool: SqlitePool,
    pub monitored_wallets: DashMap<String, WalletData>,
    pub active_trackers: DashMap<String, ActiveTracker>,
    pub pending_alerts: DashMap<String, PendingAlert>,
    pub processed_txids: DashSet<String>,
    pub is_monitoring: AtomicBool,
    pub admin_id: Option<i64>,
    pub start_time: Instant,
    pub rate_limits: DashMap<i64, Instant>,
    pub shutdown_token: CancellationToken,
}

impl AppState {
    pub async fn new(shutdown_token: CancellationToken, db_pool: SqlitePool) -> Self {
        let admin_id = std::env::var("ADMIN_ID").ok().and_then(|v| v.trim().parse::<i64>().ok());
        
        // Ensure Database Tables Exist
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tracked_wallets (
                address TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                last_balance REAL DEFAULT 0.0,
                PRIMARY KEY (address, chat_id)
            )"
        ).execute(&db_pool).await.expect("Failed to create DB tables");

        let state = Self {
            db_pool,
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
        
        state.load_wallets_from_db().await;
        state
    }

    async fn load_wallets_from_db(&self) {
        let rows = sqlx::query("SELECT address, chat_id, last_balance FROM tracked_wallets")
            .fetch_all(&self.db_pool)
            .await
            .unwrap_or_default();

        for row in rows {
            let address: String = row.try_get("address").unwrap_or_default();
            let chat_id: i64 = row.try_get("chat_id").unwrap_or(0);
            let balance: f64 = row.try_get("last_balance").unwrap_or(0.0);

            let mut entry = self.monitored_wallets.entry(address.clone()).or_insert_with(|| {
                WalletData { last_balance: balance, chat_ids: Vec::new() }
            });
            
            if !entry.chat_ids.contains(&chat_id) {
                entry.chat_ids.push(chat_id);
            }
            // Sync admin to all wallets if needed as fallback
            if let Some(admin) = self.admin_id {
                if entry.chat_ids.is_empty() { entry.chat_ids.push(admin); }
            }
        }
        tracing::info!("🗄️ Database Load Complete: {} unique wallets tracking across active users.", self.monitored_wallets.len());
    }

    pub async fn sync_wallet_to_db(&self, address: &str) {
        if let Some(data) = self.monitored_wallets.get(address) {
            let balance = data.last_balance;
            for chat_id in &data.chat_ids {
                let _ = sqlx::query(
                    "INSERT INTO tracked_wallets (address, chat_id, last_balance) 
                     VALUES (?, ?, ?) 
                     ON CONFLICT(address, chat_id) DO UPDATE SET last_balance = excluded.last_balance"
                )
                .bind(address)
                .bind(chat_id)
                .bind(balance)
                .execute(&self.db_pool)
                .await;
            }
        }
    }

    pub async fn remove_wallet_from_db(&self, address: &str, chat_id: i64) {
        let _ = sqlx::query("DELETE FROM tracked_wallets WHERE address = ? AND chat_id = ?")
            .bind(address)
            .bind(chat_id)
            .execute(&self.db_pool)
            .await;
    }

    // Graceful Shutdown Master Flush
    pub async fn save_wallets(&self) {
        for kv in self.monitored_wallets.iter() {
            self.sync_wallet_to_db(kv.key()).await;
        }
    }

    pub async fn save_trackers(&self) { /* Trackers remain ephemeral in-memory */ }
    
    pub fn get_all_users(&self) -> Vec<i64> {
        let mut users = HashSet::new();
        for kv in self.monitored_wallets.iter() {
            for id in &kv.value().chat_ids { users.insert(*id); }
        }
        users.into_iter().collect()
    }
}
