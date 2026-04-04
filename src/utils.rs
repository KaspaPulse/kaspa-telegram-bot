use teloxide::{prelude::*, types::{ChatId, KeyboardMarkup, KeyboardButton}};
use tracing::info;

pub fn f_num(n: f64) -> String {
    let s = format!("{:.0}", n);
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        result.push(c);
        if (len - i - 1) % 3 == 0 && i != len - 1 { result.push(','); }
    }
    result
}

pub fn make_golden_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![
        vec![KeyboardButton::new("/balance"), KeyboardButton::new("/network")],
        vec![KeyboardButton::new("/fees"), KeyboardButton::new("/supply")],
        vec![KeyboardButton::new("/price"), KeyboardButton::new("/market")],
        vec![KeyboardButton::new("/dag"), KeyboardButton::new("/list")],
    ]).resize_keyboard()
}

pub async fn send_and_log(bot: &Bot, chat_id: ChatId, text: String, markup: Option<KeyboardMarkup>) -> anyhow::Result<()> {
    let log_text = text.replace('\n', " | ");
    info!("[BOT OUT] Chat: {} | Msg: {}", chat_id.0, log_text);
    let mut req = bot.send_message(chat_id, text).parse_mode(teloxide::types::ParseMode::Html);
    if let Some(m) = markup { req = req.reply_markup(m); }
    req.await?;
    Ok(())
}

pub fn format_short_wallet(w: &str) -> String {
    if w.len() > 18 { format!("{}...{}", &w[..12], &w[w.len()-6..]) } else { w.to_string() }
}

pub fn format_hash(h: &str, link_type: &str) -> String {
    if h.len() > 16 && !h.contains("Searching") && !h.contains("Indexing") && h != "Not Found" {
        format!("<a href=\"https://kaspa.stream/{}/{}\">{}...{}</a>", link_type, h, &h[..8], &h[h.len()-8..])
    } else {
        format!("<code>{}</code>", h)
    }
}