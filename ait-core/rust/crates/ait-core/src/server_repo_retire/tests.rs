use super::*;
use crate::json_support::json;

#[test]
fn projects_runtime_blockers_without_identifier_fields() {
    let payload = json!({
        "active_agent_runtime_rows": [
            {"runtime_kind": "agent_run", "status": "active"},
            {"runtime_kind": "agent_run", "status": "active", "count": 2},
            {"runtime_kind": "sync_worker", "status": "active"}
        ],
        "active_planning_runtime_rows": [
            {"status": "active"},
            {"status": "active"}
        ]
    });

    let projected = project_repo_retire_runtime_blockers(&payload).expect("project blockers");

    assert_eq!(
        projected["active_agent_runtime_groups"],
        json!([
            {"runtime_kind": "agent_run", "status": "active", "count": 3},
            {"runtime_kind": "sync_worker", "status": "active", "count": 1}
        ])
    );
    assert_eq!(
        projected["active_planning_runtime_groups"],
        json!([
            {"status": "active", "count": 2}
        ])
    );
}

#[test]
fn empty_rows_project_to_empty_blocker_map() {
    let projected =
        project_repo_retire_runtime_blockers(&json!({})).expect("project empty blockers");

    assert_eq!(projected, json!({}));
}

#[test]
fn remote_export_manifest_is_exact_sorted_and_has_no_identity_extras() {
    let manifest = RemoteExportManifest::from_json(&json!({
        "schema": "ait.remote-export.v1",
        "state": "complete",
        "repo_name": "duplicate-name",
        "namespace": "R",
        "exported_at_s": 1_786_000_000,
        "files": [
            {
                "path": "patchset.bin",
                "size": 4,
                "sha256": "0".repeat(64),
            },
            {
                "path": "worker_job.bin",
                "size": 4,
                "sha256": "f".repeat(64),
            }
        ],
    }))
    .expect("exact manifest");

    assert_eq!(manifest.repo_name, "duplicate-name");
    assert_eq!(manifest.namespace, "R");
    assert_eq!(manifest.files.len(), 2);
    assert!(RemoteExportManifest::from_json(&json!({
        "schema": "ait.remote-export.v1",
        "state": "complete",
        "repo_name": "duplicate-name",
        "namespace": "R",
        "exported_at_s": 1_786_000_000,
        "server_instance_id": "forbidden",
        "files": [{
            "path": "patchset.bin",
            "size": 4,
            "sha256": "0".repeat(64),
        }],
    }))
    .is_err());
}

#[test]
fn remote_export_manifest_rejects_noncanonical_or_unsorted_paths() {
    for path in ["", "/absolute", "../escape", "a//b", "a/./b", "a\\b"] {
        assert!(
            validate_remote_authority_relative_path(path).is_err(),
            "{path}"
        );
    }
    let error = RemoteExportManifest::from_json(&json!({
        "schema": "ait.remote-export.v1",
        "state": "complete",
        "repo_name": "repo",
        "namespace": "",
        "exported_at_s": 1,
        "files": [
            {"path": "z.bin", "size": 0, "sha256": "0".repeat(64)},
            {"path": "a.bin", "size": 0, "sha256": "0".repeat(64)},
        ],
    }))
    .expect_err("unsorted files must fail");
    assert!(error.contains("path-sorted"), "{error}");
}
