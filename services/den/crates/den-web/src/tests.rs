use std::path::{Path, PathBuf};

use crate::{config::Config, template_environment};

fn collect_template_names(root: &Path) -> Vec<String> {
    fn visit(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let mut entries = std::fs::read_dir(dir)
            .unwrap_or_else(|err| panic!("failed to read template dir {}: {err}", dir.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|err| {
                panic!("failed to enumerate template dir {}: {err}", dir.display())
            });
        entries.sort_by_key(|entry| entry.path());

        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, out);
                continue;
            }

            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            if !matches!(extension, "html" | "jinja") {
                continue;
            }

            let relative = path
                .strip_prefix(root)
                .unwrap_or_else(|err| {
                    panic!(
                        "failed to strip template root {} from {}: {err}",
                        root.display(),
                        path.display()
                    )
                })
                .to_string_lossy()
                .replace('\\', "/");
            out.push(relative);
        }
    }

    let mut names = Vec::new();
    visit(root, root, &mut names);
    names
}

fn source_template_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/templates")
}

#[test]
fn source_templates_parse() {
    let mut config = Config::test_stub();
    config.templates_dir = source_template_root().to_string_lossy().into_owned();
    let env = template_environment(&config);

    let template_names = collect_template_names(Path::new(&config.templates_dir));
    assert!(
        !template_names.is_empty(),
        "expected source templates under {}",
        config.templates_dir
    );

    for template_name in template_names {
        env.get_template(&template_name)
            .unwrap_or_else(|err| panic!("template {template_name:?} failed to parse: {err:#}"));
    }
}

#[cfg(feature = "production")]
#[test]
fn embedded_templates_load_and_parse() {
    let config = Config::test_stub();
    let env = template_environment(&config);

    for template_name in collect_template_names(&source_template_root()) {
        env.get_template(&template_name).unwrap_or_else(|err| {
            panic!("embedded template {template_name:?} failed to parse: {err:#}")
        });
    }
}
