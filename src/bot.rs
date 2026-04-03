use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use std::sync::Arc;
use crate::state::AppState;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Kaspa Pulse Bot Commands:")]
pub enum Command {
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "View network DAG info")]
    Dag,
    #[command(description = "View circulating supply")]
    Supply,
    #[command(description = "View market price")]
    Price,
    #[command(description = "System diagnostics")]
    Sys,
}

pub async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let text = match msg.text() {
        Some(t) => t,
        None => return Ok(()),
    };

    if let Ok(command) = Command::parse(text, "RkKaspaPulse_bot") {
        let chat_id = msg.chat.id;

        match command {
            Command::Start => {
                let welcome = "<b>Kaspa Pulse Bot</b>\nSelect an option below:";
                bot.send_message(chat_id, welcome)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .reply_markup(crate::utils::helpers::main_keyboard())
                    .await?;
            }
            Command::Dag => {
                let msg = if let Some((net, blocks, headers, diff)) = crate::utils::local_rpc::LocalNodeRpc::get_dag_info().await {
                    format!("🔗 <b>Local Node DAG Info</b>\n━━━━━━━━━━━━━━━━━━\n<b>Network:</b> <code>{}</code>\n<b>Blocks:</b> <code>{}</code>\n<b>Headers:</b> <code>{}</code>\n<b>Difficulty:</b> <code>{:.2}</code>", net, blocks, headers, diff)
                } else {
                    "⚠️ <b>Error:</b> Failed to extract DAG parameters from Local Node.".to_string()
                };
                bot.send_message(chat_id, msg).parse_mode(teloxide::types::ParseMode::Html).await?;
            }
            Command::Supply => {
                let msg = if let Some((circ, max)) = crate::utils::local_rpc::LocalNodeRpc::get_supply().await {
                    format!("🏦 <b>Kaspa Supply (Local Node)</b>\n━━━━━━━━━━━━━━━━━━\n<b>Circulating:</b> <code>{:.0} KAS</code>\n<b>Max Supply:</b> <code>{:.0} KAS</code>", circ, max)
                } else {
                    "⚠️ <b>Error:</b> Failed to extract Supply parameters from Local Node.".to_string()
                };
                bot.send_message(chat_id, msg).parse_mode(teloxide::types::ParseMode::Html).await?;
            }
            Command::Price => {
                let price = state.api_manager.get_price().await.ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
                bot.send_message(chat_id, format!("💰 <b>Current Price:</b> <code>${:.4}</code>", price))
                    .parse_mode(teloxide::types::ParseMode::Html).await?;
            }
            Command::Sys => {
                let uptime = state.start_time.elapsed().as_secs();
                let sys_msg = format!("🖥️ <b>System Diagnostics</b>\n━━━━━━━━━━━━━━━━━━\n<b>Engine:</b> <code>ACTIVE</code>\n<b>Uptime:</b> <code>{}s</code>", uptime);
                bot.send_message(chat_id, sys_msg).parse_mode(teloxide::types::ParseMode::Html).await?;
            }
        }
    }
    Ok(())
}
pub async fn start_telegram_bot(bot: Bot, state: Arc<AppState>) {
    log::info!("Telegram Bot: Service started successfully.");
    
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}
