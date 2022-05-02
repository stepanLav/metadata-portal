use anyhow::anyhow;
use octocrab::models::repos::Asset;
use reqwest::Url;

#[derive(Debug, PartialEq, Clone, Hash, Eq)]
pub struct WasmRuntime {
    pub chain: String,
    pub version: u32,
    pub download_url: Url,
}

impl TryFrom<Asset> for WasmRuntime {
    type Error = anyhow::Error;

    fn try_from(asset: Asset) -> Result<Self, Self::Error> {
        if !asset.name.ends_with(".wasm") {
            return Err(anyhow!("{} has no .wasm extension", asset.name));
        }
        let runtime_info = asset
            .name
            .split('.')
            .next()
            .ok_or_else(|| anyhow!("no runtime info found"))?;
        let mut split = runtime_info.split("_runtime-v");
        let chain = split.next().ok_or_else(|| anyhow!("no chain name found"))?;
        let version: u32 = split
            .next()
            .ok_or_else(|| anyhow!("no metadata version found"))?
            .parse()
            .unwrap();

        Ok(Self {
            chain: String::from(chain),
            version,
            download_url: asset.browser_download_url,
        })
    }
}

// fn meta_specs_from_wasm() -> anyhow::Result<()> {
//     info!("downloading {}", wasm.chain);
//     let resp = reqwest::get(asset.browser_download_url).await?;
//     ensure!(resp.status().is_success());
//     let body = resp.bytes().await?;
//
//     let filename = format!("/tmp/{}", wasm.chain);
//     std::fs::write(&Path::new(&filename), &body)?;
//     let meta = MetaValues::from_wasm_file(&filename)
//         .map_err(|e| anyhow!("error converting wasm to metadata"))?;
// }
