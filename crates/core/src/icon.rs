//! Icon acquisition: download the best candidate from a site, or generate a
//! lettered fallback (a coloured circle with the app's initial) when none is
//! usable — mirroring Quick Web Apps' fallback, but auto-sourced.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::paths;

const MIN_SIZE: u32 = 48;

/// Favicon-service URLs for a site, as a reliable fallback when the page has
/// no usable manifest/apple-touch icon. Google's service returns a clean PNG
/// at the requested size for almost any domain.
pub fn favicon_service_urls(site_url: &str) -> Vec<String> {
    let Some(host) = url::Url::parse(site_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
    else {
        return Vec::new();
    };
    vec![
        format!("https://www.google.com/s2/favicons?domain={host}&sz=256"),
        format!("https://www.google.com/s2/favicons?domain={host}&sz=128"),
        format!("https://icons.duckduckgo.com/ip3/{host}.ico"),
    ]
}

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
    Err(anyhow!(
        "no usable icon among {} candidates",
        candidates.len()
    ))
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

/// Copy a user-selected icon into the app-managed icons directory, returning
/// the managed path. The app then owns the file it references, so deleting the
/// web app only removes this copy — never the user's original (#37).
///
/// If `src` already lives under the managed icons directory it is returned
/// unchanged (no copy). The original `src` is never modified or removed.
pub fn import(id: &str, src: &Path) -> Result<PathBuf> {
    let dir = paths::icons_dir();
    if src.starts_with(&dir) {
        return Ok(src.to_path_buf());
    }
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "png".to_string());
    let dest = dir.join(format!("{id}.{ext}"));
    std::fs::copy(src, &dest)
        .with_context(|| format!("copying icon {} -> {}", src.display(), dest.display()))?;
    Ok(dest)
}

// --- Online icon search (Iconify) ---------------------------------------

/// Search the free Iconify API for icons matching `query`. Returns icon ids
/// like `mdi:email`, best matches first.
pub async fn search_iconify(query: &str) -> Vec<String> {
    let q = query.trim().replace(' ', "+");
    if q.is_empty() {
        return Vec::new();
    }
    let url = format!("https://api.iconify.design/search?query={q}&limit=60");
    let client = reqwest::Client::new();
    let json: serde_json::Value = match client.get(&url).send().await {
        Ok(r) => match r.json().await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("iconify parse failed: {e}");
                return Vec::new();
            }
        },
        Err(e) => {
            tracing::warn!("iconify search failed: {e}");
            return Vec::new();
        }
    };
    json.get("icons")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch an Iconify icon by id (`prefix:name`) and rasterize it to a PNG of
/// `size`x`size`.
pub async fn iconify_png(icon_id: &str, size: u32) -> Option<Vec<u8>> {
    let path = icon_id.replacen(':', "/", 1); // mdi:email -> mdi/email
    let url = format!("https://api.iconify.design/{path}.svg?height={size}");
    let client = reqwest::Client::new();
    let svg = client.get(&url).send().await.ok()?.bytes().await.ok()?;
    rasterize_svg(&svg, size)
}

/// Render SVG bytes to a square PNG. Works for path-based icons (Iconify, our
/// lettered fallback) — no system SVG loader (librsvg) required.
pub fn rasterize_svg(svg: &[u8], size: u32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let ts = tree.size();
    let scale = (size as f32 / ts.width()).min(size as f32 / ts.height());
    let tx = (size as f32 - ts.width() * scale) / 2.0;
    let ty = (size as f32 - ts.height() * scale) / 2.0;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

/// Persist raw PNG bytes under `icons/<safe-name>.png`. Returns the path.
pub fn save_png(name: &str, png: &[u8]) -> Option<PathBuf> {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let out = paths::icons_dir().join(format!("{safe}.png"));
    std::fs::write(&out, png).ok()?;
    Some(out)
}

/// Deterministic pleasant colour derived from the app name.
fn pick_color(seed: &str) -> &'static str {
    const PALETTE: [&str; 8] = [
        "#3584e4", "#33d17a", "#f6d32d", "#ff7800", "#e01b24", "#9141ac", "#986a44", "#62a0ea",
    ];
    let sum: usize = seed.bytes().map(|b| b as usize).sum();
    PALETTE[sum % PALETTE.len()]
}
