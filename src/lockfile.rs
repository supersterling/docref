//! Lockfile persistence: parsing, serialization, and ordering enforcement.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::types::SemanticHash;

/// A single tracked reference in the lockfile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockEntry {
    /// The semantic hash of the resolved symbol body.
    pub hash: SemanticHash,
    /// The markdown file containing the reference.
    pub source: PathBuf,
    /// The symbol name within the target file.
    pub symbol: String,
    /// The target source file being referenced.
    pub target: PathBuf,
}

impl Ord for LockEntry {
    /// Compare entries by (source, target, symbol) for deterministic ordering.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        return (&self.source, &self.target, &self.symbol).cmp(&(
            &other.source,
            &other.target,
            &other.symbol,
        ));
    }
}

impl PartialOrd for LockEntry {
    /// Delegate to `Ord` implementation.
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        return Some(self.cmp(other));
    }
}

/// The lockfile as a whole. Entries are sorted by (source, target, symbol).
/// Constructed only via `Lockfile::new()` or `Lockfile::parse()`, both of
/// which enforce sorting and uniqueness.
#[derive(Debug, Serialize, Deserialize)]
pub struct Lockfile {
    /// The ordered list of tracked reference entries.
    pub entries: Vec<LockEntry>,
}

impl Lockfile {
    /// Create a new lockfile from unsorted entries. Sorts and deduplicates.
    pub fn new(mut entries: Vec<LockEntry>) -> Self {
        entries.sort();
        entries.dedup();
        return Self { entries };
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
        return Ok(lockfile);
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
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::LockfileNotFound { path: path.to_path_buf() });
            },
            Err(e) => return Err(Error::Io(e)),
            Ok(c) => c,
        };
        return Self::parse(&content);
    }

    /// Serialize to TOML.
    ///
    /// # Errors
    ///
    /// Returns `Error::TomlSer` if serialization fails.
    pub fn serialize(&self) -> Result<String, Error> {
        return Ok(toml::to_string_pretty(self)?);
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
        return Ok(());
    }
}

/// Validate that lockfile entries are strictly sorted.
///
/// # Errors
///
/// Returns `Error::LockfileCorrupt` if any adjacent pair is out of order.
fn enforce_lockfile_entry_ordering(entries: &[LockEntry]) -> Result<(), Error> {
    for window in entries.windows(2) {
        let Some(first) = window.first() else {
            return Err(Error::LockfileCorrupt {
                reason: "window underflow at index 0".to_string(),
            });
        };
        let Some(second) = window.get(1) else {
            return Err(Error::LockfileCorrupt {
                reason: "window underflow at index 1".to_string(),
            });
        };
        if first >= second {
            return Err(Error::LockfileCorrupt {
                reason: format!(
                    "entries not sorted: {} {} {} >= {} {} {}",
                    first.source.display(),
                    first.target.display(),
                    first.symbol,
                    second.source.display(),
                    second.target.display(),
                    second.symbol,
                ),
            });
        }
    }
    return Ok(());
}
