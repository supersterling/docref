use std::path::PathBuf;

/// All errors in docref carry enough context to produce a useful diagnostic
/// without a debugger. Each variant names the file, symbol, or reason for failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("symbol not found: `{symbol}` in {}", file.display())]
    SymbolNotFound {
        file: PathBuf,
        symbol: String,
        suggestions: Vec<String>,
    },

    #[error(
        "ambiguous symbol: `{symbol}` in {}, candidates: {}",
        file.display(),
        candidates.join(", ")
    )]
    AmbiguousSymbol {
        file: PathBuf,
        symbol: String,
        candidates: Vec<String>,
    },

    #[error("file not found: {}", path.display())]
    FileNotFound {
        path: PathBuf,
    },

    #[error("no grammar for extension: .{ext}")]
    UnsupportedLanguage {
        ext: String,
    },

    #[error("parse failed: {}: {reason}", file.display())]
    ParseFailed {
        file: PathBuf,
        reason: String,
    },

    #[error("file too large ({size_bytes} bytes, max {max_bytes}): {}", file.display())]
    FileTooLarge {
        file: PathBuf,
        size_bytes: u64,
        max_bytes: u64,
    },

    #[error("unknown namespace: `{name}`")]
    UnknownNamespace {
        name: String,
    },

    #[error("namespace `{name}` is in use by {count} references (use --force to remove)")]
    NamespaceInUse {
        name: String,
        count: usize,
    },

    #[error("config not found: {}", path.display())]
    ConfigNotFound {
        path: PathBuf,
    },

    #[error("config cycle detected: {}", chain.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" -> "))]
    ConfigCycle {
        chain: Vec<PathBuf>,
    },

    #[error("lockfile not found: {}", path.display())]
    LockfileNotFound {
        path: PathBuf,
    },

    #[error("lockfile corrupt: {reason}")]
    LockfileCorrupt {
        reason: String,
    },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml deserialize: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("toml serialize: {0}")]
    TomlSer(#[from] toml::ser::Error),
}
