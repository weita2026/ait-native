use super::*;
use ait_core::plan_store::PlanReadStore;
use tempfile::TempDir;

fn request(root: &Path) -> InitRequest {
    InitRequest {
        root: root.to_path_buf(),
        name: Some("housekeeper".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: Some("codex".to_string()),
        repair_existing: false,
    }
}

fn read_config(root: &Path) -> JsonMap<String, JsonValue> {
    let text = fs::read_to_string(root.join(".ait/config.json")).unwrap();
    match parse_value(&text, "test config").unwrap() {
        JsonValue::Object(config) => config,
        _ => panic!("config must be an object"),
    }
}

#[test]
fn init_creates_authority_agent_contract_and_configured_sprint_directory() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();

    let payload = init_repo(&request(temp.path())).unwrap();

    assert_eq!(payload["action"], "initialized");
    assert_eq!(payload["repo_name"], "housekeeper");
    assert_eq!(payload["default_line"], "main");
    assert_eq!(payload["workflow_mode"]["value"], "solo_local");
    assert_eq!(payload["policy_profile"], "prototype");
    assert_eq!(payload["default_author_mode"], "ai_with_human_review");
    assert_eq!(payload["default_model"], "codex");
    assert_eq!(payload["repairs"], json!([]));
    assert!(payload.get("agent_harness").is_none());
    assert!(payload.get("bootstrap_files").is_none());
    assert!(payload.get("next_steps").is_none());
    assert!(temp.path().join(".ait/binary-db").is_dir());
    assert!(temp.path().join(".ait/binary-db/line.bin").is_file());
    assert!(temp.path().join(".ait/objects/packs").is_dir());
    assert!(temp.path().join(".ait/refs/lines").is_dir());
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "base\n"
    );
    let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
    assert!(agents.contains("<!-- ait:workflow:start -->"));
    assert!(agents.contains("entry: mode=`solo_local`; sprint=`on`; scopes=`local`"));
    assert!(agents.contains("entry: plan-binding=`required`"));
    assert!(agents.contains("`task start` revalidates entry"));
    assert!(agents.contains("Read this block and `docs/plan.md` when it exists"));
    assert!(temp.path().join("docs/sprints").is_dir());
    for path in ["ait-native.md", "docs/plan.md", "docs/milestone.md"] {
        assert!(!temp.path().join(path).exists(), "unexpected {path}");
    }
    let config = read_config(temp.path());
    assert_eq!(config["repo_name"], "housekeeper");
    assert_eq!(config["default_line"], "main");
    assert_eq!(config["current_line"], "main");
    assert_eq!(config["default_remote"], JsonValue::Null);
    assert_eq!(config["sprint"], "on");
    assert_eq!(config["plan_task_binding"]["mode"], "required");
    let policy_text = fs::read_to_string(temp.path().join(".ait/policy.yaml")).unwrap();
    assert!(policy_text.contains("policy_id: prototype"));
    assert!(policy_text.contains("require_ai_provenance: false"));
}

#[test]
fn built_in_policy_profiles_materialize_their_exact_gate_defaults() {
    for (profile, lint, security, license) in [
        ("prototype", false, false, false),
        ("team", true, false, false),
        ("release", true, true, true),
    ] {
        let temp = TempDir::new().unwrap();
        let mut init_request = request(temp.path());
        init_request.policy_profile = profile.to_string();

        let payload = init_repo(&init_request).unwrap();
        let policy_text = fs::read_to_string(temp.path().join(".ait/policy.yaml")).unwrap();
        let policy = parse_policy_yaml(&policy_text, DEFAULT_POLICY_PROFILE).unwrap();

        assert_eq!(payload["policy_profile"], profile);
        assert_eq!(policy["policy_id"], profile);
        assert_eq!(policy["defaults"]["require_attestation"], true);
        assert_eq!(policy["defaults"]["require_tests"], true);
        assert_eq!(policy["defaults"]["require_lint"], lint);
        assert_eq!(policy["defaults"]["require_security_scan"], security);
        assert_eq!(policy["defaults"]["require_license_scan"], license);
    }
}

#[test]
fn recovery_init_is_minimal_and_creates_no_plan_lineage() {
    let temp = TempDir::new().unwrap();
    let mut init_request = request(temp.path());
    init_request.name = Some("recovered".to_string());
    init_request.default_line = "feature/task".to_string();

    let payload = init_repo_for_remote_head_recovery(&init_request).unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let plans = PlanReadStore::list_plans(
        &repo
            .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .plans(),
    )
    .unwrap();

    assert_eq!(payload["action"], "initialized");
    assert!(plans.is_empty());
    assert!(!temp.path().join("AGENTS.md").exists());
    assert!(repo
        .line_store()
        .unwrap()
        .line_by_name("feature/task")
        .unwrap()
        .is_some());
}

#[test]
fn valid_repository_reinitializes_without_repair_and_preserves_authority_bytes() {
    let temp = TempDir::new().unwrap();
    let mut initial = request(temp.path());
    initial.default_model = None;
    init_repo(&initial).unwrap();
    let config_before = fs::read(temp.path().join(".ait/config.json")).unwrap();
    let policy_before = fs::read(temp.path().join(".ait/policy.yaml")).unwrap();
    let line_before = fs::read(temp.path().join(".ait/binary-db/line.bin")).unwrap();
    let agents_path = temp.path().join("AGENTS.md");
    let mut agents = fs::read_to_string(&agents_path).unwrap();
    agents.push_str("\nRepository-specific rule.\n");
    fs::write(&agents_path, &agents).unwrap();
    fs::write(temp.path().join("WORKSPACE.md"), "keep\n").unwrap();

    let mut rerun = request(temp.path());
    rerun.name = Some("replacement".to_string());
    rerun.default_line = "replacement-line".to_string();
    rerun.policy_profile = "release".to_string();
    rerun.default_author_mode = "human_only".to_string();
    rerun.default_model = Some("replacement-model".to_string());
    let payload = init_repo(&rerun).unwrap();

    assert_eq!(payload["action"], "reinitialized");
    assert_eq!(payload["repo_name"], "housekeeper");
    assert_eq!(payload["default_line"], "main");
    assert_eq!(payload["policy_profile"], "prototype");
    assert_eq!(payload["default_author_mode"], "ai_with_human_review");
    assert_eq!(payload["default_model"], JsonValue::Null);
    assert_eq!(
        fs::read(temp.path().join(".ait/config.json")).unwrap(),
        config_before
    );
    assert_eq!(
        fs::read(temp.path().join(".ait/policy.yaml")).unwrap(),
        policy_before
    );
    assert_eq!(
        fs::read(temp.path().join(".ait/binary-db/line.bin")).unwrap(),
        line_before
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("WORKSPACE.md")).unwrap(),
        "keep\n"
    );
    let refreshed_agents = fs::read_to_string(&agents_path).unwrap();
    assert!(refreshed_agents.contains("Repository-specific rule."));
    assert_eq!(
        refreshed_agents
            .matches("<!-- ait:workflow:start -->")
            .count(),
        1
    );
}

#[test]
fn invalid_creation_values_leave_no_repository_shell() {
    for (field, mutate) in [
        (
            "policy",
            (|request: &mut InitRequest| request.policy_profile = "invalid".to_string())
                as fn(&mut InitRequest),
        ),
        (
            "author",
            (|request: &mut InitRequest| request.default_author_mode = "invalid".to_string())
                as fn(&mut InitRequest),
        ),
        (
            "line",
            (|request: &mut InitRequest| request.default_line = "   ".to_string())
                as fn(&mut InitRequest),
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let mut init_request = request(temp.path());
        mutate(&mut init_request);
        assert!(init_repo(&init_request).is_err(), "{field} must fail");
        assert!(
            fs::symlink_metadata(temp.path().join(".ait")).is_err(),
            "{field} left an .ait entry"
        );
        assert!(!temp.path().join("AGENTS.md").exists());
        assert!(!temp.path().join("docs").exists());
    }
}

#[test]
fn malformed_config_fails_closed_during_repair() {
    let temp = TempDir::new().unwrap();
    init_repo(&request(temp.path())).unwrap();
    let config_path = temp.path().join(".ait/config.json");
    let malformed = b"{not-json\n";
    fs::write(&config_path, malformed).unwrap();
    let mut repair = request(temp.path());
    repair.repair_existing = true;

    let error = init_repo(&repair).unwrap_err();

    assert!(error.contains("Failed to parse repository config"));
    assert_eq!(fs::read(&config_path).unwrap(), malformed);
}

#[test]
fn non_object_config_fails_closed_during_repair() {
    let temp = TempDir::new().unwrap();
    init_repo(&request(temp.path())).unwrap();
    let config_path = temp.path().join(".ait/config.json");
    fs::write(&config_path, "[]\n").unwrap();
    let mut repair = request(temp.path());
    repair.repair_existing = true;

    let error = init_repo(&repair).unwrap_err();

    assert!(error.contains("must contain a JSON object"));
    assert_eq!(fs::read_to_string(&config_path).unwrap(), "[]\n");
}

#[test]
fn malformed_policy_fails_closed_during_repair() {
    let temp = TempDir::new().unwrap();
    init_repo(&request(temp.path())).unwrap();
    let policy_path = temp.path().join(".ait/policy.yaml");
    let malformed = "version: 1\npolicy_id: prototype\ndefaults:\n  not-a-policy-field: true\n";
    fs::write(&policy_path, malformed).unwrap();
    let mut repair = request(temp.path());
    repair.repair_existing = true;

    let error = init_repo(&repair).unwrap_err();

    assert!(error.contains("unknown field"));
    assert_eq!(fs::read_to_string(&policy_path).unwrap(), malformed);
}

#[test]
fn repair_is_required_and_fills_only_missing_structure() {
    let temp = TempDir::new().unwrap();
    init_repo(&request(temp.path())).unwrap();
    let config_path = temp.path().join(".ait/config.json");
    let mut config = read_config(temp.path());
    config.remove("current_line");
    write_json_pretty(&config_path, &JsonValue::Object(config)).unwrap();
    let policy_path = temp.path().join(".ait/policy.yaml");
    fs::remove_file(&policy_path).unwrap();
    let missing_dir = temp.path().join(".ait/objects/manifests");
    fs::remove_dir(&missing_dir).unwrap();
    let config_before = fs::read(&config_path).unwrap();

    let error = init_repo(&request(temp.path())).unwrap_err();
    assert!(error.contains("--repair-existing"));
    assert_eq!(fs::read(&config_path).unwrap(), config_before);
    assert!(!policy_path.exists());
    assert!(!missing_dir.exists());

    let mut repair = request(temp.path());
    repair.repair_existing = true;
    let payload = init_repo(&repair).unwrap();

    assert_eq!(payload["action"], "repaired");
    assert_eq!(read_config(temp.path())["current_line"], "main");
    assert!(policy_path.is_file());
    assert!(missing_dir.is_dir());
}

#[cfg(unix)]
#[test]
fn symbolic_link_authority_paths_fail_closed() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    symlink(outside.path(), temp.path().join(".ait")).unwrap();
    let error = init_repo(&request(temp.path())).unwrap_err();
    assert!(error.contains("symbolic-link repository authority"));

    let second = TempDir::new().unwrap();
    init_repo(&request(second.path())).unwrap();
    let config_path = second.path().join(".ait/config.json");
    let config_bytes = fs::read(&config_path).unwrap();
    fs::remove_file(&config_path).unwrap();
    let outside_config = outside.path().join("config.json");
    fs::write(&outside_config, &config_bytes).unwrap();
    symlink(&outside_config, &config_path).unwrap();
    let mut repair = request(second.path());
    repair.repair_existing = true;
    let error = init_repo(&repair).unwrap_err();
    assert!(error.contains("symbolic-link Repository config"));
    assert_eq!(fs::read(&outside_config).unwrap(), config_bytes);

    let third = TempDir::new().unwrap();
    init_repo_for_remote_head_recovery(&request(third.path())).unwrap();
    let outside_agents = outside.path().join("AGENTS.md");
    fs::write(&outside_agents, "# Outside\n").unwrap();
    symlink(&outside_agents, third.path().join("AGENTS.md")).unwrap();
    let error = init_repo(&request(third.path())).unwrap_err();
    assert!(error.contains("symbolic-link Agent contract"));
    assert_eq!(fs::read_to_string(&outside_agents).unwrap(), "# Outside\n");

    let fourth = TempDir::new().unwrap();
    init_repo_for_remote_head_recovery(&request(fourth.path())).unwrap();
    symlink(outside.path(), fourth.path().join("docs")).unwrap();
    let error = init_repo(&request(fourth.path())).unwrap_err();
    assert!(error.contains("symbolic-link workflow directory"));
}

#[test]
fn wrong_authority_path_kind_fails_without_replacement() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join(".ait"), "user data\n").unwrap();

    let error = init_repo(&request(temp.path())).unwrap_err();

    assert!(error.contains("must be a directory"));
    assert_eq!(
        fs::read_to_string(temp.path().join(".ait")).unwrap(),
        "user data\n"
    );

    let agents_root = TempDir::new().unwrap();
    init_repo_for_remote_head_recovery(&request(agents_root.path())).unwrap();
    fs::create_dir(agents_root.path().join("AGENTS.md")).unwrap();
    let error = init_repo(&request(agents_root.path())).unwrap_err();
    assert!(error.contains("Agent contract must be a regular file"));
    assert!(agents_root.path().join("AGENTS.md").is_dir());

    let docs_root = TempDir::new().unwrap();
    init_repo_for_remote_head_recovery(&request(docs_root.path())).unwrap();
    fs::write(docs_root.path().join("docs"), "user data\n").unwrap();
    let error = init_repo(&request(docs_root.path())).unwrap_err();
    assert!(error.contains("Workflow directory path has the wrong file kind"));
    assert_eq!(
        fs::read_to_string(docs_root.path().join("docs")).unwrap(),
        "user data\n"
    );
}
