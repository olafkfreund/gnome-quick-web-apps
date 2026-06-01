//! Curated catalog of popular web apps users can add in one click. Each
//! template carries an Iconify icon id for a crisp logo (with a favicon
//! fallback if the id ever fails).

use crate::webapp::Category;

pub struct Template {
    pub name: &'static str,
    pub url: &'static str,
    pub category: Category,
    /// Iconify icon id, e.g. `logos:google-gmail`.
    pub icon: &'static str,
}

const fn t(
    name: &'static str,
    url: &'static str,
    category: Category,
    icon: &'static str,
) -> Template {
    Template {
        name,
        url,
        category,
        icon,
    }
}

/// The full curated list, roughly grouped by purpose.
pub fn all() -> Vec<Template> {
    use Category::*;
    vec![
        // Google
        t(
            "Gmail",
            "https://mail.google.com",
            Network,
            "logos:google-gmail",
        ),
        t(
            "Google Calendar",
            "https://calendar.google.com",
            Office,
            "logos:google-calendar",
        ),
        t(
            "Google Drive",
            "https://drive.google.com",
            Office,
            "logos:google-drive",
        ),
        t(
            "Google Docs",
            "https://docs.google.com",
            Office,
            "logos:google-docs",
        ),
        t(
            "Google Meet",
            "https://meet.google.com",
            Network,
            "logos:google-meet",
        ),
        t(
            "Google Photos",
            "https://photos.google.com",
            Graphics,
            "logos:google-photos",
        ),
        t(
            "Google Maps",
            "https://maps.google.com",
            Network,
            "logos:google-maps",
        ),
        // Microsoft
        t(
            "Outlook",
            "https://outlook.office.com",
            Network,
            "logos:microsoft-outlook",
        ),
        t(
            "Microsoft Teams",
            "https://teams.microsoft.com",
            Network,
            "logos:microsoft-teams",
        ),
        t(
            "Microsoft 365",
            "https://microsoft365.com",
            Office,
            "logos:microsoft-icon",
        ),
        t(
            "OneDrive",
            "https://onedrive.live.com",
            Office,
            "logos:microsoft-onedrive",
        ),
        // Communication
        t(
            "WhatsApp",
            "https://web.whatsapp.com",
            Network,
            "logos:whatsapp-icon",
        ),
        t(
            "Telegram",
            "https://web.telegram.org",
            Network,
            "logos:telegram",
        ),
        t(
            "Discord",
            "https://discord.com/app",
            Network,
            "logos:discord-icon",
        ),
        t(
            "Slack",
            "https://app.slack.com",
            Network,
            "logos:slack-icon",
        ),
        t(
            "Messenger",
            "https://messenger.com",
            Network,
            "logos:messenger",
        ),
        t(
            "Proton Mail",
            "https://mail.proton.me",
            Network,
            "simple-icons:protonmail",
        ),
        // Social (web-only / web-best on Linux)
        t("X", "https://x.com", Network, "simple-icons:x"),
        t(
            "Facebook",
            "https://facebook.com",
            Network,
            "logos:facebook",
        ),
        t(
            "Instagram",
            "https://instagram.com",
            Network,
            "skill-icons:instagram",
        ),
        t("TikTok", "https://tiktok.com", Video, "logos:tiktok-icon"),
        t(
            "LinkedIn",
            "https://linkedin.com",
            Network,
            "logos:linkedin-icon",
        ),
        t("Reddit", "https://reddit.com", Network, "logos:reddit-icon"),
        t(
            "Pinterest",
            "https://pinterest.com",
            Graphics,
            "logos:pinterest",
        ),
        t(
            "Threads",
            "https://threads.net",
            Network,
            "simple-icons:threads",
        ),
        t(
            "Bluesky",
            "https://bsky.app",
            Network,
            "simple-icons:bluesky",
        ),
        t(
            "Snapchat",
            "https://web.snapchat.com",
            Network,
            "logos:snapchat",
        ),
        t(
            "Mastodon",
            "https://mastodon.social",
            Network,
            "logos:mastodon-icon",
        ),
        // Productivity
        t("Notion", "https://notion.so", Office, "logos:notion-icon"),
        t("Trello", "https://trello.com", Office, "logos:trello"),
        t("Asana", "https://app.asana.com", Office, "logos:asana-icon"),
        t(
            "Todoist",
            "https://app.todoist.com",
            Office,
            "simple-icons:todoist",
        ),
        t("Figma", "https://figma.com", Graphics, "logos:figma"),
        t("Miro", "https://miro.com", Graphics, "logos:miro"),
        t("Canva", "https://canva.com", Graphics, "simple-icons:canva"),
        t(
            "Google Keep",
            "https://keep.google.com",
            Office,
            "logos:google-keep",
        ),
        // Apple (web-only on Linux)
        t(
            "iCloud",
            "https://icloud.com",
            Network,
            "simple-icons:icloud",
        ),
        t(
            "Apple Music",
            "https://music.apple.com",
            Audio,
            "simple-icons:applemusic",
        ),
        // Development
        t(
            "GitHub",
            "https://github.com",
            Development,
            "logos:github-icon",
        ),
        t("GitLab", "https://gitlab.com", Development, "logos:gitlab"),
        t(
            "Stack Overflow",
            "https://stackoverflow.com",
            Development,
            "logos:stackoverflow-icon",
        ),
        // Media
        t(
            "Spotify",
            "https://open.spotify.com",
            AudioVideo,
            "logos:spotify-icon",
        ),
        t(
            "YouTube",
            "https://youtube.com",
            Video,
            "logos:youtube-icon",
        ),
        t(
            "YouTube Music",
            "https://music.youtube.com",
            Audio,
            "simple-icons:youtubemusic",
        ),
        t(
            "Tidal",
            "https://listen.tidal.com",
            Audio,
            "simple-icons:tidal",
        ),
        t("Deezer", "https://deezer.com", Audio, "simple-icons:deezer"),
        t(
            "SoundCloud",
            "https://soundcloud.com",
            Audio,
            "logos:soundcloud",
        ),
        t(
            "Bandcamp",
            "https://bandcamp.com",
            Audio,
            "simple-icons:bandcamp",
        ),
        t(
            "Netflix",
            "https://netflix.com",
            Video,
            "logos:netflix-icon",
        ),
        t(
            "Disney+",
            "https://disneyplus.com",
            Video,
            "simple-icons:disneyplus",
        ),
        t(
            "Prime Video",
            "https://primevideo.com",
            Video,
            "simple-icons:primevideo",
        ),
        t("Max", "https://play.max.com", Video, "simple-icons:hbo"),
        t("Twitch", "https://twitch.tv", Video, "logos:twitch"),
        t(
            "Plex",
            "https://app.plex.tv",
            AudioVideo,
            "simple-icons:plex",
        ),
        // AI
        t(
            "ChatGPT",
            "https://chatgpt.com",
            Utility,
            "simple-icons:openai",
        ),
        t(
            "Codex",
            "https://chatgpt.com/codex",
            Development,
            "simple-icons:openai",
        ),
        t(
            "Claude",
            "https://claude.ai",
            Utility,
            "simple-icons:anthropic",
        ),
        t(
            "Gemini",
            "https://gemini.google.com",
            Utility,
            "simple-icons:googlegemini",
        ),
        t(
            "GitHub Copilot",
            "https://github.com/copilot",
            Development,
            "simple-icons:githubcopilot",
        ),
        t(
            "Perplexity",
            "https://perplexity.ai",
            Utility,
            "simple-icons:perplexity",
        ),
        t("Grok", "https://grok.com", Utility, "simple-icons:x"),
        t(
            "Mistral",
            "https://chat.mistral.ai",
            Utility,
            "simple-icons:mistralai",
        ),
        t(
            "DeepSeek",
            "https://chat.deepseek.com",
            Utility,
            "simple-icons:deepseek",
        ),
        t(
            "Hugging Face",
            "https://huggingface.co",
            Development,
            "logos:hugging-face-icon",
        ),
        // Shopping
        t("Amazon", "https://amazon.com", Network, "logos:amazon-icon"),
    ]
}
