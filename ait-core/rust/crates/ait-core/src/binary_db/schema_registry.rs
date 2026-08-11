//! Canonical local repository Binary DB schema registry.
//!
//! Every repository-authoritative runtime family is declared here so the
//! filesystem schema gate accepts only the layout-1 files owned by the local
//! Plan, content, line, stash, and control-plane stores.

pub const REPOSITORY_BINARY_DB_BIN_PATHS: &[&str] = &[
    "plan.bin",
    "plan_payload.bin",
    "plan_revision.bin",
    "plan_revision_payload.bin",
    "plan_item.bin",
    "plan_item_payload.bin",
    "task.bin",
    "task_payload.bin",
    "task_change_index.bin",
    "task_land_index.bin",
    "change.bin",
    "change_payload.bin",
    "change_land_index.bin",
    "land.bin",
    "blob.bin",
    "snapshot.bin",
    "snapshot_payload.bin",
    "object_pack.bin",
    "object_pack_member.bin",
    "tree_pack.bin",
    "tree.bin",
    "line.bin",
    "line_name_payload.bin",
    "stash.bin",
];

pub const REPOSITORY_BINARY_DB_INDEX_PATHS: &[&str] = &[
    "blob_id.idx",
    "snapshot_id.idx",
    "object_pack_id.idx",
    "tree_id.idx",
    "tree_pack_id.idx",
    "line_name.idx",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_db::{
        AuthorityId, BinaryDbCommandScope, BinaryDbNoopFsyncPolicy, BinaryFileId, BinaryIndexId,
        BinaryPayloadFileId, LocalBinaryDbFs, LocalStateScope,
    };
    use std::collections::BTreeSet;
    use tempfile::TempDir;

    #[test]
    fn repository_bin_registry_is_exact_unique_and_leaf_only() {
        let unique = REPOSITORY_BINARY_DB_BIN_PATHS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), 24);
        assert_eq!(unique.len(), REPOSITORY_BINARY_DB_BIN_PATHS.len());
        assert!(REPOSITORY_BINARY_DB_BIN_PATHS
            .iter()
            .all(|path| path.ends_with(".bin") && !path.contains('/')));
        assert!(
            !REPOSITORY_BINARY_DB_BIN_PATHS.contains(&"change_lifecycle.bin"),
            "Change lifecycle authority is inline in change.bin"
        );
        assert!(
            !REPOSITORY_BINARY_DB_BIN_PATHS.contains(&"land_target_line.bin"),
            "Land target-Line authority is inline in land.bin"
        );
    }

    #[test]
    fn repository_index_registry_is_unique_and_leaf_only() {
        let unique = REPOSITORY_BINARY_DB_INDEX_PATHS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), 6);
        assert_eq!(unique.len(), REPOSITORY_BINARY_DB_INDEX_PATHS.len());
        assert!(REPOSITORY_BINARY_DB_INDEX_PATHS
            .iter()
            .all(|path| path.ends_with(".idx") && !path.contains('/')));
    }

    #[test]
    fn repository_schema_gate_rejects_undeclared_bin_and_index_paths_before_creation() {
        let temp = TempDir::new().unwrap();
        let authority = temp.path().join("binary-db");
        let db = LocalBinaryDbFs::new(
            authority.clone(),
            temp.path().to_path_buf(),
            AuthorityId::new("schema-gate"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
        let mut write = db
            .begin_write_txn_with_fsync_policy(
                BinaryDbCommandScope::General,
                BinaryDbNoopFsyncPolicy,
            )
            .unwrap();

        let bin_error = write
            .append_record(BinaryFileId::new("undeclared_alias.bin", 1, 8), &[0_u8; 8])
            .unwrap_err();
        assert!(bin_error
            .to_string()
            .contains("no local schema declaration"));
        let index_error = write
            .append_index_candidate(BinaryIndexId::new("task_alias.idx", 1), b"alias", 0)
            .unwrap_err();
        assert!(index_error
            .to_string()
            .contains("no local schema declaration"));
        let payload_error = write
            .append_payload(BinaryPayloadFileId::new("plan_payload.bin", 1), b"")
            .unwrap_err();
        assert!(payload_error.to_string().contains("header-only"));
        drop(write);

        assert!(!authority.join("undeclared_alias.bin").exists());
        assert!(!authority.join("task_alias.idx").exists());
        assert!(!authority.join("plan_payload.bin").exists());
    }
}
