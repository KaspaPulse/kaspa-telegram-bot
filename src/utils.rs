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

pub fn make_golden_keyboard() -> teloxide::types::KeyboardMarkup { 
    teloxide::types::KeyboardMarkup::new(vec![ 
        vec![KeyboardButton::new("/balance"), KeyboardButton::new("/list")], 
        vec![KeyboardButton::new("/network"), KeyboardButton::new("/hashrate")], 
        vec![KeyboardButton::new("/price"), KeyboardButton::new("/market")], 
        vec![KeyboardButton::new("/supply"), KeyboardButton::new("/fees")], 
        vec![KeyboardButton::new("/dag"), KeyboardButton::new("/sys")] 
    ]).resize_keyboard() 
}

pub async fn send_and_log<T: AsRef<str>>(bot: &Bot, chat_id: ChatId, text: T, markup: Option<KeyboardMarkup>) -> anyhow::Result<()> { 
    if is_spam(chat_id.0) { tracing::warn!("[ANTI-SPAM] Blocked Chat: {}", chat_id.0); return Ok(()); } 
    let text_ref = text.as_ref(); 
    tracing::info!("[BOT OUT] Chat: {}", chat_id.0); 
    let mut req = bot.send_message(chat_id, text_ref.to_string()).parse_mode(teloxide::types::ParseMode::Html); 
    if let Some(m) = markup { req = req.reply_markup(m); } 
    req.await.context("API Error")?; 
    Ok(()) 
}

pub fn format_short_wallet(w: &str) -> String { 
    let chars: Vec<char> = w.chars().collect(); 
    if chars.len() > 18 { 
        let start: String = chars.iter().take(12).collect(); 
        let end: String = chars.iter().skip(chars.len().saturating_sub(6)).collect(); 
        format!("{}...{}", start, end) 
    } else { w.to_string() } 
}

pub fn format_hash(h: &str, link_type: &str) -> String {
    if h.len() > 16 && !h.contains("Searching") && !h.contains("Indexing") && h != "Not Found" {
        format!("<a href=\"https://kaspa.stream/{}/{}\">{}...{}</a>", link_type, h, h.chars().take(8).collect::<String>(), h.chars().skip(h.chars().count().saturating_sub(8)).collect::<String>())
    } else { format!("<code>{}</code>", h) }
}

pub fn is_spam(chat_id: i64) -> bool {
    static ADMIN_ID: std::sync::OnceLock<Option<i64>> = std::sync::OnceLock::new();
    let admin = ADMIN_ID.get_or_init(|| std::env::var("ADMIN_ID").ok().and_then(|val| val.trim().parse::<i64>().ok()) );
    if Some(chat_id) == *admin { return false; }
    static RATE_LIMITER: std::sync::OnceLock<dashmap::DashMap<i64, std::time::Instant>> = std::sync::OnceLock::new();
    let limiter = RATE_LIMITER.get_or_init(|| dashmap::DashMap::new());
    let now = std::time::Instant::now();
    if let Some(mut last_seen) = limiter.get_mut(&chat_id) {
        let elapsed = now.duration_since(*last_seen);
        if elapsed < std::time::Duration::from_millis(50) { return false; }
        if elapsed < std::time::Duration::from_secs(3) { return true; }
        *last_seen = now;
        return false;
    }
    limiter.insert(chat_id, now);
    false
}
