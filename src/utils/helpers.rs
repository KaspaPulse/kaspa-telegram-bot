pub fn clean_and_validate_wallet(address: &str) -> Option<String> {
    let cleaned = address.trim().to_lowercase();
    let target = if cleaned.starts_with("kaspa:") { 
        cleaned 
    } else { 
        format!("kaspa:{}", cleaned) 
    };

    let parts: Vec<&str> = target.split(':').collect();
    if parts.len() != 2 || parts[0] != "kaspa" { 
        return None; 
    }

    let payload = parts[1];
    
    // Kaspa payloads are exactly 61 chars (P2PKH) or 63 chars (P2SH)
    if payload.len() != 61 && payload.len() != 63 { 
        return None; 
    }

    // Strict Bech32 alphabet check (Kaspa deliberately excludes 1, b, i, o)
    let bech32_alphabet = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    if !payload.chars().all(|c| bech32_alphabet.contains(c)) { 
        return None; 
    }

    Some(target)
}

pub fn format_short_wallet(address: &str) -> String {
    if address.is_empty() { return "N/A".to_string(); }
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
    if val.is_nan() || val == 0.0 { return "0.00 TH/s".to_string(); }
    if val < 1_000.0 { format!("{:.2} TH/s", val) } 
    else if val < 1_000_000.0 { format!("{:.2} PH/s", val / 1_000.0) } 
    else { format!("{:.2} EH/s", val / 1_000_000.0) }
}

#[allow(dead_code)]
pub fn format_difficulty(val: f64) -> String {
    if val == 0.0 { return "0.00".to_string(); }
    if val >= 1_000_000_000_000_000.0 { format!("{:.2} P", val / 1_000_000_000_000_000.0) }
    else if val >= 1_000_000_000_000.0 { format!("{:.2} T", val / 1_000_000_000_000.0) }
    else if val >= 1_000_000_000.0 { format!("{:.2} G", val / 1_000_000_000.0) }
    else { format!("{:.2}", val) }
}
