#![allow(deprecated, unused_imports)]
use crate::api::ApiManager;
use crate::state::{AppState, WalletData, MAX_ACCOUNTS_PER_WALLET};
use crate::utils::helpers::{clean_and_validate_wallet, format_hashrate, format_short_wallet};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::System;
use teloxide::{
    prelude::*,
    types::{BotCommand, BotCommandScope, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
    utils::command::BotCommands,
};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    Start,
    Help,
    Add(String),
    Remove(String),
    List,
    Balance,
    Price,
    Market,
    Fees,
    Network,
    Supply,
    Dag,
    Sys,
    Pause,
    Resume,
    Restart,
    Logs,
    Broadcast(String),
}

pub async fn start_telegram_bot(bot: Bot, state: Arc<AppState>, api: Arc<ApiManager>) {
    // 1. Restart Flag Handling
    if let Ok(contents) = tokio::fs::read_to_string(".restart_flag").await {
        if let Ok(chat_id) = contents.trim().parse::<i64>() {
            let _ = bot
                .send_message(
                    ChatId(chat_id),
                    "✅ *Restart Successful!*\nSystem is back online and running at full capacity.",
                )
                .parse_mode(ParseMode::Markdown)
                .await;
            let _ = tokio::fs::remove_file(".restart_flag").await;
        }
    }

    // 2. Auto-configure Telegram Menus
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
    let _ = bot
        .set_my_commands(public_cmds.clone())
        .scope(BotCommandScope::Default)
        .await;

    if let Some(admin) = state.admin_id {
        let mut admin_cmds = public_cmds;
        admin_cmds.push(BotCommand::new("sys", "👑 Admin: Server Diagnostics"));
        admin_cmds.push(BotCommand::new("pause", "👑 Admin: Disconnect Engine"));
        admin_cmds.push(BotCommand::new("resume", "👑 Admin: Reconnect Engine"));
        admin_cmds.push(BotCommand::new("restart", "👑 Admin: Reboot Process"));
        admin_cmds.push(BotCommand::new("logs", "👑 Admin: View Last 25 Logs"));
        admin_cmds.push(BotCommand::new("broadcast", "👑 Admin: Send msg to all"));
        let _ = bot
            .set_my_commands(admin_cmds)
            .scope(BotCommandScope::Chat {
                chat_id: teloxide::types::Recipient::Id(ChatId(admin)),
            })
            .await;
    }

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(handle_cmd),
        )
        .branch(Update::filter_message().endpoint(handle_plain_text))
        .branch(Update::filter_callback_query().endpoint(handle_cb));

    tracing::info!("[SYSTEM] Enterprise Admin Engine Active.");
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state, api])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

fn check_rate_limit(state: &Arc<AppState>, chat_id: i64) -> bool {
    if Some(chat_id) == state.admin_id {
        return true;
    }
    let now = std::time::Instant::now();
    if let Some(mut last_time) = state.rate_limits.get_mut(&chat_id) {
        if now.duration_since(*last_time).as_secs() < 3 {
            return false;
        }
        *last_time = now;
    } else {
        state.rate_limits.insert(chat_id, now);
    }
    true
}

fn main_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("💰 Balances", "cmd_balance"),
            InlineKeyboardButton::callback("📋 Tracked", "cmd_list"),
        ],
        vec![
            InlineKeyboardButton::callback("📊 Market", "cmd_market"),
            InlineKeyboardButton::callback("⛽ Fees", "cmd_fees"),
        ],
        vec![
            InlineKeyboardButton::callback("🌐 Network", "cmd_network"),
            InlineKeyboardButton::callback("🪙 Supply", "cmd_supply"),
        ],
    ])
}

async fn handle_plain_text(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    if !check_rate_limit(&state, msg.chat.id.0) {
        return Ok(());
    }
    if let Some(text) = msg.text() {
        if let Some(valid) = clean_and_validate_wallet(text) {
            let cid = msg.chat.id.0;
            let mut is_new = false;
            let mut should_save = false;

            {
                // Scope to safely drop DashMap locks BEFORE awaiting Async I/O
                let mut entry = state
                    .monitored_wallets
                    .entry(valid.clone())
                    .or_insert_with(|| {
                        is_new = true;
                        WalletData {
                            last_balance: 0.0,
                            chat_ids: vec![cid],
                        }
                    });
                if !is_new && !entry.chat_ids.contains(&cid) {
                    if entry.chat_ids.len() < MAX_ACCOUNTS_PER_WALLET {
                        entry.chat_ids.push(cid);
                        should_save = true;
                    }
                } else if is_new {
                    should_save = true;
                }
            } // Write Lock is safely dropped here!

            if should_save {
                state.save_wallets().await;
            }
            bot.send_message(msg.chat.id, "✅ *Wallet Linked Successfully*")
                .parse_mode(ParseMode::Markdown)
                .await?;
        }
    }
    Ok(())
}

#[tracing::instrument(skip(bot, state, api))]
async fn handle_cmd(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<AppState>,
    api: Arc<ApiManager>,
) -> ResponseResult<()> {
    let cid = msg.chat.id.0;

    if !check_rate_limit(&state, cid) {
        let _ = bot
            .send_message(
                msg.chat.id,
                "⏳ *Please wait a few seconds before executing again.*",
            )
            .parse_mode(ParseMode::Markdown)
            .await;
        return Ok(());
    }

    match cmd {
        Command::Start | Command::Help => {
            bot.send_message(msg.chat.id, "🤖 *Kaspa Pulse Bot*\nSelect an option below:")
                .parse_mode(ParseMode::Markdown)
                .reply_markup(main_menu())
                .await?;
        }
        Command::Balance => {
            let mut total = 0.0;
            let mut txt = String::from("🏦 *Balances*\n");
            for kv in state.monitored_wallets.iter() {
                if kv.value().chat_ids.contains(&cid) {
                    let bal = api.get_balance(kv.key()).await.unwrap_or(0.0);
                    total += bal;
                    txt.push_str(&format!(
                        "• `{}`: *{:.2} KAS*\n",
                        format_short_wallet(kv.key()),
                        bal
                    ));
                }
            }
            txt.push_str(&format!("━━━━━━━━━━\n💰 *Total:* `{:.2} KAS`", total));
            bot.send_message(msg.chat.id, txt)
                .parse_mode(ParseMode::Markdown)
                .await?;
        }
        Command::List => {
            let mut txt = String::from("📋 *Tracked Wallets*\n━━━━━━━━━━━━━━━━━━\n");
            let mut count = 0;
            for kv in state.monitored_wallets.iter() {
                if kv.value().chat_ids.contains(&cid) {
                    txt.push_str(&format!("• `{}`\n", kv.key()));
                    count += 1;
                }
            }
            if count == 0 {
                txt.push_str("_No wallets tracked yet._");
            }
            bot.send_message(msg.chat.id, txt)
                .parse_mode(ParseMode::Markdown)
                .await?;
        }
        Command::Market => {
            let p = api
                .get_price()
                .await
                .map(|v| v["price"].as_f64().unwrap_or(0.0))
                .unwrap_or(0.0);
            let m = api
                .get_market()
                .await
                .map(|v| v["marketcap"].as_f64().unwrap_or(0.0))
                .unwrap_or(0.0);
            bot.send_message(
                msg.chat.id,
                format!("📊 *Market*\nPrice: `${:.4}`\nCap: `${:.0}`", p, m),
            )
            .parse_mode(ParseMode::Markdown)
            .await?;
        }
        Command::Price => {
            let p = api
                .get_price()
                .await
                .map(|v| v["price"].as_f64().unwrap_or(0.0))
                .unwrap_or(0.0);
            bot.send_message(msg.chat.id, format!("💵 *Price:* `${:.4}`", p))
                .parse_mode(ParseMode::Markdown)
                .await?;
        }
        Command::Network => {
            if let Ok(n) = api.get_network().await {
                let hr = format_hashrate(n["hashrate"].as_f64().unwrap_or(0.0));
                bot.send_message(msg.chat.id, format!("🌐 *Network Hashrate:* `{}`", hr))
                    .parse_mode(ParseMode::Markdown)
                    .await?;
            }
        }
        Command::Supply => {
            if let Ok(s) = api.get_supply().await {
                let circ_raw = s["circulatingSupply"]
                    .as_str()
                    .unwrap_or("0")
                    .parse::<f64>()
                    .unwrap_or(0.0);
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "🪙 *Circulating Supply:* `{:.0} KAS*`",
                        circ_raw / 100_000_000.0
                    ),
                )
                .parse_mode(ParseMode::Markdown)
                .await?;
            }
        }
        Command::Fees => {
            if let Ok(f) = api.get_fees().await {
                let fee = f["priorityBucket"]["feerate"].as_f64().unwrap_or(0.0);
                bot.send_message(
                    msg.chat.id,
                    format!("⛽ *Priority Fee:* `{:.2} sompi/byte`", fee),
                )
                .parse_mode(ParseMode::Markdown)
                .await?;
            }
        }
        Command::Dag => {
            bot.send_message(
                msg.chat.id,
                "📦 *DAG info is currently synced via WS Node.*",
            )
            .parse_mode(ParseMode::Markdown)
            .await?;
        }
        Command::Add(ref _w) | Command::Remove(ref _w) => {
            bot.send_message(
                msg.chat.id,
                "Please just paste the wallet address directly to add/remove.",
            )
            .await?;
        }

        // --- ADMIN COMMANDS ---
        Command::Sys => {
            if Some(cid) == state.admin_id {
                let mut sys = System::new_all();
                sys.refresh_all();
                let uptime = state.start_time.elapsed().as_secs();
                let (d, h, m) = (
                    uptime / 86400,
                    (uptime % 86400) / 3600,
                    (uptime % 3600) / 60,
                );
                let total_ram = sys.total_memory() as f64 / 1_073_741_824.0;
                let used_ram = sys.used_memory() as f64 / 1_073_741_824.0;

                let text = format!("⚙️ *System Diagnostics*\n\n*Status:*\n🔸 Engine: `{}`\n🔸 Tracked Wallets: `{}`\n🔸 Uptime: `{}d {}h {}m`\n\n*Server:*\n🔹 OS: `{}`\n🔹 RAM: `{:.2} GB / {:.2} GB`",
                    if state.is_monitoring.load(std::sync::atomic::Ordering::Relaxed) { "ACTIVE ▶️" } else { "PAUSED ⏸️" },
                    state.monitored_wallets.len(), d, h, m,
                    System::long_os_version().unwrap_or_else(|| "Unknown".to_string()),
                    used_ram, total_ram
                );
                bot.send_message(msg.chat.id, text)
                    .parse_mode(ParseMode::Markdown)
                    .await?;
            } else {
                bot.send_message(msg.chat.id, "⛔ *Access Denied*")
                    .parse_mode(ParseMode::Markdown)
                    .await?;
            }
        }
        Command::Pause => {
            if Some(cid) == state.admin_id {
                state
                    .is_monitoring
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                bot.send_message(msg.chat.id, "⏸️ *Engine Paused*")
                    .parse_mode(ParseMode::Markdown)
                    .await?;
            }
        }
        Command::Resume => {
            if Some(cid) == state.admin_id {
                state
                    .is_monitoring
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                bot.send_message(msg.chat.id, "▶️ *Engine Resumed*")
                    .parse_mode(ParseMode::Markdown)
                    .await?;
            }
        }
        Command::Restart => {
            if Some(cid) == state.admin_id {
                let _ = tokio::fs::write(".restart_flag", cid.to_string()).await;
                let _ = bot
                    .send_message(msg.chat.id, "🔄 *System Reboot Initiated*")
                    .parse_mode(ParseMode::Markdown)
                    .await;
                state.shutdown_token.cancel();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    std::process::exit(0);
                });
            }
        }
        Command::Logs => {
            if Some(cid) == state.admin_id {
                if let Ok(output) = std::process::Command::new("journalctl")
                    .args(&["-u", "kaspa-rust-bot.service", "-n", "25", "--no-pager"])
                    .output()
                {
                    let logs = String::from_utf8_lossy(&output.stdout);
                    let safe_logs = if logs.len() > 3900 {
                        &logs[logs.len() - 3900..]
                    } else {
                        &logs
                    };
                    bot.send_message(
                        msg.chat.id,
                        format!("📜 *Recent Logs:*\n```text\n{}\n```", safe_logs),
                    )
                    .parse_mode(ParseMode::Markdown)
                    .await?;
                } else {
                    bot.send_message(
                        msg.chat.id,
                        "❌ Failed to retrieve logs. Are you using systemd?",
                    )
                    .parse_mode(ParseMode::Markdown)
                    .await?;
                }
            }
        }
        Command::Broadcast(text) => {
            if Some(cid) == state.admin_id {
                let users = state.get_all_users();
                let mut count = 0;
                for user in &users {
                    if bot
                        .send_message(
                            ChatId(*user),
                            format!("📢 *Admin Announcement:*\n\n{}", text),
                        )
                        .parse_mode(ParseMode::Markdown)
                        .await
                        .is_ok()
                    {
                        count += 1;
                    }
                }
                bot.send_message(
                    msg.chat.id,
                    format!("✅ Broadcast sent to {} users.", count),
                )
                .parse_mode(ParseMode::Markdown)
                .await?;
            }
        }
    }
    Ok(())
}

async fn handle_cb(
    bot: Bot,
    q: CallbackQuery,
    state: Arc<AppState>,
    api: Arc<ApiManager>,
) -> ResponseResult<()> {
    let _ = bot.answer_callback_query(q.id).await;
    if let (Some(msg), Some(data)) = (q.message, q.data) {
        let cmd = match data.as_str() {
            "cmd_balance" => Command::Balance,
            "cmd_list" => Command::List,
            "cmd_price" => Command::Price,
            "cmd_market" => Command::Market,
            "cmd_network" => Command::Network,
            "cmd_supply" => Command::Supply,
            "cmd_fees" => Command::Fees,
            _ => Command::Start,
        };
        let _ = handle_cmd(bot, msg, cmd, state, api).await;
    }
    Ok(())
}
