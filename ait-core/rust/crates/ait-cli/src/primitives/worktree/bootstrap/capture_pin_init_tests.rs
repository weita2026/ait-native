use super::*;
use crate::init_surface::{init_repo, InitRequest};
use ait_core::local_snapshot::LocalSnapshotReadStore;
use tempfile::TempDir;

fn fixture() -> (TempDir, RepoRuntime, RemoteRow, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("repo");
    fs::create_dir(&root).unwrap();
    init_repo(&InitRequest {
        root: root.clone(),
        name: Some("capture-pin".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    fs::write(root.join("code.rs"), "fn baseline() {}\n").unwrap();
    let config_path = root.join(".ait/config.json");
    let mut config = read_json_object_value(&config_path);
    config.insert("repository_index".into(), json!(7));
    config.insert("default_remote".into(), json!("a"));
    config.insert("id_namespace_prefix".into(), json!(""));
    config.insert(
        "remotes".into(),
        json!({
            "a": {"remote_id": 1, "url": "https://a.test", "repo_name": "capture-pin"},
            "b": {"remote_id": 2, "url": "https://b.test///", "repo_name": "capture-pin"},
        }),
    );
    write_json_pretty(&config_path, &JsonValue::Object(config)).unwrap();
    let repo = RepoRuntime::discover_from_path(&root).unwrap();
    ensure_task_start_local_base_snapshot(&repo, "main").unwrap();
    let remote = repo.remote_row(Some("b")).unwrap();
    let edit = temp.path().canonicalize().unwrap().join("edit");
    (temp, repo, remote, edit)
}

fn start(
    repo: &RepoRuntime,
    remote: Option<&RemoteRow>,
    edit: &Path,
    progress: Option<&mut TaskStartProgressEmitter<'_>>,
) -> Result<JsonValue, String> {
    let base = local_line_head_snapshot_id(repo, "main")?;
    task_start_bootstrap_created_records_with_progress(
        repo,
        json!({"task_id": "RT-1", "repo_name": "capture-pin", "status": "active"}),
        None,
        "main",
        remote.is_none(),
        remote,
        remote.and(base.as_deref()),
        Some(edit),
        None,
        progress,
    )
}

fn snapshots(repo: &RepoRuntime) -> JsonValue {
    repo.local_snapshot_operation_store::<1>(&repo.workspace_root())
        .unwrap()
        .list_snapshots()
        .unwrap()
}

#[test]
fn remote_pin_is_ready_before_success_progress_and_exact_retry_reuses_it() {
    let (_temp, repo, remote, edit) = fixture();
    let before = snapshots(&repo);
    let expected = json!({"remote_name": "b", "repository_index": 7});
    let mut ready = false;
    let mut progress = |event: &JsonValue| {
        if event["phase"].as_str() == Some("worktree_ready") {
            let name = event["worktree_name"].as_str().unwrap();
            assert_eq!(
                worktree_show(&repo, Some(name), false).unwrap()["capture_authority"],
                expected
            );
            ready = true;
        }
        Ok(())
    };
    let result = start(&repo, Some(&remote), &edit, Some(&mut progress)).unwrap();
    assert!(ready);
    assert!(result["change"].is_null());
    let name = result["worktree"]["name"].as_str().unwrap();
    let stored = load_worktree_metadata(&repo, name).unwrap();
    assert_eq!(stored["capture_remote_name"], json!("b"));
    assert_eq!(stored["capture_remote_url"], json!("https://b.test"));
    assert_eq!(stored["capture_repository_index"], json!(7));
    let retry = start(&repo, Some(&remote), &edit, None).unwrap();
    assert_eq!(retry["task_id"], result["task_id"]);
    assert_eq!(retry["worktree_reused"], json!(true));
    assert_eq!(snapshots(&repo), before);
}

#[test]
fn local_start_keeps_capture_authority_null() {
    let (_temp, repo, _remote, edit) = fixture();
    let result = start(&repo, None, &edit, None).unwrap();
    let name = result["worktree"]["name"].as_str().unwrap();
    assert_eq!(
        worktree_show(&repo, Some(name), false).unwrap()["capture_authority"],
        JsonValue::Null
    );
}

#[test]
fn local_retry_cannot_adopt_a_remote_bound_worktree() {
    let (_temp, repo, remote, edit) = fixture();
    let result = start(&repo, Some(&remote), &edit, None).unwrap();
    let name = result["worktree"]["name"].as_str().unwrap();
    let before = fs::read(worktree_registry_path(&repo, name)).unwrap();
    assert!(start(&repo, None, &edit, None)
        .unwrap_err()
        .contains("cannot be reused for Local work"));
    assert_eq!(
        fs::read(worktree_registry_path(&repo, name)).unwrap(),
        before
    );
}

#[test]
fn retry_never_retargets_repairs_or_backfills_an_existing_pin() {
    for (field, value) in [
        ("capture_remote_name", json!("a")),
        ("capture_remote_url", json!("https://a.test")),
        ("capture_repository_index", json!(8)),
        ("capture_remote_url", JsonValue::Null),
        ("absent", JsonValue::Null),
    ] {
        let (_temp, repo, remote, edit) = fixture();
        let result = start(&repo, Some(&remote), &edit, None).unwrap();
        let name = result["worktree"]["name"].as_str().unwrap();
        let mut metadata = load_worktree_metadata(&repo, name).unwrap();
        if field == "absent" {
            for key in [
                "capture_remote_name",
                "capture_remote_url",
                "capture_repository_index",
            ] {
                metadata.remove(key);
            }
        } else {
            metadata.insert(field.into(), value);
        }
        save_worktree_metadata(&repo, name, &metadata).unwrap();
        let before = fs::read(worktree_registry_path(&repo, name)).unwrap();
        let error = start(&repo, Some(&remote), &edit, None).unwrap_err();
        assert!(error.contains("Task RT-1"), "{error}");
        assert!(error.contains("cannot be reused"), "{error}");
        assert_eq!(
            fs::read(worktree_registry_path(&repo, name)).unwrap(),
            before
        );
    }
}

#[test]
fn retry_rejects_a_changed_or_missing_configured_remote() {
    for missing in [false, true] {
        let (_temp, repo, remote, edit) = fixture();
        let result = start(&repo, Some(&remote), &edit, None).unwrap();
        let name = result["worktree"]["name"].as_str().unwrap();
        let before = fs::read(worktree_registry_path(&repo, name)).unwrap();
        let path = repo.ait_dir.join("config.json");
        let mut config = read_json_value(&path);
        if missing {
            config["remotes"].as_object_mut().unwrap().remove("b");
        } else {
            config["remotes"]["b"]["url"] = json!("https://changed.test");
        }
        write_json_pretty(&path, &config).unwrap();
        assert!(start(&repo, Some(&remote), &edit, None).is_err());
        assert_eq!(
            fs::read(worktree_registry_path(&repo, name)).unwrap(),
            before
        );
    }
}

#[test]
fn retry_rejects_another_remote_name_even_with_the_same_endpoint_and_index() {
    let (_temp, repo, remote, edit) = fixture();
    let result = start(&repo, Some(&remote), &edit, None).unwrap();
    let name = result["worktree"]["name"].as_str().unwrap();
    let before = fs::read(worktree_registry_path(&repo, name)).unwrap();
    let path = repo.ait_dir.join("config.json");
    let mut config = read_json_value(&path);
    config["remotes"]["a"]["url"] = json!(remote.url);
    write_json_pretty(&path, &config).unwrap();
    let other = repo.remote_row(Some("a")).unwrap();
    let error = start(&repo, Some(&other), &edit, None).unwrap_err();
    assert!(error.contains("cannot be reused"), "{error}");
    assert_eq!(
        fs::read(worktree_registry_path(&repo, name)).unwrap(),
        before
    );
}

#[test]
fn acknowledged_task_setup_failure_preserves_identity_and_foreign_workspace() {
    let (_temp, repo, remote, edit) = fixture();
    fs::create_dir(&edit).unwrap();
    fs::write(edit.join("foreign.txt"), "preserve me").unwrap();
    let before = snapshots(&repo);
    let error = start(&repo, Some(&remote), &edit, None).unwrap_err();
    assert!(error.contains("Task RT-1 was created"), "{error}");
    assert_eq!(
        fs::read_to_string(edit.join("foreign.txt")).unwrap(),
        "preserve me"
    );
    assert_eq!(snapshots(&repo), before);
    assert!(!worktree_registry_path(&repo, "rt-1").exists());
    fs::remove_file(edit.join("foreign.txt")).unwrap();
    let retry = start(&repo, Some(&remote), &edit, None).unwrap();
    assert_eq!(retry["task_id"], json!("RT-1"));
    assert!(retry["change"].is_null());
}

#[test]
fn changed_target_during_setup_cannot_return_ready_or_repin_on_retry() {
    let (_temp, repo, remote, edit) = fixture();
    let mut ready = false;
    let mut progress = |event: &JsonValue| {
        if event["phase"].as_str() == Some("materializing_worktree") {
            let path = repo.ait_dir.join("config.json");
            let mut config = read_json_value(&path);
            config["remotes"]["b"]["url"] = json!("https://changed.test");
            write_json_pretty(&path, &config).unwrap();
        }
        ready |= event["phase"].as_str() == Some("worktree_ready");
        Ok(())
    };
    let error = start(&repo, Some(&remote), &edit, Some(&mut progress)).unwrap_err();
    assert!(error.contains("Task RT-1 was created"), "{error}");
    assert!(!ready);
    let path = repo.ait_dir.join("config.json");
    let mut config = read_json_value(&path);
    config["remotes"]["b"]["url"] = json!(remote.url);
    write_json_pretty(&path, &config).unwrap();
    assert!(start(&repo, Some(&remote), &edit, None)
        .unwrap_err()
        .contains("supported recovery"));
}

#[test]
fn remote_initial_bootstrap_requires_verified_base_and_local_scope_rejects_remote_input() {
    for local in [false, true] {
        let (_temp, repo, remote, edit) = fixture();
        let base = local_line_head_snapshot_id(&repo, "main").unwrap().unwrap();
        let before = snapshots(&repo);
        let error = task_start_bootstrap_created_records_with_progress(
            &repo,
            json!({"task_id":"RT-1","repo_name":"capture-pin","status":"active"}),
            None,
            "main",
            local,
            if local { None } else { Some(&remote) },
            if local { Some(base.as_str()) } else { None },
            Some(&edit),
            None,
            None,
        )
        .unwrap_err();
        assert!(
            error.contains(if local {
                "Local Task bootstrap cannot use a Remote base"
            } else {
                "Remote Task bootstrap requires its verified base"
            }),
            "{error}"
        );
        assert!(!edit.exists());
        assert!(!worktree_registry_path(&repo, "rt-1").exists());
        assert!(local_line_row(&repo, "feature/rt-1").is_err());
        assert_eq!(snapshots(&repo), before);
    }
}
