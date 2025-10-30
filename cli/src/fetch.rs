use std::{thread, time};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use definitions::network_specs::NetworkSpecs;
use generate_message::helpers::{meta_fetch, specs_agnostic, MetaFetched};
use generate_message::parser::Token;
use log::{info, warn};
use tokio::time::Duration;

use crate::common::types::get_crypto;
use crate::config::Chain;

pub(crate) trait Fetcher {
    fn fetch_specs(&self, chain: &Chain) -> Result<NetworkSpecs>;
    fn fetch_metadata(&self, chain: &Chain) -> Result<MetaFetched>;
}

#[async_trait]
pub(crate) trait AsyncFetcher: Send + Sync {
    async fn fetch_specs(&self, chain: &Chain) -> Result<NetworkSpecs>;
    async fn fetch_metadata(&self, chain: &Chain) -> Result<MetaFetched>;
}

// try to call all urls unless successful
fn call_urls<F, T>(urls: &[String], f: F) -> Result<T>
where
    F: Fn(&str) -> Result<T, generate_message::Error>,
{
    for url in urls.iter() {
        for i in 1..7 {
            match f(url) {
                Ok(res) => return Ok(res),
                Err(e) => warn!("Failed to fetch {}: {:?}", url, e),
            }
            let interval_seconds = time::Duration::from_secs(5 * i);
            thread::sleep(interval_seconds);
        }
    }
    bail!("Error calling chain node");
}

pub(crate) struct RpcFetcher;

impl Fetcher for RpcFetcher {
    fn fetch_specs(&self, chain: &Chain) -> Result<NetworkSpecs> {
        let specs = call_urls(&chain.rpc_endpoints, |url| {
            let optional_token_override = chain.token_decimals.zip(chain.token_unit.as_ref()).map(
                |(token_decimals, token_unit)| Token {
                    decimals: token_decimals,
                    unit: token_unit.to_string(),
                },
            );

            specs_agnostic(url, get_crypto(chain), optional_token_override, None)
        })
        .map_err(|e| anyhow!("{:?}", e))?;
        if specs.name.to_lowercase() != chain.name {
            bail!(
                "Network name mismatch. Expected {}, got {}. Please fix it in `config.toml`",
                chain.name,
                specs.name
            )
        }
        Ok(specs)
    }

    fn fetch_metadata(&self, chain: &Chain) -> Result<MetaFetched> {
        let meta = call_urls(&chain.rpc_endpoints, meta_fetch).map_err(|e| anyhow!("{:?}", e))?;
        if meta.meta_values.name.to_lowercase() != chain.name {
            bail!(
                "Network name mismatch. Expected {}, got {}. Please fix it in `config.toml`",
                chain.name,
                meta.meta_values.name
            )
        }
        Ok(meta)
    }
}

pub(crate) struct ConfigRpcFetcher;

impl Fetcher for ConfigRpcFetcher {
    fn fetch_specs(&self, chain: &Chain) -> Result<NetworkSpecs> {
        let specs = call_urls(&chain.rpc_endpoints, |url| {
            let optional_token_override = chain.token_decimals.zip(chain.token_unit.as_ref()).map(
                |(token_decimals, token_unit)| Token {
                    decimals: token_decimals,
                    unit: token_unit.to_string(),
                },
            );

            specs_agnostic(url, get_crypto(chain), optional_token_override, None)
        })
        .map_err(|e| anyhow!("{:?}", e))?;
        Ok(specs)
    }

    fn fetch_metadata(&self, _chain: &Chain) -> Result<MetaFetched> {
        bail!("Not implemented!");
    }
}

// Async implementation with optimized retry logic
async fn call_urls_async<F, Fut, T>(urls: &[String], f: F) -> Result<T>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, generate_message::Error>>,
{
    for url in urls.iter() {
        // Optimized retry with exponential backoff (max 3 attempts instead of 6)
        for attempt in 1..=3 {
            match f(url.clone()).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    if attempt < 3 {
                        warn!("Attempt {}/3 failed for {}: {:?}, retrying...", attempt, url, e);
                        // Exponential backoff: 2s, 4s
                        tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
                    } else {
                        warn!("All attempts failed for {}: {:?}", url, e);
                    }
                }
            }
        }
    }
    bail!("Error calling chain node - all endpoints failed");
}

pub(crate) struct AsyncRpcFetcher;

#[async_trait]
impl AsyncFetcher for AsyncRpcFetcher {
    async fn fetch_specs(&self, chain: &Chain) -> Result<NetworkSpecs> {
        let chain_clone = chain.clone();
        let specs = call_urls_async(&chain.rpc_endpoints, move |url| {
            let chain = chain_clone.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let optional_token_override = chain.token_decimals.zip(chain.token_unit.as_ref()).map(
                        |(token_decimals, token_unit)| Token {
                            decimals: token_decimals,
                            unit: token_unit.to_string(),
                        },
                    );
                    specs_agnostic(&url, get_crypto(&chain), optional_token_override, None)
                })
                .await
                .map_err(|e| generate_message::Error::SpecsTransfer(format!("Task join error: {}", e)))?
            }
        })
        .await
        .map_err(|e| anyhow!("{:?}", e))?;
        
        if specs.name.to_lowercase() != chain.name {
            bail!(
                "Network name mismatch. Expected {}, got {}. Please fix it in `config.toml`",
                chain.name,
                specs.name
            )
        }
        Ok(specs)
    }

    async fn fetch_metadata(&self, chain: &Chain) -> Result<MetaFetched> {
        let chain_clone = chain.clone();
        let meta = call_urls_async(&chain.rpc_endpoints, move |url| {
            async move {
                tokio::task::spawn_blocking(move || meta_fetch(&url))
                    .await
                    .map_err(|e| generate_message::Error::MetaFetch(format!("Task join error: {}", e)))?
            }
        })
        .await
        .map_err(|e| anyhow!("{:?}", e))?;
        
        if meta.meta_values.name.to_lowercase() != chain.name {
            bail!(
                "Network name mismatch. Expected {}, got {}. Please fix it in `config.toml`",
                chain.name,
                meta.meta_values.name
            )
        }
        Ok(meta)
    }
}

pub(crate) struct AsyncConfigRpcFetcher;

#[async_trait]
impl AsyncFetcher for AsyncConfigRpcFetcher {
    async fn fetch_specs(&self, chain: &Chain) -> Result<NetworkSpecs> {
        let chain_clone = chain.clone();
        let specs = call_urls_async(&chain.rpc_endpoints, move |url| {
            let chain = chain_clone.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let optional_token_override = chain.token_decimals.zip(chain.token_unit.as_ref()).map(
                        |(token_decimals, token_unit)| Token {
                            decimals: token_decimals,
                            unit: token_unit.to_string(),
                        },
                    );
                    specs_agnostic(&url, get_crypto(&chain), optional_token_override, None)
                })
                .await
                .map_err(|e| generate_message::Error::SpecsTransfer(format!("Task join error: {}", e)))?
            }
        })
        .await
        .map_err(|e| anyhow!("{:?}", e))?;
        Ok(specs)
    }

    async fn fetch_metadata(&self, _chain: &Chain) -> Result<MetaFetched> {
        bail!("Not implemented!");
    }
}
