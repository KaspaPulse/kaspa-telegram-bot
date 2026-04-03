pub mod dag_buffer;
use std::sync::Arc;
use teloxide::prelude::*;
use dotenvy::dotenv;
use tokio_util::sync::CancellationToken;
use secrecy::{SecretString, ExposeSecret};
use sqlx::sqlite::SqlitePoolOptions;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
    
    let file_appender = tracing_appender::rolling::never(".", "kaspa_bot.log");
    let (file_writer, _guard) = tracing_appender::non_blocking(file_appender);
    
    let stdout_layer = tracing_subscriber::fmt::layer().with_thread_ids(true);
    let file_layer = tracing_subscriber::fmt::layer().with_writer(file_writer).with_ansi(false);

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .init();
        
    tracing::info!("🚀 Starting Kaspa Rust Bot Engine (Enterprise Edition)...");

    let raw_token = std::env::var("BOT_TOKEN").or_else(|_| std::env::var("TELOXIDE_TOKEN")).expect("❌ FATAL ERROR: BOT_TOKEN is missing");
    let secret_token = SecretString::from(raw_token);
    tracing::info!("🔐 Bot Token securely loaded into Zeroized Memory.");

    let shutdown_token = CancellationToken::new();
    let db_pool = SqlitePoolOptions::new().max_connections(5).connect("sqlite:kaspa_bot.db?mode=rwc").await.expect("❌ DB Error");
        
    let state = Arc::new(AppState::new(shutdown_token.clone(), db_pool).await);
    let api = Arc::new(ApiManager::new());

    let token_clone = shutdown_token.clone();
    tokio::spawn(async move {
        if let Ok(_) = tokio::signal::ctrl_c().await {
            tracing::warn!("🛑 [SYSTEM] Received SIGINT (Ctrl+C). Initiating Graceful Shutdown...");
            token_clone.cancel();
        }
    });

    let gc_state = state.clone();
    let gc_shutdown = shutdown_token.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); 
        loop {
            tokio::select! {
                _ = gc_shutdown.cancelled() => { break; }
                _ = interval.tick() => {
                    let now = std::time::Instant::now();
                    gc_state.processed_txids.retain(|_, timestamp| now.duration_since(*timestamp).as_secs() < 86400);
                }
            }
        }
    });

    let bot_client = Bot::new(secret_token.expose_secret());
    let state_clone = state.clone();
    // bot_clone removed
    
    // api_clone removed
    let bot_for_kaspa = bot_client.clone();
    tokio::spawn(async move { kaspa::start_kaspa_listener(state_clone, bot_for_kaspa).await;
    });

    bot::start_telegram_bot(bot_client, state, api).await;
}
