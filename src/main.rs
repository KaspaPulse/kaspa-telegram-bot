use std::sync::Arc;
use teloxide::prelude::*;
use dotenvy::dotenv;
use tokio_util::sync::CancellationToken;

pub mod api;
pub mod bot;
pub mod kaspa;
pub mod state;
pub mod utils;

use crate::state::AppState;
use crate::api::ApiManager;

#[tokio::main]
async fn main() {
    dotenv().ok();
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    log::info!("🚀 Starting Kaspa Rust Bot Engine (Enterprise Edition)...");

    // [ENTERPRISE FIX] Initialize Global Cancellation Token
    let shutdown_token = CancellationToken::new();
    let state = Arc::new(AppState::new(shutdown_token.clone()));
    let api = Arc::new(ApiManager::new());

    // [ENTERPRISE FIX] Hook into OS Signals (Ctrl+C)
    let token_clone = shutdown_token.clone();
    tokio::spawn(async move {
        if let Ok(_) = tokio::signal::ctrl_c().await {
            log::warn!("🛑 [SYSTEM] Received SIGINT (Ctrl+C). Initiating Graceful Shutdown...");
            token_clone.cancel();
        }
    });

    let bot_client = Bot::from_env();
    let state_clone = state.clone();
    let bot_clone = bot_client.clone();
    
    tokio::spawn(async move {
        kaspa::start_kaspa_engine(state_clone, bot_clone).await;
    });

    bot::start_telegram_bot(bot_client, state, api).await;
}
