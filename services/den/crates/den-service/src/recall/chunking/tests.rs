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
