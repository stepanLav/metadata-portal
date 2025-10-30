use std::cmp::Ordering;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use indexmap::IndexMap;
use log::{info, warn};
use tokio::sync::Semaphore;

use crate::common::path::{ContentType, QrPath};
use crate::common::types::MetaVersion;
use crate::export::{
    ExportChainSpec, ExportData, MetadataQr, MetadataStatus, QrCode, ReactAssetPath,
};
use crate::fetch::{AsyncConfigRpcFetcher, AsyncFetcher, Fetcher};
use crate::qrs::{collect_metadata_qrs, metadata_files, spec_files};
use crate::AppConfig;

// Legacy sync version for backward compatibility
pub(crate) fn export_specs(config: &AppConfig, fetcher: impl Fetcher) -> Result<ExportData> {
    let all_specs = spec_files(&config.qr_dir)?;
    let all_metadata = metadata_files(&config.qr_dir)?;

    let mut export_specs = IndexMap::new();
    for chain in &config.chains {
        info!("Collecting {} info...", chain.name);
        let specs = fetcher.fetch_specs(chain)?;
        let meta = fetcher.fetch_metadata(chain)?;
        let live_meta_version = meta.meta_values.version;

        let metadata_qrs = collect_metadata_qrs(&all_metadata, &chain.name, &live_meta_version)?;

        let specs_qr = all_specs
            .get(chain.name.as_str())
            .with_context(|| format!("No specs qr found for {}", chain.name))?
            .clone();
        let latest_meta = update_pointer_to_latest_metadata(
            metadata_qrs
                .first()
                .context(format!("No metadata QRs for {}", &chain.name))?,
        )?;
        export_specs.insert(
            chain.name.clone(),
            ExportChainSpec {
                title: chain.title.as_ref().unwrap_or(&chain.name).clone(),
                color: chain.color.clone(),
                rpc_endpoint: chain.rpc_endpoints[0].clone(), // keep only the first one
                genesis_hash: format!("0x{}", hex::encode(specs.genesis_hash)),
                unit: specs.unit,
                icon: chain.icon.clone(),
                decimals: specs.decimals,
                base58prefix: specs.base58prefix,
                specs_qr: QrCode::from_qr_path(config, specs_qr, &chain.verifier)?,
                latest_metadata: ReactAssetPath::from_fs_path(&latest_meta, &config.public_dir)?,
                metadata_qrs: export_metadata_files(
                    config,
                    metadata_qrs,
                    &live_meta_version,
                    &chain.verifier,
                ),
                live_meta_version,
                testnet: chain.testnet.unwrap_or(false),
            },
        );
    }
    Ok(export_specs)
}

// New async version with parallel processing
pub(crate) async fn export_specs_async(config: &AppConfig) -> Result<ExportData> {
    info!("🚀 Starting parallel chain data collection");
    
    let all_specs = spec_files(&config.qr_dir)?;
    let all_metadata = metadata_files(&config.qr_dir)?;
    
    let fetcher = Arc::new(AsyncConfigRpcFetcher);
    let semaphore = Arc::new(Semaphore::new(10));
    let config_arc = Arc::new(config.clone());
    
    info!("📊 Collecting data for {} chains (max 10 concurrent)", config.chains.len());
    
    let results: Vec<_> = stream::iter(config.chains.iter())
        .map(|chain| {
            let fetcher = Arc::clone(&fetcher);
            let semaphore = Arc::clone(&semaphore);
            let config = Arc::clone(&config_arc);
            let all_specs = all_specs.clone();
            let all_metadata = all_metadata.clone();
            let chain = chain.clone();
            
            async move {
                let _permit = semaphore.acquire().await.unwrap();
                
                info!("Collecting {} info...", chain.name);
                
                let specs = match fetcher.fetch_specs(&chain).await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Failed to fetch specs for {}: {}", chain.name, e);
                        return Err(anyhow::anyhow!("Failed to fetch specs for {}: {}", chain.name, e));
                    }
                };
                
                let meta = match fetcher.fetch_metadata(&chain).await {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("Failed to fetch metadata for {}: {}", chain.name, e);
                        return Err(anyhow::anyhow!("Failed to fetch metadata for {}: {}", chain.name, e));
                    }
                };
                
                let live_meta_version = meta.meta_values.version;
                
                let metadata_qrs = match collect_metadata_qrs(&all_metadata, &chain.name, &live_meta_version) {
                    Ok(qrs) => qrs,
                    Err(e) => {
                        warn!("Failed to collect metadata QRs for {}: {}", chain.name, e);
                        return Err(e);
                    }
                };
                
                let specs_qr = match all_specs.get(chain.name.as_str()) {
                    Some(qr) => qr.clone(),
                    None => {
                        warn!("No specs qr found for {}", chain.name);
                        return Err(anyhow::anyhow!("No specs qr found for {}", chain.name));
                    }
                };
                
                let latest_meta = match metadata_qrs.first() {
                    Some(qr) => match update_pointer_to_latest_metadata(qr) {
                        Ok(path) => path,
                        Err(e) => {
                            warn!("Failed to update pointer for {}: {}", chain.name, e);
                            return Err(e);
                        }
                    },
                    None => {
                        warn!("No metadata QRs for {}", chain.name);
                        return Err(anyhow::anyhow!("No metadata QRs for {}", chain.name));
                    }
                };
                
                let export_chain = ExportChainSpec {
                    title: chain.title.as_ref().unwrap_or(&chain.name).clone(),
                    color: chain.color.clone(),
                    rpc_endpoint: chain.rpc_endpoints[0].clone(),
                    genesis_hash: format!("0x{}", hex::encode(specs.genesis_hash)),
                    unit: specs.unit,
                    icon: chain.icon.clone(),
                    decimals: specs.decimals,
                    base58prefix: specs.base58prefix,
                    specs_qr: QrCode::from_qr_path(&config, specs_qr, &chain.verifier)?,
                    latest_metadata: ReactAssetPath::from_fs_path(&latest_meta, &config.public_dir)?,
                    metadata_qrs: export_metadata_files(
                        &config,
                        metadata_qrs,
                        &live_meta_version,
                        &chain.verifier,
                    ),
                    live_meta_version,
                    testnet: chain.testnet.unwrap_or(false),
                };
                
                info!("✅ Successfully collected data for {}", chain.name);
                Ok::<_, anyhow::Error>((chain.name.clone(), export_chain))
            }
        })
        .buffer_unordered(10)
        .collect()
        .await;
    
    let mut export_specs = IndexMap::new();
    let mut error_count = 0;
    
    for result in results {
        match result {
            Ok((name, chain_spec)) => {
                export_specs.insert(name, chain_spec);
            }
            Err(e) => {
                warn!("Error collecting chain data: {}", e);
                error_count += 1;
            }
        }
    }
    
    info!("📈 Summary: {}/{} chains collected successfully", 
          export_specs.len(), config.chains.len());
    
    if error_count > 0 {
        warn!("⚠️ {} chains failed to collect", error_count);
    }
    
    Ok(export_specs)
}

fn export_metadata_files(
    config: &AppConfig,
    qrs: Vec<QrPath>,
    live_version: &MetaVersion,
    verifier_name: &String,
) -> Vec<MetadataQr> {
    qrs.into_iter()
        .map(|qr| {
            if let ContentType::Metadata(version) = qr.file_name.content_type {
                let status = match version.cmp(live_version) {
                    Ordering::Less => MetadataStatus::Outdated,
                    Ordering::Equal => MetadataStatus::Now,
                    Ordering::Greater => MetadataStatus::Future,
                };
                MetadataQr {
                    version,
                    file: QrCode::from_qr_path(config, qr, verifier_name).unwrap(),
                    status,
                }
            } else {
                panic!("Not a metadata qr: {:?}", qr);
            }
        })
        .collect()
}

// Create symlink to latest metadata qr
fn update_pointer_to_latest_metadata(metadata_qr: &QrPath) -> Result<PathBuf> {
    let latest_metadata_qr = metadata_qr.dir.join(format!(
        "{}_metadata_latest.apng",
        metadata_qr.file_name.chain
    ));
    if latest_metadata_qr.is_symlink() {
        fs::remove_file(&latest_metadata_qr).unwrap();
    }
    symlink(metadata_qr.to_path_buf(), &latest_metadata_qr).unwrap();
    Ok(latest_metadata_qr)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::{env, fs};

    use definitions::crypto::Encryption;
    use definitions::metadata::MetaValues;
    use definitions::network_specs::NetworkSpecs;
    use generate_message::helpers::MetaFetched;
    use sp_core::H256;

    use super::*;
    use crate::config::Chain;

    struct MockFetcher;
    impl Fetcher for MockFetcher {
        fn fetch_specs(&self, _chain: &Chain) -> Result<NetworkSpecs> {
            Ok(NetworkSpecs {
                base58prefix: 0,
                color: "".to_string(),
                decimals: 10,
                encryption: Encryption::Ed25519,
                genesis_hash: H256::from_str(
                    "a8dfb73a4b44e6bf84affe258954c12db1fe8e8cf00b965df2af2f49c1ec11cd",
                )
                .expect("checked value"),
                logo: "logo".to_string(),
                name: "polkadot".to_string(),
                path_id: "".to_string(),
                secondary_color: "".to_string(),
                title: "".to_string(),
                unit: "DOT".to_string(),
            })
        }

        fn fetch_metadata(&self, _chain: &Chain) -> Result<MetaFetched> {
            Ok(MetaFetched {
                meta_values: MetaValues {
                    name: "".to_string(),
                    version: 9,
                    optional_base58prefix: None,
                    warn_incomplete_extensions: false,
                    meta: vec![],
                },
                block_hash: H256::zero(),
                genesis_hash: H256::zero(),
            })
        }
    }

    #[test]
    fn test_collector() {
        let root_dir = env::current_dir().unwrap();
        let config = AppConfig {
            qr_dir: root_dir.join("src/collector/for_tests"),
            public_dir: root_dir.join("src/collector"),
            ..Default::default()
        };

        let specs = export_specs(&config, MockFetcher).unwrap();
        let result = serde_json::to_string_pretty(&specs).unwrap();
        let expected = fs::read_to_string(config.qr_dir.join("expected.json"))
            .expect("unable to read expected file");
        assert_eq!(result, expected);

        let latest_symlink = config.qr_dir.join("polkadot_metadata_latest.apng");
        assert!(latest_symlink.exists());
        fs::remove_file(latest_symlink).unwrap();
    }
}
