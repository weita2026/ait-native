use super::{
    config_set, config_show, normalize_id_namespace_prefix_value, ConfigSetRequest,
    DEFAULT_ID_NAMESPACE_PREFIX,
};
use crate::init_surface::{init_repo, InitRequest};
use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_object_or_empty};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonValue};
use tempfile::TempDir;

#[test]
fn default_id_namespace_prefix_is_blank() {
    assert_eq!(DEFAULT_ID_NAMESPACE_PREFIX, "");
    assert_eq!(normalize_id_namespace_prefix_value("").unwrap(), "");
    assert_eq!(normalize_id_namespace_prefix_value("ait").unwrap(), "AIT");
}

#[test]
fn repository_index_config_is_numeric_exclusive_and_clearable() {
    let (_temp, repo) = initialized_repo();

    let set = config_set(
        &repo,
        &ConfigSetRequest {
            repository_index: Some(7),
            ..ConfigSetRequest::default()
        },
    )
    .expect("set numeric repository index");
    assert_eq!(set["repository_index"], 7);

    let refreshed = RepoRuntime::discover_from_path(&repo.root).expect("refreshed runtime");
    assert_eq!(
        refreshed
            .require_repository_index()
            .expect("configured repository index")
            .get(),
        7
    );

    let conflict = config_set(
        &refreshed,
        &ConfigSetRequest {
            repository_index: Some(8),
            clear_repository_index: true,
            ..ConfigSetRequest::default()
        },
    )
    .expect_err("set and clear must be mutually exclusive");
    assert!(conflict.contains("--repository-index"));

    let cleared = config_set(
        &refreshed,
        &ConfigSetRequest {
            clear_repository_index: true,
            ..ConfigSetRequest::default()
        },
    )
    .expect("clear repository index");
    assert!(cleared["repository_index"].is_null());
    let cleared_runtime =
        RepoRuntime::discover_from_path(&repo.root).expect("cleared repository runtime");
    assert!(cleared_runtime.require_repository_index().is_err());
}

#[test]
fn retired_task_dag_config_is_inert_and_preserved() {
    let (_temp, repo) = initialized_repo();
    let config_path = repo.root.join(".ait/config.json");
    let mut raw_config = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    let legacy_task_dag = json!({"allow_multi_worker": true});
    raw_config.insert("task_dag".to_string(), legacy_task_dag.clone());
    std::fs::write(
        &config_path,
        encode_value_pretty_with_newline_error_string(&JsonValue::Object(raw_config)).unwrap(),
    )
    .unwrap();

    let repo_with_legacy_config =
        RepoRuntime::discover_from_path(&repo.root).expect("runtime with legacy config");
    assert!(config_show(&repo_with_legacy_config)
        .unwrap()
        .get("task_dag")
        .is_none());

    let updated = config_set(
        &repo_with_legacy_config,
        &ConfigSetRequest {
            default_model: Some("test-model".to_string()),
            ..ConfigSetRequest::default()
        },
    )
    .expect("unrelated config update");
    assert!(updated.get("task_dag").is_none());
    let preserved = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    assert_eq!(preserved.get("task_dag"), Some(&legacy_task_dag));
}

fn initialized_repo() -> (TempDir, RepoRuntime) {
    let temp = TempDir::new().unwrap();
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("demo".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    (temp, repo)
}

#[test]
#[cfg(unix)]
fn config_show_separates_current_worktree_from_active_root_pointer() {
    use std::os::unix::fs::symlink;

    let (temp, _) = initialized_repo();
    let root = temp.path();
    let config_path = root.join(".ait/config.json");
    let mut root_config = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    root_config.insert(
        "worktree_name".to_string(),
        JsonValue::String("rct-active".to_string()),
    );
    std::fs::write(
        &config_path,
        encode_value_pretty_with_newline_error_string(&JsonValue::Object(root_config)).unwrap(),
    )
    .unwrap();

    let root_repo = RepoRuntime::discover_from_path(root).unwrap();
    let root_payload = config_show(&root_repo).unwrap();
    assert!(root_payload["worktree_name"].is_null());
    assert_eq!(root_payload["active_root_worktree_name"], "rct-active");

    let worktree = root.join("rct-current");
    std::fs::create_dir_all(&worktree).unwrap();
    symlink(root.join(".ait"), worktree.join(".ait")).unwrap();
    std::fs::write(
        worktree.join(".ait-worktree.json"),
        format!(
            "{{\"repo_root\":\"{}\",\"workspace_root\":\"{}\",\"worktree_name\":\"rct-current\",\"current_line\":\"feature/rct-current\"}}\n",
            root.display(),
            worktree.display()
        ),
    )
    .unwrap();
    let worktree_repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let worktree_payload = config_show(&worktree_repo).unwrap();
    assert_eq!(worktree_payload["worktree_name"], "rct-current");
    assert_eq!(worktree_payload["active_root_worktree_name"], "rct-active");
}

#[test]
fn config_set_sprint_off_disables_plan_task_binding() {
    let (temp, repo) = initialized_repo();

    let payload = config_set(
        &repo,
        &ConfigSetRequest {
            sprint: Some("off".to_string()),
            ..ConfigSetRequest::default()
        },
    )
    .unwrap();

    assert_eq!(payload["sprint"]["value"], "off");
    assert_eq!(payload["sprint"]["plan_task_binding_mode"], "off");
    assert_eq!(payload["plan_task_binding"]["mode"], "off");
    assert_eq!(payload["agent_harness"]["status"], "synced");
    assert_eq!(payload["agent_harness"]["scope"], "local");
    assert_eq!(payload["agent_harness"]["plan_sync"]["status"], "ok");
    let refreshed = RepoRuntime::discover_from_path(&repo.root).unwrap();
    assert_eq!(refreshed.config["sprint"], "off");
    assert_eq!(refreshed.config["plan_task_binding"]["mode"], "off");
    assert_eq!(refreshed.effective_workflow_mode(), "solo_local");
    let agents = std::fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
    assert!(agents.contains("workflow mode: `solo_local`"));
    assert!(agents.contains("sprint mode: `off`"));
    assert!(agents.contains("sprint card is not required"));
    assert!(agents.contains("`--from` is unavailable"));
    assert!(temp.path().join("docs/sprints").is_dir());
}

#[test]
fn config_set_workflow_mode_sprint_on_requires_plan_task_binding() {
    let (temp, repo) = initialized_repo();
    config_set(
        &repo,
        &ConfigSetRequest {
            sprint: Some("off".to_string()),
            ..ConfigSetRequest::default()
        },
    )
    .unwrap();
    std::fs::remove_dir(temp.path().join("docs/sprints")).unwrap();
    std::fs::remove_dir(temp.path().join("docs")).unwrap();
    let repo = RepoRuntime::discover_from_path(&repo.root).unwrap();

    let payload = config_set(
        &repo,
        &ConfigSetRequest {
            workflow_mode: Some("solo_remote".to_string()),
            sprint: Some("on".to_string()),
            ..ConfigSetRequest::default()
        },
    )
    .unwrap();

    assert_eq!(payload["workflow_mode"]["value"], "solo_remote");
    assert_eq!(payload["sprint"]["value"], "on");
    assert_eq!(payload["sprint"]["plan_task_binding_mode"], "required");
    assert_eq!(payload["plan_task_binding"]["mode"], "required");
    assert_eq!(payload["agent_harness"]["status"], "pending");
    assert_eq!(payload["agent_harness"]["reason"], "no_default_remote");
    let refreshed = RepoRuntime::discover_from_path(&repo.root).unwrap();
    assert_eq!(refreshed.config["workflow_mode"], "solo_remote");
    assert_eq!(refreshed.config["sprint"], "on");
    assert_eq!(refreshed.config["plan_task_binding"]["mode"], "required");
    let agents = std::fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
    assert!(agents.contains("workflow mode: `solo_remote`"));
    assert!(agents.contains("sprint mode: `on`"));
    assert!(agents.contains("ait plan sync <markdown-file-or-dir> --remote origin"));
    assert!(agents.contains("ait task start --from"));
    assert!(agents.contains("owns exact-file Plan sync"));
    assert!(!agents.contains("--plan-item-ref"));
    assert!(agents.contains("After every context-window compaction, re-read the bound sprint card"));
    assert!(agents.contains("patchset publication"));
    assert!(temp.path().join("docs/sprints").is_dir());
}
