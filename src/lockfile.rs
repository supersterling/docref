use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::types::SemanticHash;

/// A single tracked reference in the lockfile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockEntry {
    pub source: PathBuf,
    pub target: PathBuf,
    pub symbol: String,
    pub hash: SemanticHash,
}

impl PartialOrd for LockEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LockEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.source, &self.target, &self.symbol).cmp(&(
            &other.source,
            &other.target,
            &other.symbol,
        ))
    }
}

/// The lockfile as a whole. Entries are sorted by (source, target, symbol).
/// Constructed only via `Lockfile::new()` or `Lockfile::parse()`, both of
/// which enforce sorting and uniqueness.
#[derive(Debug, Serialize, Deserialize)]
pub struct Lockfile {
    pub entries: Vec<LockEntry>,
}

impl Lockfile {
    /// Create a new lockfile from unsorted entries. Sorts and deduplicates.
    pub fn new(mut entries: Vec<LockEntry>) -> Self {
        entries.sort();
        entries.dedup();
        Self { entries }
    }

    /// Parse a lockfile from TOML content.
    ///
    /// # Errors
    ///
    /// Returns `Error::TomlDe` if the content is not valid TOML,
    /// or `Error::LockfileCorrupt` if entries are not sorted.
    pub fn parse(content: &str) -> Result<Self, Error> {
        let lockfile: Self = toml::from_str(content)?;
        enforce_lockfile_entry_ordering(&lockfile.entries)?;
        Ok(lockfile)
    }

    /// Serialize to TOML.
    ///
    /// # Errors
    ///
    /// Returns `Error::TomlSer` if serialization fails.
    pub fn serialize(&self) -> Result<String, Error> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Write the lockfile to disk.
    ///
    /// # Errors
    ///
    /// Returns `Error::TomlSer` if serialization fails,
    /// or `Error::Io` if the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let content = self.serialize()?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Read and parse a lockfile from disk.
    ///
    /// # Errors
    ///
    /// Returns `Error::LockfileNotFound` if the file doesn't exist,
    /// `Error::Io` for other read failures,
    /// `Error::TomlDe` if the content is invalid TOML,
    /// or `Error::LockfileCorrupt` if entries are not sorted.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::LockfileNotFound { path: path.to_path_buf() });
            },
            Err(e) => return Err(Error::Io(e)),
        };
        Self::parse(&content)
    }
}

/// Validate that lockfile entries are strictly sorted.
///
/// # Errors
///
/// Returns `Error::LockfileCorrupt` if any adjacent pair is out of order.
fn enforce_lockfile_entry_ordering(entries: &[LockEntry]) -> Result<(), Error> {
    for window in entries.windows(2) {
        if window[0] >= window[1] {
            return Err(Error::LockfileCorrupt {
                reason: format!(
                    "entries not sorted: {} {} {} >= {} {} {}",
                    window[0].source.display(),
                    window[0].target.display(),
                    window[0].symbol,
                    window[1].source.display(),
                    window[1].target.display(),
                    window[1].symbol,
                ),
            });
        }
    }
    Ok(())
}
