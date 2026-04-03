use serde_json::Value;
use std::time::{Duration, Instant};
use dashmap::DashMap;
use reqwest::Client;
use crate::utils::errors::AppResult;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum CacheType { Price, Market, Fees, Network, Supply, Dag }

struct CacheEntry { data: Value, timestamp: Instant }

pub struct ApiManager {
    pub client: Client,
    cache: DashMap<CacheType, CacheEntry>,
}

impl ApiManager {
    pub fn new() -> Self {
        Self {
            client: Client::builder().timeout(Duration::from_secs(5)).build().unwrap_or_default(),
            cache: DashMap::new(),
        }
    }

    async fn fetch_with_cache(&self, url: &str, cache_type: CacheType) -> AppResult<Value> {
        if let Some(entry) = self.cache.get(&cache_type) {
            if entry.timestamp.elapsed() < Duration::from_secs(60) {
                return Ok(entry.value().data.clone());
            }
        }
        let res = self.client.get(url).send().await?.json::<Value>().await?;
        self.cache.insert(cache_type, CacheEntry { data: res.clone(), timestamp: Instant::now() });
        Ok(res)
    }

    pub async fn get_price(&self) -> AppResult<Value> { self.fetch_with_cache("https://api.kaspa.org/info/price", CacheType::Price).await }
    #[allow(dead_code)]
    pub async fn get_market(&self) -> AppResult<Value> { self.fetch_with_cache("https://api.kaspa.org/info/marketcap", CacheType::Market).await }
    #[allow(dead_code)]
    pub async fn get_fees(&self) -> AppResult<Value> { self.fetch_with_cache("https://api.kaspa.org/info/fee-estimate", CacheType::Fees).await }
    pub async fn get_network(&self) -> AppResult<Value> { self.fetch_with_cache("https://api.kaspa.org/info/hashrate", CacheType::Network).await }
    
    // Fixed Endpoints to fetch strict JSON objects
    pub async fn get_supply(&self) -> AppResult<Value> { self.fetch_with_cache("https://api.kaspa.org/info/coinsupply", CacheType::Supply).await }
    pub async fn get_dag_info(&self) -> AppResult<Value> { self.fetch_with_cache("https://api.kaspa.org/info/blockdag", CacheType::Dag).await }
    
    pub async fn get_balance(&self, addr: &str) -> AppResult<f64> {
        let url = format!("https://api.kaspa.org/addresses/{}/balance", addr);
        let res = self.client.get(&url).send().await?.json::<Value>().await?;
        let balance = res.get("balance").and_then(|v| v.as_f64()).map(|b| b / 100_000_000.0).unwrap_or(0.0);
        Ok(balance)
    }
}


