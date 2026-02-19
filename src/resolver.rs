use std::ops::Range;
use std::path::Path;

use tree_sitter::{Language, Node, Parser, Tree};

use crate::error::Error;
use crate::types::{ResolvedSymbol, SymbolQuery};

/// Maximum source file size (16 MiB).
const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;

/// A raw declaration found while walking the CST.
struct Declaration {
    /// Byte range of the declaration in the source.
    byte_range: Range<u32>,
    /// Short name of the declaration.
    name: String,
    /// Fully qualified name (e.g., "Type.method").
    qualified_name: String,
}

/// A symbol found during file listing (for the resolve command).
pub struct SymbolInfo {
    /// The qualified name (e.g., "add" or "Config.validate").
    pub name: String,
}

/// Collect members from a TypeScript class, qualified as "Class.member".
fn collect_class_members(node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(class_name) = name_node.utf8_text(source.as_bytes()) else {
        return;
    };
    let class_name = class_name.to_string();

    let Some(body) = node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "method_definition" && child.kind() != "public_field_definition" {
            continue;
        }
        let Some(name_child) = first_child_of_kind(child, "property_identifier") else {
            continue;
        };
        let Ok(member_name) = name_child.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: member_name.to_string(),
            qualified_name: format!("{class_name}.{member_name}"),
        });
    }
}

/// Dispatch to the correct collector based on file extension.
fn collect_declarations(root: Node<'_>, source: &str, ext: &str) -> Vec<Declaration> {
    return match ext {
        "go" => collect_go_declarations(root, source),
        "js" | "jsx" | "ts" | "tsx" => collect_ts_declarations(root, source),
        "md" | "markdown" => collect_md_declarations(root, source),
        "py" => collect_py_declarations(root, source),
        "rs" => collect_rust_declarations(root, source),
        _ => Vec::new(),
    };
}

/// Collect members from a TypeScript enum, qualified as "Enum.Member".
fn collect_enum_members(node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(enum_name) = name_node.utf8_text(source.as_bytes()) else {
        return;
    };
    let enum_name = enum_name.to_string();

    let Some(body) = node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if let Some(decl) = ts_enum_member_declaration(child, source, &enum_name) {
            declarations.push(decl);
        }
    }
}

/// Collect variants from a Rust enum, qualified as "Enum.Variant".
fn collect_enum_variants(node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(enum_name) = name_node.utf8_text(source.as_bytes()) else {
        return;
    };
    let enum_name = enum_name.to_string();

    let Some(body) = node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "enum_variant" {
            continue;
        }
        let Some(variant_name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(variant_name) = variant_name_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: variant_name.to_string(),
            qualified_name: format!("{enum_name}.{variant_name}"),
        });
    }
}

/// Collect const declarations from a Go `const_declaration` node.
fn collect_go_const_specs(node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "const_spec" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(name) = name_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: name.to_string(),
            qualified_name: name.to_string(),
        });
    }
}

/// Walk the tree and collect all named Go declarations.
fn collect_go_declarations(root: Node<'_>, source: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let mut cursor = root.walk();

    for node in root.children(&mut cursor) {
        match node.kind() {
            "const_declaration" => collect_go_const_specs(node, source, &mut declarations),
            "function_declaration" => declarations.extend(go_function_declaration(node, source)),
            "method_declaration" => declarations.extend(go_method_declaration(node, source)),
            "type_declaration" => collect_go_type_specs(node, source, &mut declarations),
            "var_declaration" => collect_go_var_specs(node, source, &mut declarations),
            _ => {},
        }
    }

    return declarations;
}

/// Collect method signatures from a Go interface type, qualified as "Interface.Method".
fn collect_go_interface_methods(
    type_name: &str,
    type_node: Node<'_>,
    source: &str,
    declarations: &mut Vec<Declaration>,
) {
    let mut cursor = type_node.walk();
    for child in type_node.children(&mut cursor) {
        if child.kind() != "method_elem" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(method_name) = name_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: method_name.to_string(),
            qualified_name: format!("{type_name}.{method_name}"),
        });
    }
}

/// Collect fields from a Go struct type, qualified as "Struct.Field".
fn collect_go_struct_fields(
    type_name: &str,
    type_node: Node<'_>,
    source: &str,
    declarations: &mut Vec<Declaration>,
) {
    let Some(field_list) = first_child_of_kind(type_node, "field_declaration_list") else {
        return;
    };
    let mut cursor = field_list.walk();
    for child in field_list.children(&mut cursor) {
        if child.kind() != "field_declaration" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(field_name) = name_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: field_name.to_string(),
            qualified_name: format!("{type_name}.{field_name}"),
        });
    }
}

/// Collect type specs from a Go `type_declaration`, extracting sub-declarations
/// for struct fields and interface methods.
fn collect_go_type_specs(node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "type_spec" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(type_name) = name_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: type_name.to_string(),
            qualified_name: type_name.to_string(),
        });

        // Check for struct or interface body.
        let Some(type_body) = child.child_by_field_name("type") else {
            continue;
        };
        match type_body.kind() {
            "interface_type" => {
                collect_go_interface_methods(type_name, type_body, source, declarations);
            },
            "struct_type" => {
                collect_go_struct_fields(type_name, type_body, source, declarations);
            },
            _ => {},
        }
    }
}

/// Collect var declarations from a Go `var_declaration` node.
fn collect_go_var_specs(node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "var_spec" {
            continue;
        }
        if let Some(decl) = go_var_spec_declaration(child, source) {
            declarations.push(decl);
        }
    }
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

/// Collect properties from a TypeScript interface, qualified as "Interface.prop".
fn collect_interface_properties(
    node: Node<'_>,
    source: &str,
    declarations: &mut Vec<Declaration>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(iface_name) = name_node.utf8_text(source.as_bytes()) else {
        return;
    };
    let iface_name = iface_name.to_string();

    let Some(body) = node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "property_signature" {
            continue;
        }
        let Some(prop_node) = first_child_of_kind(child, "property_identifier") else {
            continue;
        };
        let Ok(prop_name) = prop_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: prop_name.to_string(),
            qualified_name: format!("{iface_name}.{prop_name}"),
        });
    }
}

/// Walk the tree and collect all headings as declarations.
///
/// Nested headings get qualified names: a `### Example` under `## Foo`
/// becomes `foo.example`. The document title (h1) doesn't participate
/// in scoping — the file path provides that context.
fn collect_md_declarations(root: Node<'_>, source: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    walk_markdown_sections_with_scope(root, source, "", &mut declarations);
    return declarations;
}

/// Collect methods from a Python class body, qualified as "Class.method".
fn collect_py_class_members(
    node: Node<'_>,
    source: &str,
    class_name: &str,
    declarations: &mut Vec<Declaration>,
) {
    let Some(body) = node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        let inner = py_unwrap_decorated(child);

        if inner.kind() != "function_definition" {
            continue;
        }
        let Some(name_node) = inner.child_by_field_name("name") else {
            continue;
        };
        let Ok(method_name) = name_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        if method_name == "__init__" {
            collect_py_init_attributes(inner, source, class_name, declarations);
            continue;
        }
        declarations.extend(py_method_declaration(inner, source, class_name, child));
    }
}

/// Walk the tree and collect all named Python declarations.
fn collect_py_declarations(root: Node<'_>, source: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let mut cursor = root.walk();

    for node in root.children(&mut cursor) {
        collect_py_top_level_node(node, source, &mut declarations);
    }

    return declarations;
}

/// Extract `self.attr` assignments from a Python `__init__` method body.
fn collect_py_init_attributes(
    func: Node<'_>,
    source: &str,
    class_name: &str,
    declarations: &mut Vec<Declaration>,
) {
    let Some(body) = func.child_by_field_name("body") else {
        return;
    };
    let mut seen = std::collections::HashSet::new();
    collect_py_self_attributes_recursive(body, source, class_name, declarations, &mut seen);
}

/// Recursively walk a function body to find `self.attr = ...` assignments.
///
/// Handles `self.attr` inside if/for/try/with blocks. Deduplicates by qualified name
/// since conditional branches may assign the same attribute.
fn collect_py_self_attributes_recursive(
    node: Node<'_>,
    source: &str,
    class_name: &str,
    declarations: &mut Vec<Declaration>,
    seen: &mut std::collections::HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "expression_statement"
            && let Some(decl) = py_self_attribute_assignment(child, source, class_name)
            && seen.insert(decl.qualified_name.clone())
        {
            declarations.push(decl);
            continue;
        }
        if matches!(
            child.kind(),
            "if_statement" | "for_statement" | "while_statement" | "try_statement"
                | "with_statement" | "block" | "else_clause" | "elif_clause" | "except_clause"
                | "finally_clause"
        ) {
            collect_py_self_attributes_recursive(child, source, class_name, declarations, seen);
        }
    }
}

/// Process a single top-level Python node, collecting its declaration and any class members.
fn collect_py_top_level_node(
    node: Node<'_>,
    source: &str,
    declarations: &mut Vec<Declaration>,
) {
    let inner = py_unwrap_decorated(node);

    if let Some(decl) = py_named_declaration(inner, source, node) {
        if inner.kind() == "class_definition" {
            collect_py_class_members(inner, source, &decl.name, declarations);
        }
        declarations.push(decl);
        return;
    }

    if inner.kind() == "expression_statement"
        && let Some(decl) = py_module_variable(inner, source)
    {
        declarations.push(decl);
    }
}

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
        if node.kind() == "struct_item" {
            collect_struct_fields(node, source, &mut declarations);
        }
        if node.kind() == "enum_item" {
            collect_enum_variants(node, source, &mut declarations);
        }
        if node.kind() == "trait_item" {
            collect_trait_methods(node, source, &mut declarations);
        }
    }

    return declarations;
}

/// Collect fields from a Rust struct, qualified as "Struct.field".
fn collect_struct_fields(node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(struct_name) = name_node.utf8_text(source.as_bytes()) else {
        return;
    };
    let struct_name = struct_name.to_string();

    let Some(body) = node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "field_declaration" {
            continue;
        }
        let Some(field_name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(field_name) = field_name_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: field_name.to_string(),
            qualified_name: format!("{struct_name}.{field_name}"),
        });
    }
}

/// Collect method signatures and default methods from a Rust trait, qualified as "Trait.method".
fn collect_trait_methods(node: Node<'_>, source: &str, declarations: &mut Vec<Declaration>) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(trait_name) = name_node.utf8_text(source.as_bytes()) else {
        return;
    };
    let trait_name = trait_name.to_string();

    let Some(body) = node.child_by_field_name("body") else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "function_signature_item" && child.kind() != "function_item" {
            continue;
        }
        let Some(method_name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(method_name) = method_name_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(start) = u32::try_from(child.start_byte()).ok() else {
            continue;
        };
        let Some(end) = u32::try_from(child.end_byte()).ok() else {
            continue;
        };
        declarations.push(Declaration {
            byte_range: start..end,
            name: method_name.to_string(),
            qualified_name: format!("{trait_name}.{method_name}"),
        });
    }
}

/// Walk the tree and collect all named TypeScript declarations.
fn collect_ts_declarations(root: Node<'_>, source: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let mut cursor = root.walk();

    for node in root.children(&mut cursor) {
        let inner = if node.kind() == "export_statement" {
            unwrap_export(node)
        } else {
            node
        };

        if let Some(decl) = ts_top_level_declaration(inner, source) {
            declarations.push(decl);
        }
        if inner.kind() == "lexical_declaration" {
            collect_ts_variable_declarators(inner, source, &mut declarations);
        }
        if inner.kind() == "interface_declaration" {
            collect_interface_properties(inner, source, &mut declarations);
        }
        if inner.kind() == "class_declaration" {
            collect_class_members(inner, source, &mut declarations);
        }
        if inner.kind() == "enum_declaration" {
            collect_enum_members(inner, source, &mut declarations);
        }
    }

    return declarations;
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

/// Convert a declaration to its resolved symbol representation.
fn declaration_to_resolved_symbol(decl: &Declaration) -> ResolvedSymbol {
    return ResolvedSymbol {
        byte_range: decl.byte_range.clone(),
    };
}

/// Process a single section node: extract its heading, build qualified name, recurse.
fn extract_declaration_from_markdown_section(
    section: Node<'_>,
    source: &str,
    parent_slug: &str,
    declarations: &mut Vec<Declaration>,
) {
    let Some((slug, is_document_title)) =
        extract_section_slug_and_title_flag(section, source)
    else {
        return;
    };

    let qualified = if is_document_title || parent_slug.is_empty() {
        slug.clone()
    } else {
        format!("{parent_slug}.{slug}")
    };

    let start = u32::try_from(section.start_byte()).ok();
    let end = u32::try_from(section.end_byte()).ok();
    if let (Some(start), Some(end)) = (start, end) {
        declarations.push(Declaration {
            byte_range: start..end,
            name: slug.clone(),
            qualified_name: qualified.clone(),
        });
    }

    let child_scope = if is_document_title { "" } else { &qualified };
    walk_markdown_sections_with_scope(section, source, child_scope, declarations);
}

/// Extract raw heading text by reading everything after the heading marker.
fn extract_heading_inline_text(heading: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = heading.walk();
    for child in heading.children(&mut cursor) {
        if child.kind() == "heading_content" || child.kind() == "inline" {
            return child.utf8_text(source.as_bytes()).ok().map(String::from);
        }
    }
    let text = heading.utf8_text(source.as_bytes()).ok()?;
    let stripped = text.trim_start_matches('#').trim();
    return Some(stripped.to_string());
}

/// Extract the slugified heading text and whether this is an h1 (document title).
fn extract_section_slug_and_title_flag(
    section: Node<'_>,
    source: &str,
) -> Option<(String, bool)> {
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
    return None;
}

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
    let matches: Vec<&Declaration> = declarations
        .iter()
        .filter(|d| return d.name == name)
        .collect();

    match matches.len() {
        0 => return Err(symbol_not_found_error(file_path, name, declarations)),
        1 => {
            return Ok(declaration_to_resolved_symbol(
                matches.first().ok_or_else(|| {
                    return symbol_not_found_error(file_path, name, declarations);
                })?,
            ));
        }
        _ => {
            let candidates = matches
                .iter()
                .map(|d| return d.qualified_name.clone())
                .collect();
            return Err(Error::AmbiguousSymbol {
                candidates,
                file: file_path.to_path_buf(),
                symbol: name.to_string(),
            });
        }
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

    return declarations
        .iter()
        .find(|d| return d.qualified_name == qualified)
        .map(declaration_to_resolved_symbol)
        .ok_or_else(|| {
            return symbol_not_found_error(file_path, &qualified, declarations);
        });
}

/// Find the first child of a specific node kind.
fn first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    return node.children(&mut cursor).find(|c| return c.kind() == kind);
}

/// Extract a top-level function declaration from Go.
fn go_function_declaration(node: Node<'_>, source: &str) -> Option<Declaration> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    return Some(Declaration {
        byte_range: start..end,
        name: name.clone(),
        qualified_name: name,
    });
}

/// Extract a method declaration from Go, qualified as "Type.Method".
///
/// Handles pointer receivers: `func (c *Config) Validate()` → `Config.Validate`.
fn go_method_declaration(node: Node<'_>, source: &str) -> Option<Declaration> {
    let receiver = node.child_by_field_name("receiver")?;
    let type_name = go_receiver_type_name(receiver, source)?;

    let name_node = node.child_by_field_name("name")?;
    let method_name = name_node.utf8_text(source.as_bytes()).ok()?;
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    return Some(Declaration {
        byte_range: start..end,
        name: method_name.to_string(),
        qualified_name: format!("{type_name}.{method_name}"),
    });
}

/// Extract the receiver type name, unwrapping pointer types.
///
/// `(c *Config)` → `Config`, `(c Config)` → `Config`.
fn go_receiver_type_name(receiver: Node<'_>, source: &str) -> Option<String> {
    // receiver is a parameter_list containing parameter_declaration(s).
    let mut cursor = receiver.walk();
    for child in receiver.children(&mut cursor) {
        if child.kind() != "parameter_declaration" {
            continue;
        }
        let type_node = child.child_by_field_name("type")?;
        // Unwrap pointer_type if present.
        let base = if type_node.kind() == "pointer_type" {
            let mut inner_cursor = type_node.walk();
            type_node
                .children(&mut inner_cursor)
                .find(|c| return c.kind() == "type_identifier")?
        } else {
            type_node
        };
        return base.utf8_text(source.as_bytes()).ok().map(String::from);
    }
    return None;
}

/// Extract a single var spec as a declaration.
fn go_var_spec_declaration(node: Node<'_>, source: &str) -> Option<Declaration> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    return Some(Declaration {
        byte_range: start..end,
        name: name.clone(),
        qualified_name: name,
    });
}

/// Check whether a heading is an h1 (document title) by looking for `atx_h1_marker`.
fn heading_has_h1_marker(heading: Node<'_>) -> bool {
    let mut cursor = heading.walk();
    return heading
        .children(&mut cursor)
        .any(|c| return c.kind() == "atx_h1_marker");
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

    return Some(Declaration {
        byte_range: start..end,
        name: method_name.to_string(),
        qualified_name: format!("{type_name}.{method_name}"),
    });
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
            max_bytes: MAX_FILE_SIZE,
            size_bytes: source_len,
        });
    }

    let tree = parse_source(file_path, source, language)?;
    let ext = file_path
        .extension()
        .and_then(|e| return e.to_str())
        .unwrap_or("");
    let declarations = collect_declarations(tree.root_node(), source, ext);

    return Ok(declarations
        .into_iter()
        .map(|d| {
            return SymbolInfo {
                name: d.qualified_name,
            };
        })
        .collect());
}

/// Parse source into a tree-sitter tree.
///
/// # Errors
///
/// Returns `Error::ParseFailed` if the language cannot be set or parsing fails.
fn parse_source(file_path: &Path, source: &str, language: &Language) -> Result<Tree, Error> {
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|e| {
            return Error::ParseFailed {
                file: file_path.to_path_buf(),
                reason: e.to_string(),
            };
        })?;

    return parser
        .parse(source, None)
        .ok_or_else(|| {
            return Error::ParseFailed {
                file: file_path.to_path_buf(),
                reason: "tree-sitter returned None".to_string(),
            };
        });
}

/// Extract a method declaration from a Python class body.
///
/// `outer` is the possibly-decorated node whose byte range covers decorators.
fn py_method_declaration(
    func: Node<'_>,
    source: &str,
    class_name: &str,
    outer: Node<'_>,
) -> Option<Declaration> {
    let name_node = func.child_by_field_name("name")?;
    let method_name = name_node.utf8_text(source.as_bytes()).ok()?;

    // Skip dunder methods — __init__ is handled separately for self.attr extraction.
    if method_name.starts_with("__") && method_name.ends_with("__") {
        return None;
    }

    let start = u32::try_from(outer.start_byte()).ok()?;
    let end = u32::try_from(outer.end_byte()).ok()?;

    return Some(Declaration {
        byte_range: start..end,
        name: method_name.to_string(),
        qualified_name: format!("{class_name}.{method_name}"),
    });
}

/// Extract a top-level module variable from `NAME = ...` assignment.
fn py_module_variable(node: Node<'_>, source: &str) -> Option<Declaration> {
    // expression_statement -> assignment
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "assignment" {
            continue;
        }
        let left = child.child_by_field_name("left")?;
        if left.kind() != "identifier" {
            return None;
        }
        let name = left.utf8_text(source.as_bytes()).ok()?.to_string();
        // Skip private variables (leading underscore).
        if name.starts_with('_') {
            return None;
        }
        let start = u32::try_from(node.start_byte()).ok()?;
        let end = u32::try_from(node.end_byte()).ok()?;

        return Some(Declaration {
            byte_range: start..end,
            name: name.clone(),
            qualified_name: name,
        });
    }
    return None;
}

/// Extract a top-level function or class declaration from Python.
///
/// `outer` is the node to use for byte range (may be `decorated_definition`).
fn py_named_declaration(node: Node<'_>, source: &str, outer: Node<'_>) -> Option<Declaration> {
    match node.kind() {
        "class_definition" | "function_definition" => {},
        _ => return None,
    }

    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let start = u32::try_from(outer.start_byte()).ok()?;
    let end = u32::try_from(outer.end_byte()).ok()?;

    return Some(Declaration {
        byte_range: start..end,
        name: name.clone(),
        qualified_name: name,
    });
}

/// Extract a `self.attr = ...` assignment as a `Class.attr` declaration.
fn py_self_attribute_assignment(
    node: Node<'_>,
    source: &str,
    class_name: &str,
) -> Option<Declaration> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "assignment" {
            continue;
        }
        let left = child.child_by_field_name("left")?;
        if left.kind() != "attribute" {
            return None;
        }
        let object = left.child_by_field_name("object")?;
        if object.utf8_text(source.as_bytes()).ok()? != "self" {
            return None;
        }
        let attr = left.child_by_field_name("attribute")?;
        let attr_name = attr.utf8_text(source.as_bytes()).ok()?;
        let start = u32::try_from(node.start_byte()).ok()?;
        let end = u32::try_from(node.end_byte()).ok()?;

        return Some(Declaration {
            byte_range: start..end,
            name: attr_name.to_string(),
            qualified_name: format!("{class_name}.{attr_name}"),
        });
    }
    return None;
}

/// Unwrap a `decorated_definition` to its inner definition node.
/// Returns the node itself if it's not a decorated definition.
fn py_unwrap_decorated(node: Node<'_>) -> Node<'_> {
    if node.kind() != "decorated_definition" {
        return node;
    }
    return node
        .child_by_field_name("definition")
        .unwrap_or(node);
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
            max_bytes: MAX_FILE_SIZE,
            size_bytes: source_len,
        });
    }

    let tree = parse_source(file_path, source, language)?;
    let ext = file_path
        .extension()
        .and_then(|e| return e.to_str())
        .unwrap_or("");
    let declarations = collect_declarations(tree.root_node(), source, ext);

    return match query {
        SymbolQuery::Bare(name) => find_declaration_by_bare_name(&declarations, name, file_path),
        SymbolQuery::Scoped { parent, child } => {
            find_declaration_by_qualified_dotpath(&declarations, parent, child, file_path)
        }
        SymbolQuery::WholeFile => unreachable!("resolver should not be called for whole-file queries"),
    };
}

/// Try to extract a top-level declaration from a Rust CST node.
fn rust_top_level_declaration(node: Node<'_>, source: &str) -> Option<Declaration> {
    match node.kind() {
        "function_item" | "const_item" | "struct_item" | "enum_item" | "static_item"
        | "type_item" | "trait_item" => {}
        _ => return None,
    }

    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    return Some(Declaration {
        byte_range: start..end,
        name: name.clone(),
        qualified_name: name,
    });
}

/// Convert heading text to a URL-compatible slug.
///
/// Lowercase, spaces/non-alphanumeric to hyphens, collapse runs, trim edges.
fn slugify(text: &str) -> String {
    let lowered = text.to_lowercase();
    let mut result = String::with_capacity(lowered.len());
    let mut prev_hyphen = true;

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

    if result.ends_with('-') {
        result.pop();
    }
    return result;
}

/// Build a `SymbolNotFound` error with suggestion names from available declarations.
fn symbol_not_found_error(
    file_path: &Path,
    name: &str,
    declarations: &[Declaration],
) -> Error {
    let suggestions: Vec<String> = declarations
        .iter()
        .map(|d| return d.qualified_name.clone())
        .take(10)
        .collect();
    return Error::SymbolNotFound {
        file: file_path.to_path_buf(),
        referenced_from: vec![],
        suggestions,
        symbol: name.to_string(),
    };
}

/// Extract a single enum member declaration from a TypeScript enum body child.
fn ts_enum_member_declaration(
    node: Node<'_>,
    source: &str,
    enum_name: &str,
) -> Option<Declaration> {
    let name_text = match node.kind() {
        "enum_assignment" => {
            let prop_node = first_child_of_kind(node, "property_identifier")?;
            prop_node.utf8_text(source.as_bytes()).ok()?
        }
        "property_identifier" => node.utf8_text(source.as_bytes()).ok()?,
        _ => return None,
    };
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    return Some(Declaration {
        byte_range: start..end,
        name: name_text.to_string(),
        qualified_name: format!("{enum_name}.{name_text}"),
    });
}

/// Try to extract a top-level TypeScript declaration with a direct "name" field.
fn ts_top_level_declaration(node: Node<'_>, source: &str) -> Option<Declaration> {
    match node.kind() {
        "function_declaration" | "class_declaration" | "interface_declaration"
        | "type_alias_declaration" | "enum_declaration" => {}
        _ => return None,
    }

    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let start = u32::try_from(node.start_byte()).ok()?;
    let end = u32::try_from(node.end_byte()).ok()?;

    return Some(Declaration {
        byte_range: start..end,
        name: name.clone(),
        qualified_name: name,
    });
}

/// Extract a single variable declarator as a declaration.
///
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

    return Some(Declaration {
        byte_range: start..end,
        name: name.clone(),
        qualified_name: name,
    });
}

/// Unwrap an `export_statement` to its inner declaration node.
/// Falls back to the export node itself if no declaration child is found.
fn unwrap_export(export: Node<'_>) -> Node<'_> {
    let mut cursor = export.walk();
    for child in export.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "class_declaration" | "interface_declaration"
            | "type_alias_declaration" | "enum_declaration" | "lexical_declaration" => {
                return child;
            }
            _ => {}
        }
    }
    return export;
}

/// Recursively walk section nodes, threading the parent heading slug as context.
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

#[cfg(test)]
#[allow(clippy::missing_panics_doc, reason = "test code uses unwrap freely")]
mod tests {
    use super::slugify;

    #[test]
    fn consecutive_spaces() {
        assert_eq!(slugify("  Hello   World  "), "hello-world");
    }

    #[test]
    fn empty_string() {
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn multi_word() {
        assert_eq!(slugify("Getting Started"), "getting-started");
    }

    #[test]
    fn simple_heading() {
        assert_eq!(slugify("Architecture"), "architecture");
    }

    #[test]
    fn special_chars() {
        assert_eq!(slugify("What's New?"), "what-s-new");
    }
}
