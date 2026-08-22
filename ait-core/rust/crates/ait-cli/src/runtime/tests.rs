use super::*;
use ait_core::binary_db::BinaryDbCommandScope;
use ait_core::binary_db_generation::{
    activate_binary_db_generation, capture_binary_db_generation,
    BinaryDbGenerationActivationOptions, CaptureBinaryDbGenerationOptions,
};
use ait_core::content_binary_db::{
    blob_id_from_sha256, object_pack_id_from_hash48, snapshot_id_from_hash48,
    tree_pack_id_from_hash48, BinaryDbContentWriteCoordinator, BinaryDbObjectPackMemberWriteInput,
    BinaryDbObjectPackWriteInput, BinaryDbSnapshotWriteInput, BinaryDbTreeEntryWriteInput,
    BinaryDbTreePackTreeWriteInput, BinaryDbTreePackWriteInput, BinarySnapshotPayload,
    BinarySnapshotRecord,
};
use ait_core::json_support::json;
use ait_core::line_store::LineStore;
use ait_core::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotReadStore, LocalSnapshotTreeReadStore,
    LocalSnapshotWriteStore,
};
use ait_core::pack_substrate::{
    build_pack_members, build_tree_pack_members, default_object_pack_relative_path,
    default_tree_pack_relative_path, write_pack_archive_with_format,
    write_tree_pack_archive_with_format, DEFAULT_MAX_DELTA_CHAIN_DEPTH,
    PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use ait_core::plan_command_execution::execute_plan_list_command_request_json;
use ait_core::remote_sync_local_store::{
    RemoteSyncLocalInventorySource, RemoteSyncLocalSnapshotSource, RemoteSyncLocalStoreContext,
    RemoteSyncZstdLocalPlanSource,
};
use sha2::{Digest, Sha256};
use std::io::Write;
use tempfile::TempDir;

#[test]
fn binary_timestamp_projection_fails_closed_beyond_rfc3339_range() {
    assert_eq!(binary_created_at(0).unwrap(), "1970-01-01T00:00:00Z");
    assert!(binary_created_at(u64::MAX).is_err());
}

fn direct_activation_tempdir() -> TempDir {
    // Patchset CI intentionally places ordinary test scratch on a fast RAM
    // volume. Some filesystems there do not implement the atomic directory
    // exchange that direct Binary DB activation is required to exercise.
    // Keep these activation contract fixtures on the system temp filesystem.
    let system_tmp = Path::new("/tmp");
    if system_tmp.is_dir() {
        tempfile::Builder::new()
            .prefix("ait-direct-activation-test-")
            .tempdir_in(system_tmp)
            .unwrap()
    } else {
        TempDir::new().unwrap()
    }
}

fn write_file(path: &Path, content: &str) {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    let mut handle = fs::File::create(path).unwrap();
    handle.write_all(content.as_bytes()).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> ([u8; 32], String) {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    (digest, hex)
}

fn generated_worktree_cargo_config_text(root: &Path) -> String {
    let ait_dir = root.join(".ait");
    let shared_ait_dir = fs::canonicalize(&ait_dir).unwrap_or(ait_dir);
    let target_dir = shared_ait_dir
        .join("cargo-target")
        .join("task-workspaces")
        .join("lct-test");
    let build_dir = shared_ait_dir
        .join("cargo-build")
        .join("task-workspaces")
        .join("lct-test");
    format!(
        "# Managed by ait: workspace-isolated final artifacts and intermediates.\n[build]\ntarget-dir = \"{}\"\nbuild-dir = \"{}\"\n\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n",
        target_dir.to_string_lossy(),
        build_dir.to_string_lossy(),
    )
}

#[test]
fn activated_runtime_keeps_workspace_scan_root_and_repository_pack_root_distinct() {
    let temp = direct_activation_tempdir();
    let root = temp.path().join("repo");
    let generation = temp.path().join("generation");
    fs::create_dir_all(root.join(".ait/binary-db")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"fixture","snapshot_binary_db_storage":"binary","plan_binary_db_storage":"binary","remote_sync_binary_db_storage":"binary"}"#,
    );
    let ctx = RepoRuntime::discover_from(&root).unwrap();
    ctx.binary_db_stores::<1>()
        .content()
        .ensure_blob_bytes_content(b"before activation", Some("before.txt"))
        .unwrap();
    capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
        repo_root: root.clone(),
        output_root: generation.clone(),
        jobs: 7,
    })
    .unwrap();
    activate_binary_db_generation(BinaryDbGenerationActivationOptions {
        repo_root: root.clone(),
        generation_root: generation.clone(),
        expected_current_authority_fingerprint: None,
    })
    .unwrap();

    let ctx = RepoRuntime::discover_from(&root).unwrap();
    let workspace = root.join("task-workspace");
    fs::create_dir_all(&workspace).unwrap();
    let stores = ctx.binary_db_stores::<1>();
    let content = stores.content_for_root(workspace.clone());
    assert_eq!(content.workspace_root().as_path(), workspace);
    assert_eq!(content.pack_root().as_path(), root.canonicalize().unwrap());
    content
        .ensure_blob_bytes_content(b"after activation", Some("after.txt"))
        .unwrap();
    write_file(&workspace.join("workspace.txt"), "workspace snapshot\n");
    content
        .create_no_parent_snapshot_content("fixture", "main", Some("workspace"), false)
        .unwrap();
    assert!(!workspace.join(".ait/objects").exists());
    assert_eq!(
        fs::read_dir(root.join(".ait/objects/packs"))
            .unwrap()
            .count(),
        3
    );
    assert_eq!(
        fs::read_dir(root.join(".ait/objects/tree-packs"))
            .unwrap()
            .count(),
        1
    );
    // Activation retains the disconnected pre-migration archive physically for
    // rollback, but the canonical Binary DB counts only Snapshot-reachable packs.
    assert_eq!(
        ctx.repo_status_store().unwrap().storage_counts().unwrap(),
        ait_core::repo_status_store::RepoStatusStorageCounts {
            snapshot_count: 1,
            pack_count: 2,
            packed_blob_count: 2,
        }
    );
}

#[test]
fn activated_task_worktree_uses_canonical_repository_and_scans_only_its_workspace() {
    let temp = direct_activation_tempdir();
    let repo = temp.path().join("repo");
    let workspace = temp.path().join("worktree");
    let generation = temp.path().join("generation");
    fs::create_dir_all(repo.join(".ait/binary-db")).unwrap();
    write_file(
        &repo.join(".ait/config.json"),
        r#"{"repo_name":"fixture","snapshot_binary_db_storage":"binary"}"#,
    );
    RepoRuntime::discover_from(&repo)
        .unwrap()
        .binary_db_stores::<1>()
        .content()
        .ensure_blob_bytes_content(b"canonical seed", Some("seed.txt"))
        .unwrap();
    capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
        repo_root: repo.clone(),
        output_root: generation.clone(),
        jobs: 7,
    })
    .unwrap();
    activate_binary_db_generation(BinaryDbGenerationActivationOptions {
        repo_root: repo.clone(),
        generation_root: generation.clone(),
        expected_current_authority_fingerprint: None,
    })
    .unwrap();

    fs::create_dir_all(workspace.join(".ait")).unwrap();
    write_file(
        &workspace.join(".ait/config.json"),
        r#"{"repo_name":"fixture","snapshot_binary_db_storage":"binary"}"#,
    );
    write_file(
        &workspace.join(".ait-worktree.json"),
        &format!(
            r#"{{"repo_root":"{}","workspace_root":"{}","worktree_name":"rct-test"}}"#,
            repo.to_string_lossy(),
            workspace.to_string_lossy()
        ),
    );
    write_file(&workspace.join("worktree.txt"), "worktree content\n");

    let ctx = RepoRuntime::discover_from(&workspace).unwrap();
    let stores = ctx.binary_db_stores::<1>();
    let canonical_repo = repo.canonicalize().unwrap();
    assert_eq!(stores.repo_root(), repo.as_path());
    assert_eq!(
        stores.authority_root(),
        canonical_repo.join(".ait/binary-db").as_path()
    );
    assert_eq!(stores.pack_root(), canonical_repo.as_path());
    assert_eq!(stores.current_line_state_scope(), LocalStateScope::Task);

    let content = stores.content_for_root(ctx.workspace_root());
    assert_eq!(content.workspace_root().as_path(), workspace.as_path());
    content
        .create_no_parent_snapshot_content("fixture", "main", Some("worktree"), false)
        .unwrap();
    assert!(!workspace.join(".ait/binary-db").exists());
    assert!(!workspace.join(".ait/objects").exists());
    assert!(repo.join(".ait/binary-db/snapshot.bin").is_file());
    assert!(repo.join(".ait/objects/tree-packs").is_dir());
}

#[test]
fn discover_merges_worktree_overlay() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","default_remote":"origin","user_email":"root@example.com"}"#,
    );
    write_file(
        &root.join(".ait-worktree.json"),
        r#"{"user_email":"overlay@example.com","worktree_name":"rt-1"}"#,
    );
    let ctx = RepoRuntime::discover_from(root).unwrap();
    assert_eq!(ctx.repo_name(), "ait");
    assert_eq!(ctx.actor_identity().as_deref(), Some("overlay@example.com"));
    assert_eq!(ctx.default_remote_name().as_deref(), Some("origin"));
}

#[test]
fn runtime_remote_store_reads_repository_config_directly() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "ait",
  "default_remote": "origin",
  "remotes": {
    "origin": {
      "url": "http://example.test/ait",
      "repo_name": "ait"
    }
  }
}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let remote = ctx.remote_row(None).unwrap();
    assert_eq!(remote.name, "origin");
    assert_eq!(remote.url, "http://example.test/ait");
    assert_eq!(remote.repo_name.as_deref(), Some("ait"));
}

#[test]
fn binary_store_factory_uses_runtime_authority_without_retired_backend_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<1>();

    assert_eq!(stores.repo_root(), ctx.root.as_path());
    assert_eq!(stores.authority_root(), ctx.root.join(".ait/binary-db"));
    assert_eq!(stores.pack_root(), ctx.root.as_path());
    assert_eq!(stores.local_authority_id(), &AuthorityId::new("local:ait"));
}

#[test]
fn control_plane_store_decisions_cover_runtime_accessors() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let decisions = ctx.control_plane_store_decisions();
    let actual = decisions
        .iter()
        .map(|decision| {
            (
                decision.family.as_str(),
                decision.mode.as_str(),
                decision.owner_phase,
                decision.runtime_accessor,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        vec![
            (
                "line",
                "selected_binary_db",
                "global-binary-db-only-runtime-closeout",
                "line_store"
            ),
            (
                "current_line",
                "repository_config",
                "current-line-config-cutover",
                "current_line_name"
            ),
            (
                "stash",
                "selected_binary_db",
                "global-binary-db-only-runtime-closeout",
                "stash_store"
            ),
            (
                "remote",
                "repository_config",
                "remote-config-json-cutover",
                "remote_store"
            ),
            (
                "repo_status",
                "selected_binary_db",
                "global-binary-db-only-runtime-closeout",
                "repo_status_store"
            ),
        ]
    );
    for decision in decisions {
        match decision.mode {
            ControlPlaneStoreDecisionMode::SelectedBinaryDb => assert!(
                decision.reason.contains("stored only")
                    && decision
                        .reason
                        .contains("no alternate backend selector or fallback"),
                "{} decision must explain Binary DB-only authority",
                decision.family.as_str()
            ),
            ControlPlaneStoreDecisionMode::RepositoryConfig => assert!(
                decision.reason.contains("configuration")
                    && decision.reason.contains("never database authority"),
                "{} decision must explain repository config storage",
                decision.family.as_str()
            ),
        }
    }
}

#[test]
fn control_plane_store_decisions_json_is_stable() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let decisions = ctx.control_plane_store_decisions_json();
    let JsonValue::Array(rows) = decisions else {
        panic!("control-plane decisions should render as an array");
    };

    assert_eq!(rows.len(), CONTROL_PLANE_STORE_FAMILIES.len());
    assert_eq!(rows[0]["family"], "line");
    assert_eq!(rows[0]["label"], "LineStore");
    assert_eq!(rows[0]["mode"], "selected_binary_db");
    assert_eq!(
        rows[0]["owner_phase"],
        "global-binary-db-only-runtime-closeout"
    );
    assert_eq!(rows[0]["runtime_accessor"], "line_store");
    assert!(rows[0]["reason"].as_str().unwrap().contains("stored only"));
    let remote = rows
        .iter()
        .find(|row| row["family"] == "remote")
        .expect("remote store decision");
    assert_eq!(remote["mode"], "repository_config");
    assert_eq!(remote["owner_phase"], "remote-config-json-cutover");
    assert!(remote["reason"]
        .as_str()
        .unwrap()
        .contains(".ait/config.json"));
}

#[test]
fn binary_db_store_factory_uses_runtime_authority_root() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();

    assert_eq!(stores.repo_root(), ctx.root.as_path());
    assert_eq!(stores.authority_root(), ctx.root.join(".ait/binary-db"));
    assert_eq!(stores.local_authority_id(), &AuthorityId::new("local:ait"));
    assert_eq!(
        stores.current_line_state_scope(),
        LocalStateScope::Repository
    );

    let plans = stores.plans();
    assert_eq!(plans.authority_root().as_path(), stores.authority_root());

    let content = stores.content();
    assert_eq!(
        content.blobs().authority_root().as_path(),
        stores.authority_root()
    );
    assert_eq!(content.repo_root().as_path(), stores.repo_root());
}

#[test]
fn plan_binary_db_storage_request_defaults_to_binary_with_layout_metadata() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let request = ctx
        .plan_binary_db_storage_request::<TEST_LAYOUT>()
        .expect("plan storage request");

    assert!(request.get("mode").is_none());
    assert_eq!(request["write_layout"], 1);
    assert_eq!(
        request["authority_root"],
        ctx.root.join(".ait/binary-db").to_string_lossy().as_ref()
    );
    assert_eq!(request["repo_root"], ctx.root.to_string_lossy().as_ref());
    assert_eq!(request["local_authority_id"], "local:ait");
    assert_eq!(request["current_line_state_scope"], "repository");
}

#[test]
fn plan_binary_db_selected_command_request_admits_direct_authority_without_retired_backend_fallback(
) {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::create_dir_all(root.join(".ait/binary-db")).unwrap();
    fs::create_dir_all(root.join(".ait/objects")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","plan_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let plan_store = ctx.binary_db_stores::<TEST_LAYOUT>().plans();
    let mut write = plan_store
        .begin_write_txn(BinaryDbCommandScope::PlanSyncLocalPlan)
        .unwrap();
    plan_store
        .append_plan(
            &mut write,
            ait_core::plan_binary_db::PlanRecord {
                plan_meta: 0,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                latest_revision_index_plus1: 0,
                published_plan_index_plus1: 0,
                published_latest_revision_index_plus1: 0,
                created_at_s: 1,
                updated_at_s: 1,
                published_at_s: 0,
            },
            &ait_core::plan_binary_db::PlanPayload {
                title_bytes: b"fixture".to_vec(),
            },
        )
        .unwrap();
    write.commit().unwrap();
    let request = json!({
        "scope": "local",
        "repo_name": ctx.repo_name(),
        "plan_storage": ctx.plan_binary_db_storage_request::<TEST_LAYOUT>().expect("plan storage request"),
    });
    assert_eq!(
        request["plan_storage"]["activation_pointer"],
        ctx.authoritative_repo_root()
            .join(".ait/binary-db")
            .to_string_lossy()
            .as_ref()
    );

    let plans = execute_plan_list_command_request_json(&request.to_string())
        .expect("direct Binary DB authority should pass activation admission");
    assert_eq!(plans.as_array().map(Vec::len), Some(1));
}

#[test]
fn plan_binary_db_storage_request_ignores_removed_compare_read_mode() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","plan_binary_db_storage":"compare_read"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let request = ctx
        .plan_binary_db_storage_request::<TEST_LAYOUT>()
        .expect("plan storage request");

    assert!(request.get("mode").is_none());
    assert_eq!(request["write_layout"], 1);
}

#[test]
fn plan_binary_db_storage_request_ignores_removed_legacy_modes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","plan_binary_db_storage":"packed"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let request = ctx
        .plan_binary_db_storage_request::<1>()
        .expect("binary-only request");

    assert!(request.get("mode").is_none());
}

#[test]
fn snapshot_binary_db_storage_request_defaults_to_binary_with_layout_metadata() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let request = ctx
        .snapshot_binary_db_storage_request::<TEST_LAYOUT>()
        .expect("snapshot storage request");

    assert!(request.get("mode").is_none());
    assert_eq!(request["write_layout"], 1);
    assert_eq!(
        request["authority_root"],
        ctx.root.join(".ait/binary-db").to_string_lossy().as_ref()
    );
    assert_eq!(request["repo_root"], ctx.root.to_string_lossy().as_ref());
    assert_eq!(request["local_authority_id"], "local:ait");
    assert_eq!(request["current_line_state_scope"], "repository");
}

#[test]
fn snapshot_binary_db_storage_request_accepts_configured_binary_mode() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let request = ctx
        .snapshot_binary_db_storage_request::<TEST_LAYOUT>()
        .expect("snapshot storage request");

    assert!(request.get("mode").is_none());
    assert_eq!(request["write_layout"], 1);
}

#[test]
fn snapshot_binary_db_removed_modes_cannot_restore_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    for mode in ["dual_write", "compare_read"] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join(".ait")).unwrap();
        write_file(
            &root.join(".ait/config.json"),
            &format!(r#"{{"repo_name":"ait","snapshot_binary_db_storage":"{mode}"}}"#),
        );

        let ctx = RepoRuntime::discover_from(root).unwrap();
        ctx.local_snapshot_operation_store::<TEST_LAYOUT>(root)
            .expect("snapshot store remains Binary DB-only");

        ctx.local_content_maintenance_store::<TEST_LAYOUT>()
            .expect("maintenance store remains Binary DB-only");
    }
}

#[test]
fn snapshot_binary_db_selected_store_does_not_create_retired_backend() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    ctx.local_snapshot_operation_store::<1>(root)
        .expect("selected Binary DB snapshot store");
}

#[test]
fn local_content_maintenance_selected_binary_reads_stats_and_prunes_only_orphan_packs() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let content = ctx.binary_db_stores::<TEST_LAYOUT>().content();
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );

    let blob_bytes = b"binary gc stats\n";
    let (blob_sha, blob_sha_hex) = sha256_hex(blob_bytes);
    let blob_id = blob_id_from_sha256(&blob_sha);
    let object_pack_id = object_pack_id_from_hash48(0x0707_0808_0909);
    let object_pack_rel_path = default_object_pack_relative_path(&object_pack_id);
    let object_pack_abs_path = root.join(&object_pack_rel_path);
    fs::create_dir_all(object_pack_abs_path.parent().unwrap()).unwrap();
    let object_members = build_pack_members(
        &json!([{
            "entry_name": format!("blobs/{blob_id}"),
            "blob_id": blob_id.clone(),
            "data": blob_bytes,
            "path_hint": "file.txt",
        }]),
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        None,
    )
    .expect("object pack members");
    let object_archive_stats = write_pack_archive_with_format(
        object_pack_abs_path.to_string_lossy().as_ref(),
        &object_pack_id,
        "2026-07-08T00:00:00Z",
        &object_members,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write object pack archive");
    coordinator
        .record_object_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbObjectPackWriteInput {
                pack_id: object_pack_id.clone(),
                pack_rel_path: object_pack_rel_path,
                pack_format: PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                member_count: object_archive_stats["member_count"].as_i64().unwrap(),
                total_bytes: object_archive_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-08T00:00:00Z".to_string(),
                members: vec![BinaryDbObjectPackMemberWriteInput {
                    blob_id,
                    sha256: blob_sha_hex,
                    size_bytes: blob_bytes.len() as i64,
                    pack_entry_type: "full".to_string(),
                    pack_base_blob_id: None,
                    pack_chain_depth: 0,
                    created_at: "2026-07-08T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("record object pack metadata");

    let tree_id = "TRE-90919293949596979899".to_string();
    let tree_pack_id = tree_pack_id_from_hash48(0x0808_0909_0A0A);
    let tree_pack_rel_path = default_tree_pack_relative_path(&tree_pack_id);
    let tree_pack_abs_path = root.join(&tree_pack_rel_path);
    fs::create_dir_all(tree_pack_abs_path.parent().unwrap()).unwrap();
    let tree_members =
        build_tree_pack_members(&json!([{"tree_id": tree_id, "entry_count": 0}]), &json!([]))
            .expect("tree pack members");
    let tree_archive_stats = write_tree_pack_archive_with_format(
        tree_pack_abs_path.to_string_lossy().as_ref(),
        &tree_pack_id,
        "2026-07-08T00:00:00Z",
        &tree_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write tree pack archive");
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: tree_pack_id.clone(),
                pack_rel_path: tree_pack_rel_path,
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: tree_archive_stats["tree_count"].as_i64().unwrap(),
                total_bytes: tree_archive_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-08T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id,
                    entry_count: 0,
                }],
            },
        )
        .expect("record tree pack metadata");
    let snapshot_id = snapshot_id_from_hash48(0x0909_0A0A_0B0B);
    coordinator
        .record_snapshot(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id,
                parent_snapshot_ids: Vec::new(),
                root_tree_pack_id: tree_pack_id,
                root_entry_ordinal: 0,
                manifest_hash: "44".repeat(32),
                message: Some("binary gc stats".to_string()),
                line_name: "main".to_string(),
                snapshot_kind: "line".to_string(),
                file_count: 0,
                total_bytes: 0,
                created_at: "2026-07-08T00:00:00Z".to_string(),
            },
        )
        .expect("record snapshot metadata");

    let store = ctx
        .local_content_maintenance_store::<TEST_LAYOUT>()
        .expect("selected local content maintenance store");
    let stats = store.storage_stats().expect("selected Binary DB stats");
    assert_eq!(stats["storage_backend"], "binary_db");
    assert_eq!(stats["snapshot_count"], 1);
    assert_eq!(stats["packed_blob_count"], 1);
    assert_eq!(stats["pack_count"], 1);
    assert_eq!(stats["tree_pack_count"], 1);
    assert!(stats.get("inventory_included").is_none());
    assert_eq!(stats["reachability_summary"]["computed"], false);
    assert!(stats["reachable_blob_count"].is_null());
    assert_eq!(
        stats["validation_summary"]["state"],
        "reachability_not_computed"
    );
    assert!(stats.get("packs").is_none());
    assert!(stats.get("tree_packs").is_none());

    let validation = store.validate().expect("selected Binary DB validate");
    assert!(validation.get("state").is_some());
    let preview = store
        .preview_orphan_pack_prune()
        .expect("selected Binary DB orphan-pack preview");
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["applied"], false);
    assert_eq!(preview["candidate_orphan_pack_count"], 0);
    let prune = store
        .prune_orphan_packs()
        .expect("selected Binary DB orphan-pack prune");
    assert_eq!(prune["mode"], "apply");
    assert_eq!(prune["applied"], true);
    assert_eq!(prune["removed_orphan_pack_count"], 0);
}

#[test]
fn snapshot_binary_db_selected_create_missing_line_fails_before_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let store = ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected snapshot store");
    let err = store
        .create_snapshot("ait", "main", Some("binary"), false)
        .expect_err("selected Binary DB snapshot create with missing line must fail closed");

    assert!(err.contains("Current line does not exist: main"));
}

#[test]
fn stash_binary_db_selected_creates_compact_metadata_without_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );
    write_file(&root.join("stashed.txt"), "binary stash\n");

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();
    stores
        .lines()
        .create_line("main", None, "2026-07-12T00:00:00Z")
        .expect("create Binary DB line");

    let snapshot_store = ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected snapshot store");
    let snapshot = snapshot_store
        .create_snapshot("ait", "main", Some("binary stash"), false)
        .expect("create Binary DB stash snapshot");
    let snapshot_id = snapshot["snapshot_id"].as_str().expect("snapshot id");
    assert_eq!(
        snapshot_store
            .set_snapshot_kind(snapshot_id, "stash")
            .expect("mark Binary DB snapshot as stash"),
        1
    );

    let stash_store = ctx.stash_store().expect("selected Binary DB stash store");
    assert!(stash_store.list_stashes().unwrap().is_empty());
    assert!(!stores.authority_root().join("stash.bin").exists());
    let stash = stash_store
        .create_stash(ait_core::stash_store::NewStashRecord {
            stash_id: "STH-request-id-is-not-persisted",
            snapshot_id,
            source_line_name: "main",
            base_snapshot_id: None,
            message: Some("binary stash"),
            workspace_cleared: true,
            created_at: "2026-07-12T00:00:01Z",
        })
        .expect("write Binary DB stash metadata");

    assert_eq!(stash.stash_id, "STH-000001");
    assert_eq!(stash.snapshot_id, snapshot_id);
    assert!(stash.workspace_cleared);
    assert_eq!(stash_store.list_stashes().unwrap(), vec![stash]);
    assert_eq!(
        fs::metadata(stores.authority_root().join("stash.bin"))
            .unwrap()
            .len(),
        12
    );
}

#[test]
fn snapshot_binary_db_selected_create_writes_no_parent_snapshot_without_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );
    write_file(&root.join("hello.txt"), "hello from Binary DB\n");

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();
    stores
        .lines()
        .create_line("main", None, "2026-07-08T00:00:00Z")
        .expect("create Binary DB line");

    let store = ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected snapshot store");
    let payload = store
        .create_snapshot("ait", "main", Some("binary first"), false)
        .expect("selected Binary DB no-parent snapshot create");
    let snapshot_id = payload["snapshot_id"].as_str().expect("snapshot_id");
    let blob_id = payload["files"][0]["blob_id"].as_str().expect("blob_id");

    assert_eq!(payload["line_name"], "main");
    assert_eq!(payload["message"], "binary first");
    assert_eq!(payload["parent_snapshot_id"], JsonValue::Null);
    assert_eq!(payload["file_count"], 1);
    assert_eq!(payload["files"][0]["path"], "hello.txt");

    let read_back = store
        .get_snapshot(snapshot_id)
        .expect("read created snapshot");
    assert_eq!(read_back["snapshot_id"], snapshot_id);
    assert_eq!(read_back["files"][0]["path"], "hello.txt");
    let blob_bytes = store
        .read_blob_bytes(blob_id)
        .expect("read Binary DB created blob");
    assert_eq!(blob_bytes, b"hello from Binary DB\n");

    let line = stores
        .lines()
        .line_by_name("main")
        .expect("read Binary DB line")
        .expect("line exists");
    assert_eq!(line.head_snapshot_id.as_deref(), Some(snapshot_id));
}

#[test]
fn snapshot_binary_db_selected_create_reuses_existing_root_tree_without_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );
    write_file(&root.join("shared.txt"), "shared tree\n");

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();
    stores
        .lines()
        .create_line("main", None, "2026-07-08T00:00:00Z")
        .expect("create main Binary DB line");
    stores
        .lines()
        .create_line("feature", None, "2026-07-08T00:00:01Z")
        .expect("create feature Binary DB line");

    let store = ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected snapshot store");
    let main = store
        .create_snapshot("ait", "main", Some("main tree"), false)
        .expect("create first Binary DB snapshot");
    let feature = store
        .create_snapshot("ait", "feature", Some("feature same tree"), false)
        .expect("reuse existing Binary DB root tree");
    let main_id = main["snapshot_id"].as_str().expect("main snapshot id");
    let feature_id = feature["snapshot_id"]
        .as_str()
        .expect("feature snapshot id");

    assert_ne!(main_id, feature_id);
    assert_eq!(feature["parent_snapshot_id"], JsonValue::Null);
    assert_eq!(feature["root_tree_pack_id"], main["root_tree_pack_id"]);
    assert_eq!(feature["root_entry_ordinal"], main["root_entry_ordinal"]);
    assert_eq!(feature["files"][0]["path"], "shared.txt");

    let read_back = store
        .get_snapshot(feature_id)
        .expect("read reused-root snapshot");
    assert_eq!(read_back["snapshot_id"], feature_id);
    assert_eq!(read_back["files"][0]["path"], "shared.txt");
    let feature_line = stores
        .lines()
        .line_by_name("feature")
        .expect("read feature Binary DB line")
        .expect("feature line exists");
    assert_eq!(feature_line.head_snapshot_id.as_deref(), Some(feature_id));
}

#[test]
fn snapshot_binary_db_selected_create_writes_parent_snapshot_without_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );
    write_file(&root.join("hello.txt"), "first\n");

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();
    stores
        .lines()
        .create_line("main", None, "2026-07-08T00:00:00Z")
        .expect("create Binary DB line");
    let store = ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected snapshot store");
    let first = store
        .create_snapshot("ait", "main", Some("first"), false)
        .expect("create first Binary DB snapshot");
    let first_id = first["snapshot_id"].as_str().expect("first snapshot id");

    write_file(&root.join("hello.txt"), "second\n");
    let second = store
        .create_snapshot("ait", "main", Some("second"), false)
        .expect("create parent Binary DB snapshot");
    let second_id = second["snapshot_id"].as_str().expect("second snapshot id");
    let second_blob_id = second["files"][0]["blob_id"].as_str().expect("blob_id");

    assert_ne!(first_id, second_id);
    assert_eq!(second["parent_snapshot_id"], first_id);
    assert_eq!(second["message"], "second");
    assert_eq!(
        store
            .read_blob_bytes(second_blob_id)
            .expect("read second Binary DB blob"),
        b"second\n"
    );

    let delta = store
        .snapshot_tree_path_delta(Some(first_id), Some(second_id))
        .expect("read Binary DB snapshot delta");
    assert_eq!(
        delta.status_by_path.get("hello.txt").map(String::as_str),
        Some("modified")
    );
    let line = stores
        .lines()
        .line_by_name("main")
        .expect("read Binary DB line")
        .expect("line exists");
    assert_eq!(line.head_snapshot_id.as_deref(), Some(second_id));

    let err = store
        .create_snapshot("ait", "main", Some("unchanged"), false)
        .expect_err("unchanged parent tree should fail before mutation");
    assert!(err.contains("workspace tree is unchanged from parent snapshot"));
}

#[test]
fn snapshot_binary_db_selected_worktree_create_preserves_parent_cargo_config_without_retired_backend_fallback(
) {
    const TEST_LAYOUT: u32 = 1;
    const SOURCE_CARGO_CONFIG: &str = "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\njobs = 8\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n";
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );
    write_file(&root.join("main.txt"), "parent\n");
    write_file(&root.join(".cargo/config.toml"), SOURCE_CARGO_CONFIG);

    let parent_ctx = RepoRuntime::discover_from(root).unwrap();
    let parent_stores = parent_ctx.binary_db_stores::<TEST_LAYOUT>();
    parent_stores
        .lines()
        .create_line("main", None, "2026-07-08T00:00:00Z")
        .expect("create repository Binary DB line");
    let parent_store = parent_ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected repository snapshot store");
    let parent = parent_store
        .create_snapshot("ait", "main", Some("parent"), false)
        .expect("create parent Binary DB snapshot");
    let parent_id = parent["snapshot_id"].as_str().expect("parent snapshot id");

    write_file(
        &root.join(".ait-worktree.json"),
        r#"{"worktree_name":"lct-test","current_line":"main"}"#,
    );
    let generated_cargo_config = generated_worktree_cargo_config_text(root);
    write_file(&root.join(".cargo/config.toml"), &generated_cargo_config);
    write_file(&root.join("task.txt"), "worktree-only\n");
    write_file(
        &root.join("docs/notes.md"),
        "projected out of worktree snapshots\n",
    );

    let worktree_ctx = RepoRuntime::discover_from(root).unwrap();
    let worktree_stores = worktree_ctx.binary_db_stores::<TEST_LAYOUT>();
    worktree_stores
        .lines()
        .set_line_head("main", Some(parent_id), "2026-07-08T00:01:00Z")
        .expect("set task-scope Binary DB line head");
    let worktree_store = worktree_ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected worktree snapshot store");
    let child = worktree_store
        .create_snapshot("ait", "main", Some("worktree"), true)
        .expect("create worktree Binary DB snapshot");
    let child_id = child["snapshot_id"].as_str().expect("child snapshot id");

    assert_ne!(parent_id, child_id);
    assert_eq!(child["parent_snapshot_id"], parent_id);
    let files = child["files"].as_array().expect("files array");
    assert!(files
        .iter()
        .any(|file| file["path"].as_str() == Some("task.txt")));
    assert!(!files
        .iter()
        .any(|file| file["path"].as_str() == Some("docs/notes.md")));
    let cargo_entry = files
        .iter()
        .find(|file| file["path"].as_str() == Some(".cargo/config.toml"))
        .expect("parent cargo config entry should be preserved");
    let cargo_blob_id = cargo_entry["blob_id"]
        .as_str()
        .expect("cargo config blob id");
    let cargo_bytes = worktree_store
        .read_blob_bytes(cargo_blob_id)
        .expect("read preserved cargo config blob");
    assert_eq!(cargo_bytes, SOURCE_CARGO_CONFIG.as_bytes());
}

#[test]
fn snapshot_binary_db_selected_worktree_create_uses_materialized_parent_when_line_head_missing_without_retired_backend_fallback(
) {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );
    write_file(&root.join("main.txt"), "parent\n");

    let parent_ctx = RepoRuntime::discover_from(root).unwrap();
    let parent_stores = parent_ctx.binary_db_stores::<TEST_LAYOUT>();
    parent_stores
        .lines()
        .create_line("main", None, "2026-07-08T00:00:00Z")
        .expect("create repository Binary DB line");
    let parent_store = parent_ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected repository snapshot store");
    let parent = parent_store
        .create_snapshot("ait", "main", Some("parent"), false)
        .expect("create parent Binary DB snapshot");
    let parent_id = parent["snapshot_id"].as_str().expect("parent snapshot id");

    write_file(
        &root.join(".ait-worktree.json"),
        &format!(
            r#"{{"worktree_name":"lct-test","current_line":"main","materialized_snapshot_id":"{parent_id}"}}"#
        ),
    );
    write_file(&root.join("task.txt"), "worktree-only\n");

    let worktree_ctx = RepoRuntime::discover_from(root).unwrap();
    let worktree_stores = worktree_ctx.binary_db_stores::<TEST_LAYOUT>();
    worktree_stores
        .lines()
        .set_line_head("main", None, "2026-07-08T00:01:00Z")
        .expect("clear task-scope Binary DB line head");
    let worktree_store = worktree_ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected worktree snapshot store");
    let child = worktree_store
        .create_snapshot("ait", "main", Some("worktree"), true)
        .expect("create worktree Binary DB snapshot from materialized parent");
    let child_id = child["snapshot_id"].as_str().expect("child snapshot id");

    assert_ne!(parent_id, child_id);
    assert_eq!(child["parent_snapshot_id"], parent_id);
    let line = worktree_stores
        .lines()
        .line_by_name("main")
        .expect("read Binary DB line")
        .expect("line exists");
    assert_eq!(line.head_snapshot_id.as_deref(), Some(child_id));
}

#[test]
fn snapshot_binary_db_selected_get_line_reads_binary_line_store_without_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","snapshot_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();
    stores
        .lines()
        .append_line_for_bootstrap(
            "main",
            "active",
            Some("2026-07-08T00:00:00Z"),
            Some("2026-07-08T00:01:00Z"),
            None,
            None,
        )
        .expect("record Binary DB line metadata");

    let store = ctx
        .local_snapshot_operation_store::<TEST_LAYOUT>(root)
        .expect("selected snapshot store");
    let line = store
        .get_line("main")
        .expect("selected Binary DB snapshot line read");

    assert_eq!(line["line_name"], "main");
    assert_eq!(line["status"], "active");
    assert!(line["head_snapshot_id"].is_null());
    let err = store
        .get_line("missing")
        .expect_err("missing Binary DB line should remain a selected-mode domain error");
    assert!(err.contains("Unknown line: missing"));
}

#[test]
fn snapshot_binary_db_selected_line_commands_create_and_list_binary_lines_without_retired_backend()
{
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","current_line":"main","snapshot_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let store = ctx.line_store().expect("selected line store");
    store
        .create_line("main", None, "2026-07-14T00:00:00Z")
        .expect("create Binary DB main line");
    store
        .create_line("feature/rct-1000", None, "2026-07-14T00:00:01Z")
        .expect("create Binary DB feature line");

    let lines = store.list_lines().expect("list selected Binary DB lines");
    assert_eq!(
        lines
            .iter()
            .map(|line| line.line_name.as_str())
            .collect::<Vec<_>>(),
        vec!["main", "feature/rct-1000"]
    );
    assert_eq!(
        ctx.current_line_name().expect("config current line"),
        "main"
    );
}

#[test]
fn remote_sync_binary_db_storage_request_defaults_to_binary_with_layout_metadata() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let request = ctx
        .remote_sync_binary_db_storage_request::<TEST_LAYOUT>()
        .expect("remote sync storage request");

    assert!(request.get("mode").is_none());
    assert_eq!(request["write_layout"], 1);
    assert_eq!(
        request["authority_root"],
        ctx.root.join(".ait/binary-db").to_string_lossy().as_ref()
    );
    assert_eq!(request["repo_root"], ctx.root.to_string_lossy().as_ref());
    assert_eq!(request["local_authority_id"], "local:ait");
    assert_eq!(request["current_line_state_scope"], "repository");
}

#[test]
fn remote_sync_binary_db_storage_request_accepts_configured_binary_mode() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let request = ctx
        .remote_sync_binary_db_storage_request::<TEST_LAYOUT>()
        .expect("remote sync storage request");

    assert!(request.get("mode").is_none());
    assert_eq!(request["write_layout"], 1);
}

#[test]
fn remote_sync_binary_db_removed_modes_cannot_restore_retired_backend_fallback() {
    const TEST_LAYOUT: u32 = 1;
    for mode in ["dual_write", "compare_read"] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join(".ait")).unwrap();
        write_file(
            &root.join(".ait/config.json"),
            &format!(r#"{{"repo_name":"ait","remote_sync_binary_db_storage":"{mode}"}}"#),
        );

        let ctx = RepoRuntime::discover_from(root).unwrap();
        ctx.remote_sync_local_store::<TEST_LAYOUT>()
            .expect("remote sync store remains Binary DB-only");
    }
}

#[test]
fn remote_sync_binary_db_selected_inventory_reads_binary_metadata_without_retired_backend_fallback()
{
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let content = ctx.binary_db_stores::<TEST_LAYOUT>().content();
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );
    let blob_bytes = b"remote sync inventory\n";
    let (blob_sha, blob_sha_hex) = sha256_hex(blob_bytes);
    let blob_id = blob_id_from_sha256(&blob_sha);
    let object_pack_id = object_pack_id_from_hash48(0x0102_0304_0506);
    coordinator
        .record_object_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbObjectPackWriteInput {
                pack_id: object_pack_id.clone(),
                pack_rel_path: default_object_pack_relative_path(&object_pack_id),
                pack_format: PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                member_count: 1,
                total_bytes: 256,
                created_at: "2026-07-07T00:00:00Z".to_string(),
                members: vec![BinaryDbObjectPackMemberWriteInput {
                    blob_id,
                    sha256: blob_sha_hex,
                    size_bytes: blob_bytes.len() as i64,
                    pack_entry_type: "full".to_string(),
                    pack_base_blob_id: None,
                    pack_chain_depth: 0,
                    created_at: "2026-07-07T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("record object pack metadata");
    let tree_pack_id = tree_pack_id_from_hash48(0x0A0B_0C0D_0E0F);
    let tree_id = "TRE-0102030405060708090A".to_string();
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: tree_pack_id.clone(),
                pack_rel_path: default_tree_pack_relative_path(&tree_pack_id),
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 1,
                total_bytes: 128,
                created_at: "2026-07-07T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id,
                    entry_count: 0,
                }],
            },
        )
        .expect("record tree pack metadata");
    let snapshot_id = snapshot_id_from_hash48(0x0B0C_0D0E_0F10);
    coordinator
        .record_snapshot(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: snapshot_id.clone(),
                parent_snapshot_ids: Vec::new(),
                root_tree_pack_id: tree_pack_id,
                root_entry_ordinal: 0,
                manifest_hash: "00".repeat(32),
                message: Some("binary remote sync inventory".to_string()),
                line_name: "main".to_string(),
                snapshot_kind: "line".to_string(),
                file_count: 0,
                total_bytes: 0,
                created_at: "2026-07-07T00:00:00Z".to_string(),
            },
        )
        .expect("record snapshot metadata");

    let store = ctx
        .remote_sync_local_store::<TEST_LAYOUT>()
        .expect("selected remote sync store");
    let sync_ctx = RemoteSyncLocalStoreContext::new(root);
    let metadata = store
        .snapshot_inventory_metadata(&sync_ctx, &[snapshot_id])
        .expect("selected Binary DB remote sync inventory");

    assert_eq!(
        metadata.object_pack_formats,
        BTreeSet::from([PACK_FORMAT_ZSTD_CHUNKED_V1.to_string()])
    );
    assert_eq!(
        metadata.tree_pack_formats,
        BTreeSet::from([TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string()])
    );
}

#[test]
fn remote_sync_binary_db_snapshot_boundary_uses_only_committed_root_metadata() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let content = ctx.binary_db_stores::<TEST_LAYOUT>().content();
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );
    let blob_bytes = b"complete graph\n";
    let (blob_sha, blob_sha_hex) = sha256_hex(blob_bytes);
    let blob_id = blob_id_from_sha256(&blob_sha);
    let object_pack_id = object_pack_id_from_hash48(0x1111_2222_3333);
    let object_pack_rel_path = default_object_pack_relative_path(&object_pack_id);
    let object_pack_abs_path = root.join(&object_pack_rel_path);
    fs::create_dir_all(object_pack_abs_path.parent().unwrap()).unwrap();
    let object_members = build_pack_members(
        &json!([{
            "entry_name": format!("blobs/{blob_id}"),
            "blob_id": blob_id.clone(),
            "data": blob_bytes,
            "path_hint": "complete.txt",
        }]),
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        None,
    )
    .unwrap();
    let object_stats = write_pack_archive_with_format(
        object_pack_abs_path.to_string_lossy().as_ref(),
        &object_pack_id,
        "2026-07-15T00:00:00Z",
        &object_members,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    coordinator
        .record_object_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbObjectPackWriteInput {
                pack_id: object_pack_id.clone(),
                pack_rel_path: object_pack_rel_path,
                pack_format: PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                member_count: object_stats["member_count"].as_i64().unwrap(),
                total_bytes: object_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-15T00:00:00Z".to_string(),
                members: vec![BinaryDbObjectPackMemberWriteInput {
                    blob_id: blob_id.clone(),
                    sha256: blob_sha_hex,
                    size_bytes: blob_bytes.len() as i64,
                    pack_entry_type: "full".to_string(),
                    pack_base_blob_id: None,
                    pack_chain_depth: 0,
                    created_at: "2026-07-15T00:00:00Z".to_string(),
                }],
            },
        )
        .unwrap();

    let tree_pack_id = tree_pack_id_from_hash48(0x4444_5555_6666);
    let tree_id = "TRE-1234567890ABCDEF1234".to_string();
    let tree_pack_rel_path = default_tree_pack_relative_path(&tree_pack_id);
    let tree_pack_abs_path = root.join(&tree_pack_rel_path);
    fs::create_dir_all(tree_pack_abs_path.parent().unwrap()).unwrap();
    let tree_members = build_tree_pack_members(
        &json!([{"tree_id": tree_id, "entry_count": 1}]),
        &json!([{
            "tree_id": tree_id,
            "entry_name": "complete.txt",
            "entry_type": "blob",
            "target_id": blob_id,
            "size_bytes": blob_bytes.len(),
            "mode": "0o644",
        }]),
    )
    .unwrap();
    let tree_stats = write_tree_pack_archive_with_format(
        tree_pack_abs_path.to_string_lossy().as_ref(),
        &tree_pack_id,
        "2026-07-15T00:00:00Z",
        &tree_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    coordinator
        .record_tree_pack_metadata_with_entries(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: tree_pack_id.clone(),
                pack_rel_path: tree_pack_rel_path,
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: tree_stats["tree_count"].as_i64().unwrap(),
                total_bytes: tree_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-15T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: tree_id.clone(),
                    entry_count: 1,
                }],
            },
            &[BinaryDbTreeEntryWriteInput {
                tree_id,
                entry_name: "complete.txt".to_string(),
                entry_type: "blob".to_string(),
                target_id: blob_id,
                mode: "0o644".to_string(),
            }],
        )
        .unwrap();

    let correct_snapshot_id = snapshot_id_from_hash48(0x7777_8888_9999);
    let wrong_snapshot_id = snapshot_id_from_hash48(0x7777_8888_999A);
    for (snapshot_id, file_count) in [
        (correct_snapshot_id.clone(), 1),
        (wrong_snapshot_id.clone(), 2),
    ] {
        coordinator
            .record_snapshot(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbSnapshotWriteInput {
                    snapshot_id,
                    parent_snapshot_ids: Vec::new(),
                    root_tree_pack_id: tree_pack_id.clone(),
                    root_entry_ordinal: 0,
                    manifest_hash: "33".repeat(32),
                    message: None,
                    line_name: "main".to_string(),
                    snapshot_kind: "line".to_string(),
                    file_count,
                    total_bytes: blob_bytes.len() as i64,
                    created_at: "2026-07-15T00:00:00Z".to_string(),
                },
            )
            .unwrap();
    }

    // Boundary detection must not reopen either physical pack or traverse the
    // declared graph totals. Exact physical and descendant validation remains
    // the explicit `ait gc validate` responsibility.
    fs::remove_file(object_pack_abs_path).unwrap();
    fs::remove_file(tree_pack_abs_path).unwrap();

    let store = ctx.remote_sync_local_store::<TEST_LAYOUT>().unwrap();
    let sync_ctx = RemoteSyncLocalStoreContext::new(root);
    assert!(store
        .snapshot_content_complete(&sync_ctx, &correct_snapshot_id)
        .unwrap());
    assert!(store
        .snapshot_content_complete(&sync_ctx, &wrong_snapshot_id)
        .unwrap());
}

#[test]
fn remote_sync_binary_db_snapshot_boundary_fails_closed_without_committed_root() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();
    let content = stores.content();
    let snapshots = content.snapshots();
    let store = ctx.remote_sync_local_store::<TEST_LAYOUT>().unwrap();
    let sync_ctx = RemoteSyncLocalStoreContext::new(root);
    let missing_snapshot_id = snapshot_id_from_hash48(0x1111_0000_0001);
    assert!(!store
        .snapshot_content_complete(&sync_ctx, &missing_snapshot_id)
        .unwrap());

    let rootless_hash48 = 0x1111_0000_0002;
    let rootless_snapshot_id = snapshot_id_from_hash48(rootless_hash48);
    let rootless_record = BinarySnapshotRecord {
        snapshot_meta: 0,
        history_flags: 0,
        payload_len: 0,
        payload_offset: 0,
        snapshot_hash48: rootless_hash48,
        parent_snapshot_index_plus1: 0,
        root_tree_pack_index_plus1: 0,
        root_entry_ordinal: 0,
        line_index_plus1: 0,
        manifest_hash: [0x44; 32],
        file_count: 0,
        total_bytes: 0,
        created_at_s: 1_786_953_600,
    };
    let rootless_payload = BinarySnapshotPayload {
        line_name: "main".to_string(),
        message: None,
        additional_parent_snapshot_indices: Vec::new(),
    };

    let mut uncommitted = snapshots
        .begin_write_txn(BinaryDbCommandScope::ContentWrite)
        .unwrap();
    let (_, appended_id, _) = snapshots
        .append_snapshot_with_id_index(&mut uncommitted, rootless_record.clone(), &rootless_payload)
        .unwrap();
    assert_eq!(appended_id, rootless_snapshot_id);
    assert!(
        store
            .snapshot_content_complete(&sync_ctx, &rootless_snapshot_id)
            .is_err(),
        "an in-flight content write must never be reported as a complete boundary"
    );
    uncommitted.abort().unwrap();
    assert!(!store
        .snapshot_content_complete(&sync_ctx, &rootless_snapshot_id)
        .unwrap());

    let mut committed = snapshots
        .begin_write_txn(BinaryDbCommandScope::ContentWrite)
        .unwrap();
    snapshots
        .append_snapshot_with_id_index(&mut committed, rootless_record, &rootless_payload)
        .unwrap();
    committed.commit().unwrap();
    assert!(!store
        .snapshot_content_complete(&sync_ctx, &rootless_snapshot_id)
        .unwrap());
}

#[test]
fn remote_sync_binary_db_selected_snapshot_ordering_reads_line_reachable_metadata_without_retired_backend_fallback(
) {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();
    let content = stores.content();
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );
    let tree_pack_id = tree_pack_id_from_hash48(0x0A0B_0C0D_0E11);
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: tree_pack_id.clone(),
                pack_rel_path: default_tree_pack_relative_path(&tree_pack_id),
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 1,
                total_bytes: 128,
                created_at: "2026-07-08T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: "TRE-01020304050607080911".to_string(),
                    entry_count: 0,
                }],
            },
        )
        .expect("record tree pack metadata");
    let parent_snapshot_id = snapshot_id_from_hash48(0x0101_0101_0101);
    let child_snapshot_id = snapshot_id_from_hash48(0x0202_0202_0202);
    let orphan_snapshot_id = snapshot_id_from_hash48(0x0303_0303_0303);
    for (snapshot_id, parent_snapshot_id, created_at) in [
        (
            parent_snapshot_id.clone(),
            None,
            "2026-07-08T00:00:00Z".to_string(),
        ),
        (
            orphan_snapshot_id.clone(),
            None,
            "2026-07-08T00:00:30Z".to_string(),
        ),
        (
            child_snapshot_id.clone(),
            Some(parent_snapshot_id.clone()),
            "2026-07-08T00:01:00Z".to_string(),
        ),
    ] {
        coordinator
            .record_snapshot(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbSnapshotWriteInput {
                    snapshot_id,
                    parent_snapshot_ids: parent_snapshot_id.into_iter().collect(),
                    root_tree_pack_id: tree_pack_id.clone(),
                    root_entry_ordinal: 0,
                    manifest_hash: "22".repeat(32),
                    message: Some("binary remote sync ordering".to_string()),
                    line_name: "main".to_string(),
                    snapshot_kind: "line".to_string(),
                    file_count: 0,
                    total_bytes: 0,
                    created_at,
                },
            )
            .expect("record snapshot metadata");
    }

    stores
        .lines()
        .append_line_for_bootstrap(
            "main",
            "active",
            Some("2026-07-08T00:00:00Z"),
            Some("2026-07-08T00:01:01Z"),
            None,
            Some(&child_snapshot_id),
        )
        .expect("record line head metadata");

    let store = ctx
        .remote_sync_local_store::<TEST_LAYOUT>()
        .expect("selected remote sync store");
    let sync_ctx = RemoteSyncLocalStoreContext::new(root);
    let rows = store
        .snapshot_parent_rows(&sync_ctx)
        .expect("selected Binary DB remote sync snapshot ordering rows");

    assert_eq!(
        rows,
        vec![
            ait_core::remote_sync_local_store::RemoteSyncLocalSnapshotParent {
                snapshot_id: parent_snapshot_id.clone(),
                parent_snapshot_ids: Vec::new(),
                primary_parent_snapshot_id: None,
                parent_snapshot_id: None,
            },
            ait_core::remote_sync_local_store::RemoteSyncLocalSnapshotParent {
                snapshot_id: child_snapshot_id.clone(),
                parent_snapshot_ids: vec![parent_snapshot_id.clone()],
                primary_parent_snapshot_id: Some(parent_snapshot_id.clone()),
                parent_snapshot_id: Some(parent_snapshot_id),
            },
        ]
    );
    assert!(!rows.iter().any(|row| row.snapshot_id == orphan_snapshot_id));
}

#[test]
fn remote_sync_binary_db_selected_line_helpers_use_binary_metadata_without_retired_backend_fallback(
) {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let content = ctx.binary_db_stores::<TEST_LAYOUT>().content();
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );
    let tree_pack_id = tree_pack_id_from_hash48(0x0A0B_0C0D_0E12);
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: tree_pack_id.clone(),
                pack_rel_path: default_tree_pack_relative_path(&tree_pack_id),
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 1,
                total_bytes: 128,
                created_at: "2026-07-08T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: "TRE-01020304050607080912".to_string(),
                    entry_count: 0,
                }],
            },
        )
        .expect("record tree pack metadata");
    let parent_snapshot_id = snapshot_id_from_hash48(0x0404_0404_0404);
    let child_snapshot_id = snapshot_id_from_hash48(0x0505_0505_0505);
    for (snapshot_id, parent_snapshot_id, created_at) in [
        (
            parent_snapshot_id.clone(),
            None,
            "2026-07-08T00:00:00Z".to_string(),
        ),
        (
            child_snapshot_id.clone(),
            Some(parent_snapshot_id.clone()),
            "2026-07-08T00:01:00Z".to_string(),
        ),
    ] {
        coordinator
            .record_snapshot(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbSnapshotWriteInput {
                    snapshot_id,
                    parent_snapshot_ids: parent_snapshot_id.into_iter().collect(),
                    root_tree_pack_id: tree_pack_id.clone(),
                    root_entry_ordinal: 0,
                    manifest_hash: "33".repeat(32),
                    message: Some("binary remote sync line helper".to_string()),
                    line_name: "feature/sync".to_string(),
                    snapshot_kind: "line".to_string(),
                    file_count: 0,
                    total_bytes: 0,
                    created_at,
                },
            )
            .expect("record snapshot metadata");
    }

    let store = ctx
        .remote_sync_local_store::<TEST_LAYOUT>()
        .expect("selected remote sync store");
    assert!(store
        .line_by_name("feature/sync")
        .expect("read missing binary line")
        .is_none());

    let created = store
        .create_line(
            "feature/sync",
            Some(parent_snapshot_id.as_str()),
            "2026-07-08T00:02:00Z",
        )
        .expect("create selected Binary DB remote-sync line");
    assert_eq!(created["head_snapshot_id"], parent_snapshot_id);

    let moved = store
        .set_line_head(
            "feature/sync",
            Some(child_snapshot_id.as_str()),
            "2026-07-08T00:03:00Z",
        )
        .expect("move selected Binary DB remote-sync line head");
    assert_eq!(moved["head_snapshot_id"], child_snapshot_id);
    assert_eq!(
        store
            .line_by_name("feature/sync")
            .expect("read moved binary line")
            .expect("line exists")
            .head_snapshot_id
            .as_deref(),
        Some(child_snapshot_id.as_str())
    );
    assert!(store
        .snapshot_exists(&child_snapshot_id)
        .expect("selected Binary DB snapshot exists"));
    assert_eq!(
        store
            .snapshot_chain(&child_snapshot_id)
            .expect("selected Binary DB snapshot chain"),
        vec![parent_snapshot_id, child_snapshot_id]
    );
}

#[test]
fn remote_sync_binary_db_zstd_upload_orders_full_tree_pack_dependency_closure() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let content = ctx.binary_db_stores::<TEST_LAYOUT>().content();
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );
    let tree_pack_id = tree_pack_id_from_hash48(0x0101_0202_0303);
    let tree_id = "TRE-1112131415161718191A".to_string();
    let pack_only_tree_id = "TRE-00010203040506070809".to_string();
    let child_tree_pack_id = tree_pack_id_from_hash48(0xFFFF_FFFF_FFFE);
    let child_tree_id = "TRE-FFEEDDCCBBAA99887766".to_string();
    let child_tree_pack_rel_path = default_tree_pack_relative_path(&child_tree_pack_id);
    let child_tree_pack_abs_path = root.join(&child_tree_pack_rel_path);
    fs::create_dir_all(child_tree_pack_abs_path.parent().unwrap()).unwrap();
    let child_tree_members = build_tree_pack_members(
        &json!([{"tree_id": child_tree_id, "entry_count": 0}]),
        &json!([]),
    )
    .expect("child tree pack members");
    let child_tree_archive_stats = write_tree_pack_archive_with_format(
        child_tree_pack_abs_path.to_string_lossy().as_ref(),
        &child_tree_pack_id,
        "2026-07-08T00:00:00Z",
        &child_tree_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write child tree pack archive");
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: child_tree_pack_id.clone(),
                pack_rel_path: child_tree_pack_rel_path,
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: child_tree_archive_stats["tree_count"].as_i64().unwrap(),
                total_bytes: child_tree_archive_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-08T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: child_tree_id.clone(),
                    entry_count: 0,
                }],
            },
        )
        .expect("record child tree pack metadata");
    let tree_pack_rel_path = default_tree_pack_relative_path(&tree_pack_id);
    let tree_pack_abs_path = root.join(&tree_pack_rel_path);
    fs::create_dir_all(tree_pack_abs_path.parent().unwrap()).unwrap();
    let tree_members = build_tree_pack_members(
        &json!([
            {"tree_id": pack_only_tree_id, "entry_count": 1},
            {"tree_id": tree_id, "entry_count": 0}
        ]),
        &json!([{
            "tree_id": pack_only_tree_id,
            "entry_name": "tests",
            "entry_type": "tree",
            "target_id": child_tree_id,
            "size_bytes": 0,
            "mode": "0o040000"
        }]),
    )
    .expect("tree pack members");
    let tree_archive_stats = write_tree_pack_archive_with_format(
        tree_pack_abs_path.to_string_lossy().as_ref(),
        &tree_pack_id,
        "2026-07-08T00:00:00Z",
        &tree_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write tree pack archive");
    coordinator
        .record_tree_pack_metadata_with_entries(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: tree_pack_id.clone(),
                pack_rel_path: tree_pack_rel_path,
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: tree_archive_stats["tree_count"].as_i64().unwrap(),
                total_bytes: tree_archive_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-08T00:00:00Z".to_string(),
                trees: vec![
                    BinaryDbTreePackTreeWriteInput {
                        tree_id: pack_only_tree_id.clone(),
                        entry_count: 1,
                    },
                    BinaryDbTreePackTreeWriteInput {
                        tree_id: tree_id.clone(),
                        entry_count: 0,
                    },
                ],
            },
            &[BinaryDbTreeEntryWriteInput {
                tree_id: pack_only_tree_id.clone(),
                entry_name: "tests".to_string(),
                entry_type: "tree".to_string(),
                target_id: child_tree_id.clone(),
                mode: "0o040000".to_string(),
            }],
        )
        .expect("record tree pack metadata");
    let snapshot_id = snapshot_id_from_hash48(0x0202_0303_0404);
    coordinator
        .record_snapshot(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: snapshot_id.clone(),
                parent_snapshot_ids: Vec::new(),
                root_tree_pack_id: tree_pack_id.clone(),
                root_entry_ordinal: 1,
                manifest_hash: "11".repeat(32),
                message: Some("binary remote sync upload".to_string()),
                line_name: "main".to_string(),
                snapshot_kind: "line".to_string(),
                file_count: 0,
                total_bytes: 0,
                created_at: "2026-07-08T00:00:00Z".to_string(),
            },
        )
        .expect("record snapshot metadata");

    let store = ctx
        .remote_sync_local_store::<TEST_LAYOUT>()
        .expect("selected remote sync store");
    let sync_ctx = RemoteSyncLocalStoreContext::new(root);
    let plan = store
        .zstd_bulk_local_plan(
            &sync_ctx,
            std::slice::from_ref(&snapshot_id),
            &BTreeSet::new(),
        )
        .expect("selected Binary DB zstd upload plan");

    assert_eq!(plan.snapshot_order, vec![snapshot_id.clone()]);
    assert_eq!(
        plan.snapshots[&snapshot_id]["root_tree_pack_id"].as_str(),
        Some(tree_pack_id.as_str())
    );
    assert_eq!(
        plan.snapshots[&snapshot_id]["created_at"].as_str(),
        Some("2026-07-08T00:00:00Z")
    );
    assert_eq!(plan.tree_packs.len(), 2);
    assert_eq!(
        plan.tree_pack_order,
        vec![child_tree_pack_id.clone(), tree_pack_id.clone()]
    );
    assert_eq!(
        plan.tree_packs[&tree_pack_id].metadata["pack_format"].as_str(),
        Some(TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
    );
    assert_eq!(
        plan.tree_packs[&tree_pack_id].metadata["created_at"].as_str(),
        Some("2026-07-08T00:00:00Z")
    );
    assert_eq!(
        plan.tree_locators[&tree_id]["tree_pack_id"].as_str(),
        Some(tree_pack_id.as_str())
    );
    assert_eq!(
        plan.tree_locators[&pack_only_tree_id]["tree_pack_id"].as_str(),
        Some(tree_pack_id.as_str())
    );
    assert!(plan.object_packs.is_empty());
    assert!(plan.blob_locators.is_empty());
}

#[test]
fn remote_sync_binary_db_sparse_tree_pack_reports_physical_archive_count() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let content = ctx.binary_db_stores::<TEST_LAYOUT>().content();
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );
    let existing_tree_id = "TRE-11111111111111111111".to_string();
    let existing_pack_id = tree_pack_id_from_hash48(0x1111_1111_1111);
    let existing_pack_rel_path = default_tree_pack_relative_path(&existing_pack_id);
    let existing_pack_abs_path = root.join(&existing_pack_rel_path);
    fs::create_dir_all(existing_pack_abs_path.parent().unwrap()).unwrap();
    let existing_members = build_tree_pack_members(
        &json!([{"tree_id": existing_tree_id, "entry_count": 0}]),
        &json!([]),
    )
    .expect("existing tree-pack members");
    let existing_stats = write_tree_pack_archive_with_format(
        existing_pack_abs_path.to_string_lossy().as_ref(),
        &existing_pack_id,
        "2026-07-08T00:00:00Z",
        &existing_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write existing tree-pack archive");
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: existing_pack_id,
                pack_rel_path: existing_pack_rel_path,
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: existing_stats["tree_count"].as_i64().unwrap(),
                total_bytes: existing_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-08T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: existing_tree_id.clone(),
                    entry_count: 0,
                }],
            },
        )
        .expect("record existing tree-pack metadata");

    let sparse_pack_id = tree_pack_id_from_hash48(0x2222_2222_2222);
    let selected_tree_id = "TRE-00000000000000000001".to_string();
    let sparse_pack_rel_path = default_tree_pack_relative_path(&sparse_pack_id);
    let sparse_pack_abs_path = root.join(&sparse_pack_rel_path);
    let sparse_members = build_tree_pack_members(
        &json!([
            {"tree_id": selected_tree_id, "entry_count": 0},
            {"tree_id": existing_tree_id, "entry_count": 0}
        ]),
        &json!([]),
    )
    .expect("sparse tree-pack members");
    let sparse_stats = write_tree_pack_archive_with_format(
        sparse_pack_abs_path.to_string_lossy().as_ref(),
        &sparse_pack_id,
        "2026-07-08T00:01:00Z",
        &sparse_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write sparse tree-pack archive");
    coordinator
        .record_tree_pack_metadata_with_reachable_entries(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: sparse_pack_id.clone(),
                pack_rel_path: sparse_pack_rel_path,
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: sparse_stats["tree_count"].as_i64().unwrap(),
                total_bytes: sparse_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-08T00:01:00Z".to_string(),
                trees: vec![
                    BinaryDbTreePackTreeWriteInput {
                        tree_id: selected_tree_id.clone(),
                        entry_count: 0,
                    },
                    BinaryDbTreePackTreeWriteInput {
                        tree_id: existing_tree_id.clone(),
                        entry_count: 0,
                    },
                ],
            },
            &[],
            &BTreeSet::from([selected_tree_id.to_ascii_lowercase()]),
        )
        .expect("record sparse tree-pack metadata");
    let read = content.tree_packs().begin_read_txn();
    let sparse_pack = content
        .tree_packs()
        .get_tree_pack_view(&read, &sparse_pack_id)
        .expect("read sparse tree-pack metadata")
        .expect("sparse tree pack exists");
    assert!(sparse_pack.record.has_sparse_physical_ordinals());
    assert_eq!(sparse_pack.record.tree_count, 1);
    drop(read);

    let snapshot_id = snapshot_id_from_hash48(0x2222_3333_4444);
    coordinator
        .record_snapshot(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: snapshot_id.clone(),
                parent_snapshot_ids: Vec::new(),
                root_tree_pack_id: sparse_pack_id.clone(),
                root_entry_ordinal: 0,
                manifest_hash: "22".repeat(32),
                message: Some("sparse tree-pack remote sync".to_string()),
                line_name: "main".to_string(),
                snapshot_kind: "line".to_string(),
                file_count: 0,
                total_bytes: 0,
                created_at: "2026-07-08T00:01:00Z".to_string(),
            },
        )
        .expect("record sparse tree-pack snapshot");

    let store = ctx
        .remote_sync_local_store::<TEST_LAYOUT>()
        .expect("selected remote sync store");
    let sync_ctx = RemoteSyncLocalStoreContext::new(root);
    let plan = store
        .zstd_bulk_local_plan(
            &sync_ctx,
            std::slice::from_ref(&snapshot_id),
            &BTreeSet::new(),
        )
        .expect("sparse Binary DB zstd upload plan");

    assert_eq!(
        plan.tree_packs[&sparse_pack_id].metadata["tree_count"].as_i64(),
        Some(2)
    );
    assert_eq!(
        plan.tree_packs[&sparse_pack_id].metadata["pack_index"]["tree_count"].as_i64(),
        Some(2)
    );
    assert_eq!(plan.tree_locators.len(), 1);
    assert!(plan.tree_locators.contains_key(&selected_tree_id));
    assert!(!plan.tree_locators.contains_key(&existing_tree_id));
}

#[test]
fn remote_sync_binary_db_object_pack_upload_exports_full_pack_and_uses_identity_boundary() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"ait","remote_sync_binary_db_storage":"binary"}"#,
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let content = ctx.binary_db_stores::<TEST_LAYOUT>().content();
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );
    let blob_bytes = b"remote sync upload object\n";
    let (blob_sha, blob_sha_hex) = sha256_hex(blob_bytes);
    let blob_id = blob_id_from_sha256(&blob_sha);
    let second_blob_bytes = b"remote sync upload sibling object\n";
    let (second_blob_sha, second_blob_sha_hex) = sha256_hex(second_blob_bytes);
    let second_blob_id = blob_id_from_sha256(&second_blob_sha);
    let object_pack_id = object_pack_id_from_hash48(0x0303_0404_0505);
    let object_pack_rel_path = default_object_pack_relative_path(&object_pack_id);
    let object_pack_abs_path = root.join(&object_pack_rel_path);
    fs::create_dir_all(object_pack_abs_path.parent().unwrap()).unwrap();
    let object_members = build_pack_members(
        &json!([
            {
                "entry_name": format!("blobs/{blob_id}"),
                "blob_id": blob_id.clone(),
                "data": blob_bytes.as_slice(),
                "path_hint": "file.txt",
            },
            {
                "entry_name": format!("blobs/{second_blob_id}"),
                "blob_id": second_blob_id.clone(),
                "data": second_blob_bytes.as_slice(),
                "path_hint": "sibling.txt",
            }
        ]),
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        None,
    )
    .expect("object pack members");
    let object_archive_stats = write_pack_archive_with_format(
        object_pack_abs_path.to_string_lossy().as_ref(),
        &object_pack_id,
        "2026-07-08T00:00:00Z",
        &object_members,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write object pack archive");
    coordinator
        .record_object_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbObjectPackWriteInput {
                pack_id: object_pack_id.clone(),
                pack_rel_path: object_pack_rel_path,
                pack_format: PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                member_count: object_archive_stats["member_count"].as_i64().unwrap(),
                total_bytes: object_archive_stats["total_bytes"].as_i64().unwrap(),
                created_at: "2026-07-08T00:00:00Z".to_string(),
                members: vec![
                    BinaryDbObjectPackMemberWriteInput {
                        blob_id: blob_id.clone(),
                        sha256: blob_sha_hex,
                        size_bytes: blob_bytes.len() as i64,
                        pack_entry_type: "full".to_string(),
                        pack_base_blob_id: None,
                        pack_chain_depth: 0,
                        created_at: "2026-07-08T00:00:00Z".to_string(),
                    },
                    BinaryDbObjectPackMemberWriteInput {
                        blob_id: second_blob_id.clone(),
                        sha256: second_blob_sha_hex,
                        size_bytes: second_blob_bytes.len() as i64,
                        pack_entry_type: "full".to_string(),
                        pack_base_blob_id: None,
                        pack_chain_depth: 0,
                        created_at: "2026-07-08T00:00:00Z".to_string(),
                    },
                ],
            },
        )
        .expect("record object pack metadata");

    let sync_ctx = RemoteSyncLocalStoreContext::new(root);
    let mut object_packs = BTreeMap::new();
    let mut blob_locators = BTreeMap::new();
    let read = content.blobs().begin_read_txn();
    let object_pack_index = content
        .object_packs()
        .get_object_pack_view(&read, &object_pack_id)
        .expect("read object pack metadata")
        .expect("object pack metadata")
        .pack_index;
    binary_collect_zstd_object_pack(
        &sync_ctx,
        &read,
        content.blobs(),
        content.object_packs(),
        object_pack_index,
        &mut object_packs,
        &mut blob_locators,
    )
    .expect("collect Binary DB object pack upload metadata");

    assert_eq!(object_packs.len(), 1);
    assert_eq!(
        object_packs[&object_pack_id].metadata["pack_format"].as_str(),
        Some(PACK_FORMAT_ZSTD_CHUNKED_V1)
    );
    assert_eq!(
        blob_locators[&blob_id]["pack_entry_name"].as_str(),
        Some(format!("blobs/{blob_id}").as_str())
    );
    assert_eq!(
        blob_locators[&blob_id]["pack_id"].as_str(),
        Some(object_pack_id.as_str())
    );
    assert_eq!(
        blob_locators[&second_blob_id]["pack_id"].as_str(),
        Some(object_pack_id.as_str())
    );
    assert_eq!(
        object_packs[&object_pack_id].metadata["pack_index"]["entries"]
            .as_array()
            .expect("pack entries")
            .len(),
        blob_locators.len()
    );

    let blob_index = content
        .blobs()
        .get_blob_view(&read, &blob_id)
        .expect("read blob metadata")
        .expect("blob metadata")
        .blob_index;
    let mut seen_blobs = BTreeSet::new();
    let mut pack_indices = BTreeSet::new();
    binary_collect_blob_zstd_pack_closure_until_boundary(
        content.blobs(),
        content.object_packs(),
        &read,
        blob_index,
        &BTreeSet::from([blob_id.clone()]),
        &mut seen_blobs,
        &mut pack_indices,
    )
    .expect("identity boundary collection");
    assert!(pack_indices.is_empty());
}

#[test]
fn binary_db_store_factory_uses_task_scope_for_worktrees() {
    const TEST_LAYOUT: u32 = 1;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let authoritative_root = root.join("authoritative");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::create_dir_all(&authoritative_root).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);
    write_file(
        &root.join(".ait-worktree.json"),
        &format!(
            r#"{{"repo_root":"{}","worktree_name":"rt-1"}}"#,
            authoritative_root.to_string_lossy()
        ),
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<TEST_LAYOUT>();

    assert_eq!(stores.repo_root(), authoritative_root.as_path());
    assert_eq!(
        stores.authority_root(),
        authoritative_root.join(".ait/binary-db")
    );
    assert_eq!(stores.current_line_state_scope(), LocalStateScope::Task);
}

#[test]
fn binary_store_factory_uses_authoritative_repo_root() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let authoritative_root = root.join("authoritative");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::create_dir_all(&authoritative_root).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);
    write_file(
        &root.join(".ait-worktree.json"),
        &format!(
            r#"{{"repo_root":"{}","worktree_name":"rt-1"}}"#,
            authoritative_root.to_string_lossy()
        ),
    );

    let ctx = RepoRuntime::discover_from(root).unwrap();
    let stores = ctx.binary_db_stores::<1>();

    assert_eq!(stores.repo_root(), authoritative_root.as_path());
    assert_eq!(
        stores.authority_root(),
        authoritative_root.join(".ait/binary-db")
    );
    assert_eq!(
        stores.content().repo_root().as_path(),
        authoritative_root.as_path()
    );
}

#[test]
fn authoritative_repo_root_prefers_overlay_repo_root() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);
    write_file(
        &root.join(".ait-worktree.json"),
        r#"{"repo_root":"/tmp/ait-root","worktree_name":"rt-1"}"#,
    );
    let ctx = RepoRuntime::discover_from(root).unwrap();
    assert_eq!(
        ctx.authoritative_repo_root(),
        PathBuf::from("/tmp/ait-root")
    );
}

#[test]
fn workspace_root_prefers_overlay_workspace_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(root.join(".ait/config.json"), r#"{"repo_name":"fixture"}"#).unwrap();
    fs::write(
        root.join(".ait-worktree.json"),
        r#"{"workspace_root":"/tmp/ait-worktree","worktree_name":"rt-1"}"#,
    )
    .unwrap();
    let ctx = RepoRuntime::discover_from(root).unwrap();
    assert_eq!(ctx.workspace_root(), PathBuf::from("/tmp/ait-worktree"));
}

#[test]
fn team_review_enabled_only_in_team_remote_mode() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(
        root.join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"team_remote","workflow_default_scope":"remote","task_default_scope":"remote","change_default_scope":"remote"}"#,
    )
    .unwrap();
    let ctx = RepoRuntime::discover_from(root).unwrap();
    assert!(ctx.team_review_enabled());

    fs::write(
        root.join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_remote","workflow_default_scope":"remote","task_default_scope":"remote","change_default_scope":"remote"}"#,
    )
    .unwrap();
    let ctx = RepoRuntime::discover_from(root).unwrap();
    assert!(!ctx.team_review_enabled());
}

#[test]
fn change_scope_resolution_uses_change_default_and_explicit_solo_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(
        root.join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_default_scope":"remote","task_default_scope":"remote","change_default_scope":"local"}"#,
    )
    .unwrap();
    let ctx = RepoRuntime::discover_from(root).unwrap();

    assert!(ctx.change_uses_local_scope(false, None));
    assert!(ctx.change_uses_local_scope(true, None));
    assert!(!ctx.change_uses_local_scope(false, Some("origin")));
    assert!(!ctx.task_uses_local_scope(false, None).unwrap());
    assert!(ctx
        .task_uses_local_scope(true, Some("origin"))
        .unwrap_err()
        .contains("cannot be combined"));
}

#[test]
fn ai_code_review_identity_uses_executable_basename_without_human_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(
        root.join(".ait/config.json"),
        r#"{"repo_name":"fixture","user_name":"Alice Example","user_email":"alice@example.com"}"#,
    )
    .unwrap();
    let ctx = RepoRuntime::discover_from(root).unwrap();
    let identity = ctx.ai_code_review_reviewer_identity();
    assert!(identity.is_some());
    assert_ne!(identity.as_deref(), Some("custom-reviewer"));
    assert_ne!(
        identity.as_deref(),
        Some("Alice Example <alice@example.com>")
    );
}

#[test]
fn task_review_identity_uses_configured_user_name_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(
        root.join(".ait/config.json"),
        r#"{"repo_name":"fixture","user_name":"Alice Example","user_email":"alice@example.com"}"#,
    )
    .unwrap();
    let ctx = RepoRuntime::discover_from(root).unwrap();
    assert_eq!(
        ctx.task_review_reviewer_identity().as_deref(),
        Some("Alice Example")
    );
}

#[test]
fn actor_identity_prefers_configured_user_email_over_formatted_user_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(
        root.join(".ait/config.json"),
        r#"{"repo_name":"fixture","user_name":"Alice Example","user_email":"alice@example.com"}"#,
    )
    .unwrap();
    let ctx = RepoRuntime::discover_from(root).unwrap();
    assert_eq!(ctx.actor_identity().as_deref(), Some("alice@example.com"));
}
