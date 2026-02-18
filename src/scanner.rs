use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::{Captures, Regex};
use walkdir::WalkDir;

use crate::config::Config;
use crate::error::Error;
use crate::types::{Reference, SymbolQuery};

/// Scan all markdown files under `root` and extract references.
/// Applies the config's include/exclude filters to control which markdown files are scanned.
/// Returns references grouped by target file path for batch resolution.
///
/// # Errors
///
/// Returns `Error::Io` if any markdown file cannot be read.
///
/// # Panics
///
/// Panics if the hardcoded reference regex is invalid (compile-time invariant).
pub fn scan(root: &Path, config: &Config) -> Result<HashMap<PathBuf, Vec<Reference>>, Error> {
    let pattern = Regex::new(r"\[([^\]]+)\]\(([^)#]+)#([^)]+)\)").expect("valid regex");
    let mut grouped: HashMap<PathBuf, Vec<Reference>> = HashMap::new();

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
    {
        let md_path = entry.path();
        let relative_source = md_path.strip_prefix(root).unwrap_or(md_path).to_path_buf();

        let relative_str = relative_source.to_string_lossy();
        if !config.should_scan(&relative_str) {
            continue;
        }

        let content = std::fs::read_to_string(md_path)?;
        extract_refs_from_content(&content, &relative_source, &pattern, &mut grouped);
    }

    Ok(grouped)
}

/// Extract all `[text](path#symbol)` references from markdown content.
fn extract_refs_from_content(
    content: &str,
    source: &Path,
    pattern: &Regex,
    grouped: &mut HashMap<PathBuf, Vec<Reference>>,
) {
    for line in content.lines() {
        extract_refs_from_line(line, source, pattern, grouped);
    }
}

/// Extract references from a single markdown line.
fn extract_refs_from_line(
    line: &str,
    source: &Path,
    pattern: &Regex,
    grouped: &mut HashMap<PathBuf, Vec<Reference>>,
) {
    for cap in pattern.captures_iter(line) {
        let Some(reference) = parse_capture(&cap, source) else {
            continue;
        };
        let target = reference.target.clone();
        grouped.entry(target).or_default().push(reference);
    }
}

/// Try to parse a regex capture into a local code reference.
/// Returns `None` for external URLs or empty fragments.
fn parse_capture(cap: &Captures<'_>, source: &Path) -> Option<Reference> {
    let raw_target = &cap[2];
    let raw_symbol = &cap[3];

    if raw_target.starts_with("http://")
        || raw_target.starts_with("https://")
        || raw_target.is_empty()
        || raw_symbol.is_empty()
    {
        return None;
    }

    let target = PathBuf::from(raw_target);
    let symbol = parse_symbol_query(raw_symbol);

    Some(Reference {
        source: source.to_path_buf(),
        target,
        symbol,
    })
}

/// Parse a symbol fragment into bare or dot-scoped form.
fn parse_symbol_query(raw: &str) -> SymbolQuery {
    if let Some((parent, child)) = raw.split_once('.') {
        SymbolQuery::Scoped {
            parent: parent.to_string(),
            child: child.to_string(),
        }
    } else {
        SymbolQuery::Bare(raw.to_string())
    }
}
