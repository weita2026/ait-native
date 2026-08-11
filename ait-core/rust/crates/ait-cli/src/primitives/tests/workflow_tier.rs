use super::*;
use tempfile::TempDir;

fn quick_repo() -> (TempDir, RepoRuntime) {
    let repo_tmp = tempdir().expect("quick repo tempdir");
    let root = repo_tmp.path();
    init_repo(&InitRequest {
        root: root.to_path_buf(),
        name: Some("quick-fixture".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "human_only".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init quick fixture");
    let repo = RepoRuntime::discover_from_path(root).expect("discover quick fixture");
    fs::write(root.join("README.txt"), "baseline\n").expect("write baseline");
    snapshot_create(&repo, Some("baseline")).expect("create baseline Snapshot");
    line_create(&repo, "quick-notes", None, true, false, false).expect("create quick line");
    let repo = RepoRuntime::discover_from_path(root).expect("rediscover switched quick fixture");
    (repo_tmp, repo)
}

#[test]
fn quick_snapshot_records_immutable_intent_and_validation_provenance() {
    let (repo_tmp, repo) = quick_repo();
    fs::write(repo_tmp.path().join("notes.txt"), "small safe change\n")
        .expect("write quick change");

    let tier = workflow_tier_payload(&repo).expect("evaluate quick tier");
    assert_eq!(tier["recommended_tier"], "quick_modification");
    assert_eq!(tier["quick_allowed"], true);
    assert_eq!(tier["facts"]["current_line"], "quick-notes");

    let created = snapshot_create_quick(
        &repo,
        Some("Clarify notes"),
        Some("Clarify one local note"),
        Some("text check passed"),
    )
    .expect("create guarded quick Snapshot");
    assert_eq!(created["profile"], "quick");
    assert_eq!(created["intent"], "Clarify one local note");
    assert_eq!(created["validation"], "text check passed");
    assert_eq!(
        created["workflow_tier"]["recommended_tier"],
        "quick_modification"
    );

    let snapshot_id = created["snapshot_id"].as_str().expect("quick snapshot id");
    let stored = snapshot_show(&repo, snapshot_id).expect("show stored quick Snapshot");
    let stored_message = stored["message"].as_str().expect("stored quick message");
    assert!(stored_message.starts_with("Clarify notes\n\nAIT-Quick-Provenance: "));
    assert!(stored_message.contains("ait.quick-snapshot-provenance/v1"));
    assert!(stored_message.contains("Clarify one local note"));
    assert!(stored_message.contains("text check passed"));
}

#[test]
fn quick_snapshot_escalates_high_risk_content_without_advancing_the_line() {
    let (repo_tmp, repo) = quick_repo();
    let head_before = line_show(&repo, Some("quick-notes"))
        .expect("line before high risk")
        .get("head_snapshot_id")
        .cloned()
        .unwrap_or(JsonValue::Null);
    fs::write(
        repo_tmp.path().join("Cargo.toml"),
        "[package]\nname='risk'\n",
    )
    .expect("write dependency manifest");

    let tier = workflow_tier_payload(&repo).expect("evaluate governed tier");
    assert_eq!(tier["recommended_tier"], "fully_governed");
    assert_eq!(tier["quick_allowed"], false);
    assert_eq!(tier["high_risk_paths"][0]["path"], "Cargo.toml");

    let error = snapshot_create_quick(
        &repo,
        Some("Unsafe shortcut"),
        Some("Change dependencies"),
        Some("fast check passed"),
    )
    .expect_err("protected change must escalate");
    assert!(error.contains("recommended tier is `fully_governed`"));
    assert!(error.contains("ait workflow ready"));

    let head_after = line_show(&repo, Some("quick-notes"))
        .expect("line after rejected quick Snapshot")
        .get("head_snapshot_id")
        .cloned()
        .unwrap_or(JsonValue::Null);
    assert_eq!(head_after, head_before);
}

#[test]
fn repository_quick_limits_escalate_before_snapshot_mutation() {
    let (repo_tmp, _repo) = quick_repo();
    let config_path = repo_tmp.path().join(".ait/config.json");
    let mut config = ait_core::json_support::JsonCodec::parse_object(
        &fs::read_to_string(&config_path).expect("read quick config"),
        "quick config",
    )
    .expect("parse quick config");
    config.insert(
        "workflow_quick".to_string(),
        json!({"max_files": 1, "max_bytes": 4, "forbidden_prefixes": ["internal/"]}),
    );
    fs::write(config_path, JsonValue::Object(config).to_string()).expect("write quick config");
    let repo = RepoRuntime::discover_from_path(repo_tmp.path()).expect("reload quick limits");
    fs::write(repo_tmp.path().join("notes.txt"), "more than four bytes\n")
        .expect("write oversized quick change");

    let tier = workflow_tier_payload(&repo).expect("evaluate configured quick limits");
    assert_eq!(tier["recommended_tier"], "normal_task");
    assert_eq!(tier["facts"]["limits_source"], "repository_config");
    assert_eq!(tier["limits"]["max_bytes"], 4);
    assert!(tier["reasons"]
        .as_array()
        .expect("tier reasons")
        .iter()
        .any(|reason| reason["code"] == "byte_limit_exceeded"));
}
