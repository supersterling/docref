mod commands;
mod config;
mod error;
mod grammar;
mod hasher;
mod lockfile;
mod namespace;
mod resolver;
mod scanner;
mod types;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
        Commands::Accept { reference, file } => match (reference, file) {
            (Some(r), None) => commands::accept(&r).map(|()| ExitCode::SUCCESS),
            (None, Some(f)) => commands::accept_file(&f).map(|()| ExitCode::SUCCESS),
            _ => {
                eprintln!("error: provide either a file#symbol reference or --file");
                Ok(ExitCode::FAILURE)
            },
        },
        Commands::Resolve { file, symbol } => {
            commands::resolve(&file, symbol.as_deref()).map(|()| ExitCode::SUCCESS)
        },
        Commands::Status => commands::status().map(|()| ExitCode::SUCCESS),
        Commands::Namespace { action } => match action {
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
        },
    };

    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        },
    }
}
