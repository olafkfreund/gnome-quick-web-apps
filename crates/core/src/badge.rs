//! Unread-count extraction from a window/page title, for the dock badge.
//!
//! Many webmail/chat sites surface the unread count in the document title,
//! e.g. Gmail's "Inbox (3) - me@gmail.com" or Teams' "(2) | Microsoft Teams".
//! `count_from_title` pulls that number out with a small hand-written scanner
//! (no regex dependency), staying deliberately conservative: it prefers a
//! false negative (no badge) over a false positive (a wrong badge).

/// Extract the unread count from a window title, or `None` when absent/zero.
///
/// Heuristic, scanning left to right for the FIRST count marker:
///   - a number inside parentheses `(N)` or brackets `[N]`, or
///   - a leading bare `N ` token at the very start of the title.
///
/// A digit run is only accepted when its neighbours are non-digit,
/// non-alphabetic (so `10,000` or `v2` inside a word never count); a
/// parsed `0` returns `None` (an empty inbox should clear the badge).
pub fn count_from_title(title: &str) -> Option<u32> {
    let bytes = title.as_bytes();

    // Leading bare count: "12 WhatsApp" — only at the very start, followed by
    // whitespace (so a leading "2024-..." date-like token isn't a count).
    {
        let mut i = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i > 0 && (i == bytes.len() || bytes[i] == b' ') {
            if let Some(n) = parse_count(&title[..i]) {
                return Some(n);
            }
        }
    }

    // First parenthesized/bracketed integer: "(3)" or "[5]".
    let mut i = 0;
    while i < bytes.len() {
        let (open, close) = match bytes[i] {
            b'(' => (b'(', b')'),
            b'[' => (b'[', b']'),
            _ => {
                i += 1;
                continue;
            }
        };
        let _ = open;
        // Collect the digit run immediately following the opener.
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        // Accept only when the bracket holds *exactly* a digit run and closes,
        // e.g. "(3)" — not "(re: 3)" or "(v2)".
        if j > start && j < bytes.len() && bytes[j] == close {
            if let Some(n) = parse_count(&title[start..j]) {
                return Some(n);
            }
        }
        i += 1;
    }

    None
}

/// Parse a pure-digit string into a count, returning `None` for `0` or for
/// anything that isn't a valid `u32`.
fn parse_count(digits: &str) -> Option<u32> {
    match digits.parse::<u32>() {
        Ok(0) => None,
        Ok(n) => Some(n),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_inbox_count() {
        assert_eq!(count_from_title("Inbox (3) - me@gmail.com"), Some(3));
    }

    #[test]
    fn leading_parenthesized_count() {
        assert_eq!(count_from_title("(12) WhatsApp"), Some(12));
    }

    #[test]
    fn teams_count() {
        assert_eq!(count_from_title("(2) | Microsoft Teams"), Some(2));
    }

    #[test]
    fn no_count_titles() {
        assert_eq!(count_from_title("Calendar"), None);
        assert_eq!(count_from_title("YouTube"), None);
    }

    #[test]
    fn comma_grouped_number_is_not_a_count() {
        // A comma-grouped number that's not a bracketed/leading count marker.
        assert_eq!(count_from_title("10,000 Days - YouTube Music"), None);
    }

    #[test]
    fn bracketed_count() {
        assert_eq!(count_from_title("Slack [5] workspace"), Some(5));
    }

    #[test]
    fn empty_string() {
        assert_eq!(count_from_title(""), None);
    }

    #[test]
    fn zero_count_clears_badge() {
        assert_eq!(count_from_title("Inbox (0) - me@gmail.com"), None);
    }
}
