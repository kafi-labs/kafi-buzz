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

    /// Content-preservation coverage for the paragraph (`\n\n`) split path.
    ///
    /// `large_splits_on_paragraphs` above only checks chunk count/size, so a bug that
    /// silently dropped or reordered a whole paragraph would still pass it. Here every
    /// paragraph is tagged with a distinct, zero-padded index so a dropped, duplicated,
    /// or reordered paragraph is directly detectable. Comparison is whitespace-normalised
    /// (each paragraph is `.trim()`-ed before comparison) because `chunk_content` is
    /// documented to trim seam whitespace — that is expected, not a defect.
    #[test]
    fn paragraph_path_preserves_all_markers_in_order() {
        let filler = "x".repeat(50);
        let para_count = 3072usize;
        let mut text = String::new();
        for i in 0..para_count {
            if i > 0 {
                text.push_str("\n\n");
            }
            text.push_str(&format!("PARA{i:04}-{filler}"));
        }
        assert!(
            text.len() > CHUNK_SOFT_LIMIT * 2,
            "test text must be comfortably oversized to force multiple chunks"
        );

        let chunks = chunk_content(&text);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for c in &chunks {
            assert!(
                c.len() <= CHUNK_SOFT_LIMIT,
                "chunk of {} bytes exceeds soft limit",
                c.len()
            );
        }

        // Internal "\n\n" separators inside a chunk survive untouched — only the seam at
        // each chunk boundary is trimmed — so splitting each chunk on "\n\n" and trimming
        // recovers the original paragraphs losslessly.
        let mut found = Vec::with_capacity(para_count);
        for chunk in &chunks {
            for part in chunk.split("\n\n") {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let (marker, body) = part
                    .split_once('-')
                    .unwrap_or_else(|| panic!("malformed paragraph fragment: {part:?}"));
                let idx: usize = marker
                    .strip_prefix("PARA")
                    .unwrap_or_else(|| panic!("missing PARA prefix in {marker:?}"))
                    .parse()
                    .unwrap_or_else(|_| panic!("non-numeric paragraph index in {marker:?}"));
                assert_eq!(body, filler, "paragraph {idx} body corrupted");
                found.push(idx);
            }
        }

        assert_eq!(
            found.len(),
            para_count,
            "expected {para_count} paragraph markers, found {} (dropped or merged paragraphs)",
            found.len()
        );
        assert_eq!(
            found,
            (0..para_count).collect::<Vec<_>>(),
            "paragraph markers out of order or duplicated"
        );
    }

    /// Same content-preservation guarantee as the paragraph-path test above, but for the
    /// single-newline fallback: text with `\n`-separated lines and deliberately no `\n\n`
    /// anywhere, so `chunk_content` must fall through to the `rfind('\n')` branch.
    #[test]
    fn single_newline_path_preserves_all_markers_in_order() {
        let filler = "y".repeat(50);
        let line_count = 3072usize;
        let mut text = String::new();
        for i in 0..line_count {
            if i > 0 {
                text.push('\n');
            }
            text.push_str(&format!("LINE{i:04}-{filler}"));
        }
        // Sanity: this construction must not accidentally contain a paragraph break,
        // otherwise we'd be exercising the "\n\n" path instead of the "\n" path.
        assert!(!text.contains("\n\n"));
        assert!(
            text.len() > CHUNK_SOFT_LIMIT * 2,
            "test text must be comfortably oversized to force multiple chunks"
        );

        let chunks = chunk_content(&text);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for c in &chunks {
            assert!(c.len() <= CHUNK_SOFT_LIMIT);
            assert!(
                !c.contains("\n\n"),
                "chunk unexpectedly contains a paragraph break"
            );
        }

        let mut found = Vec::with_capacity(line_count);
        for chunk in &chunks {
            for part in chunk.split('\n') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let (marker, body) = part
                    .split_once('-')
                    .unwrap_or_else(|| panic!("malformed line fragment: {part:?}"));
                let idx: usize = marker
                    .strip_prefix("LINE")
                    .unwrap_or_else(|| panic!("missing LINE prefix in {marker:?}"))
                    .parse()
                    .unwrap_or_else(|_| panic!("non-numeric line index in {marker:?}"));
                assert_eq!(body, filler, "line {idx} body corrupted");
                found.push(idx);
            }
        }

        assert_eq!(
            found.len(),
            line_count,
            "expected {line_count} line markers, found {} (dropped or merged lines)",
            found.len()
        );
        assert_eq!(
            found,
            (0..line_count).collect::<Vec<_>>(),
            "line markers out of order or duplicated"
        );
    }

    /// Content-preservation guarantee for the space-fallback path: text with spaces but
    /// no newlines at all, forcing `chunk_content` down to `rfind(' ')`. Asserts that no
    /// non-whitespace character is lost, duplicated, or reordered.
    #[test]
    fn space_fallback_path_preserves_all_tokens_in_order() {
        let token_count = 20_000usize;
        let mut expected_tokens = Vec::with_capacity(token_count);
        let mut text = String::new();
        for i in 0..token_count {
            if i > 0 {
                text.push(' ');
            }
            let tok = format!("TOK{i:05}");
            text.push_str(&tok);
            expected_tokens.push(tok);
        }
        assert!(!text.contains('\n'));
        assert!(
            text.len() > CHUNK_SOFT_LIMIT * 2,
            "test text must be comfortably oversized to force multiple chunks"
        );

        let chunks = chunk_content(&text);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for c in &chunks {
            assert!(c.len() <= CHUNK_SOFT_LIMIT);
            assert!(!c.contains('\n'));
        }

        let found: Vec<&str> = chunks.iter().flat_map(|c| c.split_whitespace()).collect();
        assert_eq!(
            found.len(),
            expected_tokens.len(),
            "token count mismatch: some tokens were dropped or merged"
        );
        assert_eq!(found, expected_tokens, "tokens reordered or corrupted");

        // No non-whitespace character lost: concatenating tokens in found order
        // reproduces the original non-whitespace content exactly.
        let expected_chars: String = expected_tokens.concat();
        let found_chars: String = found.concat();
        assert_eq!(found_chars, expected_chars);
    }

    /// Characterizes the *intended* trimming behavior at a chunk seam: `chunk_content` is
    /// documented to eat the whitespace immediately surrounding a split point (so chunks
    /// don't start/end with stray blank lines), but it must NEVER eat non-whitespace
    /// characters adjacent to that seam. This pins the exact boundary so a future change
    /// that starts trimming real characters (instead of only whitespace) fails loudly,
    /// rather than being masked by the coarser ">CHUNK_SOFT_LIMIT" checks elsewhere.
    #[test]
    fn seam_trims_only_whitespace_never_real_characters_characterization() {
        let head_marker = "HEADEND";
        let tail_marker = "TAILSTART";
        // Deliberately messy seam: two "\n\n" paragraph breaks with a stray space between
        // them, plus trailing spaces before the next real content starts. rfind("\n\n")
        // lands on the *second* "\n\n", so trim_end() must eat "\n\n \n\n" off the head
        // side and trim_start() must eat the trailing "   " off the tail side — proving
        // the trim consumes the *entire* whitespace run, not just one adjacent char.
        let seam = "\n\n \n\n   ";
        let tail_filler = "t".repeat(2_000);
        let filler_len = CHUNK_SOFT_LIMIT - head_marker.len() - seam.len() - 50;
        let filler = "h".repeat(filler_len);

        let text = format!("{filler}{head_marker}{seam}{tail_marker}{tail_filler}");
        assert!(text.len() > CHUNK_SOFT_LIMIT);

        let chunks = chunk_content(&text);
        assert_eq!(
            chunks.len(),
            2,
            "expected exactly one split for this construction"
        );

        // Non-whitespace content on both sides of the seam survives byte-for-byte...
        assert!(chunks[0].ends_with(head_marker));
        assert!(chunks[1].starts_with(tail_marker));
        // ...and no whitespace at all remains at either seam edge...
        assert!(!chunks[0].ends_with(|c: char| c.is_whitespace()));
        assert!(!chunks[1].starts_with(|c: char| c.is_whitespace()));
        // ...meaning the seam whitespace was removed in its entirety, not just one
        // character of it, and nothing else was touched: concatenating the chunks
        // reproduces the original text with exactly the seam whitespace deleted.
        let expected = format!("{filler}{head_marker}{tail_marker}{tail_filler}");
        assert_eq!(chunks.concat(), expected);
    }
}
