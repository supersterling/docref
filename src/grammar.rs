use std::path::Path;

use tree_sitter::Language;

use crate::error::Error;

/// Map a file extension to its tree-sitter language.
///
/// # Errors
///
/// Returns `Error::UnsupportedLanguage` for unknown extensions.
pub fn language_for_path(path: &Path) -> Result<Language, Error> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "rs" => Ok(tree_sitter_rust::LANGUAGE.into()),
        "ts" => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Ok(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "md" | "markdown" => Ok(tree_sitter_md::LANGUAGE.into()),
        _ => Err(Error::UnsupportedLanguage {
            ext: ext.to_string(),
        }),
    }
}
