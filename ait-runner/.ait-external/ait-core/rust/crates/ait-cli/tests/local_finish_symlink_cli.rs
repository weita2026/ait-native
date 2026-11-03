#![cfg(unix)]

#[path = "support/native_home.rs"]
mod native_home;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ait-cli"))
        .env("CODEX_HOME", native_home::path())
        .env_remove("AIT_REPO_ROOT")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn cli(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "{args:?}: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_files(root: &Path) {
    assert!(fs::symlink_metadata(root.join("alias"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(root.join("alias")).unwrap(),
        Path::new("secret.txt")
    );
    assert_eq!(fs::read(root.join("alias")).unwrap(), b"secret");
    assert!(fs::symlink_metadata(root.join("dangling"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(root.join("dangling")).unwrap(),
        Path::new("missing")
    );
    #[cfg(target_os = "macos")]
    for (name, mode) in [("alias", 0o700), ("dangling", 0o711)] {
        assert_eq!(
            fs::symlink_metadata(root.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            mode
        );
    }
    for (name, mode) in [
        ("secret.txt", 0o600),
        ("plain.txt", 0o644),
        ("run.sh", 0o751),
    ] {
        assert_eq!(
            fs::metadata(root.join(name)).unwrap().permissions().mode() & 0o777,
            mode
        );
    }
}

fn snapshot_link_mode(edit: &Path, snapshot: &str) -> String {
    let shown = cli(edit, &["snapshot", "show", snapshot, "--json"]);
    let row = shown["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == "alias")
        .unwrap();
    let mode = row["mode"].as_str().unwrap();
    assert_eq!(
        u32::from_str_radix(mode.trim_start_matches("0o"), 8).unwrap() & 0o170000,
        0o120000
    );
    mode.to_string()
}

#[test]
fn native_local_finish_and_next_task_preserve_symlinks_and_file_modes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    cli(&repo, &["init", "--json"]);
    cli(
        &repo,
        &[
            "config",
            "set",
            "--sprint",
            "off",
            "--agent-teams",
            "disabled",
            "--default-author-mode",
            "ai_only_experimental",
            "--task-review",
            "automatic",
            "--user-name",
            "Symlink fixture",
            "--user-email",
            "symlink@example.test",
            "--json",
        ],
    );
    let first_root = root.join("first");
    let first = cli(
        &repo,
        &[
            "task",
            "start",
            "--title",
            "Symlink source",
            "--intent",
            "Preserve link entries",
            "--edit-root",
            first_root.to_str().unwrap(),
            "--local",
            "--json",
        ],
    );
    let first_task = first["task_id"].as_str().unwrap();
    let edit = PathBuf::from(first["edit_root"].as_str().unwrap());
    for (name, body, mode) in [
        ("secret.txt", "secret", 0o600),
        ("plain.txt", "plain\n", 0o644),
        ("run.sh", "#!/bin/sh\nexit 0\n", 0o751),
    ] {
        fs::write(edit.join(name), body).unwrap();
        fs::set_permissions(edit.join(name), fs::Permissions::from_mode(mode)).unwrap();
    }
    symlink("secret.txt", edit.join("alias")).unwrap();
    symlink("missing", edit.join("dangling")).unwrap();
    #[cfg(target_os = "macos")]
    for (name, mode) in [("alias", 0o700), ("dangling", 0o711)] {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(edit.join(name).as_os_str().as_bytes()).unwrap();
        // SAFETY: the NUL-terminated fixture path remains live for this syscall.
        assert_eq!(
            unsafe {
                libc::fchmodat(
                    libc::AT_FDCWD,
                    path.as_ptr(),
                    mode as libc::mode_t,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            },
            0
        );
    }
    assert_files(&edit);
    let captured = cli(
        &edit,
        &[
            "snapshot",
            "create",
            first_task,
            "--message",
            "Capture link fixture",
            "--json",
        ],
    );
    let snapshot = captured["snapshot_id"].as_str().unwrap();
    let original_mode = snapshot_link_mode(&edit, snapshot);
    cli(
        &edit,
        &["task", "finish", first_task, "--local", "--json", "--full"],
    );
    assert_files(&repo);

    let next_root = root.join("next");
    let next = cli(
        &repo,
        &[
            "task",
            "start",
            "--title",
            "Unchanged successor",
            "--intent",
            "Capture unchanged link entries",
            "--edit-root",
            next_root.to_str().unwrap(),
            "--local",
            "--json",
        ],
    );
    let next_task = next["task_id"].as_str().unwrap();
    let next_edit = PathBuf::from(next["edit_root"].as_str().unwrap());
    assert_files(&next_edit);
    let next_head = next["head_snapshot_id"].as_str().unwrap();
    assert_eq!(next_head, snapshot);
    let unchanged = run(
        &next_edit,
        &[
            "snapshot",
            "create",
            next_task,
            "--message",
            "Capture unchanged links",
            "--json",
        ],
    );
    assert!(!unchanged.status.success());
    assert!(String::from_utf8_lossy(&unchanged.stderr).contains("workspace tree is unchanged"));
    assert_eq!(snapshot_link_mode(&next_edit, next_head), original_mode);
    assert_files(&next_edit);
    assert_files(&repo);
}
