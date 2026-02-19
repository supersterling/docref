//! CLI entry point for docref.
//!
//! Parses command-line arguments and dispatches to the appropriate command handler.

/// Command implementations for each CLI subcommand.
mod commands;
/// Configuration loading and namespace resolution.
mod config;
/// User-facing diagnostic formatting.
mod diagnostics;
/// Unified error type for the crate.
mod error;
/// Freshness checking logic for locked references.
mod freshness;
/// Tree-sitter grammar loading and symbol extraction.
mod grammar;
/// Content hashing for reference targets.
mod hasher;
/// Info command output generation.
mod info;
/// Lockfile serialization and deserialization.
mod lockfile;
/// Namespace mapping management.
mod namespace;
/// Symbol resolution from source files.
mod resolver;
/// Markdown scanning and reference extraction.
mod scanner;
/// Core domain types for references and symbols.
mod types;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

// ── Help text constants ───────────────────────────────────────────────

/// After-help text displayed for the top-level CLI.
const AFTER_HELP: &str = "\
Workflow:
  1. Write [text](file#symbol) or [text](file) references in markdown
  2. docref init                       # Generate .docref.lock
  3. docref check                      # Verify freshness (CI gate)
  4. docref update <file#symbol>       # Accept intentional changes
  5. docref update <file>              # Accept whole-file changes

Exit codes (check):  0=fresh  1=stale  2=broken  3=error

Run `docref info` for the full reference.";

/// After-help text for the `check` subcommand.
const CHECK_HELP: &str = "\
Exit codes:
  0  All references fresh
  1  Stale references (code changed)
  2  Broken references (symbol/file missing)

Examples:
  docref check                      # Verify all references
  docref check && echo 'Fresh'      # CI gate pattern

Supports both [text](file#symbol) and [text](file) whole-file references.";

/// After-help text for the `fix` subcommand.
const FIX_HELP: &str = "\
Auto-corrects references where the symbol name is a close match
(e.g., missing generic parameters). Rewrites markdown in-place.

Modes:
  docref fix                                    # Auto-fix all (closest match)
  docref fix src/lib.rs#old_symbol new_symbol   # Replace with a specific symbol

Examples:
  docref fix                                         # Fix all broken references
  docref fix src/lib.rs#RingBuffer.new 'RingBuffer<T>.new'
  docref init || docref fix                          # Init, fix if broken";

/// After-help text for the `info` subcommand.
const INFO_HELP: &str = "\
Examples:
  docref info                       # Full markdown reference
  docref info --json                # Structured JSON output";

/// After-help text for the `init` subcommand.
const INIT_HELP: &str = "\
Without .docref.toml, ALL markdown files from the project root are scanned
(including node_modules, vendor, etc). Create a config with include patterns first.

Examples:
  docref init                       # Scan and generate lockfile
  docref init && docref check       # Init then verify";

/// After-help text for the `resolve` subcommand.
const RESOLVE_HELP: &str = "\
Examples:
  docref resolve src/lib.rs              # List all symbols
  docref resolve src/lib.rs add          # Check if 'add' exists
  docref resolve src/lib.rs Config.validate  # Dot-scoped lookup";

/// After-help text for the `status` subcommand.
const STATUS_HELP: &str = "\
Examples:
  docref status                     # Show all tracked references
  docref status | grep STALE        # Find stale references";

/// After-help text for the `update` subcommand.
const UPDATE_HELP: &str = "\
Modes:
  docref update <file#symbol>       # Re-hash one symbol reference
  docref update <file>              # Re-hash a whole-file reference
  docref update --from <file.md>    # Re-hash all refs from a markdown file
  docref update --all               # Re-hash every lockfile entry

Examples:
  docref update src/lib.rs#add
  docref update src/lib.rs
  docref update --from docs/guide.md
  docref update --all";

// ── CLI definition ────────────────────────────────────────────────────

/// Top-level CLI structure parsed by clap.
#[derive(Parser)]
#[command(name = "docref", version, about = "Semantic code references for markdown")]
#[command(subcommand_required = true, after_help = AFTER_HELP)]
struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    command: Commands,
}

/// Available CLI subcommands.
#[derive(Subcommand)]
enum Commands {
    /// Verify all references are still fresh
    #[command(after_help = CHECK_HELP)]
    Check,
    /// Auto-fix broken references when a close match exists
    #[command(after_help = FIX_HELP)]
    Fix {
        /// Broken reference in `file#symbol` format (e.g., `src/lib.rs#old_name`)
        reference: Option<String>,
        /// Replacement symbol name (required when reference is specified)
        symbol: Option<String>,
    },
    /// Show the full docref reference document
    #[command(after_help = INFO_HELP)]
    Info {
        /// Output as JSON instead of markdown
        #[arg(long)]
        json: bool,
    },
    /// Scan markdown files and generate .docref.lock
    #[command(after_help = INIT_HELP)]
    Init,
    /// Manage namespace mappings
    Namespace {
        /// The namespace action to perform.
        #[command(subcommand)]
        action: NamespaceAction,
    },
    /// List addressable symbols in a file, or resolve a specific symbol
    #[command(after_help = RESOLVE_HELP)]
    Resolve {
        /// Path to the source file
        file: String,
        /// Optional symbol name to resolve
        symbol: Option<String>,
    },
    /// Show all tracked references and their current freshness
    #[command(after_help = STATUS_HELP)]
    Status,
    /// Re-hash a stale reference so check passes again
    #[command(after_help = UPDATE_HELP)]
    Update {
        /// Re-hash every entry in the lockfile
        #[arg(long)]
        all: bool,
        /// Update all references originating from this markdown file
        #[arg(long, conflicts_with = "all")]
        from: Option<String>,
        /// Reference in file#symbol format (e.g., src/lib.rs#add)
        #[arg(conflicts_with_all = ["from", "all"])]
        reference: Option<String>,
    },
}

/// Actions available under the `namespace` subcommand.
#[derive(Subcommand)]
enum NamespaceAction {
    /// Add a namespace mapping
    Add {
        /// Namespace name
        name: String,
        /// Directory path (relative to config root)
        path: String,
    },
    /// List all configured namespaces
    List,
    /// Remove a namespace mapping
    Remove {
        /// Force removal even if references exist
        #[arg(long)]
        force: bool,
        /// Namespace name to remove
        name: String,
    },
    /// Rename a namespace (updates config, lockfile, and markdown files)
    Rename {
        /// New namespace name
        #[arg(index = 2)]
        new: String,
        /// Current namespace name
        #[arg(index = 1)]
        old: String,
    },
}

/// Route the `fix` subcommand to the right handler.
///
/// # Errors
///
/// Returns errors from the underlying fix operation.
fn dispatch_fix(
    reference: Option<String>,
    symbol: Option<String>,
) -> Result<ExitCode, error::Error> {
    return match (reference, symbol) {
        (None, None) => commands::fix().map(|()| return ExitCode::SUCCESS),
        (Some(r), Some(s)) => commands::fix_targeted(&r, &s).map(|()| return ExitCode::SUCCESS),
        _ => {
            eprintln!("error: provide both a file#symbol reference and a replacement symbol, or neither");
            Ok(ExitCode::FAILURE)
        },
    };
}

/// Route the `namespace` subcommand to the right handler.
///
/// # Errors
///
/// Returns errors from the underlying namespace operation.
fn dispatch_namespace(action: NamespaceAction) -> Result<ExitCode, error::Error> {
    return match action {
        NamespaceAction::Add { name, path } => {
            namespace::cmd_add(&name, &path).map(|()| return ExitCode::SUCCESS)
        },
        NamespaceAction::List => namespace::cmd_list().map(|()| return ExitCode::SUCCESS),
        NamespaceAction::Remove { name, force } => {
            namespace::cmd_remove(&name, force).map(|()| return ExitCode::SUCCESS)
        },
        NamespaceAction::Rename { old, new } => {
            namespace::cmd_rename(&old, &new).map(|()| return ExitCode::SUCCESS)
        },
    };
}

/// Route the `update` subcommand to the right handler.
///
/// # Errors
///
/// Returns errors from the underlying update operation.
fn dispatch_update(
    reference: Option<String>,
    from: Option<String>,
    all: bool,
) -> Result<ExitCode, error::Error> {
    if all {
        return commands::update_all().map(|()| return ExitCode::SUCCESS);
    }
    return match (reference, from) {
        (Some(r), None) => commands::update(&r).map(|()| return ExitCode::SUCCESS),
        (None, Some(f)) => commands::update_file(&f).map(|()| return ExitCode::SUCCESS),
        _ => {
            eprintln!("error: provide a file#symbol reference, --from, or --all");
            Ok(ExitCode::FAILURE)
        },
    };
}

/// Entry point that parses CLI arguments and dispatches to command handlers.
fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Check => commands::check(),
        Commands::Fix { reference, symbol } => dispatch_fix(reference, symbol),
        Commands::Info { json } => {
            commands::info(json);
            Ok(ExitCode::SUCCESS)
        },
        Commands::Init => commands::init().map(|()| return ExitCode::SUCCESS),
        Commands::Namespace { action } => dispatch_namespace(action),
        Commands::Resolve { file, symbol } => {
            commands::resolve(&file, symbol.as_deref()).map(|()| return ExitCode::SUCCESS)
        },
        Commands::Status => commands::status().map(|()| return ExitCode::SUCCESS),
        Commands::Update { reference, from, all } => dispatch_update(reference, from, all),
    };

    return match result {
        Ok(code) => code,
        Err(e) => {
            diagnostics::print_error(&e);
            ExitCode::from(3_u8)
        },
    };
}
