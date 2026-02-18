use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// Copy a fixture directory into an isolated temp dir and return both.
fn isolated_fixture(name: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let src = Path::new("tests/fixtures").join(name);
    let path = tmp.path().to_path_buf();
    copy_dir_recursive(&src, &path);
    (tmp, path)
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let dest_path = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dest_path).unwrap();
            copy_dir_recursive(&entry.path(), &dest_path);
        } else {
            std::fs::copy(entry.path(), &dest_path).unwrap();
        }
    }
}

fn docref_at(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_docref"));
    cmd.current_dir(dir);
    cmd
}

#[test]
fn init_then_check_passes() {
    let (_tmp, dir) = isolated_fixture("basic");

    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(dir.join(".docref.lock").exists(), "lockfile not created");

    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn check_detects_stale_reference() {
    let (_tmp, dir) = isolated_fixture("basic");
    let src = dir.join("src/lib.rs");

    let original = std::fs::read_to_string(&src).unwrap();

    // Init with original code.
    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(init.status.success());

    // Modify the source (change A's value).
    let modified = original.replace("const A: i32 = 10;", "const A: i32 = 20;");
    std::fs::write(&src, &modified).unwrap();

    // Check should fail with exit code 1.
    let check = docref_at(&dir).arg("check").output().unwrap();
    let code = check.status.code().unwrap();
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert_eq!(code, 1, "expected exit 1 (stale), got {code}\nstdout: {stdout}");
    assert!(stdout.contains("STALE"), "output should mention STALE: {stdout}");
}

#[test]
fn check_detects_broken_reference() {
    let (_tmp, dir) = isolated_fixture("basic");
    let src = dir.join("src/lib.rs");

    let original = std::fs::read_to_string(&src).unwrap();

    // Init.
    docref_at(&dir).arg("init").output().unwrap();

    // Remove the referenced symbol entirely.
    let broken = original.replace("const A: i32 = 10;\n", "");
    std::fs::write(&src, &broken).unwrap();

    // Check should fail with exit code 2 (broken).
    let check = docref_at(&dir).arg("check").output().unwrap();
    let code = check.status.code().unwrap();
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert_eq!(code, 2, "expected exit 2 (broken), got {code}\nstdout: {stdout}");
    assert!(stdout.contains("BROKEN"), "output should mention BROKEN: {stdout}");
}
