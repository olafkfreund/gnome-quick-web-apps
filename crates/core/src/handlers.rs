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

    roles
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
