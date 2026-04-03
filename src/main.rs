use std::sync::Arc;
use teloxide::prelude::*;
use dotenvy::dotenv;
use tokio_util::sync::CancellationToken;
use secrecy::{Secret, ExposeSecret};

pub mod api;
pub mod bot;
pub mod kaspa;
pub mod state;
pub mod utils;

use crate::state::AppState;
use crate::api::ApiManager;

#[tokio::main]
async fn main() {
    // 1. Load Environment Variables
    dotenv().ok();
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    tracing::info!("🚀 Starting Kaspa Rust Bot Engine (Enterprise Edition)...");

    // [ENTERPRISE FIX] Secure Secret Management
    // Read the token and immediately wrap it in a memory-safe Secret String
    let raw_token = std::env::var("BOT_TOKEN")
        .or_else(|_| std::env::var("TELOXIDE_TOKEN"))
        .expect("❌ FATAL ERROR: BOT_TOKEN is missing in .env file");
        
    let secret_token = Secret::new(raw_token);
    tracing::info!("🔐 Bot Token securely loaded into Zeroized Memory.");

    // [ENTERPRISE FIX] Global Graceful Shutdown Token
    let shutdown_token = CancellationToken::new();
    let state = Arc::new(AppState::new(shutdown_token.clone()));
    let api = Arc::new(ApiManager::new());

    // Hook OS Signals (Ctrl+C) to the Cancellation Token
    let token_clone = shutdown_token.clone();
    tokio::spawn(async move {
        if let Ok(_) = tokio::signal::ctrl_c().await {
            tracing::warn!("🛑 [SYSTEM] Received SIGINT (Ctrl+C). Initiating Graceful Shutdown...");
            token_clone.cancel();
        }
    });

    // Extract the token safely ONLY at the exact moment of initializing the Bot client
    let bot_client = Bot::new(secret_token.expose_secret());
    
    let state_clone = state.clone();
    let bot_clone = bot_client.clone();
    
    // Spawn the Kaspa wRPC Engine in the background
    tokio::spawn(async move {
        kaspa::start_kaspa_engine(state_clone, bot_clone).await;
    });

    // Start the Telegram Polling Engine
    bot::start_telegram_bot(bot_client, state, api).await;
}

