use sqlx::SqlitePool;
use std::sync::Arc;
use dashmap::DashMap;
use tokio_util::sync::CancellationToken;
use std::time::Instant;
use crate::api::ApiManager;
use crate::dag_buffer::DagBuffer;

pub const MAX_ACCOUNTS_PER_WALLET: usize = 2;

pub struct WalletData {
    pub last_balance: f64,
}

pub struct AppState {
    pub db_pool: SqlitePool,
    pub monitored_wallets: DashMap<String, Vec<i64>>,
    pub processed_txids: DashMap<String, Instant>,
    pub pending_alerts: DashMap<String, Instant>,
    pub rate_limits: DashMap<i64, Instant>, // Changed to i64 for easier comparison
    pub api_manager: Arc<ApiManager>,
    pub dag_buffer: Arc<DagBuffer>,
    pub admin_id: Option<i64>, // Changed to i64 to fix comparison errors
    pub start_time: Instant,
    pub shutdown_token: CancellationToken,
    pub is_monitoring: std::sync::atomic::AtomicBool,
}

impl AppState {
    pub async fn new(shutdown_token: CancellationToken, db_pool: SqlitePool) -> Self {
        let monitored_wallets = DashMap::new();
        
        if let Ok(rows) = sqlx::query("SELECT address, chat_id FROM tracked_wallets").fetch_all(&db_pool).await {
            for row in rows {
                use sqlx::Row;
                let addr: String = row.get("address");
                let cid: i64 = row.get("chat_id");
                monitored_wallets.entry(addr).or_insert_with(Vec::new).push(cid);
            }
        }

        let admin_id = std::env::var("ADMIN_ID").ok().and_then(|v| v.parse().ok());

        Self {
            db_pool,
            monitored_wallets,
            processed_txids: DashMap::new(),
            pending_alerts: DashMap::new(),
            rate_limits: DashMap::new(),
            api_manager: Arc::new(ApiManager::new()),
            dag_buffer: Arc::new(DagBuffer::new()),
            admin_id,
            start_time: Instant::now(),
            shutdown_token,
            is_monitoring: std::sync::atomic::AtomicBool::new(true),
        }
    }

    pub async fn sync_wallet_to_db(&self, address: &str) {
        if let Some(ids) = self.monitored_wallets.get(address) {
            for &cid in ids.value() {
                let _ = sqlx::query("INSERT OR IGNORE INTO tracked_wallets (address, chat_id) VALUES (?, ?)")
                    .bind(address).bind(cid).execute(&self.db_pool).await;
            }
        }
    }

    pub async fn remove_wallet_from_db(&self, address: &str, chat_id: i64) {
        let _ = sqlx::query("DELETE FROM tracked_wallets WHERE address = ? AND chat_id = ?")
            .bind(address).bind(chat_id).execute(&self.db_pool).await;
    }

    pub fn get_all_users(&self) -> Vec<i64> {
        let mut users = std::collections::HashSet::new();
        for entry in self.monitored_wallets.iter() {
            for &id in entry.value() { users.insert(id); }
        }
        users.into_iter().collect()
    }
}