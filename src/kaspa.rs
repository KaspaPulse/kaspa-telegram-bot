#![allow(deprecated, unused_variables)]
use std::sync::Arc;
use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use serde_json::{json, Value};
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use reqwest::Client;
use std::time::Duration;
use chrono::Utc;

use crate::state::{AppState, ActiveTracker, MessageRef, PendingAlert};
use crate::utils::helpers::format_short_wallet;

// [NEW] Prioritize Local Node for Mined Block Fetch
async fn fetch_local_block(hash: &str, ws_url: &str) -> Option<String> {
    if let Ok((mut ws_stream, _)) = connect_async(ws_url).await {
        let req = json!({ "getBlockRequest": { "hash": hash, "includeTransactions": false } });
        if ws_stream.send(Message::Text(req.to_string())).await.is_ok() {
            if let Some(Ok(Message::Text(res))) = ws_stream.next().await {
                if let Ok(parsed) = serde_json::from_str::<Value>(&res) {
                    if let Some(blues) = parsed.get("getBlockResponse").and_then(|b| b.get("block")).and_then(|b| b.get("verboseData")).and_then(|v| v.get("mergeSetBluesHashes")).and_then(|h| h.as_array()) {
                        if !blues.is_empty() { return blues[0].as_str().map(|s| s.to_string()); }
                    }
                }
            }
        }
    }
    None
}

pub async fn start_kaspa_engine(state: Arc<AppState>, bot: Bot) {
    let ws_url = std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
    let http_client = Client::builder().timeout(Duration::from_secs(5)).build().unwrap();

    loop {
        if !state.is_monitoring.load(std::sync::atomic::Ordering::Relaxed) { tokio::time::sleep(Duration::from_secs(5)).await; continue; }
        log::info!("[NODE] Connecting to Node02 at {}...", ws_url);

        match connect_async(&ws_url).await {
            Ok((mut ws_stream, _)) => {
                log::info!("✅ [NODE] Connected directly to Node02 successfully!");
                state.is_monitoring.store(true, std::sync::atomic::Ordering::Relaxed);

                let addresses: Vec<String> = state.monitored_wallets.iter().map(|kv| kv.key().clone()).collect();
                if !addresses.is_empty() {
                    let sub_req = json!({ "notifyUtxosChangedRequest": { "addresses": addresses } });
                    let _ = ws_stream.send(Message::Text(sub_req.to_string())).await;
                }

                while let Some(msg) = ws_stream.next().await {
                    if let Ok(Message::Text(text)) = msg {
                        if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                            if let Some(notification) = parsed.get("utxosChangedNotification") {
                                handle_utxos_changed(notification, state.clone(), bot.clone(), http_client.clone(), ws_url.clone()).await;
                            }
                        }
                    } else { break; }
                }
            }
            Err(e) => { log::error!("[NODE FATAL] Connection failed: {}", e); }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn handle_utxos_changed(payload: &Value, state: Arc<AppState>, bot: Bot, http_client: Client, ws_url: String) {
    if let Some(removed) = payload.get("removed").and_then(|v| v.as_array()) {
        for entry in removed {
            if let Some(tx_id) = extract_tx_id(entry) {
                if state.pending_alerts.remove(&tx_id).is_some() {}
                else if state.active_trackers.remove(&tx_id).is_some() { state.save_trackers(); }
                state.processed_txids.remove(&tx_id);
            }
        }
    }

    if let Some(added) = payload.get("added").and_then(|v| v.as_array()) {
        for entry in added {
            let tx_id = match extract_tx_id(entry) { Some(id) => id, None => continue };
            let address = match extract_address(entry) { Some(addr) => addr, None => continue };
            if !state.monitored_wallets.contains_key(&address) { continue; }

            let amount_sompi = match extract_amount(entry) { Some(amt) => amt, None => continue };
            let daa_score = extract_daa_score(entry).unwrap_or(0);

            if state.processed_txids.contains(&tx_id) {
                if let Some(mut tracker) = state.active_trackers.get_mut(&tx_id) { tracker.daa_score = daa_score; }
                continue;
            }
            if let Some(mut pending) = state.pending_alerts.get_mut(&tx_id) { pending.daa_score = daa_score; continue; }

            state.pending_alerts.insert(tx_id.clone(), PendingAlert { daa_score });

            let state_clone = state.clone(); let bot_clone = bot.clone(); let http_clone = http_client.clone();
            let tx_id_clone = tx_id.clone(); let address_clone = address.clone(); let ws_clone = ws_url.clone();

            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(20)).await;
                state_clone.processed_txids.insert(tx_id_clone.clone());
                let final_daa = match state_clone.pending_alerts.remove(&tx_id_clone) { Some(alert) => alert.1.daa_score, None => return };

                let exact_reward = amount_sompi / 100_000_000.0;
                let mut live_bal = 0.0;
                if let Some(mut wallet) = state_clone.monitored_wallets.get_mut(&address_clone) {
                    live_bal = wallet.last_balance + exact_reward;
                    wallet.last_balance = live_bal;
                }
                state_clone.save_wallets();

                let dt_str = format!("{} UTC", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                let short_wallet = format_short_wallet(&address_clone);
                let tx_link = format!("https://kaspa.stream/transactions/{}", tx_id_clone);
                let short_tx_id = format!("{}...{}", &tx_id_clone[0..8], &tx_id_clone[tx_id_clone.len()-8..]);

                let build_msg = |b_hash: &str, a_hash: &str, m_hash: &str| -> String {
                    format!("⚡ *Native Node Reward!* 💎\n━━━━━━━━━━━━━━━━━━\n*Time:* `{}`\n*Wallet:* [{}]({})\n*Amount:* `+{:.8} KAS`\n*Live Balance:* `{:.8} KAS`\n━━━━━━━━━━━━━━━━━━\n*TXID:* [{}]({})\n*Mined Block:* {}\n*TX Block:* {}\n*Accepting Block:* {}\n*DAA Score:* `{}`", dt_str, short_wallet, format!("https://kaspa.stream/addresses/{}", address_clone), exact_reward, live_bal, short_tx_id, tx_link, m_hash, b_hash, a_hash, final_daa)
                };

                let initial_msg = build_msg("⏳ `Searching...`", "⏳ `Indexing...`", "⏳ `Searching...`");
                let subscribers = match state_clone.monitored_wallets.get(&address_clone) { Some(w) => w.chat_ids.clone(), None => return };
                if subscribers.is_empty() { return; }

                let mut active_tracker = ActiveTracker { messages: Vec::new(), daa_score: final_daa };
                for chat_id in subscribers {
                    if let Ok(sent_msg) = bot_clone.send_message(ChatId(chat_id), &initial_msg).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await {
                        active_tracker.messages.push(MessageRef { chat_id: chat_id, message_id: sent_msg.id.0 });
                    }
                }
                state_clone.active_trackers.insert(tx_id_clone.clone(), active_tracker);
                state_clone.save_trackers();

                poll_api_for_hashes(state_clone, bot_clone, http_clone, tx_id_clone, ws_clone, build_msg).await;
            });
        }
    }
}

async fn poll_api_for_hashes<F>(state: Arc<AppState>, bot: Bot, http_client: Client, tx_id: String, ws_url: String, build_msg: F) 
where F: Fn(&str, &str, &str) -> String 
{
    let mut attempts = 0;
    loop {
        attempts += 1; tokio::time::sleep(Duration::from_secs(10)).await;
        if !state.active_trackers.contains_key(&tx_id) { break; }

        let api_url = format!("https://api.kaspa.org/transactions/{}", tx_id);
        if let Ok(res) = http_client.get(&api_url).send().await {
            if let Ok(tx_data) = res.json::<Value>().await {
                if let Some(block_hashes) = tx_data.get("block_hash").and_then(|v| v.as_array()) {
                    if !block_hashes.is_empty() {
                        let b_hash_full = block_hashes[0].as_str().unwrap_or("");
                        let b_hash_display = format!("{}...{}", &b_hash_full[0..8], &b_hash_full[b_hash_full.len()-8..]);
                        let b_hash_link = format!("[{}](https://kaspa.stream/blocks/{})", b_hash_display, b_hash_full);

                        let mut a_hash_link = "`N/A`".to_string();
                        if let Some(a_hash_full) = tx_data.get("accepting_block_hash").and_then(|v| v.as_str()) {
                            let a_hash_display = format!("{}...{}", &a_hash_full[0..8], &a_hash_full[a_hash_full.len()-8..]);
                            a_hash_link = format!("[{}](https://kaspa.stream/blocks/{})", a_hash_display, a_hash_full);
                        }

                        // [NEW] Attempt Local Fetch First, fallback to API
                        let mut mined_hash_link = "`Not Found`".to_string();
                        if let Some(local_mined) = fetch_local_block(b_hash_full, &ws_url).await {
                             let m_hash_disp = format!("{}...{}", &local_mined[0..8], &local_mined[local_mined.len()-8..]);
                             mined_hash_link = format!("[{}](https://kaspa.stream/blocks/{})", m_hash_disp, local_mined);
                        } else {
                            let block_url = format!("https://api.kaspa.org/blocks/{}", b_hash_full);
                            if let Ok(b_res) = http_client.get(&block_url).send().await {
                                if let Ok(b_data) = b_res.json::<Value>().await {
                                    if let Some(blues) = b_data.get("merge_set_blues_hashes").and_then(|v| v.as_array()) {
                                        if !blues.is_empty() {
                                            let mined_hash_full = blues[0].as_str().unwrap_or("");
                                            let m_hash_disp = format!("{}...{}", &mined_hash_full[0..8], &mined_hash_full[mined_hash_full.len()-8..]);
                                            mined_hash_link = format!("[{}](https://kaspa.stream/blocks/{})", m_hash_disp, mined_hash_full);
                                        }
                                    }
                                }
                            }
                        }

                        let updated_msg = build_msg(&b_hash_link, &a_hash_link, &mined_hash_link);
                        if let Some(tracker) = state.active_trackers.get(&tx_id) {
                            for m in &tracker.messages {
                                let _ = bot.edit_message_text(ChatId(m.chat_id), teloxide::types::MessageId(m.message_id), &updated_msg).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await;
                            }
                        }
                        break;
                    }
                }
            }
        }

        if attempts >= 45 {
            let failed_msg = build_msg("❌ `Timeout`", "❌ `Timeout`", "❌ `Timeout`");
            if let Some(tracker) = state.active_trackers.get(&tx_id) {
                for m in &tracker.messages {
                    let _ = bot.edit_message_text(ChatId(m.chat_id), teloxide::types::MessageId(m.message_id), &failed_msg).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await;
                }
            }
            break;
        }
    }
}

fn extract_tx_id(entry: &Value) -> Option<String> { entry.get("outpoint").and_then(|o| o.get("transactionId").or(o.get("transaction_id")).or(o.get("id"))).and_then(|v| v.as_str().map(|s| s.to_string())) }
fn extract_address(entry: &Value) -> Option<String> { let addr_val = entry.get("address")?; let payload = addr_val.get("payload").or(Some(addr_val)).and_then(|v| v.as_str())?; Some(if !payload.starts_with("kaspa:") { format!("kaspa:{}", payload) } else { payload.to_string() }) }
fn extract_amount(entry: &Value) -> Option<f64> { entry.get("utxoEntry").and_then(|u| u.get("amount").or(u.get("amount_sompi"))).and_then(|v| v.as_f64().or_else(|| v.as_str().unwrap_or("0").parse::<f64>().ok())) }
fn extract_daa_score(entry: &Value) -> Option<u64> { entry.get("utxoEntry").and_then(|u| u.get("blockDaaScore").or(u.get("block_daa_score"))).and_then(|v| v.as_u64()) }

