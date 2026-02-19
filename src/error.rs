/// Crate-level error types for docref diagnostics.
use std::path::PathBuf;

use crate::types::SourceRef;

/// All errors in docref carry enough context to produce a useful diagnostic
/// without a debugger. Each variant names the file, symbol, or reason for failure.
#[allow(clippy::error_impl_error, reason = "crate-internal error type in binary")]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Two or more symbols matched the query and the user must disambiguate.
    #[error(
        "ambiguous symbol: `{symbol}` in {}, candidates: {}",
        file.display(),
        candidates.join(", ")
    )]
    AmbiguousSymbol {
        /// Candidate symbol names that matched.
        candidates: Vec<String>,
        /// File containing the ambiguous symbol.
        file: PathBuf,
        /// Symbol query string that matched multiple candidates.
        symbol: String,
    },

    /// Config file includes form a cycle.
    #[error("config cycle detected: {}", chain.iter().map(|p| return p.display().to_string()).collect::<Vec<_>>().join(" -> "))]
    ConfigCycle {
        /// Ordered chain of config file paths forming the cycle.
        chain: Vec<PathBuf>,
    },

    /// A referenced config file does not exist on disk.
    #[error("config not found: {}", path.display())]
    ConfigNotFound {
        /// Path to the missing config file.
        path: PathBuf,
    },

    /// A referenced source file does not exist on disk.
    #[error("file not found: {}", path.display())]
    FileNotFound {
        /// Path to the missing file.
        path: PathBuf,
    },

    /// Source file exceeds the configured size limit.
    #[error("file too large ({size_bytes} bytes, max {max_bytes}): {}", file.display())]
    FileTooLarge {
        /// File that exceeded the size limit.
        file: PathBuf,
        /// Maximum allowed file size in bytes.
        max_bytes: u64,
        /// Actual file size in bytes.
        size_bytes: u64,
    },

    /// Underlying I/O error from the filesystem.
    #[error("io: {0}")]
    Io(
        /// The wrapped I/O error.
        #[from]
        std::io::Error,
    ),

    /// Lockfile exists but cannot be parsed.
    #[error("lockfile corrupt: {reason}")]
    LockfileCorrupt {
        /// Description of the corruption.
        reason: String,
    },

    /// Expected lockfile does not exist on disk.
    #[error("lockfile not found: {}", path.display())]
    LockfileNotFound {
        /// Path to the missing lockfile.
        path: PathBuf,
    },

    /// A namespace still has active references and cannot be removed without `--force`.
    #[error("namespace `{name}` is in use by {count} references (use --force to remove)")]
    NamespaceInUse {
        /// Number of references still using this namespace.
        count: usize,
        /// Namespace identifier.
        name: String,
    },

    /// Tree-sitter failed to parse a source file.
    #[error("parse failed: {}: {reason}", file.display())]
    ParseFailed {
        /// File that failed to parse.
        file: PathBuf,
        /// Description of the parse failure.
        reason: String,
    },

    /// A referenced symbol does not exist in the target file.
    #[error("symbol not found: `{symbol}` in {}", file.display())]
    SymbolNotFound {
        /// File that was searched for the symbol.
        file: PathBuf,
        /// Source locations that reference this symbol.
        referenced_from: Vec<SourceRef>,
        /// Similar symbol names found in the file.
        suggestions: Vec<String>,
        /// Symbol name that was not found.
        symbol: String,
    },

    /// TOML deserialization failed.
    #[error("toml deserialize: {0}")]
    TomlDe(
        /// The wrapped TOML deserialization error.
        #[from]
        toml::de::Error,
    ),

    /// TOML serialization failed.
    #[error("toml serialize: {0}")]
    TomlSer(
        /// The wrapped TOML serialization error.
        #[from]
        toml::ser::Error,
    ),

    /// No configured namespace matches the given name.
    #[error("unknown namespace: `{name}`")]
    UnknownNamespace {
        /// Namespace identifier that was not found.
        name: String,
    },

    /// No tree-sitter grammar registered for this file extension.
    #[error("no grammar for extension: .{ext}")]
    UnsupportedLanguage {
        /// File extension without the leading dot.
        ext: String,
    },
}
