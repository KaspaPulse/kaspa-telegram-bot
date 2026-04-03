pub mod dag_buffer;
use dotenvy::dotenv;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use teloxide::prelude::*;
use tokio_util::sync::CancellationToken;

pub mod api;
pub mod bot;
pub mod kaspa;
pub mod state;
pub mod utils;


use crate::state::AppState;

#[tokio::main]
async fn main() {
    // Load Environment Variables
    dotenv().ok();

    // [ENTERPRISE FIX] Initialize Tracing (Replaces old env_logger)
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(true)
        .with_level(true)
        .init();

    tracing::info!("🚀 Starting Kaspa Rust Bot Engine (Enterprise Edition)...");

    // [ENTERPRISE FIX] Secure Secret Management using SecretString
    let raw_token = std::env::var("BOT_TOKEN")
        .or_else(|_| std::env::var("TELOXIDE_TOKEN"))
        .expect("❌ FATAL ERROR: BOT_TOKEN is missing in .env file");

    let secret_token = SecretString::from(raw_token);
    tracing::info!("🔐 Bot Token securely loaded into Zeroized Memory.");

    // Global Graceful Shutdown Token
    let shutdown_token = CancellationToken::new();
        // [ENTERPRISE FIX] Establish Async Database Connection Pool
    let db_pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite:kaspa_bot.db?mode=rwc")
        .await
        .expect("❌ FATAL ERROR: Failed to create or connect to SQLite database");
        
    let state = Arc::new(AppState::new(shutdown_token.clone(), db_pool).await);
    

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
        let api_clone = api.clone(); kaspa::start_kaspa_engine(state_clone, bot_clone, api_clone).await;
    });

    // Start the Telegram Polling Engine
    bot::start_telegram_bot(bot_client, state).await;
}


