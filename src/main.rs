mod config;
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
        #[arg(conflicts_with = "file")]
        reference: Option<String>,
        /// Accept all references originating from this markdown file
        #[arg(long)]
        file: Option<String>,
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
        Commands::Accept { reference, file } => match (reference, file) {
            (Some(r), None) => cmd_accept(&r).map(|()| ExitCode::SUCCESS),
            (None, Some(f)) => cmd_accept_file(&f).map(|()| ExitCode::SUCCESS),
            _ => {
                eprintln!("error: provide either a file#symbol reference or --file");
                Ok(ExitCode::FAILURE)
            },
        },
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

    let config = config::Config::load(&root)?;
    let grouped = scanner::scan(&root, &config)?;
    let entries = resolve_and_hash_all_references(&root, &config, &grouped)?;
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

    let config = config::Config::load(&root)?;
    let lockfile = Lockfile::read(&lock_path)?;
    let mut stale_count = 0u32;
    let mut broken_count = 0u32;

    for entry in &lockfile.entries {
        match compare_lockfile_entry_against_source(&root, &config, entry)? {
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

    let config = config::Config::load(&root)?;
    let lockfile = Lockfile::read(&lock_path)?;

    for entry in &lockfile.entries {
        let label = match compare_lockfile_entry_against_source(&root, &config, entry)? {
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
            let query = parse_lockfile_symbol_as_query(name);
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

    let config = config::Config::load(&root)?;
    let (file, symbol) = split_file_hash_symbol_reference(reference)?;
    let mut lockfile = Lockfile::read(&lock_path)?;

    let disk_path = config.resolve_target(&file)?;
    let source = std::fs::read_to_string(root.join(&disk_path))
        .map_err(|_| error::Error::FileNotFound { path: disk_path.clone() })?;
    let language = grammar::language_for_path(&disk_path)?;
    let query = parse_lockfile_symbol_as_query(&symbol);
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
            symbol,
        });
    }

    lockfile.write(&lock_path)?;
    println!("Accepted {}#{symbol}", file.display());

    Ok(())
}

/// Re-hash all references originating from a specific markdown source file.
/// Groups entries by target file so each target is parsed once.
///
/// # Errors
///
/// Returns errors from lockfile I/O, resolution, or hashing.
fn cmd_accept_file(source_file: &str) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");
    let source_path = PathBuf::from(source_file);

    let config = config::Config::load(&root)?;
    let mut lockfile = Lockfile::read(&lock_path)?;

    // Collect indices of entries matching this source file.
    let matching_indices: Vec<usize> = lockfile
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.source == source_path)
        .map(|(i, _)| i)
        .collect();

    if matching_indices.is_empty() {
        return Err(error::Error::FileNotFound {
            path: source_path,
        });
    }

    // Group matching entries by target file for batch resolution.
    let mut by_target: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for &idx in &matching_indices {
        let target = lockfile.entries[idx].target.clone();
        by_target.entry(target).or_default().push(idx);
    }

    // Re-resolve and re-hash each group, parsing each target file once.
    for (target, indices) in &by_target {
        let disk_path = config.resolve_target(target)?;
        let target_path = root.join(&disk_path);
        let source = std::fs::read_to_string(&target_path)
            .map_err(|_| error::Error::FileNotFound { path: target_path })?;
        let language = grammar::language_for_path(&disk_path)?;

        for &idx in indices {
            let symbol = &lockfile.entries[idx].symbol;
            let query = parse_lockfile_symbol_as_query(symbol);
            let resolved = resolver::resolve(&disk_path, &source, &language, &query)?;
            let new_hash = hasher::hash_symbol(&source, &language, &resolved)?;
            lockfile.entries[idx].hash = new_hash;
        }
    }

    lockfile.write(&lock_path)?;
    let count = matching_indices.len();
    println!("Accepted {count} references from {source_file}");

    Ok(())
}

/// Parse a `file#symbol` string into its components.
///
/// # Errors
///
/// Returns `Error::ParseFailed` if the string doesn't contain `#`.
fn split_file_hash_symbol_reference(input: &str) -> Result<(PathBuf, String), error::Error> {
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
fn compare_lockfile_entry_against_source(
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

    let query = parse_lockfile_symbol_as_query(&entry.symbol);
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
fn resolve_and_hash_all_references(
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
            let resolved = resolver::resolve(&disk_path, &source, &language, &reference.symbol)?;
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
fn parse_lockfile_symbol_as_query(symbol: &str) -> SymbolQuery {
    if let Some((parent, child)) = symbol.split_once('.') {
        SymbolQuery::Scoped {
            parent: parent.to_string(),
            child: child.to_string(),
        }
    } else {
        SymbolQuery::Bare(symbol.to_string())
    }
}
