use std::path::Path;

use crate::error::Error;

/// Project configuration loaded from `.docref.toml`.
/// Include/exclude patterns are path prefixes applied to markdown source files.
pub struct Config {
    include: Vec<String>,
    exclude: Vec<String>,
}

/// Raw TOML structure for `.docref.toml`.
#[derive(serde::Deserialize)]
struct DocrefTomlConfig {
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

impl Config {
    /// Load config from `.docref.toml` in the given root directory.
    /// Returns a default that scans everything if the file doesn't exist.
    /// Returns an error if the file exists but is malformed — never silently
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
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::scan_everything_by_default()),
            Err(e) => return Err(Error::Io(e)),
        };

        let raw: DocrefTomlConfig = toml::from_str(&content)?;
        Ok(Self {
            include: raw.include,
            exclude: raw.exclude,
        })
    }

    /// Default config that includes everything and excludes nothing.
    fn scan_everything_by_default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }

    /// Check whether a markdown file path should be scanned.
    ///
    /// A path is included if no include patterns are set (scan everything),
    /// or if the path starts with at least one include pattern.
    /// An included path is then excluded if it starts with any exclude pattern.
    pub fn should_scan(&self, relative_path: &str) -> bool {
        let included = self.include.is_empty()
            || self.include.iter().any(|p| relative_path.starts_with(p.as_str()));

        if !included {
            return false;
        }

        !self.exclude.iter().any(|p| relative_path.starts_with(p.as_str()))
    }
}
