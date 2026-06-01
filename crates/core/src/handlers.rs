//! Default URL-scheme handler roles an app can fulfil, chosen by its URL.
//! Lets the editor show only relevant "Set as default for…" toggles
//! (email for web mail, calendar for web calendars, …) and nothing for apps
//! like Google Drive that aren't a default handler for anything.

/// A default-handler capability the editor can offer for a given app.
pub struct HandlerRole {
    pub label: String,
    pub subtitle: String,
    /// Freedesktop mime, e.g. `x-scheme-handler/mailto`.
    pub mime: String,
    /// Target URL template (`{to}/{subject}/…` for mailto, `{value}` otherwise).
    pub template: String,
}

/// All default-handler roles `app_url` qualifies for (possibly empty).
pub fn roles_for(app_url: &str) -> Vec<HandlerRole> {
    let mut roles = Vec::new();

    if let Some(template) = crate::mailto::compose_template_for(app_url) {
        roles.push(HandlerRole {
            label: "Use as default email app".to_string(),
            subtitle: "Open mailto: links here".to_string(),
            mime: "x-scheme-handler/mailto".to_string(),
            template,
        });
    }

    if let Some(template) = calendar_template(app_url) {
        roles.push(HandlerRole {
            label: "Use as default calendar".to_string(),
            subtitle: "Open webcal: subscriptions here".to_string(),
            mime: "x-scheme-handler/webcal".to_string(),
            template,
        });
    }

    if let Some(template) = call_template(app_url) {
        roles.push(HandlerRole {
            label: "Use for phone calls".to_string(),
            subtitle: "Open tel: / click-to-call links here".to_string(),
            mime: "x-scheme-handler/tel".to_string(),
            template,
        });
    }

    if let Some(role) = deeplink_role(app_url) {
        roles.push(role);
    }

    roles
}

/// App-specific deep-link scheme (msteams:, zoommtg:, slack:, spotify:, tg:)
/// that has no native handler on Linux — routing it to the web app makes
/// otherwise-dead "Open in app" links (e.g. Teams meeting invites) work.
fn deeplink_role(app_url: &str) -> Option<HandlerRole> {
    let host = host_of(app_url)?;
    let (scheme, label) = if host.contains("teams.microsoft") {
        ("msteams", "Open Microsoft Teams links")
    } else if host.contains("zoom.us") || host.contains("zoom.com") {
        ("zoommtg", "Open Zoom meeting links")
    } else if host.contains("slack.com") {
        ("slack", "Open Slack links")
    } else if host.contains("spotify") {
        ("spotify", "Open Spotify links")
    } else if host.contains("telegram") {
        ("tg", "Open Telegram links")
    } else {
        return None;
    };
    Some(HandlerRole {
        label: label.to_string(),
        subtitle: format!("Route {scheme}: deep links to this app (no native Linux app)"),
        mime: format!("x-scheme-handler/{scheme}"),
        template: app_url.to_string(), // fallback; real translation is bespoke
    })
}

/// Translate an app-specific deep-link URI into the equivalent web URL.
fn translate_deeplink(scheme: &str, arg: &str) -> Option<String> {
    let rest = arg.splitn(2, ':').nth(1)?; // everything after "scheme:"
    match scheme {
        // msteams:/l/meetup-join/... -> teams.microsoft.com/l/meetup-join/...
        "msteams" => {
            let path = rest.trim_start_matches('/');
            Some(format!("https://teams.microsoft.com/{path}"))
        }
        // spotify:track:ID -> open.spotify.com/track/ID
        "spotify" => {
            let path = rest.replace(':', "/");
            Some(format!("https://open.spotify.com/{}", path.trim_start_matches('/')))
        }
        // zoommtg://zoom.us/join?confno=NN&pwd=XX -> zoom.us/wc/join/NN?pwd=XX
        "zoommtg" => {
            let (_, query) = rest.split_once('?')?;
            let p = query_pairs(query);
            let confno = p.iter().find(|(k, _)| k == "confno")?.1.clone();
            let mut url = format!("https://zoom.us/wc/join/{confno}");
            if let Some((_, pwd)) = p.iter().find(|(k, _)| k == "pwd") {
                url.push_str(&format!("?pwd={pwd}"));
            }
            Some(url)
        }
        // slack://channel?team=T&id=C -> app.slack.com/client/T/C
        "slack" => {
            let (_, query) = rest.split_once('?')?;
            let p = query_pairs(query);
            let team = p.iter().find(|(k, _)| k == "team")?.1.clone();
            let id = p.iter().find(|(k, _)| k == "id").map(|(_, v)| v.clone()).unwrap_or_default();
            Some(format!("https://app.slack.com/client/{team}/{id}"))
        }
        // tg://resolve?domain=X -> web.telegram.org/k/#@X ; join?invite=H -> t.me/+H
        "tg" => {
            let (_, query) = rest.trim_start_matches('/').split_once('?')?;
            let p = query_pairs(query);
            if let Some((_, d)) = p.iter().find(|(k, _)| k == "domain") {
                Some(format!("https://web.telegram.org/k/#@{d}"))
            } else {
                p.iter()
                    .find(|(k, _)| k == "invite")
                    .map(|(_, h)| format!("https://t.me/+{h}"))
            }
        }
        _ => None,
    }
}

fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_depend_on_host() {
        assert!(roles_for("https://mail.google.com").iter().any(|r| r.mime.ends_with("mailto")));
        assert!(roles_for("https://teams.microsoft.com").iter().any(|r| r.mime.ends_with("msteams")));
        // Google Drive is a default handler for nothing.
        assert!(roles_for("https://drive.google.com").is_empty());
    }

    #[test]
    fn deeplinks_translate() {
        assert_eq!(
            expand("", "msteams:/l/meetup-join/19%3ameeting_abc"),
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc"
        );
        assert_eq!(expand("", "spotify:track:4uLU6hMCjMI75M1A2tKUQC"),
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC");
        assert_eq!(expand("", "zoommtg://zoom.us/join?confno=123456789&pwd=xyz"),
            "https://zoom.us/wc/join/123456789?pwd=xyz");
    }
}

fn call_template(app_url: &str) -> Option<String> {
    let host = host_of(app_url)?;
    if host.contains("teams.microsoft") {
        Some("https://teams.microsoft.com/l/call/0/0?users=4:{value}".to_string())
    } else if host.contains("voice.google") {
        Some("https://voice.google.com/u/0/calls?a=nc,{value}".to_string())
    } else if host.contains("whatsapp") {
        Some("https://wa.me/{value}".to_string())
    } else if host.contains("web.skype") || host == "skype.com" {
        Some("https://web.skype.com/call?phone={value}".to_string())
    } else {
        None
    }
}

fn host_of(app_url: &str) -> Option<String> {
    url::Url::parse(app_url)
        .ok()?
        .host_str()
        .map(|h| h.trim_start_matches("www.").to_lowercase())
}

fn calendar_template(app_url: &str) -> Option<String> {
    let host = host_of(app_url)?;
    if host.contains("calendar.google") {
        Some("https://calendar.google.com/calendar/r?cid={value}".to_string())
    } else if host.contains("outlook.") {
        Some("https://outlook.office.com/calendar/0/addfromweb?url={value}".to_string())
    } else {
        None
    }
}

/// Expand a handler `template` for an incoming scheme URL `arg`. `mailto:`
/// uses the rich field expansion; everything else fills `{value}`.
pub fn expand(template: &str, arg: &str) -> String {
    if arg.starts_with("mailto:") {
        return crate::mailto::expand(template, arg);
    }
    let scheme = arg.split(':').next().unwrap_or("");

    // App-specific deep links translate to a full web URL directly.
    if matches!(scheme, "msteams" | "zoommtg" | "slack" | "spotify" | "tg") {
        if let Some(url) = translate_deeplink(scheme, arg) {
            return url;
        }
    }

    let raw = arg.splitn(2, ':').nth(1).unwrap_or("");
    let value = match scheme {
        // Phone schemes: keep a leading '+' and digits only.
        "tel" | "callto" | "sms" => {
            let num = raw.split(['?', '&']).next().unwrap_or(raw);
            let mut out = String::new();
            for (i, c) in num.chars().enumerate() {
                if c.is_ascii_digit() || (i == 0 && c == '+') {
                    out.push(c);
                }
            }
            out
        }
        // webcal etc.: pass the full URL.
        _ => arg.to_string(),
    };
    template.replace("{value}", &urlencoding::encode(&value))
}
