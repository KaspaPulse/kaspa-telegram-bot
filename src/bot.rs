#![allow(deprecated, unused_imports)]
use teloxide::{prelude::*, utils::command::BotCommands, types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode}};
use std::sync::Arc;
use std::time::Instant;
use sysinfo::System;
use dashmap::DashMap;
use tokio::process::Command as AsyncCommand;
use std::fs;

use crate::state::{AppState, WalletData, MAX_ACCOUNTS_PER_WALLET};
use crate::api::ApiManager;
use crate::utils::helpers::{clean_and_validate_wallet, format_short_wallet, format_hashrate, format_difficulty};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Kaspa Node Command Center")]
pub enum Command {
    Start, Help, Add(String), Remove(String), List, Balance, 
    Price, Market, Fees, Network, Supply, Dag,
    Sys, Pause, Resume, Restart, Logs, Broadcast(String),
}

pub struct RateLimiter { cooldowns: DashMap<i64, Instant> }
impl RateLimiter {
    pub fn new() -> Self { Self { cooldowns: DashMap::new() } }
    pub fn check(&self, chat_id: i64, admin_id: Option<i64>) -> bool {
        if Some(chat_id) == admin_id { return true; }
        let now = Instant::now();
        if let Some(mut last) = self.cooldowns.get_mut(&chat_id) {
            if now.duration_since(*last).as_secs() < 3 { return false; }
            *last = now;
        } else { self.cooldowns.insert(chat_id, now); }
        true
    }
}

pub async fn start_telegram_bot(bot: Bot, state: Arc<AppState>, api: Arc<ApiManager>) {
    let admin_id: Option<i64> = std::env::var("ADMIN_ID").ok().and_then(|id| id.parse().ok());
    let rate_limiter = Arc::new(RateLimiter::new());

    let handler = dptree::entry()
        .branch(Update::filter_message().filter_command::<Command>().endpoint(handle_cmd))
        .branch(Update::filter_message().endpoint(handle_plain_text)) // NATIVE SMART DETECT
        .branch(Update::filter_callback_query().endpoint(handle_cb));

    Dispatcher::builder(bot, handler).dependencies(dptree::deps![state, api, admin_id, rate_limiter]).enable_ctrlc_handler().build().dispatch().await;
}

fn main_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("💰 Balances", "cmd_balance"), InlineKeyboardButton::callback("📋 Tracked", "cmd_list")],
        vec![InlineKeyboardButton::callback("💵 Price", "cmd_price"), InlineKeyboardButton::callback("📈 Market", "cmd_market")],
        vec![InlineKeyboardButton::callback("🌐 Network", "cmd_network"), InlineKeyboardButton::callback("⛽ Fees", "cmd_fees")],
        vec![InlineKeyboardButton::callback("🪙 Supply", "cmd_supply"), InlineKeyboardButton::callback("📦 DAG", "cmd_dag")],
    ])
}

// 🧠 Smart Detect Handler
async fn handle_plain_text(bot: Bot, msg: Message, state: Arc<AppState>, admin: Option<i64>, rl: Arc<RateLimiter>) -> ResponseResult<()> {
    let cid = msg.chat.id.0;
    if let Some(text) = msg.text() {
        if let Some(valid) = clean_and_validate_wallet(text) {
            if !rl.check(cid, admin) { return Ok(()); }
            let mut is_tracked = false;
            if let Some(mut w) = state.monitored_wallets.get_mut(&valid) {
                if w.chat_ids.contains(&cid) { is_tracked = true; }
                else if w.chat_ids.len() >= MAX_ACCOUNTS_PER_WALLET { bot.send_message(msg.chat.id, "🚫 Limit Reached").await?; return Ok(()); }
                else { w.chat_ids.push(cid); state.save_wallets(); }
            } else {
                state.monitored_wallets.insert(valid.clone(), WalletData { last_balance: 0.0, chat_ids: vec![cid] });
                state.save_wallets();
            }
            let prefix = if is_tracked { "⚠️ Already Tracked" } else { "🧠 *Smart Detect:*\nAuto-Tracking started for:\n" };
            bot.send_message(msg.chat.id, format!("{} [{}]({})", prefix, format_short_wallet(&valid), format!("https://kaspa.stream/addresses/{}", valid))).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await?;
        }
    }
    Ok(())
}

async fn handle_cmd(bot: Bot, msg: Message, cmd: Command, state: Arc<AppState>, api: Arc<ApiManager>, admin: Option<i64>, rl: Arc<RateLimiter>) -> ResponseResult<()> {
    let cid = msg.chat.id.0;
    if !rl.check(cid, admin) { return Ok(()); }

    match cmd {
        Command::Start | Command::Help => {
            bot.send_message(msg.chat.id, "🤖 *Kaspa Node Command Center*\n/add `<address>` \\- Track\n/remove `<address>` \\- Stop\nChoose an option:")
                .parse_mode(ParseMode::MarkdownV2).reply_markup(main_menu()).await?;
        }
        Command::Add(addr) => {
            if let Some(valid) = clean_and_validate_wallet(&addr) {
                let mut is_tracked = false;
                if let Some(mut w) = state.monitored_wallets.get_mut(&valid) {
                    if w.chat_ids.contains(&cid) { is_tracked = true; }
                    else if w.chat_ids.len() >= MAX_ACCOUNTS_PER_WALLET { bot.send_message(msg.chat.id, "🚫 Limit Reached").await?; return Ok(()); }
                    else { w.chat_ids.push(cid); state.save_wallets(); }
                } else {
                    state.monitored_wallets.insert(valid.clone(), WalletData { last_balance: 0.0, chat_ids: vec![cid] });
                    state.save_wallets();
                }
                let prefix = if is_tracked { "⚠️ Already Tracked" } else { "✅ Added" };
                bot.send_message(msg.chat.id, format!("{} [{}]({})", prefix, format_short_wallet(&valid), format!("https://kaspa.stream/addresses/{}", valid))).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await?;
            }
        }
        Command::Remove(addr) => {
             if let Some(valid) = clean_and_validate_wallet(&addr) {
                if let Some(mut w) = state.monitored_wallets.get_mut(&valid) {
                    w.chat_ids.retain(|&id| id != cid);
                    if w.chat_ids.is_empty() { drop(w); state.monitored_wallets.remove(&valid); }
                    state.save_wallets();
                    bot.send_message(msg.chat.id, format!("🗑️ Removed [{}]({})", format_short_wallet(&valid), format!("https://kaspa.stream/addresses/{}", valid))).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await?;
                }
             }
        }
        Command::List => {
            let mut txt = String::from("📁 *Tracked Portfolio*\n━━━━━━━━━━━━━━━━━━\n");
            for kv in state.monitored_wallets.iter() {
                if kv.value().chat_ids.contains(&cid) {
                    txt.push_str(&format!("💳 [{}]({})\n💰 `Cached: {:.8} KAS`\n\n", format_short_wallet(kv.key()), format!("https://kaspa.stream/addresses/{}", kv.key()), kv.value().last_balance));
                }
            }
            bot.send_message(msg.chat.id, txt).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await?;
        }
        Command::Balance => {
            let mut txt = String::from("🏦 *Live Balances*\n━━━━━━━━━━━━━━━━━━\n");
            let mut total = 0.0;
            for kv in state.monitored_wallets.iter() {
                if kv.value().chat_ids.contains(&cid) {
                    let bal = api.get_balance(kv.key()).await.unwrap_or(kv.value().last_balance);
                    total += bal;
                    txt.push_str(&format!("💳 [{}]({})\n⚖️ `{:.8} KAS`\n\n", format_short_wallet(kv.key()), format!("https://kaspa.stream/addresses/{}", kv.key()), bal));
                }
            }
            txt.push_str(&format!("━━━━━━━━━━━━━━━━━━\n💵 *Total:* `{:.2} KAS`", total));
            bot.send_message(msg.chat.id, txt).parse_mode(ParseMode::Markdown).disable_web_page_preview(true).await?;
        }
        Command::Price | Command::Market => {
            let p = api.get_price().await.map(|v| v["price"].as_f64().unwrap_or(0.0)).unwrap_or(0.0);
            let m = api.get_market().await.map(|v| v["marketcap"].as_f64().unwrap_or(0.0)).unwrap_or(0.0);
            bot.send_message(msg.chat.id, format!("📈 *Kaspa Market*\n━━━━━━━━━━━━━━━━━━\n🏷️ *Price:* `${:.4}`\n💎 *Market Cap:* `${:?}`", p, m as u64)).parse_mode(ParseMode::Markdown).await?;
        }
        Command::Network => {
            if let Ok(d) = api.get_network().await {
                let hr = format_hashrate(d["hashrate"].as_f64().unwrap_or(0.0));
                bot.send_message(msg.chat.id, format!("🌐 *Network Stats*\n━━━━━━━━━━━━━━━━━━\n⛏️ *Hashrate:* `{}`", hr)).parse_mode(ParseMode::Markdown).await?;
            }
        }
        Command::Supply => {
            if let Ok(d) = api.get_supply().await {
                let c = d["circulatingSupply"].as_f64().unwrap_or(0.0) / 100_000_000.0;
                let m = d["maxSupply"].as_f64().unwrap_or(0.0) / 100_000_000.0;
                bot.send_message(msg.chat.id, format!("🪙 *Supply*\n━━━━━━━━━━━━━━━━━━\n🔄 *Circulating:* `{:.0} KAS`\n🛑 *Max:* `{:.0} KAS`", c, m)).parse_mode(ParseMode::Markdown).await?;
            }
        }
        Command::Dag => {
            if let Ok(d) = api.get_dag().await {
                let bc = d["blockCount"].as_u64().unwrap_or(0);
                let hc = d["headerCount"].as_u64().unwrap_or(0);
                bot.send_message(msg.chat.id, format!("📦 *BlockDAG*\n━━━━━━━━━━━━━━━━━━\n🧱 *Blocks:* `{}`\n📑 *Headers:* `{}`", bc, hc)).parse_mode(ParseMode::Markdown).await?;
            }
        }
        Command::Fees => {
            if let Ok(data) = api.get_fees().await {
                let fast = data["priorityBucket"]["feerate"].as_f64().unwrap_or(0.0);
                let norm = data["normalBuckets"][0]["feerate"].as_f64().unwrap_or(0.0);
                bot.send_message(msg.chat.id, format!("⛽ *Fees*\n━━━━━━━━━━━━━━━━━━\n🟡 *Normal:* `{:.3}`\n🔴 *Fast:* `{:.3}`", norm, fast)).parse_mode(ParseMode::Markdown).await?;
            }
        }
        Command::Sys => {
            if Some(cid) != admin { return Ok(()); }
            let mut sys = System::new_all(); sys.refresh_all();
            let tram = sys.total_memory() / 1024 / 1024 / 1024; let uram = sys.used_memory() / 1024 / 1024 / 1024;
            let txt = format!("⚙️ *System*\nMonitoring: {}\nRAM: {}/{} GB\nWallets: {}", state.is_monitoring.load(std::sync::atomic::Ordering::Relaxed), uram, tram, state.monitored_wallets.len());
            bot.send_message(msg.chat.id, txt).parse_mode(ParseMode::Markdown).await?;
        }
        Command::Pause => { if Some(cid) == admin { state.is_monitoring.store(false, std::sync::atomic::Ordering::Relaxed); bot.send_message(msg.chat.id, "⏸️ Paused").await?; } }
        Command::Resume => { if Some(cid) == admin { state.is_monitoring.store(true, std::sync::atomic::Ordering::Relaxed); bot.send_message(msg.chat.id, "▶️ Resumed").await?; } }
        Command::Restart => {
            if Some(cid) == admin {
                let _ = fs::write(".restart_flag", cid.to_string());
                bot.send_message(msg.chat.id, "🔄 Rebooting Process...").await?;
                std::process::exit(0);
            }
        }
        Command::Logs => {
            if Some(cid) == admin {
                if let Ok(output) = AsyncCommand::new("journalctl").args(&["-u", "kaspabot.service", "-n", "25", "--no-pager"]).output().await {
                    let mut logs = String::from_utf8_lossy(&output.stdout).to_string();
                    if logs.len() > 3900 { logs = logs[logs.len()-3900..].to_string(); }
                    bot.send_message(msg.chat.id, format!("📜 *Logs:*\n```text\n{}\n```", logs)).parse_mode(ParseMode::Markdown).await?;
                }
            }
        }
        Command::Broadcast(text) => {
            if Some(cid) == admin {
                let users = state.get_all_users();
                for u in &users { let _ = bot.send_message(ChatId(*u), format!("📢 *Admin:*\n{}", text)).parse_mode(ParseMode::Markdown).await; }
                bot.send_message(msg.chat.id, format!("✅ Sent to {} users.", users.len())).await?;
            }
        }
    }
    Ok(())
}

async fn handle_cb(bot: Bot, q: CallbackQuery, api: Arc<ApiManager>, rl: Arc<RateLimiter>, admin: Option<i64>) -> ResponseResult<()> {
    bot.answer_callback_query(q.id).await?;
    if let (Some(msg), Some(data)) = (q.message, q.data) {
        if !rl.check(msg.chat.id.0, admin) { return Ok(()); }
        let _ = handle_cmd(bot.clone(), msg.clone(), match data.as_str() {
            "cmd_price" => Command::Price, "cmd_market" => Command::Market, "cmd_network" => Command::Network,
            "cmd_fees" => Command::Fees, "cmd_supply" => Command::Supply, "cmd_dag" => Command::Dag,
            "cmd_balance" => Command::Balance, "cmd_list" => Command::List, _ => Command::Help
        }, Arc::new(AppState::new()), api, admin, rl).await; // Dummy state for read-only cmds
    }
    Ok(())
}
