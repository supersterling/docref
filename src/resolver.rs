use std::ops::Range;
use std::path::Path;

use tree_sitter::{Language, Node, Parser, Tree};

use crate::error::Error;
use crate::types::{ResolvedSymbol, SymbolQuery};

/// Maximum source file size (16 MiB).
const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;

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
    let declarations = collect_rust_declarations(tree.root_node(), source);

    match query {
        SymbolQuery::Bare(name) => find_by_name(&declarations, name, file_path),
        SymbolQuery::Scoped {
            parent,
            child,
        } => find_scoped(&declarations, parent, child, file_path),
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

/// Walk the tree and collect all named declarations (Rust only for now).
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

/// Try to extract a top-level declaration from a CST node.
fn rust_top_level_declaration(node: Node<'_>, source: &str) -> Option<Declaration> {
    match node.kind() {
        | "function_item"
        | "const_item"
        | "struct_item"
        | "enum_item"
        | "static_item"
        | "type_item"
        | "trait_item" => {},
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

/// Collect methods from an impl block, qualified as "Type.method".
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

/// Extract a method declaration from an impl body child node.
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

/// Find a declaration by bare name.
///
/// # Errors
///
/// Returns `Error::SymbolNotFound` if no match, `Error::AmbiguousSymbol` if multiple.
fn find_by_name(
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
        1 => Ok(to_resolved(matches[0])),
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
fn find_scoped(
    declarations: &[Declaration],
    parent: &str,
    child: &str,
    file_path: &Path,
) -> Result<ResolvedSymbol, Error> {
    let qualified = format!("{parent}.{child}");

    declarations
        .iter()
        .find(|d| d.qualified_name == qualified)
        .map(to_resolved)
        .ok_or_else(|| Error::SymbolNotFound {
            file: file_path.to_path_buf(),
            symbol: qualified,
        })
}

fn to_resolved(decl: &Declaration) -> ResolvedSymbol {
    ResolvedSymbol {
        byte_range: decl.byte_range.clone(),
    }
}
