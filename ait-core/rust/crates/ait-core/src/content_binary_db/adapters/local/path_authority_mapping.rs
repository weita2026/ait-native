use super::*;

impl<const WRITE_LAYOUT: u32> LocalContentBinaryDb<WRITE_LAYOUT> {
    pub fn new(
        authority_root: impl Into<StorePath>,
        local_repo_root: impl Into<StorePath>,
        local_authority_id: AuthorityId,
        current_line_state_scope: LocalStateScope,
    ) -> Self {
        let repo_root = local_repo_root.into();
        let db = LocalBinaryDbFs::new(
            authority_root,
            repo_root.clone(),
            local_authority_id,
            current_line_state_scope,
        );
        Self::from_db(db, repo_root)
    }

    pub fn from_db(db: LocalBinaryDbFs, repo_root: impl Into<StorePath>) -> Self {
        let repo_root = repo_root.into();
        Self::from_db_with_roots(db, repo_root.clone(), repo_root)
    }

    pub fn from_db_with_roots(
        db: LocalBinaryDbFs,
        workspace_root: impl Into<StorePath>,
        pack_root: impl Into<StorePath>,
    ) -> Self {
        let workspace_root = workspace_root.into();
        let pack_root = pack_root.into();
        Self {
            blobs: BinaryDbBlobStore::new(db.clone(), pack_root.clone()),
            snapshots: BinaryDbSnapshotStore::new(db.clone(), pack_root.clone()),
            object_packs: BinaryDbObjectPackStore::new(db.clone(), pack_root.clone()),
            tree_packs: BinaryDbTreePackStore::new(db.clone(), pack_root.clone()),
            trees: BinaryDbTreeStore::new(db.clone(), pack_root.clone()),
            db,
            workspace_root,
            pack_root,
        }
    }

    pub fn db(&self) -> &LocalBinaryDbFs {
        &self.db
    }

    pub fn repo_root(&self) -> &StorePath {
        &self.workspace_root
    }

    pub fn workspace_root(&self) -> &StorePath {
        &self.workspace_root
    }

    pub fn pack_root(&self) -> &StorePath {
        &self.pack_root
    }

    pub fn blobs(&self) -> &BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        &self.blobs
    }

    pub fn snapshots(&self) -> &BinaryDbSnapshotStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        &self.snapshots
    }

    pub fn object_packs(&self) -> &BinaryDbObjectPackStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        &self.object_packs
    }

    pub fn tree_packs(&self) -> &BinaryDbTreePackStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        &self.tree_packs
    }

    pub fn trees(&self) -> &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        &self.trees
    }

    pub fn snapshot_diff(
        &self,
        old_snapshot_id: &str,
        new_snapshot_id: &str,
        include_text: bool,
        max_bytes: usize,
    ) -> Result<JsonValue, String> {
        let snapshot_reader =
            BinaryDbSnapshotReader::with_root_resolver(self.trees.clone(), self.snapshots.clone());
        crate::object_diff::snapshot_diff_from_readers(
            &snapshot_reader,
            Some(&self.blobs),
            Some(old_snapshot_id),
            Some(new_snapshot_id),
            include_text,
            max_bytes,
        )
    }
}

pub(super) fn snapshot_file_entry_json(entry: &SnapshotFileEntry) -> JsonValue {
    json!({
        "path": entry.path,
        "blob_id": entry.blob_id,
        "size_bytes": entry.size_bytes,
        "mode": entry.mode,
        "sha256": entry.sha256,
    })
}

pub(super) fn current_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(super) fn elapsed_ms(start: Instant) -> f64 {
    round_ms(start.elapsed().as_secs_f64() * 1000.0)
}

pub(super) fn round_ms(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub(super) fn number_json(value: f64) -> JsonValue {
    Number::from_f64(round_ms(value))
        .map(JsonValue::Number)
        .unwrap_or_else(|| JsonValue::Number(Number::from(0)))
}

pub(super) fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn require_non_empty(value: &str, field: &str) -> Result<String, String> {
    normalize_optional_text(Some(value)).ok_or_else(|| format!("{field} must not be empty"))
}

pub(super) fn io_error(err: std::io::Error) -> String {
    err.to_string()
}

pub(super) fn json_string_field(value: &JsonValue, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("Expected string field `{field}`."))
}

pub(super) fn json_i64_field(value: &JsonValue, field: &str) -> Result<i64, String> {
    value
        .get(field)
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| format!("Expected integer field `{field}`."))
}

pub(super) fn sha256_array(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub(super) fn hash48_from_seed(seed: &[u8]) -> u64 {
    let digest = sha256_array(seed);
    (u64::from(digest[0]) << 40)
        | (u64::from(digest[1]) << 32)
        | (u64::from(digest[2]) << 24)
        | (u64::from(digest[3]) << 16)
        | (u64::from(digest[4]) << 8)
        | u64::from(digest[5])
}

pub(super) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
