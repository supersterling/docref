mod commands;
mod config;
mod diagnostics;
mod error;
mod freshness;
mod grammar;
mod hasher;
mod info;
mod lockfile;
mod namespace;
mod resolver;
mod scanner;
mod types;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

// ── Help text constants ───────────────────────────────────────────────

const AFTER_HELP: &str = "\
Workflow:
  1. Write [text](file#symbol) references in markdown
  2. docref init                       Generate .docref.lock
  3. docref check                      Verify freshness (CI gate)
  4. docref update <file#symbol>       Accept intentional changes

Exit codes (check):  0=fresh  1=stale  2=broken  3=error

Run `docref info` for the full reference.";

const INIT_HELP: &str = "\
Examples:
  docref init                       Scan and generate lockfile
  docref init && docref check       Init then verify";

const CHECK_HELP: &str = "\
Exit codes:
  0  All references fresh
  1  Stale references (code changed)
  2  Broken references (symbol/file missing)

Examples:
  docref check                      Verify all references
  docref check && echo 'Fresh'      CI gate pattern";

const UPDATE_HELP: &str = "\
Modes:
  docref update <file#symbol>       Re-hash one reference
  docref update --from <file.md>    Re-hash all refs from a markdown file
  docref update --all               Re-hash every lockfile entry

Examples:
  docref update src/lib.rs#add
  docref update --from docs/guide.md
  docref update --all";

const FIX_HELP: &str = "\
Auto-corrects references where the symbol name is a close match
(e.g., missing generic parameters). Rewrites markdown in-place.

Modes:
  docref fix                                    Auto-fix all (closest match)
  docref fix src/lib.rs#old_symbol new_symbol   Replace with a specific symbol

Examples:
  docref fix                                         Fix all broken references
  docref fix src/lib.rs#RingBuffer.new 'RingBuffer<T>.new'
  docref init || docref fix                          Init, fix if broken";

const RESOLVE_HELP: &str = "\
Examples:
  docref resolve src/lib.rs              List all symbols
  docref resolve src/lib.rs add          Check if 'add' exists
  docref resolve src/lib.rs Config.validate  Dot-scoped lookup";

const STATUS_HELP: &str = "\
Examples:
  docref status                     Show all tracked references
  docref status | grep STALE        Find stale references";

const INFO_HELP: &str = "\
Examples:
  docref info                       Full markdown reference
  docref info --json                Structured JSON output";

// ── CLI definition ────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "docref", version, about = "Semantic code references for markdown")]
#[command(subcommand_required = true, after_help = AFTER_HELP)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan markdown files and generate .docref.lock
    #[command(after_help = INIT_HELP)]
    Init,
    /// Verify all references are still fresh
    #[command(after_help = CHECK_HELP)]
    Check,
    /// Re-hash a stale reference so check passes again
    #[command(after_help = UPDATE_HELP)]
    Update {
        /// Reference in file#symbol format (e.g., src/lib.rs#add)
        #[arg(conflicts_with_all = ["from", "all"])]
        reference: Option<String>,
        /// Update all references originating from this markdown file
        #[arg(long, conflicts_with = "all")]
        from: Option<String>,
        /// Re-hash every entry in the lockfile
        #[arg(long)]
        all: bool,
    },
    /// List addressable symbols in a file, or resolve a specific symbol
    #[command(after_help = RESOLVE_HELP)]
    Resolve {
        /// Path to the source file
        file: String,
        /// Optional symbol name to resolve
        symbol: Option<String>,
    },
    /// Auto-fix broken references when a close match exists
    #[command(after_help = FIX_HELP)]
    Fix {
        /// Broken reference in `file#symbol` format (e.g., `src/lib.rs#old_name`)
        reference: Option<String>,
        /// Replacement symbol name (required when reference is specified)
        symbol: Option<String>,
    },
    /// Show all tracked references and their current freshness
    #[command(after_help = STATUS_HELP)]
    Status,
    /// Show the full docref reference document
    #[command(after_help = INFO_HELP)]
    Info {
        /// Output as JSON instead of markdown
        #[arg(long)]
        json: bool,
    },
    /// Manage namespace mappings
    Namespace {
        #[command(subcommand)]
        action: NamespaceAction,
    },
}

#[derive(Subcommand)]
enum NamespaceAction {
    /// List all configured namespaces
    List,
    /// Add a namespace mapping
    Add {
        /// Namespace name
        name: String,
        /// Directory path (relative to config root)
        path: String,
    },
    /// Rename a namespace (updates config, lockfile, and markdown files)
    Rename {
        /// Current namespace name
        old: String,
        /// New namespace name
        new: String,
    },
    /// Remove a namespace mapping
    Remove {
        /// Namespace name to remove
        name: String,
        /// Force removal even if references exist
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init => commands::init().map(|()| ExitCode::SUCCESS),
        Commands::Check => commands::check(),
        Commands::Update { reference, from, all } => dispatch_update(reference, from, all),
        Commands::Resolve { file, symbol } => {
            commands::resolve(&file, symbol.as_deref()).map(|()| ExitCode::SUCCESS)
        },
        Commands::Fix { reference, symbol } => dispatch_fix(reference, symbol),
        Commands::Status => commands::status().map(|()| ExitCode::SUCCESS),
        Commands::Info { json } => {
            commands::info(json);
            Ok(ExitCode::SUCCESS)
        },
        Commands::Namespace { action } => dispatch_namespace(action),
    };

    match result {
        Ok(code) => code,
        Err(e) => {
            diagnostics::print_error(&e);
            ExitCode::from(3)
        },
    }
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
        commands::update_all().map(|()| ExitCode::SUCCESS)
    } else {
        match (reference, from) {
            (Some(r), None) => commands::update(&r).map(|()| ExitCode::SUCCESS),
            (None, Some(f)) => commands::update_file(&f).map(|()| ExitCode::SUCCESS),
            _ => {
                eprintln!("error: provide a file#symbol reference, --from, or --all");
                Ok(ExitCode::FAILURE)
            },
        }
    }
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
    match (reference, symbol) {
        (None, None) => commands::fix().map(|()| ExitCode::SUCCESS),
        (Some(r), Some(s)) => commands::fix_targeted(&r, &s).map(|()| ExitCode::SUCCESS),
        _ => {
            eprintln!("error: provide both a file#symbol reference and a replacement symbol, or neither");
            Ok(ExitCode::FAILURE)
        },
    }
}

/// Route the `namespace` subcommand to the right handler.
///
/// # Errors
///
/// Returns errors from the underlying namespace operation.
fn dispatch_namespace(action: NamespaceAction) -> Result<ExitCode, error::Error> {
    match action {
        NamespaceAction::List => namespace::cmd_list().map(|()| ExitCode::SUCCESS),
        NamespaceAction::Add { name, path } => {
            namespace::cmd_add(&name, &path).map(|()| ExitCode::SUCCESS)
        },
        NamespaceAction::Rename { old, new } => {
            namespace::cmd_rename(&old, &new).map(|()| ExitCode::SUCCESS)
        },
        NamespaceAction::Remove { name, force } => {
            namespace::cmd_remove(&name, force).map(|()| ExitCode::SUCCESS)
        },
    }
}
