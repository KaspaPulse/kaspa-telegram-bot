use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use serde::Deserialize;
use std::time::Duration;

// ==========================================
// 1. Main Menu Inline Keyboard Design
// ==========================================
pub fn main_menu_markup() -> InlineKeyboardMarkup {
    let row1 = vec![
        InlineKeyboardButton::callback("💰 My Balances", "cmd_balance"),
        InlineKeyboardButton::callback("📋 Tracked Wallets", "cmd_list"),
    ];
    let row2 = vec![
        InlineKeyboardButton::callback("💵 KAS Price", "cmd_price"),
        InlineKeyboardButton::callback("📈 Market Cap", "cmd_market"),
    ];
    let row3 = vec![
        InlineKeyboardButton::callback("🌐 Network Stats", "cmd_network"),
        InlineKeyboardButton::callback("⛽ Mempool Fees", "cmd_fees"),
    ];
    let row4 = vec![
        InlineKeyboardButton::callback("🪙 Coin Supply", "cmd_supply"),
        InlineKeyboardButton::callback("📦 BlockDAG Details", "cmd_dag"),
    ];

    InlineKeyboardMarkup::new(vec![row1, row2, row3, row4])
}

// ==========================================
// 2. Hashrate and Difficulty Formatting
// ==========================================
pub fn format_hashrate(val: f64) -> String {
    if val <= 0.0 {
        return "0.00 TH/s".to_string();
    }
    if val < 1000.0 {
        format!("{:.2} TH/s", val)
    } else if val < 1_000_000.0 {
        format!("{:.2} PH/s", val / 1000.0)
    } else {
        format!("{:.2} EH/s", val / 1_000_000.0)
    }
}

pub fn format_difficulty(val: f64) -> String {
    if val <= 0.0 {
        return "0.00".to_string();
    }
    if val >= 1e15 {
        format!("{:.2} P", val / 1e15)
    } else if val >= 1e12 {
        format!("{:.2} T", val / 1e12)
    } else if val >= 1e9 {
        format!("{:.2} G", val / 1e9)
    } else {
        format!("{:.2}", val)
    }
}

// ==========================================
// 3. Fallback System: Fetching Hashrate via API
// ==========================================
#[derive(Deserialize)]
pub struct HashrateResponse {
    pub hashrate: f64,
}

pub async fn fetch_hashrate_api() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Set a 5-second timeout to prevent blocking the bot
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
        
    let resp = client.get("https://api.kaspa.org/info/hashrate")
        .send()
        .await?
        .json::<HashrateResponse>()
        .await?;
    
    Ok(format_hashrate(resp.hashrate))
}
