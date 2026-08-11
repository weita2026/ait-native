use super::*;

pub(super) fn repository_row_from_db(row: pg::Row) -> RepositoryRow {
    RepositoryRow {
        repo_name: row.get("repo_name"),
        repo_id: row.get("repo_id"),
        default_line: row.get("default_line"),
        lifecycle_state: row.get("lifecycle_state"),
        id_namespace_prefix: row.get("id_namespace_prefix"),
        policy_json: row.get("policy_json"),
        created_at: row.get("created_at_text"),
        updated_at: row.get("updated_at_text"),
    }
}

pub(super) fn snapshot_row_from_db(row: pg::Row) -> SnapshotRow {
    SnapshotRow {
        snapshot_id: row.get("snapshot_id"),
        repo_name: row.get("repo_name"),
        repo_id: row.get("repo_id"),
        parent_snapshot_id: row.get("parent_snapshot_id"),
        root_tree_pack_id: row.get("root_tree_pack_id"),
        root_entry_ordinal: row.get::<_, i64>("root_entry_ordinal") as usize,
        manifest_hash: row.get("manifest_hash"),
        message: row.get("message"),
        line_name: row.get("line_name"),
        file_count: row.get::<_, i32>("file_count"),
        total_bytes: row.get::<_, i64>("total_bytes"),
        created_at: row.get("created_at_text"),
    }
}

pub(super) fn blob_row_from_db(row: pg::Row) -> BlobRow {
    BlobRow {
        sha256: row.get("sha256"),
        pack_id: row.get("pack_id"),
    }
}

pub(super) fn blob_locator_row_from_db(row: pg::Row) -> BlobLocatorRow {
    BlobLocatorRow {
        blob_id: row.get("blob_id"),
        sha256: row.get("sha256"),
        size_bytes: row.get("size_bytes"),
        pack_id: row.get("pack_id"),
        pack_entry_type: row.get("pack_entry_type"),
        pack_base_blob_id: row.get("pack_base_blob_id"),
        pack_chain_depth: row.get::<_, Option<i32>>("pack_chain_depth").map(i64::from),
        created_at: row.get("created_at_text"),
    }
}
