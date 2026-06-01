//! PWA manifest detection — the headline differentiator over Quick Web Apps.
//!
//! Given a URL the user pastes, fetch the page, locate its
//! `<link rel="manifest">`, parse the Web App Manifest, and surface name,
//! icons, theme colour and scope so the editor form fills itself.
//! Falls back to `apple-touch-icon` / `<title>` / favicon when no manifest.

use anyhow::{Context, Result};
use scraper::{Html, Selector};
use url::Url;

/// Everything we can learn about a site for pre-filling the editor.
#[derive(Debug, Default, Clone)]
pub struct SiteInfo {
    pub name: Option<String>,
    pub start_url: Option<String>,
    pub scope: Option<String>,
    pub theme_color: Option<String>,
    /// Candidate icon URLs, best (largest / most specific) first.
    pub icon_urls: Vec<String>,
}

#[derive(serde::Deserialize, Default)]
struct WebManifest {
    name: Option<String>,
    short_name: Option<String>,
    start_url: Option<String>,
    scope: Option<String>,
    theme_color: Option<String>,
    #[serde(default)]
    icons: Vec<ManifestIcon>,
}

#[derive(serde::Deserialize)]
struct ManifestIcon {
    src: String,
    #[serde(default)]
    sizes: Option<String>,
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) GnomeQuickWebApps/0.1")
        .build()?)
}

/// Fetch and analyse `page_url`. Network + parse errors degrade gracefully:
/// callers always get at least an empty `SiteInfo`.
pub async fn detect(page_url: &str) -> Result<SiteInfo> {
    let base = Url::parse(page_url).context("invalid URL")?;
    let client = client()?;

    let html = client.get(base.clone()).send().await?.text().await?;
    let mut info = parse_html(&html, &base);

    // If the page advertised a manifest, pull richer data from it.
    if let Some(manifest_url) = find_manifest_url(&html, &base) {
        if let Ok(text) = client.get(manifest_url.clone()).send().await?.text().await {
            if let Ok(m) = serde_json::from_str::<WebManifest>(&text) {
                merge_manifest(&mut info, m, &manifest_url);
            }
        }
    }

    if info.name.is_none() {
        info.name = base
            .host_str()
            .map(|h| h.trim_start_matches("www.").to_string());
    }
    Ok(info)
}

fn find_manifest_url(html: &str, base: &Url) -> Option<Url> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse(r#"link[rel="manifest"]"#).ok()?;
    let href = doc.select(&sel).next()?.value().attr("href")?;
    base.join(href).ok()
}

fn parse_html(html: &str, base: &Url) -> SiteInfo {
    let doc = Html::parse_document(html);
    let mut info = SiteInfo::default();

    if let Ok(sel) = Selector::parse("title") {
        if let Some(t) = doc.select(&sel).next() {
            let title = t.text().collect::<String>().trim().to_string();
            if !title.is_empty() {
                info.name = Some(title);
            }
        }
    }

    // Icons, in rough priority order.
    for rel in [
        r#"link[rel="apple-touch-icon"]"#,
        r#"link[rel="icon"]"#,
        r#"link[rel="shortcut icon"]"#,
    ] {
        if let Ok(sel) = Selector::parse(rel) {
            for el in doc.select(&sel) {
                if let Some(href) = el.value().attr("href") {
                    if let Ok(abs) = base.join(href) {
                        info.icon_urls.push(abs.to_string());
                    }
                }
            }
        }
    }
    // Last-resort root favicon.
    if let Ok(fav) = base.join("/favicon.ico") {
        info.icon_urls.push(fav.to_string());
    }

    info
}

fn merge_manifest(info: &mut SiteInfo, m: WebManifest, manifest_url: &Url) {
    if let Some(n) = m.name.or(m.short_name) {
        info.name = Some(n);
    }
    if let Some(tc) = m.theme_color {
        info.theme_color = Some(tc);
    }
    if let Some(su) = m.start_url.as_ref().and_then(|s| manifest_url.join(s).ok()) {
        info.start_url = Some(su.to_string());
    }
    if let Some(sc) = m.scope.as_ref().and_then(|s| manifest_url.join(s).ok()) {
        info.scope = Some(sc.to_string());
    }

    // Manifest icons take priority; sort largest-first by the `sizes` field.
    let mut icons: Vec<(u32, String)> = m
        .icons
        .into_iter()
        .filter_map(|i| {
            let abs = manifest_url.join(&i.src).ok()?.to_string();
            let area = i.sizes.as_deref().and_then(parse_size).unwrap_or(0);
            Some((area, abs))
        })
        .collect();
    icons.sort_by(|a, b| b.0.cmp(&a.0));
    let mut ordered: Vec<String> = icons.into_iter().map(|(_, u)| u).collect();
    ordered.append(&mut info.icon_urls);
    info.icon_urls = ordered;
}

/// Parse `"512x512"` (or `"192x192 512x512"`) into the largest pixel area.
fn parse_size(sizes: &str) -> Option<u32> {
    sizes
        .split_whitespace()
        .filter_map(|tok| {
            let (w, h) = tok.split_once(['x', 'X'])?;
            Some(w.parse::<u32>().ok()? * h.parse::<u32>().ok()?)
        })
        .max()
}
