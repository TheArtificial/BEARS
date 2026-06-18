//! Pure text helpers for the web tools (no I/O, no policy).

/// Strip HTML tags and decode a small set of entities into a line-trimmed
/// plain-text excerpt. Intentionally simple (not a full HTML parser): it is only
/// used to make fetched pages legible to a model.
pub fn html_to_text_excerpt(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len().min(64_000));
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncate to at most `max_chars` characters, returning the (possibly truncated)
/// string and whether truncation occurred. Character-based (not byte-based) so it
/// never splits a UTF-8 scalar.
pub fn truncate_chars(value: &str, max_chars: usize) -> (String, bool) {
    let mut out = String::new();
    let mut truncated = false;
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            truncated = true;
            break;
        }
        out.push(ch);
    }
    (out, truncated)
}
