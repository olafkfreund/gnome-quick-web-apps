//! A lightweight, self-contained network blocklist (no external engine).
//!
//! When a web app enables adblock, the runner consults [`is_blocked`] for every
//! resource request and cancels matches. The list is a curated set of ad,
//! tracker and analytics domains bundled at compile time. Matching is by
//! domain suffix, so every subdomain of a listed domain is covered.

use std::collections::HashSet;
use std::sync::OnceLock;

const HOSTS: &str = include_str!("adblock_hosts.txt");

fn blocklist() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        HOSTS
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_lowercase())
            .collect()
    })
}

/// Whether a request URL points at a blocked ad/tracker domain. A host matches
/// if it equals a listed domain or is a subdomain of one (e.g.
/// `pagead2.googlesyndication.com` matches `googlesyndication.com`).
pub fn is_blocked(url: &str) -> bool {
    let Some(host) = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
    else {
        return false;
    };
    let set = blocklist();
    // Walk the host's parent domains: a.b.c.com -> a.b.c.com, b.c.com, c.com.
    let mut rest = host.as_str();
    loop {
        if set.contains(rest) {
            return true;
        }
        match rest.split_once('.') {
            Some((_, parent)) if parent.contains('.') => rest = parent,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_blocked;

    #[test]
    fn blocks_known_trackers_and_subdomains() {
        assert!(is_blocked("https://www.google-analytics.com/collect"));
        assert!(is_blocked("https://pagead2.googlesyndication.com/x.js"));
        assert!(is_blocked("https://connect.facebook.net/en_US/fbevents.js"));
    }

    #[test]
    fn allows_normal_sites() {
        assert!(!is_blocked("https://www.notion.so/app"));
        assert!(!is_blocked("https://figma.com/file/abc"));
        assert!(!is_blocked("https://en.wikipedia.org/wiki/Ad"));
        // a non-blocked sibling of a blocked registrable domain
        assert!(!is_blocked("https://maps.google.com/"));
    }
}
