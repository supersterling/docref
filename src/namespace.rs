use std::path::{Path, PathBuf};

use crate::config;
use crate::error;
use crate::lockfile::{LockEntry, Lockfile};

// ── CLI commands ──────────────────────────────────────────────────────

/// List all configured namespaces, sorted alphabetically.
///
/// # Errors
///
/// Returns errors from config loading.
pub fn cmd_list() -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let config = config::Config::load(&root)?;

    if config.namespaces.is_empty() {
        println!("No namespaces configured.");
        return Ok(());
    }

    let mut sorted: Vec<_> = config.namespaces.iter().collect();
    sorted.sort_by_key(|(name, _)| name.as_str());
    for (name, entry) in sorted {
        println!("{name} -> {}", entry.path);
    }

    Ok(())
}

/// Add a namespace mapping to the config file.
///
/// # Errors
///
/// Returns errors from config writing.
pub fn cmd_add(name: &str, path: &str) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    add_to_config(&root, name, path)?;
    println!("Added namespace: {name} -> {path}");
    Ok(())
}

/// Rename a namespace across config, lockfile, and markdown files.
///
/// # Errors
///
/// Returns errors from config or lockfile operations, or markdown rewriting.
pub fn cmd_rename(old: &str, new: &str) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    rename_in_config(&root, old, new)?;

    if lock_path.exists() {
        let lockfile = Lockfile::read(&lock_path)?;
        let entries = rename_in_lock_entries(lockfile.entries, old, new);
        let lockfile = Lockfile::new(entries);
        lockfile.write(&lock_path)?;
    }

    let config = config::Config::load(&root)?;
    rewrite_in_markdown_files(&root, &config, old, new)?;

    println!("Renamed namespace: {old} -> {new}");
    Ok(())
}

/// Remove a namespace from config and lockfile. Refuses if references
/// exist unless `force` is set.
///
/// # Errors
///
/// Returns `Error::NamespaceInUse` if references exist (without `--force`),
/// or errors from config/lockfile operations.
pub fn cmd_remove(name: &str, force: bool) -> Result<(), error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    let prefix = format!("{name}:");
    if lock_path.exists() && !force {
        let lockfile = Lockfile::read(&lock_path)?;
        let count = lockfile
            .entries
            .iter()
            .filter(|e| e.target.to_string_lossy().starts_with(&prefix))
            .count();

        if count > 0 {
            return Err(error::Error::NamespaceInUse {
                name: name.to_string(),
                count,
            });
        }
    }

    remove_from_config(&root, name)?;

    if lock_path.exists() {
        let lockfile = Lockfile::read(&lock_path)?;
        let remaining: Vec<LockEntry> = lockfile
            .entries
            .into_iter()
            .filter(|e| !e.target.to_string_lossy().starts_with(&prefix))
            .collect();
        let lockfile = Lockfile::new(remaining);
        lockfile.write(&lock_path)?;
    }

    println!("Removed namespace: {name}");
    Ok(())
}

// ── Config file editing ───────────────────────────────────────────────

/// Parse a `.docref.toml` into a format-preserving document.
/// Returns an empty document if the file doesn't exist.
///
/// # Errors
///
/// Returns `Error::Io` on read failure or `Error::ParseFailed` on parse failure.
fn read_config_doc(root: &Path) -> Result<(PathBuf, toml_edit::DocumentMut), error::Error> {
    let config_path = root.join(".docref.toml");
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(error::Error::Io(e)),
    };

    let doc: toml_edit::DocumentMut = content.parse().map_err(|e: toml_edit::TomlError| {
        error::Error::ParseFailed {
            file: config_path.clone(),
            reason: e.to_string(),
        }
    })?;

    Ok((config_path, doc))
}

/// Add a namespace mapping to `.docref.toml`.
/// Creates the `[namespaces]` table if it doesn't exist.
///
/// # Errors
///
/// Returns `Error::ParseFailed` if the config can't be parsed,
/// or `Error::Io` if writing fails.
fn add_to_config(root: &Path, name: &str, namespace_path: &str) -> Result<(), error::Error> {
    let (config_path, mut doc) = read_config_doc(root)?;

    if !doc.contains_key("namespaces") {
        doc["namespaces"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    doc["namespaces"][name] = toml_edit::value(namespace_path);

    std::fs::write(&config_path, doc.to_string())?;
    Ok(())
}

/// Rename a namespace key in `.docref.toml`.
///
/// # Errors
///
/// Returns `Error::UnknownNamespace` if the old name isn't found.
fn rename_in_config(root: &Path, old: &str, new: &str) -> Result<(), error::Error> {
    let (config_path, mut doc) = read_config_doc(root)?;

    let namespaces = doc
        .get_mut("namespaces")
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| error::Error::UnknownNamespace {
            name: old.to_string(),
        })?;

    let value = namespaces
        .remove(old)
        .ok_or_else(|| error::Error::UnknownNamespace {
            name: old.to_string(),
        })?;

    namespaces.insert(new, value);
    std::fs::write(&config_path, doc.to_string())?;
    Ok(())
}

/// Remove a namespace key from `.docref.toml`.
///
/// # Errors
///
/// Returns `Error::UnknownNamespace` if the name isn't found.
fn remove_from_config(root: &Path, name: &str) -> Result<(), error::Error> {
    let (config_path, mut doc) = read_config_doc(root)?;

    let namespaces = doc
        .get_mut("namespaces")
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| error::Error::UnknownNamespace {
            name: name.to_string(),
        })?;

    if namespaces.remove(name).is_none() {
        return Err(error::Error::UnknownNamespace {
            name: name.to_string(),
        });
    }

    std::fs::write(&config_path, doc.to_string())?;
    Ok(())
}

// ── Lockfile + markdown rewriting ─────────────────────────────────────

/// Replace a namespace prefix in all lock entry targets.
fn rename_in_lock_entries(
    entries: Vec<LockEntry>,
    old: &str,
    new: &str,
) -> Vec<LockEntry> {
    let old_prefix = format!("{old}:");
    let new_prefix = format!("{new}:");

    entries
        .into_iter()
        .map(|mut e| {
            let target_str = e.target.to_string_lossy().to_string();
            if let Some(rest) = target_str.strip_prefix(&old_prefix) {
                e.target = PathBuf::from(format!("{new_prefix}{rest}"));
            }
            e
        })
        .collect()
}

/// Rewrite namespace prefixes in markdown link targets across all scanned files.
///
/// # Errors
///
/// Returns `Error::Io` on file read/write failures.
fn rewrite_in_markdown_files(
    root: &Path,
    config: &config::Config,
    old: &str,
    new: &str,
) -> Result<(), error::Error> {
    let old_prefix = format!("]({old}:");
    let new_prefix = format!("]({new}:");

    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
    {
        let md_path = entry.path();
        let relative = md_path.strip_prefix(root).unwrap_or(md_path);
        if !config.should_scan(&relative.to_string_lossy()) {
            continue;
        }

        let content = std::fs::read_to_string(md_path)?;
        if content.contains(&old_prefix) {
            let updated = content.replace(&old_prefix, &new_prefix);
            std::fs::write(md_path, updated)?;
        }
    }

    Ok(())
}
