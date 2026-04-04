use std::collections::{HashSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use dashmap::DashMap;
use teloxide::{prelude::*, types::{ChatId, KeyboardMarkup, KeyboardButton}, utils::command::BotCommands};
use kaspa_wrpc_client::{KaspaRpcClient, WrpcEncoding};
use kaspa_consensus_core::network::NetworkId;
use kaspa_hashes::Hash; 
use kaspa_rpc_core::api::rpc::RpcApi; 
use kaspa_addresses::Address;
use std::str::FromStr;
use dotenvy::dotenv;
use std::env;
use tokio::fs;
use tokio::time::{sleep, Duration};
use sysinfo::System;
use regex::Regex;
use tracing::{info, warn, error}; 
use tracing_subscriber::fmt::writer::MakeWriterExt; 
use chrono::Utc; 

type SharedState = Arc<DashMap<String, HashSet<i64>>>;
type UtxoState = Arc<DashMap<String, HashSet<String>>>; 
type SharedClient = Arc<KaspaRpcClient>; 

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Kaspa Node Bot Commands:")]
enum Command {
    #[command(description = "Start the bot and show help.")]
    Start,
    #[command(description = "Add a wallet: /add <address>")]
    Add(String),
    #[command(description = "Remove a wallet: /remove <address>")]
    Remove(String),
    #[command(description = "List tracked wallets.")]
    List,
    #[command(description = "Show network stats.")]
    Network,
    #[command(description = "Show BlockDAG details.")]
    Dag,
    #[command(description = "Check Live Balance.")]
    Balance,
    #[command(description = "Check KAS Price.")]
    Price,
    #[command(description = "Check Market Cap.")]
    Market,
    #[command(description = "Check Supply.")]
    Supply,
    #[command(description = "Check Mempool Fees.")]
    Fees,
    #[command(description = "👑 Admin: Server Diagnostics")]
    Sys,
    #[command(description = "👑 Admin: Disconnect Engine")]
    Pause,
    #[command(description = "👑 Admin: Reconnect Engine")]
    Resume,
    #[command(description = "👑 Admin: Reboot Process")]
    Restart,
    #[command(description = "👑 Admin: Message all users")]
    Broadcast(String),
    #[command(description = "👑 Admin: View System Logs")]
    Logs,
}

fn f_num(n: f64) -> String {
    let s = format!("{:.0}", n);
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        result.push(c);
        if (len - i - 1) % 3 == 0 && i != len - 1 { result.push(','); }
    }
    result
}

fn make_golden_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![
        vec![KeyboardButton::new("/balance"), KeyboardButton::new("/network")],
        vec![KeyboardButton::new("/fees"), KeyboardButton::new("/supply")],
        vec![KeyboardButton::new("/price"), KeyboardButton::new("/market")],
        vec![KeyboardButton::new("/dag"), KeyboardButton::new("/list")],
    ]).resize_keyboard()
}

async fn send_and_log(bot: &Bot, chat_id: ChatId, text: String, markup: Option<KeyboardMarkup>) -> anyhow::Result<()> {
    let log_text = text.replace('\n', " | ");
    info!("[BOT OUT] Chat: {} | Msg: {}", chat_id.0, log_text);
    let mut req = bot.send_message(chat_id, text).parse_mode(teloxide::types::ParseMode::Html);
    if let Some(m) = markup { req = req.reply_markup(m); }
    req.await?;
    Ok(())
}

fn format_short_wallet(w: &str) -> String {
    if w.len() > 18 { format!("{}...{}", &w[..12], &w[w.len()-6..]) } else { w.to_string() }
}

fn format_hash(h: &str, link_type: &str) -> String {
    if h.len() > 16 && !h.contains("Searching") && !h.contains("Indexing") && h != "Not Found" {
        format!("<a href=\"https://kaspa.stream/{}/{}\">{}...{}</a>", link_type, h, &h[..8], &h[h.len()-8..])
    } else {
        format!("<code>{}</code>", h)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let file_appender = tracing_appender::rolling::never(".", "bot.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt().with_writer(non_blocking.and(std::io::stdout)).with_ansi(false).init();

    info!("[INIT] Server Node02 - Final Masterpiece Engaged.");

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
    let shared_rpc: SharedClient = Arc::new(rpc_client);

    let rpc_for_bg = Arc::clone(&shared_rpc);
    tokio::spawn(async move {
        let _ = rpc_for_bg.connect(None).await;
        loop {
            sleep(Duration::from_secs(30)).await;
            if rpc_for_bg.get_server_info().await.is_err() { let _ = rpc_for_bg.connect(None).await; }
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
                            let initial_msg = format!("⚡ <b>Native Node Reward!</b> 💎\n━━━━━━━━━━━━━━━━━━\n<b>Time:</b> <code>{}</code>\n<b>Wallet:</b> <a href=\"https://kaspa.stream/addresses/{}\">{}</a>\n<b>Amount:</b> <code>+{:.8} KAS</code>\n<b>Live Balance:</b> <code>{:.8} KAS</code>\n━━━━━━━━━━━━━━━━━━\n<b>TXID:</b> {}\n<b>Mined Block:</b> ⏳ <code>Indexing...</code>\n<b>TX Block:</b> ⏳ <code>Searching...</code>\n<b>Accepting Block:</b> ⏳ <code>Indexing...</code>\n<b>DAA Score:</b> <code>{}</code>", time_str, wallet, short_wallet, diff, live_bal, format_hash(&tx_id, "transactions"), daa_score);
                            let mut s_ids = Vec::new();
                            for s in subs.clone() { if let Ok(m) = alert_bot.send_message(ChatId(s), &initial_msg).parse_mode(teloxide::types::ParseMode::Html).await { s_ids.push((ChatId(s), m.id)); } }
                            let (f_tx, w_cl, bot_cl, rpc_cl) = (tx_id.clone(), wallet.clone(), alert_bot.clone(), Arc::clone(&alert_rpc));
                            tokio::spawn(async move {
                                sleep(Duration::from_secs(12)).await;
                                let (mut b_h, mut a_h, mut m_h) = ("Not Found".to_string(), "Not Found".to_string(), "Not Found".to_string());
                                for _ in 1..=6 {
                                    if let Ok(r) = reqwest::get(format!("https://api.kaspa.org/transactions/{}", f_tx)).await {
                                        if let Ok(j) = r.json::<serde_json::Value>().await {
                                            if let Some(ba) = j["block_hash"].as_array() { if !ba.is_empty() { b_h = ba[0].as_str().unwrap_or("Not Found").to_string(); } }
                                            a_h = j["accepting_block_hash"].as_str().unwrap_or("Not Found").to_string();
                                            if b_h != "Not Found" {
                                                if let Ok(h) = Hash::from_str(&b_h) {
                                                    if let Ok(bl) = rpc_cl.get_block(h, false).await {
                                                        if let Some(v) = bl.verbose_data { if !v.merge_set_blues_hashes.is_empty() { m_h = v.merge_set_blues_hashes[0].to_string(); } }
                                                    }
                                                }
                                                if m_h == "Not Found" {
                                                    if let Ok(br) = reqwest::get(format!("https://api.kaspa.org/blocks/{}", b_h)).await {
                                                        if let Ok(bj) = br.json::<serde_json::Value>().await {
                                                            if let Some(blu) = bj["merge_set_blues_hashes"].as_array() { if !blu.is_empty() { m_h = blu[0].as_str().unwrap_or("Not Found").to_string(); } }
                                                        }
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                    }
                                    sleep(Duration::from_secs(10)).await;
                                }
                                let up_msg = format!("⚡ <b>Native Node Reward!</b> 💎\n━━━━━━━━━━━━━━━━━━\n<b>Time:</b> <code>{}</code>\n<b>Wallet:</b> <a href=\"https://kaspa.stream/addresses/{}\">{}</a>\n<b>Amount:</b> <code>+{:.8} KAS</code>\n<b>Live Balance:</b> <code>{:.8} KAS</code>\n━━━━━━━━━━━━━━━━━━\n<b>TXID:</b> {}\n<b>Mined Block:</b> {}\n<b>TX Block:</b> {}\n<b>Accepting Block:</b> {}\n<b>DAA Score:</b> <code>{}</code>", time_str, w_cl, format_short_wallet(&w_cl), diff, live_bal, format_hash(&f_tx, "transactions"), format_hash(&m_h, "blocks"), format_hash(&b_h, "blocks"), format_hash(&a_h, "blocks"), daa_score);
                                for (ci, mi) in s_ids { let _ = bot_cl.edit_message_text(ci, mi, &up_msg).parse_mode(teloxide::types::ParseMode::Html).await; }
                            });
                        }
                    }
                }
            }
        }
    });

    Command::repl(bot.clone(), move |bot: Bot, msg: Message, cmd: Command| {
        let state = Arc::clone(&state);
        let rpc = Arc::clone(&shared_rpc); 
        let monitoring = Arc::clone(&is_monitoring);
        async move {
            let chat_id = msg.chat.id.0;
            info!("[CMD IN] User: {} | Msg: {}", chat_id, msg.text().unwrap_or(""));
            let is_admin = chat_id == admin_id;
            match cmd {
                Command::Start => { let _ = send_and_log(&bot, msg.chat.id, "🤖 <b>Kaspa Platinum Command Center</b>\n━━━━━━━━━━━━━━━━━━\n<b>Node02:</b> Online 🟢\n<b>Tracking:</b> Active 🛡️\n\n<b>Choose an option:</b>".to_string(), Some(make_golden_keyboard())).await; }
                Command::Add(w) => {
                    let c = if w.starts_with("kaspa:") { w.clone() } else { format!("kaspa:{}", w) };
                    if Regex::new(r"^kaspa:[a-z0-9]{61,63}$").unwrap().is_match(&c) {
                        state.entry(c.clone()).or_insert_with(HashSet::new).insert(chat_id);
                        let _ = send_and_log(&bot, msg.chat.id, format!("✅ <b>Tracking Enabled:</b>\n<code>{}</code>", c), Some(make_golden_keyboard())).await;
                    } else { let _ = send_and_log(&bot, msg.chat.id, "❌ <b>Invalid Format!</b>".to_string(), None).await; }
                }
                Command::Remove(w) => {
                    let c = if w.starts_with("kaspa:") { w.clone() } else { format!("kaspa:{}", w) };
                    if let Some(mut subs) = state.get_mut(&c) { subs.remove(&chat_id); }
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
                        let text = format!("⚙️ <b>Server Node02:</b>\n🧠 <b>RAM Used:</b> <code>{} MB</code>\n🧠 <b>RAM Total:</b> <code>{} MB</code>\n👀 <b>Monitor:</b> <code>{}</code>", s.used_memory()/1024/1024, s.total_memory()/1024/1024, monitoring.load(Ordering::Relaxed));
                        let _ = send_and_log(&bot, msg.chat.id, text, None).await;
                    }
                }
                Command::Pause => { if is_admin { monitoring.store(false, Ordering::Relaxed); let _ = send_and_log(&bot, msg.chat.id, "⏸️ <b>Paused.</b>".to_string(), None).await; } }
                Command::Resume => { if is_admin { monitoring.store(true, Ordering::Relaxed); let _ = send_and_log(&bot, msg.chat.id, "▶️ <b>Running.</b>".to_string(), None).await; } }
                Command::Restart => { if is_admin { let _ = send_and_log(&bot, msg.chat.id, "🔄 <b>Restarting...</b>".to_string(), None).await; std::process::exit(0); } }
                Command::Broadcast(m) => {
                    if is_admin {
                        let users: HashSet<i64> = state.iter().flat_map(|e| e.value().clone()).collect();
                        for u in users { let _ = bot.send_message(ChatId(u), format!("📢 <b>Admin Broadcast:</b>\n\n{}", m)).parse_mode(teloxide::types::ParseMode::Html).await; }
                        let _ = send_and_log(&bot, msg.chat.id, format!("✅ Sent to {} users.", state.len()), None).await;
                    }
                }
                Command::Logs => {
                    if is_admin {
                        if let Ok(log) = std::fs::read_to_string("bot.log") {
                            let lines: Vec<&str> = log.lines().collect();
                            let last = if lines.len() > 20 { lines[lines.len()-20..].join("\n") } else { lines.join("\n") };
                            let _ = send_and_log(&bot, msg.chat.id, format!("📜 <b>System Logs:</b>\n<pre>{}</pre>", last), None).await;
                        }
                    }
                }
            };
            Ok(())
        }
    }).await;
    Ok(())
}
