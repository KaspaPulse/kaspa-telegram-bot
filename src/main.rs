#![allow(deprecated)]
use teloxide::payloads::SendMessageSetters;
mod api;
mod bot;
mod kaspa;
mod state;
mod utils;
use crate::api::ApiManager;
use crate::state::AppState;
use dotenvy::dotenv;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use teloxide::Bot;

#[tokio::main]
async fn main() {
    dotenv().ok();
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    log::info!("🚀 Starting Kaspa Rust Bot Engine (Enterprise Edition)...");

    let state = Arc::new(AppState::new());
    let api = Arc::new(ApiManager::new());
    let admin_id: Option<i64> = std::env::var("ADMIN_ID")
        .ok()
        .and_then(|id| id.parse().ok());

    state.load_data(admin_id);
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("CRITICAL: BOT_TOKEN must be set"));

    // Check restart flag
    if fs::metadata(".restart_flag").is_ok() {
        if let Ok(id_str) = fs::read_to_string(".restart_flag") {
            if let Ok(chat_id) = id_str.trim().parse::<i64>() {
                use teloxide::requests::Requester;
                let _ = bot
                    .send_message(
                        teloxide::types::ChatId(chat_id),
                        "✅ *Restart Successful!*\nSystem is back online at full capacity.",
                    )
                    .parse_mode(teloxide::types::ParseMode::Markdown)
                    .await;
            }
        }
        let _ = fs::remove_file(".restart_flag");
    }

    let kaspa_state = state.clone();
    let kaspa_bot = bot.clone();
    tokio::spawn(async move {
        kaspa::start_kaspa_engine(kaspa_state, kaspa_bot).await;
    });

    // [NEW] Background Memory Cleanup Task
    let cleanup_state = state.clone();
    let cleanup_api = api.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Ok(dag) = cleanup_api.get_dag().await {
                if let Some(current_daa) = dag.get("virtualDaaScore").and_then(|v| v.as_u64()) {
                    cleanup_state.cleanup_old_trackers(current_daa);
                }
            }
        }
    });

    bot::start_telegram_bot(bot, state.clone(), api.clone()).await;
}
