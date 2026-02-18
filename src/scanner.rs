use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::{Captures, Regex};
use walkdir::WalkDir;

use crate::config::Config;
use crate::error::Error;
use crate::types::{Reference, SymbolQuery};

/// Scan all markdown files under `root` and extract references.
/// Applies the config's include/exclude filters to control which markdown files are scanned.
/// Returns references grouped by target file path for batch resolution.
///
/// # Errors
///
/// Returns `Error::Io` if any markdown file cannot be read.
///
/// # Panics
///
/// Panics if the hardcoded reference regex is invalid (compile-time invariant).
pub fn scan(root: &Path, config: &Config) -> Result<HashMap<PathBuf, Vec<Reference>>, Error> {
    let pattern = Regex::new(r"\[([^\]]+)\]\(([^)#]+)#([^)]+)\)").expect("valid regex");
    let mut grouped: HashMap<PathBuf, Vec<Reference>> = HashMap::new();

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
    {
        let md_path = entry.path();
        let relative_source = md_path.strip_prefix(root).unwrap_or(md_path).to_path_buf();

        let relative_str = relative_source.to_string_lossy();
        if !config.should_scan(&relative_str) {
            continue;
        }

        let content = std::fs::read_to_string(md_path)?;
        extract_references_from_markdown_content(&content, &relative_source, &pattern, &mut grouped);
    }

    Ok(grouped)
}

/// Extract all `[text](path#symbol)` references from markdown content.
fn extract_references_from_markdown_content(
    content: &str,
    source: &Path,
    pattern: &Regex,
    grouped: &mut HashMap<PathBuf, Vec<Reference>>,
) {
    for line in content.lines() {
        extract_references_from_markdown_line(line, source, pattern, grouped);
    }
}

/// Extract references from a single markdown line.
fn extract_references_from_markdown_line(
    line: &str,
    source: &Path,
    pattern: &Regex,
    grouped: &mut HashMap<PathBuf, Vec<Reference>>,
) {
    for cap in pattern.captures_iter(line) {
        let Some(reference) = parse_markdown_link_capture(&cap, source) else {
            continue;
        };
        let target = reference.target.clone();
        grouped.entry(target).or_default().push(reference);
    }
}

/// Collapse `.` and `..` components in a path without touching the filesystem.
/// Preserves leading `..` when there is nothing left to pop.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components: Vec<std::path::Component<'_>> = Vec::new();
    for component in path.components() {
        push_normalized_component(&mut components, component);
    }
    components.iter().collect()
}

/// Handle a single path component during normalization.
/// Pops the last component for `..` when possible, preserves it otherwise.
fn push_normalized_component<'a>(
    components: &mut Vec<std::path::Component<'a>>,
    component: std::path::Component<'a>,
) {
    match component {
        std::path::Component::CurDir => {}
        std::path::Component::ParentDir => {
            let can_pop = matches!(
                components.last(),
                Some(c) if !matches!(c, std::path::Component::ParentDir)
            );
            if can_pop { components.pop(); } else { components.push(component); }
        }
        other => components.push(other),
    }
}

/// Try to parse a regex capture into a local code reference.
/// Returns `None` for external URLs or empty fragments.
fn parse_markdown_link_capture(cap: &Captures<'_>, source: &Path) -> Option<Reference> {
    let raw_target = &cap[2];
    let raw_symbol = &cap[3];

    if raw_target.starts_with("http://")
        || raw_target.starts_with("https://")
        || raw_target.is_empty()
        || raw_symbol.is_empty()
    {
        return None;
    }

    let symbol = parse_symbol_fragment_as_query(raw_symbol);

    // Namespaced reference: store as-is (resolved later through Config).
    let is_namespaced = raw_target.contains(':');
    let target = if is_namespaced {
        PathBuf::from(raw_target)
    } else {
        let source_dir = source.parent().unwrap_or(Path::new(""));
        normalize_path(&source_dir.join(raw_target))
    };

    Some(Reference {
        source: source.to_path_buf(),
        target,
        symbol,
    })
}

/// Parse a symbol fragment into bare or dot-scoped form.
fn parse_symbol_fragment_as_query(raw: &str) -> SymbolQuery {
    if let Some((parent, child)) = raw.split_once('.') {
        SymbolQuery::Scoped {
            parent: parent.to_string(),
            child: child.to_string(),
        }
    } else {
        SymbolQuery::Bare(raw.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaced_reference() {
        let pattern = Regex::new(r"\[([^\]]+)\]\(([^)#]+)#([^)]+)\)").unwrap();
        let source = Path::new("docs/guide.md");
        let line = "See [`validate`](auth:src/lib.rs#validate) for details.";
        let mut grouped: HashMap<PathBuf, Vec<Reference>> = HashMap::new();
        extract_references_from_markdown_line(line, source, &pattern, &mut grouped);

        let refs: Vec<&Reference> = grouped.values().flatten().collect();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target, PathBuf::from("auth:src/lib.rs"));
    }

    #[test]
    fn non_namespaced_resolves_relative_to_markdown() {
        let pattern = Regex::new(r"\[([^\]]+)\]\(([^)#]+)#([^)]+)\)").unwrap();
        let source = Path::new("docs/guide.md");
        let line = "See [`add`](../src/lib.rs#add) for details.";
        let mut grouped: HashMap<PathBuf, Vec<Reference>> = HashMap::new();
        extract_references_from_markdown_line(line, source, &pattern, &mut grouped);

        let refs: Vec<&Reference> = grouped.values().flatten().collect();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target, PathBuf::from("src/lib.rs"));
    }
}
