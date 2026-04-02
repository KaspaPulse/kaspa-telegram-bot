#![allow(deprecated)]
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::task::JoinHandle;

pub struct DagBuffer {
    // Stores the async task handles mapped by Transaction ID
    pending_tasks: DashMap<String, JoinHandle<()>>,
}

impl DagBuffer {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pending_tasks: DashMap::new(),
        })
    }

    /// Called when 'removed' UTXO event is received from the Node
    pub fn handle_orphan(&self, tx_id: &str) {
        if let Some((_, handle)) = self.pending_tasks.remove(tx_id) {
            log::warn!(
                "[DAG BUFFER] Orphaned TX detected! Aborting alert for TX: {}",
                tx_id
            );
            handle.abort(); // Instantly kills the 20-second waiting task
        }
    }

    /// Called when 'added' UTXO event is received from the Node
    pub fn queue_tx(
        self: &Arc<Self>,
        bot: Bot,
        tx_id: String,
        wallet: String,
        amount_kas: f64,
        chat_ids: Vec<teloxide::types::ChatId>,
    ) {
        let buffer_self = Arc::clone(self);
        let tx_clone = tx_id.clone();

        // Spawn an isolated lightweight Tokio thread
        let handle = tokio::spawn(async move {
            log::info!(
                "[DAG BUFFER] TX {} queued for 20-second BlockDAG settlement.",
                tx_clone
            );

            // 1. The 20-Second Settlement Buffer
            tokio::time::sleep(Duration::from_secs(20)).await;

            // If the thread wasn't aborted by handle_orphan, it means the TX is settled!
            buffer_self.pending_tasks.remove(&tx_clone);
            log::info!(
                "[DAG BUFFER] TX {} settled successfully. Processing alert.",
                tx_clone
            );

            let short_tx = format!("{}...{}", &tx_clone[0..8], &tx_clone[tx_clone.len() - 8..]);
            let short_wallet = format!("{}...{}", &wallet[0..12], &wallet[wallet.len() - 6..]);

            let initial_msg = format!(
                "⚡ *Native Node Reward!* 💎\n━━━━━━━━━━━━━━━━━━\n*Wallet:* [{}]({})\n*Amount:* +{:.2} KAS\n━━━━━━━━━━━━━━━━━━\n*TXID:* [{}]({})\n*Status:* ⏳ Indexing Hashes...",
                short_wallet, format!("https://kaspa.stream/addresses/{}", wallet),
                amount_kas,
                short_tx, format!("https://kaspa.stream/transactions/{}", tx_clone)
            );

            // 2. Send Initial Messages and store Message IDs for later editing
            let mut sent_messages = Vec::new();
            for cid in chat_ids {
                if let Ok(msg) = bot
                    .send_message(cid, &initial_msg)
                    .parse_mode(ParseMode::Markdown)
                    .disable_web_page_preview(true)
                    .await
                {
                    sent_messages.push((cid, msg.id));
                }
            }

            // 3. Background API Polling for Block Hashes (45 attempts, 10s intervals)
            for attempt in 1..=45 {
                tokio::time::sleep(Duration::from_secs(10)).await;
                log::info!(
                    "[API POLL] Fetching hashes for TX {} (Attempt {})",
                    tx_clone,
                    attempt
                );

                let api_url = format!("https://api.kaspa.org/transactions/{}", tx_clone);
                if let Ok(resp) = reqwest::get(&api_url).await {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if let Some(blocks) = json["block_hash"].as_array() {
                            if !blocks.is_empty() {
                                let block_hash = blocks[0].as_str().unwrap_or("Unknown");
                                let short_block = format!(
                                    "{}...{}",
                                    &block_hash[0..8],
                                    &block_hash[block_hash.len() - 8..]
                                );

                                let final_msg = format!(
                                    "⚡ *Native Node Reward!* 💎\n━━━━━━━━━━━━━━━━━━\n*Wallet:* [{}]({})\n*Amount:* +{:.2} KAS\n━━━━━━━━━━━━━━━━━━\n*TXID:* [{}]({})\n*TX Block:* [{}]({})\n✅ *Confirmed in DAG*",
                                    short_wallet, format!("https://kaspa.stream/addresses/{}", wallet),
                                    amount_kas,
                                    short_tx, format!("https://kaspa.stream/transactions/{}", tx_clone),
                                    short_block, format!("https://kaspa.stream/blocks/{}", block_hash)
                                );

                                // Edit the initial messages with the final block data
                                for (cid, mid) in sent_messages {
                                    let _ = bot
                                        .edit_message_text(cid, mid, &final_msg)
                                        .parse_mode(ParseMode::Markdown)
                                        .disable_web_page_preview(true)
                                        .await;
                                }
                                break; // Exit the loop successfully
                            }
                        }
                    }
                }
            }
        });

        // Store the handle so it can be aborted if an orphan event occurs
        self.pending_tasks.insert(tx_id, handle);
    }
}
