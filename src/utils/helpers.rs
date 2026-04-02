use once_cell::sync::Lazy;
use regex::Regex;

static KASPA_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^kaspa:[a-z0-9]{61,65}$").unwrap());

pub fn clean_and_validate_wallet(w: &str) -> Option<String> {
    let cleaned = w.trim();
    let formatted = if !cleaned.starts_with("kaspa:") {
        format!("kaspa:{}", cleaned)
    } else {
        cleaned.to_string()
    };

    if KASPA_REGEX.is_match(&formatted) {
        Some(formatted)
    } else {
        None
    }
}

pub fn format_short_wallet(address: &str) -> String {
    if address.is_empty() {
        return "N/A".to_string();
    }
    
    let parts: Vec<&str> = address.split(':').collect();
    if parts.len() == 2 {
        let prefix = parts[0];
        let payload = parts[1];
        
        if payload.len() > 12 {
            let start = &payload[0..3];
            let end = &payload[payload.len() - 6..];
            return format!("{}:{}...{}", prefix, start, end);
        }
    }
    address.to_string()
}

pub fn format_hashrate(val: f64) -> String {
    if val.is_nan() || val == 0.0 {
        return "0.00 TH/s".to_string();
    }
    if val < 1_000.0 {
        format!("{:.2} TH/s", val)
    } else if val < 1_000_000.0 {
        format!("{:.2} PH/s", val / 1_000.0)
    } else {
        format!("{:.2} EH/s", val / 1_000_000.0)
    }
}

#[allow(dead_code)]
pub fn format_difficulty(val: f64) -> String {
    if val == 0.0 {
        return "0.00".to_string();
    }
    if val >= 1e15 {
        format!("{:.2} P", val / 1e15)
    } else if val >= 1e12 {
        format!("{:.2} T", val / 1e12)
    } else if val >= 1e9 {
        format!("{:.2} G", val / 1e9)
    } else {
        // Simple 2 decimal formatting for lower numbers
        format!("{:.2}", val)
    }
}

