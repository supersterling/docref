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
    /// Re-hash a stale reference so check passes again
    Accept {
        /// Reference in file#symbol format (e.g., src/lib.rs#add)
        reference: String,
    },
    /// List addressable symbols in a file, or resolve a specific symbol
    Resolve {
        /// Path to the source file
        file: String,
        /// Optional symbol name to resolve
        symbol: Option<String>,
    },
    /// Show all tracked references and their current freshness
    Status,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init => cmd_init().map(|()| ExitCode::SUCCESS),
        Commands::Check => cmd_check(),
        Commands::Accept { reference } => cmd_accept(&reference).map(|()| ExitCode::SUCCESS),
        Commands::Resolve { file, symbol } => {
            cmd_resolve(&file, symbol.as_deref()).map(|()| ExitCode::SUCCESS)
        },
        Commands::Status => cmd_status().map(|()| ExitCode::SUCCESS),
    };

    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
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

/// Show all tracked references and their current freshness. Always exits 0.
///
/// # Errors
///
/// Returns errors from lockfile reading or hash computation.
fn cmd_status() -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let lockfile = Lockfile::read(&lock_path)?;

    for entry in &lockfile.entries {
        let label = match check_entry(&root, entry)? {
            CheckResult::Fresh => "FRESH ",
            CheckResult::Stale => "STALE ",
            CheckResult::Broken(reason) => {
                println!(
                    "BROKEN  {}#{} ({reason})",
                    entry.target.display(),
                    entry.symbol
                );
                continue;
            },
        };
        println!("{label}  {}#{}", entry.target.display(), entry.symbol);
    }

    Ok(())
}

/// List all symbols in a file, or resolve a specific symbol to its reference path.
///
/// # Errors
///
/// Returns errors from file reading, language detection, or resolution.
fn cmd_resolve(file: &str, symbol: Option<&str>) -> Result<(), error::Error> {
    let file_path = PathBuf::from(file);
    let source = std::fs::read_to_string(&file_path)
        .map_err(|_| error::Error::FileNotFound { path: file_path.clone() })?;
    let language = grammar::language_for_path(&file_path)?;

    match symbol {
        None => {
            let symbols = resolver::list_symbols(&file_path, &source, &language)?;
            for sym in &symbols {
                println!("{file}#{}", sym.name);
            }
        },
        Some(name) => {
            let query = parse_entry_symbol(name);
            resolver::resolve(&file_path, &source, &language, &query)?;
            println!("{file}#{name}");
        },
    }

    Ok(())
}

/// Re-hash a specific reference and update the lockfile.
///
/// # Errors
///
/// Returns errors from lockfile I/O, resolution, or hashing.
fn cmd_accept(reference: &str) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let (file, symbol) = parse_symbol_ref(reference)?;
    let mut lockfile = Lockfile::read(&lock_path)?;

    let source = std::fs::read_to_string(root.join(&file))
        .map_err(|_| error::Error::FileNotFound { path: file.clone() })?;
    let language = grammar::language_for_path(&file)?;
    let query = parse_entry_symbol(&symbol);
    let resolved = resolver::resolve(&file, &source, &language, &query)?;
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
            symbol,
        });
    }

    lockfile.write(&lock_path)?;
    println!("Accepted {}#{symbol}", file.display());

    Ok(())
}

/// Parse a `file#symbol` string into its components.
///
/// # Errors
///
/// Returns `Error::ParseFailed` if the string doesn't contain `#`.
fn parse_symbol_ref(input: &str) -> Result<(PathBuf, String), error::Error> {
    let Some((file, symbol)) = input.split_once('#') else {
        return Err(error::Error::ParseFailed {
            file: PathBuf::from(input),
            reason: "expected file#symbol format".to_string(),
        });
    };
    Ok((PathBuf::from(file), symbol.to_string()))
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
