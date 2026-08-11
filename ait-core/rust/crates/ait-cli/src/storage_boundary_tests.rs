use crate::runtime::RepoRuntime;
use crate::workspace_test_support;
use ait_core::binary_db::{REPOSITORY_BINARY_DB_BIN_PATHS, REPOSITORY_BINARY_DB_INDEX_PATHS};
use ait_core::content_binary_db::{
    BLOB_BIN, BLOB_ID_IDX, OBJECT_PACK_BIN, OBJECT_PACK_ID_IDX, OBJECT_PACK_MEMBER_BIN,
    SNAPSHOT_BIN, SNAPSHOT_ID_IDX, SNAPSHOT_PAYLOAD_BIN, TREE_BIN, TREE_ID_IDX, TREE_PACK_BIN,
    TREE_PACK_ID_IDX,
};
use ait_core::line_binary_db::{LINE_BIN, LINE_NAME_IDX, LINE_NAME_PAYLOAD_BIN};
use ait_core::plan_binary_db::{
    PLAN_BIN, PLAN_ITEM_BIN, PLAN_ITEM_PAYLOAD_BIN, PLAN_PAYLOAD_BIN, PLAN_REVISION_BIN,
    PLAN_REVISION_PAYLOAD_BIN,
};
use ait_core::plan_command_execution::execute_plan_list_command_request_json;
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
use ait_core::stash_binary_db::STASH_BIN;
use ait_core::task_workflow_shared_foundation::task_workflow_runtime_selection_facts;
use ait_core::workflow_binary_db::{
    CHANGE_LAND_INDEX_BIN, CHANGE_PAYLOAD_BIN, CHANGE_RECORD_BIN, LAND_RECORD_BIN,
    TASK_CHANGE_INDEX_BIN, TASK_LAND_INDEX_BIN, TASK_PAYLOAD_BIN, TASK_RECORD_BIN,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn rust_root() -> PathBuf {
    workspace_test_support::rust_workspace_root()
}

fn read(relative_path: &str) -> String {
    fs::read_to_string(rust_root().join(relative_path))
        .unwrap_or_else(|error| panic!("failed to read {relative_path}: {error}"))
}

fn rust_source_tree(root: &Path) -> String {
    fn append(path: &Path, output: &mut String) {
        let mut entries = fs::read_dir(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                append(&path, output);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                output.push_str(&fs::read_to_string(&path).unwrap());
            }
        }
    }

    let mut output = String::new();
    append(root, &mut output);
    output
}

fn trait_source<'a>(source: &'a str, trait_name: &str) -> &'a str {
    let marker = format!("pub trait {trait_name}");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("missing trait {trait_name}"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("trait {trait_name} has no body"));
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("trait {trait_name} body is unterminated")
}

#[test]
fn current_generation_library_replaces_the_retired_migration_program() {
    let core: toml::Value = toml::from_str(&read("crates/ait-core/Cargo.toml")).unwrap();
    let features = core["features"].as_table().unwrap();
    assert_eq!(features.len(), 2);
    assert_eq!(features["default"].as_array().unwrap().len(), 0);
    assert_eq!(features["perfetto-tracing"].as_array().unwrap().len(), 0);
    assert!(core.get("bin").is_none());

    let generation = read("crates/ait-core/src/binary_db_generation/mod.rs");
    for function in [
        "capture_binary_db_generation",
        "activate_binary_db_generation",
        "admit_activated_binary_db_generation",
        "admit_activated_binary_db_generation_for_runtime",
    ] {
        assert!(generation.contains(function));
    }

    let root = rust_root();
    for retired_path in [
        "crates/ait-core/src/bin/ait-binary-db-migrate.rs",
        "crates/ait-core/src/binary_db_migration/mod.rs",
        "crates/ait-core/src/binary_db_migration/local_v0_conversion.rs",
        "crates/ait-core/src/binary_db_migration/server_v0_conversion.rs",
        "crates/ait-core/src/binary_db_migration/server_v0_tail_merge.rs",
    ] {
        assert!(!root.join(retired_path).exists(), "{retired_path} returned");
    }

    let core_source = rust_source_tree(&root.join("crates/ait-core/src"));
    for retired_token in [
        "convert_local_binary_db_v0",
        "preflight_local_binary_db_v0",
        "refresh_local_binary_db_v0",
        "convert_server_binary_db_v0",
        "merge_server_binary_db_v0_tail",
        "convert_split_change_lifecycle_binary_db_generation",
        "ait-binary-db-migrate",
        "binary_db_migration",
        "OfflineConversion",
        "for_offline_migration_without_locks",
        "BinaryDbUpgradePreflight",
        "inspect_binary_db_upgrade",
        "normalize_plan_http_compatibility_payload_json",
        "list_tree_entry_views_for_legacy_physical_record",
        "record_tree_pack_metadata_batch_with_ordinals",
        "migration_compatibility",
    ] {
        assert!(
            !core_source.contains(retired_token),
            "retired Binary DB conversion token returned: {retired_token}"
        );
    }
}

#[test]
fn shared_storage_traits_are_backend_neutral() {
    for (path, names) in [
        (
            "crates/ait-core/src/content_store.rs",
            &["ContentStoreBundle", "BlobStore"][..],
        ),
        ("crates/ait-core/src/task_store/mod.rs", &["TaskStore"]),
        ("crates/ait-core/src/change_store/mod.rs", &["ChangeStore"]),
        ("crates/ait-core/src/line_store.rs", &["LineStore"]),
        ("crates/ait-core/src/snapshot_store.rs", &["SnapshotStore"]),
        (
            "crates/ait-core/src/remote_sync_local_store/models_contracts.rs",
            &["RemoteSyncZstdImportSource"],
        ),
        (
            "crates/ait-core/src/local_content_gc/contracts.rs",
            &["LocalContentMaintenanceStore"],
        ),
        (
            "crates/ait-core/src/workflow_event_store.rs",
            &["WorkflowEventStore"],
        ),
        (
            "crates/ait-core/src/workflow_release_store.rs",
            &["WorkflowReleaseStore"],
        ),
    ] {
        let source = read(path);
        for name in names {
            let body = trait_source(&source, name);
            for concrete_type in ["BinaryDb", "Connection", "PathBuf"] {
                assert!(
                    !body.contains(concrete_type),
                    "{name} exposes concrete storage type {concrete_type}"
                );
            }
        }
    }
}

#[test]
fn schema_registry_is_the_exact_twenty_four_file_runtime_authority() {
    assert_eq!(
        BTreeSet::from([
            PLAN_BIN,
            PLAN_PAYLOAD_BIN,
            PLAN_REVISION_BIN,
            PLAN_REVISION_PAYLOAD_BIN,
            PLAN_ITEM_BIN,
            PLAN_ITEM_PAYLOAD_BIN,
            TASK_RECORD_BIN,
            TASK_PAYLOAD_BIN,
            TASK_CHANGE_INDEX_BIN,
            TASK_LAND_INDEX_BIN,
            CHANGE_RECORD_BIN,
            CHANGE_PAYLOAD_BIN,
            CHANGE_LAND_INDEX_BIN,
            LAND_RECORD_BIN,
            BLOB_BIN,
            SNAPSHOT_BIN,
            SNAPSHOT_PAYLOAD_BIN,
            OBJECT_PACK_BIN,
            OBJECT_PACK_MEMBER_BIN,
            TREE_PACK_BIN,
            TREE_BIN,
            LINE_BIN,
            LINE_NAME_PAYLOAD_BIN,
            STASH_BIN,
        ]),
        REPOSITORY_BINARY_DB_BIN_PATHS.iter().copied().collect()
    );
    assert_eq!(
        BTreeSet::from([
            BLOB_ID_IDX,
            SNAPSHOT_ID_IDX,
            OBJECT_PACK_ID_IDX,
            TREE_ID_IDX,
            TREE_PACK_ID_IDX,
            LINE_NAME_IDX,
        ]),
        REPOSITORY_BINARY_DB_INDEX_PATHS.iter().copied().collect()
    );
}

#[test]
fn storage_requests_reject_unknown_selectors() {
    let sync_error = execute_plan_sync_command_request_json(
        r#"{"root_path":".","repo_name":"fixture","target":"docs/plan.md","plan_storage":{"mode":"binary"}}"#,
    )
    .unwrap_err();
    assert!(sync_error.contains("does not support plan_storage field"));

    let command_error = execute_plan_list_command_request_json(
        r#"{"scope":"local","repo_name":"fixture","plan_storage":{"mode":"binary"}}"#,
    )
    .unwrap_err();
    assert!(command_error.contains("does not support plan_storage field"));
    assert!(task_workflow_runtime_selection_facts(None)
        .unwrap()
        .is_object());
}

#[test]
fn runtime_factory_applies_schema_gate_without_expanding_control_plane_families() {
    let source = read("crates/ait-cli/src/runtime/selected_storage_adapters.rs");
    assert!(source
        .contains(".with_declared_bin_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)"));
    assert!(source.contains(
        ".with_declared_index_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS)"
    ));

    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(".ait")).unwrap();
    fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","default_line":"main"}"#,
    )
    .unwrap();
    let runtime = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let decisions = runtime.control_plane_store_decisions_json();
    let families = decisions
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["family"].as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        families,
        BTreeSet::from(["current_line", "line", "remote", "repo_status", "stash"])
    );
}
