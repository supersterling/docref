use std::fmt::Write as _;

use crate::error::Error;
use crate::types::SourceRef;

/// Render an error as terminal-formatted markdown and print to stderr.
pub fn print_error(e: &Error) {
    let md = render_error(e);
    let skin = termimad::MadSkin::default();
    eprintln!("{}", skin.term_text(&md));
}

/// Render an error as a structured markdown diagnostic.
///
/// Each variant produces a block with what happened, why, and how to fix it.
/// Designed to be readable by both humans and LLM agents.
pub fn render_error(e: &Error) -> String {
    match e {
        Error::LockfileNotFound { .. } => render_lockfile_not_found(),
        Error::SymbolNotFound { file, symbol, suggestions, referenced_from } => {
            render_symbol_not_found(&file.display().to_string(), symbol, suggestions, referenced_from)
        },
        Error::AmbiguousSymbol { file, symbol, candidates } => {
            render_ambiguous_symbol(&file.display().to_string(), symbol, candidates)
        },
        Error::UnsupportedLanguage { ext } => render_unsupported_language(ext),
        Error::UnknownNamespace { name } => render_unknown_namespace(name),
        Error::ConfigCycle { chain } => render_config_cycle(chain),
        Error::NamespaceInUse { name, count } => render_namespace_in_use(name, *count),
        Error::FileTooLarge { file, size_bytes, max_bytes } => render_file_too_large(file, *size_bytes, *max_bytes),
        _ => render_generic(e),
    }
}

fn render_generic(e: &Error) -> String {
    match e {
        Error::FileNotFound { path } => {
            format!("# Error: File Not Found\n\n`{}` does not exist.\n", path.display())
        },
        Error::ConfigNotFound { path } => {
            format!(
                "# Error: Config Not Found\n\n`{}` does not exist.\n\n## Fix\n\nCheck the `extends` path in your `.docref.toml`.\n",
                path.display()
            )
        },
        Error::LockfileCorrupt { reason } => {
            format!(
                "# Error: Lockfile Corrupt\n\n{reason}\n\n## Fix\n\nRegenerate the lockfile:\n\n    docref init\n"
            )
        },
        Error::ParseFailed { file, reason } => {
            format!(
                "# Error: Parse Failed\n\nCould not parse `{}`: {reason}\n",
                file.display()
            )
        },
        Error::Io(e) => format!("# Error: I/O\n\n{e}\n"),
        Error::TomlDe(e) => format!("# Error: Invalid TOML\n\n{e}\n"),
        Error::TomlSer(e) => format!("# Error: TOML Serialization\n\n{e}\n"),
        // Already handled in render_error, but need exhaustive match.
        _ => format!("# Error\n\n{e}\n"),
    }
}

fn render_file_too_large(file: &std::path::Path, size_bytes: u64, max_bytes: u64) -> String {
    format!(
        "# Error: File Too Large\n\n`{}` is {size_bytes} bytes (max {max_bytes}).\n",
        file.display()
    )
}

fn render_lockfile_not_found() -> String {
    "\
# Error: Lockfile Not Found

`.docref.lock` does not exist.

## Fix

Run `docref init` to scan markdown and generate the lockfile:

    docref init
"
    .to_string()
}

fn render_symbol_not_found(
    file: &str,
    symbol: &str,
    suggestions: &[String],
    referenced_from: &[SourceRef],
) -> String {
    let mut out = format!(
        "# Error: Symbol Not Found\n\nSymbol `{symbol}` does not exist in `{file}`.\n"
    );

    if !referenced_from.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "## Referenced from\n");
        for src in referenced_from {
            let _ = writeln!(out, "- {}:{}", src.file.display(), src.line);
        }
    }

    if !suggestions.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "## Available symbols\n");
        for s in suggestions {
            let _ = writeln!(out, "- `{s}`");
        }
    }

    out.push('\n');
    let _ = write!(out, "## Fix\n\n    docref resolve {file}    # list all symbols in this file\n");
    out
}

fn render_ambiguous_symbol(file: &str, symbol: &str, candidates: &[String]) -> String {
    let mut out = format!(
        "# Error: Ambiguous Symbol\n\n`{symbol}` matches multiple declarations in `{file}`.\n\n## Candidates\n\n"
    );
    for c in candidates {
        let _ = writeln!(out, "- `{c}`");
    }
    out.push('\n');
    out.push_str("## Fix\n\nUse the qualified dot-path form:\n\n");
    if let Some(first) = candidates.first() {
        let _ = writeln!(out, "    docref resolve {file} {first}");
    }
    out
}

fn render_unsupported_language(ext: &str) -> String {
    format!(
        "\
# Error: Unsupported Language

No tree-sitter grammar for `.{ext}` files.

## Supported extensions

- `.rs` — Rust
- `.ts`, `.tsx` — TypeScript
- `.md` — Markdown
"
    )
}

fn render_unknown_namespace(name: &str) -> String {
    format!(
        "\
# Error: Unknown Namespace

Namespace `{name}` is not configured.

## Fix

Add it to `.docref.toml`:

    [namespaces]
    {name} = \"path/to/{name}\"

Or run:

    docref namespace add {name} path/to/{name}
"
    )
}

fn render_config_cycle(chain: &[std::path::PathBuf]) -> String {
    let chain_str = chain
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" -> ");

    format!(
        "\
# Error: Config Cycle Detected

Circular `extends` chain: {chain_str}

## Fix

Remove the circular `extends` reference in one of the config files.
"
    )
}

fn render_namespace_in_use(name: &str, count: usize) -> String {
    format!(
        "\
# Error: Namespace In Use

Namespace `{name}` is referenced by {count} lockfile entries.

## Fix

Remove all references to `{name}:` first, or force removal:

    docref namespace remove {name} --force
"
    )
}
