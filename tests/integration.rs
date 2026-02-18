use std::path::Path;
use std::process::Command;

fn docref_cmd(fixture: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_docref"));
    cmd.current_dir(Path::new("tests/fixtures").join(fixture));
    cmd
}

#[test]
fn init_then_check_passes() {
    let lock_path = Path::new("tests/fixtures/basic/.docref.lock");
    let _ = std::fs::remove_file(lock_path);

    let init = docref_cmd("basic").arg("init").output().unwrap();
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(lock_path.exists(), "lockfile not created");

    let check = docref_cmd("basic").arg("check").output().unwrap();
    assert!(
        check.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}
