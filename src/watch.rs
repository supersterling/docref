//! File watcher: runs `check` on startup, then re-runs on source changes.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use notify::{RecursiveMode, Watcher as _};

use crate::commands;
use crate::config;
use crate::error;
use crate::lockfile::Lockfile;

/// Debounce delay between filesystem events and re-check.
const DEBOUNCE_MS: u64 = 100;

/// Collect all parent directories of source and target files, plus resolved targets.
fn collect_watch_dirs(
    lockfile: &Lockfile,
    root: &std::path::Path,
    config: &config::Config,
) -> HashSet<PathBuf> {
    let mut dirs = HashSet::new();
    for entry in &lockfile.entries {
        if let Some(parent) = entry.source.parent() {
            dirs.insert(PathBuf::from(".").join(parent));
        }
        if let Some(parent) = entry.target.parent() {
            dirs.insert(PathBuf::from(".").join(parent));
        }
        if let Ok(disk_path) = config.resolve_target(&entry.target)
            && let Some(parent) = disk_path.parent()
        {
            dirs.insert(root.join(parent));
        }
    }
    return dirs;
}

/// Create a filesystem watcher that sends events on the given channel.
///
/// # Errors
///
/// Returns an error if the watcher cannot be created.
fn create_watcher(
    tx: crossbeam_channel::Sender<()>,
) -> Result<notify::RecommendedWatcher, error::Error> {
    return notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res
            && matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Modify(_)
                    | notify::EventKind::Remove(_)
            )
        {
            let _ = tx.send(());
        }
    })
    .map_err(|e| {
        return error::Error::LockfileCorrupt {
            reason: format!("watcher setup failed: {e}"),
        };
    });
}

/// Entry point for the watch command.
///
/// Runs an initial check, then watches relevant files and re-checks on changes.
///
/// # Errors
///
/// Returns errors from config loading, lockfile reading, or watcher setup.
pub fn run(format: &str) -> Result<ExitCode, error::Error> {
    let root = PathBuf::from(".");
    let lock_path = root.join(".docref.lock");

    eprintln!("watch: initial check");
    let mut last_code = run_check(format);

    let config = config::Config::load(&root)?;
    let lockfile = Lockfile::read(&lock_path)?;
    let watch_dirs = collect_watch_dirs(&lockfile, &root, &config);

    let (tx, rx) = crossbeam_channel::unbounded();
    let mut watcher = create_watcher(tx)?;

    for dir in &watch_dirs {
        if dir.exists() {
            let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
        }
    }

    let dir_count = watch_dirs.len();
    eprintln!("watch: monitoring {dir_count} directories, press Ctrl+C to stop");

    while rx.recv().is_ok() {
        let debounce = Duration::from_millis(DEBOUNCE_MS);
        while rx.recv_timeout(debounce).is_ok() {}
        eprintln!("watch: change detected, re-checking...");
        last_code = run_check(format);
    }

    return Ok(last_code);
}

/// Run check once and print result. Returns the exit code from check.
fn run_check(format: &str) -> ExitCode {
    return match commands::check(format) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(3_u8)
        },
    };
}
