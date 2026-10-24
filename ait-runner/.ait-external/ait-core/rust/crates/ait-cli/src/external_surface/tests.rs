use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use ait_core::json_support::json;
use ait_core::json_support::JsonMap;

use crate::runtime::create_binary_test_snapshot as create_local_snapshot;
use ait_core::external::link::EXTERNAL_LINKS_FILE;
use ait_core::external::manifest::{ExternalBindingSet, ExternalDeclaration, ExternalManifest};
use ait_core::external::resolver::{
    resolve_external_lockfile, ExternalResolutionOptions, ExternalSnapshotResolver,
};
use ait_core::external::update::{
    ExternalUpdateOptions, ExternalUpdateSelection, FilesystemExternalUpdateStore,
};
use ait_core::external::ExternalResult;
use ait_core::line_store::LineStore;
use ait_core::local_snapshot::LocalSnapshotWriteStore;

use crate::init_surface::{init_repo, InitRequest};
use crate::runtime::RepoRuntime;

use super::{
    external_doctor, external_link, external_status, external_unlink, external_update,
    freeze_staged_external_update_selection, hydrate_external_update_selection_with_ports,
    line_head_from_remote_rows, materialize_locked_external_release_sources,
    render_external_doctor_text, render_external_link_text, render_external_status_text,
    render_external_text, render_external_unlink_text, render_external_update_text,
    ExternalRemoteLineHeadSource, ExternalUpdateHydrationPorts,
    RemoteAwareExternalSnapshotResolver,
};

#[test]
fn release_materialization_uses_locked_selected_snapshot_instead_of_workspace_content() {
    let (repo_root, repo, snapshot_id) = init_fixture_external_repo();
    write_external_manifest(repo_root.path(), &repo.repo_name(), &snapshot_id);
    external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();
    std::fs::write(
        repo_root.path().join(".ait-external/ait-db/db.txt"),
        "dirty workspace content\n",
    )
    .unwrap();
    let lockfile_bytes = std::fs::read(repo_root.path().join("ait-external.lock")).unwrap();
    let isolated = tempfile::tempdir().unwrap();

    let report =
        materialize_locked_external_release_sources(&repo, &lockfile_bytes, isolated.path())
            .unwrap();

    assert_eq!(report["authority"], json!("ait-external.lock"));
    assert_eq!(report["content_source"], json!("selected_snapshot_store"));
    assert_eq!(report["locked"], json!(true));
    assert_eq!(report["release_ready"], json!(true));
    assert_eq!(report["summary"]["materialized_count"], json!(1));
    assert_eq!(report["entries"][0]["snapshot"], json!(snapshot_id));
    assert_eq!(
        std::fs::read_to_string(isolated.path().join(".ait-external/ait-db/db.txt")).unwrap(),
        "alpha\n"
    );
    assert!(isolated
        .path()
        .join(".ait-external/ait-db/.ait-external-marker.json")
        .is_file());
}

#[test]
fn external_status_text_renders_summary_and_entries() {
    let payload = json!({
        "command": "external status",
        "repo_name": "ait-core",
        "externals": [
            {
                "name": "ait-db",
                "snapshot": "SNP-DB-LINKED",
                "materialize_to": ".ait-external/ait-db",
                "state": "linked",
                "link_path": "../ait-db"
            }
        ],
        "summary": {
            "missing": 0,
            "linked": 1,
            "dirty": 0,
            "outdated": 0,
            "lock_drift": 0
        }
    });

    let text = render_external_status_text(&payload).unwrap();

    assert!(text.contains("ait external status"));
    assert!(text.contains("summary: missing=0 linked=1 dirty=0 outdated=0 lock_drift=0"));
    assert!(text.contains("- ait-db [linked] SNP-DB-LINKED -> .ait-external/ait-db link=../ait-db"));
    assert_eq!(render_external_text("status", &payload).unwrap(), text);
}

#[test]
fn external_doctor_text_separates_release_blocking_and_warnings() {
    let payload = json!({
        "command": "external doctor",
        "repo_name": "ait-core",
        "release_ready": false,
        "findings": [
            {
                "code": "external_local_link_active",
                "severity": "error",
                "release_blocking": true,
                "name": "ait-db",
                "path": ".ait-external/ait-db",
                "message": "local link override is active"
            },
            {
                "code": "external_binding_path_missing",
                "severity": "warning",
                "release_blocking": false,
                "name": "ait-db",
                "path": ".ait-external/ait-db/python",
                "message": "declared Python binding path is missing"
            }
        ]
    });

    let text = render_external_doctor_text(&payload).unwrap();

    assert!(text.contains("release-blocking:"));
    assert!(text.contains("external_local_link_active ait-db .ait-external/ait-db"));
    assert!(text.contains("warnings:"));
    assert!(text.contains("external_binding_path_missing ait-db .ait-external/ait-db/python"));
    assert_eq!(render_external_text("doctor", &payload).unwrap(), text);
}

#[test]
fn external_link_and_unlink_update_declared_local_override_without_a_lockfile() {
    let repo_root = tempfile::tempdir().unwrap();
    let linked_external = tempfile::tempdir().unwrap();
    let repo = test_repo(repo_root.path());
    write_external_manifest(repo_root.path(), "fixture-consumer", "SNP-LINK-ONLY");
    let link_path = linked_external.path().to_string_lossy().to_string();

    let linked = external_link(&repo, "ait-db", &link_path).unwrap();

    assert_eq!(linked["command"], "external link");
    assert_eq!(linked["name"], "ait-db");
    assert_eq!(linked["path"], link_path);
    assert!(render_external_link_text(&linked)
        .unwrap()
        .contains("linked: ait-db ->"));
    assert!(repo_root.path().join(EXTERNAL_LINKS_FILE).exists());
    assert!(repo_root.path().join("ait-external.toml").exists());
    assert!(!repo_root.path().join("ait-external.lock").exists());

    let unlinked = external_unlink(&repo, "ait-db").unwrap();

    assert_eq!(unlinked["command"], "external unlink");
    assert_eq!(unlinked["name"], "ait-db");
    assert!(render_external_unlink_text(&unlinked)
        .unwrap()
        .contains("unlinked: ait-db"));
    assert_eq!(
        render_external_text("unlink", &unlinked).unwrap(),
        render_external_unlink_text(&unlinked).unwrap()
    );
    assert!(!repo_root.path().join(EXTERNAL_LINKS_FILE).exists());
    assert!(repo_root.path().join("ait-external.toml").exists());
    assert!(!repo_root.path().join("ait-external.lock").exists());
    assert_eq!(unlinked["restore_state"], "skipped_no_lockfile");
}

#[test]
fn external_link_rejects_a_name_missing_from_the_root_manifest() {
    let repo_root = tempfile::tempdir().unwrap();
    let linked_external = tempfile::tempdir().unwrap();
    let repo = test_repo(repo_root.path());
    write_external_manifest(repo_root.path(), "fixture-consumer", "SNP-LINK-ONLY");

    let err = external_link(
        &repo,
        "unknown-db",
        &linked_external.path().to_string_lossy(),
    )
    .unwrap_err();

    assert!(err.contains("is not declared in the root ait-external.toml"));
    assert!(!repo_root.path().join(EXTERNAL_LINKS_FILE).exists());
}

#[test]
fn external_link_rejects_an_ambiguous_root_manifest_name() {
    let repo_root = tempfile::tempdir().unwrap();
    let linked_external = tempfile::tempdir().unwrap();
    let repo = test_repo(repo_root.path());
    let declaration = r#"[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 1
remote = "origin"
line = "main"
snapshot = "SNP-LINK-ONLY"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
"#;
    std::fs::write(
        repo_root.path().join("ait-external.toml"),
        format!("{declaration}\n{declaration}"),
    )
    .unwrap();

    let err =
        external_link(&repo, "ait-db", &linked_external.path().to_string_lossy()).unwrap_err();

    assert!(err.contains("appears more than once in the root ait-external.toml"));
    assert!(!repo_root.path().join(EXTERNAL_LINKS_FILE).exists());
}

#[cfg(unix)]
#[test]
fn external_link_rejects_repo_relative_symlink_escape() {
    let repo_root = tempfile::tempdir().unwrap();
    let linked_external = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(linked_external.path(), repo_root.path().join("linked-db")).unwrap();
    let repo = test_repo(repo_root.path());

    let err = external_link(&repo, "ait-db", "linked-db").unwrap_err();

    assert!(err.contains("resolves outside repository root"));
    assert!(!repo_root.path().join(EXTERNAL_LINKS_FILE).exists());
}

#[test]
fn external_link_accepts_explicit_parent_relative_sibling_target() {
    let parent = tempfile::tempdir().unwrap();
    let repo_root = parent.path().join("consumer");
    let linked_external = parent.path().join("ait-db");
    std::fs::create_dir_all(&repo_root).unwrap();
    std::fs::create_dir_all(&linked_external).unwrap();
    let repo = test_repo(&repo_root);
    write_external_manifest(&repo_root, "fixture-consumer", "SNP-LINK-ONLY");

    let linked = external_link(&repo, "ait-db", "../ait-db").unwrap();

    assert_eq!(linked["path"], "../ait-db");
    assert!(repo_root.join(EXTERNAL_LINKS_FILE).exists());
}

#[test]
fn external_link_rejects_materialized_path_inside_consumer_repo() {
    let repo_root = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: repo_root.path().to_path_buf(),
        name: Some("fixture-consumer".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let materialized = repo_root.path().join(".ait-external/ait-db");
    std::fs::create_dir_all(&materialized).unwrap();

    let err = external_link(&repo, "ait-db", &materialized.to_string_lossy()).unwrap_err();

    assert!(err.contains("resolves to this repository"));
    assert!(!repo_root.path().join(EXTERNAL_LINKS_FILE).exists());
}

#[test]
fn external_update_materializes_local_snapshot_and_exact_pin_changes() {
    let repo_root = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: repo_root.path().to_path_buf(),
        name: Some("fixture-consumer".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let repo_root_text = repo.workspace_root().to_string_lossy().to_string();

    std::fs::write(repo_root.path().join("db.txt"), "alpha\n").unwrap();
    let first = create_local_snapshot(
        &repo_root_text,
        &repo.repo_name(),
        "main",
        Some("external alpha"),
        false,
    )
    .unwrap();
    let first_snapshot = first["snapshot_id"].as_str().unwrap().to_string();
    std::fs::write(repo_root.path().join("db.txt"), "beta\n").unwrap();
    let second = create_local_snapshot(
        &repo_root_text,
        &repo.repo_name(),
        "main",
        Some("external beta"),
        false,
    )
    .unwrap();
    let second_snapshot = second["snapshot_id"].as_str().unwrap().to_string();

    write_external_manifest(repo_root.path(), &repo.repo_name(), &first_snapshot);
    let initial = external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();

    assert_eq!(initial["command"], "external update");
    assert_eq!(initial["lockfile_changed"], true);
    assert_eq!(
        std::fs::read_to_string(repo_root.path().join(".ait-external/ait-db/db.txt")).unwrap(),
        "alpha\n"
    );
    assert!(repo_root.path().join("ait-external.lock").exists());
    let initial_text = render_external_update_text(&initial).unwrap();
    assert!(initial_text.contains("ait external update"));
    assert!(initial_text.contains(&format!("ait-db [materialized] {first_snapshot}")));
    assert_eq!(
        render_external_text("update", &initial).unwrap(),
        initial_text
    );

    let exact = external_update(
        &repo,
        ExternalUpdateOptions::exact("ait-db", &second_snapshot),
    )
    .unwrap();

    assert_eq!(exact["changed_pins"][0]["name"], "ait-db");
    assert_eq!(
        exact["changed_pins"][0]["previous_snapshot"],
        first_snapshot
    );
    assert_eq!(exact["changed_pins"][0]["new_snapshot"], second_snapshot);
    assert_eq!(
        std::fs::read_to_string(repo_root.path().join(".ait-external/ait-db/db.txt")).unwrap(),
        "beta\n"
    );
    assert!(
        std::fs::read_to_string(repo_root.path().join("ait-external.toml"))
            .unwrap()
            .contains(&second_snapshot)
    );
}

#[test]
fn external_update_materializes_selected_binary_snapshot_without_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    let repo_root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo_root.path().join(".ait")).unwrap();
    std::fs::write(
        repo_root.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture-consumer","snapshot_binary_db_storage":"binary","remote_sync_binary_db_storage":"binary"}"#,
    )
    .unwrap();
    std::fs::write(repo_root.path().join("db.txt"), "binary external\n").unwrap();

    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let stores = repo.binary_db_stores::<TEST_LAYOUT>();
    stores
        .lines()
        .create_line("main", None, "2026-07-08T00:00:00Z")
        .expect("create Binary DB line");
    let snapshot_store = repo
        .local_snapshot_operation_store::<TEST_LAYOUT>(repo_root.path())
        .expect("selected Binary DB snapshot store");
    let snapshot = snapshot_store
        .create_snapshot(&repo.repo_name(), "main", Some("external binary"), false)
        .expect("create selected Binary DB external snapshot");
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    write_external_manifest(repo_root.path(), &repo.repo_name(), &snapshot_id);

    let updated = external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();

    assert_eq!(updated["command"], "external update");
    assert_eq!(
        std::fs::read_to_string(repo_root.path().join(".ait-external/ait-db/db.txt")).unwrap(),
        "binary external\n"
    );

    let link_target = tempfile::tempdir().unwrap();
    std::fs::write(
        repo_root.path().join(".ait-external/ait-db/db.txt"),
        "linked local data\n",
    )
    .unwrap();
    external_link(&repo, "ait-db", &link_target.path().to_string_lossy()).unwrap();

    let unlinked = external_unlink(&repo, "ait-db").unwrap();

    assert_eq!(unlinked["restored"], true);
    assert_eq!(
        std::fs::read_to_string(repo_root.path().join(".ait-external/ait-db/db.txt")).unwrap(),
        "binary external\n"
    );
}

#[test]
fn external_update_validate_reports_binding_validation_findings() {
    let repo_root = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: repo_root.path().to_path_buf(),
        name: Some("fixture-consumer".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let repo_root_text = repo.workspace_root().to_string_lossy().to_string();
    std::fs::write(repo_root.path().join("db.txt"), "alpha\n").unwrap();
    let snapshot = create_local_snapshot(
        &repo_root_text,
        &repo.repo_name(),
        "main",
        Some("external alpha"),
        false,
    )
    .unwrap();
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    write_external_manifest_with_python_binding(repo_root.path(), &repo.repo_name(), &snapshot_id);

    let validated = external_update(
        &repo,
        ExternalUpdateOptions::manifest_pins().with_validate(true),
    )
    .unwrap();

    assert_eq!(validated["validated"], true);
    assert_eq!(validated["states"]["validation_required"], false);
    assert_eq!(validated["validation"]["mode"], "toolchain_probes");
    assert_eq!(validated["validation"]["summary"]["passed"], true);
    assert_eq!(validated["validation"]["summary"]["warnings"], 1);
    assert_eq!(
        validated["validation"]["findings"][0]["code"],
        "external_binding_path_missing"
    );
    let text = render_external_update_text(&validated).unwrap();
    assert!(text.contains("validation: passed=true errors=0 warnings=1"));
    assert!(text.contains("validation findings:"));
}

#[test]
fn external_update_validate_failure_leaves_visible_authority_and_materialization_unchanged() {
    let repo_root = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: repo_root.path().to_path_buf(),
        name: Some("fixture-consumer".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let binding_root = repo_root.path().join("rust/crates/ait-db");
    std::fs::create_dir_all(&binding_root).unwrap();
    std::fs::write(binding_root.join("Cargo.toml"), "[package\nname = broken\n").unwrap();
    let snapshot = create_local_snapshot(
        &repo.workspace_root().to_string_lossy(),
        &repo.repo_name(),
        "main",
        Some("broken external binding"),
        false,
    )
    .unwrap();
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    write_external_manifest_with_rust_binding(repo_root.path(), &repo.repo_name(), &snapshot_id);
    let manifest_before = std::fs::read(repo_root.path().join("ait-external.toml")).unwrap();
    let links_path = repo_root.path().join(EXTERNAL_LINKS_FILE);
    std::fs::write(&links_path, "# preserve local-link authority bytes\n").unwrap();
    let links_before = std::fs::read(&links_path).unwrap();

    let err = external_update(
        &repo,
        ExternalUpdateOptions::manifest_pins().with_validate(true),
    )
    .unwrap_err();

    assert!(err.contains("external binding validation failed"), "{err}");
    assert_eq!(
        std::fs::read(repo_root.path().join("ait-external.toml")).unwrap(),
        manifest_before
    );
    assert_eq!(std::fs::read(links_path).unwrap(), links_before);
    assert!(!repo_root.path().join("ait-external.lock").exists());
    assert!(!repo_root.path().join(".ait-external").exists());
}

#[test]
fn external_update_locked_ignores_git_submodule_artifacts_on_valid_fixture_repo() {
    let (repo_root, repo, snapshot_id) = init_fixture_external_repo();
    write_external_manifest(repo_root.path(), &repo.repo_name(), &snapshot_id);
    seed_fake_git_submodule_artifacts(repo_root.path());
    external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();

    let locked = external_update(
        &repo,
        ExternalUpdateOptions::manifest_pins().with_locked(true),
    )
    .unwrap();

    assert_eq!(locked["command"], "external update");
    assert_eq!(locked["mode"], "locked");
    assert_eq!(locked["locked"], true);
    assert_eq!(locked["manifest_changed"], false);
    assert_eq!(locked["lockfile_changed"], false);
    assert_eq!(locked["changed_pins"].as_array().unwrap().len(), 0);
    assert_eq!(
        locked["materialization"]["entries"][0]["snapshot"],
        snapshot_id
    );
    assert_eq!(
        std::fs::read_to_string(repo_root.path().join(".gitmodules")).unwrap(),
        git_submodule_fixture_text()
    );
}

#[test]
fn task_worktree_external_update_materializes_the_active_workspace() {
    let (repo_root, repo, snapshot_id) = init_fixture_external_repo();
    write_external_manifest(repo_root.path(), &repo.repo_name(), &snapshot_id);
    external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();
    let canonical_external = repo_root.path().join(".ait-external/ait-db/db.txt");
    std::fs::write(&canonical_external, "canonical-only\n").unwrap();

    let worktree_root = repo_root.path().join("managed/task-one");
    std::fs::create_dir_all(&worktree_root).unwrap();
    std::os::unix::fs::symlink(repo_root.path().join(".ait"), worktree_root.join(".ait")).unwrap();
    std::fs::copy(
        repo_root.path().join("ait-external.toml"),
        worktree_root.join("ait-external.toml"),
    )
    .unwrap();
    std::fs::copy(
        repo_root.path().join("ait-external.lock"),
        worktree_root.join("ait-external.lock"),
    )
    .unwrap();
    std::fs::write(
        worktree_root.join(".ait-worktree.json"),
        json!({
            "repo_root": repo_root.path().to_string_lossy().to_string(),
            "workspace_root": worktree_root.to_string_lossy().to_string(),
            "worktree_name": "task-one",
        })
        .to_string(),
    )
    .unwrap();
    let worktree_repo = RepoRuntime::discover_from_path(&worktree_root).unwrap();

    let before = external_status(&worktree_repo).unwrap();
    assert_eq!(before["summary"]["missing"], 1);
    assert!(!worktree_root.join(".ait-external/ait-db").exists());

    let updated = external_update(
        &worktree_repo,
        ExternalUpdateOptions::manifest_pins().with_locked(true),
    )
    .unwrap();

    assert_eq!(updated["mode"], "locked");
    assert_eq!(
        std::fs::read_to_string(worktree_root.join(".ait-external/ait-db/db.txt")).unwrap(),
        "alpha\n"
    );
    assert_eq!(
        std::fs::read_to_string(&canonical_external).unwrap(),
        "canonical-only\n"
    );
    let after = external_status(&worktree_repo).unwrap();
    assert_eq!(after["summary"]["missing"], 0);
}

#[test]
fn external_doctor_ignores_git_submodule_artifacts_on_valid_fixture_repo() {
    let (repo_root, repo, snapshot_id) = init_fixture_external_repo();
    write_external_manifest(repo_root.path(), &repo.repo_name(), &snapshot_id);
    seed_fake_git_submodule_artifacts(repo_root.path());
    external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();

    let doctor = external_doctor(&repo).unwrap();

    assert_eq!(doctor["command"], "external doctor");
    assert_eq!(doctor["release_ready"], true);
    assert_eq!(doctor["summary"]["release_blocking"], 0);
    assert_eq!(doctor["summary"]["warnings"], 0);
    assert_eq!(doctor["summary"]["errors"], 0);
    assert_eq!(doctor["findings"].as_array().unwrap().len(), 0);
    assert!(repo_root
        .path()
        .join(".git/modules/ait-db/config")
        .is_file());
}

#[test]
fn external_update_locked_rejects_active_local_links() {
    let repo_root = tempfile::tempdir().unwrap();
    let link_target = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: repo_root.path().to_path_buf(),
        name: Some("fixture-consumer".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let repo_root_text = repo.workspace_root().to_string_lossy().to_string();
    std::fs::write(repo_root.path().join("db.txt"), "alpha\n").unwrap();
    let snapshot = create_local_snapshot(
        &repo_root_text,
        &repo.repo_name(),
        "main",
        Some("external alpha"),
        false,
    )
    .unwrap();
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    write_external_manifest(repo_root.path(), &repo.repo_name(), &snapshot_id);
    external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();
    external_link(&repo, "ait-db", &link_target.path().to_string_lossy()).unwrap();

    let err = external_update(
        &repo,
        ExternalUpdateOptions::manifest_pins().with_locked(true),
    )
    .unwrap_err();

    assert!(err.contains("local external links are not accepted"));
}

#[test]
fn external_update_respects_active_local_link_without_overwriting_materialization() {
    let repo_root = tempfile::tempdir().unwrap();
    let link_target = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: repo_root.path().to_path_buf(),
        name: Some("fixture-consumer".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let repo_root_text = repo.workspace_root().to_string_lossy().to_string();
    std::fs::write(repo_root.path().join("db.txt"), "alpha\n").unwrap();
    let snapshot = create_local_snapshot(
        &repo_root_text,
        &repo.repo_name(),
        "main",
        Some("external alpha"),
        false,
    )
    .unwrap();
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    write_external_manifest(repo_root.path(), &repo.repo_name(), &snapshot_id);
    external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();
    let materialized_file = repo_root.path().join(".ait-external/ait-db/db.txt");
    std::fs::write(&materialized_file, "linked local work\n").unwrap();
    external_link(&repo, "ait-db", &link_target.path().to_string_lossy()).unwrap();

    let linked_update = external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();

    assert_eq!(
        linked_update["materialization"]["entries"][0]["state"],
        "skipped_local_link"
    );
    assert_eq!(
        std::fs::read_to_string(materialized_file).unwrap(),
        "linked local work\n"
    );
}

#[test]
fn external_update_latest_remote_resolver_uses_declared_remote_line_head() {
    let manifest = ExternalManifest {
        externals: vec![ExternalDeclaration {
            name: "ait-db".to_string(),
            repo_name: "ait-db".to_string(),
            repository_index: 11,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: "SNP-DB-OLD".to_string(),
            materialize_to: ".ait-external/ait-db".to_string(),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: ExternalBindingSet::default(),
        }],
    };
    let local = FakeLocalExternalSnapshotResolver::default().with_snapshot_without_manifest(
        11,
        "ait-db",
        "SNP-DB-NEW",
    );
    let remote = FakeExternalRemoteLineHeadSource::default().with_head(
        11,
        "ait-db",
        "origin",
        "main",
        "SNP-DB-NEW",
    );
    let resolver = RemoteAwareExternalSnapshotResolver::new(local, remote);

    let lockfile = resolve_external_lockfile(
        &resolver,
        &manifest,
        &ExternalResolutionOptions::latest("ait-db"),
    )
    .unwrap();

    assert_eq!(lockfile.nodes.len(), 1);
    assert_eq!(lockfile.nodes[0].name, "ait-db");
    assert_eq!(lockfile.nodes[0].snapshot, "SNP-DB-NEW");
    assert_eq!(
        resolver.remote_line_heads.calls.borrow().as_slice(),
        [(
            11,
            "ait-db".to_string(),
            "origin".to_string(),
            "main".to_string()
        )]
    );
}

#[test]
fn staged_latest_validation_freezes_the_resolved_snapshot_for_the_visible_update() {
    let root = tempfile::tempdir().unwrap();
    write_external_manifest(root.path(), "ait-db", "SNP-DB-NEW");
    let store = FilesystemExternalUpdateStore::for_repo_root(root.path()).unwrap();

    let selection = freeze_staged_external_update_selection(
        &ExternalUpdateOptions::latest("ait-db").with_validate(true),
        &store,
    )
    .unwrap();

    assert_eq!(
        selection,
        ExternalUpdateSelection::exact("ait-db", "SNP-DB-NEW")
    );
}

#[test]
fn external_update_latest_remote_row_reader_accepts_line_name_or_name() {
    assert_eq!(
        line_head_from_remote_rows(
            &[json!({
                "line_name": "main",
                "head_snapshot_id": "SNP-MAIN"
            })],
            "main"
        )
        .as_deref(),
        Some("SNP-MAIN")
    );
    assert_eq!(
        line_head_from_remote_rows(
            &[json!({
                "name": "trunk",
                "head_snapshot_id": "SNP-TRUNK"
            })],
            "trunk"
        )
        .as_deref(),
        Some("SNP-TRUNK")
    );
}

#[test]
fn external_unlink_restores_pinned_snapshot_materialization() {
    let repo_root = tempfile::tempdir().unwrap();
    let link_target = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: repo_root.path().to_path_buf(),
        name: Some("fixture-consumer".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let repo_root_text = repo.workspace_root().to_string_lossy().to_string();
    std::fs::write(repo_root.path().join("db.txt"), "alpha\n").unwrap();
    let snapshot = create_local_snapshot(
        &repo_root_text,
        &repo.repo_name(),
        "main",
        Some("external alpha"),
        false,
    )
    .unwrap();
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    write_external_manifest(repo_root.path(), &repo.repo_name(), &snapshot_id);
    external_update(&repo, ExternalUpdateOptions::manifest_pins()).unwrap();
    let materialized_file = repo_root.path().join(".ait-external/ait-db/db.txt");
    std::fs::write(&materialized_file, "dirty linked data\n").unwrap();
    external_link(&repo, "ait-db", &link_target.path().to_string_lossy()).unwrap();

    let unlinked = external_unlink(&repo, "ait-db").unwrap();

    assert_eq!(unlinked["command"], "external unlink");
    assert_eq!(unlinked["restored"], true);
    assert_eq!(unlinked["restore_state"], "restored");
    assert_eq!(
        std::fs::read_to_string(materialized_file).unwrap(),
        "alpha\n"
    );
    assert!(!repo_root.path().join(EXTERNAL_LINKS_FILE).exists());
    let text = render_external_unlink_text(&unlinked).unwrap();
    assert!(text.contains("restored: true (restored)"));
    assert!(text.contains(&format!("ait-db [materialized] {snapshot_id}")));
}

#[test]
fn external_unlink_restore_failure_preserves_the_local_override() {
    let repo_root = tempfile::tempdir().unwrap();
    let link_target = tempfile::tempdir().unwrap();
    let repo = test_repo(repo_root.path());
    write_external_manifest(repo_root.path(), "fixture-consumer", "SNP-LINK-ONLY");
    external_link(&repo, "ait-db", &link_target.path().to_string_lossy()).unwrap();
    let links_before = std::fs::read(repo_root.path().join(EXTERNAL_LINKS_FILE)).unwrap();
    std::fs::write(
        repo_root.path().join("ait-external.lock"),
        "[[node]\nname = invalid\n",
    )
    .unwrap();

    let err = external_unlink(&repo, "ait-db").unwrap_err();

    assert!(err.contains("ait-external.lock"), "{err}");
    assert_eq!(
        std::fs::read(repo_root.path().join(EXTERNAL_LINKS_FILE)).unwrap(),
        links_before
    );
}

fn test_repo(root: &std::path::Path) -> RepoRuntime {
    RepoRuntime {
        root: root.to_path_buf(),
        ait_dir: root.join(".ait"),
        config: JsonMap::new(),
        worktree_config_path: None,
    }
}

fn init_fixture_external_repo() -> (tempfile::TempDir, RepoRuntime, String) {
    let repo_root = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: repo_root.path().to_path_buf(),
        name: Some("fixture-consumer".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root.path()).unwrap();
    let repo_root_text = repo.workspace_root().to_string_lossy().to_string();
    std::fs::write(repo_root.path().join("db.txt"), "alpha\n").unwrap();
    let snapshot = create_local_snapshot(
        &repo_root_text,
        &repo.repo_name(),
        "main",
        Some("external alpha"),
        false,
    )
    .unwrap();
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    (repo_root, repo, snapshot_id)
}

fn seed_fake_git_submodule_artifacts(root: &std::path::Path) {
    std::fs::create_dir_all(root.join(".git/modules/ait-db")).unwrap();
    std::fs::write(root.join(".gitmodules"), git_submodule_fixture_text()).unwrap();
    std::fs::write(
        root.join(".git/modules/ait-db/config"),
        "[core]\n\trepositoryformatversion = 0\n",
    )
    .unwrap();
}

fn git_submodule_fixture_text() -> &'static str {
    "[submodule \"ait-db\"]\n\tpath = .ait-external/ait-db\n\turl = ../ait-db\n"
}

fn write_external_manifest(root: &std::path::Path, repo_name: &str, snapshot_id: &str) {
    std::fs::write(
        root.join("ait-external.toml"),
        format!(
            r#"[[external]]
name = "ait-db"
repo_name = "{repo_name}"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "{snapshot_id}"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
"#
        ),
    )
    .unwrap();
}

fn write_external_manifest_with_python_binding(
    root: &std::path::Path,
    repo_name: &str,
    snapshot_id: &str,
) {
    std::fs::write(
        root.join("ait-external.toml"),
        format!(
            r#"[[external]]
name = "ait-db"
repo_name = "{repo_name}"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "{snapshot_id}"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"

[external.bindings.python]
kind = "python-path"
path = "python"
module = "ait_db"
"#
        ),
    )
    .unwrap();
}

fn write_external_manifest_with_rust_binding(
    root: &std::path::Path,
    repo_name: &str,
    snapshot_id: &str,
) {
    std::fs::write(
        root.join("ait-external.toml"),
        format!(
            r#"[[external]]
name = "ait-db"
repo_name = "{repo_name}"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "{snapshot_id}"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"

[external.bindings.rust]
kind = "cargo-path"
path = "rust/crates/ait-db"
package = "ait-db"
"#
        ),
    )
    .unwrap();
}

#[derive(Default)]
struct FakeLocalExternalSnapshotResolver {
    snapshots: BTreeSet<(u32, String, String)>,
    manifests: BTreeMap<(u32, String, String), ExternalManifest>,
    line_heads: BTreeMap<(u32, String, String, String), String>,
}

impl FakeLocalExternalSnapshotResolver {
    fn with_snapshot_without_manifest(
        mut self,
        repository_index: u32,
        repo_name: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.snapshots
            .insert((repository_index, repo_name.into(), snapshot.into()));
        self
    }
}

impl ExternalSnapshotResolver for FakeLocalExternalSnapshotResolver {
    fn snapshot_exists(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> ExternalResult<bool> {
        Ok(self.snapshots.contains(&(
            repository_index,
            repo_name.to_string(),
            snapshot.to_string(),
        )))
    }

    fn snapshot_available_from_remote(
        &self,
        _repository_index: u32,
        _repo_name: &str,
        _remote: &str,
        _snapshot: &str,
    ) -> ExternalResult<bool> {
        Ok(false)
    }

    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> ExternalResult<Option<String>> {
        Ok(self
            .line_heads
            .get(&(
                repository_index,
                repo_name.to_string(),
                remote.to_string(),
                line.to_string(),
            ))
            .cloned())
    }

    fn snapshot_manifest(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> ExternalResult<Option<ExternalManifest>> {
        Ok(self
            .manifests
            .get(&(
                repository_index,
                repo_name.to_string(),
                snapshot.to_string(),
            ))
            .cloned())
    }
}

#[derive(Default)]
struct FakeExternalRemoteLineHeadSource {
    heads: BTreeMap<(u32, String, String, String), String>,
    calls: RefCell<Vec<(u32, String, String, String)>>,
}

impl FakeExternalRemoteLineHeadSource {
    fn with_head(
        mut self,
        repository_index: u32,
        repo_name: impl Into<String>,
        remote: impl Into<String>,
        line: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.heads.insert(
            (
                repository_index,
                repo_name.into(),
                remote.into(),
                line.into(),
            ),
            snapshot.into(),
        );
        self
    }
}

impl ExternalRemoteLineHeadSource for FakeExternalRemoteLineHeadSource {
    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> Result<Option<String>, String> {
        self.calls.borrow_mut().push((
            repository_index,
            repo_name.to_string(),
            remote.to_string(),
            line.to_string(),
        ));
        Ok(self
            .heads
            .get(&(
                repository_index,
                repo_name.to_string(),
                remote.to_string(),
                line.to_string(),
            ))
            .cloned())
    }
}

#[derive(Default)]
struct FakeExternalUpdateHydrationPorts {
    snapshots: RefCell<BTreeSet<(u32, String, String)>>,
    complete_snapshots: RefCell<BTreeSet<(u32, String, String)>>,
    manifests: BTreeMap<(u32, String, String), ExternalManifest>,
    line_heads: BTreeMap<(u32, String, String, String), String>,
    imports: RefCell<Vec<(u32, String, String, String)>>,
}

impl FakeExternalUpdateHydrationPorts {
    fn with_importable_manifest(
        mut self,
        repository_index: u32,
        repo_name: impl Into<String>,
        snapshot: impl Into<String>,
        manifest: ExternalManifest,
    ) -> Self {
        self.manifests.insert(
            (repository_index, repo_name.into(), snapshot.into()),
            manifest,
        );
        self
    }

    fn with_incomplete_snapshot(
        self,
        repository_index: u32,
        repo_name: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.snapshots
            .borrow_mut()
            .insert((repository_index, repo_name.into(), snapshot.into()));
        self
    }

    fn with_line_head(
        mut self,
        repository_index: u32,
        repo_name: impl Into<String>,
        remote: impl Into<String>,
        line: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.line_heads.insert(
            (
                repository_index,
                repo_name.into(),
                remote.into(),
                line.into(),
            ),
            snapshot.into(),
        );
        self
    }
}

impl ExternalUpdateHydrationPorts for FakeExternalUpdateHydrationPorts {
    fn snapshot_content_complete(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> Result<bool, String> {
        Ok(self.complete_snapshots.borrow().contains(&(
            repository_index,
            repo_name.to_string(),
            snapshot.to_string(),
        )))
    }

    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .line_heads
            .get(&(
                repository_index,
                repo_name.to_string(),
                remote.to_string(),
                line.to_string(),
            ))
            .cloned())
    }

    fn snapshot_manifest(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> Result<Option<ExternalManifest>, String> {
        if !self.snapshots.borrow().contains(&(
            repository_index,
            repo_name.to_string(),
            snapshot.to_string(),
        )) {
            return Ok(None);
        }
        Ok(self
            .manifests
            .get(&(
                repository_index,
                repo_name.to_string(),
                snapshot.to_string(),
            ))
            .cloned())
    }

    fn import_snapshot(
        &mut self,
        repository_index: u32,
        remote: &str,
        repo_name: &str,
        snapshot: &str,
    ) -> Result<(), String> {
        self.imports.borrow_mut().push((
            repository_index,
            remote.to_string(),
            repo_name.to_string(),
            snapshot.to_string(),
        ));
        self.snapshots.borrow_mut().insert((
            repository_index,
            repo_name.to_string(),
            snapshot.to_string(),
        ));
        self.complete_snapshots.borrow_mut().insert((
            repository_index,
            repo_name.to_string(),
            snapshot.to_string(),
        ));
        Ok(())
    }
}

#[test]
fn external_update_hydration_latest_imports_missing_root_and_nested_snapshots() {
    let manifest = ExternalManifest {
        externals: vec![ExternalDeclaration {
            name: "ait-db".to_string(),
            repo_name: "ait-db".to_string(),
            repository_index: 11,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: "SNP-DB-OLD".to_string(),
            materialize_to: ".ait-external/ait-db".to_string(),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: ExternalBindingSet::default(),
        }],
    };
    let nested_manifest = ExternalManifest {
        externals: vec![ExternalDeclaration {
            name: "ait-codec".to_string(),
            repo_name: "ait-codec".to_string(),
            repository_index: 12,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: "SNP-CODEC-NEW".to_string(),
            materialize_to: ".ait-external/ait-codec".to_string(),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: ExternalBindingSet::default(),
        }],
    };
    let mut ports = FakeExternalUpdateHydrationPorts::default()
        .with_line_head(11, "ait-db", "origin", "main", "SNP-DB-NEW")
        .with_importable_manifest(11, "ait-db", "SNP-DB-NEW", nested_manifest);

    hydrate_external_update_selection_with_ports(
        &mut ports,
        &manifest,
        &ExternalUpdateSelection::latest("ait-db"),
    )
    .unwrap();

    assert_eq!(
        ports.imports.borrow().as_slice(),
        &[
            (
                11,
                "origin".to_string(),
                "ait-db".to_string(),
                "SNP-DB-NEW".to_string()
            ),
            (
                12,
                "origin".to_string(),
                "ait-codec".to_string(),
                "SNP-CODEC-NEW".to_string()
            ),
        ]
    );
}

#[test]
fn external_update_hydration_imports_existing_snapshot_with_incomplete_content() {
    let manifest = ExternalManifest {
        externals: vec![ExternalDeclaration {
            name: "ait-db".to_string(),
            repo_name: "ait-db".to_string(),
            repository_index: 11,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: "SNP-DB-ROW-ONLY".to_string(),
            materialize_to: ".ait-external/ait-db".to_string(),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: ExternalBindingSet::default(),
        }],
    };
    let mut ports = FakeExternalUpdateHydrationPorts::default().with_incomplete_snapshot(
        11,
        "ait-db",
        "SNP-DB-ROW-ONLY",
    );

    hydrate_external_update_selection_with_ports(
        &mut ports,
        &manifest,
        &ExternalUpdateSelection::ManifestPins,
    )
    .unwrap();

    assert_eq!(
        ports.imports.borrow().as_slice(),
        &[(
            11,
            "origin".to_string(),
            "ait-db".to_string(),
            "SNP-DB-ROW-ONLY".to_string()
        )]
    );
}
