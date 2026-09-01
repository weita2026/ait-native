use super::{
    config_set, config_set_from_payload, config_show, config_unset,
    normalize_id_namespace_prefix_value, ConfigSetRequest, ConfigUnsetKey,
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
fn repository_index_is_read_only_and_removed_payload_fields_fail_before_mutation() {
    let (_temp, repo) = initialized_repo();
    let config_path = repo.root.join(".ait/config.json");
    let mut raw_config = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    raw_config.insert("repository_index".to_string(), json!(7));
    raw_config.insert("task_tracking".to_string(), json!("on"));
    raw_config.insert("command_profiling".to_string(), json!("on"));
    std::fs::write(
        &config_path,
        encode_value_pretty_with_newline_error_string(&JsonValue::Object(raw_config)).unwrap(),
    )
    .unwrap();
    let repo = RepoRuntime::discover_from_path(&repo.root).unwrap();

    let shown = config_show(&repo).unwrap();
    assert_eq!(shown["repository_index"], 7);
    assert!(shown.get("task_tracking").is_none());
    assert!(shown.get("command_profiling").is_none());

    let removed_fields = [
        ("repository_index", json!(8)),
        ("clear_repository_index", json!(true)),
        ("clear_default_author_mode", json!(true)),
        ("clear_default_model", json!(true)),
        ("task_tracking", json!("off")),
        ("clear_task_review", json!(true)),
        ("command_profiling", json!("off")),
        ("clear_task_worktree_alias_root", json!(true)),
        ("clear_task_worktree_main_seed_ram_max_bytes", json!(true)),
        ("legacy_task_auto_worktree", json!("off")),
        ("legacy_clear_task_auto_worktree", json!(true)),
        ("workflow_default_scope", json!("remote")),
        ("clear_workflow_default_scope", json!(true)),
        ("task_default_scope", json!("remote")),
        ("clear_task_default_scope", json!(true)),
        ("change_default_scope", json!("remote")),
        ("clear_change_default_scope", json!(true)),
        ("clear_id_namespace_prefix", json!(true)),
        ("plan_task_binding_mode", json!("advisory")),
        ("clear_plan_task_binding", json!(true)),
        ("clear_user_name", json!(true)),
        ("clear_user_email", json!(true)),
        ("unknown_field", json!(true)),
    ];
    for (field, value) in removed_fields {
        let before = std::fs::read(&config_path).unwrap();
        let mut payload = ait_core::json_support::JsonMap::new();
        payload.insert(field.to_string(), value);
        let error = config_set_from_payload(&repo, &JsonValue::Object(payload))
            .expect_err("removed config payload field must fail");
        assert!(error.contains("retired or unknown"), "{field}: {error}");
        assert_eq!(std::fs::read(&config_path).unwrap(), before, "{field}");
    }
}

#[test]
fn unknown_config_extension_is_inert_and_preserved() {
    let (_temp, repo) = initialized_repo();
    let config_path = repo.root.join(".ait/config.json");
    let mut raw_config = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    let extension_value = json!({"keep": true});
    raw_config.insert("future_extension".to_string(), extension_value.clone());
    std::fs::write(
        &config_path,
        encode_value_pretty_with_newline_error_string(&JsonValue::Object(raw_config)).unwrap(),
    )
    .unwrap();

    let repo_with_extension =
        RepoRuntime::discover_from_path(&repo.root).expect("runtime with unknown extension");
    assert!(config_show(&repo_with_extension)
        .unwrap()
        .get("future_extension")
        .is_none());

    let updated = config_set(
        &repo_with_extension,
        &ConfigSetRequest {
            default_model: Some("test-model".to_string()),
            ..ConfigSetRequest::default()
        },
    )
    .expect("unrelated config update");
    assert!(updated.get("future_extension").is_none());
    let preserved = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    assert_eq!(preserved.get("future_extension"), Some(&extension_value));
}

#[test]
fn admitted_overrides_set_and_unset_without_erasing_inert_legacy_data() {
    let (_temp, repo) = initialized_repo();
    let config_path = repo.root.join(".ait/config.json");
    let mut raw_config = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    raw_config.insert("legacy_config_marker".to_string(), json!({"keep": true}));
    let mut task_worktree = raw_config
        .get("task_worktree")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    task_worktree.insert("legacy_nested_marker".to_string(), json!([1, 2, 3]));
    raw_config.insert(
        "task_worktree".to_string(),
        JsonValue::Object(task_worktree),
    );
    std::fs::write(
        &config_path,
        encode_value_pretty_with_newline_error_string(&JsonValue::Object(raw_config)).unwrap(),
    )
    .unwrap();
    let repo = RepoRuntime::discover_from_path(&repo.root).unwrap();

    let payload = config_set(
        &repo,
        &ConfigSetRequest {
            default_author_mode: Some("human_only".to_string()),
            default_model: Some("model-a".to_string()),
            task_review: Some("required".to_string()),
            task_worktree_alias_root: Some("managed-links".to_string()),
            task_worktree_main_seed_ram_max_bytes: Some(4096),
            id_namespace_prefix: Some("zx".to_string()),
            user_name: Some("Reviewer".to_string()),
            user_email: Some("reviewer@example.test".to_string()),
            ..ConfigSetRequest::default()
        },
    )
    .expect("set every admitted optional override");
    assert_eq!(payload["effective_author_mode"], "human_only");
    assert_eq!(payload["default_model"], "model-a");
    assert_eq!(payload["task_review"]["value"], "required");
    assert_eq!(
        payload["task_worktree"]["alias_root"]["value"],
        "managed-links"
    );
    assert_eq!(
        payload["task_worktree"]["main_seed_ram_max_bytes"]["value"],
        4096
    );
    assert_eq!(payload["id_namespace_prefix"]["value"], "ZX");

    let stored = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    assert_eq!(stored["task_review"], true);
    assert_eq!(stored["legacy_config_marker"], json!({"keep": true}));
    assert_eq!(
        stored["task_worktree"]["legacy_nested_marker"],
        json!([1, 2, 3])
    );

    for key in [
        ConfigUnsetKey::DefaultAuthorMode,
        ConfigUnsetKey::DefaultModel,
        ConfigUnsetKey::TaskReview,
        ConfigUnsetKey::TaskWorktreeAliasRoot,
        ConfigUnsetKey::TaskWorktreeMainSeedRamMaxBytes,
        ConfigUnsetKey::IdNamespacePrefix,
        ConfigUnsetKey::UserName,
        ConfigUnsetKey::UserEmail,
    ] {
        config_unset(&repo, key).unwrap_or_else(|error| panic!("{}: {error}", key.as_str()));
    }

    let shown = config_show(&RepoRuntime::discover_from_path(&repo.root).unwrap()).unwrap();
    assert_eq!(shown["effective_author_mode"], "ai_with_human_review");
    assert!(shown["default_model"].is_null());
    assert!(shown["effective_model"].is_null());
    assert_eq!(shown["task_review"]["value"], "automatic");
    assert_eq!(
        shown["task_worktree"]["alias_root"]["value"],
        ".ait-worktree-links"
    );
    assert!(shown["task_worktree"]["main_seed_ram_max_bytes"]["value"].is_null());
    assert_eq!(shown["id_namespace_prefix"]["value"], "");
    assert!(shown["user_name"].is_null());
    assert!(shown["user_email"].is_null());

    let stored = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    assert_eq!(stored["legacy_config_marker"], json!({"keep": true}));
    assert_eq!(
        stored["task_worktree"]["legacy_nested_marker"],
        json!([1, 2, 3])
    );
    for key in [
        "default_author_mode",
        "default_model",
        "task_review",
        "id_namespace_prefix",
        "user_name",
        "user_email",
    ] {
        assert!(!stored.contains_key(key), "{key}");
    }
    assert!(stored["task_worktree"].get("alias_root").is_none());
    assert!(stored["task_worktree"]
        .get("main_seed_ram_max_bytes")
        .is_none());
}

#[test]
fn config_set_rejects_empty_legacy_alias_and_mistyped_payloads_before_writing() {
    let (_temp, repo) = initialized_repo();
    let config_path = repo.root.join(".ait/config.json");
    let invalid_requests = [
        ConfigSetRequest {
            default_model: Some("   ".to_string()),
            ..ConfigSetRequest::default()
        },
        ConfigSetRequest {
            task_review: Some("on".to_string()),
            ..ConfigSetRequest::default()
        },
        ConfigSetRequest {
            sprint: Some("yes".to_string()),
            ..ConfigSetRequest::default()
        },
        ConfigSetRequest {
            workflow_mode: Some("SOLO_LOCAL".to_string()),
            ..ConfigSetRequest::default()
        },
        ConfigSetRequest {
            task_worktree_main_seed_ram_max_bytes: Some(-1),
            ..ConfigSetRequest::default()
        },
        ConfigSetRequest {
            id_namespace_prefix: Some(String::new()),
            ..ConfigSetRequest::default()
        },
    ];
    for request in invalid_requests {
        let before = std::fs::read(&config_path).unwrap();
        config_set(&repo, &request).expect_err("invalid set request must fail");
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
    }

    for payload in [
        json!({}),
        json!({"default_model": null}),
        json!({"default_model": 7}),
        json!({"task_review": true}),
        json!({"task_review": "off"}),
        json!({"task_worktree_main_seed_ram_max_bytes": "4096"}),
    ] {
        let before = std::fs::read(&config_path).unwrap();
        config_set_from_payload(&repo, &payload).expect_err("mistyped payload must fail");
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
    }

    let mut malformed = parse_object_or_empty(&std::fs::read_to_string(&config_path).unwrap());
    malformed.insert("task_worktree".to_string(), json!("not-an-object"));
    std::fs::write(
        &config_path,
        encode_value_pretty_with_newline_error_string(&JsonValue::Object(malformed)).unwrap(),
    )
    .unwrap();
    let before = std::fs::read(&config_path).unwrap();
    config_unset(&repo, ConfigUnsetKey::TaskWorktreeAliasRoot)
        .expect_err("malformed nested config must fail before unset writes");
    assert_eq!(std::fs::read(&config_path).unwrap(), before);
}

#[test]
fn task_review_uses_required_and_automatic_while_reading_existing_booleans() {
    let (_temp, repo) = initialized_repo();
    let required = config_set(
        &repo,
        &ConfigSetRequest {
            task_review: Some("required".to_string()),
            ..ConfigSetRequest::default()
        },
    )
    .unwrap();
    assert_eq!(required["task_review"]["value"], "required");
    let stored = parse_object_or_empty(
        &std::fs::read_to_string(repo.root.join(".ait/config.json")).unwrap(),
    );
    assert_eq!(stored["task_review"], true);

    let automatic = config_set(
        &repo,
        &ConfigSetRequest {
            task_review: Some("automatic".to_string()),
            ..ConfigSetRequest::default()
        },
    )
    .unwrap();
    assert_eq!(automatic["task_review"]["value"], "automatic");
    let stored = parse_object_or_empty(
        &std::fs::read_to_string(repo.root.join(".ait/config.json")).unwrap(),
    );
    assert_eq!(stored["task_review"], false);
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
    assert!(agents.contains("Route: mode=`solo_local`; sprint=`off`; scope=`local`"));
    assert!(agents.contains("plan-binding=`off`"));
    assert!(agents.contains("--title \"<title>\" --intent \"<intent>\"`"));
    assert!(!agents.to_ascii_lowercase().contains("json"));
    assert!(agents.contains("`--from` is unavailable"));
    assert!(agents.contains("--edit-root\n<absolute-path>"));
    let claude = std::fs::read_to_string(temp.path().join("CLAUDE.md")).unwrap();
    assert!(claude.contains("Route: mode=`solo_local`; sprint=`off`; scope=`local`"));
    assert!(claude.contains("--title \"<title>\" --intent \"<intent>\" --edit-root"));
    assert!(claude.contains("&& cd <absolute-path>"));
    assert!(!claude.to_ascii_lowercase().contains("json"));
    assert!(claude.contains("Do not omit `--edit-root`"));
    assert!(!claude.contains("@AGENTS.md"));
    assert_eq!(
        payload["agent_harness"]["plan_syncs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
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
    assert!(agents.contains("Route: mode=`solo_remote`; sprint=`on`; scope=`remote`"));
    assert!(agents.contains("plan-binding=`required`"));
    assert!(agents.contains("ait plan sync <markdown-file-or-dir> --remote origin"));
    assert!(agents.contains("ait task start --from"));
    assert!(agents.contains("`--from` syncs and binds"));
    assert!(agents.contains("--remote origin`"));
    assert!(!agents.to_ascii_lowercase().contains("json"));
    assert!(agents.contains("--edit-root\n<absolute-path>"));
    assert!(!agents.contains("--plan-item-ref"));
    assert!(agents.contains("After every context-window compaction, re-read the bound sprint card"));
    assert!(agents.contains("ait workflow ready <change-id> --apply"));
    assert!(agents.contains("Workflow finish owns Review, approval"));
    let claude = std::fs::read_to_string(temp.path().join("CLAUDE.md")).unwrap();
    assert!(claude.contains("Route: mode=`solo_remote`; sprint=`on`; scope=`remote`"));
    assert!(claude.contains("--edit-root <absolute-path> --remote origin"));
    assert!(claude.contains("&& cd <absolute-path>"));
    assert!(!claude.to_ascii_lowercase().contains("json"));
    assert!(!claude.contains("@AGENTS.md"));
    assert!(temp.path().join("docs/sprints").is_dir());
}
