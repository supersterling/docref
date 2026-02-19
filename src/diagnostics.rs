use std::fmt::Write as _;

use crate::error::Error;
use crate::types::SourceRef;

const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Render an error as valid markdown with bold headings and print to stderr.
pub fn print_error(e: &Error) {
    let md = render_error(e);
    for line in md.lines() {
        if line.starts_with('#') {
            eprintln!("{BOLD}{line}{RESET}");
        } else {
            eprintln!("{line}");
        }
    }
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
        Error::FileNotFound { path } => format!("\
# Error: File Not Found

`{}` does not exist.
", path.display()),

        Error::ConfigNotFound { path } => format!("\
# Error: Config Not Found

`{}` does not exist.

## Fix

Check the `extends` path in your `.docref.toml`.
", path.display()),

        Error::LockfileCorrupt { reason } => format!("\
# Error: Lockfile Corrupt

{reason}

## Fix

Regenerate the lockfile:

    docref init
"),

        Error::ParseFailed { file, reason } => format!("\
# Error: Parse Failed

Could not parse `{}`: {reason}
", file.display()),

        Error::Io(e) => format!("\
# Error: I/O

{e}
"),
        Error::TomlDe(e) => format!("\
# Error: Invalid TOML

{e}
"),
        Error::TomlSer(e) => format!("\
# Error: TOML Serialization

{e}
"),
        // Already handled in render_error, but need exhaustive match.
        _ => format!("\
# Error

{e}
"),
    }
}

fn render_file_too_large(file: &std::path::Path, size_bytes: u64, max_bytes: u64) -> String {
    format!("\
# Error: File Too Large

`{}` is {size_bytes} bytes (max {max_bytes}).
", file.display())
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
    let mut out = format!("\
# Error: Symbol Not Found

Symbol `{symbol}` does not exist in `{file}`.
");

    if !referenced_from.is_empty() {
        out.push_str("\n## Referenced from\n\n");
        for src in referenced_from {
            let _ = writeln!(out, "- {}:{}", src.file.display(), src.line);
            let _ = writeln!(out, "  {}", src.content);
        }
    }

    let best = find_closest_suggestion(symbol, suggestions);

    if let Some(suggestion) = &best {
        let _ = write!(out, "\n## Did you mean `{suggestion}`?\n\n");
        if let Some(src) = referenced_from.first().filter(|s| !s.content.is_empty()) {
            let fixed = src.content.replace(&format!("#{symbol}"), &format!("#{suggestion}"));
            let _ = writeln!(out, "    {fixed}");
        }
        out.push_str("\
\n## Fix

    docref fix
");
    } else if !suggestions.is_empty() {
        out.push_str("\n## Available symbols\n\n");
        for s in suggestions {
            let _ = writeln!(out, "- `{s}`");
        }
    }

    out
}

/// Find the closest matching suggestion by stripping generics and comparing.
pub(crate) fn find_closest_suggestion(symbol: &str, suggestions: &[String]) -> Option<String> {
    let normalized = strip_generics(symbol);
    suggestions.iter()
        .find(|s| strip_generics(s) == normalized)
        .cloned()
}

/// Remove generic parameters (`<...>`) from a symbol name for fuzzy comparison.
fn strip_generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

fn render_ambiguous_symbol(file: &str, symbol: &str, candidates: &[String]) -> String {
    let mut out = format!("\
# Error: Ambiguous Symbol

`{symbol}` matches multiple declarations in `{file}`.

## Candidates

");
    for c in candidates {
        let _ = writeln!(out, "- `{c}`");
    }

    out.push_str("\
\n## Fix

Use the qualified dot-path form:

");
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
