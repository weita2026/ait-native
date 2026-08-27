use super::*;
use ait_core::binary_db::BinaryDbCommandScope;
use ait_core::external::lockfile::{ExternalLockCodec, ExternalLockfile, TomlExternalLockCodec};
use ait_core::external::manifest::{
    ExternalDeclaration, ExternalManifest, ExternalManifestCodec, TomlExternalManifestCodec,
};
use ait_core::external::materializer::{
    ExternalMaterializationOptions, ExternalMaterializer, FilesystemExternalMaterializer,
    FixtureExternalContentSource,
};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

fn direct_external_declaration(
    name: &str,
    repo_name: &str,
    snapshot: &str,
    materialize_to: &str,
) -> ExternalDeclaration {
    ExternalDeclaration {
        name: name.to_string(),
        repo_name: repo_name.to_string(),
        repository_index: 0,
        remote: "origin".to_string(),
        line: "main".to_string(),
        snapshot: snapshot.to_string(),
        materialize_to: materialize_to.to_string(),
        license: "Apache-2.0".to_string(),
        version: None,
        bindings: Default::default(),
    }
}

fn materialize_direct_external_fixture(repo_root: &Path, materialize_to: &str) -> ExternalLockfile {
    let manifest = ExternalManifest {
        externals: vec![direct_external_declaration(
            "ait-db",
            "ait-db",
            "SNP-DB-DIRECT",
            materialize_to,
        )],
    };
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).expect("direct lockfile");
    fs::write(
        repo_root.join("ait-external.toml"),
        TomlExternalManifestCodec
            .render_manifest(&manifest)
            .expect("render manifest"),
    )
    .expect("write manifest");
    fs::write(
        repo_root.join("ait-external.lock"),
        TomlExternalLockCodec
            .render_lockfile(&lockfile)
            .expect("render lockfile"),
    )
    .expect("write lockfile");
    FilesystemExternalMaterializer::new(repo_root, FixtureExternalContentSource)
        .expect("materializer")
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .expect("materialize lockfile");
    lockfile
}

fn markdown_guard_blob_id(text: &str) -> String {
    let sha256 = sha256_hex_bytes(text.as_bytes());
    format!("BLB-{}", &sha256[..20])
}

fn init_repo_with_tracked_markdown(
    artifact_path: &str,
    text: &str,
) -> (tempfile::TempDir, RepoRuntime) {
    init_repo_with_markdown_revision(artifact_path, text, Some(markdown_guard_blob_id(text)))
}

fn init_repo_with_markdown_revision(
    artifact_path: &str,
    text: &str,
    artifact_blob_id: Option<String>,
) -> (tempfile::TempDir, RepoRuntime) {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let config_path = repo_root.join(".ait/config.json");
    let mut config = ait_core::json_support::JsonCodec::parse_object(
        &fs::read_to_string(&config_path).expect("read fixture config"),
        "workspace CI fixture config",
    )
    .expect("parse fixture config");
    config.insert(
        "plan_binary_db_storage".to_string(),
        JsonValue::String("binary".to_string()),
    );
    fs::write(config_path, JsonValue::Object(config).to_string()).expect("write fixture config");
    let artifact_file = repo_root.join(artifact_path);
    fs::create_dir_all(artifact_file.parent().expect("artifact parent")).expect("artifact dir");
    fs::write(&artifact_file, text).expect("write markdown artifact");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let plans = repo.binary_db_stores::<1>().plans();
    let mut tx = plans.begin_local_upsert_txn().expect("begin plan fixture");
    type FixturePlanStore =
        ait_core::plan_binary_db::BinaryDbPlanStore<ait_core::binary_db::LocalBinaryDbFs, 1>;
    let plan_index = tx
        .record_count(FixturePlanStore::plan_file())
        .expect("plan fixture count");
    let revision_index = tx
        .record_count(FixturePlanStore::plan_revision_file())
        .expect("plan revision fixture count");
    tx.append_plan(
        ait_core::plan_binary_db::PlanRecord {
            plan_meta: 0,
            reserved0: 0,
            payload_len: 0,
            payload_offset: 0,
            latest_revision_index_plus1: revision_index + 1,
            published_plan_index_plus1: 0,
            published_latest_revision_index_plus1: 0,
            created_at_s: 1,
            updated_at_s: 1,
            published_at_s: 0,
        },
        &ait_core::plan_binary_db::PlanPayload {
            title_bytes: b"Guard Markdown".to_vec(),
        },
    )
    .expect("append plan fixture");
    tx.append_plan_revision_commit(
        ait_core::plan_binary_db::PlanRevisionRecord {
            revision_meta: 0,
            reserved0: 0,
            payload_len: 0,
            revision_number: 1,
            item_count: 0,
            payload_offset: 0,
            plan_index,
            previous_revision_index_plus1: 0,
            item_start_index: 0,
            published_revision_index_plus1: 0,
            root_tree_pack_index_plus1: 0,
            root_entry_ordinal: 0,
            created_at_s: 1,
            published_at_s: 0,
        },
        &ait_core::plan_binary_db::PlanRevisionPayload {
            title_snapshot_bytes: b"Guard Markdown".to_vec(),
            summary_bytes: b"guard baseline".to_vec(),
            artifact_path_bytes: artifact_path.as_bytes().to_vec(),
            artifact_selector_bytes: Vec::new(),
            artifact_heading_bytes: b"Guard Markdown".to_vec(),
            artifact_blob_id_bytes: artifact_blob_id.unwrap_or_default().into_bytes(),
        },
    )
    .expect("append plan revision fixture");
    tx.commit().expect("commit plan fixture");
    let read = plans.begin_read_txn();
    assert!(plans
        .list_plans(&read, Some("fixture-ait"), Some(artifact_path))
        .expect("read plan fixture")
        .iter()
        .any(|plan| plan.plan_index == plan_index));
    (repo_tmp, repo)
}

fn discover_markdown_guard_worktree(
    repo_root: &Path,
    worktree_name: &str,
) -> (PathBuf, RepoRuntime) {
    let worktree_root = repo_root.join(format!(".{worktree_name}-workspace"));
    fs::create_dir_all(&worktree_root).expect("worktree root");
    std::os::unix::fs::symlink(repo_root.join(".ait"), worktree_root.join(".ait"))
        .expect("symlink shared .ait");
    fs::write(
        worktree_root.join(WORKTREE_CONFIG_NAME),
        json!({
            "repo_root": repo_root.to_string_lossy(),
            "workspace_root": worktree_root.to_string_lossy(),
            "worktree_name": worktree_name,
            "current_line": format!("feature/{worktree_name}"),
        })
        .to_string(),
    )
    .expect("worktree config");
    let repo = RepoRuntime::discover_from_path(&worktree_root).expect("worktree runtime");
    assert!(repo.is_worktree());
    (worktree_root, repo)
}

fn assert_markdown_guard_error(surface: &str, err: String) {
    assert!(
        err.contains("authored Markdown drift"),
        "{surface} should fail on Markdown drift, got: {err}"
    );
    assert!(
        err.contains("ait plan sync docs/guard.md"),
        "{surface} should guide through plan sync, got: {err}"
    );
}

#[test]
fn planning_only_guard_allows_clean_tracked_markdown() {
    let (_repo_tmp, repo) = init_repo_with_tracked_markdown("docs/guard.md", "# Guard\n");

    guard_no_planning_only_artifact_drift(&repo, "ait snapshot create")
        .expect("clean tracked markdown should not block workflow authoring");
}

#[test]
fn planning_only_guard_rejects_tracked_markdown_drift() {
    let (repo_tmp, repo) = init_repo_with_tracked_markdown("docs/guard.md", "# Guard\n");
    let repo_root = repo_tmp.path();
    fs::write(repo_root.join("docs/guard.md"), "# Guard\n\nchanged\n").expect("dirty markdown");

    let err = guard_no_planning_only_artifact_drift(&repo, "ait snapshot create")
        .expect_err("dirty tracked markdown should block workflow authoring");
    assert_markdown_guard_error("shared guard", err);
}

#[test]
fn planning_only_guard_rejects_missing_blob_backed_markdown() {
    let (repo_tmp, repo) = init_repo_with_tracked_markdown("docs/guard.md", "# Guard\n");
    fs::remove_file(repo_tmp.path().join("docs/guard.md")).expect("remove tracked markdown");

    let err = guard_no_planning_only_artifact_drift(&repo, "ait snapshot create")
        .expect_err("missing blob-backed markdown should block workflow authoring");
    assert_markdown_guard_error("shared guard", err);
}

#[test]
fn planning_only_guard_ignores_legacy_revision_without_blob_evidence() {
    let (repo_tmp, repo) = init_repo_with_markdown_revision("docs/guard.md", "# Guard\n", None);
    fs::write(
        repo_tmp.path().join("docs/guard.md"),
        "# Guard\n\nchanged\n",
    )
    .expect("change legacy markdown");

    guard_no_planning_only_artifact_drift(&repo, "ait snapshot create")
        .expect("legacy revision without Blob evidence should not block workflow authoring");
}

#[test]
fn planning_only_guard_prefers_synced_worktree_markdown_over_stale_root() {
    let synced = "# Guard\n\nsynced worktree\n";
    let (repo_tmp, _root_repo) = init_repo_with_tracked_markdown("README.md", synced);
    let repo_root = repo_tmp.path();
    fs::write(repo_root.join("README.md"), "# Guard\n\nstale root\n").expect("stale root");
    let (worktree_root, worktree_repo) =
        discover_markdown_guard_worktree(repo_root, "rct-synced-markdown");
    fs::write(worktree_root.join("README.md"), synced).expect("synced worktree markdown");

    guard_no_planning_only_artifact_drift(&worktree_repo, "ait snapshot create")
        .expect("synced worktree Markdown should take precedence over a stale root copy");
}

#[test]
fn planning_only_guard_does_not_let_synced_root_hide_worktree_drift() {
    let synced = "# Guard\n\nsynced root\n";
    let (repo_tmp, _root_repo) = init_repo_with_tracked_markdown("README.md", synced);
    let (worktree_root, worktree_repo) =
        discover_markdown_guard_worktree(repo_tmp.path(), "rct-dirty-markdown");
    fs::write(
        worktree_root.join("README.md"),
        "# Guard\n\nunsynced worktree\n",
    )
    .expect("dirty worktree markdown");

    let err = guard_no_planning_only_artifact_drift(&worktree_repo, "ait snapshot create")
        .expect_err("a synced root copy must not hide worktree Markdown drift");
    assert!(
        err.contains("authored Markdown drift"),
        "unexpected error: {err}"
    );
    assert!(
        err.contains("ait plan sync README.md"),
        "unexpected error: {err}"
    );
}

#[test]
fn planning_only_guard_falls_back_to_synced_root_when_worktree_path_is_absent() {
    let synced = "# Guard\n\nshared root\n";
    let (repo_tmp, _root_repo) = init_repo_with_tracked_markdown("README.md", synced);
    let (_worktree_root, worktree_repo) =
        discover_markdown_guard_worktree(repo_tmp.path(), "rct-shared-markdown");

    guard_no_planning_only_artifact_drift(&worktree_repo, "ait snapshot create")
        .expect("an absent worktree path should fall back to the synced root artifact");
}

#[test]
fn markdown_drift_blocks_requested_mutating_workflow_surfaces() {
    let (repo_tmp, repo) = init_repo_with_tracked_markdown("docs/guard.md", "# Guard\n");
    fs::write(
        repo_tmp.path().join("docs/guard.md"),
        "# Guard\n\nchanged\n",
    )
    .expect("dirty markdown");

    assert_markdown_guard_error(
        "snapshot create",
        snapshot_create_in_current_workspace(&repo, Some("blocked"))
            .expect_err("snapshot should block"),
    );
    assert_markdown_guard_error(
        "task start initial change",
        crate::primitives::change_flow::change_create_for_worktree_bootstrap(
            &repo,
            "RCT-GUARD",
            "blocked",
            Some("main"),
            false,
            None,
        )
        .expect_err("initial change creation should block"),
    );
    assert_markdown_guard_error(
        "change close",
        change_close(&repo, "RCC-GUARD", false, None).expect_err("change close should block"),
    );
    assert_markdown_guard_error(
        "change publish",
        change_publish(&repo, "RCC-GUARD", None).expect_err("change publish should block"),
    );
    assert_markdown_guard_error(
        "patchset publish",
        patchset_publish(
            &repo,
            "RCC-GUARD",
            "blocked",
            Some("ai_with_human_review"),
            None,
        )
        .expect_err("patchset publish should block"),
    );
    assert_markdown_guard_error(
        "patchset publish explicit",
        patchset_publish_explicit(
            &repo,
            "RCC-GUARD",
            "SNP-BASE",
            "SNP-REV",
            "blocked",
            Some("ai_with_human_review"),
            None,
            None,
        )
        .expect_err("explicit patchset publish should block"),
    );
    assert_markdown_guard_error(
        "attest put",
        attest_put(
            &repo,
            Some("RCP-GUARD"),
            None,
            Some("pass"),
            None,
            None,
            None,
            Some("ai_with_human_review"),
            Some("gpt-5"),
            None,
            None,
        )
        .expect_err("attest put should block"),
    );
    assert_markdown_guard_error(
        "policy eval",
        policy_eval(&repo, "RCP-GUARD", None, None).expect_err("policy eval should block"),
    );
    assert_markdown_guard_error(
        "policy waive",
        policy_waive(&repo, "RCP-GUARD", "rule", "because", None, None, None)
            .expect_err("policy waive should block"),
    );
    assert_markdown_guard_error(
        "land submit",
        land_submit(&repo, "RCC-GUARD", Some("RCP-GUARD"), "main", "merge", None)
            .expect_err("land submit should block"),
    );
}

#[test]
fn markdown_drift_blocks_ready_but_not_remote_land_actions() {
    let (repo_tmp, repo) = init_repo_with_tracked_markdown("docs/guard.md", "# Guard\n");
    fs::write(
        repo_tmp.path().join("docs/guard.md"),
        "# Guard\n\nchanged\n",
    )
    .expect("dirty markdown");
    let state = json!({
        "patchset": {
            "patchset_id": "RCP-GUARD",
            "revision_snapshot_id": "SNP-REV"
        },
        "task": {
            "task_id": "RCT-GUARD"
        }
    });

    assert_markdown_guard_error(
        "workflow ready attestation",
        workflow_ready_apply_action(
            &repo,
            "record_attestation",
            &state,
            "RCC-GUARD",
            None,
            None,
            Some("pass"),
            None,
            None,
            None,
            Some("ai_with_human_review"),
            Some("gpt-5"),
            None,
        )
        .expect_err("workflow ready attestation should block"),
    );
    let error =
        workflow_land_apply_action(&repo, "evaluate_policy", &state, "RCC-GUARD", None, None)
            .expect_err("fixture has no remote, but land must pass the Markdown boundary first");
    assert!(
        !error.contains("authored Markdown drift"),
        "remote land policy evaluation must not read local Plan state: {error}"
    );
    assert!(
        error.to_ascii_lowercase().contains("remote"),
        "remote land policy evaluation should proceed to remote resolution: {error}"
    );
}

#[test]
fn remote_land_action_does_not_wait_for_plan_writer_lock() {
    let (_repo_tmp, repo) = init_repo_with_tracked_markdown("docs/guard.md", "# Guard\n");
    let plans = repo.binary_db_stores::<1>().plans();
    let plan_write = plans
        .begin_write_txn(BinaryDbCommandScope::PlanSyncLocalPlan)
        .expect("hold Plan writer lock");
    let state = json!({
        "patchset": {
            "patchset_id": "RCP-GUARD",
            "revision_snapshot_id": "SNP-REV"
        },
        "task": {
            "task_id": "RCT-GUARD"
        }
    });
    let (sender, receiver) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let result =
            workflow_land_apply_action(&repo, "evaluate_policy", &state, "RCC-GUARD", None, None);
        sender.send(result).expect("send land result");
    });

    let received = receiver.recv_timeout(Duration::from_secs(1));
    drop(plan_write);
    handle.join().expect("join land action");
    let error = received
        .expect("remote land action must not wait for the held Plan writer")
        .expect_err("fixture has no remote");
    assert!(
        !error.contains("authored Markdown drift"),
        "remote land must remain independent from Plan state: {error}"
    );
}

#[test]
fn workspace_delta_uses_worktree_ignore_hash_without_persisted_bin_cache() {
    let tmp = tempdir().expect("tempdir");
    let repo_root = tmp.path().join("repo");
    let worktree_root = tmp.path().join("worktree");
    fs::create_dir_all(repo_root.join(".ait")).expect("ait dir");
    fs::create_dir_all(worktree_root.join("src")).expect("worktree src dir");
    fs::write(
        repo_root.join(".ait/config.json"),
        r#"{"repo_name":"fixture-ait"}"#,
    )
    .expect("config");
    fs::write(worktree_root.join(".aitignore"), "target/\n").expect("aitignore");
    fs::write(worktree_root.join("src/lib.rs"), "pub fn fixture() {}\n").expect("source");
    std::os::unix::fs::symlink(repo_root.join(".ait"), worktree_root.join(".ait"))
        .expect("symlink .ait");
    fs::write(
        worktree_root.join(WORKTREE_CONFIG_NAME),
        format!(
            r#"{{"repo_root":"{}","workspace_root":"{}","worktree_name":"rct-cache","current_line":"feature/rct-cache"}}"#,
            repo_root.display(),
            worktree_root.display()
        ),
    )
    .expect("worktree config");
    let repo = RepoRuntime::discover_from_path(&worktree_root).expect("repo runtime");

    let effective_ignore_rules = effective_ignore_rules_text(&repo, None)
        .expect("effective rules")
        .expect("worktree rules");
    assert!(effective_ignore_rules.lines().any(|line| line == "/docs/"));
    let raw_hash = status_ignore_rules_hash(Some(&effective_ignore_rules));
    let manifest_hash = status_manifest_ignore_rules_hash(&repo, Some(&effective_ignore_rules));
    assert_ne!(raw_hash, manifest_hash);

    let payload = workspace_delta_payload(&repo, None, None).expect("workspace delta");

    assert_eq!(
        payload["baseline_manifest"]["ignore_rules_hash"],
        json!(manifest_hash)
    );
    assert_ne!(manifest_hash, raw_hash);
    assert_eq!(
        payload["phase_timings_ms"]["hashing_cache"]["state_read"],
        json!("miss")
    );
    assert_eq!(
        payload["phase_timings_ms"]["hashing_cache"]["state_write"],
        json!("read_only")
    );
    assert!(!worktree_root
        .join(".ait/workspace/status-workspace-state-v1.bin")
        .exists());
}

#[test]
fn snapshot_cache_accelerates_status_and_incremental_snapshot_without_becoming_authority() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("alpha.txt"), "alpha\n").expect("alpha");
    fs::write(repo_root.join("bravo.txt"), "bravo\n").expect("bravo");

    let baseline = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("cache baseline"),
        false,
    )
    .expect("baseline Snapshot");
    let baseline_id = required_string_field(&baseline, "snapshot_id").expect("baseline id");
    assert_eq!(
        baseline["phase_timings_ms"]["hashing_cache"]["state_write"],
        json!("written")
    );

    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let cache_path =
        ait_core::workspace_hash_cache::workspace_hash_cache_path(repo_root).expect("cache path");
    let cache_before_status = fs::read(&cache_path).expect("cache bytes");
    let clean = workspace_delta_payload(&repo, Some(&baseline_id), None).expect("clean status");
    assert_eq!(clean["clean"], json!(true));
    assert_eq!(
        clean["phase_timings_ms"]["hashing_cache"]["state_read"],
        json!("hit")
    );
    assert_eq!(
        clean["phase_timings_ms"]["hashing_cache"]["rehashed_paths"],
        json!(0)
    );
    assert!(
        clean["phase_timings_ms"]["hashing_cache"]["reused_paths"]
            .as_u64()
            .unwrap_or_default()
            >= 2
    );
    assert_eq!(
        fs::read(&cache_path).expect("cache after status"),
        cache_before_status,
        "a validated cache hit must not be rewritten"
    );

    fs::remove_file(&cache_path).expect("remove derived cache");
    let repaired = workspace_delta_payload(&repo, Some(&baseline_id), None)
        .expect("status repairs missing cache");
    assert_eq!(repaired["clean"], json!(true));
    assert_eq!(
        repaired["phase_timings_ms"]["hashing_cache"]["state_read"],
        json!("miss")
    );
    assert_eq!(
        repaired["phase_timings_ms"]["hashing_cache"]["rehashed_paths"],
        json!(2)
    );
    assert_eq!(
        repaired["phase_timings_ms"]["hashing_cache"]["state_write"],
        json!("written")
    );
    let warm = workspace_delta_payload(&repo, Some(&baseline_id), None)
        .expect("status reuses repaired cache");
    assert_eq!(
        warm["phase_timings_ms"]["hashing_cache"]["state_read"],
        json!("hit")
    );
    assert_eq!(
        warm["phase_timings_ms"]["hashing_cache"]["rehashed_paths"],
        json!(0)
    );
    assert!(
        warm["phase_timings_ms"]["hashing_cache"]["reused_paths"]
            .as_u64()
            .unwrap_or_default()
            >= 2
    );

    fs::write(repo_root.join("bravo.txt"), "BRAVO\n").expect("same-size edit");
    fs::remove_file(&cache_path).expect("remove cache before dirty status");
    let dirty = workspace_delta_payload(&repo, Some(&baseline_id), None).expect("dirty status");
    assert_eq!(dirty["modified_paths"], json!(["bravo.txt"]));
    assert_eq!(
        dirty["phase_timings_ms"]["hashing_cache"]["rehashed_paths"],
        json!(2)
    );
    assert_eq!(
        dirty["phase_timings_ms"]["hashing_cache"]["state_write"],
        json!("skipped_dirty")
    );
    assert!(
        !cache_path.exists(),
        "dirty status must not publish a workspace cache"
    );

    let incremental = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("incremental cache reuse"),
        false,
    )
    .expect("incremental Snapshot");
    let incremental_id =
        required_string_field(&incremental, "snapshot_id").expect("incremental id");
    assert_eq!(
        incremental["phase_timings_ms"]["hashing_cache"]["state_read"],
        json!("miss")
    );
    assert_eq!(
        incremental["phase_timings_ms"]["hashing_cache"]["rehashed_paths"],
        json!(2)
    );
    assert_eq!(
        incremental["phase_timings_ms"]["hashing_cache"]["reused_paths"],
        json!(0)
    );
    assert_eq!(
        incremental["phase_timings_ms"]["hashing_cache"]["state_write"],
        json!("written")
    );
    let reconstructed = snapshot_show(&repo, &incremental_id).expect("Snapshot readback");
    assert_eq!(reconstructed["snapshot_id"], json!(incremental_id));
    assert_eq!(reconstructed["file_count"], incremental["file_count"]);

    fs::write(&cache_path, b"{corrupt-cache").expect("corrupt derived cache");
    let corrupt_bytes = fs::read(&cache_path).expect("corrupt cache bytes");
    let fallback = workspace_delta_payload(&repo, Some(&incremental_id), None)
        .expect("corrupt cache fallback");
    assert_eq!(fallback["clean"], json!(true));
    assert_eq!(
        fallback["phase_timings_ms"]["hashing_cache"]["state_read"],
        json!("invalid_fallback")
    );
    assert_eq!(
        fallback["phase_timings_ms"]["hashing_cache"]["reused_paths"],
        json!(0)
    );
    assert_eq!(
        fallback["phase_timings_ms"]["hashing_cache"]["state_write"],
        json!("written")
    );
    assert_ne!(
        fs::read(&cache_path).expect("cache after fallback"),
        corrupt_bytes,
        "clean status must replace invalid derived cache bytes"
    );
    assert!(matches!(
        ait_core::workspace_hash_cache::load_workspace_hash_cache(repo_root, &incremental_id),
        ait_core::workspace_hash_cache::WorkspaceHashCacheLoad::Hit(_)
    ));
}

#[test]
fn workspace_delta_payload_projects_clean_external_roots_but_keeps_orphans_visible() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("tracked.txt"), "alpha\n").expect("tracked file");
    materialize_direct_external_fixture(repo_root, ".ait-external/ait-db");
    let baseline = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("baseline"),
        false,
    )
    .expect("create baseline snapshot");
    let baseline_snapshot_id =
        required_string_field(&baseline, "snapshot_id").expect("snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let clean = workspace_delta_payload(&repo, Some(&baseline_snapshot_id), None)
        .expect("clean workspace delta");
    assert_eq!(
        clean["baseline_manifest"]["source"],
        json!("validated_workspace_hash_cache")
    );
    assert_eq!(clean["changed_count"], json!(0));
    assert_eq!(
        clean["ignore_policy"]["external_materialization_roots"],
        json!([".ait-external/ait-db"])
    );
    assert_eq!(clean["untracked_paths"], json!([]));

    fs::create_dir_all(repo_root.join(".ait-external/orphan")).expect("orphan dir");
    fs::write(
        repo_root.join(".ait-external/orphan/orphan.txt"),
        "orphan\n",
    )
    .expect("orphan file");
    let orphan = workspace_delta_payload(&repo, Some(&baseline_snapshot_id), None)
        .expect("orphan workspace delta");

    assert_eq!(orphan["changed_count"], json!(1));
    assert_eq!(
        orphan["untracked_paths"],
        json!([".ait-external/orphan/orphan.txt"])
    );
}

#[test]
fn copy_seed_tree_reports_clonefile_when_clone_attempt_succeeds() {
    let tmp = tempdir().expect("tempdir");
    let source = tmp.path().join("source");
    let target = tmp.path().join("target");
    fs::create_dir_all(&source).expect("source dir");
    fs::write(source.join("alpha.txt"), "alpha").expect("seed file");

    let strategy = copy_seed_tree_with_file_copy(
        &source,
        &target,
        MAIN_SEED_COPY_EXCLUDE_NAMES,
        fake_clonefile_success,
    )
    .expect("copy seed tree");

    assert_eq!(strategy, "clonefile");
    assert_eq!(
        fs::read_to_string(target.join("alpha.txt")).expect("target file"),
        "alpha"
    );
}

#[test]
fn copy_seed_tree_reports_reflink_when_file_copy_reports_reflink() {
    let tmp = tempdir().expect("tempdir");
    let source = tmp.path().join("source");
    let target = tmp.path().join("target");
    fs::create_dir_all(&source).expect("source dir");
    fs::write(source.join("gamma.txt"), "gamma").expect("seed file");

    let strategy = copy_seed_tree_with_file_copy(
        &source,
        &target,
        MAIN_SEED_COPY_EXCLUDE_NAMES,
        fake_reflink_success,
    )
    .expect("copy seed tree");

    assert_eq!(strategy, "reflink");
    assert_eq!(
        fs::read_to_string(target.join("gamma.txt")).expect("target file"),
        "gamma"
    );
}

#[test]
fn copy_seed_tree_falls_back_to_copy2_when_file_copy_reports_copy2() {
    let tmp = tempdir().expect("tempdir");
    let source = tmp.path().join("source");
    let target = tmp.path().join("target");
    fs::create_dir_all(&source).expect("source dir");
    fs::write(source.join("beta.txt"), "beta").expect("seed file");

    let strategy = copy_seed_tree_with_file_copy(
        &source,
        &target,
        MAIN_SEED_COPY_EXCLUDE_NAMES,
        fake_copy2_fallback,
    )
    .expect("copy seed tree");

    assert_eq!(strategy, "copy2");
    assert_eq!(
        fs::read_to_string(target.join("beta.txt")).expect("target file"),
        "beta"
    );
}

#[test]
fn workflow_patchset_ci_contract_uses_single_catalog_path() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"fixture-ait"}"#);
    fs::create_dir_all(repo_root.join("ci")).expect("ci dir");
    fs::write(
        repo_root.join("ci").join("config.contract.json"),
        r#"{"schema_version":1,"ci":{"suite_manifest_path":"ci/patch_ci.json"}}"#,
    )
    .expect("contract");
    fs::write(
        repo_root.join("ci").join("patch_ci.json"),
        r#"{"suites":[]}"#,
    )
    .expect("catalog");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");

    assert!(workflow_patchset_ci_contract_exists(&repo));
    let hints = workflow_ready_command_hints(
        &repo,
        "RC-1",
        None,
        Some(&json!({"patchset_id":"RP-1"})),
        "main",
        None,
    );
    assert_eq!(
        hints["patchset_ci_command"],
        json!("ait patchset rerun-ci RP-1")
    );
    assert_eq!(
        hints["attestation_command"],
        json!("ait patchset rerun-ci RP-1")
    );
}

#[test]
fn workflow_patchset_ci_contract_ignores_legacy_suite_dir_without_catalog() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"fixture-ait"}"#);
    fs::create_dir_all(repo_root.join("ci").join("suites")).expect("legacy suite dir");
    fs::write(
        repo_root.join("ci").join("suites").join("preflight.json"),
        r#"{"suite_id":"preflight"}"#,
    )
    .expect("legacy suite");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");

    assert!(!workflow_patchset_ci_contract_exists(&repo));
    let hints = workflow_ready_command_hints(
        &repo,
        "RC-1",
        None,
        Some(&json!({"patchset_id":"RP-1"})),
        "main",
        None,
    );
    assert_eq!(hints["patchset_ci_command"], JsonValue::Null);
    assert_eq!(
        hints["attestation_command"],
        json!("ait attest put RP-1 --tests pass")
    );
}

#[test]
fn workflow_command_hints_preserve_explicit_remote_scope() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"fixture-ait"}"#);
    fs::create_dir_all(repo_root.join("ci")).expect("ci dir");
    fs::write(
        repo_root.join("ci").join("patch_ci.json"),
        r#"{"suites":[]}"#,
    )
    .expect("catalog");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let patchset = json!({"patchset_id":"RCT-9/C-01/P-02"});

    let ready = workflow_ready_command_hints(
        &repo,
        "RCT-9/C-01",
        Some("mirror"),
        Some(&patchset),
        "main",
        None,
    );
    assert_eq!(
        ready["apply_command"],
        json!("ait workflow ready RCT-9/C-01 --apply --remote mirror")
    );
    assert_eq!(
        ready["patchset_ci_command"],
        json!("ait patchset rerun-ci RCT-9/C-01/P-02 --remote mirror")
    );
    assert_eq!(ready["review_command"], JsonValue::Null);
    assert_eq!(ready["manual_review_command"], JsonValue::Null);
    assert_eq!(ready["code_review_summary_command"], JsonValue::Null);
    assert_eq!(ready["policy_command"], JsonValue::Null);
    assert_eq!(
        ready["land_command"],
        json!("ait workflow finish RCT-9/C-01 --apply --remote mirror")
    );

    let land = workflow_land_command_hints(
        &repo,
        "RCT-9/C-01",
        Some("mirror"),
        "RCT-9",
        Some(&patchset),
        "main",
        "main",
        None,
        1,
        true,
    );
    assert_eq!(
        land["apply_command"],
        json!("ait workflow finish RCT-9/C-01 --apply --remote mirror")
    );
    assert_eq!(
        land["ready_command"],
        json!("ait workflow ready RCT-9/C-01 --apply --remote mirror")
    );
    assert_eq!(
        land["review_command"],
        json!("ait review show RCT-9/C-01 --remote mirror")
    );
    assert_eq!(
        land["task_land_command"],
        json!("ait task finish RCT-9/C-01 --remote mirror")
    );

    let local =
        workflow_ready_command_hints(&repo, "LCT-9/C-01", None, Some(&patchset), "main", None);
    assert_eq!(
        local["land_command"],
        json!("ait workflow finish LCT-9/C-01 --apply")
    );
    assert!(!local["apply_command"]
        .as_str()
        .expect("local apply command")
        .contains("--remote"));
}

#[test]
fn workflow_review_hints_distinguish_required_and_automatic_task_review_owners() {
    let required_tmp = tempdir().expect("required repo tempdir");
    write_runtime_config(
        required_tmp.path(),
        r#"{"repo_name":"fixture-ait","task_review":true,"user_name":"Alice Example"}"#,
    );
    let required_repo =
        RepoRuntime::discover_from_path(required_tmp.path()).expect("required repo runtime");
    let patchset = json!({"patchset_id":"RCT-9/C-01/P-02"});
    let required = workflow_land_command_hints(
        &required_repo,
        "RCT-9/C-01",
        Some("mirror"),
        "RCT-9",
        Some(&patchset),
        "main",
        "main",
        None,
        0,
        false,
    );
    assert_eq!(
        required["review_command"],
        json!("ait review task approve RCT-9/C-01 --patchset RCT-9/C-01/P-02 --message \"<functional validation>\" --remote mirror")
    );
    assert_eq!(required["auto_review_reviewer"], JsonValue::Null);

    let automatic_tmp = tempdir().expect("automatic repo tempdir");
    write_runtime_config(
        automatic_tmp.path(),
        r#"{"repo_name":"fixture-ait","task_review":false,"user_name":"Alice Example"}"#,
    );
    let automatic_repo =
        RepoRuntime::discover_from_path(automatic_tmp.path()).expect("automatic repo runtime");
    let automatic = workflow_land_command_hints(
        &automatic_repo,
        "RCT-9/C-01",
        Some("mirror"),
        "RCT-9",
        Some(&patchset),
        "main",
        "main",
        None,
        0,
        true,
    );
    assert_eq!(automatic["auto_review_reviewer"], json!("Alice Example"));
    assert_eq!(automatic["manual_review_command"], JsonValue::Null);
    assert_eq!(
        automatic["review_command"],
        json!("ait workflow finish RCT-9/C-01 --apply --remote mirror")
    );
    assert_ne!(
        automatic["review_command"],
        json!("ait task finish RCT-9/C-01 --remote mirror")
    );
}
