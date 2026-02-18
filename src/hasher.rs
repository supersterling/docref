use sha2::{Digest, Sha256};
use tree_sitter::{Language, Node, Parser};

use crate::error::Error;
use crate::types::{ResolvedSymbol, SemanticHash};

/// Compute a semantic hash for a resolved symbol.
///
/// Normalization: extract the symbol's subtree, walk leaf nodes,
/// strip comment and whitespace nodes, join remaining text with
/// single spaces, then SHA-256 hash the result.
///
/// # Errors
///
/// Returns `Error::ParseFailed` if tree-sitter cannot re-parse the symbol snippet.
pub fn hash_symbol(
    source: &str,
    language: &Language,
    symbol: &ResolvedSymbol,
) -> Result<SemanticHash, Error> {
    let start = symbol.byte_range.start as usize;
    let end = symbol.byte_range.end as usize;
    let snippet = &source[start..end];

    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|e| Error::ParseFailed {
            file: std::path::PathBuf::new(),
            reason: e.to_string(),
        })?;

    let tree = parser.parse(snippet, None).ok_or_else(|| Error::ParseFailed {
        file: std::path::PathBuf::new(),
        reason: "hash re-parse failed".to_string(),
    })?;

    let normalized = normalize_symbol_to_semantic_tokens(tree.root_node(), snippet);
    let hash = Sha256::digest(normalized.as_bytes());

    Ok(SemanticHash(format!("{hash:x}")))
}

/// Walk leaf nodes, skip comments and whitespace, join with single space.
fn normalize_symbol_to_semantic_tokens(node: Node<'_>, source: &str) -> String {
    let mut tokens = Vec::new();
    collect_semantic_leaf_tokens(node, source, &mut tokens);
    tokens.join(" ")
}

/// Recursively collect non-comment, non-whitespace leaf token text.
fn collect_semantic_leaf_tokens<'a>(node: Node<'a>, source: &'a str, tokens: &mut Vec<&'a str>) {
    if node.child_count() == 0 {
        let kind = node.kind();

        // Skip comments.
        if kind.contains("comment") {
            return;
        }

        let text = &source[node.start_byte()..node.end_byte()];
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            tokens.push(trimmed);
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_semantic_leaf_tokens(child, source, tokens);
    }
}
