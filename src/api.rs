use reqwest::Client;
use serde_json::Value;
use std::time::{Instant, Duration};
use tokio::sync::RwLock;

const CACHE_TTL_SECONDS: u64 = 60;

#[derive(Clone)]
pub struct CachedData { pub data: Value, pub fetched_at: Instant }

pub struct ApiManager {
    client: Client,
    price_cache: RwLock<Option<CachedData>>,
    market_cache: RwLock<Option<CachedData>>,
    fees_cache: RwLock<Option<CachedData>>,
    network_cache: RwLock<Option<CachedData>>,
    supply_cache: RwLock<Option<CachedData>>,
    dag_cache: RwLock<Option<CachedData>>,
}

enum CacheType { Price, Market, Fees, Network, Supply, Dag }

impl ApiManager {
    pub fn new() -> Self {
        Self {
            client: Client::builder().timeout(Duration::from_secs(5)).build().unwrap(),
            price_cache: RwLock::new(None), market_cache: RwLock::new(None),
            fees_cache: RwLock::new(None), network_cache: RwLock::new(None),
            supply_cache: RwLock::new(None), dag_cache: RwLock::new(None),
        }
    }

    pub async fn get_price(&self) -> Result<Value, reqwest::Error> { self.fetch_with_cache("https://api.kaspa.org/info/price", CacheType::Price).await }
    pub async fn get_market(&self) -> Result<Value, reqwest::Error> { self.fetch_with_cache("https://api.kaspa.org/info/marketcap", CacheType::Market).await }
    pub async fn get_fees(&self) -> Result<Value, reqwest::Error> { self.fetch_with_cache("https://api.kaspa.org/info/fee-estimate", CacheType::Fees).await }
    pub async fn get_network(&self) -> Result<Value, reqwest::Error> { self.fetch_with_cache("https://api.kaspa.org/info/hashrate", CacheType::Network).await }
    pub async fn get_supply(&self) -> Result<Value, reqwest::Error> { self.fetch_with_cache("https://api.kaspa.org/info/coinsupply", CacheType::Supply).await }
    pub async fn get_dag(&self) -> Result<Value, reqwest::Error> { self.fetch_with_cache("https://api.kaspa.org/info/blockdag", CacheType::Dag).await }

    pub async fn get_balance(&self, address: &str) -> Result<f64, reqwest::Error> {
        let url = format!("https://api.kaspa.org/addresses/{}/balance", address);
        let res = self.client.get(&url).send().await?.json::<Value>().await?;
        let bal = res.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
        Ok(bal / 100_000_000.0)
    }

    async fn fetch_with_cache(&self, url: &str, cache_type: CacheType) -> Result<Value, reqwest::Error> {
        let cache_lock = match cache_type {
            CacheType::Price => &self.price_cache, CacheType::Market => &self.market_cache,
            CacheType::Fees => &self.fees_cache, CacheType::Network => &self.network_cache,
            CacheType::Supply => &self.supply_cache, CacheType::Dag => &self.dag_cache,
        };

        {
            let cache = cache_lock.read().await;
            if let Some(cached) = &*cache {
                if cached.fetched_at.elapsed().as_secs() < CACHE_TTL_SECONDS { return Ok(cached.data.clone()); }
            }
        }

        match self.client.get(url).send().await {
            Ok(response) => {
                let data = response.json::<Value>().await?;
                let mut cache = cache_lock.write().await;
                *cache = Some(CachedData { data: data.clone(), fetched_at: Instant::now() });
                Ok(data)
            }
            Err(e) => {
                let cache = cache_lock.read().await;
                if let Some(cached) = &*cache { Ok(cached.data.clone()) } else { Err(e) }
            }
        }
    }
}
