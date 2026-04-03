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

// --- Enterprise Security Imports ---
use aes_gcm::{aead::{Aead, KeyInit, OsRng}, Aes256Gcm, Key, Nonce};
use aes_gcm::aead::AeadCore;
use sha2::{Sha256, Digest};

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
    pub db: Db,
}

// --- Zero-Trust Encryption Engine ---
fn derive_key() -> Key<Aes256Gcm> {
    let secret = std::env::var("DB_ENCRYPTION_KEY")
        .unwrap_or_else(|_| "KaspaEnterpriseFallbackSecret2026".to_string());
    
    // Hash the secret to strictly enforce the 32-byte key requirement for AES-256
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let result = hasher.finalize();
    *Key::<Aes256Gcm>::from_slice(&result)
}

fn encrypt_payload(data: &[u8]) -> Vec<u8> {
    let key = derive_key();
    let cipher = Aes256Gcm::new(&key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 12 bytes
    
    let ciphertext = cipher.encrypt(&nonce, data).expect("CRITICAL: Encryption Engine Failure");
    
    let mut output = nonce.to_vec();
    output.extend_from_slice(&ciphertext);
    output
}

fn decrypt_payload(data: &[u8]) -> Vec<u8> {
    if data.len() > 12 {
        let key = derive_key();
        let cipher = Aes256Gcm::new(&key);
        let nonce = Nonce::from_slice(&data[0..12]);
        
        if let Ok(decrypted) = cipher.decrypt(nonce, &data[12..]) {
            return decrypted;
        }
    }
    // Legacy Fallback: If decryption fails, assume data is from the pre-encryption plaintext era
    data.to_vec()
}

impl AppState {
    pub fn new(shutdown_token: CancellationToken) -> Self {
        let monitored_wallets = DashMap::new();
        let db = sled::open("kaspa_bot_db").expect("CRITICAL: Failed to initialize secure database");

        // Migrate and encrypt legacy JSON if it exists
        if std::path::Path::new("wallets.json").exists() {
            if let Ok(file_content) = std::fs::read_to_string("wallets.json") {
                if let Ok(parsed_data) = serde_json::from_str::<std::collections::HashMap<String, WalletData>>(&file_content) {
                    for (wallet, data) in parsed_data {
                        if let Ok(serialized) = serde_json::to_vec(&data) {
                            let encrypted = encrypt_payload(&serialized);
                            let _ = db.insert(wallet.as_bytes(), encrypted);
                        }
                    }
                    let _ = db.flush();
                    let _ = std::fs::rename("wallets.json", "wallets.json.bak");
                    log::info!("Security Engine: Legacy JSON migrated and encrypted successfully.");
                }
            }
        }

        let mut loaded_count = 0;
        for item in db.iter() {
            if let Ok((k, v)) = item {
                if let Ok(wallet) = String::from_utf8(k.to_vec()) {
                    let decrypted = decrypt_payload(&v);
                    if let Ok(data) = serde_json::from_slice::<WalletData>(&decrypted) {
                        monitored_wallets.insert(wallet, data);
                        loaded_count += 1;
                    }
                }
            }
        }
        log::info!("Zero-Trust Security Engine: Decrypted and loaded {} wallets into RAM.", loaded_count);

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

        // [ARCHITECTURE FIX] Prevent Tokio Thread Starvation by offloading CPU/IO to a dedicated blocking thread pool
    pub async fn save_wallets(&self) {
        let db_clone = self.db.clone();
        
        // Step 1: Snapshot the DashMap to release RAM locks instantly
        let mut snapshot = Vec::new();
        for entry in self.monitored_wallets.iter() {
            snapshot.push((entry.key().clone(), entry.value().clone()));
        }

        // Step 2: Offload heavy encryption (CPU-Bound) and database inserts (Blocking I/O)
        let _ = tokio::task::spawn_blocking(move || {
            for (wallet, data) in snapshot {
                if let Ok(serialized) = serde_json::to_vec(&data) {
                    // Utilizing the encryption engine safely in a background thread
                    let encrypted = encrypt_payload(&serialized);
                    let _ = db_clone.insert(wallet.as_bytes(), encrypted);
                }
            }
        }).await;
        
        // Step 3: Flush asynchronously to keep the network event loop ultra-fast
        if let Err(e) = self.db.flush_async().await {
            log::error!("[SECURITY] Database Flush Error: {}", e);
        } else {
            log::debug!("[SECURITY] Database synchronized successfully without blocking async threads.");
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