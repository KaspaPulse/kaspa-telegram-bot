use std::sync::Arc;
use teloxide::prelude::*;
use dotenvy::dotenv;

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

    let bot_client = Bot::from_env();
    let state = Arc::new(AppState::new());
    let api = Arc::new(ApiManager::new());

    let state_clone = state.clone();
    let bot_clone = bot_client.clone();
    
    // تشغيل محرك كاسبا في مسار خلفي مستقل
    tokio::spawn(async move {
        kaspa::start_kaspa_engine(state_clone, bot_clone).await;
    });

    // تشغيل محرك التيليجرام بشكل متزامن
    bot::start_telegram_bot(bot_client, state, api).await;
}

