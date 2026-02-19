use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config;
use crate::error;
use crate::grammar;
use crate::hasher;
use crate::lockfile::LockEntry;
use crate::resolver;
use crate::types::{Reference, SourceRef, SymbolQuery};

/// Result of checking a single lockfile entry.
pub enum CheckResult {
    Fresh,
    Stale,
    Broken(&'static str),
}

/// Check one lockfile entry against the current source.
///
/// # Errors
///
/// Returns errors from resolution or hashing that aren't recoverable as broken/stale.
pub fn compare_lockfile_entry_against_source(
    root: &Path,
    config: &config::Config,
    entry: &LockEntry,
) -> Result<CheckResult, error::Error> {
    let Ok(disk_path) = config.resolve_target(&entry.target) else {
        return Ok(CheckResult::Broken("unknown namespace"));
    };
    let target_path = root.join(&disk_path);
    let Ok(source) = std::fs::read_to_string(&target_path) else {
        return Ok(CheckResult::Broken("file not found"));
    };

    let Ok(language) = grammar::language_for_path(&disk_path) else {
        return Ok(CheckResult::Broken("unsupported language"));
    };

    let query = parse_symbol_query(&entry.symbol);
    let resolved = match resolver::resolve(&disk_path, &source, &language, &query) {
        Ok(r) => r,
        Err(error::Error::SymbolNotFound { .. }) => {
            return Ok(CheckResult::Broken("symbol removed"));
        },
        Err(e) => return Err(e),
    };

    let new_hash = hasher::hash_symbol(&source, &language, &resolved)?;
    if new_hash == entry.hash {
        Ok(CheckResult::Fresh)
    } else {
        Ok(CheckResult::Stale)
    }
}

/// Resolve all references and produce lockfile entries.
/// Groups are already keyed by target file, so each file is parsed once.
///
/// # Errors
///
/// Returns errors from file reading, language detection, resolution, or hashing.
pub fn resolve_and_hash_all_references(
    root: &Path,
    config: &config::Config,
    grouped: &HashMap<PathBuf, Vec<Reference>>,
) -> Result<Vec<LockEntry>, error::Error> {
    let mut entries = Vec::new();

    for (target, refs) in grouped {
        let disk_path = config.resolve_target(target)?;
        let target_path = root.join(&disk_path);
        let source =
            std::fs::read_to_string(&target_path).map_err(|_| error::Error::FileNotFound {
                path: target_path.clone(),
            })?;

        let language = grammar::language_for_path(&disk_path)?;

        for reference in refs {
            let resolved = resolver::resolve(&disk_path, &source, &language, &reference.symbol)
                .map_err(|e| enrich_with_source_locations(e, refs))?;
            let hash = hasher::hash_symbol(&source, &language, &resolved)?;

            entries.push(LockEntry {
                source: reference.source.clone(),
                target: reference.target.clone(),
                symbol: reference.symbol.display_name(),
                hash,
            });
        }
    }

    Ok(entries)
}

/// Enrich a `SymbolNotFound` error with the markdown locations that reference the broken symbol.
fn enrich_with_source_locations(e: error::Error, refs: &[Reference]) -> error::Error {
    let error::Error::SymbolNotFound { file, symbol, suggestions, .. } = e else {
        return e;
    };
    let sources = refs.iter()
        .filter(|r| r.symbol.display_name() == symbol)
        .map(|r| SourceRef {
            file: r.source.clone(),
            line: r.source_line,
            content: read_line_from_file(&r.source, r.source_line),
        })
        .collect();
    error::Error::SymbolNotFound { file, symbol, suggestions, referenced_from: sources }
}

/// Read a single line from a file. Returns empty string on any failure.
fn read_line_from_file(path: &Path, line: u32) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let idx = (line as usize).saturating_sub(1);
    content.lines().nth(idx).unwrap_or("").trim().to_string()
}

/// Parse a symbol string into bare or dot-scoped form.
pub fn parse_symbol_query(symbol: &str) -> SymbolQuery {
    if let Some((parent, child)) = symbol.split_once('.') {
        SymbolQuery::Scoped {
            parent: parent.to_string(),
            child: child.to_string(),
        }
    } else {
        SymbolQuery::Bare(symbol.to_string())
    }
}
