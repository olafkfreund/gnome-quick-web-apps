//! `mailto:` handling — let a web mail app act as the system's default email
//! client. When the launcher is invoked with a `mailto:` URL, we translate it
//! into the provider's web compose URL.

/// A compose-URL template for a known web mail provider, derived from the
/// app's URL host. Placeholders `{to}` `{subject}` `{body}` `{cc}` `{bcc}` are
/// filled (URL-encoded) by [`expand`]. `None` if the host isn't a known
/// provider — the app can still be registered, but a `mailto:` just opens it.
pub fn compose_template_for(app_url: &str) -> Option<String> {
    let host = url::Url::parse(app_url).ok()?.host_str()?.to_lowercase();
    let host = host.trim_start_matches("www.");
    let tmpl = if host.contains("mail.google") || host == "gmail.com" {
        "https://mail.google.com/mail/?view=cm&fs=1&to={to}&su={subject}&body={body}&cc={cc}&bcc={bcc}"
    } else if host.contains("outlook.") {
        "https://outlook.office.com/mail/deeplink/compose?to={to}&subject={subject}&body={body}&cc={cc}&bcc={bcc}"
    } else if host.contains("mail.proton.me") {
        "https://mail.proton.me/u/0/inbox?action=compose&to={to}&subject={subject}&body={body}"
    } else if host.contains("mail.yahoo") {
        "https://compose.mail.yahoo.com/?to={to}&subject={subject}&body={body}&cc={cc}&bcc={bcc}"
    } else if host.contains("mail.zoho") {
        "https://mail.zoho.com/zm/#mail/compose?to={to}&subject={subject}&body={body}"
    } else {
        return None;
    };
    Some(tmpl.to_string())
}

/// Expand a compose `template` from a `mailto:` URI.
pub fn expand(template: &str, mailto: &str) -> String {
    let rest = mailto.strip_prefix("mailto:").unwrap_or(mailto);
    let (to_raw, query) = rest.split_once('?').unwrap_or((rest, ""));

    let mut subject = String::new();
    let mut body = String::new();
    let mut cc = String::new();
    let mut bcc = String::new();
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let val = urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_default();
            match k {
                "subject" => subject = val,
                "body" => body = val,
                "cc" => cc = val,
                "bcc" => bcc = val,
                _ => {}
            }
        }
    }

    let to = urlencoding::decode(to_raw).map(|s| s.into_owned()).unwrap_or_else(|_| to_raw.to_string());
    let enc = |s: &str| urlencoding::encode(s).into_owned();

    template
        .replace("{to}", &enc(&to))
        .replace("{subject}", &enc(&subject))
        .replace("{body}", &enc(&body))
        .replace("{cc}", &enc(&cc))
        .replace("{bcc}", &enc(&bcc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_template_detected() {
        let t = compose_template_for("https://mail.google.com").unwrap();
        assert!(t.contains("view=cm"));
        assert!(compose_template_for("https://example.com").is_none());
    }

    #[test]
    fn expand_fills_and_encodes() {
        let t = "https://m/?to={to}&su={subject}&body={body}";
        let out = expand(t, "mailto:a@b.com?subject=Hi%20there&body=Line+1");
        assert!(out.contains("to=a%40b.com"));
        assert!(out.contains("su=Hi%20there"));
    }
}
