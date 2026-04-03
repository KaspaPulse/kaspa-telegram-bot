#![allow(deprecated)]
use std::sync::Arc;
use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use serde_json::{json, Value};
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::{mpsc, Semaphore};
use std::time::Duration;
use chrono::Utc;

use crate::state::AppState;
use crate::utils::helpers::format_short_wallet;
use crate::api::ApiManager;
use crate::utils::types::{Sompi, Kaspa};

// Event Payload Struct
#[derive(Debug, Clone)]
pub struct UtxoEvent {
    pub tx_id: String,
    pub address: String,
    pub amount: Sompi,
    pub daa_score: u64,
}

#[tracing::instrument(skip(state, bot, api))]
async fn process_reward_event(event: UtxoEvent, state: Arc<AppState>, bot: Bot, api: Arc<ApiManager>) {
    let exact_reward: Kaspa = event.amount.into();
    let mut live_bal = Kaspa(0.0);

    if let Ok(api_bal) = api.get_balance(&event.address).await {
        live_bal = Kaspa(api_bal);
    }

    if let Some(mut wallet) = state.monitored_wallets.get_mut(&event.address) {
        if live_bal.0 == 0.0 { 
            live_bal = Kaspa(wallet.last_balance + exact_reward.0); 
        }
        wallet.last_balance = live_bal.0;
    }

    state.sync_wallet_to_db(&event.address).await;

    let dt_str = format!("{} UTC", Utc::now().format("%Y-%m-%d %H:%M:%S"));
    let short_wallet = format_short_wallet(&event.address);

    // Note: _b tells the compiler we are intentionally ignoring this parameter
    let build_msg = |_b: &str, a: &str, m: &str| -> String {
        format!("⚡ *Native Node Reward!* 💎\n━━━━━━━━━━━━━━━━━━\n*Time:* `{}`\n*Wallet:* [{}]({})\n*Amount:* `+{} KAS`\n*Live Balance:* `{} KAS`\n━━━━━━━━━━━━━━━━━━\n*Mined Block:* {}\n*Accepting Block:* {}\n*DAA Score:* `{}`", 
        dt_str, short_wallet, format!("https://kaspa.stream/addresses/{}", event.address), exact_reward, live_bal, m, a, event.daa_score)
    };

    let subscribers = match state.monitored_wallets.get(&event.address) { Some(w) => w.chat_ids.clone(), None => return };

    for chat_id in subscribers {
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = bot.send_message(ChatId(chat_id), build_msg("...", "...", "...")).parse_mode(ParseMode::Markdown).await;
    }
}

    // 2. Safely update RAM state
    if let Some(mut wallet) = state.monitored_wallets.get_mut(&event.address) {
        if live_bal.0 == 0.0 { live_bal = Kaspa(wallet.last_balance + exact_reward.0); }
        wallet.last_balance = live_bal.0;
    }
    
    // 3. Atomic SQLite DB flush for this specific wallet
    state.sync_wallet_to_db(&event.address).await;

    let dt_str = format!("{} UTC", Utc::now().format("%Y-%m-%d %H:%M:%S"));
    let short_wallet = format_short_wallet(&event.address);
    
    let build_msg = |b: &str, a: &str, m: &str| -> String {
        format!("⚡ *Native Node Reward!* 💎\n━━━━━━━━━━━━━━━━━━\n*Time:* `{}`\n*Wallet:* [{}]({})\n*Amount:* `+{} KAS`\n*Live Balance:* `{} KAS`\n━━━━━━━━━━━━━━━━━━\n*Mined Block:* {}\n*Accepting Block:* {}\n*DAA Score:* `{}`", 
        dt_str, short_wallet, format!("https://kaspa.stream/addresses/{}", event.address), exact_reward, live_bal, m, a, event.daa_score)
    };

    let subscribers = match state.monitored_wallets.get(&event.address) { Some(w) => w.chat_ids.clone(), None => return };
    
    for chat_id in subscribers {
        // [BEST PRACTICE] Telegram API Rate Limiting: Max 30 msgs/sec
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = bot.send_message(ChatId(chat_id), build_msg("...", "...", "...")).parse_mode(ParseMode::MarkdownV2).await;
    }
}

fn extract_tx_id(entry: &Value) -> Option<String> { entry.get("outpoint").and_then(|o| o.get("transactionId")).and_then(|v| v.as_str().map(|s| s.to_string())) }
fn extract_address(entry: &Value) -> Option<String> { entry.get("address").and_then(|v| v.as_str().map(|s| s.to_string())) }
fn extract_amount(entry: &Value) -> Option<Sompi> { entry.get("utxoEntry").and_then(|u| u.get("amount")).and_then(|v| v.as_u64()).map(Sompi) }
fn extract_daa_score(entry: &Value) -> Option<u64> { entry.get("utxoEntry").and_then(|u| u.get("blockDaaScore")).and_then(|v| v.as_u64()) }





