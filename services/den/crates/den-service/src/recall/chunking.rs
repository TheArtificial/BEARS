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
mod tests;

