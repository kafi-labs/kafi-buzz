//! Reply content chunking for the 64 KiB event content cap.

/// Soft chunk size (~60 KiB) leaving headroom under the 64 KiB builder cap.
pub const CHUNK_SOFT_LIMIT: usize = 60 * 1024;

/// Split text into chunks at paragraph boundaries when possible.
///
/// Empty input yields an empty vec (caller should post nothing).
/// All split points are UTF-8 char boundaries — multi-byte codepoints
/// (emoji, CJK) that straddle the soft limit never panic.
pub fn chunk_content(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if text.len() <= CHUNK_SOFT_LIMIT {
        return vec![text.to_owned()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= CHUNK_SOFT_LIMIT {
            chunks.push(remaining.to_owned());
            break;
        }

        // Cap window at a char boundary so slicing never panics on multi-byte UTF-8.
        let window_end = floor_char_boundary(remaining, CHUNK_SOFT_LIMIT);
        if window_end == 0 {
            // Pathological: single char larger than soft limit — force-emit one char.
            let ch_len = remaining
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1)
                .min(remaining.len());
            chunks.push(remaining[..ch_len].to_owned());
            remaining = &remaining[ch_len..];
            continue;
        }
        let window = &remaining[..window_end];

        // Prefer double-newline, then single newline, then space — all within the window.
        let split_at = window
            .rfind("\n\n")
            .map(|i| i + 2)
            .or_else(|| window.rfind('\n').map(|i| i + 1))
            .or_else(|| window.rfind(' ').map(|i| i + 1))
            .filter(|&i| i > 0 && remaining.is_char_boundary(i))
            .unwrap_or(window_end);

        let (head, tail) = remaining.split_at(split_at);
        let head = head.trim_end();
        if !head.is_empty() {
            chunks.push(head.to_owned());
        }
        remaining = tail.trim_start();
    }

    chunks
}

/// Largest index `<= idx` that is a UTF-8 char boundary (or 0).
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_yields_nothing() {
        assert!(chunk_content("").is_empty());
    }

    #[test]
    fn small_is_single() {
        let c = chunk_content("hello");
        assert_eq!(c, vec!["hello".to_string()]);
    }

    #[test]
    fn large_splits_on_paragraphs() {
        let para = "a".repeat(1000);
        let mut text = String::new();
        while text.len() < CHUNK_SOFT_LIMIT + 5000 {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&para);
        }
        let chunks = chunk_content(&text);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.len() <= CHUNK_SOFT_LIMIT + 100);
        }
        assert!(!chunks.is_empty());
    }

    #[test]
    fn exact_soft_limit_is_single_chunk() {
        let text = "x".repeat(CHUNK_SOFT_LIMIT);
        let chunks = chunk_content(&text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), CHUNK_SOFT_LIMIT);
    }

    #[test]
    fn one_byte_over_soft_limit_splits() {
        let head = "h".repeat(CHUNK_SOFT_LIMIT - 10);
        let text = format!("{head}\n\n{}", "t".repeat(100));
        let chunks = chunk_content(&text);
        assert!(
            chunks.len() >= 2,
            "expected split, got {} chunks",
            chunks.len()
        );
        for c in &chunks {
            assert!(
                c.len() <= CHUNK_SOFT_LIMIT,
                "chunk len {} exceeds soft limit",
                c.len()
            );
        }
        let rejoined = chunks.join("\n\n");
        assert!(rejoined.contains('h') && rejoined.contains('t'));
    }

    #[test]
    fn splits_on_single_newline_when_no_paragraph_break() {
        let line = format!("{}\n", "a".repeat(80));
        let mut text = String::new();
        while text.len() < CHUNK_SOFT_LIMIT + 200 {
            text.push_str(&line);
        }
        let chunks = chunk_content(&text);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.len() <= CHUNK_SOFT_LIMIT);
        }
    }

    /// Critical regression: multi-byte emoji straddling the soft-limit boundary
    /// must not panic and must not split mid-codepoint.
    #[test]
    fn emoji_straddling_soft_limit_does_not_panic() {
        // 🔥 is 4 bytes (F0 9F 94 A5). Fill so the soft-limit index lands mid-emoji.
        let emoji = "🔥";
        assert_eq!(emoji.len(), 4);
        let prefix_len = CHUNK_SOFT_LIMIT - 2; // mid-emoji if we naively slice at SOFT_LIMIT
        let mut text = "a".repeat(prefix_len);
        // No whitespace near the boundary — forces char-boundary fallback.
        text.push_str(&emoji.repeat(2000));
        assert!(text.len() > CHUNK_SOFT_LIMIT);

        let chunks = chunk_content(&text);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.is_char_boundary(c.len()));
            assert!(
                c.len() <= CHUNK_SOFT_LIMIT + emoji.len(),
                "chunk {} bytes too large",
                c.len()
            );
            // No U+FFFD replacement — lossless relative to original chars.
            assert!(!c.contains('\u{FFFD}'));
        }
        // All chunks are valid UTF-8 (guaranteed by String) and rejoin conserves emoji count.
        let emoji_in: usize = text.matches(emoji).count();
        let emoji_out: usize = chunks.iter().map(|c| c.matches(emoji).count()).sum();
        assert_eq!(emoji_in, emoji_out);
    }

    /// CJK multi-byte chars with no ASCII whitespace in the window.
    #[test]
    fn cjk_no_whitespace_window_splits_on_char_boundary() {
        // "测" is 3 bytes in UTF-8. Build a long string past the soft limit.
        let unit = "测试内容";
        let mut text = String::new();
        while text.len() < CHUNK_SOFT_LIMIT + 1000 {
            text.push_str(unit);
        }
        let chunks = chunk_content(&text);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.is_char_boundary(c.len()));
            assert!(c.len() <= CHUNK_SOFT_LIMIT);
            assert!(!c.contains('\u{FFFD}'));
        }
        let rejoined: String = chunks.concat();
        // trim_end on heads may drop nothing here (no whitespace); full rejoin equals original.
        assert!(rejoined.chars().all(|ch| unit.contains(ch)));
        assert_eq!(rejoined.chars().count(), text.chars().count());
    }

    #[test]
    fn floor_char_boundary_inside_multibyte() {
        let s = "ab🔥cd"; // '🔥' starts at index 2, len 4 → occupies 2..6
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 2), 2);
        assert_eq!(floor_char_boundary(s, 3), 2); // mid-emoji → back to start
        assert_eq!(floor_char_boundary(s, 4), 2);
        assert_eq!(floor_char_boundary(s, 5), 2);
        assert_eq!(floor_char_boundary(s, 6), 6);
        assert_eq!(floor_char_boundary(s, 100), s.len());
    }
}
