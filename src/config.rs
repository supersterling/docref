use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// A namespace mapping from a config file, binding a short prefix
/// to a relative directory path and the config root that defined it.
pub struct NamespaceEntry {
    pub path: String,
    pub config_root: PathBuf,
}

/// Project configuration loaded from `.docref.toml`.
/// Include/exclude patterns are path prefixes applied to markdown source files.
/// Namespaces map short prefixes to directory paths for cross-project references.
pub struct Config {
    include: Vec<String>,
    exclude: Vec<String>,
    pub namespaces: HashMap<String, NamespaceEntry>,
}

/// Raw TOML structure for `.docref.toml`.
#[derive(serde::Deserialize)]
struct DocrefTomlConfig {
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    namespaces: HashMap<String, String>,
}

impl Config {
    /// Load config from `.docref.toml` in the given root directory.
    /// Returns a default that scans everything if the file doesn't exist.
    /// Returns an error if the file exists but is malformed -- never silently
    /// falls back to defaults when the user wrote a config file.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if reading fails (other than not-found),
    /// or `Error::TomlDe` if the TOML is malformed.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let path = root.join(".docref.toml");
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::scan_everything_by_default());
            }
            Err(e) => return Err(Error::Io(e)),
        };

        let raw: DocrefTomlConfig = toml::from_str(&content)?;
        let namespaces = raw
            .namespaces
            .into_iter()
            .map(|(name, path)| {
                let entry = NamespaceEntry {
                    path,
                    config_root: root.to_path_buf(),
                };
                (name, entry)
            })
            .collect();

        Ok(Self {
            include: raw.include,
            exclude: raw.exclude,
            namespaces,
        })
    }

    /// Default config that includes everything and excludes nothing.
    fn scan_everything_by_default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            namespaces: HashMap::new(),
        }
    }

    /// Resolve a potentially namespace-prefixed target to a relative path.
    /// Targets like `auth:src/lib.rs` are split on the first `:` and the
    /// namespace prefix is replaced with the mapped directory. Plain paths
    /// pass through unchanged.
    ///
    /// # Errors
    ///
    /// Returns `Error::UnknownNamespace` if the prefix doesn't match any
    /// configured namespace.
    pub fn resolve_target(&self, target: &Path) -> Result<PathBuf, Error> {
        let target_str = target.to_string_lossy();
        let Some((namespace, path)) = target_str.split_once(':') else {
            return Ok(target.to_path_buf());
        };

        let entry = self.namespaces.get(namespace).ok_or_else(|| {
            Error::UnknownNamespace {
                name: namespace.to_string(),
            }
        })?;

        Ok(PathBuf::from(&entry.path).join(path))
    }

    /// Check whether a markdown file path should be scanned.
    ///
    /// A path is included if no include patterns are set (scan everything),
    /// or if the path starts with at least one include pattern.
    /// An included path is then excluded if it starts with any exclude pattern.
    pub fn should_scan(&self, relative_path: &str) -> bool {
        let included = self.include.is_empty()
            || self
                .include
                .iter()
                .any(|p| relative_path.starts_with(p.as_str()));

        if !included {
            return false;
        }

        !self
            .exclude
            .iter()
            .any(|p| relative_path.starts_with(p.as_str()))
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn loads_namespaces_from_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(".docref.toml"),
            r#"
[namespaces]
auth = "services/auth"
shared = "packages/shared"
"#,
        )
        .unwrap();

        let config = Config::load(tmp.path()).unwrap();
        assert_eq!(config.namespaces.len(), 2);
    }

    #[test]
    fn resolve_target_with_namespace() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(".docref.toml"),
            r#"
[namespaces]
auth = "services/auth"
"#,
        )
        .unwrap();

        let config = Config::load(tmp.path()).unwrap();
        let resolved = config
            .resolve_target(&PathBuf::from("auth:src/lib.rs"))
            .unwrap();
        assert_eq!(resolved, PathBuf::from("services/auth/src/lib.rs"));
    }

    #[test]
    fn resolve_target_without_namespace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = Config::load(tmp.path()).unwrap();
        let resolved = config
            .resolve_target(&PathBuf::from("src/lib.rs"))
            .unwrap();
        assert_eq!(resolved, PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn resolve_target_unknown_namespace_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = Config::load(tmp.path()).unwrap();
        let result = config.resolve_target(&PathBuf::from("nope:src/lib.rs"));
        assert!(result.is_err());
    }
}
