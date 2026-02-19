//! Diagnostic rendering for docref errors.
//!
//! Converts structured `Error` variants into human-readable markdown
//! diagnostics printed to stderr, with bold headings for terminal display.

use std::fmt::Write as _;

use crate::error::Error;
use crate::types::SourceRef;

/// ANSI escape code for bold text.
const BOLD: &str = "\x1b[1m";
/// ANSI escape code to reset text formatting.
const RESET: &str = "\x1b[0m";

/// Find the closest matching suggestion by stripping generics and comparing.
pub(crate) fn find_closest_suggestion(symbol: &str, suggestions: &[String]) -> Option<String> {
    let normalized = strip_generics(symbol);
    return suggestions.iter()
        .find(|s| return strip_generics(s) == normalized)
        .cloned();
}

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
    return;
}

/// Render an ambiguous symbol diagnostic with candidate list and fix suggestion.
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
    return out;
}

/// Render a configuration cycle diagnostic showing the extends chain.
fn render_config_cycle(chain: &[std::path::PathBuf]) -> String {
    let chain_str = chain
        .iter()
        .map(|p| return p.display().to_string())
        .collect::<Vec<_>>()
        .join(" -> ");

    return format!(
        "\
# Error: Config Cycle Detected

Circular `extends` chain: {chain_str}

## Fix

Remove the circular `extends` reference in one of the config files.
"
    );
}

/// Render a config-not-found diagnostic with fix instructions.
fn render_config_not_found(path: &std::path::Path) -> String {
    return format!("\
# Error: Config Not Found

`{}` does not exist.

## Fix

Check the `extends` path in your `.docref.toml`.
", path.display());
}

/// Render an error as a structured markdown diagnostic.
///
/// Each variant produces a block with what happened, why, and how to fix it.
/// Designed to be readable by both humans and LLM agents.
pub fn render_error(e: &Error) -> String {
    return match e {
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
    };
}

/// Render a file-not-found diagnostic.
fn render_file_not_found(path: &std::path::Path) -> String {
    return format!("\
# Error: File Not Found

`{}` does not exist.
", path.display());
}

/// Render a file-too-large diagnostic with actual and maximum sizes.
fn render_file_too_large(file: &std::path::Path, size_bytes: u64, max_bytes: u64) -> String {
    return format!("\
# Error: File Too Large

`{}` is {size_bytes} bytes (max {max_bytes}).
", file.display());
}

/// Render a generic error diagnostic for variants without specialized rendering.
fn render_generic(e: &Error) -> String {
    return match e {
        Error::FileNotFound { path } => render_file_not_found(path),
        Error::ConfigNotFound { path } => render_config_not_found(path),
        Error::LockfileCorrupt { reason } => render_lockfile_corrupt(reason),
        Error::ParseFailed { file, reason } => render_parse_failed(file, reason),
        Error::Io(e) => format!("# Error: I/O\n\n{e}\n"),
        Error::TomlDe(e) => format!("# Error: Invalid TOML\n\n{e}\n"),
        Error::TomlSer(e) => format!("# Error: TOML Serialization\n\n{e}\n"),
        // Already handled in render_error, but need exhaustive match.
        _ => format!("# Error\n\n{e}\n"),
    };
}

/// Render a lockfile-corrupt diagnostic with regeneration instructions.
fn render_lockfile_corrupt(reason: &str) -> String {
    return format!("\
# Error: Lockfile Corrupt

{reason}

## Fix

Regenerate the lockfile:

    docref init
");
}

/// Render a lockfile-not-found diagnostic with fix instructions.
fn render_lockfile_not_found() -> String {
    return "\
# Error: Lockfile Not Found

`.docref.lock` does not exist.

## Fix

Run `docref init` to scan markdown and generate the lockfile:

    docref init
"
    .to_string();
}

/// Render a namespace-in-use diagnostic showing reference count and force option.
fn render_namespace_in_use(name: &str, count: usize) -> String {
    return format!(
        "\
# Error: Namespace In Use

Namespace `{name}` is referenced by {count} lockfile entries.

## Fix

Remove all references to `{name}:` first, or force removal:

    docref namespace remove {name} --force
"
    );
}

/// Render a parse-failed diagnostic.
fn render_parse_failed(file: &std::path::Path, reason: &str) -> String {
    return format!("\
# Error: Parse Failed

Could not parse `{}`: {reason}
", file.display());
}

/// Render a symbol-not-found diagnostic with suggestions and fix hints.
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
        if let Some(src) = referenced_from.first().filter(|s| return !s.content.is_empty()) {
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

    return out;
}

/// Render an unknown-namespace diagnostic with configuration instructions.
fn render_unknown_namespace(name: &str) -> String {
    return format!(
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
    );
}

/// Render an unsupported-language diagnostic listing supported extensions.
fn render_unsupported_language(ext: &str) -> String {
    return format!(
        "\
# Error: Unsupported Language

No tree-sitter grammar for `.{ext}` files.

## Supported extensions

- `.rs` — Rust
- `.ts`, `.tsx` — TypeScript
- `.md` — Markdown
"
    );
}

/// Remove generic parameters (`<...>`) from a symbol name for fuzzy comparison.
fn strip_generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0_u32;
    for ch in s.chars() {
        match ch {
            '<' => depth = depth.saturating_add(1),
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    return out;
}
