//! Known DRM streaming services that will NOT play in Quick Web Apps.
//!
//! These services gate playback behind the Widevine CDM with a Verified Media
//! Path (VMP) signature that Google grants only to certified browser vendors —
//! something an embeddable engine (CEF) cannot obtain. The site loads, but
//! audio/video never play. We use this list to warn the user up front when they
//! add such a site (see the editor) and to keep these out of the templates.

/// `(host_needle, display_name)` for services known to require DRM. The needle
/// matches the URL host exactly or as a parent domain (`foo.<needle>`).
const KNOWN_DRM: &[(&str, &str)] = &[
    ("netflix.com", "Netflix"),
    ("spotify.com", "Spotify"),
    ("music.apple.com", "Apple Music"),
    ("tv.apple.com", "Apple TV+"),
    ("music.amazon.com", "Amazon Music"),
    ("primevideo.com", "Prime Video"),
    ("disneyplus.com", "Disney+"),
    ("hulu.com", "Hulu"),
    ("max.com", "Max"),
    ("hbomax.com", "Max"),
    ("peacocktv.com", "Peacock"),
    ("paramountplus.com", "Paramount+"),
    ("crunchyroll.com", "Crunchyroll"),
    ("tidal.com", "Tidal"),
    ("deezer.com", "Deezer"),
    ("qobuz.com", "Qobuz"),
];

/// If `url`'s host belongs to a known DRM service, return its display name.
/// Matching is on the registrable host: exact host or any subdomain of the
/// needle (so `open.spotify.com` matches `spotify.com`).
pub fn drm_service(url: &str) -> Option<&'static str> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
    KNOWN_DRM.iter().find_map(|(needle, name)| {
        (host == *needle || host.ends_with(&format!(".{needle}"))).then_some(*name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_drm_hosts() {
        assert_eq!(
            drm_service("https://www.netflix.com/browse"),
            Some("Netflix")
        );
        assert_eq!(drm_service("https://open.spotify.com"), Some("Spotify"));
        assert_eq!(
            drm_service("https://music.apple.com/us"),
            Some("Apple Music")
        );
        assert_eq!(drm_service("https://listen.tidal.com"), Some("Tidal"));
        assert_eq!(drm_service("https://play.max.com/foo"), Some("Max"));
    }

    #[test]
    fn ignores_non_drm_and_lookalike_hosts() {
        assert_eq!(drm_service("https://youtube.com"), None);
        assert_eq!(drm_service("https://github.com"), None);
        // iCloud / Apple non-media must NOT trip the music.apple.com needle.
        assert_eq!(drm_service("https://icloud.com"), None);
        assert_eq!(drm_service("https://apple.com"), None);
        // A lookalike that merely contains the string but isn't a subdomain.
        assert_eq!(drm_service("https://notnetflix.com"), None);
        assert_eq!(drm_service("not a url"), None);
    }
}
