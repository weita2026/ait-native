use super::*;
use crate::init_surface::{init_repo, InitRequest};
use ait_core::task_store::TaskStore;
use fs2::FileExt;
use std::{fs::OpenOptions, os::unix::fs::symlink};
use tempfile::{tempdir, TempDir};

struct Fixture {
    _temp: TempDir,
    root: RepoRuntime,
    worktree: RepoRuntime,
    task: String,
    name: String,
    alias: PathBuf,
}

impl Fixture {
    fn new(dirty: bool) -> Self {
        let temp = tempdir().unwrap();
        let root_path = temp.path().join("canonical");
        fs::create_dir(&root_path).unwrap();
        init_repo(&InitRequest {
            root: root_path.clone(),
            name: Some("abandon-fixture".into()),
            default_line: "main".into(),
            policy_profile: "prototype".into(),
            default_author_mode: "ai_with_human_review".into(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let root = RepoRuntime::discover_from_path(&root_path).unwrap();
        let task = TaskStore::create_task(
            &root.task_store().unwrap(),
            &root.repo_name(),
            "Discard owned scratch work",
            "Exercise native abandonment",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let task = required_string_field(&task, "task_id").unwrap();
        let line = format!("feature/{task}");
        create_local_line(&root, &line, None).unwrap();
        let name = task_bound_worktree_name(&task).unwrap();
        // Physical path is independent; native registry supplies Task ownership.
        let path = root.root.join(".ait-worktree/independent-directory");
        fs::create_dir_all(&path).unwrap();
        symlink(root.root.join(APP_DIR), path.join(APP_DIR)).unwrap();
        fs::write(
            path.join(WORKTREE_CONFIG_NAME),
            json!({"repo_root":root.root,
            "workspace_root":path,"worktree_name":name,"current_line":line,
            "rebase_state":"idle","merge_state":"idle"})
            .to_string(),
        )
        .unwrap();
        fs::write(root.root.join("docs/shared.txt"), "shared authority").unwrap();
        symlink(root.root.join("docs"), path.join("docs")).unwrap();
        let alias = root.root.join(".ait-worktree-links").join(&name);
        fs::create_dir_all(alias.parent().unwrap()).unwrap();
        symlink(&path, &alias).unwrap();
        write_worktree_registration(
            &root,
            &name,
            &path,
            Some(&alias),
            &line,
            "2026-10-23T00:00:00Z",
            Some("repo_internal_fallback"),
            &task,
            None,
            None,
            None,
            Some("main"),
            Some("main"),
        )
        .unwrap();
        if dirty {
            fs::write(path.join("uncaptured.txt"), "discard me").unwrap();
        }
        let worktree = RepoRuntime::discover_from_path(&path).unwrap();
        Self {
            _temp: temp,
            root,
            worktree,
            task,
            name,
            alias,
        }
    }

    fn metadata(&self, key: &str, value: JsonValue) {
        let path = worktree_registry_path(&self.root, &self.name);
        let mut row = read_json_value(&path);
        row[key] = value;
        fs::write(path, row.to_string()).unwrap();
    }

    fn overlay(&self, key: &str, value: JsonValue) {
        let path = self.worktree.root.join(WORKTREE_CONFIG_NAME);
        let mut row = read_json_value(&path);
        row[key] = value;
        fs::write(path, row.to_string()).unwrap();
    }

    fn task_record(&self) -> JsonValue {
        TaskStore::get_task(&self.root.task_store().unwrap(), &self.task).unwrap()
    }

    fn cache(&self, target: bool) -> PathBuf {
        let family = if target {
            "cargo-target"
        } else {
            "cargo-build"
        };
        self.root
            .root
            .join(APP_DIR)
            .join(family)
            .join("task-workspaces")
            .join(&self.name)
    }

    fn close_only(&self) {
        TaskStore::close_task(&self.root.task_store().unwrap(), &self.task, "abandoned").unwrap();
    }

    fn assert_removed(&self) {
        assert!(!self.worktree.root.exists());
        assert!(fs::symlink_metadata(&self.alias).is_err());
        assert!(!worktree_registry_path(&self.root, &self.name).exists());
        assert!(!self.cache(false).exists());
        assert!(!self.cache(true).exists());
        assert_eq!(
            fs::read_to_string(self.root.root.join("docs/shared.txt")).unwrap(),
            "shared authority"
        );
        assert!(self.root.root.join(APP_DIR).is_dir());
        assert_eq!(self.task_record()["status"], "abandoned");
        assert!(self.task_record()["local_finish"].is_null());
    }
}

#[test]
fn clean_and_dirty_owned_roots_are_removed_from_canonical_or_target_context() {
    for dirty in [false, true] {
        for from_target in [false, true] {
            let f = Fixture::new(dirty);
            f.metadata(
                "bound_task_status",
                json!(if dirty { "active" } else { "completed" }),
            );
            for path in [f.cache(false), f.cache(true)] {
                fs::create_dir_all(&path).unwrap();
                fs::write(path.join("artifact"), "owned").unwrap();
            }
            let shared = f.root.root.join(".ait/cargo-target/shared");
            fs::create_dir_all(&shared).unwrap();
            fs::write(shared.join("keep"), "shared").unwrap();
            let payload = task_abandon(
                if from_target { &f.worktree } else { &f.root },
                &f.task,
                true,
                None,
            )
            .unwrap();
            assert_eq!(payload["bound_worktree_cleanup"]["status"], "removed");
            f.assert_removed();
            assert!(shared.join("keep").is_file());
            let closed = f.task_record();
            let tasks = TaskStore::list_tasks(&f.root.task_store().unwrap()).unwrap();
            let retried = task_abandon(&f.root, &f.task, true, None).unwrap();
            assert_eq!(
                retried["bound_worktree_cleanup"]["status"],
                "already_removed"
            );
            assert_eq!(f.task_record(), closed);
            assert_eq!(
                TaskStore::list_tasks(&f.root.task_store().unwrap()).unwrap(),
                tasks
            );
        }
    }
}

#[test]
fn conflicting_overlay_registry_and_foreign_repository_hold_before_task_close() {
    for conflict in ["overlay", "registry", "authority", "alias"] {
        let f = Fixture::new(true);
        let other = TaskStore::create_task(
            &f.root.task_store().unwrap(),
            &f.root.repo_name(),
            "Other owner",
            "Keep it",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        match conflict {
            "overlay" => f.overlay("bound_task_id", other["task_id"].clone()),
            "registry" => f.metadata("bound_task_id", other["task_id"].clone()),
            "authority" => f.metadata("repo_root", json!(f.root.root.join("docs"))),
            "alias" => {
                fs::remove_file(&f.alias).unwrap();
                symlink(f.root.root.join("docs"), &f.alias).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            task_abandon(&f.root, &f.task, true, None).is_err(),
            "{conflict}"
        );
        assert_eq!(f.task_record()["status"], "active");
        assert!(f.worktree.root.join("uncaptured.txt").is_file());
        assert_eq!(
            TaskStore::get_task(
                &f.root.task_store().unwrap(),
                other["task_id"].as_str().unwrap()
            )
            .unwrap()["status"],
            "active"
        );
    }
}

#[test]
fn explicit_manual_and_merge_rebase_guards_remain_visible_after_close() {
    for guard in ["explicit", "manual", "merge", "rebase"] {
        let f = Fixture::new(true);
        match guard {
            "explicit" => f.metadata("root_source", json!("explicit")),
            "manual" => f.metadata("cleanup_policy", json!("manual_only")),
            "merge" => f.metadata("merge_state", json!("conflicted")),
            "rebase" => f.overlay("rebase_state", json!("conflicted")),
            _ => unreachable!(),
        }
        let error = task_abandon(&f.root, &f.task, true, None).unwrap_err();
        assert!(
            error.contains("is abandoned") && error.contains("cleanup is incomplete"),
            "{error}"
        );
        assert_eq!(f.task_record()["status"], "abandoned");
        assert!(f.worktree.root.join("uncaptured.txt").is_file());
        let closed = f.task_record();
        assert!(task_abandon(&f.root, &f.task, true, None).is_err());
        assert_eq!(f.task_record(), closed);
    }
}

#[test]
fn live_writer_lock_blocks_logical_close_without_trusting_metadata_age() {
    let f = Fixture::new(true);
    let writer = WorkspaceCommandLock::acquire(&f.worktree, "competing writer").unwrap();
    let lock_path = crate::workspace_lock::workspace_command_lock_path(&f.worktree);
    let mut stale_metadata = read_json_value(&lock_path);
    stale_metadata["started_at"] = json!("0");
    fs::write(lock_path, stale_metadata.to_string()).unwrap();
    let error = task_abandon(&f.root, &f.task, true, None).unwrap_err();
    assert!(error.contains("writer lock"), "{error}");
    assert_eq!(f.task_record()["status"], "active");
    assert!(f.worktree.root.is_dir());
    drop(writer);
    task_abandon(&f.root, &f.task, true, None).unwrap();
    f.assert_removed();
}

#[test]
fn cargo_process_blocks_cleanup_after_close_and_retry_only_finishes_cleanup() {
    let f = Fixture::new(true);
    let cache = f.cache(false);
    fs::create_dir_all(&cache).unwrap();
    let lease = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(cache.join(".ait-ci-build-lease"))
        .unwrap();
    lease.lock_exclusive().unwrap();
    let error = task_abandon(&f.root, &f.task, true, None).unwrap_err();
    assert!(
        error.contains("cleanup is incomplete") && error.contains("active"),
        "{error}"
    );
    assert_eq!(f.task_record()["status"], "abandoned");
    assert!(f.worktree.root.join("uncaptured.txt").exists());
    let closed = f.task_record();
    FileExt::unlock(&lease).unwrap();
    task_abandon(&f.root, &f.task, true, None).unwrap();
    f.assert_removed();
    assert_eq!(f.task_record(), closed);
}

#[test]
fn interrupted_cleanup_recovers_from_owner_overlay_empty_root_and_missing_root() {
    for phase in [
        "overlay",
        "authority_link_only",
        "empty_root",
        "missing_root",
    ] {
        let f = Fixture::new(true);
        f.close_only();
        for cache in [f.cache(false), f.cache(true)] {
            fs::create_dir_all(&cache).unwrap();
            fs::write(cache.join("unfinished"), "owned cache").unwrap();
        }
        fs::remove_file(f.worktree.root.join("uncaptured.txt")).unwrap();
        fs::remove_file(f.worktree.root.join("docs")).unwrap();
        if phase != "overlay" {
            fs::remove_file(f.worktree.root.join(WORKTREE_CONFIG_NAME)).unwrap();
        }
        if matches!(phase, "empty_root" | "missing_root") {
            fs::remove_file(f.worktree.root.join(APP_DIR)).unwrap();
        }
        if phase == "missing_root" {
            fs::remove_dir(&f.worktree.root).unwrap();
        }
        let closed = f.task_record();
        task_abandon(&f.root, &f.task, true, None).unwrap();
        f.assert_removed();
        assert_eq!(f.task_record(), closed);
    }
}

#[test]
fn changed_owner_after_close_and_other_terminal_outcomes_never_delete_roots() {
    let f = Fixture::new(true);
    let prepared = prepare(&f.root, &f.root, &f.task).unwrap();
    f.close_only();
    f.overlay("bound_task_id", json!("LCT-9999"));
    assert!(prepared.finish(&f.root).is_err());
    assert!(f.worktree.root.join("uncaptured.txt").exists());
    drop(prepared);
    for status in ["completed", "canceled"] {
        let f = Fixture::new(true);
        TaskStore::close_task(&f.root.task_store().unwrap(), &f.task, status).unwrap();
        assert!(task_abandon(&f.root, &f.task, true, None).is_err());
        assert_eq!(f.task_record()["status"], status);
        assert!(f.worktree.root.join("uncaptured.txt").is_file());
    }
}
