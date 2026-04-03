#![allow(deprecated)]
use std::sync::Arc;
use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use serde_json::{json, Value};
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::{mpsc, Semaphore};
use std::time::Duration;
use chrono::Utc;

use crate::state::AppState;
use crate::utils::helpers::format_short_wallet;
use crate::api::ApiManager;
use crate::utils::types::{Sompi, Kaspa};

#[derive(Debug, Clone)]
pub struct UtxoEvent {
    pub tx_id: String,
    pub address: String,
    pub amount: Sompi,
    pub daa_score: u64,
}

#[tracing::instrument(skip(state, bot, api))]
pub async fn start_kaspa_engine(state: Arc<AppState>, bot: Bot, api: Arc<ApiManager>) {
    let ws_url = std::env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
    
    let (tx_event, mut rx_event) = mpsc::channel::<UtxoEvent>(5000);
    let api_semaphore = Arc::new(Semaphore::new(10));

    let worker_state = state.clone();
    let worker_bot = bot.clone();
    let worker_api = api.clone();
    let worker_shutdown = state.shutdown_token.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = worker_shutdown.cancelled() => { break; }
                event_opt = rx_event.recv() => {
                    if let Some(event) = event_opt {
                        let sem_clone = api_semaphore.clone();
                        let s_clone = worker_state.clone();
                        let b_clone = worker_bot.clone();
                        let a_clone = worker_api.clone();
                        
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(20)).await;
                            let _permit = match sem_clone.acquire().await { Ok(p) => p, Err(_) => return };
                            process_reward_event(event, s_clone, b_clone, a_clone).await;
                        });
                    }
                }
            }
        }
    });

    loop {
        if state.shutdown_token.is_cancelled() { return; }
        if !state.is_monitoring.load(std::sync::atomic::Ordering::Relaxed) { 
            tokio::time::sleep(Duration::from_secs(5)).await; 
            continue; 
        }
        
        tracing::info!("[NODE] Connecting to wRPC at {}...", ws_url);

        let mut request = match ws_url.clone().into_client_request() {
            Ok(req) => req,
            Err(_) => { tokio::time::sleep(Duration::from_secs(5)).await; continue; }
        };
        if let Ok(header_val) = "wrpc-json".parse() { request.headers_mut().insert("sec-websocket-protocol", header_val); }

        match connect_async(request).await {
            Ok((mut ws_stream, _)) => {
                tracing::info!("✅ [NODE] Connected! Handshaking...");
                let mut last_wallet_count = 0;
                let mut sub_interval = tokio::time::interval(Duration::from_secs(10));

                loop {
                    tokio::select! {
                        _ = sub_interval.tick() => {
                            let current_count = state.monitored_wallets.len();
                            if current_count != last_wallet_count {
                                last_wallet_count = current_count;
                                let addrs: Vec<String> = state.monitored_wallets.iter().map(|kv| kv.key().clone()).collect();
                                
                                // [CRITICAL FIX]: Added "id": 1 to prevent Node from dropping the connection!
                                let sub_req = json!({
                                    "id": 1, 
                                    "notifyUtxosChangedRequest": {
                                        "addresses": addrs
                                    }
                                });
                                
                                let _ = ws_stream.send(Message::Text(sub_req.to_string())).await;
                                tracing::info!("🔄 [NODE] Requested Subscription for {} wallets.", current_count);
                            }
                        }
                        _ = state.shutdown_token.cancelled() => {
                            let _ = ws_stream.close(None).await;
                            return;
                        }
                        msg = ws_stream.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                                        
                                        // 1. Process Live Reward Notifications
                                        if let Some(notification) = parsed.get("utxosChangedNotification") {
                                            parse_utxos_and_queue(notification, &state, &tx_event).await;
                                        } 
                                        // 2. Process Subscription Acknowledgements
                                        else if let Some(response) = parsed.get("notifyUtxosChangedResponse") {
                                            if let Some(err) = response.get("error") {
                                                if !err.is_null() {
                                                    tracing::error!("❌ [NODE RPC ERROR]: {}", err);
                                                    last_wallet_count = 0; // Force retry
                                                }
                                            } else {
                                                tracing::info!("✅ [NODE ACK] UTXO Subscription Active!");
                                            }
                                        }
                                        // 3. General Errors
                                        else if let Some(err) = parsed.get("error") {
                                            tracing::error!("❌ [NODE ERROR]: {}", err);
                                        }
                                    }
                                }
                                Some(Ok(Message::Close(c))) => { 
                                    tracing::warn!("⚠️ [NODE] Connection closed by server: {:?}", c); 
                                    break; 
                                }
                                Some(Err(e)) => { 
                                    tracing::error!("❌ [NODE] WebSocket Error: {}", e); 
                                    break; 
                                }
                                None => break,
                                _ => {}
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("❌ [NODE FATAL] Could not connect: {}", e);
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn parse_utxos_and_queue(payload: &Value, state: &Arc<AppState>, tx: &mpsc::Sender<UtxoEvent>) {
    if let Some(added) = payload.get("added").and_then(|v: &Value| v.as_array()) {
        for entry in added {
            let tx_id = match extract_tx_id(entry) { Some(id) => id, None => continue };
            let address = match extract_address(entry) { Some(addr) => addr, None => continue };
            
            if !state.monitored_wallets.contains_key(&address) { continue; }
            
            let amount = match extract_amount(entry) { Some(amt) => amt, None => continue };
            let daa_score = extract_daa_score(entry).unwrap_or(0);
            
            if state.processed_txids.contains_key(&tx_id) { continue; }
            state.processed_txids.insert(tx_id.clone(), std::time::Instant::now());

            tracing::info!("💎 [EVENT] Valid Reward Queued: {} KAS for {}", Kaspa::from(amount), address);
            let _ = tx.try_send(UtxoEvent { tx_id, address, amount, daa_score });
        }
    }
}

macro_rules! build_reward_msg {
    ($b_hash:expr, $a_hash:expr, $m_hash:expr, $dt:expr, $s_wal:expr, $addr:expr, $amt:expr, $bal:expr, $s_tx:expr, $tx_link:expr, $daa:expr) => {
        format!("⚡ *Native Node Reward!* 💎\n━━━━━━━━━━━━━━━━━━\n*Time:* `{}`\n*Wallet:* [{}]({})\n*Amount:* `+{} KAS`\n*Live Balance:* `{} KAS`\n━━━━━━━━━━━━━━━━━━\n*TXID:* [{}]({})\n*Mined Block:* {}\n*TX Block:* {}\n*Accepting Block:* {}\n*DAA Score:* `{}`", 
        $dt, $s_wal, format!("https://kaspa.stream/addresses/{}", $addr), $amt, $bal, $s_tx, $tx_link, $m_hash, $b_hash, $a_hash, $daa)
    }
}

async fn process_reward_event(event: UtxoEvent, state: Arc<AppState>, bot: Bot, api: Arc<ApiManager>) {
    let exact_reward: Kaspa = event.amount.into();
    let mut live_bal = Kaspa(0.0);

    if let Ok(Ok(bal)) = tokio::time::timeout(Duration::from_secs(3), api.get_balance(&event.address)).await {
        if bal > 0.0 { live_bal = Kaspa(bal); }
    }
    
    if let Some(mut wallet) = state.monitored_wallets.get_mut(&event.address) {
        if live_bal.0 == 0.0 { live_bal = Kaspa(wallet.last_balance + exact_reward.0); }
        wallet.last_balance = live_bal.0;
    }
    state.sync_wallet_to_db(&event.address).await;

    let dt_str = format!("{} UTC", Utc::now().format("%Y-%m-%d %H:%M:%S"));
    let short_wallet = format_short_wallet(&event.address);
    let full_address = event.address.clone();
    let tx_id = event.tx_id.clone();
    let short_tx_id = format!("{}...{}", &tx_id[..8], &tx_id[tx_id.len()-8..]);
    let tx_link = format!("https://kaspa.stream/transactions/{}", tx_id);

    let initial_msg = build_reward_msg!("⏳ `Searching...`", "⏳ `Indexing...`", "⏳ `Searching...`", dt_str, short_wallet, full_address, exact_reward, live_bal, short_tx_id, tx_link, event.daa_score);

    tracing::info!("[PROCESS] NEW REWARD: {} KAS | TX: {} | Sent initial alert, starting API poll.", exact_reward, tx_id);

    let subscribers = match state.monitored_wallets.get(&event.address) { Some(w) => w.chat_ids.clone(), None => return };

    for chat_id in subscribers {
        tokio::time::sleep(Duration::from_millis(40)).await;
        
        if let Ok(sent_msg) = bot.send_message(ChatId(chat_id), &initial_msg).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await {
            let msg_id = sent_msg.id;
            let b_clone = bot.clone();
            let a_clone = api.clone();
            let t_id = tx_id.clone();
            let addr = full_address.clone();
            let dt = dt_str.clone();
            let s_wal = short_wallet.clone();
            let s_tx = short_tx_id.clone();
            let t_link = tx_link.clone();
            let d_score = event.daa_score;

            tokio::spawn(async move {
                let mut attempts = 0;
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                
                loop {
                    interval.tick().await;
                    attempts += 1;
                    tracing::info!("[API POLL] Checking TX {} (Attempt {})", t_id, attempts);

                    if attempts > 45 {
                        tracing::warn!("[TIMEOUT] Gave up on TX {}", t_id);
                        let failed_msg = build_reward_msg!("❌ `Timeout`", "❌ `Timeout`", "❌ `Timeout`", dt, s_wal, addr, exact_reward, live_bal, s_tx, t_link, d_score);
                        let _ = b_clone.edit_message_text(ChatId(chat_id), msg_id, failed_msg).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await;
                        break;
                    }

                    if let Ok(tx_res) = a_clone.client.get(&format!("https://api.kaspa.org/transactions/{}", t_id)).send().await {
                        if let Ok(tx_data) = tx_res.json::<Value>().await {
                            if let Some(b_hash_arr) = tx_data.get("block_hash").and_then(|v: &Value| v.as_array()) {
                                if let Some(b_hash_full) = b_hash_arr.get(0).and_then(|v: &Value| v.as_str()) {
                                    let b_hash_display = format!("{}...{}", &b_hash_full[..8], &b_hash_full[b_hash_full.len()-8..]);
                                    let b_hash_link = format!("[{}](https://kaspa.stream/blocks/{})", b_hash_display, b_hash_full);

                                    let mut a_hash_link = "`N/A`".to_string();
                                    if let Some(a_hash) = tx_data.get("accepting_block_hash").and_then(|v: &Value| v.as_str()) {
                                        let a_hash_display = format!("{}...{}", &a_hash[..8], &a_hash[a_hash.len()-8..]);
                                        a_hash_link = format!("[{}](https://kaspa.stream/blocks/{})", a_hash_display, a_hash);
                                    }

                                    let mut mined_hash_link = "`Not Found`".to_string();
                                    if let Ok(block_res) = a_clone.client.get(&format!("https://api.kaspa.org/blocks/{}", b_hash_full)).send().await {
                                        if let Ok(block_data) = block_res.json::<Value>().await {
                                            if let Some(blues) = block_data.get("merge_set_blues_hashes").and_then(|v: &Value| v.as_array()) {
                                                let mut idx = 0;
                                                if let Some(outputs) = tx_data.get("outputs").and_then(|v: &Value| v.as_array()) {
                                                    for (i, out) in outputs.iter().enumerate() {
                                                        if out.get("script_public_key_address").and_then(|v: &Value| v.as_str()) == Some(addr.as_str()) {
                                                            idx = i; break;
                                                        }
                                                    }
                                                }
                                                if let Some(mined_hash_val) = blues.get(idx).or_else(|| blues.get(0)) {
                                                    if let Some(mined_hash) = mined_hash_val.as_str() {
                                                        let m_hash_display = format!("{}...{}", &mined_hash[..8], &mined_hash[mined_hash.len()-8..]);
                                                        mined_hash_link = format!("[{}](https://kaspa.stream/blocks/{})", m_hash_display, mined_hash);
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    let updated_msg = build_reward_msg!(&b_hash_link, &a_hash_link, &mined_hash_link, dt, s_wal, addr, exact_reward, live_bal, s_tx, t_link, d_score);
                                    let _ = b_clone.edit_message_text(ChatId(chat_id), msg_id, updated_msg).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await;
                                    break;
                                }
                            }
                        }
                    }
                }
            });
        }
    }
}

fn extract_tx_id(entry: &Value) -> Option<String> {
    let outpoint = entry.get("outpoint")?;
    outpoint.get("transactionId").or(outpoint.get("transaction_id")).and_then(|v: &Value| v.as_str().map(|s| s.to_string()))
}

fn extract_address(entry: &Value) -> Option<String> {
    if let Some(addr) = entry.get("address") {
        if let Some(s) = addr.as_str() { return Some(s.to_string()); }
        if let Some(payload) = addr.get("payload").and_then(|v: &Value| v.as_str()) {
            let prefix = addr.get("prefix").and_then(|v: &Value| v.as_str()).unwrap_or("kaspa");
            return Some(format!("{}:{}", prefix, payload));
        }
    }
    None
}

fn extract_amount(entry: &Value) -> Option<Sompi> {
    let utxo = entry.get("utxoEntry")?;
    let val = utxo.get("amount").or(utxo.get("amount_sompi"))?;
    if let Some(n) = val.as_u64() { Some(Sompi(n)) }
    else if let Some(s) = val.as_str() { s.parse::<u64>().ok().map(Sompi) }
    else { None }
}

fn extract_daa_score(entry: &Value) -> Option<u64> {
    let val = entry.get("utxoEntry")?.get("blockDaaScore")?;
    if let Some(n) = val.as_u64() { Some(n) }
    else if let Some(s) = val.as_str() { s.parse::<u64>().ok() }
    else { None }
}
