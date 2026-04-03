use dashmap::DashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::task::JoinHandle;
use std::time::Duration;
use tokio::sync::Semaphore;

pub struct DagBuffer {
    pub pending_tasks: DashMap<String, JoinHandle<()>>,
    // [ANTI-SPAM] Limits the maximum number of concurrent tracking tasks
    semaphore: Arc<Semaphore>, 
}

impl DagBuffer {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pending_tasks: DashMap::new(),
            // Set max concurrent operations to a safe limit for a standard VPS
            semaphore: Arc::new(Semaphore::new(std::env::var("MAX_CONCURRENT_TXS").unwrap_or_else(|_| "5000".to_string()).parse().unwrap_or(5000))), 
        })
    }

    pub fn handle_orphan(&self, tx_id: &str) {
        if let Some((_, handle)) = self.pending_tasks.remove(tx_id) {
            log::warn!("[DAG BUFFER] Orphaned TX detected! Aborting alert for TX: {}", tx_id);
            handle.abort();
        }
    }

    pub fn queue_tx(
        self: &Arc<Self>,
        bot: Bot,
        tx_id: String,
        wallet: String,
        amount_kas: f64,
        chat_ids: Vec<i64>,
    ) {
        // [ANTI-SPAM] Attempt to acquire a concurrency permit
        // If the server is overwhelmed (e.g., Kaspa network spam attack), we drop the alert.
        let permit = match self.semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                log::error!("[ANTI-SPAM] Dropping TX {} - Max capacity reached. Network spam detected!", tx_id);
                return;
            }
        };

        let buffer_self = Arc::clone(self);
        let tx_clone = tx_id.clone();
        
        let handle = tokio::spawn(async move {
            // Move the permit into the task. It will automatically be released when this task completes.
            let _permit = permit; 
            
            log::info!("[DAG BUFFER] TX {} queued for 20-second settlement.", tx_clone);
            
            tokio::time::sleep(Duration::from_secs(std::env::var("DAG_SETTLEMENT_DELAY_SECS").unwrap_or_else(|_| "20".to_string()).parse().unwrap_or(20))).await;

            buffer_self.pending_tasks.remove(&tx_clone);
            log::info!("[DAG BUFFER] TX {} settled successfully. Processing alert.", tx_clone);

            // Utilizing the shared formatting helper functions
                        // [SECURITY] Neutralize any potential HTML injection payloads
            let safe_wallet = crate::utils::helpers::sanitize_html(&wallet);
            let safe_tx = crate::utils::helpers::sanitize_html(&tx_clone);
            let short_wallet = crate::utils::helpers::format_short_wallet(&safe_wallet);
            let short_tx = crate::utils::helpers::format_short_wallet(&safe_tx);
            
                        let initial_msg = format!(
                "⚡ <b>Native Node Reward!</b> 💎\n━━━━━━━━━━━━━━━━━━\n<b>Wallet:</b> <a href=\"https://kaspa.stream/addresses/{}\">{}</a>\n<b>Amount:</b> <code>+{:.2} KAS</code>\n━━━━━━━━━━━━━━━━━━\n<b>TXID:</b> <a href=\"https://kaspa.stream/transactions/{}\">{}</a>\n<b>Status:</b> ⏳ <code>Indexing Hashes...</code>",
                safe_wallet, short_wallet, amount_kas, safe_tx, short_tx
            );

            let mut sent_messages = Vec::new();
            for cid in chat_ids {
                if let Ok(msg) = bot.send_message(teloxide::types::ChatId(cid), &initial_msg)
                    .parse_mode(ParseMode::Html)
                    .disable_web_page_preview(true)
                    .await 
                {
                    sent_messages.push((teloxide::types::ChatId(cid), msg.id));
                }
            }

            for attempt in 1..=45 {
                tokio::time::sleep(Duration::from_secs(10)).await;
                log::info!("[API POLL] Fetching hashes for TX {} (Attempt {})", tx_clone, attempt);
                
                let api_url = format!("https://api.kaspa.org/transactions/{}", tx_clone);
                if true { // [ARCHITECTURE] Dummy scope to maintain strict brace alignment
                    if let Some(json) = crate::utils::helpers::fetch_json_safe(&api_url, 1_048_576).await { // 1MB Hard Limit
                        if let Some(blocks) = json["block_hash"].as_array() {
                            if !blocks.is_empty() {
                                let block_hash = blocks[0].as_str().unwrap_or("Unknown");
                                let short_block = crate::utils::helpers::format_short_wallet(block_hash);
                                
                                            let final_msg = format!(
                "⚡ <b>Native Node Reward!</b> 💎\n━━━━━━━━━━━━━━━━━━\n<b>Wallet:</b> <a href=\"https://kaspa.stream/addresses/{}\">{}</a>\n<b>Amount:</b> <code>+{:.2} KAS</code>\n━━━━━━━━━━━━━━━━━━\n<b>TXID:</b> <a href=\"https://kaspa.stream/transactions/{}\">{}</a>\n<b>TX Block:</b> <a href=\"https://kaspa.stream/blocks/{}\">{}</a>\n✅ <b>Confirmed in DAG</b>",
                safe_wallet, short_wallet, amount_kas, safe_tx, short_tx, block_hash, short_block
            );

                                for (cid, mid) in sent_messages {
                                    let _ = bot.edit_message_text(cid, mid, &final_msg)
                                        .parse_mode(ParseMode::Html)
                                        .disable_web_page_preview(true)
                                        .await;
                                }
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.pending_tasks.insert(tx_id, handle);
    }
}