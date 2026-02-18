use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// A namespace mapping from a config file, binding a short prefix
/// to a relative directory path and the config root that defined it.
#[derive(Debug)]
pub struct NamespaceEntry {
    pub path: String,
    pub config_root: PathBuf,
}

/// Project configuration loaded from `.docref.toml`.
/// Include/exclude patterns are path prefixes applied to markdown source files.
/// Namespaces map short prefixes to directory paths for cross-project references.
#[derive(Debug)]
pub struct Config {
    include: Vec<String>,
    exclude: Vec<String>,
    pub namespaces: HashMap<String, NamespaceEntry>,
}

/// Raw TOML structure for `.docref.toml`.
#[derive(serde::Deserialize)]
struct DocrefTomlConfig {
    #[serde(default)]
    extends: Option<String>,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    namespaces: HashMap<String, String>,
}

impl Config {
    /// Load config from `.docref.toml` in the given root directory.
    /// Follows `extends` chains to inherit parent namespaces, detecting cycles.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if reading fails (other than not-found),
    /// `Error::TomlDe` if TOML is malformed, `Error::ConfigCycle` on
    /// circular extends, or `Error::ConfigNotFound` if an extends target
    /// doesn't exist.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let mut chain = Vec::new();
        Self::load_recursive(root, &mut chain)
    }

    /// # Errors
    ///
    /// Propagates IO, TOML, cycle, and not-found errors from the extends chain.
    fn load_recursive(root: &Path, chain: &mut Vec<PathBuf>) -> Result<Self, Error> {
        let raw = Self::read_toml(root)?;
        let Some(raw) = raw else {
            return Ok(Self::scan_everything_by_default());
        };

        let parent_namespaces = Self::load_parent(raw.extends.as_ref(), root, chain)?;
        let namespaces = Self::merge_namespaces(parent_namespaces, raw.namespaces, root);

        Ok(Self {
            include: raw.include,
            exclude: raw.exclude,
            namespaces,
        })
    }

    /// Read and parse `.docref.toml`, returning `None` if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` on read failure or `Error::TomlDe` on parse failure.
    fn read_toml(root: &Path) -> Result<Option<DocrefTomlConfig>, Error> {
        let path = root.join(".docref.toml");
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::Io(e)),
        };
        let raw: DocrefTomlConfig = toml::from_str(&content)?;
        Ok(Some(raw))
    }

    /// If `extends` is set, validate the path, detect cycles, and recursively
    /// load the parent config, returning its namespaces.
    ///
    /// # Errors
    ///
    /// Returns `Error::ConfigNotFound` if the extends target doesn't exist,
    /// `Error::ConfigCycle` if the chain revisits a config, or propagates
    /// errors from the recursive load.
    fn load_parent(
        extends: Option<&String>,
        root: &Path,
        chain: &mut Vec<PathBuf>,
    ) -> Result<HashMap<String, NamespaceEntry>, Error> {
        let Some(extends_rel) = extends else {
            return Ok(HashMap::new());
        };

        let parent_path = root.join(extends_rel);
        if !parent_path.exists() {
            return Err(Error::ConfigNotFound { path: parent_path });
        }

        let canonical = std::fs::canonicalize(&parent_path)?;
        if chain.contains(&canonical) {
            chain.push(canonical);
            return Err(Error::ConfigCycle {
                chain: chain.clone(),
            });
        }
        chain.push(canonical);

        let parent_dir = parent_path
            .parent()
            .ok_or_else(|| Error::ConfigNotFound {
                path: parent_path.clone(),
            })?;
        let parent = Self::load_recursive(parent_dir, chain)?;
        Ok(parent.namespaces)
    }

    /// Merge parent namespaces with child overrides. Child entries win on conflict.
    fn merge_namespaces(
        mut base: HashMap<String, NamespaceEntry>,
        child_raw: HashMap<String, String>,
        child_root: &Path,
    ) -> HashMap<String, NamespaceEntry> {
        for (name, path) in child_raw {
            base.insert(
                name,
                NamespaceEntry {
                    path,
                    config_root: child_root.to_path_buf(),
                },
            );
        }
        base
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

    #[test]
    fn extends_inherits_parent_namespaces() {
        let tmp = tempfile::TempDir::new().unwrap();

        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(".docref.toml"),
            "[namespaces]\nauth = \"services/auth\"\nshared = \"packages/shared\"\n",
        )
        .unwrap();

        let child = tmp.path().join("root/services/web");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join(".docref.toml"),
            "extends = \"../../.docref.toml\"\ninclude = [\"docs/\"]\n",
        )
        .unwrap();

        let config = Config::load(&child).unwrap();
        assert_eq!(config.namespaces.len(), 2);

        let resolved = config
            .resolve_target(&PathBuf::from("auth:src/lib.rs"))
            .unwrap();
        assert_eq!(resolved, PathBuf::from("services/auth/src/lib.rs"));
    }

    #[test]
    fn child_namespace_overrides_parent() {
        let tmp = tempfile::TempDir::new().unwrap();

        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(".docref.toml"),
            "[namespaces]\nauth = \"services/auth-legacy\"\n",
        )
        .unwrap();

        let child = tmp.path().join("root/services/web");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join(".docref.toml"),
            "extends = \"../../.docref.toml\"\n\n[namespaces]\nauth = \"services/auth-v2\"\n",
        )
        .unwrap();

        let config = Config::load(&child).unwrap();
        let resolved = config
            .resolve_target(&PathBuf::from("auth:src/lib.rs"))
            .unwrap();
        assert_eq!(resolved, PathBuf::from("services/auth-v2/src/lib.rs"));
    }

    #[test]
    fn extends_cycle_detected() {
        let tmp = tempfile::TempDir::new().unwrap();

        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        std::fs::write(
            dir_a.join(".docref.toml"),
            "extends = \"../b/.docref.toml\"\n",
        )
        .unwrap();
        std::fs::write(
            dir_b.join(".docref.toml"),
            "extends = \"../a/.docref.toml\"\n",
        )
        .unwrap();

        let result = Config::load(&dir_a);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cycle"), "expected cycle error: {err}");
    }

    #[test]
    fn extends_target_not_found_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(".docref.toml"),
            "extends = \"../nonexistent/.docref.toml\"\n",
        )
        .unwrap();

        let result = Config::load(tmp.path());
        assert!(result.is_err());
    }
}
