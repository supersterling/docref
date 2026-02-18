#![allow(clippy::missing_panics_doc)]

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

#[test]
fn accept_updates_stale_reference() {
    let (_tmp, dir) = isolated_fixture("basic");
    let src = dir.join("src/lib.rs");

    let original = std::fs::read_to_string(&src).unwrap();

    // Init, then modify source.
    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(init.status.success());
    let modified = original.replace("const A: i32 = 10;", "const A: i32 = 20;");
    std::fs::write(&src, &modified).unwrap();

    // Check should be stale.
    let check = docref_at(&dir).arg("check").output().unwrap();
    assert_eq!(check.status.code().unwrap(), 1);

    // Accept the specific reference.
    let accept = docref_at(&dir)
        .args(["accept", "src/lib.rs#A"])
        .output()
        .unwrap();
    assert!(
        accept.status.success(),
        "accept failed: {}",
        String::from_utf8_lossy(&accept.stderr)
    );

    // Check should pass now.
    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "check still failing after accept: {}",
        String::from_utf8_lossy(&check.stdout)
    );
}

#[test]
fn typescript_references_resolve_and_check() {
    let (_tmp, dir) = isolated_fixture("basic");

    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    // Lockfile should contain TypeScript references.
    let content = std::fs::read_to_string(dir.join(".docref.lock")).unwrap();
    assert!(content.contains("app.ts"), "lockfile missing TypeScript refs");
    assert!(content.contains("VERSION"), "lockfile missing VERSION symbol");
    assert!(content.contains("greet"), "lockfile missing greet symbol");

    // Check should pass.
    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn markdown_heading_references() {
    let (_tmp, dir) = isolated_fixture("basic");

    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    // Lockfile should contain the markdown-to-markdown ref.
    let content = std::fs::read_to_string(dir.join(".docref.lock")).unwrap();
    assert!(
        content.contains("overview.md"),
        "lockfile missing markdown ref: {content}"
    );
    assert!(
        content.contains("architecture"),
        "lockfile missing heading symbol: {content}"
    );

    // Check passes.
    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn reformatting_does_not_break_check() {
    let (_tmp, dir) = isolated_fixture("basic");
    let src = dir.join("src/lib.rs");

    let original = std::fs::read_to_string(&src).unwrap();

    // Init.
    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(init.status.success());

    // Reformat: add whitespace around parameters and operators.
    let reformatted = original
        .replace("fn add(x: i32) -> i32 {", "fn add( x: i32 ) -> i32 {")
        .replace("x + A", "x  +  A");
    std::fs::write(&src, &reformatted).unwrap();

    // Check should STILL pass (whitespace is normalized away).
    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "whitespace change broke check: {}",
        String::from_utf8_lossy(&check.stdout)
    );
}

#[test]
fn comment_changes_do_not_break_check() {
    let (_tmp, dir) = isolated_fixture("basic");
    let src = dir.join("src/lib.rs");

    let original = std::fs::read_to_string(&src).unwrap();

    // Init.
    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(init.status.success());

    // Add a comment above a referenced symbol.
    let commented =
        original.replace("const A: i32 = 10;", "// base offset\nconst A: i32 = 10;");
    std::fs::write(&src, &commented).unwrap();

    // Check should still pass (comments are stripped from hash).
    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "comment change broke check: {}",
        String::from_utf8_lossy(&check.stdout)
    );
}

#[test]
fn resolve_lists_symbols_in_rust_file() {
    let (_tmp, dir) = isolated_fixture("basic");

    let output = docref_at(&dir)
        .args(["resolve", "src/lib.rs"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains('A'), "should list constant A: {stdout}");
    assert!(stdout.contains("add"), "should list function add: {stdout}");
}

#[test]
fn resolve_finds_specific_symbol() {
    let (_tmp, dir) = isolated_fixture("basic");

    let output = docref_at(&dir)
        .args(["resolve", "src/lib.rs", "add"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("src/lib.rs#add"),
        "should show full reference path: {stdout}"
    );
}

#[test]
fn resolve_lists_markdown_headings() {
    let (_tmp, dir) = isolated_fixture("basic");

    let output = docref_at(&dir)
        .args(["resolve", "docs/overview.md"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("architecture"),
        "should list architecture heading: {stdout}"
    );
    assert!(
        stdout.contains("configuration"),
        "should list configuration heading: {stdout}"
    );
}

#[test]
fn status_shows_all_references() {
    let (_tmp, dir) = isolated_fixture("basic");

    // Init first to create lockfile.
    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(init.status.success());

    let output = docref_at(&dir).arg("status").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should list all tracked references.
    assert!(stdout.contains("lib.rs") && stdout.contains('A'), "missing A: {stdout}");
    assert!(stdout.contains("lib.rs") && stdout.contains("add"), "missing add: {stdout}");
    assert!(
        stdout.contains("app.ts") && stdout.contains("VERSION"),
        "missing VERSION: {stdout}"
    );
}

#[test]
fn dotpath_resolves_impl_method() {
    let (_tmp, dir) = isolated_fixture("scoped");

    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let content = std::fs::read_to_string(dir.join(".docref.lock")).unwrap();
    assert!(
        content.contains("Config.validate"),
        "lockfile missing Config.validate: {content}"
    );

    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(check.status.success());
}

#[test]
fn dotpath_resolves_scoped_heading() {
    let (_tmp, dir) = isolated_fixture("scoped");

    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let content = std::fs::read_to_string(dir.join(".docref.lock")).unwrap();
    assert!(
        content.contains("foo.example"),
        "lockfile missing foo.example: {content}"
    );

    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(check.status.success());
}

#[test]
fn ambiguous_bare_symbol_errors_with_candidates() {
    let (_tmp, dir) = isolated_fixture("scoped");

    // "example" is ambiguous — two ### Example headings under different parents.
    let output = docref_at(&dir)
        .args(["resolve", "docs/overview.md", "example"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "should fail on ambiguous symbol"
    );

    // Error output should suggest qualified dot-paths.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("foo.example") && stderr.contains("bar.example"),
        "should suggest qualified candidates: {stderr}"
    );
}

#[test]
fn namespaced_references_resolve_and_check() {
    let (_tmp, dir) = isolated_fixture("namespaced");

    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    // Lockfile should contain the namespace-prefixed target.
    let content = std::fs::read_to_string(dir.join(".docref.lock")).unwrap();
    assert!(
        content.contains("auth:src/lib.rs"),
        "lockfile should preserve namespace form: {content}"
    );
    assert!(
        content.contains("validate"),
        "lockfile should contain validate symbol: {content}"
    );
    // Also contains the local non-namespaced reference.
    assert!(
        content.contains("\"src/lib.rs\""),
        "lockfile should contain local ref: {content}"
    );

    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn namespaced_reference_detects_stale() {
    let (_tmp, dir) = isolated_fixture("namespaced");

    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(init.status.success());

    // Modify the namespaced target.
    let auth_src = dir.join("services/auth/src/lib.rs");
    std::fs::write(&auth_src, "pub fn validate(input: &str) -> bool {\n    input.len() > 3\n}\n").unwrap();

    let check = docref_at(&dir).arg("check").output().unwrap();
    let code = check.status.code().unwrap();
    assert_eq!(code, 1, "expected stale after modifying namespaced target");
}

#[test]
fn config_excludes_directories() {
    let (_tmp, dir) = isolated_fixture("configured");

    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let content = std::fs::read_to_string(dir.join(".docref.lock")).unwrap();
    assert!(
        content.contains("guide.md"),
        "should include guide.md: {content}"
    );
    assert!(
        !content.contains("ignored.md"),
        "should exclude docs/external/: {content}"
    );
}

#[test]
fn extends_inherits_parent_namespaces() {
    let (_tmp, dir) = isolated_fixture("monorepo");

    // Run docref from the sub-project directory.
    let web_dir = dir.join("services/web");
    let init = docref_at(&web_dir).arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let content = std::fs::read_to_string(web_dir.join(".docref.lock")).unwrap();
    assert!(
        content.contains("shared:src/lib.rs"),
        "lockfile should use inherited namespace: {content}"
    );
    assert!(
        content.contains("greet"),
        "lockfile should contain greet symbol: {content}"
    );

    let check = docref_at(&web_dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn accept_file_updates_all_refs_in_doc() {
    let (_tmp, dir) = isolated_fixture("basic");
    let src = dir.join("src/lib.rs");

    let original = std::fs::read_to_string(&src).unwrap();

    // Init, then modify both referenced symbols.
    let init = docref_at(&dir).arg("init").output().unwrap();
    assert!(init.status.success());

    let modified = original
        .replace("const A: i32 = 10;", "const A: i32 = 99;")
        .replace("x + A", "x * A");
    std::fs::write(&src, &modified).unwrap();

    // Both A and add should be stale.
    let check = docref_at(&dir).arg("check").output().unwrap();
    assert_eq!(check.status.code().unwrap(), 1, "expected stale");

    // Accept all refs originating from guide.md.
    let accept = docref_at(&dir)
        .args(["accept", "--file", "docs/guide.md"])
        .output()
        .unwrap();
    assert!(
        accept.status.success(),
        "accept --file failed: {}",
        String::from_utf8_lossy(&accept.stderr)
    );

    // Check should pass now — guide.md's refs are accepted,
    // and api.md's refs (app.ts) were never stale.
    let check = docref_at(&dir).arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "check still failing after accept --file: {}",
        String::from_utf8_lossy(&check.stdout)
    );
}
