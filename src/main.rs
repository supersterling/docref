mod error;
mod grammar;
mod hasher;
mod lockfile;
mod resolver;
mod scanner;
mod types;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::lockfile::{LockEntry, Lockfile};
use crate::types::{Reference, SymbolQuery};

#[derive(Parser)]
#[command(name = "docref", about = "Semantic code references for markdown")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan markdown files and generate .docref.lock
    Init,
    /// Verify all references are still fresh
    Check,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => match cmd_init() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            },
        },
        Commands::Check => match cmd_check() {
            Ok(code) => code,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            },
        },
    }
}

/// Scan markdown, resolve all references, hash symbols, write lockfile.
///
/// # Errors
///
/// Returns errors from scanning, resolution, hashing, or lockfile writing.
fn cmd_init() -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let grouped = scanner::scan(&root)?;
    let entries = resolve_and_hash(&root, &grouped)?;
    let lockfile = Lockfile::new(entries);

    lockfile.write(&lock_path)?;
    let count = lockfile.entries.len();
    println!("Wrote {count} references to .docref.lock");

    Ok(())
}

/// Read lockfile, re-resolve and re-hash each entry, compare.
///
/// # Errors
///
/// Returns errors from lockfile reading or hash computation.
fn cmd_check() -> Result<ExitCode, error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let lockfile = Lockfile::read(&lock_path)?;
    let mut stale_count = 0u32;
    let mut broken_count = 0u32;

    for entry in &lockfile.entries {
        match check_entry(&root, entry)? {
            CheckResult::Fresh => {},
            CheckResult::Stale => stale_count += 1,
            CheckResult::Broken(reason) => {
                broken_count += 1;
                println!(
                    "BROKEN  {}#{} ({reason})",
                    entry.target.display(),
                    entry.symbol
                );
            },
        }
    }

    if stale_count > 0 {
        println!();
    }

    // Exit code priority: broken (2) > stale (1) > fresh (0).
    if broken_count > 0 {
        println!("{broken_count} broken, {stale_count} stale");
        Ok(ExitCode::from(2))
    } else if stale_count > 0 {
        println!("{stale_count} stale");
        Ok(ExitCode::from(1))
    } else {
        let total = lockfile.entries.len();
        println!("All {total} references fresh");
        Ok(ExitCode::SUCCESS)
    }
}

/// Result of checking a single lockfile entry.
enum CheckResult {
    Fresh,
    Stale,
    Broken(&'static str),
}

/// Check one lockfile entry against the current source.
///
/// # Errors
///
/// Returns errors from resolution or hashing that aren't recoverable as broken/stale.
fn check_entry(root: &Path, entry: &LockEntry) -> Result<CheckResult, error::Error> {
    let target_path = root.join(&entry.target);
    let Ok(source) = std::fs::read_to_string(&target_path) else {
        return Ok(CheckResult::Broken("file not found"));
    };

    let Ok(language) = grammar::language_for_path(&entry.target) else {
        return Ok(CheckResult::Broken("unsupported language"));
    };

    let query = parse_entry_symbol(&entry.symbol);
    let resolved = match resolver::resolve(&entry.target, &source, &language, &query) {
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
        println!("STALE   {}#{}", entry.target.display(), entry.symbol);
        Ok(CheckResult::Stale)
    }
}

/// Resolve all references and produce lockfile entries.
/// Groups are already keyed by target file, so each file is parsed once.
///
/// # Errors
///
/// Returns errors from file reading, language detection, resolution, or hashing.
fn resolve_and_hash(
    root: &Path,
    grouped: &HashMap<PathBuf, Vec<Reference>>,
) -> Result<Vec<LockEntry>, error::Error> {
    let mut entries = Vec::new();

    for (target, refs) in grouped {
        let target_path = root.join(target);
        let source =
            std::fs::read_to_string(&target_path).map_err(|_| error::Error::FileNotFound {
                path: target_path.clone(),
            })?;

        let language = grammar::language_for_path(target)?;

        for reference in refs {
            let resolved = resolver::resolve(target, &source, &language, &reference.symbol)?;
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

/// Parse a lockfile entry's symbol string back into a query.
fn parse_entry_symbol(symbol: &str) -> SymbolQuery {
    if let Some((parent, child)) = symbol.split_once('.') {
        SymbolQuery::Scoped {
            parent: parent.to_string(),
            child: child.to_string(),
        }
    } else {
        SymbolQuery::Bare(symbol.to_string())
    }
}
