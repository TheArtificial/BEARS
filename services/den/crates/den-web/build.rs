//! Build script for `den-web`.
//!
//! Validates/embeds the web template tree at build time so MiniJinja syntax errors
//! fail normal `cargo check`/build preflights instead of only failing production
//! template embedding. Dev runtime still path-loads templates from `config.templates_dir`,
//! default `crates/den-web/src/templates`. Assets are embedded by the `memory_serve`
//! proc-macro at compile time, so they need no build-script step here.
use std::{
    fs,
    path::{Path, PathBuf},
};

fn emit_rerun_if_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());

    if path.is_dir() {
        let mut entries = fs::read_dir(path)
            .unwrap_or_else(|err| panic!("failed to read template path {}: {err}", path.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|err| {
                panic!(
                    "failed to enumerate template path {}: {err}",
                    path.display()
                )
            });
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            emit_rerun_if_changed(&entry.path());
        }
    }
}

fn collect_template_names(root: &Path) -> Vec<String> {
    fn visit(root: &Path, dir: &Path, names: &mut Vec<String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap_or_else(|err| panic!("failed to read template path {}: {err}", dir.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|err| {
                panic!("failed to enumerate template path {}: {err}", dir.display())
            });
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, names);
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("html" | "jinja")
            ) {
                names.push(
                    path.strip_prefix(root)
                        .expect("template must be under template root")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut names = Vec::new();
    visit(root, root, &mut names);
    names
}

fn strip_minijinja_blocks(source: &str) -> String {
    let mut output = source.as_bytes().to_vec();
    let pairs = [
        (b"{{".as_slice(), b"}}".as_slice()),
        (b"{%".as_slice(), b"%}".as_slice()),
        (b"{#".as_slice(), b"#}".as_slice()),
    ];
    let mut cursor = 0;
    while cursor + 1 < output.len() {
        let Some((open, close)) = pairs
            .iter()
            .find(|(open, _)| output[cursor..].starts_with(open))
        else {
            cursor += 1;
            continue;
        };
        let content_start = cursor + open.len();
        let relative_end = output[content_start..]
            .windows(close.len())
            .position(|window| window == *close)
            .unwrap_or_else(|| panic!("unterminated MiniJinja block while validating HTML"));
        let end = content_start + relative_end + close.len();
        output[cursor..end].fill(b' ');
        cursor = end;
    }
    String::from_utf8(output).expect("template source remained UTF-8 after stripping blocks")
}

fn validate_html_structure(template_name: &str, source: &str) {
    const VOID: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ];
    const TRACKED: &[&str] = &[
        "button", "code", "details", "form", "label", "section", "select", "small", "strong",
        "summary", "table", "tbody", "td", "textarea", "th", "thead", "tr", "ul",
    ];
    let source = strip_minijinja_blocks(source);
    let source = source.as_str();
    let mut stack: Vec<String> = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = source[cursor..].find('<') {
        let start = cursor + relative_start;
        let Some(relative_end) = source[start..].find('>') else {
            panic!("template {template_name:?} contains an unterminated HTML tag");
        };
        let end = start + relative_end;
        let raw = source[start + 1..end].trim();
        cursor = end + 1;
        if raw.is_empty() || raw.starts_with('!') || raw.starts_with('?') {
            continue;
        }
        assert!(
            !raw.contains('<'),
            "template {template_name:?} contains malformed HTML tag <{raw}>"
        );
        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/').trim_start();
        let name = body
            .split(|ch: char| ch.is_ascii_whitespace() || ch == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.is_empty()
            || !name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        {
            panic!("template {template_name:?} contains malformed HTML tag <{raw}>");
        }
        if !closing && matches!(name.as_str(), "script" | "style") {
            let closing_tag = format!("</{name}>");
            let relative_close = source[cursor..].find(&closing_tag).unwrap_or_else(|| {
                panic!("template {template_name:?} has an unclosed <{name}> block")
            });
            cursor += relative_close + closing_tag.len();
            continue;
        }
        if !TRACKED.contains(&name.as_str()) {
            continue;
        }
        if closing {
            let open = stack.pop().unwrap_or_else(|| {
                panic!("template {template_name:?} closes </{name}> without an opening tag")
            });
            assert_eq!(
                open, name,
                "template {template_name:?} closes </{name}> while <{open}> is still open"
            );
        } else if !VOID.contains(&name.as_str()) && !raw.ends_with('/') {
            stack.push(name);
        }
    }
    assert!(
        stack.is_empty(),
        "template {template_name:?} has unclosed HTML tags: {stack:?}"
    );
}

fn validate_templates(templates_dir: &Path) {
    let canonical = templates_dir
        .canonicalize()
        .unwrap_or_else(|err| panic!("failed to canonicalize {}: {err}", templates_dir.display()));
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader(PathBuf::from(&canonical)));
    // Parsing resolves filter names. Runtime supplies the real implementations;
    // build-time validation needs only signatures so syntax/inheritance compile.
    for name in [
        "hexadecimal",
        "urlencode",
        "markdown",
        "timeago",
        "humanize_time",
        "is_future",
    ] {
        env.add_filter(name, |value: minijinja::Value| value);
    }
    for template_name in collect_template_names(&canonical) {
        let source = fs::read_to_string(canonical.join(&template_name)).unwrap_or_else(|err| {
            panic!("failed to read template {template_name:?} for validation: {err}")
        });
        validate_html_structure(&template_name, &source);
        env.get_template(&template_name).unwrap_or_else(|err| {
            panic!("template {template_name:?} failed build-time validation: {err:#}")
        });
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=TEMPLATES_DIR");
    let requested_templates_dir = std::env::var("TEMPLATES_DIR").ok();
    let templates_dir = requested_templates_dir
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty() && Path::new(path).is_dir())
        .unwrap_or("src/templates");
    let templates_path = Path::new(templates_dir);
    emit_rerun_if_changed(templates_path);
    validate_templates(templates_path);
    minijinja_embed::embed_templates!(templates_dir);
}
