//! Passage chunking + content hashing for the derived recall index (ADR-0038 §4).
//!
//! Targets ~400–800 tokens per chunk with overlap. We approximate tokens by characters
//! (~4 chars/token) and prefer to break on whitespace so chunks stay coherent. Each chunk
//! carries a stable `content_hash` so re-embedding only happens when content changes.

use sha2::{Digest, Sha256};

/// Target chunk size in characters (~600 tokens at ~4 chars/token).
const TARGET_CHARS: usize = 2400;
/// Overlap in characters carried between consecutive chunks (~50 tokens).
const OVERLAP_CHARS: usize = 200;

/// A single chunk of a memory record's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub index: i32,
    pub text: String,
    pub content_hash: String,
}

/// Stable hex SHA-256 of `text` (used for chunk dedup).
pub fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Split `content` into overlapping chunks. Empty/whitespace-only input yields no chunks.
pub fn chunk_text(content: &str) -> Vec<Chunk> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Char-boundary-safe windowing over the trimmed content.
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= TARGET_CHARS {
        return vec![Chunk {
            index: 0,
            text: trimmed.to_string(),
            content_hash: content_hash(trimmed),
        }];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut index = 0i32;
    while start < chars.len() {
        let hard_end = (start + TARGET_CHARS).min(chars.len());
        // Prefer to break at the last whitespace before the hard end (unless we're at the
        // very end of the content), so we don't split mid-word.
        let end = if hard_end < chars.len() {
            chars[start..hard_end]
                .iter()
                .rposition(|c| c.is_whitespace())
                .map(|rel| start + rel + 1)
                .filter(|&e| e > start)
                .unwrap_or(hard_end)
        } else {
            hard_end
        };

        let text: String = chars[start..end].iter().collect();
        let text = text.trim().to_string();
        if !text.is_empty() {
            let hash = content_hash(&text);
            chunks.push(Chunk {
                index,
                text,
                content_hash: hash,
            });
            index += 1;
        }

        if end >= chars.len() {
            break;
        }
        // Advance with overlap, but always make forward progress.
        start = end.saturating_sub(OVERLAP_CHARS).max(start + 1);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_content_yields_no_chunks() {
        assert!(chunk_text("").is_empty());
        assert!(chunk_text("   \n  ").is_empty());
    }

    #[test]
    fn short_content_is_a_single_chunk() {
        let chunks = chunk_text("bears keep canonical memory in sqlite");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].text, "bears keep canonical memory in sqlite");
        assert_eq!(chunks[0].content_hash, content_hash(&chunks[0].text));
    }

    #[test]
    fn content_hash_is_stable_and_distinct() {
        assert_eq!(content_hash("alpha"), content_hash("alpha"));
        assert_ne!(content_hash("alpha"), content_hash("beta"));
        let h = content_hash("alpha");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn long_content_splits_into_ordered_overlapping_chunks() {
        let word = "lorem ";
        let content = word.repeat(1000); // ~6000 chars
        let chunks = chunk_text(&content);
        assert!(chunks.len() >= 2, "expected multiple chunks");
        // Indices are contiguous starting at 0.
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.index, i as i32);
            assert!(!c.text.is_empty());
            assert_eq!(c.content_hash, content_hash(&c.text));
        }
    }

    #[test]
    fn unicode_content_does_not_panic_on_boundaries() {
        let content = "🐻memory ".repeat(800);
        let chunks = chunk_text(&content);
        assert!(!chunks.is_empty());
    }
}
