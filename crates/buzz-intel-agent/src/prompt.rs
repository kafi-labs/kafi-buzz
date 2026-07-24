//! Harness prompt parsing — channel id + reply-to extraction.

use uuid::Uuid;

/// Fields extracted from a harness-formatted prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedPrompt {
    /// Buzz channel UUID when extractable.
    pub channel_id: Option<Uuid>,
    /// Event id to use for `--reply-to` threading.
    pub reply_to_event_id: Option<String>,
}

/// Parse channel_id and reply_to from a harness prompt.
///
/// Tolerant: returns whatever it can find; callers still forward the turn
/// even when fields are missing (unthreaded post).
pub fn parse_prompt(text: &str) -> ParsedPrompt {
    // Primary: reply-instruction line.
    // "… use `--reply-to <event_id>` on `buzz messages send` …"
    let mut reply_to_event_id = extract_reply_to(text);
    let mut channel_id = None;

    // Channel + Event ID from event blocks (walk all; last event wins for reply fallback).
    let mut last_event_id: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Channel:") {
            if let Some(id) = parse_channel_line(rest.trim()) {
                channel_id = Some(id);
            }
        } else if let Some(rest) = line.strip_prefix("Event ID:") {
            let id = rest.trim();
            if is_hex64(id) {
                last_event_id = Some(id.to_ascii_lowercase());
            }
        }
    }

    if reply_to_event_id.is_none() {
        reply_to_event_id = last_event_id;
    }

    ParsedPrompt {
        channel_id,
        reply_to_event_id,
    }
}

/// Extract `--reply-to <hex>` from the reply-instruction appendix.
fn extract_reply_to(text: &str) -> Option<String> {
    // Match both backtick and plain forms.
    const MARKERS: &[&str] = &["`--reply-to ", "--reply-to "];
    for marker in MARKERS {
        if let Some(idx) = text.find(marker) {
            let after = &text[idx + marker.len()..];
            let id: String = after
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            if is_hex64(&id) {
                return Some(id.to_ascii_lowercase());
            }
        }
    }
    None
}

/// Parse `Channel: <name> (#<uuid>)` or bare `Channel: <uuid>`.
fn parse_channel_line(rest: &str) -> Option<Uuid> {
    // Named form: "general (#aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee)"
    if let Some(open) = rest.rfind("(#") {
        let after = &rest[open + 2..];
        if let Some(close) = after.find(')') {
            let candidate = after[..close].trim();
            if let Ok(u) = Uuid::parse_str(candidate) {
                return Some(u);
            }
        }
    }
    // Bare UUID form.
    let candidate = rest.trim();
    // May still have trailing noise; take first whitespace-delimited token.
    let token = candidate.split_whitespace().next().unwrap_or(candidate);
    Uuid::parse_str(token).ok()
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_NAMED: &str = "\
Event ID: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n\
Channel: general (#11111111-2222-3333-4444-555555555555)\n\
Kind: 9\n\
From: Alice (npub: npub1abc, hex: deadbeef)\n\
Time: 2026-01-01T00:00:00Z\n\
Content: hello agent\n\
Tags: [[\"h\",\"11111111-2222-3333-4444-555555555555\"]]\n\
\n\
IMPORTANT: For ordinary replies in this turn, use `--reply-to 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef` \
on `buzz messages send` so the conversation stays threaded. \
If the human explicitly asks for a channel-root, top-level, \
or broadcast post, send that message without `--reply-to`. \
If the requested destination is ambiguous, ask before sending.";

    const SAMPLE_BARE: &str = "\
Event ID: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
Channel: 11111111-2222-3333-4444-555555555555\n\
Kind: 9\n\
From: Bob\n\
Time: 2026-01-01T00:00:00Z\n\
Content: hi";

    #[test]
    fn parses_named_channel_and_reply_instruction() {
        let p = parse_prompt(SAMPLE_NAMED);
        assert_eq!(
            p.channel_id,
            Some(Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap())
        );
        assert_eq!(
            p.reply_to_event_id.as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn parses_bare_channel_falls_back_to_event_id() {
        let p = parse_prompt(SAMPLE_BARE);
        assert_eq!(
            p.channel_id,
            Some(Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap())
        );
        assert_eq!(
            p.reply_to_event_id.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn tolerant_on_garbage() {
        let p = parse_prompt("just free text with no structure");
        assert!(p.channel_id.is_none());
        assert!(p.reply_to_event_id.is_none());
    }

    #[test]
    fn newest_event_id_wins_for_fallback() {
        let text = "\
Event ID: 1111111111111111111111111111111111111111111111111111111111111111\n\
Channel: 11111111-2222-3333-4444-555555555555\n\
Content: older\n\
\n\
Event ID: 2222222222222222222222222222222222222222222222222222222222222222\n\
Channel: 11111111-2222-3333-4444-555555555555\n\
Content: newer";
        let p = parse_prompt(text);
        assert_eq!(
            p.reply_to_event_id.as_deref(),
            Some("2222222222222222222222222222222222222222222222222222222222222222")
        );
    }
}
