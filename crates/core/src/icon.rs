//! Icon acquisition: download the best candidate from a site, or generate a
//! lettered fallback (a coloured circle with the app's initial) when none is
//! usable — mirroring Quick Web Apps' fallback, but auto-sourced.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::paths;

const MIN_SIZE: u32 = 48;

/// Try each candidate URL in order; save the first decodable raster image
/// (>= MIN_SIZE) as a PNG under `icons/<id>.png`. Returns the saved path.
pub async fn download_best(id: &str, candidates: &[String]) -> Result<PathBuf> {
    let client = reqwest::Client::new();
    for url in candidates {
        let Ok(resp) = client.get(url).send().await else {
            continue;
        };
        let Ok(bytes) = resp.bytes().await else {
            continue;
        };
        if let Ok(img) = image::load_from_memory(&bytes) {
            if img.width() >= MIN_SIZE && img.height() >= MIN_SIZE {
                let out = paths::icons_dir().join(format!("{id}.png"));
                img.save(&out)?;
                return Ok(out);
            }
        }
    }
    Err(anyhow!("no usable icon among {} candidates", candidates.len()))
}

/// Write a lettered fallback SVG (coloured circle + initial) for `name`.
pub fn generate_lettered(id: &str, name: &str) -> Result<PathBuf> {
    let letter = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "W".to_string());
    let color = pick_color(name);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
  <circle cx="256" cy="256" r="240" fill="{color}"/>
  <text x="256" y="256" font-family="Cantarell, 'Noto Sans', sans-serif"
        font-size="300" fill="#ffffff" text-anchor="middle"
        dominant-baseline="central">{letter}</text>
</svg>"##
    );
    let out = paths::icons_dir().join(format!("{id}.svg"));
    std::fs::write(&out, svg)?;
    Ok(out)
}

/// Read an icon file into raw bytes (for handing to the launcher portal).
pub fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    Ok(std::fs::read(path)?)
}

/// Deterministic pleasant colour derived from the app name.
fn pick_color(seed: &str) -> &'static str {
    const PALETTE: [&str; 8] = [
        "#3584e4", "#33d17a", "#f6d32d", "#ff7800", "#e01b24", "#9141ac", "#986a44", "#62a0ea",
    ];
    let sum: usize = seed.bytes().map(|b| b as usize).sum();
    PALETTE[sum % PALETTE.len()]
}
