//! The `info` subcommand — outputs a comprehensive docref reference document
//! as either markdown (for humans) or JSON (for tooling).

use std::path::PathBuf;

use serde::Serialize;

use crate::config;
use crate::lockfile::Lockfile;

/// Snapshot of the current project state for rendering.
struct CurrentState {
    /// Whether `.docref.toml` was found.
    config_found: bool,
    /// Number of lockfile entries, or `None` if no lockfile.
    lockfile_entries: Option<usize>,
    /// Sorted list of (name, path) namespace mappings.
    namespaces: Vec<(String, String)>,
}

/// JSON representation of an exit code and its meaning.
#[derive(Serialize)]
struct ExitCodeInfo {
    /// Numeric exit code.
    code: u8,
    /// Human-readable description.
    meaning: String,
}

/// Top-level JSON output structure for `docref info --json`.
#[derive(Serialize)]
struct InfoJson {
    /// Current project state.
    current_state: StateJson,
    /// Exit code definitions.
    exit_codes: Vec<ExitCodeInfo>,
    /// Supported language grammars.
    supported_languages: Vec<LanguageInfo>,
    /// Tool version string.
    version: String,
}

/// JSON representation of a supported language grammar.
#[derive(Serialize)]
struct LanguageInfo {
    /// File extensions triggering this grammar.
    extensions: Vec<String>,
    /// Human-readable language name.
    language: String,
}

/// JSON representation of a namespace mapping.
#[derive(Serialize)]
struct NamespaceJson {
    /// Namespace short name.
    name: String,
    /// Mapped directory path.
    path: String,
}

/// JSON representation of current project state.
#[derive(Serialize)]
struct StateJson {
    /// Whether `.docref.toml` was found.
    config_found: bool,
    /// Number of lockfile entries, or `None` if no lockfile.
    lockfile_entries: Option<usize>,
    /// Configured namespaces.
    namespaces: Vec<NamespaceJson>,
}

/// Collect project state from the given root directory.
fn gather_state(root: &std::path::Path) -> CurrentState {
    let config_path = root.join(".docref.toml");
    let lock_path = root.join(".docref.lock");

    let config_found = config_path.exists();
    let lockfile_entries = Lockfile::read(&lock_path).ok().map(|l| return l.entries.len());

    let namespaces = config::Config::load(root)
        .ok()
        .map(|c| {
            let mut ns: Vec<(String, String)> = c
                .namespaces
                .iter()
                .map(|(name, entry)| return (name.clone(), entry.path.clone()))
                .collect();
            ns.sort();
            return ns;
        })
        .unwrap_or_default();

    return CurrentState { config_found, lockfile_entries, namespaces };
}

/// Serialize and print the info structure as pretty JSON.
fn print_json(state: &CurrentState) {
    let info = InfoJson {
        current_state: StateJson {
            config_found: state.config_found,
            lockfile_entries: state.lockfile_entries,
            namespaces: state
                .namespaces
                .iter()
                .map(|(name, path)| return NamespaceJson {
                    name: name.clone(),
                    path: path.clone(),
                })
                .collect(),
        },
        exit_codes: vec![
            ExitCodeInfo { code: 0, meaning: "Success / all references fresh".to_string() },
            ExitCodeInfo { code: 1, meaning: "Stale references found".to_string() },
            ExitCodeInfo { code: 2, meaning: "Broken references found".to_string() },
            ExitCodeInfo { code: 3, meaning: "Runtime error".to_string() },
        ],
        supported_languages: vec![
            LanguageInfo {
                extensions: vec![".rs".to_string()],
                language: "Rust".to_string(),
            },
            LanguageInfo {
                extensions: vec![".ts".to_string(), ".tsx".to_string()],
                language: "TypeScript".to_string(),
            },
            LanguageInfo {
                extensions: vec![".md".to_string()],
                language: "Markdown".to_string(),
            },
        ],
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    // serde_json::to_string_pretty won't fail on this structure.
    let json = serde_json::to_string_pretty(&info).unwrap_or_default();
    println!("{json}");
    return;
}

/// Print the full markdown reference document to stdout.
fn print_markdown(state: &CurrentState) {
    let version = env!("CARGO_PKG_VERSION");
    print_markdown_header(version);
    print_markdown_state(state);
    println!();
    print_markdown_exit_codes();
    return;
}

/// Print the exit codes table section.
fn print_markdown_exit_codes() {
    print!(
        "\
## Exit Codes

| Code | Meaning |
|------|---------|
| 0    | Success / all references fresh |
| 1    | Stale references found |
| 2    | Broken references found |
| 3    | Runtime error |
"
    );
    return;
}

/// Print the header with syntax, workflow, and configuration sections.
fn print_markdown_header(version: &str) {
    print_markdown_header_intro(version);
    print_markdown_header_reference();
    return;
}

/// Print the title and reference syntax sections.
fn print_markdown_header_intro(version: &str) {
    print!(
        "\
# docref {version}

Semantic code references for markdown — track code symbols referenced in docs
and detect when code changes make references stale.

## Reference Syntax

    [link text](path/to/file.rs#symbol)            local reference
    [link text](path/to/file.rs#Type.method)        dot-scoped symbol
    [link text](namespace:path/to/file.rs#symbol)   namespaced reference

## Workflow

    docref init                       Scan markdown, hash symbols, write .docref.lock
    docref check                      Verify all references (exit 0/1/2)
    docref update <file#symbol>       Re-hash after intentional code changes
    docref update --from <file.md>    Re-hash all refs from a markdown file
    docref update --all               Re-hash everything
    docref status                     Show freshness of all references
    docref resolve <file>             List addressable symbols in a file

"
    );
    return;
}

/// Print the languages, configuration, and current state heading.
fn print_markdown_header_reference() {
    print!(
        "\
## Supported Languages

| Extension | Language   |
|-----------|------------|
| .rs       | Rust       |
| .ts .tsx  | TypeScript |
| .md       | Markdown   |

## Configuration (.docref.toml)

    include = [\"docs/\"]                 # only scan these paths
    exclude = [\"docs/archive/\"]         # skip these paths
    extends = \"../.docref.toml\"         # inherit parent config

    [namespaces]
    auth = \"services/auth\"              # auth:src/lib.rs -> services/auth/src/lib.rs

## Current State

"
    );
    return;
}

/// Print the current project state (config, lockfile, namespaces).
fn print_markdown_state(state: &CurrentState) {
    if state.config_found {
        println!("Config:     .docref.toml (found)");
    } else {
        println!("Config:     .docref.toml (not found)");
    }

    match state.lockfile_entries {
        Some(n) => println!("Lockfile:   .docref.lock ({n} references)"),
        None => println!("Lockfile:   .docref.lock (not found)"),
    }

    if state.namespaces.is_empty() {
        println!("Namespaces: (none)");
    } else {
        let ns_list = state
            .namespaces
            .iter()
            .map(|(name, path)| return format!("{name} -> {path}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("Namespaces: {ns_list}");
    }
    return;
}

/// Output the comprehensive docref reference document.
pub fn run(json: bool) {
    let root = PathBuf::from(".");
    let state = gather_state(&root);

    if json {
        print_json(&state);
    } else {
        print_markdown(&state);
    }
    return;
}
