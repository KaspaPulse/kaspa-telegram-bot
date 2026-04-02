#![allow(deprecated)]
use teloxide::prelude::*;
use std::sync::Arc;
use crate::state::AppState;
use crate::bot::{Command, handle_cmd, handle_smart_detect};

pub mod api;
pub mod bot;
pub mod kaspa;
pub mod state;
pub mod utils;
pub mod dag_buffer;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    if let Ok(token) = std::env::var("BOT_TOKEN") {
        std::env::set_var("TELOXIDE_TOKEN", token);
    }
    
    pretty_env_logger::init();
    log::info!("Starting Kaspa Rust Bot...");

    let bot = Bot::from_env();
    let state = AppState::new();
    let state_arc = Arc::new(state);

    // Enterprise Dispatcher with proper Dependency Injection
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(handle_cmd),
        )
        .branch(
            Update::filter_message()
                .endpoint(handle_smart_detect),
        );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            state_arc.clone(), 
            state_arc.api_manager.clone()
        ])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}