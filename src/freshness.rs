//! Freshness checking and batch resolution for lockfile entries.

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
    /// The target file, language, or symbol could not be resolved.
    Broken(&'static str),
    /// The entry hash matches the current source — no changes.
    Fresh,
    /// The entry hash differs from the current source — symbol body changed.
    Stale,
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

    let new_hash = if entry.symbol.is_empty() {
        hasher::hash_file(&source, &language)?
    } else {
        let query = parse_symbol_query(&entry.symbol);
        let resolved = match resolver::resolve(&disk_path, &source, &language, &query) {
            Err(error::Error::SymbolNotFound { .. }) => {
                return Ok(CheckResult::Broken("symbol removed"));
            },
            Err(e) => return Err(e),
            Ok(r) => r,
        };
        hasher::hash_symbol(&source, &language, &resolved)?
    };

    if new_hash == entry.hash {
        return Ok(CheckResult::Fresh);
    } else {
        return Ok(CheckResult::Stale);
    }
}

/// Enrich a `SymbolNotFound` error with the markdown locations that reference the broken symbol.
fn enrich_with_source_locations(e: error::Error, refs: &[Reference]) -> error::Error {
    let error::Error::SymbolNotFound { file, symbol, suggestions, .. } = e else {
        return e;
    };
    let sources = refs.iter()
        .filter(|r| return r.symbol.display_name() == symbol)
        .map(|r| {
            return SourceRef {
                content: read_line_from_file(&r.source, r.source_line),
                file: r.source.clone(),
                line: r.source_line,
            };
        })
        .collect();
    return error::Error::SymbolNotFound { file, referenced_from: sources, suggestions, symbol };
}

/// Hash a single reference — whole-file or symbol-scoped.
///
/// # Errors
///
/// Returns resolution or hashing errors.
fn hash_reference(
    disk_path: &std::path::Path,
    source: &str,
    language: &tree_sitter::Language,
    reference: &Reference,
) -> Result<crate::types::SemanticHash, error::Error> {
    if matches!(reference.symbol, SymbolQuery::WholeFile) {
        return hasher::hash_file(source, language);
    }
    let resolved = resolver::resolve(disk_path, source, language, &reference.symbol)?;
    return hasher::hash_symbol(source, language, &resolved);
}

/// Parse a symbol string into bare, dot-scoped, or whole-file form.
pub fn parse_symbol_query(symbol: &str) -> SymbolQuery {
    if symbol.is_empty() {
        return SymbolQuery::WholeFile;
    }
    return match symbol.split_once('.') {
        None => SymbolQuery::Bare(symbol.to_string()),
        Some((parent, child)) => SymbolQuery::Scoped {
            child: child.to_string(),
            parent: parent.to_string(),
        },
    };
}

/// Read a single line from a file. Returns empty string on any failure.
fn read_line_from_file(path: &Path, line: u32) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let idx = usize::try_from(line).unwrap_or(0).saturating_sub(1);
    return content.lines().nth(idx).unwrap_or("").trim().to_string();
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
            std::fs::read_to_string(&target_path).map_err(|_err| return error::Error::FileNotFound {
                path: target_path.clone(),
            })?;

        let language = grammar::language_for_path(&disk_path)?;

        for reference in refs {
            let hash = hash_reference(&disk_path, &source, &language, reference)
                .map_err(|e| return enrich_with_source_locations(e, refs))?;

            entries.push(LockEntry {
                hash,
                source: reference.source.clone(),
                symbol: reference.symbol.display_name(),
                target: reference.target.clone(),
            });
        }
    }

    return Ok(entries);
}
