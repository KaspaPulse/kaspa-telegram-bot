use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use teloxide::{prelude::*, types::ChatId, utils::command::BotCommands};
use kaspa_wrpc_client::KaspaRpcClient;
use kaspa_rpc_core::api::rpc::RpcApi; 
use kaspa_addresses::Address;
use tokio::time::{sleep, Duration};
use sysinfo::System;
use regex::Regex;
use tracing::{info, error}; 
use rev_lines::RevLines;
use std::io::BufReader;

use crate::state::{SharedState, save_state};
use crate::utils::{f_num, make_golden_keyboard, send_and_log, format_short_wallet};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Kaspa Node Bot Commands:")]
pub enum Command {
    #[command(description = "Start the bot and show help.")] Start,
    #[command(description = "Add a wallet: /add <address>")] Add(String),
    #[command(description = "Remove a wallet: /remove <address>")] Remove(String),
    #[command(description = "List tracked wallets.")] List,
    #[command(description = "Show network stats.")] Network,
    #[command(description = "Show BlockDAG details.")] Dag,
    #[command(description = "Check Live Balance.")] Balance,
    #[command(description = "Check KAS Price.")] Price,
    #[command(description = "Check Market Cap.")] Market,
    #[command(description = "Check Supply.")] Supply,
    #[command(description = "Check Mempool Fees.")] Fees,
    #[command(description = "Admin Command")] Sys,
    #[command(description = "Admin Command")] Pause,
    #[command(description = "Admin Command")] Resume,
    #[command(description = "Admin Command")] Restart,
    #[command(description = "Admin Command")] Broadcast(String),
    #[command(description = "Admin Command")] Logs,
}

pub async fn handle_command(
    bot: Bot, 
    msg: Message, 
    cmd: Command, 
    state: SharedState, 
    rpc: Arc<KaspaRpcClient>, 
    monitoring: Arc<AtomicBool>, 
    admin_id: i64
) -> anyhow::Result<()> {
    let chat_id = msg.chat.id.0;
    info!("[CMD IN] User: {} | Msg: {}", chat_id, msg.text().unwrap_or(""));
    let is_admin = chat_id == admin_id;
    
    match cmd {
        Command::Start => { 
            let mut msg_text = "🤖 <b>Kaspa Platinum Command Center</b>\n━━━━━━━━━━━━━━━━━━\n\
            Welcome! This system provides secure, real-time Kaspa wallet monitoring directly via a private node.\n\n\
            📌 <b>Public Commands:</b>\n\
            <code>/add &lt;address&gt;</code> - Track a wallet\n\
            <code>/remove &lt;address&gt;</code> - Stop tracking\n\
            <code>/list</code> - View your portfolio\n\
            <code>/balance</code> - Check live balances\n\
            <code>/network</code> - Network statistics\n\
            <code>/fees</code> - Live mempool fees\n\
            <code>/price</code> - Current KAS price\n\
            <code>/supply</code> - Circulating supply\n\
            <code>/market</code> - Market capitalization\n\
            <code>/dag</code> - BlockDAG details".to_string();

            if is_admin {
                msg_text.push_str("\n\n👑 <b>Admin Commands:</b>\n\
                <code>/sys</code> - Server Diagnostics\n\
                <code>/pause</code> - Disconnect Engine\n\
                <code>/resume</code> - Reconnect Engine\n\
                <code>/restart</code> - Reboot Process\n\
                <code>/broadcast &lt;msg&gt;</code> - Message all users\n\
                <code>/logs</code> - View System Logs");
            }

            let _ = send_and_log(&bot, msg.chat.id, msg_text, Some(make_golden_keyboard())).await; 
        }
        Command::Add(w) => {
            let c = if w.starts_with("kaspa:") { w.clone() } else { format!("kaspa:{}", w) };
            if Regex::new(r"^kaspa:[a-z0-9]{61,63}$").unwrap().is_match(&c) {
                state.entry(c.clone()).or_insert_with(HashSet::new).insert(chat_id);
                save_state(&state).await; 
                let _ = send_and_log(&bot, msg.chat.id, format!("✅ <b>Tracking Enabled:</b>\n<code>{}</code>", c), Some(make_golden_keyboard())).await;
            } else { 
                let _ = send_and_log(&bot, msg.chat.id, "❌ <b>Invalid Format!</b>\nPlease provide a valid Kaspa address.".to_string(), None).await; 
            }
        }
        Command::Remove(w) => {
            let c = if w.starts_with("kaspa:") { w.clone() } else { format!("kaspa:{}", w) };
            if let Some(mut subs) = state.get_mut(&c) { subs.remove(&chat_id); }
            save_state(&state).await; 
            let _ = send_and_log(&bot, msg.chat.id, "🗑️ <b>Wallet Removed.</b>".to_string(), None).await;
        }
        Command::List => {
            let my: Vec<String> = state.iter().filter(|e| e.value().contains(&chat_id)).map(|e| e.key().clone()).collect();
            let text = if my.is_empty() { "No wallets tracked.".to_string() } else { format!("📁 <b>Portfolio:</b>\n<code>{}</code>", my.join("\n")) };
            let _ = send_and_log(&bot, msg.chat.id, text, None).await;
        }
        Command::Balance => {
            let mut total = 0.0; let mut text = "🏦 <b>Live Balances:</b>\n".to_string();
            for e in state.iter().filter(|e| e.value().contains(&chat_id)) {
                if let Ok(a) = Address::try_from(e.key().as_str()) {
                    if let Ok(b) = rpc.get_balance_by_address(a).await {
                        let k = b as f64 / 1e8; total += k;
                        text.push_str(&format!("💳 <code>{}</code>\n⚖️ <b>{} KAS</b>\n\n", format_short_wallet(e.key()), f_num(k)));
                    }
                }
            }
            text.push_str(&format!("━━━━━━━━━━━━━━\n💵 <b>Total Holdings:</b> <code>{} KAS</code>", f_num(total)));
            let _ = send_and_log(&bot, msg.chat.id, text, None).await;
        }
        Command::Network => {
            if let Ok(info) = rpc.get_block_dag_info().await {
                let text = format!("🌐 <b>Network Stats:</b>\n🧩 <b>Network:</b> <code>{}</code>\n⚙️ <b>Difficulty:</b> <code>{}</code>\n📊 <b>DAA Score:</b> <code>{}</code>", info.network, f_num(info.difficulty as f64), f_num(info.virtual_daa_score as f64));
                let _ = send_and_log(&bot, msg.chat.id, text, None).await;
            }
        }
        Command::Dag => {
            if let Ok(info) = rpc.get_block_dag_info().await {
                let text = format!("🧱 <b>BlockDAG Details:</b>\n🧱 <b>Blocks:</b> <code>{}</code>\n📑 <b>Headers:</b> <code>{}</code>", f_num(info.block_count as f64), f_num(info.header_count as f64));
                let _ = send_and_log(&bot, msg.chat.id, text, None).await;
            }
        }
        Command::Price => {
            if let Ok(r) = reqwest::get("https://api.kaspa.org/info/price").await {
                if let Ok(j) = r.json::<serde_json::Value>().await {
                    let _ = send_and_log(&bot, msg.chat.id, format!("💵 <b>Price:</b> <code>${:.4} USD</code>", j["price"].as_f64().unwrap_or(0.0)), None).await;
                }
            }
        }
        Command::Market => {
            if let Ok(r) = reqwest::get("https://api.kaspa.org/info/marketcap").await {
                if let Ok(j) = r.json::<serde_json::Value>().await {
                    let _ = send_and_log(&bot, msg.chat.id, format!("📈 <b>Market Cap:</b> <code>${} USD</code>", f_num(j["marketcap"].as_f64().unwrap_or(0.0))), None).await;
                }
            }
        }
        Command::Supply => {
            if let Ok(r) = reqwest::get("https://api.kaspa.org/info/coinsupply/circulating").await {
                if let Ok(c) = r.text().await {
                    let val = c.parse::<f64>().unwrap_or(0.0);
                    let _ = send_and_log(&bot, msg.chat.id, format!("🪙 <b>Supply:</b> <code>{} KAS</code>", f_num(val)), None).await;
                }
            }
        }
        Command::Fees => {
            if let Ok(r) = reqwest::get("https://api.kaspa.org/info/fee-estimate").await {
                if let Ok(j) = r.json::<serde_json::Value>().await {
                    let feerate = j["normalBuckets"][0]["feerate"].as_f64().unwrap_or(0.0);
                    let _ = send_and_log(&bot, msg.chat.id, format!("⛽ <b>Fee Estimate:</b> <code>{:.2} sompi/gram</code>", feerate), None).await;
                }
            }
        }
        Command::Sys => {
            if is_admin {
                let mut s = System::new_all(); s.refresh_all();
                let text = format!("⚙️ <b>Server Node Diagnostics:</b>\n🧠 <b>RAM Used:</b> <code>{} MB</code>\n🧠 <b>RAM Total:</b> <code>{} MB</code>\n👀 <b>Monitor:</b> <code>{}</code>", s.used_memory()/1024/1024, s.total_memory()/1024/1024, monitoring.load(Ordering::Relaxed));
                let _ = send_and_log(&bot, msg.chat.id, text, None).await;
            }
        }
        Command::Pause => { 
            if is_admin { 
                monitoring.store(false, Ordering::Relaxed); 
                let _ = send_and_log(&bot, msg.chat.id, "⏸️ <b>Paused.</b>".to_string(), None).await; 
            } 
        }
        Command::Resume => { 
            if is_admin { 
                monitoring.store(true, Ordering::Relaxed); 
                let _ = send_and_log(&bot, msg.chat.id, "▶️ <b>Running.</b>".to_string(), None).await; 
            } 
        }
        Command::Restart => { 
            if is_admin { 
                let _ = send_and_log(&bot, msg.chat.id, "🔄 <b>Restarting safely...</b>".to_string(), None).await; 
                save_state(&state).await; 
                std::process::exit(0); 
            } 
        }
        Command::Broadcast(m) => {
            if is_admin {
                let users: HashSet<i64> = state.iter().flat_map(|e| e.value().clone()).collect();
                let mut success_count = 0;
                let msg_text = format!("📢 <b>Admin Broadcast:</b>\n\n{}", m);
                
                for u in users { 
                    match bot.send_message(ChatId(u), &msg_text).parse_mode(teloxide::types::ParseMode::Html).await {
                        Ok(_) => success_count += 1,
                        Err(e) => {
                            error!("[BROADCAST ERROR] Failed to send to {}: {}", u, e);
                            for mut entry in state.iter_mut() { entry.value_mut().remove(&u); }
                        }
                    }
                    sleep(Duration::from_millis(50)).await; 
                }
                save_state(&state).await; 
                let _ = send_and_log(&bot, msg.chat.id, format!("✅ Broadcast sent successfully to {} users.", success_count), None).await;
            }
        }
        Command::Logs => {
            if is_admin {
                if let Ok(file) = std::fs::File::open("bot.log") {
                    let rev_lines = RevLines::new(BufReader::new(file));
                    let mut lines: Vec<String> = rev_lines.take(20).filter_map(|r| r.ok()).collect();
                    lines.reverse(); 
                    let last = lines.join("\n");
                    let _ = send_and_log(&bot, msg.chat.id, format!("📜 <b>System Logs:</b>\n<pre>{}</pre>", last), None).await;
                } else {
                    let _ = send_and_log(&bot, msg.chat.id, "❌ <b>Could not read log file.</b>".to_string(), None).await;
                }
            }
        }
    };
    Ok(())
}