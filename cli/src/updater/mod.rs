mod generate;
mod github;
pub(crate) mod source;
mod wasm;

use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;

use blake2_rfc::blake2b::blake2b;
use futures::stream::{self, StreamExt};
use log::{info, warn};
use sp_core::H256;
use tokio::sync::Semaphore;

use crate::common::types::get_crypto;
use crate::config::AppConfig;
use crate::fetch::{AsyncFetcher, AsyncRpcFetcher, Fetcher};
use crate::qrs::{metadata_files, spec_files};
use crate::source::{save_source_info, Source};
use crate::updater::generate::{download_metadata_qr, generate_metadata_qr, generate_spec_qr};
use crate::updater::github::fetch_latest_runtime;
use crate::updater::wasm::{download_wasm, meta_values_from_wasm_bytes};

// Legacy sync version for backward compatibility
pub(crate) fn update_from_node(
    config: AppConfig,
    sign: bool,
    signing_key: String,
    fetcher: impl Fetcher,
) -> anyhow::Result<()> {
    let metadata_qrs = metadata_files(&config.qr_dir)?;
    let specs_qrs = spec_files(&config.qr_dir)?;
    let mut is_changed = false;
    let mut error_fetching_data = false;
    for chain in config.chains {
        let encryption = get_crypto(&chain);
        if !specs_qrs.contains_key(chain.name.as_str()) {
            let specs_res = fetcher.fetch_specs(&chain);
            if specs_res.is_err() {
                error_fetching_data = true;
                warn!(
                    "Can't get specs for {}. Error is {}",
                    chain.name,
                    specs_res.err().unwrap()
                );
                continue;
            }
            if chain.verifier == "parity" {
                warn!("The chain {} should be added and signed by Parity, please check it on the Parity Metadata portal https://metadata.parity.io/", chain.name);
            } else {
                generate_spec_qr(
                    &specs_res.unwrap(),
                    &config.qr_dir,
                    sign,
                    signing_key.to_owned(),
                    &encryption,
                )?;
            }
            is_changed = true;
        }

        let fetched_meta_res = fetcher.fetch_metadata(&chain);
        if fetched_meta_res.is_err() {
            error_fetching_data = true;
            warn!(
                "Can't get metadata for {}. Error is {}",
                chain.name,
                fetched_meta_res.err().unwrap()
            );
            continue;
        }
        let fetched_meta = fetched_meta_res.unwrap();
        let version = fetched_meta.meta_values.version;

        // Skip if already have QR for the same version
        if let Some(map) = metadata_qrs.get(&chain.name) {
            if map.contains_key(&version) {
                continue;
            }
        }
        if chain.verifier == "parity" {
            download_metadata_qr(
                "https://metadata.parity.io/qr",
                &fetched_meta.meta_values,
                &config.qr_dir,
            )?;
        } else {
            let path = generate_metadata_qr(
                &fetched_meta.meta_values,
                &fetched_meta.genesis_hash,
                &config.qr_dir,
                sign,
                signing_key.to_owned(),
                &encryption,
            )?;
            let source = Source::Rpc {
                block: fetched_meta.block_hash,
            };
            save_source_info(&path, &source)?;
        }
        is_changed = true;
    }

    if error_fetching_data {
        warn!("⚠️ Some chain data wasn't read. Please check the log!");
        exit(12);
    }

    if !is_changed {
        info!("🎉 Everything is up to date!");
    }

    Ok(())
}

// New async version with parallel processing
#[tokio::main]
pub(crate) async fn update_from_node_async(
    config: AppConfig,
    sign: bool,
    signing_key: String,
) -> anyhow::Result<()> {
    info!("🚀 Starting parallel chain updates with optimized async processing");

    let metadata_qrs = metadata_files(&config.qr_dir)?;
    let specs_qrs = spec_files(&config.qr_dir)?;

    // Limit concurrent connections to avoid overwhelming network/CPU
    // With 10 concurrent chains, 80 chains should complete in ~8 batches
    let semaphore = Arc::new(Semaphore::new(10));
    let fetcher = Arc::new(AsyncRpcFetcher);
    let qr_dir = Arc::new(config.qr_dir.clone());
    let signing_key = Arc::new(signing_key);

    let chains: Vec<_> = config.chains.into_iter().collect();
    let total_chains = chains.len();

    info!(
        "📊 Processing {} chains with max 10 concurrent connections",
        total_chains
    );

    // Process chains concurrently
    let results: Vec<_> = stream::iter(chains)
        .map(|chain| {
            let semaphore = Arc::clone(&semaphore);
            let fetcher = Arc::clone(&fetcher);
            let qr_dir = Arc::clone(&qr_dir);
            let signing_key = Arc::clone(&signing_key);
            let metadata_qrs = metadata_qrs.clone();
            let specs_qrs = specs_qrs.clone();

            async move {
                // Acquire semaphore permit (limits concurrency)
                let _permit = semaphore.acquire().await.unwrap();

                info!("🔍 Processing chain: {}", chain.name);

                let encryption = get_crypto(&chain);
                let mut chain_changed = false;

                // Process specs if missing
                if !specs_qrs.contains_key(chain.name.as_str()) {
                    match fetcher.fetch_specs(&chain).await {
                        Ok(specs) => {
                            if chain.verifier == "parity" {
                                warn!("The chain {} should be added and signed by Parity, please check it on the Parity Metadata portal https://metadata.parity.io/", chain.name);
                            } else {
                                if let Err(e) = generate_spec_qr(
                                    &specs,
                                    &qr_dir,
                                    sign,
                                    signing_key.to_string(),
                                    &encryption,
                                ) {
                                    warn!("Failed to generate spec QR for {}: {}", chain.name, e);
                                    return (chain.name.clone(), false, true);
                                }
                            }
                            chain_changed = true;
                        }
                        Err(e) => {
                            warn!("Can't get specs for {}. Error is {}", chain.name, e);
                            return (chain.name.clone(), false, true);
                        }
                    }
                }

                // Process metadata
                match fetcher.fetch_metadata(&chain).await {
                    Ok(fetched_meta) => {
                        let version = fetched_meta.meta_values.version;

                        // Skip if already have QR for the same version
                        if let Some(map) = metadata_qrs.get(&chain.name) {
                            if map.contains_key(&version) {
                                info!("✓ Chain {} already up to date (v{})", chain.name, version);
                                return (chain.name.clone(), chain_changed, false);
                            }
                        }

                        if chain.verifier == "parity" {
                            if let Err(e) = download_metadata_qr(
                                "https://metadata.parity.io/qr",
                                &fetched_meta.meta_values,
                                &qr_dir,
                            ) {
                                warn!("Failed to download metadata QR for {}: {}", chain.name, e);
                                return (chain.name.clone(), false, true);
                            }
                        } else {
                            match generate_metadata_qr(
                                &fetched_meta.meta_values,
                                &fetched_meta.genesis_hash,
                                &qr_dir,
                                sign,
                                signing_key.to_string(),
                                &encryption,
                            ) {
                                Ok(path) => {
                                    let source = Source::Rpc {
                                        block: fetched_meta.block_hash,
                                    };
                                    if let Err(e) = save_source_info(&path, &source) {
                                        warn!("Failed to save source info for {}: {}", chain.name, e);
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to generate metadata QR for {}: {}", chain.name, e);
                                    return (chain.name.clone(), false, true);
                                }
                            }
                        }
                        info!("✅ Successfully updated chain: {} (v{})", chain.name, version);
                        (chain.name.clone(), true, false)
                    }
                    Err(e) => {
                        warn!("Can't get metadata for {}. Error is {}", chain.name, e);
                        (chain.name.clone(), false, true)
                    }
                }
            }
        })
        .buffer_unordered(10) // Process up to 10 chains concurrently
        .collect()
        .await;

    // Aggregate results
    let mut is_changed = false;
    let mut error_count = 0;
    let mut success_count = 0;

    for (chain_name, changed, has_error) in results {
        if changed {
            is_changed = true;
            success_count += 1;
        }
        if has_error {
            error_count += 1;
        } else if !changed {
            success_count += 1;
        }
    }

    info!(
        "📈 Summary: {}/{} chains processed successfully, {} errors",
        success_count, total_chains, error_count
    );

    if error_count > 0 {
        warn!("⚠️ Some chain data wasn't read. Please check the log!");
        exit(12);
    }

    if !is_changed {
        info!("🎉 Everything is up to date!");
    } else {
        info!("✨ Updates completed successfully!");
    }

    Ok(())
}

#[tokio::main]
pub(crate) async fn update_from_github(
    config: AppConfig,
    sign: bool,
    signing_key: String,
) -> anyhow::Result<()> {
    info!("🚀 Starting parallel GitHub release updates");

    let metadata_qrs = metadata_files(&config.qr_dir)?;
    let qr_dir = Arc::new(config.qr_dir.clone());
    let signing_key = Arc::new(signing_key);

    // Limit concurrent GitHub API calls to avoid rate limiting
    let semaphore = Arc::new(Semaphore::new(5));

    let chains: Vec<_> = config.chains.into_iter().collect();
    let total_chains = chains.len();

    info!(
        "📊 Checking {} chains for GitHub releases (max 5 concurrent)",
        total_chains
    );

    let results: Vec<_> = stream::iter(chains)
        .map(|chain| {
            let semaphore = Arc::clone(&semaphore);
            let qr_dir = Arc::clone(&qr_dir);
            let signing_key = Arc::clone(&signing_key);
            let metadata_qrs = metadata_qrs.clone();

            async move {
                let _permit = semaphore.acquire().await.unwrap();

                info!("🔍 Checking for updates for {}", chain.name);

                if chain.github_release.is_none() {
                    info!(
                        "↪️ No GitHub releases configured for {}, skipping",
                        chain.name
                    );
                    return Ok::<_, anyhow::Error>(false);
                }

                let github_repo = chain.github_release.as_ref().unwrap();
                let wasm = match fetch_latest_runtime(github_repo, &chain.name).await {
                    Ok(Some(w)) => w,
                    Ok(None) => {
                        warn!("🤨 No releases found for {}", chain.name);
                        return Ok(false);
                    }
                    Err(e) => {
                        warn!("Failed to fetch latest runtime for {}: {}", chain.name, e);
                        return Ok(false);
                    }
                };

                info!("📅 Found version {} for {}", wasm.version, chain.name);
                let genesis_hash = H256::from_str(&github_repo.genesis_hash).unwrap();

                // Skip if already have QR for the same version
                if let Some(map) = metadata_qrs.get(&chain.name) {
                    if map.contains_key(&wasm.version)
                        || map.keys().min().unwrap_or(&0) > &wasm.version
                    {
                        info!("✓ {} is up to date!", chain.name);
                        return Ok(false);
                    }
                }

                let wasm_bytes = match download_wasm(wasm.to_owned()).await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        warn!("Failed to download wasm for {}: {}", chain.name, e);
                        return Ok(false);
                    }
                };

                let meta_hash = blake2b(32, &[], &wasm_bytes).as_bytes().to_vec();
                let meta_values = match meta_values_from_wasm_bytes(&wasm_bytes) {
                    Ok(mv) => mv,
                    Err(e) => {
                        warn!(
                            "Failed to extract metadata from wasm for {}: {}",
                            chain.name, e
                        );
                        return Ok(false);
                    }
                };

                let encryption = get_crypto(&chain);
                let path = match generate_metadata_qr(
                    &meta_values,
                    &genesis_hash,
                    &qr_dir,
                    sign,
                    signing_key.to_string(),
                    &encryption,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("Failed to generate metadata QR for {}: {}", chain.name, e);
                        return Ok(false);
                    }
                };

                let source = Source::Wasm {
                    github_repo: format!("{}/{}", github_repo.owner, github_repo.repo),
                    hash: format!("0x{}", hex::encode(meta_hash)),
                };

                if let Err(e) = save_source_info(&path, &source) {
                    warn!("Failed to save source info for {}: {}", chain.name, e);
                }

                info!("✅ Successfully updated {} from GitHub release", chain.name);
                Ok(true)
            }
        })
        .buffer_unordered(5) // Process up to 5 chains concurrently
        .collect()
        .await;

    let mut update_count = 0;
    for result in results {
        if let Ok(true) = result {
            update_count += 1;
        }
    }

    if update_count > 0 {
        info!("✨ Updated {} chains from GitHub releases", update_count);
    } else {
        info!("🎉 All chains are up to date!");
    }

    Ok(())
}
