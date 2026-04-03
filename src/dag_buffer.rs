use std::sync::Arc;
use crate::state::{AppState, ActiveTracker, MessageRef};
use tokio::time::{sleep, Duration};
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use teloxide::payloads::{SendMessageSetters, EditMessageTextSetters};
use chrono::Utc;
use reqwest::Client;
use serde_json::Value;

fn shorten(s: &str, chars: usize) -> String {
    if s.len() <= chars * 2 { return s.to_string(); }
    format!("{}...{}", &s[0..chars], &s[s.len() - chars..])
}

fn build_msg(
    time_str: &str, wallet: &str, short_wallet: &str, amount: f64, live_bal: f64, 
    tx_id: &str, short_tx_id: &str, mined: &str, tx_block: &str, accepting: &str, daa: u64
) -> String {
    format!(
        "⚡ *Native Node Reward!* 💎\n━━━━━━━━━━━━━━━━━━\n*Time:* {}\n*Wallet:* [{}](https://kaspa.stream/addresses/{})\n*Amount:* +{:.8} KAS\n*Live Balance:* {:.8} KAS\n━━━━━━━━━━━━━━━━━━\n*TXID:* [{}](https://kaspa.stream/transactions/{})\n*Mined Block:* {}\n*TX Block:* {}\n*Accepting Block:* {}\n*DAA Score:* {}",
        time_str, short_wallet, wallet, amount, live_bal, short_tx_id, tx_id, mined, tx_block, accepting, daa
    )
}

pub async fn process_reward(tx_id: String, address: String, amount_kas: f64, daa_score: u64, state: Arc<AppState>, bot: Bot) {
    sleep(Duration::from_secs(20)).await;

    // 1. Check if Orphaned
    if state.pending_alerts.remove(&tx_id).is_none() {
        return; // It was silently removed
    }

    state.processed_txids.insert(tx_id.clone(), std::time::Instant::now());

    // 2. Fetch Live Balance
    let mut live_bal = 0.0;
    if let Ok(res) = reqwest::get(&format!("https://api.kaspa.org/addresses/{}/balance", address)).await {
        if let Ok(json) = res.json::<Value>().await {
            live_bal = json.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0) / 100_000_000.0;
        }
    }

    let time_str = format!("{} UTC", Utc::now().format("%Y-%m-%d %H:%M:%S"));
    let short_wallet = crate::utils::helpers::format_short_wallet(&address);
    let short_tx = shorten(&tx_id, 8);

    // 3. Build Initial "Searching..." Message
    let initial_msg = build_msg(
        &time_str, &address, &short_wallet, amount_kas, live_bal, &tx_id, &short_tx,
        "⏳ Searching...", "⏳ Searching...", "⏳ Indexing...", daa_score
    );

    // 4. Send Message to Users
    let mut message_refs = Vec::new();
    if let Some(wallet_data) = state.monitored_wallets.get(&address) {
        for chat_id in &wallet_data.value().chat_ids {
            log::info!("[BOT OUT] Chat ID: {} | Msg: Initial Reward Sent", chat_id);
            if let Ok(sent_msg) = bot.send_message(teloxide::types::ChatId(*chat_id), &initial_msg)
                .parse_mode(ParseMode::Markdown)
                .disable_web_page_preview(true)
                .await 
            {
                message_refs.push(MessageRef { chat_id: *chat_id, message_id: sent_msg.id.0 });
            }
        }
    }

    if message_refs.is_empty() { return; }

    state.active_trackers.insert(tx_id.clone(), ActiveTracker { messages: message_refs, daa_score });
    
    log::info!("[PROCESS] NEW REWARD: {} KAS | TX: {} | Sent initial alert, starting background API poll.", amount_kas, tx_id);

    // 5. Start Background Polling for Block Hashes
    tokio::spawn(poll_api_and_edit(tx_id, address, short_wallet, amount_kas, live_bal, time_str, daa_score, short_tx, state, bot));
}

async fn poll_api_and_edit(
    tx_id: String, address: String, short_wallet: String, amount_kas: f64, live_bal: f64, 
    time_str: String, daa_score: u64, short_tx: String, state: Arc<AppState>, bot: Bot
) {
    let client = Client::builder().timeout(Duration::from_secs(5)).build().unwrap();
    let mut attempts = 0;

    loop {
        attempts += 1;
        if attempts > 45 {
            log::info!("[TIMEOUT] Gave up fetching hashes for TX {}", tx_id);
            break;
        }

        sleep(Duration::from_secs(10)).await;
        log::info!("[API POLL] Checking for TX {} (Attempt {})", tx_id, attempts);

        if !state.active_trackers.contains_key(&tx_id) { break; } // Was removed

        if let Ok(res) = client.get(&format!("https://api.kaspa.org/transactions/{}", tx_id)).send().await {
            if let Ok(tx_data) = res.json::<Value>().await {
                if let Some(block_hashes) = tx_data.get("block_hash").and_then(|v| v.as_array()) {
                    if !block_hashes.is_empty() {
                        log::info!("[API SUCCESS] Found Hashes for TX {}", tx_id);
                        
                        let b_hash = block_hashes[0].as_str().unwrap_or("");
                        let a_hash = tx_data.get("accepting_block_hash").and_then(|v| v.as_str()).unwrap_or("");
                        
                        let mut mined_hash = String::new();
                        
                        // Try to fetch mined block from API blocks endpoint
                        if let Ok(block_res) = client.get(&format!("https://api.kaspa.org/blocks/{}", b_hash)).send().await {
                            if let Ok(block_json) = block_res.json::<Value>().await {
                                if let Some(blues) = block_json.get("merge_set_blues_hashes").and_then(|v| v.as_array()) {
                                    // Simplified logic: Grab first blue hash as mined block
                                    if let Some(first_blue) = blues.first().and_then(|v| v.as_str()) {
                                        mined_hash = first_blue.to_string();
                                    }
                                }
                            }
                        }

                        let b_link = format!("[{}](https://kaspa.stream/blocks/{})", shorten(b_hash, 8), b_hash);
                        let a_link = if a_hash.is_empty() { "N/A".to_string() } else { format!("[{}](https://kaspa.stream/blocks/{})", shorten(a_hash, 8), a_hash) };
                        let m_link = if mined_hash.is_empty() { "Not Found".to_string() } else { format!("[{}](https://kaspa.stream/blocks/{})", shorten(&mined_hash, 8), mined_hash) };

                        let final_msg = build_msg(
                            &time_str, &address, &short_wallet, amount_kas, live_bal, &tx_id, &short_tx,
                            &m_link, &b_link, &a_link, daa_score
                        );

                        if let Some(tracker) = state.active_trackers.get(&tx_id) {
                            for m in &tracker.messages {
                                log::info!("[BOT EDIT] Chat ID: {} | Msg ID: {} | Updating Hashes", m.chat_id, m.message_id);
                                let _ = bot.edit_message_text(teloxide::types::ChatId(m.chat_id), teloxide::types::MessageId(m.message_id), &final_msg)
                                    .parse_mode(ParseMode::Markdown)
                                    .disable_web_page_preview(true)
                                    .await;
                            }
                        }
                        
                        // Delete tracker after successful edit to save RAM
                        state.active_trackers.remove(&tx_id);
                        break;
                    }
                }
            }
        }
    }
}
