use anyhow::Context;
use teloxide::{prelude::*, types::{ChatId, KeyboardMarkup, KeyboardButton}};


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

pub async fn send_and_log<T: AsRef<str>>(bot: &Bot, chat_id: ChatId, text: T, markup: Option<KeyboardMarkup>) -> anyhow::Result<()> { if is_spam(chat_id.0) { tracing::warn!("[ANTI-SPAM] Rate limit exceeded and message blocked for Chat: {}", chat_id.0); return Ok(()); } let text_ref = text.as_ref(); let mut log_text = text_ref.replace("\n", " | "); if let Ok(re) = regex::Regex::new(r"(kaspa:[a-z0-9]{8})[a-z0-9]+([a-z0-9]{6})") { log_text = re.replace_all(&log_text, "${1}...[REDACTED]...${2}").to_string(); } tracing::info!("[BOT OUT] Chat: {} | Msg: {}", chat_id.0, log_text); let mut req = bot.send_message(chat_id, text_ref.to_string()).parse_mode(teloxide::types::ParseMode::Html); if let Some(m) = markup { req = req.reply_markup(m); } req.await.context("Failed to send message to Telegram API")?; Ok(()) }

pub fn format_short_wallet(w: &str) -> String { let chars: Vec<char> = w.chars().collect(); if chars.len() > 18 { let start: String = chars.iter().take(12).collect(); let end: String = chars.iter().skip(chars.len().saturating_sub(6)).collect(); format!("{}...{}", start, end) } else { w.to_string() } }

pub fn format_hash(h: &str, link_type: &str) -> String {
    if h.len() > 16 && !h.contains("Searching") && !h.contains("Indexing") && h != "Not Found" {
        format!("<a href=\"https://kaspa.stream/{}/{}\">{}...{}</a>", link_type, h, h.chars().take(8).collect::<String>(), h.chars().skip(h.chars().count().saturating_sub(8)).collect::<String>())
    } else {
        format!("<code>{}</code>", h)
    }
}












pub fn is_spam(chat_id: i64) -> bool { static RATE_LIMITER: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<i64, std::time::Instant>>> = std::sync::OnceLock::new(); let limiter_mutex = RATE_LIMITER.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new())); let mut limiter = limiter_mutex.lock().unwrap(); let admin_id: i64 = 1725327216; if chat_id == admin_id { return false; } let now = std::time::Instant::now(); if let Some(&last) = limiter.get(&chat_id) { if now.duration_since(last) < std::time::Duration::from_secs(3) { return true; } } limiter.insert(chat_id, now); false }

#[allow(dead_code)]
pub fn is_valid_address(addr: &str) -> bool { let re = regex::Regex::new("^kaspa:[a-z0-9]{61,63}$").unwrap(); re.is_match(addr) }

