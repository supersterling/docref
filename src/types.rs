use std::ops::Range;
use std::path::PathBuf;

/// Parsed from markdown link syntax by the scanner.
#[derive(Debug, Clone)]
pub struct Reference {
    pub source: PathBuf,
    pub target: PathBuf,
    pub symbol: SymbolQuery,
}

/// Parsed from a symbol fragment. Either bare ("add") or dot-scoped ("Config.validate").
#[derive(Debug, Clone)]
pub enum SymbolQuery {
    Bare(String),
    Scoped {
        parent: String,
        child: String,
    },
}

impl SymbolQuery {
    /// The display name used in lockfile entries and error messages.
    pub fn display_name(&self) -> String {
        match self {
            SymbolQuery::Bare(name) => name.clone(),
            SymbolQuery::Scoped {
                parent,
                child,
            } => format!("{parent}.{child}"),
        }
    }
}

/// Output of successful symbol resolution. Byte range is guaranteed
/// within source bounds by construction.
#[derive(Debug, Clone)]
pub struct ResolvedSymbol {
    pub byte_range: Range<u32>,
}

/// A semantic hash — 64 hex chars, always lowercase.
/// Newtype prevents mixing with arbitrary strings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SemanticHash(pub String);
