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
    print_section_title(version);
    print_section_how_it_works();
    print_section_reference_syntax();
    print_section_path_resolution();
    print_section_setup();
    print_section_commands();
    print_section_configuration();
    print_section_languages();
    print_section_state(state);
    println!();
    print_section_exit_codes();
    return;
}

/// Print title and one-line description.
fn print_section_title(version: &str) {
    print!(
        "\
# docref {version}

Semantic code references for markdown — track code symbols in your docs
and detect when code changes make them stale.

"
    );
    return;
}

/// Print the core mental model for how docref works.
fn print_section_how_it_works() {
    print!(
        "\
## How It Works

Markdown files in your project are the documents docref tracks. Any standard
markdown link pointing to a source file becomes a tracked reference:

    [descriptive text](path/to/file.rs#symbol_name)

docref hashes each referenced symbol's source code. When code changes,
`docref check` detects which references became stale or broken.

"
    );
    return;
}

/// Print reference syntax with all supported forms.
fn print_section_reference_syntax() {
    print!(
        "\
## Reference Syntax

    [text](path/to/file.rs#symbol)           symbol reference
    [text](path/to/file.rs#Type.method)       dot-scoped sub-symbol
    [text](ns:path/to/file.rs#symbol)         namespaced reference
    [text](path/to/file.rs)                   whole-file reference (no #symbol)

Use `docref resolve <file>` to list all addressable symbols in a source file.

"
    );
    return;
}

/// Print path resolution rules — the #1 footgun for new users.
fn print_section_path_resolution() {
    print!(
        "\
## Path Resolution

IMPORTANT: Paths are relative to the markdown file, not the project root.

    # In docs/guide.md, referencing src/config.rs:
    [load](../src/config.rs#load)          CORRECT (relative from docs/)
    [load](src/config.rs#load)             WRONG (resolves to docs/src/config.rs)

For cross-directory references, use namespaces instead of deep relative paths:

    # Fragile — breaks if markdown file moves:
    [load](../../services/auth/src/config.rs#load)

    # Robust — works from any markdown file:
    [load](auth:src/config.rs#load)

Set up namespaces: `docref namespace add <name> <path>`

"
    );
    return;
}

/// Print the recommended setup flow.
fn print_section_setup() {
    print!(
        "\
## Quick Start

    1. Create .docref.toml with include patterns (see Configuration)
    2. Write [text](file#symbol) references in your markdown
    3. docref init       Scan markdown, hash symbols, write .docref.lock
    4. docref check      Verify all references (CI gate)

IMPORTANT: Without .docref.toml, docref scans ALL markdown files from the
project root — including node_modules, vendor, .next, etc. Always create
a config with include patterns before running `docref init`.

"
    );
    return;
}

/// Print all available commands.
fn print_section_commands() {
    print!(
        "\
## Commands

    docref init                          Scan markdown, hash symbols, write .docref.lock
    docref check                         Verify all references (exit 0/1/2)
    docref status                        Show freshness of all tracked references
    docref update <file#symbol>          Re-hash after intentional code changes
    docref update --from <file.md>       Re-hash all refs from a markdown file
    docref update --all                  Re-hash everything
    docref fix                           Auto-fix all broken refs (closest match)
    docref fix <file#sym> <newsym>       Fix a specific broken reference
    docref resolve <file>                List addressable symbols in a source file
    docref namespace add <name> <path>   Map a short name to a directory
    docref namespace list                Show all namespace mappings
    docref namespace remove <name>       Remove a namespace mapping
    docref namespace rename <old> <new>  Rename (rewrites config + markdown)
    docref info                          Show this reference document
    docref info --json                   Machine-readable output

"
    );
    return;
}

/// Print configuration file format and behavior.
fn print_section_configuration() {
    print!(
        "\
## Configuration (.docref.toml)

    include = [\"docs/\", \"src/\"]         # only scan these paths for markdown
    exclude = [\"docs/archive/\"]          # skip these within included paths
    extends = \"../.docref.toml\"          # inherit parent config

    [namespaces]
    auth = \"services/auth\"               # auth:src/lib.rs -> services/auth/src/lib.rs

Include/exclude patterns are path prefixes, not globs. Without .docref.toml,
ALL markdown under the project root is scanned. Create a config to avoid
errors from third-party markdown in node_modules, vendor, etc.

"
    );
    return;
}

/// Print supported language table.
fn print_section_languages() {
    print!(
        "\
## Supported Languages

| Extension | Language   |
|-----------|------------|
| .rs       | Rust       |
| .ts .tsx  | TypeScript |
| .md       | Markdown   |

"
    );
    return;
}

/// Print the exit codes table section.
fn print_section_exit_codes() {
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

/// Print the current project state (config, lockfile, namespaces).
fn print_section_state(state: &CurrentState) {
    println!("## Current State");
    println!();

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
