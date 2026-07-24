//! Reply content chunking for the 64 KiB event content cap.

/// Soft chunk size (~60 KiB) leaving headroom under the 64 KiB builder cap.
pub const CHUNK_SOFT_LIMIT: usize = 60 * 1024;

/// Split text into chunks at paragraph boundaries when possible.
///
/// Empty input yields an empty vec (caller should post nothing).
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

        let window = &remaining[..CHUNK_SOFT_LIMIT];
        // Prefer double-newline, then single newline, then space.
        let split_at = window
            .rfind("\n\n")
            .map(|i| i + 2)
            .or_else(|| window.rfind('\n').map(|i| i + 1))
            .or_else(|| window.rfind(' ').map(|i| i + 1))
            .filter(|&i| i > 0)
            .unwrap_or(CHUNK_SOFT_LIMIT);

        let (head, tail) = remaining.split_at(split_at);
        let head = head.trim_end();
        if !head.is_empty() {
            chunks.push(head.to_owned());
        }
        remaining = tail.trim_start();
    }

    chunks
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
        assert_eq!(
            chunks.join("\n\n").replace("\n\n\n\n", "\n\n").len() > 0,
            true
        );
    }
}
