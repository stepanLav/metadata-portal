// use crate::lib::read::ChainName;
use crate::lib::types::ChainName;
use crate::updater::wasm::WasmRuntime;
use std::collections::HashMap;

// fetch latest runtimes from Parity GitHub
pub async fn fetch_release_runtimes() -> anyhow::Result<HashMap<ChainName, WasmRuntime>> {
    let mut runtimes: HashMap<ChainName, WasmRuntime> = HashMap::new();
    let release = octocrab::instance()
        .repos("paritytech", "polkadot")
        .releases()
        .get_latest()
        .await?;
    for asset in release.assets {
        if let Ok(wasm) = WasmRuntime::try_from(asset) {
            runtimes.insert(wasm.chain.clone(), wasm);
        }
    }
    Ok(runtimes)
}
