#![deny(warnings)]
mod state;
mod utils;
mod commands;

use std::collections::{HashSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use dashmap::DashMap;
use teloxide::prelude::*;
use teloxide::RequestError;
use kaspa_wrpc_client::{KaspaRpcClient, WrpcEncoding};
use kaspa_consensus_core::network::NetworkId;
 
use kaspa_rpc_core::api::rpc::RpcApi; 
use kaspa_addresses::Address;
use std::str::FromStr;
use dotenvy::dotenv;
use std::env;
use tokio::fs;
use tokio::time::{sleep, Duration};
use tracing::{info, warn, error}; 

use chrono::Utc;

use crate::state::{SharedState, UtxoState};
use crate::utils::{format_short_wallet, format_hash};
use crate::commands::{Command, handle_command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _guard = crate::utils::init_enterprise_telemetry();
    // [CORE] Initialize Enterprise Telemetry Engine


    dotenv().ok();
    let file_appender = tracing_appender::rolling::never(".", "bot.log");
    let (_non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
// [PURGED BY SCRIPT]     tracing_subscriber::fmt().with_writer(non_blocking.and(std::io::stdout)).with_ansi(false).init();

    info!("[INIT] Secure Modular Rust Engine Started - Node02 - Jeddah.");

    let state: SharedState = Arc::new(DashMap::new());
    let utxo_state: UtxoState = Arc::new(DashMap::new());
    let is_monitoring = Arc::new(AtomicBool::new(true));
    let admin_id: i64 = env::var("ADMIN_ID").unwrap_or_else(|_| "0".to_string()).parse().unwrap_or(0);

    if let Ok(data) = fs::read_to_string("wallets.json").await {
        if let Ok(parsed) = serde_json::from_str::<HashMap<String, HashSet<i64>>>(&data) {
            for (k, v) in parsed { if k.len() > 20 { state.insert(k, v); } }
            info!("[DATA] Wallets loaded: {}", state.len());
        }
    }

    let bot_token = env::var("BOT_TOKEN").expect("BOT_TOKEN missing!");
    let bot = Bot::new(bot_token);

    let ws_url = env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:18110".to_string());
    let network_id = NetworkId::from_str("mainnet").unwrap();
    let client_result = KaspaRpcClient::new(WrpcEncoding::SerdeJson, Some(&ws_url), None, Some(network_id), None);
    let rpc_client = client_result.expect("RPC Init failed!");
    let shared_rpc = Arc::new(rpc_client);

    let rpc_for_bg = Arc::clone(&shared_rpc);
    tokio::spawn(async move {
        let _ = rpc_for_bg.connect(None).await;
        loop {
            sleep(Duration::from_secs(30)).await;
            if rpc_for_bg.get_server_info().await.is_err() { 
                warn!("[RPC] Reconnecting to Node02...");
                let _ = rpc_for_bg.connect(None).await; 
            }
        }
    });

    let alert_rpc = Arc::clone(&shared_rpc);
    let alert_state = Arc::clone(&state);
    let alert_utxos = Arc::clone(&utxo_state);
    let alert_bot = bot.clone();
    let alert_monitoring = Arc::clone(&is_monitoring);

    tokio::spawn(async move {
        sleep(Duration::from_secs(5)).await;
        loop {
            sleep(Duration::from_secs(10)).await; 
            if !alert_monitoring.load(Ordering::Relaxed) { continue; }
            let check_list: Vec<(String, HashSet<i64>)> = alert_state.iter().map(|e| (e.key().clone(), e.value().clone())).collect();
            for (wallet, subs) in check_list {
                if let Ok(addr) = Address::try_from(wallet.as_str()) {
                    if let Ok(utxos) = alert_rpc.get_utxos_by_addresses(vec![addr.clone()]).await {
                        let mut current_outpoints = HashSet::new();
                        let mut new_rewards = Vec::new();
                        let mut known = alert_utxos.entry(wallet.clone()).or_insert_with(HashSet::new);
                        let is_first_run = known.is_empty();
                        for entry in utxos {
                            let tx_id = entry.outpoint.transaction_id.to_string();
                            let outpoint_id = format!("{}:{}", tx_id, entry.outpoint.index);
                            current_outpoints.insert(outpoint_id.clone());
                            if !is_first_run && !known.contains(&outpoint_id) {
                                new_rewards.push((tx_id, entry.utxo_entry.amount as f64 / 1e8, entry.utxo_entry.block_daa_score));
                                known.insert(outpoint_id);
                            } else if is_first_run { known.insert(outpoint_id); }
                        }
                        known.retain(|k| current_outpoints.contains(k));
                        
                        for (tx_id, diff, daa_score) in new_rewards {
                            let live_bal = alert_rpc.get_balance_by_address(addr.clone()).await.unwrap_or(0) as f64 / 1e8;
                            let time_str = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
                            let short_wallet = format_short_wallet(&wallet);
                            
                            info!("--------------------------------------------------");
                            info!("[REWARD] Wallet: {}", wallet);
                            info!("[REWARD] Amount: +{:.8} KAS", diff);
                            info!("[REWARD] TXID  : {}", tx_id);
                            info!("--------------------------------------------------");

                            let initial_msg = format!("⚡ <b>Native Node Reward!</b> ðŸ’Ž\nâ”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”\n<b>Time:</b> <code>{}</code>\n<b>Wallet:</b> <a href=\"https://kaspa.stream/addresses/{}\">{}</a>\n<b>Amount:</b> <code>+{:.8} KAS</code>\n<b>Live Balance:</b> <code>{:.8} KAS</code>\nâ”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”\n<b>TXID:</b> {}\n<b>Mined Block:</b> â³ <code>Searching...</code>\n<b>TX Block:</b> â³ <code>Indexing...</code>\n<b>Accepting Block:</b> â³ <code>Indexing...</code>\n<b>DAA Score:</b> <code>{}</code>", time_str, wallet, short_wallet, diff, live_bal, format_hash(&tx_id, "transactions"), daa_score);
                            let mut s_ids = Vec::new();
                            for s in subs.clone() { 
                                if let Ok(m) = alert_bot.send_message(ChatId(s), &initial_msg).parse_mode(teloxide::types::ParseMode::Html).await { 
                                    s_ids.push((ChatId(s), m.id)); 
                                } 
                            }
                            
                            let (f_tx, w_cl, bot_cl, rpc_cl) = (tx_id.clone(), wallet.clone(), alert_bot.clone(), Arc::clone(&alert_rpc));
                            tokio::spawn(async move {
                                sleep(Duration::from_secs(12)).await;
                                let (mut b_h, mut a_h, mut m_h) = ("Not Found".to_string(), "Not Found".to_string(), "Not Found".to_string());
                                
                                                                  for attempt in 1..=6 {
                                      tracing::info!("[DEBUG] Resolving hashes for TX: {} (Attempt {})", f_tx, attempt);
                                      if let Ok(r) = reqwest::get(format!("https://api.kaspa.org/transactions/{}", f_tx)).await {
                                          if let Ok(j) = r.json::<serde_json::Value>().await {
                                              if let Some(ba) = j["block_hash"].as_array() {
                                                  if !ba.is_empty() { b_h = ba[0].as_str().unwrap_or("Not Found").to_string(); }
                                              }
                                              a_h = j["accepting_block_hash"].as_str().unwrap_or("Not Found").to_string();

                                              if b_h != "Not Found" {
                                                  if let Ok(h) = kaspa_consensus_core::Hash::from_str(&b_h) {
                                                      // 1. Direct Node Query for the TX Block
                                                      if let Ok(bl) = rpc_cl.get_block(h, true).await {
                                                          let tx_block = bl;
                                                          let blues: Vec<String> = tx_block.verbose_data.as_ref().map(|v| v.merge_set_blues_hashes.iter().map(|h| h.to_string()).collect()).unwrap_or_default();
                                                          
                                                          tracing::info!("[DEBUG] Node returned {} blue blocks.", blues.len());

                                                          // 2. Locate Target Wallet Index
                                                          let mut target_index: Option<usize> = None;
                                                          for tx in tx_block.transactions {
                                                              if tx.verbose_data.as_ref().map(|v| v.transaction_id.to_string()) == Some(f_tx.clone()) {
                                                                  if let Ok(target_addr) = kaspa_addresses::Address::try_from(w_cl.as_str()) {
                                                                      target_index = tx.outputs.iter().position(|out| {
                                                                          out.verbose_data.as_ref().map(|v| v.script_public_key_address.to_string()) == Some(target_addr.to_string())
                                                                      });
                                                                  }
                                                                  break;
                                                              }
                                                          }

                                                          // 3. Safe Index Matching against Merge Set
                                                          if !blues.is_empty() {
                                                              let safe_idx = target_index.filter(|&i| i < blues.len()).unwrap_or(0);
                                                              if let Some(mined_hash) = blues.get(safe_idx) {
                                                                  m_h = mined_hash.to_string();
                                                                  tracing::info!("[MATCH SUCCESS] Correct Mined Block located via Node Indexing: {}", m_h);
                                                              } else {
                                                                  m_h = blues[0].to_string();
                                                                  tracing::warn!("[MATCH WARNING] Safe index failed, using first block fallback.");
                                                              }
                                                          }
                                                      }
                                                  }
                                                  break; // Important: Exit the attempt loop once successful
                                              }
                                          }
                                      }
                                      tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                                  }
                                
                                let up_msg = format!("⚡ <b>Native Node Reward!</b> ðŸ’Ž\nâ”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”\n<b>Time:</b> <code>{}</code>\n<b>Wallet:</b> <a href=\"https://kaspa.stream/addresses/{}\">{}</a>\n<b>Amount:</b> <code>+{:.8} KAS</code>\n<b>Live Balance:</b> <code>{:.8} KAS</code>\nâ”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”\n<b>TXID:</b> {}\n<b>Mined Block:</b> {}\n<b>TX Block:</b> {}\n<b>Accepting Block:</b> {}\n<b>DAA Score:</b> <code>{}</code>", time_str, w_cl, format_short_wallet(&w_cl), diff, live_bal, format_hash(&f_tx, "transactions"), format_hash(&m_h, "blocks"), format_hash(&b_h, "blocks"), format_hash(&a_h, "blocks"), daa_score);
                                for (ci, mi) in s_ids { let _ = bot_cl.edit_message_text(ci, mi, &up_msg).parse_mode(teloxide::types::ParseMode::Html).await; }
                            });
                        }
                    }
                }
            }
        }
    });

    let repl_state = Arc::clone(&state);
    let repl_rpc = Arc::clone(&shared_rpc);
    let repl_monitoring = Arc::clone(&is_monitoring);

    Command::repl(bot.clone(), move |bot: Bot, msg: Message, cmd: Command| {
        let state_cl = Arc::clone(&repl_state);
        let rpc_cl = Arc::clone(&repl_rpc); 
        let monitoring_cl = Arc::clone(&repl_monitoring);
        async move {
            if let Err(e) = handle_command(bot, msg, cmd, state_cl, rpc_cl, monitoring_cl, admin_id).await {
                error!("[CMD ERROR] {}", e);
            }
            Ok::<(), RequestError>(())
        }
    }).await;
    
    Ok(())
}
