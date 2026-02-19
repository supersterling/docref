/// Core domain types for docref references, symbols, and hashes.
use std::ops::Range;
use std::path::PathBuf;

/// Parsed from markdown link syntax by the scanner.
#[derive(Debug, Clone)]
pub struct Reference {
    /// Markdown file containing this reference.
    pub source: PathBuf,
    /// One-based line number of the reference in the source file.
    pub source_line: u32,
    /// Symbol query parsed from the fragment portion of the link.
    pub symbol: SymbolQuery,
    /// Path to the target source file.
    pub target: PathBuf,
}

/// Output of successful symbol resolution. Byte range is guaranteed
/// within source bounds by construction.
#[derive(Debug, Clone)]
pub struct ResolvedSymbol {
    /// Byte offset range of the symbol in the source file.
    pub byte_range: Range<u32>,
}

/// A semantic hash — 64 hex chars, always lowercase.
/// Newtype prevents mixing with arbitrary strings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SemanticHash(
    /// The hex-encoded SHA-256 digest string.
    pub String,
);

/// Location in a markdown file that references a symbol.
/// Used in error diagnostics to show where a broken reference originated.
#[derive(Debug, Clone)]
pub struct SourceRef {
    /// Raw text content of the reference line.
    pub content: String,
    /// Markdown file containing the reference.
    pub file: PathBuf,
    /// One-based line number in the markdown file.
    pub line: u32,
}

/// Parsed from a symbol fragment. Either bare ("add"), dot-scoped ("Config.validate"),
/// or whole-file (no fragment).
#[derive(Debug, Clone)]
pub enum SymbolQuery {
    /// Unscoped symbol name such as `add`.
    Bare(String),
    /// Dot-scoped symbol such as `Config.validate`.
    Scoped {
        /// Nested member name.
        child: String,
        /// Enclosing type or module name.
        parent: String,
    },
    /// Entire file reference — no symbol fragment.
    WholeFile,
}

impl SymbolQuery {
    /// The display name used in lockfile entries and error messages.
    pub fn display_name(&self) -> String {
        return match self {
            SymbolQuery::Bare(name) => name.clone(),
            SymbolQuery::Scoped {
                parent,
                child,
            } => format!("{parent}.{child}"),
            SymbolQuery::WholeFile => String::new(),
        };
    }
}
