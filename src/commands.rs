//! Core CLI commands for docref: init, check, status, resolve, update, fix, refs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use serde::Serialize;

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

/// JSON output for a single check entry.
#[derive(Serialize)]
struct CheckEntryJson {
    /// Optional reason for broken status.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// The markdown file containing the reference.
    source: PathBuf,
    /// Freshness status: "fresh", "stale", or "broken".
    status: String,
    /// The symbol name (empty for whole-file refs).
    symbol: String,
    /// The target source file.
    target: PathBuf,
}

/// JSON output for the check command.
#[derive(Serialize)]
struct CheckJson {
    /// All tracked entries with their statuses.
    entries: Vec<CheckEntryJson>,
    /// Summary counts.
    summary: CheckSummaryJson,
}

/// Summary counts for the check command JSON output.
#[derive(Serialize)]
struct CheckSummaryJson {
    /// Number of broken references.
    broken: u32,
    /// Number of fresh references.
    fresh: u32,
    /// Number of stale references.
    stale: u32,
}

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

/// Output format for commands that support structured output.
enum OutputFormat {
    /// JSON output for machine consumption.
    Json,
    /// Human-readable text (default).
    Text,
}

/// JSON output for a single status entry.
#[derive(Serialize)]
struct StatusEntryJson {
    /// The stored hash.
    hash: String,
    /// Optional reason for broken status.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// The markdown file containing the reference.
    source: PathBuf,
    /// Freshness status.
    status: String,
    /// The symbol name.
    symbol: String,
    /// The target source file.
    target: PathBuf,
}

/// JSON output for the status command.
#[derive(Serialize)]
struct StatusJson {
    /// All tracked entries with their statuses and hashes.
    entries: Vec<StatusEntryJson>,
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
pub fn check(format: &str) -> Result<ExitCode, error::Error> {
    let output_format = parse_output_format(format)?;
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");
    let config = config::Config::load(&root)?;
    let lockfile = Lockfile::read(&lock_path)?;

    return match output_format {
        OutputFormat::Json => check_json(&root, &config, &lockfile),
        OutputFormat::Text => check_text(&root, &config, &lockfile),
    };
}

/// Produce JSON check output and determine exit code.
///
/// # Errors
///
/// Returns errors from hash computation.
fn check_json(
    root: &std::path::Path,
    config: &config::Config,
    lockfile: &Lockfile,
) -> Result<ExitCode, error::Error> {
    let mut entries: Vec<CheckEntryJson> = Vec::new();
    let mut summary = CheckSummaryJson { broken: 0, fresh: 0, stale: 0 };

    for entry in &lockfile.entries {
        let (status, reason) = match compare_lockfile_entry_against_source(root, config, entry)? {
            CheckResult::Broken(r) => {
                summary.broken = summary.broken.saturating_add(1);
                ("broken", Some(r.to_string()))
            },
            CheckResult::Fresh => {
                summary.fresh = summary.fresh.saturating_add(1);
                ("fresh", None)
            },
            CheckResult::Stale => {
                summary.stale = summary.stale.saturating_add(1);
                ("stale", None)
            },
        };
        entries.push(CheckEntryJson {
            reason,
            source: entry.source.clone(),
            status: status.to_string(),
            symbol: entry.symbol.clone(),
            target: entry.target.clone(),
        });
    }

    let broken = summary.broken;
    let stale = summary.stale;
    let output = CheckJson { entries, summary };
    println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());

    if broken > 0 {
        return Ok(ExitCode::from(2));
    } else if stale > 0 {
        return Ok(ExitCode::from(1));
    }
    return Ok(ExitCode::SUCCESS);
}

/// Produce human-readable text check output and determine exit code.
///
/// # Errors
///
/// Returns errors from hash computation.
fn check_text(
    root: &std::path::Path,
    config: &config::Config,
    lockfile: &Lockfile,
) -> Result<ExitCode, error::Error> {
    let mut stale_refs: Vec<String> = Vec::new();
    let mut broken_count = 0_u32;

    for entry in &lockfile.entries {
        let refstr = format_ref(&entry.target, &entry.symbol);
        match compare_lockfile_entry_against_source(root, config, entry)? {
            CheckResult::Broken(reason) => {
                broken_count = broken_count.saturating_add(1);
                println!("BROKEN  {refstr} ({reason})");
            },
            CheckResult::Fresh => {},
            CheckResult::Stale => {
                println!("STALE   {refstr}");
                stale_refs.push(refstr);
            },
        }
    }

    let stale_count: u32 = stale_refs.len().try_into().unwrap_or(u32::MAX);
    if broken_count > 0 {
        eprintln!();
        eprintln!("{broken_count} broken, {stale_count} stale");
        return Ok(ExitCode::from(2));
    } else if !stale_refs.is_empty() {
        eprintln!();
        eprintln!("# Stale References");
        eprintln!();
        eprintln!("{stale_count} references have changed since the docs were written:");
        eprintln!();
        for r in &stale_refs {
            eprintln!("- `{r}`");
        }
        eprintln!();
        print_update_hints(&stale_refs);
        return Ok(ExitCode::from(1));
    }
    let total = lockfile.entries.len();
    eprintln!("All {total} references fresh");
    return Ok(ExitCode::SUCCESS);
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
        if matches!(reference.symbol, crate::types::SymbolQuery::WholeFile) {
            continue;
        }
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
    let (target_file, old_symbol) = split_reference(reference);

    if old_symbol.is_empty() {
        eprintln!("Whole-file references don't have symbols to fix.");
        return Ok(());
    }

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

/// Format a reference as `file#symbol` or just `file` for whole-file refs.
fn format_ref(target: &std::path::Path, symbol: &str) -> String {
    if symbol.is_empty() {
        return target.display().to_string();
    }
    return format!("{}#{symbol}", target.display());
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

/// Parse a format string into an `OutputFormat`.
///
/// # Errors
///
/// Returns `Error::LockfileCorrupt` (reused as generic user error) for unknown formats.
fn parse_output_format(s: &str) -> Result<OutputFormat, error::Error> {
    return match s {
        "json" => Ok(OutputFormat::Json),
        "text" => Ok(OutputFormat::Text),
        _ => Err(error::Error::LockfileCorrupt {
            reason: format!("unknown format: {s} (expected 'text' or 'json')"),
        }),
    };
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
    eprintln!("## Warning");
    eprintln!();
    eprintln!("Stale means the source code changed since the docs were written.");
    eprintln!("Before updating, check the markdown that references each target");
    eprintln!("to ensure the documentation still accurately describes the code.");
    eprintln!("Running `docref update` accepts the new code as-is — if the docs");
    eprintln!("are now wrong, update the markdown first, then run the command.");
    eprintln!();
    eprintln!("## Fix");
    eprintln!();
    eprintln!("Run `docref update <ref>` to accept changes:");
    eprintln!();
    for r in stale_refs {
        eprintln!("    docref update {r}");
    }
    return;
}

/// Show which markdown files reference a given target file or symbol.
///
/// # Errors
///
/// Returns errors from lockfile reading.
pub fn refs(reference: &str) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let lockfile = Lockfile::read(&lock_path)?;
    let (file, symbol) = split_reference(reference);

    let mut found = false;
    for entry in &lockfile.entries {
        if entry.target != file {
            continue;
        }
        if !symbol.is_empty() && entry.symbol != symbol {
            continue;
        }
        let refstr = format_ref(&entry.target, &entry.symbol);
        println!("{} -> {refstr}", entry.source.display());
        found = true;
    }

    if !found {
        let refstr = format_ref(&file, &symbol);
        eprintln!("No references to `{refstr}` found in lockfile.");
    }

    return Ok(());
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
        let new_hash = if symbol.is_empty() {
            hasher::hash_file(source, language)?
        } else {
            let query = parse_symbol_query(&symbol);
            let resolved = resolver::resolve(disk_path, source, language, &query)?;
            hasher::hash_symbol(source, language, &resolved)?
        };
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

/// Parse a `file#symbol` or bare `file` string into its components.
///
/// Returns an empty symbol string for bare file references.
fn split_reference(input: &str) -> (PathBuf, String) {
    return match input.split_once('#') {
        Some((file, symbol)) => (PathBuf::from(file), symbol.to_string()),
        None => (PathBuf::from(input), String::new()),
    };
}

/// Show all tracked references and their current freshness. Always exits 0.
///
/// # Errors
///
/// Returns errors from lockfile reading or hash computation.
pub fn status(format: &str) -> Result<(), error::Error> {
    let output_format = parse_output_format(format)?;
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");
    let config = config::Config::load(&root)?;
    let lockfile = Lockfile::read(&lock_path)?;

    return match output_format {
        OutputFormat::Json => status_json(&root, &config, &lockfile),
        OutputFormat::Text => status_text(&root, &config, &lockfile),
    };
}

/// Produce JSON status output.
///
/// # Errors
///
/// Returns errors from hash computation.
fn status_json(
    root: &std::path::Path,
    config: &config::Config,
    lockfile: &Lockfile,
) -> Result<(), error::Error> {
    let mut entries: Vec<StatusEntryJson> = Vec::new();

    for entry in &lockfile.entries {
        let result = compare_lockfile_entry_against_source(root, config, entry)?;
        let (status_str, reason) = match result {
            CheckResult::Broken(r) => ("broken", Some(r.to_string())),
            CheckResult::Fresh => ("fresh", None),
            CheckResult::Stale => ("stale", None),
        };
        entries.push(StatusEntryJson {
            hash: entry.hash.0.clone(),
            reason,
            source: entry.source.clone(),
            status: status_str.to_string(),
            symbol: entry.symbol.clone(),
            target: entry.target.clone(),
        });
    }

    let output = StatusJson { entries };
    println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
    return Ok(());
}

/// Produce human-readable text status output.
///
/// # Errors
///
/// Returns errors from hash computation.
fn status_text(
    root: &std::path::Path,
    config: &config::Config,
    lockfile: &Lockfile,
) -> Result<(), error::Error> {
    for entry in &lockfile.entries {
        let refstr = format_ref(&entry.target, &entry.symbol);
        let result = compare_lockfile_entry_against_source(root, config, entry)?;
        let label = match result {
            CheckResult::Broken(reason) => {
                println!("BROKEN  {refstr} ({reason})");
                continue;
            },
            CheckResult::Fresh => "FRESH ",
            CheckResult::Stale => "STALE ",
        };
        println!("{label}  {refstr}");
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
    let (file, symbol) = split_reference(reference);
    let mut lockfile = Lockfile::read(&lock_path)?;

    let disk_path = config.resolve_target(&file)?;
    let source = std::fs::read_to_string(root.join(&disk_path))
        .map_err(|_err| return error::Error::FileNotFound { path: disk_path.clone() })?;
    let language = grammar::language_for_path(&disk_path)?;

    let new_hash = if symbol.is_empty() {
        hasher::hash_file(&source, &language)?
    } else {
        let query = parse_symbol_query(&symbol);
        let resolved = resolver::resolve(&disk_path, &source, &language, &query)?;
        hasher::hash_symbol(&source, &language, &resolved)?
    };

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
    eprintln!("Updated {}", format_ref(&file, &symbol));

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
        entry.hash = if entry.symbol.is_empty() {
            hasher::hash_file(&source, &language)?
        } else {
            let query = parse_symbol_query(&entry.symbol);
            let resolved = resolver::resolve(&disk_path, &source, &language, &query)?;
            hasher::hash_symbol(&source, &language, &resolved)?
        };
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
