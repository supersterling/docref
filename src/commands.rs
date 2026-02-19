//! Core CLI commands for docref: init, check, status, resolve, update, fix.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::config;
use crate::diagnostics;
use crate::error;
use crate::freshness::{
    CheckResult, compare_lockfile_entry_against_source, parse_symbol_query,
    resolve_and_hash_all_references,
};
use crate::grammar;
use crate::hasher;
use crate::lockfile::Lockfile;
use crate::resolver;
use crate::scanner;
use crate::types::Reference;

/// A pending rewrite: replace a symbol fragment in a markdown file.
struct FixAction {
    /// The markdown file to rewrite.
    file: PathBuf,
    /// The 1-based line number where the symbol appears.
    line: u32,
    /// The symbol name to replace in the reference.
    new_symbol: String,
    /// The original broken symbol name.
    old_symbol: String,
}

/// Apply fix actions by rewriting markdown files.
///
/// # Errors
///
/// Returns `Error::Io` if any markdown file cannot be read or written.
fn apply_fixes(fixes: &[FixAction]) -> Result<(), error::Error> {
    // Group fixes by file so each file is read/written once.
    let mut by_file: HashMap<PathBuf, Vec<&FixAction>> = HashMap::new();
    for fix in fixes {
        by_file.entry(fix.file.clone()).or_default().push(fix);
    }

    for (path, file_fixes) in &by_file {
        let content = std::fs::read_to_string(path)?;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();

        for fix in file_fixes {
            rewrite_symbol_on_line(&mut lines, fix);
        }

        let mut output = lines.join("\n");
        if content.ends_with('\n') {
            output.push('\n');
        }
        std::fs::write(path, output)?;
    }

    return Ok(());
}

/// Read lockfile, re-resolve and re-hash each entry, compare.
///
/// # Errors
///
/// Returns errors from lockfile reading or hash computation.
pub fn check() -> Result<ExitCode, error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let config = config::Config::load(&root)?;
    let lockfile = Lockfile::read(&lock_path)?;
    let mut stale_refs: Vec<String> = Vec::new();
    let mut broken_count = 0_u32;

    for entry in &lockfile.entries {
        match compare_lockfile_entry_against_source(&root, &config, entry)? {
            CheckResult::Broken(reason) => {
                broken_count = broken_count.saturating_add(1);
                println!(
                    "BROKEN  {}#{} ({reason})",
                    entry.target.display(),
                    entry.symbol
                );
            },
            CheckResult::Fresh => {},
            CheckResult::Stale => {
                let refstr = format!("{}#{}", entry.target.display(), entry.symbol);
                println!("STALE   {refstr}");
                stale_refs.push(refstr);
            },
        }
    }

    let stale_count: u32 = stale_refs.len().try_into().unwrap_or(u32::MAX);

    // Exit code priority: broken (2) > stale (1) > fresh (0).
    if broken_count > 0 {
        println!();
        println!("{broken_count} broken, {stale_count} stale");
        return Ok(ExitCode::from(2));
    } else if !stale_refs.is_empty() {
        println!();
        println!("{stale_count} stale");
        print_update_hints(&stale_refs);
        return Ok(ExitCode::from(1));
    } else {
        let total = lockfile.entries.len();
        println!("All {total} references fresh");
        return Ok(ExitCode::SUCCESS);
    }
}

/// Sort a broken reference into fixable (close match found) or unfixable.
fn classify_broken_ref(
    reference: &Reference,
    symbol: &str,
    suggestions: &[String],
    fixes: &mut Vec<FixAction>,
    unfixable: &mut Vec<String>,
) {
    let location = format!("{}:{}", reference.source.display(), reference.source_line);
    match diagnostics::find_closest_suggestion(symbol, suggestions) {
        None => unfixable.push(format!("{location}  #{symbol}")),
        Some(suggestion) => {
            eprintln!("fix: {location}  #{symbol} -> #{suggestion}");
            fixes.push(FixAction {
                file: reference.source.clone(),
                line: reference.source_line,
                new_symbol: suggestion,
                old_symbol: symbol.to_string(),
            });
        },
    }
    return;
}

/// Try resolving each reference in a target group, collecting fixable and unfixable entries.
///
/// # Errors
///
/// Returns resolution errors other than `SymbolNotFound` (which are classified, not propagated).
fn collect_fixes_for_target(
    root: &std::path::Path,
    config: &config::Config,
    target: &std::path::Path,
    refs: &[Reference],
    fixes: &mut Vec<FixAction>,
    unfixable: &mut Vec<String>,
) -> Result<(), error::Error> {
    let disk_path = config.resolve_target(target)?;
    let target_path = root.join(&disk_path);
    let Ok(source) = std::fs::read_to_string(&target_path) else {
        unfixable.push(format!("{}  (file not found)", target.display()));
        return Ok(());
    };

    let Ok(language) = grammar::language_for_path(&disk_path) else {
        unfixable.push(format!("{}  (unsupported language)", target.display()));
        return Ok(());
    };

    for reference in refs {
        match resolver::resolve(&disk_path, &source, &language, &reference.symbol) {
            Err(error::Error::SymbolNotFound { symbol, suggestions, .. }) => {
                classify_broken_ref(reference, &symbol, &suggestions, fixes, unfixable);
            },
            Err(e) => return Err(e),
            Ok(_) => {},
        }
    }

    return Ok(());
}

/// Scan markdown, find broken references, auto-fix those with a close match.
/// Outputs a markdown report of what was fixed and what couldn't be.
///
/// # Errors
///
/// Returns errors from scanning, config loading, or file I/O.
pub fn fix() -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let config = config::Config::load(&root)?;
    let grouped = scanner::scan(&root, &config)?;

    let mut fixes: Vec<FixAction> = Vec::new();
    let mut unfixable: Vec<String> = Vec::new();

    for (target, refs) in &grouped {
        collect_fixes_for_target(&root, &config, target, refs, &mut fixes, &mut unfixable)?;
    }

    if fixes.is_empty() && unfixable.is_empty() {
        eprintln!("All references valid, nothing to fix.");
        return Ok(());
    }

    if !fixes.is_empty() {
        apply_fixes(&fixes)?;
    }

    print_fix_report(&fixes, &unfixable);
    return Ok(());
}

/// Fix a specific broken reference with a user-chosen symbol.
///
/// Validates that `new_symbol` exists in the target file before rewriting.
///
/// # Errors
///
/// Returns errors from scanning, resolution, or file I/O.
pub fn fix_targeted(reference: &str, new_symbol: &str) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let (target_file, old_symbol) = split_reference(reference)?;

    let config = config::Config::load(&root)?;

    // Validate the new symbol exists in the target.
    let disk_path = config.resolve_target(&target_file)?;
    let source = std::fs::read_to_string(root.join(&disk_path))
        .map_err(|_err| return error::Error::FileNotFound { path: disk_path.clone() })?;
    let language = grammar::language_for_path(&disk_path)?;
    let query = parse_symbol_query(new_symbol);
    resolver::resolve(&disk_path, &source, &language, &query)?;

    // Scan markdown to find all references using the old symbol.
    let grouped = scanner::scan(&root, &config)?;
    let Some(refs) = grouped.get(&target_file) else {
        eprintln!("No references to `{}` found in markdown.", target_file.display());
        return Ok(());
    };

    let fixes: Vec<FixAction> = refs
        .iter()
        .filter(|r| return r.symbol.display_name() == old_symbol)
        .map(|r| {
            return FixAction {
                file: r.source.clone(),
                line: r.source_line,
                new_symbol: new_symbol.to_string(),
                old_symbol: old_symbol.clone(),
            };
        })
        .collect();

    if fixes.is_empty() {
        eprintln!("No references to `{}#{old_symbol}` found in markdown.", target_file.display());
        return Ok(());
    }

    apply_fixes(&fixes)?;
    print_fix_report(&fixes, &[]);
    return Ok(());
}

/// Group entry indices by their target file path.
///
/// # Errors
///
/// Returns `Error::LockfileCorrupt` if any index is out of bounds.
fn group_indices_by_target(
    lockfile: &Lockfile,
    indices: &[usize],
) -> Result<HashMap<PathBuf, Vec<usize>>, error::Error> {
    let mut by_target: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for &idx in indices {
        let Some(entry) = lockfile.entries.get(idx) else {
            return Err(error::Error::LockfileCorrupt {
                reason: format!("index {idx} out of bounds"),
            });
        };
        let target = entry.target.clone();
        by_target.entry(target).or_default().push(idx);
    }
    return Ok(by_target);
}

/// Output a comprehensive reference document for docref.
pub fn info(json: bool) {
    return crate::info::run(json);
}

/// Scan markdown, resolve all references, hash symbols, write lockfile.
///
/// # Errors
///
/// Returns errors from scanning, resolution, hashing, or lockfile writing.
pub fn init() -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let config = config::Config::load(&root)?;
    let grouped = scanner::scan(&root, &config)?;
    let entries = resolve_and_hash_all_references(&root, &config, &grouped)?;
    let lockfile = Lockfile::new(entries);

    lockfile.write(&lock_path)?;
    let count = lockfile.entries.len();
    eprintln!("Wrote {count} references to .docref.lock");

    return Ok(());
}

/// Print a markdown summary of fix results.
fn print_fix_report(fixes: &[FixAction], unfixable: &[String]) {
    if !fixes.is_empty() {
        eprintln!("## Fixed\n");
        for fix in fixes {
            eprintln!(
                "- {}:{}  `#{}` -> `#{}`",
                fix.file.display(), fix.line, fix.old_symbol, fix.new_symbol,
            );
        }
        eprintln!();
    }

    if !unfixable.is_empty() {
        eprintln!("## Unfixable\n");
        for msg in unfixable {
            eprintln!("- {msg}");
        }
        eprintln!();
    }

    if !fixes.is_empty() {
        eprintln!("Run `docref init` to regenerate the lockfile.");
    }
    return;
}

/// Print recovery hints to stderr showing exact update commands.
fn print_update_hints(stale_refs: &[String]) {
    eprintln!();
    eprintln!("hint: run `docref update <ref>` to accept changes:");
    for r in stale_refs {
        eprintln!("  docref update {r}");
    }
    return;
}

/// Re-hash entries at given indices against a single parsed target file.
///
/// # Errors
///
/// Returns errors from resolution or hashing.
fn rehash_entries_for_target(
    lockfile: &mut Lockfile,
    indices: &[usize],
    disk_path: &std::path::Path,
    source: &str,
    language: &tree_sitter::Language,
) -> Result<(), error::Error> {
    for &idx in indices {
        let Some(entry) = lockfile.entries.get(idx) else {
            return Err(error::Error::LockfileCorrupt {
                reason: format!("index {idx} out of bounds"),
            });
        };
        let symbol = entry.symbol.clone();
        let query = parse_symbol_query(&symbol);
        let resolved = resolver::resolve(disk_path, source, language, &query)?;
        let new_hash = hasher::hash_symbol(source, language, &resolved)?;
        let Some(entry_mut) = lockfile.entries.get_mut(idx) else {
            return Err(error::Error::LockfileCorrupt {
                reason: format!("index {idx} out of bounds"),
            });
        };
        entry_mut.hash = new_hash;
    }
    return Ok(());
}

/// List all symbols in a file, or resolve a specific symbol to its reference path.
///
/// # Errors
///
/// Returns errors from file reading, language detection, or resolution.
pub fn resolve(file: &str, symbol: Option<&str>) -> Result<(), error::Error> {
    let file_path = PathBuf::from(file);
    let source = std::fs::read_to_string(&file_path)
        .map_err(|_err| return error::Error::FileNotFound { path: file_path.clone() })?;
    let language = grammar::language_for_path(&file_path)?;

    match symbol {
        None => {
            let symbols = resolver::list_symbols(&file_path, &source, &language)?;
            for sym in &symbols {
                println!("{file}#{}", sym.name);
            }
        },
        Some(name) => {
            let query = parse_symbol_query(name);
            resolver::resolve(&file_path, &source, &language, &query)?;
            println!("{file}#{name}");
        },
    }

    return Ok(());
}

/// Replace a symbol fragment on a specific line.
fn rewrite_symbol_on_line(lines: &mut [String], fix: &FixAction) {
    let idx = usize::try_from(fix.line).unwrap_or(0).saturating_sub(1);
    let Some(line) = lines.get_mut(idx) else { return };
    let old_fragment = format!("#{}", fix.old_symbol);
    let new_fragment = format!("#{}", fix.new_symbol);
    *line = line.replace(&old_fragment, &new_fragment);
    return;
}

/// Parse a `file#symbol` string into its components.
///
/// # Errors
///
/// Returns `Error::ParseFailed` if the string doesn't contain `#`.
fn split_reference(input: &str) -> Result<(PathBuf, String), error::Error> {
    let Some((file, symbol)) = input.split_once('#') else {
        return Err(error::Error::ParseFailed {
            file: PathBuf::from(input),
            reason: "expected file#symbol format".to_string(),
        });
    };
    return Ok((PathBuf::from(file), symbol.to_string()));
}

/// Show all tracked references and their current freshness. Always exits 0.
///
/// # Errors
///
/// Returns errors from lockfile reading or hash computation.
pub fn status() -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let config = config::Config::load(&root)?;
    let lockfile = Lockfile::read(&lock_path)?;

    for entry in &lockfile.entries {
        let label = match compare_lockfile_entry_against_source(&root, &config, entry)? {
            CheckResult::Broken(reason) => {
                println!(
                    "BROKEN  {}#{} ({reason})",
                    entry.target.display(),
                    entry.symbol
                );
                continue;
            },
            CheckResult::Fresh => "FRESH ",
            CheckResult::Stale => "STALE ",
        };
        println!("{label}  {}#{}", entry.target.display(), entry.symbol);
    }

    return Ok(());
}

/// Re-hash a specific reference and update the lockfile.
///
/// # Errors
///
/// Returns errors from lockfile I/O, resolution, or hashing.
pub fn update(reference: &str) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let config = config::Config::load(&root)?;
    let (file, symbol) = split_reference(reference)?;
    let mut lockfile = Lockfile::read(&lock_path)?;

    let disk_path = config.resolve_target(&file)?;
    let source = std::fs::read_to_string(root.join(&disk_path))
        .map_err(|_err| return error::Error::FileNotFound { path: disk_path.clone() })?;
    let language = grammar::language_for_path(&disk_path)?;
    let query = parse_symbol_query(&symbol);
    let resolved = resolver::resolve(&disk_path, &source, &language, &query)?;
    let new_hash = hasher::hash_symbol(&source, &language, &resolved)?;

    let mut updated = false;
    for entry in &mut lockfile.entries {
        if entry.target == file && entry.symbol == symbol {
            entry.hash = new_hash.clone();
            updated = true;
        }
    }

    if !updated {
        return Err(error::Error::SymbolNotFound {
            file,
            referenced_from: vec![],
            suggestions: vec![],
            symbol,
        });
    }

    lockfile.write(&lock_path)?;
    eprintln!("Updated {}#{symbol}", file.display());

    return Ok(());
}

/// Re-hash every lockfile entry. Semantically equivalent to `init` but
/// preserves intent: "I know the code changed, update everything."
///
/// # Errors
///
/// Returns errors from lockfile I/O, resolution, or hashing.
pub fn update_all() -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let config = config::Config::load(&root)?;
    let mut lockfile = Lockfile::read(&lock_path)?;

    for entry in &mut lockfile.entries {
        let disk_path = config.resolve_target(&entry.target)?;
        let source = std::fs::read_to_string(root.join(&disk_path))
            .map_err(|_err| return error::Error::FileNotFound { path: disk_path.clone() })?;
        let language = grammar::language_for_path(&disk_path)?;
        let query = parse_symbol_query(&entry.symbol);
        let resolved = resolver::resolve(&disk_path, &source, &language, &query)?;
        entry.hash = hasher::hash_symbol(&source, &language, &resolved)?;
    }

    lockfile.write(&lock_path)?;
    let count = lockfile.entries.len();
    eprintln!("Updated {count} references");

    return Ok(());
}

/// Re-hash all references originating from a specific markdown source file.
/// Groups entries by target file so each target is parsed once.
///
/// # Errors
///
/// Returns errors from lockfile I/O, resolution, or hashing.
pub fn update_file(source_file: &str) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");
    let source_path = PathBuf::from(source_file);

    let config = config::Config::load(&root)?;
    let mut lockfile = Lockfile::read(&lock_path)?;

    let matching_indices: Vec<usize> = lockfile
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| return e.source == source_path)
        .map(|(i, _)| return i)
        .collect();

    if matching_indices.is_empty() {
        return Err(error::Error::FileNotFound {
            path: source_path,
        });
    }

    let by_target = group_indices_by_target(&lockfile, &matching_indices)?;

    for (target, indices) in &by_target {
        let disk_path = config.resolve_target(target)?;
        let target_path = root.join(&disk_path);
        let source = std::fs::read_to_string(&target_path)
            .map_err(|_err| return error::Error::FileNotFound { path: target_path })?;
        let language = grammar::language_for_path(&disk_path)?;
        rehash_entries_for_target(&mut lockfile, indices, &disk_path, &source, &language)?;
    }

    lockfile.write(&lock_path)?;
    let count = matching_indices.len();
    eprintln!("Updated {count} references from {source_file}");

    return Ok(());
}
