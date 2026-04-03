use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents raw Sompi (the smallest atomic unit of Kaspa). Guaranteed to be a whole number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Sompi(pub u64);

/// Represents formatted Kaspa (used for UI and standard mathematics).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Kaspa(pub f64);

// Implement strict, zero-cost conversions
impl From<Sompi> for Kaspa {
    fn from(s: Sompi) -> Self {
        Kaspa(s.0 as f64 / 100_000_000.0)
    }
}

impl From<Kaspa> for Sompi {
    fn from(k: Kaspa) -> Self {
        Sompi((k.0 * 100_000_000.0) as u64)
    }
}

// Automatically format Kaspa to 8 decimal places whenever printed
impl fmt::Display for Kaspa {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.8}", self.0)
    }
}
