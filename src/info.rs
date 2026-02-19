use std::path::PathBuf;

use serde::Serialize;

use crate::config;
use crate::lockfile::Lockfile;

/// Output the comprehensive docref reference document.
pub fn run(json: bool) {
    let root = PathBuf::from(".");
    let state = gather_state(&root);

    if json {
        print_json(&state);
    } else {
        print_markdown(&state);
    }
}

// ── State gathering ───────────────────────────────────────────────────

struct CurrentState {
    config_found: bool,
    lockfile_entries: Option<usize>,
    namespaces: Vec<(String, String)>,
}

fn gather_state(root: &std::path::Path) -> CurrentState {
    let config_path = root.join(".docref.toml");
    let lock_path = root.join(".docref.lock");

    let config_found = config_path.exists();
    let lockfile_entries = Lockfile::read(&lock_path).ok().map(|l| l.entries.len());

    let namespaces = config::Config::load(root)
        .ok()
        .map(|c| {
            let mut ns: Vec<(String, String)> = c
                .namespaces
                .iter()
                .map(|(name, entry)| (name.clone(), entry.path.clone()))
                .collect();
            ns.sort();
            ns
        })
        .unwrap_or_default();

    CurrentState { config_found, lockfile_entries, namespaces }
}

// ── Markdown output ───────────────────────────────────────────────────

fn print_markdown(state: &CurrentState) {
    let version = env!("CARGO_PKG_VERSION");
    print_markdown_header(version);
    print_markdown_state(state);
    println!();
    print_markdown_exit_codes();
}

fn print_markdown_header(version: &str) {
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
}

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
            .map(|(name, path)| format!("{name} -> {path}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("Namespaces: {ns_list}");
    }
}

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
}

// ── JSON output ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct InfoJson {
    version: String,
    supported_languages: Vec<LanguageInfo>,
    exit_codes: Vec<ExitCodeInfo>,
    current_state: StateJson,
}

#[derive(Serialize)]
struct LanguageInfo {
    extensions: Vec<String>,
    language: String,
}

#[derive(Serialize)]
struct ExitCodeInfo {
    code: u8,
    meaning: String,
}

#[derive(Serialize)]
struct StateJson {
    config_found: bool,
    lockfile_entries: Option<usize>,
    namespaces: Vec<NamespaceJson>,
}

#[derive(Serialize)]
struct NamespaceJson {
    name: String,
    path: String,
}

fn print_json(state: &CurrentState) {
    let info = InfoJson {
        version: env!("CARGO_PKG_VERSION").to_string(),
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
        exit_codes: vec![
            ExitCodeInfo { code: 0, meaning: "Success / all references fresh".to_string() },
            ExitCodeInfo { code: 1, meaning: "Stale references found".to_string() },
            ExitCodeInfo { code: 2, meaning: "Broken references found".to_string() },
            ExitCodeInfo { code: 3, meaning: "Runtime error".to_string() },
        ],
        current_state: StateJson {
            config_found: state.config_found,
            lockfile_entries: state.lockfile_entries,
            namespaces: state
                .namespaces
                .iter()
                .map(|(name, path)| NamespaceJson {
                    name: name.clone(),
                    path: path.clone(),
                })
                .collect(),
        },
    };

    // serde_json::to_string_pretty won't fail on this structure.
    let json = serde_json::to_string_pretty(&info).unwrap_or_default();
    println!("{json}");
}
