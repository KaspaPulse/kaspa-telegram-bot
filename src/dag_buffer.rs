use std::sync::Arc;
use crate::state::AppState;
use tokio::time::{sleep, Duration};
use teloxide::prelude::*;

pub struct DagBuffer;

impl DagBuffer {
    pub fn new() -> Self { Self }

    pub async fn handle_new_utxo(address: String, amount: String, state: Arc<AppState>, bot: Bot) {
        log::info!("[DAG-BUFFER] Processing new UTXO for: {} | Amount: {}", address, amount);
        
        sleep(Duration::from_secs(10)).await;
        
        // Extract chat_ids from the WalletData struct returned by state
        if let Some(wallet_ref) = state.monitored_wallets.get(&address) {
            for &chat_id in &wallet_ref.value().chat_ids {
                let msg = format!(
                    "⚡ <b>New Reward Detected!</b>\n━━━━━━━━━━━━━━━━━━\n<b>Wallet:</b> <code>{}</code>\n<b>Amount:</b> <code>{} KAS</code>",
                    address, amount
                );
                
                let _ = bot.send_message(teloxide::types::ChatId(chat_id), msg)
                           .parse_mode(teloxide::types::ParseMode::Html)
                           .await;
                
                log::info!("[NOTIFICATION] Alert sent to ChatID: {}", chat_id);
            }
        }
    }
}