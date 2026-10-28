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
fn plan_preferences_set_unset_and_generated_lineage_are_independent_and_stable() {
    let (temp, repo) = initialized_repo();
    let updated = config_set_from_payload(
        &repo,
        &json!({"plan_language":"ZH-hant-tw","plan_style":"detailed"}),
    )
    .unwrap();
    assert_eq!(
        updated["plan_language"],
        json!({"value":"zh-Hant-TW","source":"repo_config"})
    );
    assert_eq!(updated["plan_style"]["value"], "detailed");
    for path in ["AGENTS.md", "CLAUDE.md"] {
        let text = std::fs::read_to_string(temp.path().join(path)).unwrap();
        assert!(text.contains("language=`zh-Hant-TW` (BCP 47); style=`detailed`"));
        assert!(!text.contains("Traditional Chinese"));
    }
    let same = config_set_from_payload(
        &repo,
        &json!({"plan_language":"zh-Hant-TW","plan_style":"detailed"}),
    )
    .unwrap();
    assert_eq!(same["agent_harness"]["refresh"]["changed"], false);
    for sync in same["agent_harness"]["plan_syncs"].as_array().unwrap() {
        for result in sync["result"]["results"].as_array().unwrap() {
            assert_eq!(result["action"], "unchanged");
        }
    }
    let updated = config_set_from_payload(&repo, &json!({"plan_language":"ko-KR"})).unwrap();
    assert_eq!(updated["plan_style"]["value"], "detailed");
    let unset = config_unset(&repo, ConfigUnsetKey::PlanLanguage).unwrap();
    assert_eq!(
        unset["plan_language"],
        json!({"value":"en","source":"built_in"})
    );
    assert_eq!(unset["plan_style"]["value"], "detailed");
    let unset = config_unset(&repo, ConfigUnsetKey::PlanStyle).unwrap();
    assert_eq!(
        unset["plan_style"],
        json!({"value":"concise","source":"built_in"})
    );
    let stored = parse_object_or_empty(
        &std::fs::read_to_string(temp.path().join(".ait/config.json")).unwrap(),
    );
    assert!(!stored.contains_key("plan_language"));
    assert!(!stored.contains_key("plan_style"));
}

#[test]
fn invalid_plan_preferences_reject_all_writes_and_allow_explicit_repair() {
    let (temp, repo) = initialized_repo();
    let paths = [".ait/config.json", "AGENTS.md", "CLAUDE.md"];
    let bytes = || paths.map(|path| std::fs::read(temp.path().join(path)).unwrap());
    for payload in [
        json!({"plan_language":"ja-JP","plan_style":"invalid"}),
        json!({"plan_language":null,"plan_style":"concise"}),
        json!({"plan_language":3}),
        json!({"plan_style":false}),
        json!({"plan_language":"zh_TW.UTF-8"}),
    ] {
        let before = bytes();
        assert!(config_set_from_payload(&repo, &payload).is_err());
        assert_eq!(bytes(), before);
    }
    let path = temp.path().join(".ait/config.json");
    let mut config = parse_object_or_empty(&std::fs::read_to_string(&path).unwrap());
    config.insert("plan_language".into(), json!(false));
    std::fs::write(
        &path,
        encode_value_pretty_with_newline_error_string(&JsonValue::Object(config)).unwrap(),
    )
    .unwrap();
    let broken = RepoRuntime::discover_from_path(temp.path()).unwrap();
    assert!(config_show(&broken).unwrap_err().contains("plan_language"));
    let before = bytes();
    assert!(config_set_from_payload(&broken, &json!({"default_model":"example"})).is_err());
    assert_eq!(bytes(), before);
    let repaired = config_set_from_payload(&broken, &json!({"plan_language":"ja-JP"})).unwrap();
    assert_eq!(repaired["plan_language"]["value"], "ja-JP");
    let mut config = parse_object_or_empty(&std::fs::read_to_string(&path).unwrap());
    config.insert("plan_style".into(), JsonValue::Null);
    std::fs::write(
        &path,
        encode_value_pretty_with_newline_error_string(&JsonValue::Object(config)).unwrap(),
    )
    .unwrap();
    let removed = config_unset(&repo, ConfigUnsetKey::PlanStyle).unwrap();
    assert_eq!(removed["plan_style"]["value"], "concise");
}

#[test]
#[cfg(unix)]
fn worktree_plan_preferences_use_the_root_and_preserve_worktree_documents() {
    let (temp, repo) = initialized_repo();
    config_set_from_payload(
        &repo,
        &json!({"plan_language":"zh-TW","plan_style":"detailed"}),
    )
    .unwrap();
    let root = temp.path().canonicalize().unwrap();
    let worktree = root.join("worktree");
    std::fs::create_dir(&worktree).unwrap();
    std::os::unix::fs::symlink(root.join(".ait"), worktree.join(".ait")).unwrap();
    let overlay = json!({"repo_root":root,"workspace_root":worktree,"worktree_name":"test", "plan_language":false,"plan_style":"broken"});
    std::fs::write(worktree.join(".ait-worktree.json"), overlay.to_string()).unwrap();
    std::fs::write(worktree.join("AGENTS.md"), "Existing worktree document\n").unwrap();
    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    assert_eq!(
        config_show(&repo).unwrap()["plan_language"]["value"],
        "zh-TW"
    );
    let rendered = crate::agent_harness::render_agent_workflow_block(&repo).unwrap();
    assert!(rendered.contains("language=`zh-TW` (BCP 47); style=`detailed`"));
    let updated = config_set_from_payload(&repo, &json!({"plan_language":"ko-KR"})).unwrap();
    assert_eq!(updated["plan_language"]["value"], "ko-KR");
    assert!(std::fs::read_to_string(root.join("AGENTS.md"))
        .unwrap()
        .contains("language=`ko-KR`"));
    assert_eq!(
        std::fs::read_to_string(worktree.join("AGENTS.md")).unwrap(),
        "Existing worktree document\n"
    );
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
    let path = temp.path().join(".ait/config.json");
    let mut config = parse_object_or_empty(&std::fs::read_to_string(&path).unwrap());
    config.insert("agent_teams".into(), json!(false));
    std::fs::write(path, json!(config).to_string()).unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    (temp, repo)
}

#[test]
fn config_mutation_reuses_the_config_inode_across_repeated_set_and_unset() {
    let (temp, repo) = initialized_repo();
    let path = temp.path().join(".ait/config.json");
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&path).unwrap();
        (metadata.dev(), metadata.ino())
    };
    for model in ["long-model-name-".repeat(32), "short".into(), "next".into()] {
        config_set_from_payload(&repo, &json!({"default_model":model})).unwrap();
        let fresh = RepoRuntime::discover_from_path(temp.path()).unwrap();
        assert_eq!(config_show(&fresh).unwrap()["default_model"], model);
        let bytes = std::fs::read_to_string(&path).unwrap();
        assert!(bytes.ends_with('\n'));
        assert_eq!(
            crate::json_support::parse_value(&bytes, "config").unwrap()["default_model"],
            model
        );
    }
    config_unset(&repo, ConfigUnsetKey::DefaultModel).unwrap();
    let fresh = RepoRuntime::discover_from_path(temp.path()).unwrap();
    assert!(config_show(&fresh).unwrap()["default_model"].is_null());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!((metadata.dev(), metadata.ino()), inode);
    }
}

#[test]
fn config_mutations_respect_shared_cleanup_and_exclusive_provisioning_guards() {
    let (temp, repo) = initialized_repo();
    let path = temp.path().join(".ait/config.json");
    for exclusive in [false, true] {
        let before = std::fs::read(&path).unwrap();
        let lease = super::lock_root_config(&repo, exclusive).unwrap();
        let error = config_set_from_payload(&repo, &json!({"default_model":"busy"})).unwrap_err();
        assert!(error.contains("operation_busy"));
        let error = config_unset(&repo, ConfigUnsetKey::DefaultModel).unwrap_err();
        assert!(error.contains("operation_busy"));
        drop(lease);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        config_set_from_payload(&repo, &json!({"default_model":"released"})).unwrap();
        config_unset(&repo, ConfigUnsetKey::DefaultModel).unwrap();
    }
}

#[test]
fn config_mutation_holds_exclusive_ownership_through_validation_and_write() {
    let (temp, repo) = initialized_repo();
    let path = temp.path().join(".ait/config.json");
    super::update_root_config(&repo, |config| {
        let contender = std::fs::File::open(&path).unwrap();
        assert!(fs2::FileExt::try_lock_shared(&contender).is_err());
        assert!(fs2::FileExt::try_lock_exclusive(&contender).is_err());
        #[cfg(windows)]
        {
            // A second handle in this process is not the exclusive lock owner.
            let error = std::fs::read(&path).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(33));
        }
        config.insert("default_model".into(), json!("same-handle"));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        parse_object_or_empty(&std::fs::read_to_string(&path).unwrap())["default_model"],
        "same-handle"
    );
    let lease = super::lock_root_config(&repo, true).unwrap();
    drop(lease);
}

#[test]
fn config_validation_and_write_errors_preserve_original_bytes_and_release_lock() {
    let (temp, repo) = initialized_repo();
    let path = temp.path().join(".ait/config.json");
    let before = std::fs::read(&path).unwrap();
    super::update_root_config(&repo, |config| {
        config.insert("default_model".into(), json!("must-not-persist"));
        Err("injected validation failure".into())
    })
    .unwrap_err();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut read_only = std::fs::File::open(&path).unwrap();
    super::write_json_pretty(
        &mut read_only,
        &json!({"default_model":"must-not-truncate"}),
    )
    .unwrap_err();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    config_set_from_payload(&repo, &json!({"default_model":"after-error"})).unwrap();
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
    let agents = std::fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
    assert!(agents.contains("Route: sprint=`off`; scope=`local`"));
    assert!(agents.contains("plan-binding=`off`"));
    assert!(agents.contains("--title \"<title>\" --intent \"<intent>\"`"));
    assert!(!agents.to_ascii_lowercase().contains("json"));
    assert!(agents.contains("`--from` is unavailable"));
    assert!(agents.contains("--edit-root\n<absolute-path>"));
    let claude = std::fs::read_to_string(temp.path().join("CLAUDE.md")).unwrap();
    assert!(claude.contains("Route: sprint=`off`; scope=`local`"));
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
fn config_set_rejects_retired_mode_and_preserves_independent_sprint_binding() {
    let (temp, repo) = initialized_repo();
    assert!(config_set_from_payload(&repo, &json!({"workflow_mode":"solo_remote"})).is_err());
    let enabled =
        config_set_from_payload(&repo, &json!({"agent_teams":"enabled","sprint":"on"})).unwrap();
    assert_eq!(enabled["agent_teams"], true);
    assert_eq!(enabled["sprint"]["value"], "on");
    assert_eq!(enabled["default_author_mode"], "ai_with_human_review");
    assert_eq!(enabled["effective_author_mode"], "ai_only_experimental");
    let configured = RepoRuntime::discover_from_path(temp.path()).unwrap();
    assert_eq!(
        configured.effective_author_mode(None),
        "ai_only_experimental"
    );
    for path in ["AGENTS.md", "CLAUDE.md"] {
        let guidance = std::fs::read_to_string(temp.path().join(path)).unwrap();
        assert!(guidance.contains("author-mode=`ai_only_experimental`"));
        assert!(guidance.contains("a missing Reviewer requires assignment"));
    }
    let disabled = config_set_from_payload(&repo, &json!({"agent_teams":"disabled"})).unwrap();
    assert_eq!(disabled["agent_teams"], false);
    assert_eq!(disabled["default_author_mode"], "ai_with_human_review");
    assert_eq!(disabled["effective_author_mode"], "ai_with_human_review");
    for path in ["AGENTS.md", "CLAUDE.md"] {
        let guidance = std::fs::read_to_string(temp.path().join(path)).unwrap();
        assert!(guidance.contains("author-mode=`ai_with_human_review`"));
        assert!(!guidance.contains("Fresh team candidates"));
    }
    assert!(disabled.get("workflow_mode").is_none());
}

#[test]
fn agent_count_payload_is_optional_bounded_and_does_not_change_solo_route() {
    let (temp, repo) = initialized_repo();
    let before = std::fs::read(temp.path().join(".ait/config.json")).unwrap();
    for value in [json!(0), json!(4), json!(50)] {
        assert!(config_set_from_payload(&repo, &json!({"agent_team_size":value})).is_err());
    }
    assert_eq!(
        std::fs::read(temp.path().join(".ait/config.json")).unwrap(),
        before
    );
}
