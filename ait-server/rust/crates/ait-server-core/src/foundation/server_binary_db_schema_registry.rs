use crate::foundation::remote_binary_db::BinaryDbFileFamily;

pub const SERVER_BINARY_DB_LAYOUT_ID: u32 = 1;
pub const SERVER_BINARY_DB_V0_AUTHORITY_SHA256: &str =
    "41989f27330ed4d2a8b9fefc2cdd332812ea00c09d4540f023a1cc5f59fd66be";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerBinaryDbBinSchema {
    pub path: &'static str,
    pub layout_id: u32,
    pub family: BinaryDbFileFamily,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerBinaryDbIndexSchema {
    pub path: &'static str,
    pub layout_id: u32,
    pub family: BinaryDbFileFamily,
    pub record_size: u32,
}

const fn schema(
    path: &'static str,
    layout_id: u32,
    family: BinaryDbFileFamily,
) -> ServerBinaryDbBinSchema {
    ServerBinaryDbBinSchema {
        path,
        layout_id,
        family,
    }
}

const fn layout_one(path: &'static str, family: BinaryDbFileFamily) -> ServerBinaryDbBinSchema {
    schema(path, SERVER_BINARY_DB_LAYOUT_ID, family)
}

pub const SERVER_BINARY_DB_BIN_SCHEMAS: &[ServerBinaryDbBinSchema] = &[
    layout_one("actor.bin", BinaryDbFileFamily::Workflow),
    layout_one("actor_payload.bin", BinaryDbFileFamily::Workflow),
    layout_one("attest.bin", BinaryDbFileFamily::Workflow),
    layout_one("task.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_payload.bin", BinaryDbFileFamily::Workflow),
    layout_one("change.bin", BinaryDbFileFamily::Workflow),
    layout_one("change_payload.bin", BinaryDbFileFamily::Workflow),
    layout_one("patchset.bin", BinaryDbFileFamily::Workflow),
    layout_one("patchset_summary_payload.bin", BinaryDbFileFamily::Workflow),
    layout_one("review.bin", BinaryDbFileFamily::Workflow),
    layout_one("review_payload.bin", BinaryDbFileFamily::Workflow),
    layout_one("policy.bin", BinaryDbFileFamily::Workflow),
    layout_one("policy_check.bin", BinaryDbFileFamily::Workflow),
    layout_one("land.bin", BinaryDbFileFamily::Workflow),
    layout_one("waiver.bin", BinaryDbFileFamily::Workflow),
    layout_one("waiver_payload.bin", BinaryDbFileFamily::Workflow),
    layout_one("snapshot_link.bin", BinaryDbFileFamily::Workflow),
    layout_one("snapshot_link_payload.bin", BinaryDbFileFamily::Workflow),
    layout_one("change_land_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("change_patchset_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("change_snapshot_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("patchset_attest_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("patchset_policy_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("patchset_review_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("patchset_waiver_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_attest_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_change_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_land_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_patchset_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_policy_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_review_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_snapshot_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("task_waiver_index.bin", BinaryDbFileFamily::Workflow),
    layout_one("line.bin", BinaryDbFileFamily::Content),
    layout_one("line_name_payload.bin", BinaryDbFileFamily::Content),
    layout_one("snapshot.bin", BinaryDbFileFamily::Content),
    layout_one("snapshot_payload.bin", BinaryDbFileFamily::Content),
    layout_one("snapshot_parent_edge.bin", BinaryDbFileFamily::Content),
    layout_one("blob.bin", BinaryDbFileFamily::Content),
    layout_one("object_pack.bin", BinaryDbFileFamily::Content),
    layout_one("object_pack_member.bin", BinaryDbFileFamily::Content),
    layout_one("tree.bin", BinaryDbFileFamily::Content),
    layout_one("tree_entry.bin", BinaryDbFileFamily::Content),
    layout_one("tree_entry_range.bin", BinaryDbFileFamily::Content),
    layout_one("tree_name_payload.bin", BinaryDbFileFamily::Content),
    layout_one("tree_pack.bin", BinaryDbFileFamily::Content),
    layout_one("plan.bin", BinaryDbFileFamily::Plan),
    layout_one("plan_payload.bin", BinaryDbFileFamily::Plan),
    layout_one("plan_revision.bin", BinaryDbFileFamily::Plan),
    layout_one("plan_revision_payload.bin", BinaryDbFileFamily::Plan),
    layout_one("plan_item.bin", BinaryDbFileFamily::Plan),
    layout_one("plan_item_payload.bin", BinaryDbFileFamily::Plan),
];

pub const SERVER_BINARY_DB_INDEX_SCHEMAS: &[ServerBinaryDbIndexSchema] = &[
    index("actor_lookup.idx", BinaryDbFileFamily::Workflow, 12),
    index("blob_id.idx", BinaryDbFileFamily::Content, 14),
    index("line_name.idx", BinaryDbFileFamily::Content, 12),
    index("manifest_hash.idx", BinaryDbFileFamily::Content, 36),
    index("object_pack_id.idx", BinaryDbFileFamily::Content, 12),
    index("snapshot_id.idx", BinaryDbFileFamily::Content, 12),
    index("snapshot_parent_child.idx", BinaryDbFileFamily::Content, 8),
    index("tree_id.idx", BinaryDbFileFamily::Content, 14),
    index("tree_pack_id.idx", BinaryDbFileFamily::Content, 12),
];

pub const SERVER_REPOSITORY_OPERATIONAL_BIN_SCHEMAS: &[ServerBinaryDbBinSchema] =
    &[layout_one("worker_job.bin", BinaryDbFileFamily::Queue)];

pub const SERVER_REPOSITORY_OPERATIONAL_INDEX_SCHEMAS: &[ServerBinaryDbIndexSchema] = &[
    index("worker_ready.idx", BinaryDbFileFamily::Queue, 12),
    index("worker_state.idx", BinaryDbFileFamily::Queue, 8),
];

pub const SERVER_GLOBAL_OPERATIONAL_BIN_SCHEMAS: &[ServerBinaryDbBinSchema] = &[
    layout_one("repository.bin", BinaryDbFileFamily::Queue),
    layout_one("repository_payload.bin", BinaryDbFileFamily::Queue),
];

pub const SERVER_GLOBAL_OPERATIONAL_INDEX_SCHEMAS: &[ServerBinaryDbIndexSchema] = &[index(
    "repository_namespace.idx",
    BinaryDbFileFamily::Queue,
    8,
)];

const fn index(
    path: &'static str,
    family: BinaryDbFileFamily,
    record_size: u32,
) -> ServerBinaryDbIndexSchema {
    ServerBinaryDbIndexSchema {
        path,
        layout_id: SERVER_BINARY_DB_LAYOUT_ID,
        family,
        record_size,
    }
}

pub fn server_binary_db_fixed_record_size(path: &str) -> Option<u32> {
    match path {
        "actor.bin" => Some(36),
        "attest.bin" => Some(24),
        "blob.bin" => Some(64),
        "change.bin" => Some(68),
        "land.bin" => Some(48),
        "line.bin" => Some(40),
        "object_pack.bin" => Some(32),
        "object_pack_member.bin" => Some(16),
        "patchset.bin" => Some(65),
        "plan.bin" => Some(48),
        "plan_item.bin" => Some(16),
        "plan_revision.bin" => Some(56),
        "policy.bin" => Some(32),
        "policy_check.bin" => Some(8),
        "review.bin" => Some(40),
        "repository.bin" => Some(33),
        "snapshot.bin" => Some(88),
        "snapshot_link.bin" => Some(40),
        "snapshot_parent_edge.bin" => Some(12),
        "task.bin" => Some(60),
        "tree.bin" => Some(20),
        "tree_entry.bin" => Some(16),
        "tree_entry_range.bin" => Some(4),
        "tree_pack.bin" => Some(32),
        "waiver.bin" => Some(44),
        "worker_job.bin" => Some(52),
        "change_land_index.bin"
        | "change_patchset_index.bin"
        | "change_snapshot_index.bin"
        | "patchset_attest_index.bin"
        | "patchset_policy_index.bin"
        | "patchset_review_index.bin"
        | "patchset_waiver_index.bin"
        | "task_attest_index.bin"
        | "task_change_index.bin"
        | "task_land_index.bin"
        | "task_patchset_index.bin"
        | "task_policy_index.bin"
        | "task_review_index.bin"
        | "task_snapshot_index.bin"
        | "task_waiver_index.bin" => Some(8),
        _ => None,
    }
}

pub fn server_binary_db_bin_schema(path: &str) -> Option<&'static ServerBinaryDbBinSchema> {
    SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .chain(SERVER_REPOSITORY_OPERATIONAL_BIN_SCHEMAS)
        .find(|schema| schema.path == path)
}

pub fn server_binary_db_bin_path_is_declared(path: &str) -> bool {
    server_binary_db_bin_schema(path).is_some()
}

pub fn server_binary_db_index_schema(path: &str) -> Option<&'static ServerBinaryDbIndexSchema> {
    SERVER_BINARY_DB_INDEX_SCHEMAS
        .iter()
        .chain(SERVER_REPOSITORY_OPERATIONAL_INDEX_SCHEMAS)
        .find(|schema| schema.path == path)
}

pub fn server_global_operational_bin_schema(
    path: &str,
) -> Option<&'static ServerBinaryDbBinSchema> {
    SERVER_GLOBAL_OPERATIONAL_BIN_SCHEMAS
        .iter()
        .find(|schema| schema.path == path)
}

pub fn server_global_operational_index_schema(
    path: &str,
) -> Option<&'static ServerBinaryDbIndexSchema> {
    SERVER_GLOBAL_OPERATIONAL_INDEX_SCHEMAS
        .iter()
        .find(|schema| schema.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::operational_binary_v0::{
        SERVER_GLOBAL_OPERATIONAL_BIN_PATHS, SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS,
        SERVER_REPOSITORY_OPERATIONAL_BIN_PATHS, SERVER_REPOSITORY_OPERATIONAL_INDEX_PATHS,
    };
    use crate::foundation::remote_binary_db::{
        BinaryFileId, FilesystemServerRemoteBinaryDb, RepoId, RepoName, StoreGeneration, StorePath,
    };
    use crate::foundation::workflow_binary_v0::WORKFLOW_V0_LAYOUT_ID;
    use std::collections::BTreeSet;

    #[test]
    fn server_binary_db_bin_registry_is_unique_and_uses_declared_layouts() {
        assert_eq!(SERVER_BINARY_DB_BIN_SCHEMAS.len(), 52);
        assert_eq!(
            SERVER_BINARY_DB_BIN_SCHEMAS
                .iter()
                .filter(|schema| schema.family == BinaryDbFileFamily::Workflow)
                .count(),
            33
        );
        assert_eq!(
            SERVER_BINARY_DB_BIN_SCHEMAS
                .iter()
                .filter(|schema| schema.family == BinaryDbFileFamily::Content)
                .count(),
            13
        );
        assert_eq!(
            SERVER_BINARY_DB_BIN_SCHEMAS
                .iter()
                .filter(|schema| schema.family == BinaryDbFileFamily::Plan)
                .count(),
            6
        );
        assert_eq!(SERVER_REPOSITORY_OPERATIONAL_BIN_SCHEMAS.len(), 1);
        assert_eq!(
            SERVER_REPOSITORY_OPERATIONAL_BIN_SCHEMAS[0].family,
            BinaryDbFileFamily::Queue
        );
        let mut paths = BTreeSet::new();
        for schema in SERVER_BINARY_DB_BIN_SCHEMAS {
            assert!(paths.insert(schema.path), "duplicate {}", schema.path);
            assert!(schema.path.ends_with(".bin"));
            assert!(
                !schema.path.ends_with("_head.bin"),
                "head Binary DB files are forbidden: {}",
                schema.path
            );
            assert_eq!(
                schema.layout_id, SERVER_BINARY_DB_LAYOUT_ID,
                "{} layout",
                schema.path
            );
        }
        assert_eq!(WORKFLOW_V0_LAYOUT_ID, 1);
        assert_eq!(
            SERVER_BINARY_DB_V0_AUTHORITY_SHA256,
            "41989f27330ed4d2a8b9fefc2cdd332812ea00c09d4540f023a1cc5f59fd66be"
        );
        assert_eq!(SERVER_BINARY_DB_INDEX_SCHEMAS.len(), 9);
        let index_paths = SERVER_BINARY_DB_INDEX_SCHEMAS
            .iter()
            .map(|schema| schema.path)
            .collect::<BTreeSet<_>>();
        assert_eq!(index_paths.len(), SERVER_BINARY_DB_INDEX_SCHEMAS.len());
        assert_eq!(SERVER_REPOSITORY_OPERATIONAL_INDEX_SCHEMAS.len(), 2);
        assert_eq!(SERVER_GLOBAL_OPERATIONAL_BIN_SCHEMAS.len(), 2);
        assert_eq!(SERVER_GLOBAL_OPERATIONAL_INDEX_SCHEMAS.len(), 1);
        assert_eq!(
            SERVER_GLOBAL_OPERATIONAL_BIN_SCHEMAS
                .iter()
                .map(|schema| schema.path)
                .collect::<Vec<_>>(),
            SERVER_GLOBAL_OPERATIONAL_BIN_PATHS
        );
        assert_eq!(
            SERVER_GLOBAL_OPERATIONAL_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path)
                .collect::<Vec<_>>(),
            SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS
        );
        assert_eq!(
            SERVER_REPOSITORY_OPERATIONAL_BIN_SCHEMAS
                .iter()
                .map(|schema| schema.path)
                .collect::<Vec<_>>(),
            SERVER_REPOSITORY_OPERATIONAL_BIN_PATHS
        );
        assert_eq!(
            SERVER_REPOSITORY_OPERATIONAL_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path)
                .collect::<Vec<_>>(),
            SERVER_REPOSITORY_OPERATIONAL_INDEX_PATHS
        );
        assert_eq!(
            server_binary_db_fixed_record_size("repository.bin"),
            Some(33)
        );
        assert_eq!(
            server_binary_db_fixed_record_size("worker_job.bin"),
            Some(52)
        );
    }

    #[test]
    fn retired_workflow_files_are_not_declared() {
        for path in [
            "patchset_payload.bin",
            "attestation.bin",
            "attestation_payload.bin",
            "policy_payload.bin",
            "land_payload.bin",
            "change_lifecycle.bin",
            "land_target_line.bin",
        ] {
            assert!(!server_binary_db_bin_path_is_declared(path), "{path}");
        }
    }

    #[test]
    fn undeclared_bin_path_is_rejected() {
        assert!(!server_binary_db_bin_path_is_declared("runtime-cache.bin"));
        assert!(!server_binary_db_bin_path_is_declared(
            "workflow-unknown.bin"
        ));
        assert!(!server_binary_db_bin_path_is_declared(
            "server_worker_job.bin"
        ));
        assert!(!server_binary_db_bin_path_is_declared(
            "server_worker_job_payload.bin"
        ));
        assert!(!server_binary_db_bin_path_is_declared(
            "worker_job_payload.bin"
        ));
        assert!(server_binary_db_bin_path_is_declared("worker_job.bin"));
        assert!(server_binary_db_index_schema("worker_ready.idx").is_some());
        assert!(server_binary_db_index_schema("worker_state.idx").is_some());
        assert!(!server_binary_db_bin_path_is_declared("repository.bin"));
        assert!(server_global_operational_bin_schema("repository.bin").is_some());
        assert!(server_global_operational_index_schema("repository_namespace.idx").is_some());
        assert!(!server_binary_db_bin_path_is_declared("future_head.bin"));
        assert!(server_binary_db_bin_path_is_declared("tree_entry.bin"));
        assert!(server_binary_db_bin_path_is_declared(
            "tree_name_payload.bin"
        ));
        assert!(!server_binary_db_bin_path_is_declared(
            "attestation_payload.bin"
        ));
        for path in [
            "workflow_line_projection.bin",
            "workflow_line_projection_payload.bin",
            "line_head.bin",
            "workflow_snapshot_projection.bin",
            "workflow_snapshot_projection_payload.bin",
            "snapshot_head.bin",
            "task_head.bin",
            "change_head.bin",
            "patchset_head.bin",
            "review_head.bin",
            "attestation_head.bin",
            "policy_head.bin",
            "land_head.bin",
            "plan_head.bin",
            "plan_revision_head.bin",
        ] {
            assert!(
                !server_binary_db_bin_path_is_declared(path),
                "duplicate workflow content projection must remain undeclared: {path}"
            );
        }
    }

    #[test]
    fn serving_authorities_reject_undeclared_bin_before_creation() {
        let root =
            std::env::temp_dir().join(format!("ait-server-schema-registry-{}", std::process::id()));
        let db = FilesystemServerRemoteBinaryDb::serving_authority(
            RepoId::new("repo-id"),
            RepoName::new("repo"),
            StorePath::new(root.clone()),
            StoreGeneration::new(1),
        );
        let undeclared = BinaryFileId::new("runtime-cache.bin", 1, 4, BinaryDbFileFamily::Workflow);

        let error = db
            .resolve_record_path(&undeclared)
            .expect_err("undeclared .bin must fail before path use");
        assert!(error.to_string().contains("undeclared"));
        assert!(!root.join("runtime-cache.bin").exists());
    }

    #[test]
    fn test_fixture_authority_also_rejects_undeclared_bin_before_creation() {
        let root = std::env::temp_dir().join(format!(
            "ait-server-schema-registry-test-fixture-{}",
            std::process::id()
        ));
        let db = FilesystemServerRemoteBinaryDb::test_fixture(
            RepoId::new("repo-id"),
            RepoName::new("repo"),
            StorePath::new(root.clone()),
            StoreGeneration::new(1),
        );
        let undeclared = BinaryFileId::new("runtime-cache.bin", 1, 4, BinaryDbFileFamily::Workflow);

        let error = db
            .resolve_record_path(&undeclared)
            .expect_err("test fixtures must obey the same declared .bin registry");
        assert!(error.to_string().contains("undeclared"));
        assert!(!root.join("runtime-cache.bin").exists());
    }
}
