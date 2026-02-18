use std::ops::Range;
use std::path::Path;

use tree_sitter::{Language, Node, Parser, Tree};

use crate::error::Error;
use crate::types::{ResolvedSymbol, SymbolQuery};

/// Maximum source file size (16 MiB).
const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;

/// A symbol found during file listing (for the resolve command).
pub struct SymbolInfo {
    /// The qualified name (e.g., "add" or "Config.validate").
    pub name: String,
}

/// List all addressable symbols in a source file.
///
/// # Errors
///
/// Returns `Error::FileTooLarge` or `Error::ParseFailed` on invalid input.
pub fn list_symbols(
    file_path: &Path,
    source: &str,
    language: &Language,
) -> Result<Vec<SymbolInfo>, Error> {
    let source_len: u64 = source.len().try_into().unwrap_or(u64::MAX);
    if source_len > MAX_FILE_SIZE {
        return Err(Error::FileTooLarge {
            file: file_path.to_path_buf(),
            size_bytes: source_len,
            max_bytes: MAX_FILE_SIZE,
        });
    }

    let tree = parse_source(file_path, source, language)?;
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let declarations = collect_declarations(tree.root_node(), source, ext);

    Ok(declarations
        .into_iter()
        .map(|d| SymbolInfo { name: d.qualified_name })
        .collect())
}

/// Parse a source file and resolve one symbol query against it.
///
/// # Errors
///
/// Returns `Error::SymbolNotFound` if no declaration matches the query,
/// `Error::AmbiguousSymbol` if multiple declarations match a bare query,
/// `Error::FileTooLarge` if the source exceeds the size limit,
/// or `Error::ParseFailed` if tree-sitter cannot parse the source.
pub fn resolve(
    file_path: &Path,
    source: &str,
    language: &Language,
    query: &SymbolQuery,
) -> Result<ResolvedSymbol, Error> {
    let source_len: u64 = source.len().try_into().unwrap_or(u64::MAX);
    if source_len > MAX_FILE_SIZE {
        return Err(Error::FileTooLarge {
            file: file_path.to_path_buf(),
            size_bytes: source_len,
            max_bytes: MAX_FILE_SIZE,
        });
    }

    let tree = parse_source(file_path, source, language)?;
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let declarations = collect_declarations(tree.root_node(), source, ext);

    match query {
        SymbolQuery::Bare(name) => find_declaration_by_bare_name(&declarations, name, file_path),
        SymbolQuery::Scoped { parent, child } => {
            find_declaration_by_qualified_dotpath(&declarations, parent, child, file_path)
        },
    }
}

/// Parse source into a tree-sitter tree.
///
/// # Errors
///
/// Returns `Error::ParseFailed` if the language cannot be set or parsing fails.
fn parse_source(file_path: &Path, source: &str, language: &Language) -> Result<Tree, Error> {
    let mut parser = Parser::new();
    parser.set_language(language).map_err(|e| Error::ParseFailed {
        file: file_path.to_path_buf(),
        reason: e.to_string(),
    })?;

    parser.parse(source, None).ok_or_else(|| Error::ParseFailed {
        file: file_path.to_path_buf(),
        reason: "tree-sitter returned None".to_string(),
    })
}

/// A raw declaration found while walking the CST.
struct Declaration {
    name: String,
    qualified_name: String,
    byte_range: Range<u32>,
}

/// Dispatch to the correct collector based on file extension.
fn collect_declarations(root: Node<'_>, source: &str, ext: &str) -> Vec<Declaration> {
    match ext {
        "rs" => collect_rust_declarations(root, source),
        "ts" | "tsx" => collect_ts_declarations(root, source),
        "md" | "markdown" => collect_md_declarations(root, source),
        _ => Vec::new(),
    }
}

// ── Rust ───────────────────────────────────────────────────────────────

/// Walk the tree and collect all named Rust declarations.
fn collect_rust_declarations(root: Node<'_>, source: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let mut cursor = root.walk();

    for node in root.children(&mut cursor) {
        if let Some(decl) = rust_top_level_declaration(node, source) {
            declarations.push(decl);
        }
        if node.kind() == "impl_item" {
            collect_impl_methods(node, source, &mut declarations);
        }
    }

    declarations
}

/// Try to extract a top-level declaration from a Rust CST node.
fn rust_top_level_declaration(node: Node<'_>, source: &str) -> Option<Declaration> {
    match node.kind() {
        "function_item" | "const_item" | "struct_item" | "enum_item" | "static_item"
        | "type_item" | "trait_item" => {},
        _ => return None,
    }

    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    Some(Declaration {
        qualified_name: name.clone(),
        name,
        byte_range: start..end,
    })
}

/// Collect methods from a Rust impl block, qualified as "Type.method".
fn collect_impl_methods(impl_node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let Some(type_node) = impl_node.child_by_field_name("type") else {
        return;
    };
    let Ok(type_name) = type_node.utf8_text(source.as_bytes()) else {
        return;
    };
    let type_name = type_name.to_string();

    let Some(body) = impl_node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if let Some(decl) = impl_method_declaration(child, source, &type_name) {
            declarations.push(decl);
        }
    }
}

/// Extract a method declaration from a Rust impl body child node.
fn impl_method_declaration(
    node: Node<'_>,
    source: &str,
    type_name: &str,
) -> Option<Declaration> {
    if node.kind() != "function_item" {
        return None;
    }

    let name_node = node.child_by_field_name("name")?;
    let method_name = name_node.utf8_text(source.as_bytes()).ok()?;
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    Some(Declaration {
        name: method_name.to_string(),
        qualified_name: format!("{type_name}.{method_name}"),
        byte_range: start..end,
    })
}

// ── TypeScript ─────────────────────────────────────────────────────────

/// Walk the tree and collect all named TypeScript declarations.
fn collect_ts_declarations(root: Node<'_>, source: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let mut cursor = root.walk();

    for node in root.children(&mut cursor) {
        if let Some(decl) = ts_top_level_declaration(node, source) {
            declarations.push(decl);
        }
        // lexical_declaration wraps variable_declarator(s).
        if node.kind() == "lexical_declaration" {
            collect_ts_variable_declarators(node, source, &mut declarations);
        }
    }

    declarations
}

/// Try to extract a top-level TypeScript declaration with a direct "name" field.
fn ts_top_level_declaration(node: Node<'_>, source: &str) -> Option<Declaration> {
    match node.kind() {
        "function_declaration" | "class_declaration" | "interface_declaration"
        | "type_alias_declaration" | "enum_declaration" => {},
        _ => return None,
    }

    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    Some(Declaration {
        qualified_name: name.clone(),
        name,
        byte_range: start..end,
    })
}

/// Extract variable names from a TypeScript `lexical_declaration` (const/let/var).
fn collect_ts_variable_declarators(
    node: Node<'_>,
    source: &str,
    declarations: &mut Vec<Declaration>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "variable_declarator" {
            continue;
        }
        let Some(decl) = ts_variable_declarator(child, source, node) else {
            continue;
        };
        declarations.push(decl);
    }
}

/// Extract a single variable declarator as a declaration.
/// Uses the parent `lexical_declaration`'s byte range so the hash
/// covers the full `const X = ...;` statement.
fn ts_variable_declarator(
    node: Node<'_>,
    source: &str,
    parent: Node<'_>,
) -> Option<Declaration> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let start = u32::try_from(parent.start_byte()).ok()?;
    let end = u32::try_from(parent.end_byte()).ok()?;

    Some(Declaration {
        qualified_name: name.clone(),
        name,
        byte_range: start..end,
    })
}

// ── Shared lookup ──────────────────────────────────────────────────────

/// Find a declaration by bare name.
///
/// # Errors
///
/// Returns `Error::SymbolNotFound` if no match, `Error::AmbiguousSymbol` if multiple.
fn find_declaration_by_bare_name(
    declarations: &[Declaration],
    name: &str,
    file_path: &Path,
) -> Result<ResolvedSymbol, Error> {
    let matches: Vec<&Declaration> = declarations.iter().filter(|d| d.name == name).collect();

    match matches.len() {
        0 => Err(Error::SymbolNotFound {
            file: file_path.to_path_buf(),
            symbol: name.to_string(),
        }),
        1 => Ok(declaration_to_resolved_symbol(matches[0])),
        _ => {
            let candidates = matches.iter().map(|d| d.qualified_name.clone()).collect();
            Err(Error::AmbiguousSymbol {
                file: file_path.to_path_buf(),
                symbol: name.to_string(),
                candidates,
            })
        },
    }
}

/// Find a declaration by qualified dot-path (e.g., "Config.validate").
///
/// # Errors
///
/// Returns `Error::SymbolNotFound` if no declaration matches the qualified name.
fn find_declaration_by_qualified_dotpath(
    declarations: &[Declaration],
    parent: &str,
    child: &str,
    file_path: &Path,
) -> Result<ResolvedSymbol, Error> {
    let qualified = format!("{parent}.{child}");

    declarations
        .iter()
        .find(|d| d.qualified_name == qualified)
        .map(declaration_to_resolved_symbol)
        .ok_or_else(|| Error::SymbolNotFound {
            file: file_path.to_path_buf(),
            symbol: qualified,
        })
}

fn declaration_to_resolved_symbol(decl: &Declaration) -> ResolvedSymbol {
    ResolvedSymbol {
        byte_range: decl.byte_range.clone(),
    }
}

// ── Markdown ───────────────────────────────────────────────────────────

/// Walk the tree and collect all headings as declarations (slugified names).
/// Nested headings get qualified names: a `### Example` under `## Foo` becomes `foo.example`.
/// The document title (h1) doesn't participate in scoping — the file path provides that context.
fn collect_md_declarations(root: Node<'_>, source: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    walk_markdown_sections_with_scope(root, source, "", &mut declarations);
    declarations
}

/// Recursively walk section nodes, threading the parent heading slug as context.
/// Each section's heading produces a declaration; child sections inherit the parent's slug.
fn walk_markdown_sections_with_scope(
    node: Node<'_>,
    source: &str,
    parent_slug: &str,
    declarations: &mut Vec<Declaration>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "section" {
            extract_declaration_from_markdown_section(child, source, parent_slug, declarations);
        }
    }
}

/// Process a single section node: extract its heading, build qualified name, recurse into children.
fn extract_declaration_from_markdown_section(
    section: Node<'_>,
    source: &str,
    parent_slug: &str,
    declarations: &mut Vec<Declaration>,
) {
    let Some((slug, is_document_title)) = extract_section_slug_and_title_flag(section, source) else {
        return;
    };

    // Document title (h1) gets a bare qualified name; its slug does NOT prefix children.
    // Scoping starts at h2 and below (the file path already identifies the document).
    let qualified = if is_document_title || parent_slug.is_empty() {
        slug.clone()
    } else {
        format!("{parent_slug}.{slug}")
    };

    let start = u32::try_from(section.start_byte()).ok();
    let end = u32::try_from(section.end_byte()).ok();
    if let (Some(start), Some(end)) = (start, end) {
        declarations.push(Declaration {
            name: slug.clone(),
            qualified_name: qualified.clone(),
            byte_range: start..end,
        });
    }

    // h1 children get bare names (no parent scope); h2+ children get qualified scope.
    let child_scope = if is_document_title { "" } else { &qualified };
    walk_markdown_sections_with_scope(section, source, child_scope, declarations);
}

/// Extract the slugified heading text and whether this is an h1 (document title).
fn extract_section_slug_and_title_flag(section: Node<'_>, source: &str) -> Option<(String, bool)> {
    let mut cursor = section.walk();
    for child in section.children(&mut cursor) {
        if child.kind() != "atx_heading" {
            continue;
        }
        let is_h1 = heading_has_h1_marker(child);
        let text = extract_heading_inline_text(child, source)?;
        let slug = slugify(&text);
        if slug.is_empty() {
            return None;
        }
        return Some((slug, is_h1));
    }
    None
}

/// Check whether a heading is an h1 (document title) by looking for `atx_h1_marker`.
fn heading_has_h1_marker(heading: Node<'_>) -> bool {
    let mut cursor = heading.walk();
    heading
        .children(&mut cursor)
        .any(|c| c.kind() == "atx_h1_marker")
}

/// Extract raw heading text by reading everything after the heading marker.
fn extract_heading_inline_text(heading: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = heading.walk();
    for child in heading.children(&mut cursor) {
        if child.kind() == "heading_content" || child.kind() == "inline" {
            return child.utf8_text(source.as_bytes()).ok().map(String::from);
        }
    }
    // Fallback: take the full heading text and strip the leading #s.
    let text = heading.utf8_text(source.as_bytes()).ok()?;
    let stripped = text.trim_start_matches('#').trim();
    Some(stripped.to_string())
}

/// Convert heading text to a URL-compatible slug.
/// Lowercase, spaces/non-alphanumeric to hyphens, collapse runs, trim edges.
fn slugify(text: &str) -> String {
    let lowered = text.to_lowercase();
    let mut result = String::with_capacity(lowered.len());
    let mut prev_hyphen = true; // Start true to trim leading hyphens.

    for c in lowered.chars() {
        if c.is_alphanumeric() {
            result.push(c);
            prev_hyphen = false;
            continue;
        }
        if prev_hyphen {
            continue;
        }
        result.push('-');
        prev_hyphen = true;
    }

    // Trim trailing hyphen.
    if result.ends_with('-') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn simple_heading() {
        assert_eq!(slugify("Architecture"), "architecture");
    }

    #[test]
    fn multi_word() {
        assert_eq!(slugify("Getting Started"), "getting-started");
    }

    #[test]
    fn special_chars() {
        assert_eq!(slugify("What's New?"), "what-s-new");
    }

    #[test]
    fn consecutive_spaces() {
        assert_eq!(slugify("  Hello   World  "), "hello-world");
    }

    #[test]
    fn empty_string() {
        assert_eq!(slugify(""), "");
    }
}
