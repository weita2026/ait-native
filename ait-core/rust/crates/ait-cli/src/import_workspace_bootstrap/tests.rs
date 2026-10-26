use super::*;
use crate::init_surface::{init_repo, InitRequest};
use std::path::Path;
use tempfile::TempDir;

fn fixture() -> (TempDir, RepoRuntime) {
    let dir = TempDir::new().unwrap();
    init_repo(&InitRequest {
        root: dir.path().to_path_buf(),
        name: Some("bootstrap".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(dir.path()).unwrap();
    (dir, repo)
}

fn custom_ci() -> Vec<u8> {
    br#"{"schema_version":1,"suites":[{"schema_version":1,"suite_id":"custom","plane":"patchset","mode":"gate","default_blocking":true,"runner":{"kind":"command_bundle","commands":["custom-check --literal"]}}]}"#.to_vec()
}

fn saved(path: &Path) -> (Vec<u8>, std::time::SystemTime, u32) {
    let m = fs::metadata(path).unwrap();
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode()
    };
    #[cfg(not(unix))]
    let mode = u32::from(m.permissions().readonly());
    (fs::read(path).unwrap(), m.modified().unwrap(), mode)
}

#[test]
fn first_setup_and_repeat_create_no_ci_or_history() {
    let (dir, repo) = fixture();
    for path in [".aitignore", "AGENTS.md", "CLAUDE.md"] {
        fs::remove_file(dir.path().join(path)).unwrap();
    }
    fs::remove_dir(dir.path().join("docs/sprints")).unwrap();
    let first = complete(&repo).unwrap();
    assert_eq!(
        first["created_paths"],
        json!([".aitignore", "AGENTS.md", "CLAUDE.md", "docs/sprints"])
    );
    assert_eq!(first["status"], json!("completed"));
    assert_eq!(first["code_snapshot_created"], json!(false));
    let paths = [".aitignore", "AGENTS.md", "CLAUDE.md"];
    let before = paths.map(|p| saved(&dir.path().join(p)));
    let again = complete(&repo).unwrap();
    assert_eq!(again["mutated"], json!(false));
    assert_eq!(before, paths.map(|p| saved(&dir.path().join(p))));
    assert!(!dir.path().join("ci").exists());
}

#[test]
fn existing_custom_settings_permissions_and_pins_are_not_refreshed() {
    let (dir, repo) = fixture();
    fs::create_dir(dir.path().join("ci")).unwrap();
    fs::write(dir.path().join("ci/patch_ci.json"), custom_ci()).unwrap();
    fs::write(
        dir.path().join("AGENTS.md"),
        "# User agent\r\ncustom instructions\r\n",
    )
    .unwrap();
    fs::write(dir.path().join("CLAUDE.md"), "# User Claude\ncustom\n").unwrap();
    fs::write(dir.path().join(".aitignore"), "# user\r\ncache/\r\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            dir.path().join("ci/patch_ci.json"),
            fs::Permissions::from_mode(0o440),
        )
        .unwrap();
    }
    let paths = [
        "ci/patch_ci.json",
        "AGENTS.md",
        "CLAUDE.md",
        ".aitignore",
        ".ait/config.json",
        ".ait/policy.yaml",
        ".ait/binary-db/plan_revision.bin",
    ];
    let before = paths.map(|p| saved(&dir.path().join(p)));
    let report = complete(&repo).unwrap();
    assert_eq!(report["mutated"], json!(false));
    assert_eq!(before, paths.map(|p| saved(&dir.path().join(p))));
}

#[test]
fn partial_setup_and_interruption_retry_preserve_completed_files() {
    let (dir, repo) = fixture();
    for path in [".aitignore", "AGENTS.md", "CLAUDE.md"] {
        fs::remove_file(dir.path().join(path)).unwrap();
    }
    fs::remove_dir(dir.path().join("docs/sprints")).unwrap();
    let error = complete_with_observer(&repo, |path| {
        if path == ".aitignore" {
            Err("injected interruption after durable file".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(error.contains("injected interruption"));
    let persisted = saved(&dir.path().join(".aitignore"));
    assert!(!dir.path().join("AGENTS.md").exists());
    let report = complete(&repo).unwrap();
    assert_eq!(persisted, saved(&dir.path().join(".aitignore")));
    assert_eq!(report["guidance_plan_sync_required"], json!(true));
    assert!(dir.path().join("docs/sprints").is_dir());
    assert!(dir.path().join("AGENTS.md").is_file());
    assert!(dir.path().join("CLAUDE.md").is_file());
    assert!(!dir.path().join("ci").exists());
    assert_eq!(complete(&repo).unwrap()["mutated"], json!(false));
}

#[test]
fn invalid_settings_are_diagnosed_without_reset_or_partial_creation() {
    for (path, bytes) in [
        (
            "AGENTS.md",
            b"custom\n<!-- ait:workflow:start -->\n".as_slice(),
        ),
        (".aitignore", b"\xff".as_slice()),
        ("CLAUDE.md", b"\xff".as_slice()),
    ] {
        let (dir, repo) = fixture();
        fs::write(dir.path().join(path), bytes).unwrap();
        let before = saved(&dir.path().join(path));
        assert!(preflight(&repo).is_err(), "{path}");
        assert!(complete(&repo).is_err(), "{path}");
        assert_eq!(before, saved(&dir.path().join(path)));
        assert!(!dir.path().join("ci").exists());
    }
}

#[test]
fn conflicting_directory_and_file_kinds_are_not_replaced() {
    for path in [".aitignore", "AGENTS.md", "docs/sprints"] {
        let (dir, repo) = fixture();
        let p = dir.path().join(path);
        if path != "docs/sprints" {
            fs::remove_file(&p).unwrap();
            fs::create_dir(&p).unwrap();
        } else {
            if p.is_dir() {
                fs::remove_dir(&p).unwrap();
            }
            fs::write(&p, b"user file").unwrap();
        }
        assert!(preflight(&repo).is_err(), "{path}");
        assert!(complete(&repo).is_err(), "{path}");
        if path != "docs/sprints" {
            assert!(p.is_dir());
        } else {
            assert_eq!(fs::read(&p).unwrap(), b"user file");
        }
    }
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_rejected_without_writing_their_targets() {
    use std::os::unix::fs::symlink;
    for path in [".aitignore", "AGENTS.md", "docs"] {
        let (dir, repo) = fixture();
        let outside = TempDir::new().unwrap();
        let sentinel = outside.path().join("sentinel");
        fs::write(&sentinel, b"private user bytes").unwrap();
        let p = dir.path().join(path);
        if path == "docs" {
            fs::remove_dir_all(&p).unwrap();
        } else if p.is_file() {
            fs::remove_file(&p).unwrap();
        }
        symlink(
            if path == "docs" {
                outside.path()
            } else {
                &sentinel
            },
            &p,
        )
        .unwrap();
        assert!(preflight(&repo).is_err(), "{path}");
        assert!(complete(&repo).is_err(), "{path}");
        assert_eq!(fs::read(&sentinel).unwrap(), b"private user bytes");
        assert!(fs::symlink_metadata(p).unwrap().file_type().is_symlink());
    }
}

#[test]
fn malformed_user_ci_is_preserved_and_is_not_an_import_requirement() {
    for bytes in [b"not JSON".as_slice(), b"{}", b"\xff"] {
        let (dir, repo) = fixture();
        fs::create_dir(dir.path().join("ci")).unwrap();
        let path = dir.path().join("ci/patch_ci.json");
        fs::write(&path, bytes).unwrap();
        let before = saved(&path);
        assert_eq!(complete(&repo).unwrap()["mutated"], json!(false));
        assert_eq!(before, saved(&path));
    }
}

#[test]
fn concurrent_no_clobber_writes_keep_one_complete_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("settings");
    let written = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            init_surface::write_workspace_file_if_absent(&path, b"first complete body", 0o600)
                .unwrap()
        });
        let b = scope.spawn(|| {
            init_surface::write_workspace_file_if_absent(&path, b"second complete body", 0o600)
                .unwrap()
        });
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(written.into_iter().filter(|v| *v).count(), 1);
    assert!([
        b"first complete body".as_slice(),
        b"second complete body".as_slice()
    ]
    .contains(&fs::read(&path).unwrap().as_slice()));
}
