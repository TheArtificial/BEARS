//! Persistence for the Den-managed provider configuration.
//!
//! Den is the source of truth (its Postgres); the provider caches the last
//! pushed set under the workspaces volume so provisioning works between
//! pushes and across restarts:
//!
//! ```text
//! <workspaces_dir>/managed/config.json                 # no secret material
//! <workspaces_dir>/managed/credentials/<name>.sshkey   # 0600, one per surface
//! <workspaces_dir>/managed/credentials/<name>.token
//! ```
//!
//! Credential values are written to their own 0600 files and referenced from
//! the config as `ssh_key_path` / `token_path`; they never appear in
//! `config.json`, logs, or command lines.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::{ManagedConfig, ManagedCredential};
use crate::roots::{CatalogImageSpec, GitUpstream, RootCredential, RootsError, SyncableRoot};

const MANAGED_DIR: &str = "managed";
const CONFIG_FILE: &str = "config.json";
const CREDENTIALS_DIR: &str = "credentials";

/// The on-disk shape. Contains no secret values — credentials are path
/// references into the sibling `credentials/` directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedManagedConfig {
    #[serde(default)]
    pub version: Option<String>,
    pub roots: Vec<SyncableRoot>,
    #[serde(default)]
    pub images: Vec<CatalogImageSpec>,
}

fn managed_dir(workspaces_dir: &Path) -> PathBuf {
    workspaces_dir.join(MANAGED_DIR)
}

fn config_path(workspaces_dir: &Path) -> PathBuf {
    managed_dir(workspaces_dir).join(CONFIG_FILE)
}

fn credentials_dir(workspaces_dir: &Path) -> PathBuf {
    managed_dir(workspaces_dir).join(CREDENTIALS_DIR)
}

/// Same shape as the DB CHECK constraint Den enforces; re-validated here as
/// defense in depth because names become path components on this host.
pub fn name_is_valid(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// Load the persisted managed config, if any has been pushed.
pub fn load(workspaces_dir: &Path) -> Result<Option<PersistedManagedConfig>, RootsError> {
    let path = config_path(workspaces_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RootsError::ConfigRead {
                path: path.to_string_lossy().into_owned(),
                source,
            })
        }
    };
    let persisted = serde_json::from_str(&raw).map_err(|source| RootsError::ConfigParse {
        path: path.to_string_lossy().into_owned(),
        source,
    })?;
    Ok(Some(persisted))
}

/// Validate and persist a pushed managed config: write credential files
/// (0600, atomic), build the root set, persist `config.json` atomically, and
/// remove credential files for surfaces no longer present.
pub fn apply(
    workspaces_dir: &Path,
    config: &ManagedConfig,
) -> Result<PersistedManagedConfig, RootsError> {
    for surface in &config.surfaces {
        if !name_is_valid(&surface.name) {
            return Err(RootsError::InvalidManagedConfig(format!(
                "invalid surface name '{}'",
                surface.name
            )));
        }
        if surface.upstream_url.trim().is_empty() {
            return Err(RootsError::InvalidManagedConfig(format!(
                "surface '{}' has an empty upstream URL",
                surface.name
            )));
        }
        if surface.default_ref.trim().is_empty() {
            return Err(RootsError::InvalidManagedConfig(format!(
                "surface '{}' has an empty default ref",
                surface.name
            )));
        }
    }
    for image in &config.images {
        if !name_is_valid(&image.name) {
            return Err(RootsError::InvalidManagedConfig(format!(
                "invalid image name '{}'",
                image.name
            )));
        }
        if image.image.trim().is_empty() {
            return Err(RootsError::InvalidManagedConfig(format!(
                "image '{}' has an empty reference",
                image.name
            )));
        }
    }

    let credentials = credentials_dir(workspaces_dir);
    std::fs::create_dir_all(&credentials)
        .map_err(|err| RootsError::ManagedPersist(format!("create credentials dir: {err}")))?;

    let mut roots = Vec::with_capacity(config.surfaces.len());
    let mut kept_files = Vec::new();
    for surface in &config.surfaces {
        let credential = match &surface.credential {
            None => None,
            Some(ManagedCredential::SshKey { private_key }) => {
                let file = credentials.join(format!("{}.sshkey", surface.name));
                write_secret_file(&file, private_key)?;
                kept_files.push(file.clone());
                Some(RootCredential::SshKeyPath {
                    ssh_key_path: file.to_string_lossy().into_owned(),
                })
            }
            Some(ManagedCredential::HttpsToken { token }) => {
                let file = credentials.join(format!("{}.token", surface.name));
                write_secret_file(&file, token)?;
                kept_files.push(file.clone());
                Some(RootCredential::TokenPath {
                    token_path: file.to_string_lossy().into_owned(),
                })
            }
        };
        roots.push(SyncableRoot {
            name: surface.name.clone(),
            path: None,
            upstream: Some(GitUpstream {
                url: surface.upstream_url.clone(),
                default_ref: surface.default_ref.clone(),
                credential,
            }),
            default_image: surface.default_image.clone(),
            allowed_outbound_hosts: surface.allowed_outbound_hosts.as_slice().to_vec(),
        });
    }

    let images: Vec<CatalogImageSpec> = config
        .images
        .iter()
        .map(|image| CatalogImageSpec {
            name: image.name.clone(),
            image: image.image.clone(),
            description: image.description.clone(),
            default: image.default,
        })
        .collect();

    let persisted = PersistedManagedConfig {
        version: config.version.clone(),
        roots,
        images,
    };
    let serialized = serde_json::to_string_pretty(&persisted)
        .map_err(|err| RootsError::ManagedPersist(format!("serialize config: {err}")))?;
    write_atomic(&config_path(workspaces_dir), serialized.as_bytes(), 0o644)?;

    // Drop credential files for surfaces no longer in the set.
    if let Ok(entries) = std::fs::read_dir(&credentials) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && !kept_files.contains(&path) {
                if let Err(err) = std::fs::remove_file(&path) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "failed to remove stale credential file"
                    );
                }
            }
        }
    }

    Ok(persisted)
}

fn write_secret_file(path: &Path, value: &str) -> Result<(), RootsError> {
    let mut contents = value.trim_end().to_string();
    contents.push('\n');
    write_atomic(path, contents.as_bytes(), 0o600)
}

/// Write via a same-directory temp file + rename so readers never observe a
/// partial file, with the final mode set before any content is written.
fn write_atomic(path: &Path, contents: &[u8], mode: u32) -> Result<(), RootsError> {
    let dir = path
        .parent()
        .ok_or_else(|| RootsError::ManagedPersist(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir)
        .map_err(|err| RootsError::ManagedPersist(format!("create {}: {err}", dir.display())))?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string()),
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(mode);
        }
        #[cfg(not(unix))]
        let _ = mode;
        let mut file = options.open(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    result.map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        RootsError::ManagedPersist(format!("write {}: {err}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AllowedOutboundHosts, ManagedImage, ManagedSurface};

    fn tempdir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("den-sbx-managed-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn surface(name: &str, credential: Option<ManagedCredential>) -> ManagedSurface {
        ManagedSurface {
            name: name.to_string(),
            upstream_url: "https://example.invalid/repo.git".to_string(),
            default_ref: "main".to_string(),
            default_image: Some("base".to_string()),
            allowed_outbound_hosts: AllowedOutboundHosts::default(),
            credential,
            github_app: None,
        }
    }

    fn config(surfaces: Vec<ManagedSurface>) -> ManagedConfig {
        ManagedConfig {
            surfaces,
            images: vec![ManagedImage {
                name: "base".to_string(),
                image: "bears/sandbox:latest".to_string(),
                description: None,
                default: true,
            }],
            version: Some("v-test".to_string()),
        }
    }

    #[test]
    fn apply_then_load_roundtrips_without_secret_material() {
        let dir = tempdir();
        let cfg = config(vec![surface(
            "site",
            Some(ManagedCredential::HttpsToken {
                token: "sekrit".to_string(),
            }),
        )]);
        let persisted = apply(&dir, &cfg).unwrap();
        assert_eq!(persisted.version.as_deref(), Some("v-test"));
        assert_eq!(persisted.roots.len(), 1);

        // The token value is on disk 0600 but not in config.json.
        let raw = std::fs::read_to_string(config_path(&dir)).unwrap();
        assert!(!raw.contains("sekrit"), "secret leaked into config.json");
        let token_file = credentials_dir(&dir).join("site.token");
        assert_eq!(std::fs::read_to_string(&token_file).unwrap(), "sekrit\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&token_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "credential file mode");
        }

        let loaded = load(&dir).unwrap().expect("persisted config");
        assert_eq!(loaded.version.as_deref(), Some("v-test"));
        assert_eq!(loaded.roots.len(), 1);
        let upstream = loaded.roots[0].upstream.as_ref().unwrap();
        assert!(matches!(
            upstream.credential,
            Some(RootCredential::TokenPath { .. })
        ));
        assert_eq!(loaded.images.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removed_surfaces_lose_their_credential_files() {
        let dir = tempdir();
        apply(
            &dir,
            &config(vec![
                surface(
                    "keep",
                    Some(ManagedCredential::SshKey {
                        private_key: "KEY".to_string(),
                    }),
                ),
                surface(
                    "drop",
                    Some(ManagedCredential::HttpsToken {
                        token: "tok".to_string(),
                    }),
                ),
            ]),
        )
        .unwrap();
        assert!(credentials_dir(&dir).join("drop.token").is_file());

        apply(
            &dir,
            &config(vec![surface(
                "keep",
                Some(ManagedCredential::SshKey {
                    private_key: "KEY".to_string(),
                }),
            )]),
        )
        .unwrap();
        assert!(!credentials_dir(&dir).join("drop.token").exists());
        assert!(credentials_dir(&dir).join("keep.sshkey").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_path_hostile_and_empty_values() {
        let dir = tempdir();
        for bad in ["../evil", "a/b", "", "UPPER"] {
            let err = apply(&dir, &config(vec![surface(bad, None)])).unwrap_err();
            assert!(
                matches!(err, RootsError::InvalidManagedConfig(_)),
                "{bad:?}: {err}"
            );
        }
        let mut empty_url = surface("ok", None);
        empty_url.upstream_url = "  ".to_string();
        assert!(apply(&dir, &config(vec![empty_url])).is_err());

        let mut cfg = config(vec![]);
        cfg.images[0].name = "bad/name".to_string();
        assert!(apply(&dir, &cfg).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_none_when_nothing_pushed() {
        let dir = tempdir();
        assert!(load(&dir).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
