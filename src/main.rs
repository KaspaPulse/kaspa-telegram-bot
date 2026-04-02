#![allow(deprecated)]
use crate::bot::{handle_cmd, handle_smart_detect, Command};
use crate::state::AppState;
use std::sync::Arc;
use teloxide::prelude::*;

pub mod api;
pub mod bot;
pub mod dag_buffer;
pub mod kaspa;
pub mod state;
pub mod utils;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("Starting Kaspa Rust Bot...");

    let bot = Bot::from_env();
    let state = AppState::new();
    let state_arc = Arc::new(state);

    // Enterprise Dispatcher Tree
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(handle_cmd),
        )
        .branch(Update::filter_message().endpoint(handle_smart_detect));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state_arc])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}
