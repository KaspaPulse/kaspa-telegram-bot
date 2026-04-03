#![allow(deprecated, unused_imports)]
use teloxide::{prelude::*, utils::command::BotCommands, types::{InlineKeyboardMarkup, ParseMode, BotCommand, BotCommandScope}};
use std::sync::Arc;
use sysinfo::System;
use crate::state::{AppState, WalletData, MAX_ACCOUNTS_PER_WALLET};
use crate::api::ApiManager;
use crate::utils::helpers::{clean_and_validate_wallet, format_short_wallet, format_hashrate, main_keyboard};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    Start, Help, Add(String), Remove(String), List, Balance, Price, Market, Fees, Network, Supply, Dag,
    Sys, Pause, Resume, Restart, Logs, Broadcast(String)
}

async fn send_msg(bot: &Bot, chat_id: i64, text: &str) -> ResponseResult<Message> {
    let safe_log = text.replace('\n', " \\ ");
    tracing::info!("[BOT OUT] Chat ID: {} | Msg: {}", chat_id, safe_log);
    bot.send_message(ChatId(chat_id), text)
        .parse_mode(ParseMode::Markdown)
        .disable_web_page_preview(true)
        .await
}

pub async fn start_telegram_bot(bot: Bot, state: Arc<AppState>, api: Arc<ApiManager>) {
    if let Ok(contents) = tokio::fs::read_to_string(".restart_flag").await {
        if let Ok(chat_id) = contents.trim().parse::<i64>() {
            let _ = send_msg(&bot, chat_id, "✅ *Restart Successful!*\nSystem is back online and running at full capacity.").await;
            let _ = tokio::fs::remove_file(".restart_flag").await;
        }
    }

    let public_cmds = vec![
        BotCommand::new("start", "Show commands menu"),
        BotCommand::new("add", "Start tracking a wallet"),
        BotCommand::new("remove", "Stop monitoring a wallet"),
        BotCommand::new("list", "Show your tracked wallets"),
        BotCommand::new("balance", "Check direct Node Balance"),
        BotCommand::new("network", "Node Hashrate & Difficulty"),
        BotCommand::new("fees", "Mempool Fee Estimates"),
        BotCommand::new("supply", "Circulating Supply"),
        BotCommand::new("dag", "Local BlockDAG Details"),
        BotCommand::new("price", "Current KAS Price"),
        BotCommand::new("market", "Price & Market Cap"),
    ];
    let _ = bot.set_my_commands(public_cmds.clone()).scope(BotCommandScope::Default).await;

    if let Some(admin) = state.admin_id {
        let mut admin_cmds = public_cmds;
        admin_cmds.push(BotCommand::new("sys", "👑 Admin: Server Diagnostics"));
        admin_cmds.push(BotCommand::new("pause", "👑 Admin: Disconnect Engine"));
        admin_cmds.push(BotCommand::new("resume", "👑 Admin: Reconnect Engine"));
        admin_cmds.push(BotCommand::new("restart", "👑 Admin: Reboot Process"));
        admin_cmds.push(BotCommand::new("logs", "👑 Admin: View Last 25 Logs"));
        admin_cmds.push(BotCommand::new("broadcast", "👑 Admin: Send msg to all"));
        let _ = bot.set_my_commands(admin_cmds).scope(BotCommandScope::Chat { chat_id: teloxide::types::Recipient::Id(ChatId(admin)) }).await;
    }

    let handler = dptree::entry()
        .branch(Update::filter_message().filter_command::<Command>().endpoint(handle_cmd))
        .branch(Update::filter_message().endpoint(handle_plain_text))
        .branch(Update::filter_callback_query().endpoint(handle_cb));

    tracing::info!("[SYSTEM] Enterprise Admin Engine Active.");
    Dispatcher::builder(bot, handler).dependencies(dptree::deps![state, api]).enable_ctrlc_handler().build().dispatch().await;
}

fn check_rate_limit(state: &Arc<AppState>, chat_id: i64) -> bool {
    if Some(chat_id) == state.admin_id { return true; }
    let now = std::time::Instant::now();
    if let Some(mut last_time) = state.rate_limits.get_mut(&chat_id) {
        if now.duration_since(*last_time).as_secs() < 3 { return false; }
        *last_time = now;
    } else {
        state.rate_limits.insert(chat_id, now);
    }
    true
}

async fn handle_plain_text(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let cid = msg.chat.id.0;
    if !check_rate_limit(&state, cid) { return Ok(()); }
    
    if let Some(text) = msg.text() {
        tracing::info!("[CMD IN] Chat ID: {} | Executing: PlainText ({})", cid, text);
        if let Some(valid) = clean_and_validate_wallet(text) {
            let mut is_new = false;
            let mut should_save = false;
            
            { 
                let mut entry = state.monitored_wallets.entry(valid.clone()).or_insert_with(|| {
                    is_new = true;
                    WalletData { last_balance: 0.0, chat_ids: vec![cid] }
                });
                if !is_new && !entry.chat_ids.contains(&cid) {
                    if entry.chat_ids.len() < MAX_ACCOUNTS_PER_WALLET {
                        entry.chat_ids.push(cid);
                        should_save = true;
                    }
                } else if is_new {
                    should_save = true;
                }
            }

            if should_save {
                state.sync_wallet_to_db(&valid).await;
                send_msg(&bot, cid, "✅ *Wallet Linked Successfully*").await?;
            } else {
                send_msg(&bot, cid, "⚠️ Wallet is already tracked or limit reached.").await?;
            }
        }
    }
    Ok(())
}

#[tracing::instrument(skip(bot, state, api))]
async fn handle_cmd(bot: Bot, msg: Message, cmd: Command, state: Arc<AppState>, api: Arc<ApiManager>) -> ResponseResult<()> {
    let cid = msg.chat.id.0;
    tracing::info!("[CMD IN] Chat ID: {} | Executing: {:?}", cid, cmd);
    
    if !check_rate_limit(&state, cid) {
        send_msg(&bot, cid, "⏳ *Please wait a few seconds before executing again.*").await?;
        return Ok(());
    }

    match cmd {
        Command::Start | Command::Help => {
            bot.send_message(msg.chat.id, "🤖 *Kaspa Pulse Bot*\nSelect an option below:").parse_mode(ParseMode::Markdown).reply_markup(main_keyboard()).await?;
        },
        Command::Balance => {
            let mut total = 0.0;
            let mut txt = String::from("🏦 *Balances*\n");
            for kv in state.monitored_wallets.iter() {
                if kv.value().chat_ids.contains(&cid) {
                    let bal = api.get_balance(kv.key()).await.unwrap_or(0.0);
                    total += bal;
                    txt.push_str(&format!("• `{}`: *{:.2} KAS*\n", format_short_wallet(kv.key()), bal));
                }
            }
            txt.push_str(&format!("━━━━━━━━━━\n💰 *Total:* `{:.2} KAS`", total));
            send_msg(&bot, cid, &txt).await?;
        },
        Command::List => {
            let mut txt = String::from("📋 *Tracked Wallets*\n━━━━━━━━━━━━━━━━━━\n");
            let mut count = 0;
            for kv in state.monitored_wallets.iter() {
                if kv.value().chat_ids.contains(&cid) { 
                    txt.push_str(&format!("• `{}`\n", kv.key()));
                    count += 1;
                }
            }
            if count == 0 { txt.push_str("_No wallets tracked yet._"); }
            send_msg(&bot, cid, &txt).await?;
        },
        Command::Market => {
            let p = match api.get_price().await { Ok(v) => v["price"].as_f64().unwrap_or(0.0), Err(_) => 0.0 };
            let m = match api.get_market().await { Ok(v) => v["marketcap"].as_f64().unwrap_or(0.0), Err(_) => 0.0 };
            send_msg(&bot, cid, &format!("📈 *Kaspa Market Overview*\n━━━━━━━━━━━━━━━━━━\n🏷️ *Price:* `${:.4}`\n💎 *Market Cap:* `${:.0}`", p, m)).await?;
        },
        Command::Price => {
            let price = match api.get_price().await { Ok(v) => v["price"].as_f64().unwrap_or(0.0), Err(_) => 0.0 };
            send_msg(&bot, cid, &format!("💵 *Kaspa (KAS) Price*\n━━━━━━━━━━━━━━━━━━\n🏷️ *Current Price:* `${:.4} USD`", price)).await?;
        },
        Command::Network => {
            if let Ok(n) = api.get_network().await {
                let hr = format_hashrate(n["hashrate"].as_f64().unwrap_or(0.0));
                send_msg(&bot, cid, &format!("🌐 *Kaspa Network Stats*\n━━━━━━━━━━━━━━━━━━\n⛏️ *Hashrate:* `{}`", hr)).await?;
            }
        },
        Command::Supply => {
            if let Ok(s) = api.get_supply().await {
                let circ = s["circulatingSupply"].as_u64().unwrap_or(0) as f64 / 100_000_000.0;
                let max = s["maxSupply"].as_u64().unwrap_or(0) as f64 / 100_000_000.0;
                let mined = if max > 0.0 { (circ / max) * 100.0 } else { 0.0 };
                send_msg(&bot, cid, &format!("🪙 *Kaspa Supply Info*\n━━━━━━━━━━━━━━━━━━\n🔄 *Circulating:* `{:.0} KAS`\n🛑 *Max Supply:* `{:.0} KAS`\n⛏️ *Mined:* `{:.2}%`", circ, max, mined)).await?;
            }
        },
        Command::Fees => {
            if let Ok(f) = api.get_fees().await {
                let fee = f["priorityBucket"]["feerate"].as_f64().unwrap_or(0.0);
                send_msg(&bot, cid, &format!("⛽ *Network Mempool Fees*\n━━━━━━━━━━━━━━━━━━\n🔴 *Priority:* `{:.2} sompi/gram`", fee)).await?;
            }
        },
        Command::Dag => {
            if let Ok(d) = api.get_dag_info().await {
                let blocks = d["blockCount"].as_u64().unwrap_or(0);
                let headers = d["headerCount"].as_u64().unwrap_or(0);
                let diff = d["difficulty"].as_f64().unwrap_or(0.0);
                let net = d["networkName"].as_str().unwrap_or("Mainnet");
                send_msg(&bot, cid, &format!("📦 *Node BlockDAG Details*\n━━━━━━━━━━━━━━━━━━\n🧩 *Network:* `{}`\n🧱 *Blocks:* `{}`\n📑 *Headers:* `{}`\n⚙️ *Difficulty:* `{:.2}`", net, blocks, headers, diff)).await?;
            }
        },
        Command::Add(_) => { send_msg(&bot, cid, "Please just paste the wallet address directly to track it.").await?; },
        Command::Remove(wallet) => {
            let valid = clean_and_validate_wallet(&wallet).unwrap_or(wallet.clone());
            if let Some(mut entry) = state.monitored_wallets.get_mut(&valid) {
                entry.chat_ids.retain(|&id| id != cid);
                state.remove_wallet_from_db(&valid, cid).await;
                send_msg(&bot, cid, &format!("🗑️ *Removed from Tracking:*\n`{}`", valid)).await?;
            } else {
                send_msg(&bot, cid, "⚠️ Wallet not found in your tracking list.").await?;
            }
        },
        
        // --- ADMIN COMMANDS ---
        Command::Sys => {
            if Some(cid) == state.admin_id {
                let mut sys = System::new_all();
                sys.refresh_all();
                let uptime = state.start_time.elapsed().as_secs();
                let (d, h, m) = (uptime / 86400, (uptime % 86400) / 3600, (uptime % 3600) / 60);
                let total_ram = sys.total_memory() as f64 / 1_073_741_824.0;
                let used_ram = sys.used_memory() as f64 / 1_073_741_824.0;
                
                let text = format!("⚙️ *System Diagnostics*\n\n*Status:*\n🔸 Engine: `{}`\n🔸 Tracked Wallets: `{}`\n🔸 Uptime: `{}d {}h {}m`\n\n*Server:*\n🔹 OS: `{}`\n🔹 RAM: `{:.2} GB / {:.2} GB`",
                    if state.is_monitoring.load(std::sync::atomic::Ordering::Relaxed) { "ACTIVE ▶️" } else { "PAUSED ⏸️" },
                    state.monitored_wallets.len(), d, h, m,
                    System::long_os_version().unwrap_or_else(|| "Unknown".to_string()),
                    used_ram, total_ram
                );
                send_msg(&bot, cid, &text).await?;
            } else { send_msg(&bot, cid, "⛔ *Access Denied*").await?; }
        },
        Command::Pause => {
            if Some(cid) == state.admin_id {
                state.is_monitoring.store(false, std::sync::atomic::Ordering::Relaxed);
                send_msg(&bot, cid, "⏸️ *Engine Paused*").await?;
            }
        },
        Command::Resume => {
            if Some(cid) == state.admin_id {
                state.is_monitoring.store(true, std::sync::atomic::Ordering::Relaxed);
                send_msg(&bot, cid, "▶️ *Engine Resumed*").await?;
            }
        },
        Command::Restart => {
            if Some(cid) == state.admin_id {
                let _ = tokio::fs::write(".restart_flag", cid.to_string()).await;
                let _ = send_msg(&bot, cid, "🔄 *System Reboot Initiated*\n_Saving databases securely before exit..._").await;
                state.shutdown_token.cancel(); 
                tokio::spawn(async move { tokio::time::sleep(std::time::Duration::from_secs(3)).await; std::process::exit(0); });
            }
        },
        Command::Logs => {
            if Some(cid) == state.admin_id {
                let logs_content = tokio::fs::read_to_string("kaspa_bot.log").await.unwrap_or_else(|_| "No logs found on disk.".to_string());
                let lines: Vec<&str> = logs_content.lines().collect();
                let recent_logs = lines.into_iter().rev().take(25).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                let safe_logs = if recent_logs.len() > 3900 { &recent_logs[recent_logs.len()-3900..] } else { &recent_logs };
                let _ = bot.send_message(msg.chat.id, format!("📜 *Recent System Logs:*\n```text\n{}\n```", safe_logs)).parse_mode(ParseMode::Markdown).await;
            }
        },
        Command::Broadcast(text) => {
            if Some(cid) == state.admin_id {
                let users = state.get_all_users();
                let mut count = 0;
                for user in &users {
                    if send_msg(&bot, *user, &format!("📢 *Admin Announcement:*\n\n{}", text)).await.is_ok() { count += 1; }
                }
                send_msg(&bot, cid, &format!("✅ Broadcast sent to {} users.", count)).await?;
            }
        },
    }
    Ok(())
}

async fn handle_cb(bot: Bot, q: CallbackQuery, state: Arc<AppState>, api: Arc<ApiManager>) -> ResponseResult<()> {
    let _ = bot.answer_callback_query(q.id.clone()).await;
    if let (Some(msg), Some(data)) = (q.message, q.data) {
        tracing::info!("[RAW CALLBACK] Received callback query from ID: {} | Data: {}", q.from.id, data);
        let cmd = match data.as_str() {
            "cmd_balance" => Command::Balance,
            "cmd_list" => Command::List,
            "cmd_price" => Command::Price,
            "cmd_market" => Command::Market,
            "cmd_network" => Command::Network,
            "cmd_supply" => Command::Supply,
            "cmd_fees" => Command::Fees,
            "cmd_dag" => Command::Dag,
            _ => Command::Start,
        };
        let _ = handle_cmd(bot, msg, cmd, state, api).await;
    }
    Ok(())
}

