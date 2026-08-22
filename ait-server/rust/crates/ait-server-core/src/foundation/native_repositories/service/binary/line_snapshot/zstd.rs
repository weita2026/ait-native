use super::*;
use crate::foundation::remote_binary_db::acquire_serving_repository_pack_lock;
use crate::foundation::server_content_binary_db::ServerBinaryTreeReadCache;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

const ZSTD_PULL_MANIFEST_MAX_SNAPSHOTS: usize = 100_000;
const ZSTD_PACK_FILE_COMPARE_BUFFER_BYTES: usize = 64 * 1024;
const ZSTD_STAGED_UPLOAD_MARKER: &str = ".zstpack.upload-";
static ZSTD_STAGED_UPLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
std::thread_local! {
    static TEST_ZSTD_PACK_PAYLOAD_READ_COUNT: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
}

pub(in super::super) struct PreparedRepositoryMetadataMutation {
    pub kind: &'static str,
    pub value: JsonValue,
}

pub(in super::super) struct PreparedSnapshotMutation {
    pub snapshot_id: String,
    pub value: JsonValue,
}

struct BinaryZstdPackPayloadEvidence {
    pack_index: JsonValue,
    detected_checksum: Option<String>,
    pack_sha256: String,
    payload_bytes: u64,
}

fn binary_zstd_pack_files_equal(
    left_path: &Path,
    right_path: &Path,
) -> Result<bool, NativeRepositoryError> {
    let left_len = std::fs::metadata(left_path)
        .map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to inspect Binary DB pack {}: {error}",
                left_path.display()
            ))
        })?
        .len();
    let right_len = std::fs::metadata(right_path)
        .map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to inspect staged Binary DB pack {}: {error}",
                right_path.display()
            ))
        })?
        .len();
    if left_len != right_len {
        return Ok(false);
    }

    let mut left = BufReader::with_capacity(
        ZSTD_PACK_FILE_COMPARE_BUFFER_BYTES,
        File::open(left_path).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to open Binary DB pack {}: {error}",
                left_path.display()
            ))
        })?,
    );
    let mut right = BufReader::with_capacity(
        ZSTD_PACK_FILE_COMPARE_BUFFER_BYTES,
        File::open(right_path).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to open staged Binary DB pack {}: {error}",
                right_path.display()
            ))
        })?,
    );
    let mut left_buffer = [0_u8; ZSTD_PACK_FILE_COMPARE_BUFFER_BYTES];
    let mut right_buffer = [0_u8; ZSTD_PACK_FILE_COMPARE_BUFFER_BYTES];
    loop {
        let left_read = left.read(&mut left_buffer).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to read Binary DB pack {}: {error}",
                left_path.display()
            ))
        })?;
        let right_read = right.read(&mut right_buffer).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to read staged Binary DB pack {}: {error}",
                right_path.display()
            ))
        })?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

#[derive(Default)]
pub(in super::super) struct BinaryZstdImportManifestContent {
    pub object_packs: Vec<JsonValue>,
    pub tree_packs: Vec<JsonValue>,
    pub blob_locators: Vec<JsonValue>,
    pub tree_locators: Vec<JsonValue>,
}

#[derive(Default)]
pub(in super::super) struct BinaryZstdCommitReadSet {
    metadata: BTreeMap<(String, String), Option<JsonValue>>,
    pack_payloads: BTreeMap<(bool, String), Vec<u8>>,
    snapshots: BTreeMap<String, Option<JsonValue>>,
    lines: BTreeMap<String, Option<JsonValue>>,
}

#[derive(Default)]
struct BinaryTreePackChecksumCache {
    by_pack: BTreeMap<String, BTreeMap<String, String>>,
}

impl BinaryTreePackChecksumCache {
    fn checksum<F>(
        &mut self,
        pack_id: &str,
        tree_id: &str,
        load_index: F,
    ) -> Result<String, NativeRepositoryError>
    where
        F: FnOnce() -> Result<JsonValue, NativeRepositoryError>,
    {
        if !self.by_pack.contains_key(pack_id) {
            let index = load_index()?;
            let mut checksums = BTreeMap::new();
            if let Some(trees) = index.get("trees").and_then(JsonValue::as_array) {
                for tree in trees {
                    let Some(tree_id) = tree.get("tree_id").and_then(JsonValue::as_str) else {
                        continue;
                    };
                    let Some(checksum) = tree.get("checksum").and_then(JsonValue::as_str) else {
                        continue;
                    };
                    checksums
                        .entry(tree_id.to_string())
                        .or_insert_with(|| checksum.to_string());
                }
            }
            self.by_pack.insert(pack_id.to_string(), checksums);
        }
        self.by_pack
            .get(pack_id)
            .and_then(|checksums| checksums.get(tree_id))
            .cloned()
            .ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "tree pack {pack_id} has no checksum for {tree_id}"
                ))
            })
    }
}

impl BinaryZstdCommitReadSet {
    fn metadata(&self, kind: &str, id: &str) -> Result<Option<&JsonValue>, NativeRepositoryError> {
        self.metadata
            .get(&(kind.to_string(), id.to_string()))
            .map(Option::as_ref)
            .ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB zstd commit did not prefetch metadata {kind}:{id}"
                ))
            })
    }

    fn pack_index(
        &self,
        pack_id: &str,
        tree_pack: bool,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let kind = if tree_pack {
            BINARY_ZSTD_TREE_PACK_KIND
        } else {
            BINARY_ZSTD_OBJECT_PACK_KIND
        };
        let label = if tree_pack {
            "tree pack"
        } else {
            "object pack"
        };
        let value = self.metadata(kind, pack_id)?.ok_or_else(|| {
            NativeRepositoryError::not_found(format!("Unknown zstd {label} {pack_id}"))
        })?;
        value.get("pack_index").cloned().ok_or_else(|| {
            NativeRepositoryError::internal(format!("{label} {pack_id} is missing pack_index"))
        })
    }

    fn pack_payload(&self, pack_id: &str, tree_pack: bool) -> Result<&[u8], NativeRepositoryError> {
        self.pack_payloads
            .get(&(tree_pack, pack_id.to_string()))
            .map(Vec::as_slice)
            .ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB zstd commit did not prefetch {} pack payload {pack_id}",
                    if tree_pack { "tree" } else { "object" }
                ))
            })
    }

    pub(in super::super) fn snapshot(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<&JsonValue>, NativeRepositoryError> {
        self.snapshots
            .get(snapshot_id)
            .map(Option::as_ref)
            .ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB zstd commit did not prefetch snapshot {snapshot_id}"
                ))
            })
    }

    pub(in super::super) fn line(
        &self,
        line_name: &str,
    ) -> Result<Option<&JsonValue>, NativeRepositoryError> {
        self.lines
            .get(line_name)
            .map(Option::as_ref)
            .ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB zstd commit did not prefetch line {line_name}"
                ))
            })
    }
}

impl<D> BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    #[cfg(test)]
    pub fn seed_zstd_pack_batch_for_test(
        &self,
        repo_name: &str,
        packs: Vec<(String, Vec<u8>)>,
        tree_pack: bool,
    ) -> Result<(), NativeRepositoryError> {
        self.ensure_test_fixture_authority()?;
        self.ensure_repository(repo_name)?;
        let mut seen = BTreeSet::new();
        for (pack_id, bytes) in packs {
            validate_pack_id_segment(&pack_id)?;
            if !seen.insert(pack_id.clone()) {
                return Err(NativeRepositoryError::bad_request(format!(
                    "duplicate fixture pack {pack_id}"
                )));
            }
            if bytes.is_empty() {
                return Err(NativeRepositoryError::bad_request(format!(
                    "fixture pack {pack_id} is empty"
                )));
            }
            self.put_binary_zstd_pack(repo_name, &pack_id, bytes, tree_pack)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn seed_zstd_locator_batch_for_test(
        &self,
        repo_name: &str,
        values: Vec<JsonValue>,
        tree_locator: bool,
    ) -> Result<(), NativeRepositoryError> {
        self.ensure_test_fixture_authority()?;
        self.ensure_repository(repo_name)?;
        let mut seen = BTreeSet::new();
        let mut by_pack = BTreeMap::<String, Vec<JsonValue>>::new();
        for value in values {
            let object = json_object(&value, "fixture locator")?;
            let identity_field = if tree_locator { "tree_id" } else { "blob_id" };
            let identity = required_json_text(object, identity_field)
                .map_err(NativeRepositoryError::bad_request)?;
            if !seen.insert(identity.clone()) {
                return Err(NativeRepositoryError::bad_request(format!(
                    "duplicate fixture locator {identity}"
                )));
            }
            let pack_field = if tree_locator {
                "tree_pack_id"
            } else {
                "pack_id"
            };
            let pack_id = required_json_text(object, pack_field)
                .map_err(NativeRepositoryError::bad_request)?;
            by_pack.entry(pack_id).or_default().push(value);
        }
        let mut prepared = Vec::with_capacity(by_pack.len());
        for (pack_id, locators) in by_pack {
            let mut pack = self.typed_pack_metadata(&pack_id, tree_locator)?;
            pack.as_object_mut()
                .ok_or_else(|| NativeRepositoryError::internal("pack metadata is not an object"))?
                .insert("status".to_string(), json!("ready"));
            prepared.push((pack, locators));
        }
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerRemoteSyncCommit)
                .map_err(binary_native_repository_store_error)?;
        for (pack, locators) in prepared {
            if tree_locator {
                self.repository_content()
                    .append_tree_pack_in_tx(&mut tx, &pack, &locators)
                    .map_err(binary_native_repository_store_error)?;
            } else {
                self.repository_content()
                    .append_object_pack_in_tx(&mut tx, &pack, &locators)
                    .map_err(binary_native_repository_store_error)?;
            }
        }
        self.invalidate_zstd_pull_catalog()?;
        tx.commit()
            .map(|_| ())
            .map_err(binary_native_repository_store_error)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn prefetch_binary_zstd_commit_read_set(
        &self,
        repo_name: &str,
        object_pack_values: &[&JsonValue],
        tree_pack_values: &[&JsonValue],
        blob_locator_values: &[&JsonValue],
        tree_locator_values: &[&JsonValue],
        snapshot_values: &[&JsonValue],
        line_update: Option<&(String, LineUpdateRequest)>,
    ) -> Result<BinaryZstdCommitReadSet, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let mut metadata_keys = BTreeSet::<(String, String)>::new();
        let mut snapshot_ids = BTreeSet::<String>::new();
        let mut line_names = BTreeSet::<String>::new();

        for value in object_pack_values {
            let object = json_object(value, "object_packs[]")?;
            let pack_id = required_json_text(object, "pack_id")
                .map_err(NativeRepositoryError::bad_request)?;
            metadata_keys.insert((BINARY_ZSTD_OBJECT_PACK_KIND.to_string(), pack_id));
        }
        for value in tree_pack_values {
            let object = json_object(value, "tree_packs[]")?;
            let pack_id = required_json_text(object, "pack_id")
                .map_err(NativeRepositoryError::bad_request)?;
            metadata_keys.insert((BINARY_ZSTD_TREE_PACK_KIND.to_string(), pack_id));
        }
        for value in blob_locator_values {
            let object = json_object(value, "blob_locators[]")?;
            let blob_id = required_json_text(object, "blob_id")
                .map_err(NativeRepositoryError::bad_request)?;
            let pack_id = required_json_text(object, "pack_id")
                .map_err(NativeRepositoryError::bad_request)?;
            metadata_keys.insert((BINARY_ZSTD_BLOB_LOCATOR_KIND.to_string(), blob_id));
            metadata_keys.insert((BINARY_ZSTD_OBJECT_PACK_KIND.to_string(), pack_id));
        }
        for value in tree_locator_values {
            let object = json_object(value, "tree_locators[]")?;
            let tree_id = required_json_text(object, "tree_id")
                .map_err(NativeRepositoryError::bad_request)?;
            let pack_id = required_json_text(object, "tree_pack_id")
                .map_err(NativeRepositoryError::bad_request)?;
            metadata_keys.insert((BINARY_ZSTD_TREE_LOCATOR_KIND.to_string(), tree_id));
            metadata_keys.insert((BINARY_ZSTD_TREE_PACK_KIND.to_string(), pack_id));
        }
        for value in snapshot_values {
            let object = json_object(value, "snapshots[]")?;
            let snapshot_id = required_json_text(object, "snapshot_id")
                .map_err(NativeRepositoryError::bad_request)?;
            let root_tree_pack_id = required_json_text(object, "root_tree_pack_id")
                .map_err(NativeRepositoryError::bad_request)?;
            snapshot_ids.insert(snapshot_id);
            if let Some(parent_id) = optional_json_text(object, "parent_snapshot_id") {
                snapshot_ids.insert(parent_id);
            }
            metadata_keys.insert((BINARY_ZSTD_TREE_PACK_KIND.to_string(), root_tree_pack_id));
        }
        if let Some((line_name, request)) = line_update {
            line_names.insert(line_name.clone());
            if let Some(snapshot_id) = normalize_optional_text(request.head_snapshot_id.clone()) {
                snapshot_ids.insert(snapshot_id);
            }
        }

        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let mut read_set = BinaryZstdCommitReadSet::default();
        let mut tree_pack_checksums = BinaryTreePackChecksumCache::default();
        for (kind, id) in &metadata_keys {
            let field = match kind.as_str() {
                BINARY_ZSTD_OBJECT_PACK_KIND | BINARY_ZSTD_TREE_PACK_KIND => "pack_id",
                BINARY_ZSTD_BLOB_LOCATOR_KIND => "blob_id",
                BINARY_ZSTD_TREE_LOCATOR_KIND => "tree_id",
                other => {
                    return Err(NativeRepositoryError::internal(format!(
                        "unsupported typed Binary DB content kind {other}"
                    )))
                }
            };
            let value = if kind == BINARY_ZSTD_TREE_LOCATOR_KIND {
                self.repository_content()
                    .tree_with_read(&read, id)
                    .map_err(binary_native_repository_store_error)?
                    .map(|view| self.typed_tree_locator_with_cache(&view, &mut tree_pack_checksums))
                    .transpose()?
            } else {
                self.latest_binary_zstd_record_optional_with_read(
                    &read, repo_name, kind, field, id,
                )?
            };
            if value
                .as_ref()
                .is_some_and(|value| binary_json_text(value, field).as_deref() != Some(id))
            {
                return Err(NativeRepositoryError::internal(format!(
                    "Binary DB zstd metadata identity {kind}:{id} disagrees with field {field}"
                )));
            }
            read_set.metadata.insert((kind.clone(), id.clone()), value);
        }
        for ((kind, pack_id), value) in &read_set.metadata {
            let tree_pack = kind == BINARY_ZSTD_TREE_PACK_KIND;
            if !tree_pack && kind != BINARY_ZSTD_OBJECT_PACK_KIND {
                continue;
            }
            let Some(value) = value else {
                continue;
            };
            if binary_json_text(value, "status").as_deref() == Some("ready") {
                continue;
            }
            let bytes = self.read_zstd_pack_payload(value, tree_pack, pack_id)?;
            read_set
                .pack_payloads
                .insert((tree_pack, pack_id.clone()), bytes);
        }
        for snapshot_id in snapshot_ids {
            let value =
                self.latest_snapshot_value_optional_with_read(&read, repo_name, &snapshot_id)?;
            read_set.snapshots.insert(snapshot_id, value);
        }
        for line_name in line_names {
            let value = self.latest_line_value_optional_with_read(&read, repo_name, &line_name)?;
            read_set.lines.insert(line_name, value);
        }
        Ok(read_set)
    }

    pub(in super::super) fn binary_zstd_import_manifest_content(
        &self,
        repo_name: &str,
        snapshot: &JsonValue,
    ) -> Result<BinaryZstdImportManifestContent, NativeRepositoryError> {
        self.binary_zstd_import_manifest_content_for_snapshots(
            repo_name,
            std::slice::from_ref(snapshot),
        )
    }

    pub(in super::super) fn binary_zstd_pull_manifest_parts(
        &self,
        repo_name: &str,
        head_snapshot_id: &str,
        have_snapshot_ids: &BTreeSet<String>,
    ) -> Result<(Vec<JsonValue>, Vec<String>, BinaryZstdImportManifestContent), NativeRepositoryError>
    {
        self.ensure_repository(repo_name)?;
        let catalog = self.binary_zstd_pull_catalog()?;
        let (snapshots, boundary_snapshot_ids) = self
            .binary_zstd_pull_manifest_snapshots_with_catalog(
                &catalog,
                repo_name,
                head_snapshot_id,
                have_snapshot_ids,
            )?;
        let content = self
            .binary_zstd_import_manifest_content_for_snapshots_with_catalog(&catalog, &snapshots)?;
        Ok((snapshots, boundary_snapshot_ids, content))
    }

    fn binary_zstd_pull_catalog(
        &self,
    ) -> Result<Arc<BinaryZstdPullCatalog>, NativeRepositoryError> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let content = self.repository_content();
        let revision = content
            .manifest_revision_with_read(&read)
            .map_err(binary_native_repository_store_error)?;
        let mut current = self.pull_catalog_cache.current.lock().map_err(|_| {
            NativeRepositoryError::internal("Binary DB pull catalog cache lock is poisoned")
        })?;
        if let Some(catalog) = current
            .as_ref()
            .filter(|catalog| catalog.revision == revision)
        {
            return Ok(catalog.clone());
        }

        let catalog = Arc::new(self.build_binary_zstd_pull_catalog(&read, revision)?);
        *current = Some(catalog.clone());
        #[cfg(test)]
        self.pull_catalog_cache
            .build_count
            .fetch_add(1, Ordering::Relaxed);
        Ok(catalog)
    }

    fn build_binary_zstd_pull_catalog(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        revision: [u8; 32],
    ) -> Result<BinaryZstdPullCatalog, NativeRepositoryError> {
        let content = self.repository_content();
        let mut manifest_cache = content
            .manifest_tree_read_cache_with_read(read)
            .map_err(binary_native_repository_store_error)?;
        content
            .validate_complete_manifest_identity_indexes_with_read(read, &manifest_cache)
            .map_err(binary_native_repository_store_error)?;

        let mut object_pack_rows_by_id = BTreeMap::new();
        for pack in manifest_cache
            .projected_object_packs()
            .map_err(binary_native_repository_store_error)?
        {
            let metadata = self.committed_object_pack_metadata(&pack)?;
            object_pack_rows_by_id.insert(
                pack.pack_id.clone(),
                binary_zstd_import_manifest_pack_row(metadata, false)?,
            );
        }
        let mut blob_locator_rows_by_index = BTreeMap::new();
        for blob in manifest_cache
            .projected_blobs()
            .map_err(binary_native_repository_store_error)?
        {
            blob_locator_rows_by_index.insert(
                blob.blob_index,
                binary_zstd_import_manifest_blob_locator_row(self.typed_blob_locator(&blob)?)?,
            );
        }

        let mut tree_pack_rows_by_id = BTreeMap::new();
        for pack in manifest_cache
            .projected_tree_packs()
            .map_err(binary_native_repository_store_error)?
        {
            // `tree_count` owns the normalized logical Tree range, not the
            // physical archive member count. A conversion may retain a
            // physical-only pack after every Tree in it resolves to an
            // earlier normalized identity. Such a zero-range pack cannot be
            // selected by any projected Tree, so it has no transfer row in a
            // Snapshot content closure and its archive must not poison the
            // request-wide catalog.
            if pack.record.tree_count == 0 {
                continue;
            }
            let metadata =
                self.committed_tree_pack_metadata_with_cache(&pack, &mut manifest_cache)?;
            tree_pack_rows_by_id.insert(
                pack.pack_id.clone(),
                binary_zstd_import_manifest_pack_row(metadata, true)?,
            );
        }
        let mut tree_locator_rows_by_index = BTreeMap::new();
        let mut tree_entries_by_index = BTreeMap::new();
        for tree in manifest_cache
            .projected_trees()
            .map_err(binary_native_repository_store_error)?
        {
            tree_locator_rows_by_index.insert(
                tree.tree_index,
                binary_zstd_import_manifest_tree_locator_row(
                    self.typed_tree_locator_with_manifest_cache(&tree, &mut manifest_cache)?,
                )?,
            );
            tree_entries_by_index.insert(
                tree.tree_index,
                content
                    .projected_tree_entries_for_tree_with_read_cache(
                        read,
                        &tree,
                        &mut manifest_cache,
                    )
                    .map_err(binary_native_repository_store_error)?,
            );
        }

        let mut snapshots_by_id = BTreeMap::new();
        for entry in self
            .content_snapshots()
            .snapshot_catalog(read)
            .map_err(binary_native_repository_store_error)?
        {
            let value = self.canonical_snapshot_value_with_parent_snapshot_ids_and_manifest_cache(
                read,
                &manifest_cache,
                self.repo_name(),
                entry.snapshot_index,
                &entry.record,
                &entry.parent_snapshot_ids,
            )?;
            let key = entry.snapshot_id.to_ascii_uppercase();
            if snapshots_by_id
                .insert(
                    key,
                    BinaryZstdPullCatalogSnapshot {
                        snapshot_id: entry.snapshot_id,
                        parent_snapshot_ids: entry.parent_snapshot_ids,
                        value,
                    },
                )
                .is_some()
            {
                return Err(NativeRepositoryError::internal(
                    "canonical Binary DB Snapshot catalog repeats an identity",
                ));
            }
        }
        manifest_cache.compact_for_immutable_pull_catalog();

        Ok(BinaryZstdPullCatalog {
            revision,
            manifest_cache,
            snapshots_by_id,
            object_pack_rows_by_id,
            tree_pack_rows_by_id,
            blob_locator_rows_by_index,
            tree_locator_rows_by_index,
            tree_entries_by_index,
        })
    }

    fn binary_zstd_pull_manifest_snapshots_with_catalog(
        &self,
        catalog: &BinaryZstdPullCatalog,
        repo_name: &str,
        head_snapshot_id: &str,
        have_snapshot_ids: &BTreeSet<String>,
    ) -> Result<(Vec<JsonValue>, Vec<String>), NativeRepositoryError> {
        let mut pending = VecDeque::from([head_snapshot_id.to_string()]);
        let mut queued = BTreeSet::from([head_snapshot_id.to_string()]);
        let mut boundary_snapshot_ids = BTreeSet::new();
        let mut snapshots = BTreeMap::<String, (JsonValue, Vec<String>)>::new();

        while let Some(snapshot_id) = pending.pop_front() {
            if have_snapshot_ids.contains(&snapshot_id) {
                boundary_snapshot_ids.insert(snapshot_id);
                continue;
            }
            let entry = catalog
                .snapshots_by_id
                .get(&snapshot_id.to_ascii_uppercase())
                .ok_or_else(|| {
                    NativeRepositoryError::not_found(format!(
                        "Unknown snapshot {snapshot_id} for repository {repo_name}"
                    ))
                })?;
            let parents = entry.parent_snapshot_ids.clone();
            let snapshot = entry.value.clone();
            for parent in &parents {
                if queued.insert(parent.clone()) {
                    if queued.len() > ZSTD_PULL_MANIFEST_MAX_SNAPSHOTS {
                        return Err(NativeRepositoryError::bad_request(format!(
                            "Zstd pull manifest ancestry exceeds {ZSTD_PULL_MANIFEST_MAX_SNAPSHOTS} snapshots"
                        )));
                    }
                    pending.push_back(parent.clone());
                }
            }
            snapshots.insert(entry.snapshot_id.clone(), (snapshot, parents));
        }

        let missing_ids = snapshots.keys().cloned().collect::<BTreeSet<_>>();
        let mut child_ids = BTreeMap::<String, BTreeSet<String>>::new();
        let mut unresolved_parent_counts = BTreeMap::<String, usize>::new();
        for (snapshot_id, (_, parents)) in &snapshots {
            let mut unresolved = 0_usize;
            for parent in parents {
                if missing_ids.contains(parent) {
                    unresolved += 1;
                    child_ids
                        .entry(parent.clone())
                        .or_default()
                        .insert(snapshot_id.clone());
                } else if !boundary_snapshot_ids.contains(parent) {
                    return Err(NativeRepositoryError::internal(format!(
                        "Zstd pull manifest ancestry for {snapshot_id} omits parent {parent}"
                    )));
                }
            }
            unresolved_parent_counts.insert(snapshot_id.clone(), unresolved);
        }

        let mut ready = unresolved_parent_counts
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(snapshot_id, _)| snapshot_id.clone())
            .collect::<BTreeSet<_>>();
        let mut ordered = Vec::with_capacity(snapshots.len());
        while let Some(snapshot_id) = ready.pop_first() {
            let (snapshot, _) = snapshots.get(&snapshot_id).ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Zstd pull manifest ordering lost snapshot {snapshot_id}"
                ))
            })?;
            ordered.push(snapshot.clone());
            if let Some(children) = child_ids.get(&snapshot_id) {
                for child_id in children {
                    let count = unresolved_parent_counts.get_mut(child_id).ok_or_else(|| {
                        NativeRepositoryError::internal(format!(
                            "Zstd pull manifest ordering lost child {child_id}"
                        ))
                    })?;
                    *count = count.checked_sub(1).ok_or_else(|| {
                        NativeRepositoryError::internal(format!(
                            "Zstd pull manifest ordering underflowed child {child_id}"
                        ))
                    })?;
                    if *count == 0 {
                        ready.insert(child_id.clone());
                    }
                }
            }
        }
        if ordered.len() != snapshots.len() {
            return Err(NativeRepositoryError::internal(
                "Zstd pull manifest Snapshot ancestry contains a cycle",
            ));
        }
        Ok((
            ordered,
            boundary_snapshot_ids.into_iter().collect::<Vec<_>>(),
        ))
    }

    pub(in super::super) fn binary_zstd_import_manifest_content_for_snapshots(
        &self,
        repo_name: &str,
        snapshots: &[JsonValue],
    ) -> Result<BinaryZstdImportManifestContent, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let catalog = self.binary_zstd_pull_catalog()?;
        self.binary_zstd_import_manifest_content_for_snapshots_with_catalog(&catalog, snapshots)
    }

    fn binary_zstd_import_manifest_content_for_snapshots_with_catalog(
        &self,
        catalog: &BinaryZstdPullCatalog,
        snapshots: &[JsonValue],
    ) -> Result<BinaryZstdImportManifestContent, NativeRepositoryError> {
        let manifest_cache = &catalog.manifest_cache;
        let mut pending_trees = BTreeMap::new();
        for snapshot in snapshots {
            let snapshot_id =
                binary_snapshot_id(snapshot).unwrap_or_else(|| "<unknown>".to_string());
            let root_tree_pack_id =
                binary_json_text(snapshot, "root_tree_pack_id").ok_or_else(|| {
                    NativeRepositoryError::internal(format!(
                        "canonical Binary DB snapshot {snapshot_id} is missing root_tree_pack_id"
                    ))
                })?;
            let root_entry_ordinal = snapshot
                .get("root_entry_ordinal")
                .and_then(JsonValue::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    NativeRepositoryError::internal(format!(
                        "canonical Binary DB snapshot {snapshot_id} has invalid root_entry_ordinal"
                    ))
                })?;
            let root_pack = manifest_cache
                .projected_tree_pack(&root_tree_pack_id)
                .map_err(binary_native_repository_store_error)?
                .ok_or_else(|| {
                    NativeRepositoryError::internal(format!(
                        "canonical Binary DB snapshot {snapshot_id} references missing tree pack {root_tree_pack_id}"
                    ))
                })?;
            let root_tree = manifest_cache
                .projected_tree_for_pack_entry_ordinal(&root_pack, root_entry_ordinal)
                .map_err(binary_native_repository_store_error)?;
            pending_trees
                .entry(root_tree.tree_id.clone())
                .or_insert(root_tree);
        }
        let mut visited_tree_ids = BTreeSet::new();
        let mut tree_pack_rows = BTreeMap::new();
        let mut selected_tree_pack_ids_by_tree_id = BTreeMap::<String, BTreeSet<String>>::new();
        let mut tree_pack_dependencies = BTreeMap::<String, BTreeSet<String>>::new();
        let mut tree_locator_rows = BTreeMap::new();
        let mut referenced_blob_ids = BTreeSet::new();
        while !pending_trees.is_empty() {
            let current_trees = std::mem::take(&mut pending_trees);
            let mut child_tree_sources = BTreeMap::<String, BTreeMap<String, String>>::new();
            for (tree_id, tree) in current_trees {
                if !visited_tree_ids.insert(tree_id.clone()) {
                    continue;
                }
                let pack_id = tree.pack_id.clone();
                if !tree_pack_rows.contains_key(&pack_id) {
                    let pack_view = manifest_cache
                        .projected_tree_pack(&pack_id)
                        .map_err(binary_native_repository_store_error)?
                        .ok_or_else(|| {
                            NativeRepositoryError::internal(format!(
                                "canonical Binary DB tree pack {pack_id} is missing"
                            ))
                        })?;
                    tree_pack_rows.insert(
                        pack_id.clone(),
                        catalog
                            .tree_pack_rows_by_id
                            .get(&pack_id)
                            .cloned()
                            .ok_or_else(|| {
                                NativeRepositoryError::internal(format!(
                                    "validated Binary DB pull catalog has no tree pack row {pack_id}"
                                ))
                            })?,
                    );
                    for pack_tree in manifest_cache
                        .projected_trees_for_tree_pack(&pack_view)
                        .map_err(binary_native_repository_store_error)?
                    {
                        selected_tree_pack_ids_by_tree_id
                            .entry(pack_tree.tree_id.clone())
                            .or_default()
                            .insert(pack_tree.pack_id.clone());
                        tree_locator_rows.insert(
                            pack_tree.tree_id.clone(),
                            catalog
                                .tree_locator_rows_by_index
                                .get(&pack_tree.tree_index)
                                .cloned()
                                .ok_or_else(|| {
                                    NativeRepositoryError::internal(format!(
                                        "validated Binary DB pull catalog has no Tree locator row {}",
                                        pack_tree.tree_id
                                    ))
                                })?,
                        );
                        pending_trees
                            .entry(pack_tree.tree_id.clone())
                            .or_insert(pack_tree);
                    }
                }

                let tree_entries = catalog
                    .tree_entries_by_index
                    .get(&tree.tree_index)
                    .ok_or_else(|| {
                        NativeRepositoryError::internal(format!(
                            "validated Binary DB pull catalog has no Tree entry projection {}",
                            tree.tree_id
                        ))
                    })?;
                for entry in tree_entries {
                    match entry.entry_type.as_str() {
                        "blob" => {
                            referenced_blob_ids.insert(entry.target_id.to_ascii_uppercase());
                        }
                        "tree" => {
                            child_tree_sources
                                .entry(entry.target_id.to_ascii_uppercase())
                                .or_default()
                                .entry(tree.pack_id.clone())
                                .or_insert_with(|| tree.tree_id.clone());
                        }
                        other => {
                            return Err(NativeRepositoryError::internal(format!(
                                "canonical Binary DB tree {} has unsupported entry kind {other}",
                                tree.tree_id
                            )))
                        }
                    }
                }
            }

            let unresolved_child_tree_ids = child_tree_sources
                .keys()
                .filter(|tree_id| !selected_tree_pack_ids_by_tree_id.contains_key(*tree_id))
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut resolved_children = BTreeMap::new();
            for child_tree_id in unresolved_child_tree_ids {
                if let Some(child) = manifest_cache
                    .projected_tree(&child_tree_id)
                    .map_err(binary_native_repository_store_error)?
                {
                    resolved_children.insert(child_tree_id, child);
                }
            }
            for child_tree_id in child_tree_sources.keys() {
                let sources = child_tree_sources
                    .get(child_tree_id)
                    .expect("child tree source must be recorded");
                let source_tree_id = sources
                    .values()
                    .next()
                    .expect("child tree source set must not be empty");
                let child = resolved_children.remove(child_tree_id);
                if let Some(child) = &child {
                    selected_tree_pack_ids_by_tree_id
                        .entry(child_tree_id.clone())
                        .or_default()
                        .insert(child.pack_id.clone());
                }
                let selected_child_pack_ids = selected_tree_pack_ids_by_tree_id
                    .get(child_tree_id)
                    .ok_or_else(|| {
                        NativeRepositoryError::internal(format!(
                            "canonical Binary DB tree {source_tree_id} references missing tree {child_tree_id}"
                        ))
                    })?;
                for source_pack_id in sources.keys() {
                    if !selected_child_pack_ids.contains(source_pack_id) {
                        let required_pack_id = selected_child_pack_ids
                            .iter()
                            .next()
                            .expect("selected child Tree must have a pack");
                        tree_pack_dependencies
                            .entry(source_pack_id.clone())
                            .or_default()
                            .insert(required_pack_id.clone());
                    }
                }
                if let Some(child) = child {
                    pending_trees.insert(child.tree_id.clone(), child);
                }
            }
        }

        let object_content = self.binary_zstd_import_manifest_object_content_with_catalog(
            catalog,
            referenced_blob_ids,
            "tree closure",
        )?;

        Ok(BinaryZstdImportManifestContent {
            object_packs: object_content.object_packs,
            tree_packs: dependency_ordered_pack_rows(
                "tree",
                tree_pack_rows,
                &tree_pack_dependencies,
            )?,
            blob_locators: object_content.blob_locators,
            tree_locators: tree_locator_rows.into_values().collect(),
        })
    }

    /// Builds the exact committed Object Pack and Blob locator closure for an
    /// arbitrary set of Blob identities. Snapshot manifests use the same
    /// closure internally; this entry point additionally supports durable
    /// content such as historical Plan artifacts that need not be reachable
    /// from a Snapshot Tree.
    pub fn get_zstd_blob_import_manifest(
        &self,
        repo_name: &str,
        blob_ids: &[String],
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let referenced_blob_ids = blob_ids
            .iter()
            .map(|blob_id| {
                normalize_required_text(blob_id, "blob_id")
                    .map(|blob_id| blob_id.to_ascii_uppercase())
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let catalog = self.binary_zstd_pull_catalog()?;
        let content = self.binary_zstd_import_manifest_object_content_with_catalog(
            &catalog,
            referenced_blob_ids,
            "requested Blob closure",
        )?;
        Ok(json!({
            "object_packs": content.object_packs,
            "blob_locators": content.blob_locators,
        }))
    }

    fn binary_zstd_import_manifest_object_content_with_catalog(
        &self,
        catalog: &BinaryZstdPullCatalog,
        referenced_blob_ids: BTreeSet<String>,
        closure_label: &str,
    ) -> Result<BinaryZstdImportManifestContent, NativeRepositoryError> {
        let manifest_cache = &catalog.manifest_cache;
        let mut pending_object_packs = BTreeSet::new();
        for blob_id in referenced_blob_ids {
            let blob = manifest_cache
                .projected_blob(&blob_id)
                .map_err(binary_native_repository_store_error)?
                .ok_or_else(|| {
                    NativeRepositoryError::internal(format!(
                        "canonical Binary DB {closure_label} references missing blob {blob_id}"
                    ))
                })?;
            pending_object_packs.insert(blob.pack_id);
        }
        let mut visited_object_packs = BTreeSet::new();
        let mut object_pack_rows = BTreeMap::new();
        let mut object_pack_dependencies = BTreeMap::<String, BTreeSet<String>>::new();
        let mut blob_locator_rows = BTreeMap::new();
        while let Some(pack_id) = pending_object_packs.iter().next().cloned() {
            pending_object_packs.remove(&pack_id);
            if !visited_object_packs.insert(pack_id.clone()) {
                continue;
            }
            let pack_view = manifest_cache
                .projected_object_pack(&pack_id)
                .map_err(binary_native_repository_store_error)?
                .ok_or_else(|| {
                    NativeRepositoryError::internal(format!(
                        "canonical Binary DB object pack {pack_id} is missing"
                    ))
                })?;
            object_pack_rows.insert(
                pack_id.clone(),
                catalog
                    .object_pack_rows_by_id
                    .get(&pack_id)
                    .cloned()
                    .ok_or_else(|| {
                        NativeRepositoryError::internal(format!(
                            "validated Binary DB pull catalog has no object pack row {pack_id}"
                        ))
                    })?,
            );
            let blobs = manifest_cache
                .projected_blobs_for_object_pack(&pack_view)
                .map_err(binary_native_repository_store_error)?;
            let base_blob_sources = blobs
                .iter()
                .filter_map(|blob| {
                    blob.base_blob_id.as_deref().map(|base_blob_id| {
                        (base_blob_id.to_ascii_uppercase(), blob.blob_id.clone())
                    })
                })
                .collect::<BTreeMap<_, _>>();
            for (base_blob_id, source_blob_id) in base_blob_sources {
                let base = manifest_cache
                    .projected_blob(&base_blob_id)
                    .map_err(binary_native_repository_store_error)?
                    .ok_or_else(|| {
                        NativeRepositoryError::internal(format!(
                            "canonical Binary DB blob {source_blob_id} references missing base {base_blob_id}"
                        ))
                    })?;
                if base.pack_id != pack_id {
                    object_pack_dependencies
                        .entry(pack_id.clone())
                        .or_default()
                        .insert(base.pack_id.clone());
                }
                pending_object_packs.insert(base.pack_id);
            }
            for blob in blobs {
                blob_locator_rows.insert(
                    blob.blob_id.clone(),
                    catalog
                        .blob_locator_rows_by_index
                        .get(&blob.blob_index)
                        .cloned()
                        .ok_or_else(|| {
                            NativeRepositoryError::internal(format!(
                                "validated Binary DB pull catalog has no Blob locator row {}",
                                blob.blob_id
                            ))
                        })?,
                );
            }
        }

        Ok(BinaryZstdImportManifestContent {
            object_packs: dependency_ordered_pack_rows(
                "object",
                object_pack_rows,
                &object_pack_dependencies,
            )?,
            blob_locators: blob_locator_rows.into_values().collect(),
            ..BinaryZstdImportManifestContent::default()
        })
    }

    pub(in super::super) fn latest_binary_zstd_record_optional(
        &self,
        repo_name: &str,
        kind: &str,
        field: &str,
        id: &str,
    ) -> Result<Option<JsonValue>, NativeRepositoryError> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        self.latest_binary_zstd_record_optional_with_read(&read, repo_name, kind, field, id)
    }

    pub(in super::super) fn latest_binary_zstd_record_optional_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        repo_name: &str,
        kind: &str,
        field: &str,
        id: &str,
    ) -> Result<Option<JsonValue>, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let store = self.repository_content();
        let value = match kind {
            BINARY_ZSTD_OBJECT_PACK_KIND => {
                if store
                    .object_pack_with_read(read, id)
                    .map_err(binary_native_repository_store_error)?
                    .is_some()
                    || store.object_pack_path(id).is_file()
                {
                    Some(self.typed_pack_metadata_with_read(read, id, false)?)
                } else {
                    None
                }
            }
            BINARY_ZSTD_TREE_PACK_KIND => {
                if store
                    .tree_pack_with_read(read, id)
                    .map_err(binary_native_repository_store_error)?
                    .is_some()
                    || store.tree_pack_path(id).is_file()
                {
                    Some(self.typed_pack_metadata_with_read(read, id, true)?)
                } else {
                    None
                }
            }
            BINARY_ZSTD_BLOB_LOCATOR_KIND => store
                .blob_with_read(read, id)
                .map_err(binary_native_repository_store_error)?
                .map(|view| self.typed_blob_locator(&view))
                .transpose()?,
            BINARY_ZSTD_TREE_LOCATOR_KIND => store
                .tree_with_read(read, id)
                .map_err(binary_native_repository_store_error)?
                .map(|view| self.typed_tree_locator(&view))
                .transpose()?,
            other => {
                return Err(NativeRepositoryError::internal(format!(
                    "unsupported typed Binary DB content kind {other}"
                )))
            }
        };
        if value
            .as_ref()
            .is_some_and(|value| binary_json_text(value, field).as_deref() != Some(id))
        {
            return Err(NativeRepositoryError::internal(format!(
                "Binary DB zstd metadata identity {kind}:{id} disagrees with field {field}"
            )));
        }
        Ok(value)
    }

    #[cfg(test)]
    fn typed_pack_metadata(
        &self,
        pack_id: &str,
        tree_pack: bool,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        self.typed_pack_metadata_with_read(&read, pack_id, tree_pack)
    }

    fn typed_pack_metadata_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        pack_id: &str,
        tree_pack: bool,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let store = self.repository_content();
        let committed = if tree_pack {
            store
                .tree_pack_with_read(read, pack_id)
                .map_err(binary_native_repository_store_error)?
                .is_some()
        } else {
            store
                .object_pack_with_read(read, pack_id)
                .map_err(binary_native_repository_store_error)?
                .is_some()
        };
        let bytes = self.read_zstd_pack_payload(&JsonValue::Null, tree_pack, pack_id)?;
        self.binary_zstd_pack_metadata(
            self.repo_name(),
            pack_id,
            &bytes,
            tree_pack,
            committed,
            None,
        )
    }

    fn committed_object_pack_metadata(
        &self,
        pack: &crate::foundation::server_content_binary_db::ServerBinaryObjectPackView,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let store = self.repository_content();
        let path = store.object_pack_path(&pack.pack_id);
        let path_text = path_to_string(&path)?;
        let archive = PackEntryArchive::open_with_format(&path_text, PACK_FORMAT_ZSTD_CHUNKED_V1)
            .map_err(NativeRepositoryError::internal)?;
        let (pack_index, index_checksum) = archive
            .index_json_and_checksum()
            .map_err(NativeRepositoryError::internal)?;
        self.committed_pack_metadata_from_index(
            &pack.pack_id,
            false,
            pack.record.is_ready(),
            pack.record.pack_format_kind,
            u64::from(pack.record.member_count),
            pack.record.total_bytes,
            pack.record.created_at_s,
            &path,
            pack_index,
            index_checksum,
        )
    }

    fn committed_tree_pack_metadata_with_cache(
        &self,
        pack: &crate::foundation::server_content_binary_db::ServerBinaryTreePackView,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let store = self.repository_content();
        let path = store.tree_pack_path(&pack.pack_id);
        let (pack_index, index_checksum) = store
            .tree_pack_index_metadata_with_read_cache(pack, cache)
            .map_err(binary_native_repository_store_error)?;
        self.committed_pack_metadata_from_index(
            &pack.pack_id,
            true,
            pack.record.is_ready(),
            pack.record.pack_format_kind,
            u64::from(pack.record.tree_count),
            pack.record.total_bytes,
            pack.record.created_at_s,
            &path,
            pack_index,
            index_checksum,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn committed_pack_metadata_from_index(
        &self,
        pack_id: &str,
        tree_pack: bool,
        ready: bool,
        format_kind: u8,
        count: u64,
        total_bytes: u64,
        created_at_s: u64,
        path: &Path,
        pack_index: JsonValue,
        index_checksum: String,
    ) -> Result<JsonValue, NativeRepositoryError> {
        if !ready {
            return Err(NativeRepositoryError::internal(format!(
                "canonical Binary DB {} pack {pack_id} is not committed",
                if tree_pack { "tree" } else { "object" }
            )));
        }
        if format_kind != 1 {
            return Err(NativeRepositoryError::internal(format!(
                "canonical Binary DB {} pack {pack_id} is not zstd chunked",
                if tree_pack { "tree" } else { "object" }
            )));
        }

        let metadata = std::fs::metadata(path).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to stat Binary DB {} pack {pack_id} at {}: {error}",
                if tree_pack { "tree" } else { "object" },
                path.display()
            ))
        })?;
        if !metadata.is_file() {
            return Err(NativeRepositoryError::internal(format!(
                "Binary DB {} pack {pack_id} is not a regular file",
                if tree_pack { "tree" } else { "object" }
            )));
        }
        validate_remote_sync_uploaded_zstd_pack_index_metadata(
            &pack_index,
            &JsonMap::new(),
            pack_id,
            tree_pack,
            None,
        )?;
        let object = pack_index.as_object().ok_or_else(|| {
            NativeRepositoryError::internal(format!(
                "zstd pack {pack_id} index metadata must be an object"
            ))
        })?;
        let count_field = if tree_pack {
            "tree_count"
        } else {
            "member_count"
        };
        let indexed_count = required_i64_field(object, count_field)?;
        let indexed_total_bytes = required_i64_field(object, "total_bytes")?;
        if u64::try_from(indexed_count).ok() != Some(count)
            || u64::try_from(indexed_total_bytes).ok() != Some(total_bytes)
        {
            return Err(NativeRepositoryError::internal(format!(
                "Binary DB {} pack {pack_id} fixed metadata disagrees with its zstd index",
                if tree_pack { "tree" } else { "object" }
            )));
        }
        let index_entry_name = required_json_text(object, "index_entry_name")
            .map_err(NativeRepositoryError::internal)?;
        let created_at = timestamp_rfc3339(created_at_s)?;
        Ok(json!({
            BINARY_ZSTD_PAYLOAD_KIND_FIELD: if tree_pack { BINARY_ZSTD_TREE_PACK_KIND } else { BINARY_ZSTD_OBJECT_PACK_KIND },
            "repo_name": self.repo_name(),
            "repo_id": self.repo_id(),
            "pack_id": pack_id,
            "pack_format": if tree_pack { REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1 } else { REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1 },
            "status": "ready",
            count_field: count,
            "total_bytes": total_bytes,
            "pack_index_entry_name": index_entry_name,
            "pack_index_checksum": index_checksum,
            "payload_total_bytes": metadata.len(),
            "pack_index": pack_index,
            "created_at": created_at,
            "updated_at": created_at,
            "raw_binary_upload": true,
        }))
    }

    fn typed_blob_locator(
        &self,
        view: &crate::foundation::server_content_binary_db::ServerBinaryBlobView,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let entry_type = if view.member.member_meta & 0b0000_0011 == 1 {
            "delta"
        } else {
            "full"
        };
        Ok(json!({
            BINARY_ZSTD_PAYLOAD_KIND_FIELD: BINARY_ZSTD_BLOB_LOCATOR_KIND,
            "repo_name": self.repo_name(),
            "repo_id": self.repo_id(),
            "blob_id": view.blob_id,
            "sha256": hex_bytes(&view.record.sha256),
            "size_bytes": view.record.size_bytes,
            "storage_kind": "pack_full",
            "pack_id": view.pack_id,
            "pack_entry_name": format!("blobs/{}", view.blob_id),
            "pack_entry_type": entry_type,
            "pack_base_blob_id": view.base_blob_id,
            "pack_chain_depth": view.member.delta_chain_depth,
            "created_at": timestamp_rfc3339(view.record.created_at_s)?,
            "updated_at": timestamp_rfc3339(view.record.created_at_s)?,
        }))
    }

    fn typed_tree_locator(
        &self,
        view: &crate::foundation::server_content_binary_db::ServerBinaryTreeView,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.typed_tree_locator_with_cache(view, &mut BinaryTreePackChecksumCache::default())
    }

    fn typed_tree_locator_with_cache(
        &self,
        view: &crate::foundation::server_content_binary_db::ServerBinaryTreeView,
        cache: &mut BinaryTreePackChecksumCache,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let checksum = cache.checksum(&view.pack_id, &view.tree_id, || {
            let path = self.repository_content().tree_pack_path(&view.pack_id);
            let path = path_to_string(&path)?;
            read_tree_pack_index_with_format(&path, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
                .map_err(NativeRepositoryError::internal)
        })?;
        self.typed_tree_locator_with_checksum(view, checksum)
    }

    fn typed_tree_locator_with_manifest_cache(
        &self,
        view: &crate::foundation::server_content_binary_db::ServerBinaryTreeView,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let pack = crate::foundation::server_content_binary_db::ServerBinaryTreePackView {
            pack_index: view.pack_index,
            pack_id: view.pack_id.clone(),
            record: view.pack.clone(),
        };
        let checksum = self
            .repository_content()
            .tree_pack_tree_checksum_with_read_cache(&pack, &view.tree_id, cache)
            .map_err(binary_native_repository_store_error)?;
        self.typed_tree_locator_with_checksum(view, checksum)
    }

    fn typed_tree_locator_with_checksum(
        &self,
        view: &crate::foundation::server_content_binary_db::ServerBinaryTreeView,
        checksum: String,
    ) -> Result<JsonValue, NativeRepositoryError> {
        Ok(json!({
            BINARY_ZSTD_PAYLOAD_KIND_FIELD: BINARY_ZSTD_TREE_LOCATOR_KIND,
            "repo_name": self.repo_name(),
            "repo_id": self.repo_id(),
            "tree_id": view.tree_id,
            "entry_count": view.record.entry_count,
            "tree_pack_id": view.pack_id,
            "tree_pack_checksum": checksum,
            "created_at": timestamp_rfc3339(view.pack.created_at_s)?,
            "updated_at": timestamp_rfc3339(view.pack.created_at_s)?,
        }))
    }

    pub(in super::super) fn latest_binary_zstd_record(
        &self,
        repo_name: &str,
        kind: &str,
        field: &str,
        id: &str,
        label: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.latest_binary_zstd_record_optional(repo_name, kind, field, id)?
            .ok_or_else(|| {
                NativeRepositoryError::not_found(format!(
                    "Unknown zstd {label} {id} for repository {repo_name}"
                ))
            })
    }

    pub(in crate::foundation::native_repositories::service) fn read_zstd_pack_payload(
        &self,
        _value: &JsonValue,
        tree_pack: bool,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        #[cfg(test)]
        TEST_ZSTD_PACK_PAYLOAD_READ_COUNT.with(|count| count.set(count.get() + 1));
        let path = if tree_pack {
            self.repository_content().tree_pack_path(pack_id)
        } else {
            self.repository_content().object_pack_path(pack_id)
        };
        std::fs::read(&path).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to read Binary DB {} pack {pack_id} at {}: {error}",
                if tree_pack { "tree" } else { "object" },
                path.display()
            ))
        })
    }

    #[cfg(test)]
    pub(in crate::foundation) fn reset_test_zstd_pack_payload_read_count(&self) {
        TEST_ZSTD_PACK_PAYLOAD_READ_COUNT.with(|count| count.set(0));
    }

    #[cfg(test)]
    pub(in crate::foundation) fn test_zstd_pack_payload_read_count(&self) -> u64 {
        TEST_ZSTD_PACK_PAYLOAD_READ_COUNT.with(std::cell::Cell::get)
    }

    #[cfg(test)]
    pub(in crate::foundation) fn reset_test_import_manifest_read_counts(&self) {
        crate::foundation::server_content_binary_db::reset_test_content_read_ranges();
        crate::foundation::pack_substrate::reset_test_zstd_file_read_counts();
    }

    #[cfg(test)]
    pub(in crate::foundation) fn test_import_manifest_record_read_ranges(
        &self,
        file: &str,
    ) -> Vec<(u32, u32)> {
        crate::foundation::server_content_binary_db::test_content_record_read_ranges(file)
    }

    #[cfg(test)]
    pub(in crate::foundation) fn test_import_manifest_payload_read_ranges(
        &self,
        file: &str,
    ) -> Vec<(u64, u32)> {
        crate::foundation::server_content_binary_db::test_content_payload_read_ranges(file)
    }

    #[cfg(test)]
    pub(in crate::foundation) fn test_import_manifest_zstd_file_read_counts(
        &self,
    ) -> (u64, u64, u64) {
        crate::foundation::pack_substrate::test_zstd_file_read_counts()
    }

    /// Reads a logical content blob from the remote Binary DB pack authority.
    /// Plan revisions use this path through their schema-owned
    /// `artifact_blob_id`; no extra Plan-specific body file participates.
    #[cfg(test)]
    pub(in crate::foundation) fn read_binary_blob_content(
        &self,
        blob_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        let mut session = BinaryBlobReadSession::default();
        self.read_binary_blob_content_with_session(blob_id, &mut session)
    }

    pub(in super::super) fn read_binary_blob_content_with_session(
        &self,
        blob_id: &str,
        session: &mut BinaryBlobReadSession,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        let mut visited = HashSet::new();
        self.read_binary_blob_content_inner(blob_id, &mut visited, session)?
            .ok_or_else(|| NativeRepositoryError::not_found(format!("Unknown blob: {blob_id}")))
    }

    fn read_binary_blob_content_inner(
        &self,
        blob_id: &str,
        visited: &mut HashSet<String>,
        session: &mut BinaryBlobReadSession,
    ) -> Result<Option<Vec<u8>>, NativeRepositoryError> {
        if let Some(bytes) = session.resolved_blobs.get(blob_id) {
            return Ok(Some(bytes.clone()));
        }
        if visited.len() > crate::foundation::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH {
            return Err(NativeRepositoryError::internal(format!(
                "Pack delta chain depth exceeded for blobs/{blob_id}"
            )));
        }
        if !visited.insert(blob_id.to_string()) {
            return Err(NativeRepositoryError::internal(format!(
                "Cyclic pack delta chain detected for blobs/{blob_id}"
            )));
        }
        let Some(blob) = self
            .repository_content()
            .blob(blob_id)
            .map_err(binary_native_repository_store_error)?
        else {
            visited.remove(blob_id);
            return Ok(None);
        };
        let pack_id = blob.pack_id.clone();
        if !session.ready_pack_ids.contains(&pack_id) {
            if !blob.pack.is_ready() {
                return Err(NativeRepositoryError::internal(format!(
                    "Binary DB object pack {pack_id} for blob {blob_id} is not ready"
                )));
            }
            session.ready_pack_ids.insert(pack_id.clone());
        }
        let mut base_blob_map = BTreeMap::new();
        if blob.member.member_meta & 0b0000_0011 == 1 {
            let base_blob_id = blob.base_blob_id.clone().ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB delta blob {blob_id} is missing its base blob identity"
                ))
            })?;
            let base = self
                .read_binary_blob_content_inner(&base_blob_id, visited, session)?
                .ok_or_else(|| {
                    NativeRepositoryError::internal(format!(
                        "Binary DB delta blob {blob_id} is missing base blob {base_blob_id}"
                    ))
                })?;
            base_blob_map.insert(base_blob_id, base);
        }
        if !session.pack_archives.contains_key(&pack_id) {
            let pack_path = self.repository_content().object_pack_path(&pack_id);
            let pack_path_text = path_to_string(&pack_path)?;
            let archive =
                PackEntryArchive::open_with_format(&pack_path_text, PACK_FORMAT_ZSTD_CHUNKED_V1)
                    .map_err(|error| {
                        NativeRepositoryError::internal(format!(
                            "failed to open Binary DB object pack {pack_id} at {}: {error}",
                            pack_path.display()
                        ))
                    })?;
            session.pack_archives.insert(pack_id.clone(), archive);
        }
        let bytes = session
            .pack_archives
            .get_mut(&pack_id)
            .ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB object pack reader {pack_id} was not retained"
                ))
            })?
            .read_entry(
                &format!("blobs/{blob_id}"),
                (!base_blob_map.is_empty()).then_some(&base_blob_map),
                crate::foundation::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH,
            )
            .map_err(NativeRepositoryError::internal)?;
        if usize::try_from(blob.record.size_bytes).ok() != Some(bytes.len()) {
            return Err(NativeRepositoryError::internal(format!(
                "Binary DB blob {blob_id} size disagrees with its locator"
            )));
        }
        let expected = hex_bytes(&blob.record.sha256);
        let actual = sha256_hex(&bytes);
        if expected != actual {
            return Err(NativeRepositoryError::internal(format!(
                "Binary DB blob {blob_id} checksum disagrees with its locator"
            )));
        }
        visited.remove(blob_id);
        session
            .resolved_blobs
            .insert(blob_id.to_string(), bytes.clone());
        Ok(Some(bytes))
    }

    pub(in super::super) fn binary_zstd_pack_metadata(
        &self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
        tree_pack: bool,
        committed: bool,
        object: Option<&JsonMap<String, JsonValue>>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (pack_index, detected_checksum) =
            zstd_pack_index_from_bytes(pack_bytes, pack_id, tree_pack)?;
        self.binary_zstd_pack_metadata_from_evidence(
            repo_name,
            pack_id,
            tree_pack,
            committed,
            object,
            BinaryZstdPackPayloadEvidence {
                pack_index,
                detected_checksum,
                pack_sha256: sha256_hex(pack_bytes),
                payload_bytes: pack_bytes.len() as u64,
            },
        )
    }

    fn binary_zstd_pack_metadata_from_staged_upload(
        &self,
        repo_name: &str,
        upload: &NativeZstdPackUpload,
        payload_bytes: u64,
        payload_sha256: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let (pack_index, detected_checksum) = zstd_pack_index_from_path(
            upload.temporary_path(),
            upload.pack_id(),
            upload.kind().is_tree(),
        )?;
        self.binary_zstd_pack_metadata_from_evidence(
            repo_name,
            upload.pack_id(),
            upload.kind().is_tree(),
            false,
            None,
            BinaryZstdPackPayloadEvidence {
                pack_index,
                detected_checksum,
                pack_sha256: payload_sha256.to_string(),
                payload_bytes,
            },
        )
    }

    fn binary_zstd_pack_metadata_from_evidence(
        &self,
        repo_name: &str,
        pack_id: &str,
        tree_pack: bool,
        committed: bool,
        object: Option<&JsonMap<String, JsonValue>>,
        evidence: BinaryZstdPackPayloadEvidence,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let BinaryZstdPackPayloadEvidence {
            pack_index,
            detected_checksum,
            pack_sha256,
            payload_bytes,
        } = evidence;
        if let Some(object) = object {
            validate_remote_sync_uploaded_zstd_pack_index_metadata(
                &pack_index,
                object,
                pack_id,
                tree_pack,
                detected_checksum.as_deref(),
            )?;
        } else {
            validate_remote_sync_uploaded_zstd_pack_index_metadata(
                &pack_index,
                &JsonMap::new(),
                pack_id,
                tree_pack,
                detected_checksum.as_deref(),
            )?;
        }
        let count_field = if tree_pack {
            "tree_count"
        } else {
            "member_count"
        };
        let pack_format = if tree_pack {
            REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1
        } else {
            REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1
        };
        let created_at = object
            .and_then(|object| optional_json_text(object, "created_at"))
            .or_else(|| {
                pack_index
                    .get("created_at")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(now_rfc3339);
        let pack_index_object = pack_index.as_object().ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "zstd pack {pack_id} index must be an object"
            ))
        })?;
        let count = required_i64_field(pack_index_object, count_field)?;
        let total_bytes = required_i64_field(pack_index_object, "total_bytes")?;
        let index_entry_name = required_json_text(pack_index_object, "index_entry_name")
            .map_err(NativeRepositoryError::bad_request)?;
        let index_checksum = detected_checksum.ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "zstd pack {pack_id} is missing current index checksum"
            ))
        })?;
        let payload_total_bytes = i64::try_from(payload_bytes).map_err(|_| {
            NativeRepositoryError::bad_request(format!(
                "zstd pack {pack_id} payload length exceeds i64"
            ))
        })?;
        Ok(json!({
            BINARY_ZSTD_PAYLOAD_KIND_FIELD: if tree_pack { BINARY_ZSTD_TREE_PACK_KIND } else { BINARY_ZSTD_OBJECT_PACK_KIND },
            "repo_name": repo_name,
            "repo_id": self.repo_id(),
            "pack_id": pack_id,
            "pack_format": pack_format,
            "status": if committed { "ready" } else { "uploaded" },
            count_field: count,
            "total_bytes": total_bytes,
            "pack_index_entry_name": index_entry_name,
            "pack_index_checksum": index_checksum,
            "pack_sha256": pack_sha256,
            "payload_total_bytes": payload_total_bytes,
            "pack_index": pack_index,
            "created_at": created_at,
            "updated_at": created_at,
            "raw_binary_upload": true,
        }))
    }

    pub(in super::super) fn put_binary_zstd_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
        tree_pack: bool,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let kind = if tree_pack {
            NativeZstdPackKind::Tree
        } else {
            NativeZstdPackKind::Object
        };
        let mut upload = self.begin_binary_zstd_pack_upload(repo_name, pack_id, kind)?;
        let mut file = upload.take_file()?;
        file.write_all(&pack_bytes).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to write staged Binary DB {} pack {}: {error}",
                kind.label(),
                upload.temporary_path().display()
            ))
        })?;
        file.sync_all().map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to sync staged Binary DB {} pack {}: {error}",
                kind.label(),
                upload.temporary_path().display()
            ))
        })?;
        drop(file);
        self.finish_binary_zstd_pack_upload(
            repo_name,
            upload,
            pack_bytes.len() as u64,
            &sha256_hex(&pack_bytes),
        )
    }

    pub(in super::super) fn begin_binary_zstd_pack_upload(
        &self,
        repo_name: &str,
        pack_id: &str,
        kind: NativeZstdPackKind,
    ) -> Result<NativeZstdPackUpload, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        validate_pack_id_segment(pack_id)?;
        let final_path = if kind.is_tree() {
            self.repository_content().tree_pack_path(pack_id)
        } else {
            self.repository_content().object_pack_path(pack_id)
        };
        let parent = final_path.parent().ok_or_else(|| {
            NativeRepositoryError::internal(format!(
                "Binary DB pack path has no parent: {}",
                final_path.display()
            ))
        })?;
        std::fs::create_dir_all(parent).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to create Binary DB pack directory {}: {error}",
                parent.display()
            ))
        })?;

        let final_name = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB pack path has no UTF-8 file name: {}",
                    final_path.display()
                ))
            })?;
        for _ in 0..128 {
            let sequence = ZSTD_STAGED_UPLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temporary_path = parent.join(format!(
                "{final_name}.upload-{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)
            {
                Ok(file) => {
                    return Ok(NativeZstdPackUpload::new(
                        file,
                        temporary_path,
                        final_path,
                        repo_name.to_string(),
                        pack_id.to_string(),
                        kind,
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(NativeRepositoryError::internal(format!(
                        "failed to create staged Binary DB {} pack in {}: {error}",
                        kind.label(),
                        parent.display()
                    )));
                }
            }
        }
        Err(NativeRepositoryError::internal(format!(
            "failed to allocate a unique staged Binary DB {} pack in {}",
            kind.label(),
            parent.display()
        )))
    }

    pub(in super::super) fn finish_binary_zstd_pack_upload(
        &self,
        repo_name: &str,
        mut upload: NativeZstdPackUpload,
        payload_bytes: u64,
        payload_sha256: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        validate_pack_id_segment(upload.pack_id())?;
        if repo_name != upload.repo_name {
            return Err(NativeRepositoryError::bad_request(
                "staged zstd Pack upload repository does not match publication repository",
            ));
        }
        if upload.file.is_some() {
            return Err(NativeRepositoryError::internal(
                "staged zstd Pack upload file must be closed before publication",
            ));
        }
        if payload_bytes == 0 {
            return Err(NativeRepositoryError::bad_request(format!(
                "zstd {} pack body is empty",
                upload.kind().label()
            )));
        }
        if payload_sha256.len() != 64
            || !payload_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(NativeRepositoryError::internal(
                "staged zstd Pack upload has an invalid SHA-256 digest",
            ));
        }

        let expected_final_path = if upload.kind().is_tree() {
            self.repository_content().tree_pack_path(upload.pack_id())
        } else {
            self.repository_content().object_pack_path(upload.pack_id())
        };
        if upload.final_path() != expected_final_path {
            return Err(NativeRepositoryError::internal(format!(
                "staged zstd Pack final path {} does not match repository path {}",
                upload.final_path().display(),
                expected_final_path.display()
            )));
        }
        let parent = expected_final_path.parent().ok_or_else(|| {
            NativeRepositoryError::internal(format!(
                "Binary DB pack path has no parent: {}",
                expected_final_path.display()
            ))
        })?;
        if upload.temporary_path().parent() != Some(parent) {
            return Err(NativeRepositoryError::internal(format!(
                "staged zstd Pack {} is not in final directory {}",
                upload.temporary_path().display(),
                parent.display()
            )));
        }
        let staged_metadata = std::fs::metadata(upload.temporary_path()).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to inspect staged Binary DB {} pack {}: {error}",
                upload.kind().label(),
                upload.temporary_path().display()
            ))
        })?;
        if !staged_metadata.is_file() || staged_metadata.len() != payload_bytes {
            return Err(NativeRepositoryError::internal(format!(
                "staged zstd {} pack length does not match streamed byte count",
                upload.kind().label()
            )));
        }
        self.db
            .sync_file(upload.temporary_path())
            .map_err(binary_native_repository_store_error)?;
        let metadata = self.binary_zstd_pack_metadata_from_staged_upload(
            repo_name,
            &upload,
            payload_bytes,
            &payload_sha256.to_ascii_lowercase(),
        )?;

        let tree_pack = upload.kind().is_tree();
        let label = if tree_pack {
            "tree pack"
        } else {
            "object pack"
        };
        let pack_id = upload.pack_id().to_string();
        let temporary_path = upload.temporary_path().to_path_buf();
        let mut pack_lock = acquire_serving_repository_pack_lock(&self.db)
            .map_err(binary_native_repository_store_error)?;
        // This is the complete RepositoryPack lock boundary: committed-state
        // consistency, final-path comparison, and same-directory rename.
        let publish_result = (|| {
            if expected_final_path.exists() {
                if !binary_zstd_pack_files_equal(&expected_final_path, &temporary_path)? {
                    return Err(NativeRepositoryError::conflict(format!(
                        "{} pack {pack_id} already exists with different content",
                        if tree_pack { "Tree" } else { "Object" }
                    )));
                }
                return Ok("already_present");
            }

            let committed = if tree_pack {
                self.repository_content()
                    .tree_pack(&pack_id)
                    .map_err(binary_native_repository_store_error)?
                    .is_some()
            } else {
                self.repository_content()
                    .object_pack(&pack_id)
                    .map_err(binary_native_repository_store_error)?
                    .is_some()
            };
            if committed {
                return Err(NativeRepositoryError::internal(format!(
                    "Binary DB {label} {pack_id} is committed but its payload is missing"
                )));
            }

            match std::fs::rename(&temporary_path, &expected_final_path) {
                Ok(()) => Ok("uploaded"),
                Err(error) if expected_final_path.exists() => {
                    if !binary_zstd_pack_files_equal(&expected_final_path, &temporary_path)? {
                        return Err(NativeRepositoryError::conflict(format!(
                            "Binary DB pack {pack_id} already exists with different bytes ({error})"
                        )));
                    }
                    Ok("already_present")
                }
                Err(error) => Err(NativeRepositoryError::internal(format!(
                    "failed to publish Binary DB pack {}: {error}",
                    expected_final_path.display()
                ))),
            }
        })();

        // Namespace exclusion ends at rename. Temp cleanup and the final
        // directory durability barrier intentionally happen after release.
        let release_result = pack_lock
            .release()
            .map_err(binary_native_repository_store_error);
        let cleanup_result = match std::fs::remove_file(&temporary_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(NativeRepositoryError::internal(format!(
                "failed to remove Binary DB pack temporary file {}: {error}",
                temporary_path.display()
            ))),
        };
        if cleanup_result.is_ok() {
            upload.disarm_cleanup();
        }
        // A successful rename must reach the directory durability barrier even
        // if lock release reports an error. Failed or idempotent publications
        // also durably remove their staging directory entry.
        let directory_sync_result = self
            .db
            .sync_directory(parent)
            .map_err(binary_native_repository_store_error);
        release_result?;
        cleanup_result?;
        let status = publish_result?;
        directory_sync_result?;
        if status == "already_present" {
            let pack_format = metadata.get("pack_format").cloned().ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB {label} {pack_id} metadata is missing pack_format"
                ))
            })?;
            return Ok(json!({
                "repo_name": repo_name,
                "repo_id": self.repo_id(),
                "pack_id": pack_id,
                "pack_format": pack_format,
                "status": status,
                "raw_binary_upload": true,
            }));
        }
        binary_zstd_pack_upload_response(
            &metadata,
            repo_name,
            self.repo_id(),
            &pack_id,
            label,
            tree_pack,
            status,
        )
    }

    pub fn cleanup_abandoned_zstd_pack_uploads(&self) -> Result<usize, NativeRepositoryError> {
        let store = self.repository_content();
        let pack_paths = [
            store.object_pack_path("cleanup-probe"),
            store.tree_pack_path("cleanup-probe"),
        ];
        let mut cleaned = 0_usize;
        for pack_path in pack_paths {
            let parent = pack_path.parent().ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB pack path has no parent: {}",
                    pack_path.display()
                ))
            })?;
            let entries = match std::fs::read_dir(parent) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(NativeRepositoryError::internal(format!(
                        "failed to scan Binary DB pack directory {}: {error}",
                        parent.display()
                    )));
                }
            };
            let mut cleaned_parent = false;
            for entry in entries {
                let entry = entry.map_err(|error| {
                    NativeRepositoryError::internal(format!(
                        "failed to scan Binary DB pack directory {}: {error}",
                        parent.display()
                    ))
                })?;
                let file_name = entry.file_name();
                let Some(file_name) = file_name.to_str() else {
                    continue;
                };
                if !file_name.contains(ZSTD_STAGED_UPLOAD_MARKER) || !file_name.ends_with(".tmp") {
                    continue;
                }
                let path = entry.path();
                let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                    NativeRepositoryError::internal(format!(
                        "failed to inspect abandoned zstd Pack upload {}: {error}",
                        path.display()
                    ))
                })?;
                if !metadata.file_type().is_file() {
                    continue;
                }
                std::fs::remove_file(&path).map_err(|error| {
                    NativeRepositoryError::internal(format!(
                        "failed to remove abandoned zstd Pack upload {}: {error}",
                        path.display()
                    ))
                })?;
                cleaned += 1;
                cleaned_parent = true;
            }
            if cleaned_parent {
                self.db
                    .sync_directory(parent)
                    .map_err(binary_native_repository_store_error)?;
            }
        }
        Ok(cleaned)
    }

    pub(in super::super) fn prepare_binary_zstd_pack_from_commit(
        &self,
        repo_name: &str,
        object: &JsonMap<String, JsonValue>,
        tree_pack: bool,
        read_set: &BinaryZstdCommitReadSet,
    ) -> Result<(JsonValue, Option<PreparedRepositoryMetadataMutation>), NativeRepositoryError>
    {
        let pack_id =
            required_json_text(object, "pack_id").map_err(NativeRepositoryError::bad_request)?;
        validate_pack_id_segment(&pack_id)?;
        let expected_format = if tree_pack {
            REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1
        } else {
            REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1
        };
        let pack_format = required_json_text(object, "pack_format")
            .map_err(NativeRepositoryError::bad_request)?;
        if pack_format != expected_format {
            return Err(NativeRepositoryError::bad_request(format!(
                "{} pack {pack_id} has unsupported pack_format {pack_format:?}",
                if tree_pack { "Tree" } else { "Object" }
            )));
        }
        let kind = if tree_pack {
            BINARY_ZSTD_TREE_PACK_KIND
        } else {
            BINARY_ZSTD_OBJECT_PACK_KIND
        };
        let Some(existing) = read_set.metadata(kind, &pack_id)?.cloned() else {
            return Err(NativeRepositoryError::bad_request(format!(
                "zstd {} pack {pack_id} was not uploaded before commit",
                if tree_pack { "tree" } else { "object" }
            )));
        };
        let pack_index = existing.get("pack_index").cloned().ok_or_else(|| {
            NativeRepositoryError::internal(format!("zstd pack {pack_id} is missing pack_index"))
        })?;
        let detected_checksum = binary_json_text(&existing, "pack_index_checksum");
        validate_remote_sync_uploaded_zstd_pack_index_metadata(
            &pack_index,
            object,
            &pack_id,
            tree_pack,
            detected_checksum.as_deref(),
        )?;
        let already_ready = binary_json_text(&existing, "status").as_deref() == Some("ready");
        if already_ready {
            return Ok((pack_index, None));
        }
        let pack_bytes = read_set.pack_payload(&pack_id, tree_pack)?;
        let mut metadata = self.binary_zstd_pack_metadata(
            repo_name,
            &pack_id,
            pack_bytes,
            tree_pack,
            true,
            Some(object),
        )?;
        if optional_json_text(object, "created_at").is_none() {
            if let (Some(metadata), Some(created_at)) = (
                metadata.as_object_mut(),
                binary_json_text(&existing, "created_at"),
            ) {
                metadata.insert("created_at".to_string(), json!(created_at));
            }
        }
        Ok((
            pack_index,
            Some(PreparedRepositoryMetadataMutation {
                kind,
                value: metadata,
            }),
        ))
    }

    pub(in super::super) fn prepare_binary_zstd_blob_locator(
        &self,
        repo_name: &str,
        object: &JsonMap<String, JsonValue>,
        object_pack_indexes: &mut BTreeMap<String, JsonValue>,
        read_set: &BinaryZstdCommitReadSet,
    ) -> Result<Option<PreparedRepositoryMetadataMutation>, NativeRepositoryError> {
        let blob_id =
            required_json_text(object, "blob_id").map_err(NativeRepositoryError::bad_request)?;
        let sha256 =
            required_json_text(object, "sha256").map_err(NativeRepositoryError::bad_request)?;
        let size_bytes = required_i64_field(object, "size_bytes")?;
        let pack_id =
            required_json_text(object, "pack_id").map_err(NativeRepositoryError::bad_request)?;
        let entry_name = required_json_text(object, "pack_entry_name")
            .map_err(NativeRepositoryError::bad_request)?;
        let entry_type = required_json_text(object, "pack_entry_type")
            .map_err(NativeRepositoryError::bad_request)?;
        let chain_depth = required_i64_field(object, "pack_chain_depth")?;
        if !object_pack_indexes.contains_key(&pack_id) {
            let index = read_set.pack_index(&pack_id, false)?;
            object_pack_indexes.insert(pack_id.clone(), index);
        }
        validate_object_pack_entry(
            object_pack_indexes,
            &pack_id,
            &blob_id,
            &sha256,
            &entry_type,
        )?;
        if let Some(existing) = read_set.metadata(BINARY_ZSTD_BLOB_LOCATOR_KIND, &blob_id)? {
            let existing_sha = binary_json_text(existing, "sha256").unwrap_or_default();
            if existing_sha != sha256 {
                return Err(NativeRepositoryError::conflict(format!(
                    "Blob locator {blob_id} already exists for repository {repo_name} with different sha256"
                )));
            }
            let same = existing.get("size_bytes").and_then(JsonValue::as_i64) == Some(size_bytes)
                && binary_json_text(existing, "pack_id").as_deref() == Some(pack_id.as_str())
                && binary_json_text(existing, "pack_entry_type").as_deref()
                    == Some(entry_type.as_str())
                && binary_json_text(existing, "pack_entry_name").as_deref()
                    == Some(entry_name.as_str())
                && binary_json_text(existing, "pack_base_blob_id")
                    == optional_json_text(object, "pack_base_blob_id")
                && existing.get("pack_chain_depth").and_then(JsonValue::as_i64)
                    == Some(chain_depth);
            if same {
                return Ok(None);
            }
        }
        let created_at = optional_json_text(object, "created_at").unwrap_or_else(now_rfc3339);
        let value = json!({
            BINARY_ZSTD_PAYLOAD_KIND_FIELD: BINARY_ZSTD_BLOB_LOCATOR_KIND,
            "repo_name": repo_name,
            "repo_id": self.repo_id(),
            "blob_id": blob_id,
            "sha256": sha256,
            "size_bytes": size_bytes,
            "storage_kind": "pack_full",
            "pack_id": pack_id,
            "pack_entry_name": entry_name,
            "pack_entry_type": entry_type,
            "pack_base_blob_id": optional_json_text(object, "pack_base_blob_id"),
            "pack_chain_depth": chain_depth,
            "created_at": created_at,
            "updated_at": created_at,
        });
        Ok(Some(PreparedRepositoryMetadataMutation {
            kind: BINARY_ZSTD_BLOB_LOCATOR_KIND,
            value,
        }))
    }

    pub(in super::super) fn prepare_binary_zstd_tree_locator(
        &self,
        repo_name: &str,
        object: &JsonMap<String, JsonValue>,
        tree_pack_indexes: &mut BTreeMap<String, JsonValue>,
        read_set: &BinaryZstdCommitReadSet,
    ) -> Result<Option<PreparedRepositoryMetadataMutation>, NativeRepositoryError> {
        let tree_id =
            required_json_text(object, "tree_id").map_err(NativeRepositoryError::bad_request)?;
        let entry_count = required_i64_field(object, "entry_count")?;
        let tree_pack_id = required_json_text(object, "tree_pack_id")
            .map_err(NativeRepositoryError::bad_request)?;
        let checksum = required_json_text(object, "tree_pack_checksum")
            .map_err(NativeRepositoryError::bad_request)?;
        if !tree_pack_indexes.contains_key(&tree_pack_id) {
            let index = read_set.pack_index(&tree_pack_id, true)?;
            tree_pack_indexes.insert(tree_pack_id.clone(), index);
        }
        validate_tree_pack_entry(
            tree_pack_indexes,
            &tree_pack_id,
            &tree_id,
            entry_count as i32,
            &checksum,
        )?;
        if let Some(existing) = read_set.metadata(BINARY_ZSTD_TREE_LOCATOR_KIND, &tree_id)? {
            let same = existing.get("entry_count").and_then(JsonValue::as_i64) == Some(entry_count)
                && binary_json_text(existing, "tree_pack_id").as_deref()
                    == Some(tree_pack_id.as_str())
                && binary_json_text(existing, "tree_pack_checksum").as_deref()
                    == Some(checksum.as_str());
            if same {
                return Ok(None);
            }
        }
        let created_at = optional_json_text(object, "created_at").unwrap_or_else(now_rfc3339);
        let value = json!({
            BINARY_ZSTD_PAYLOAD_KIND_FIELD: BINARY_ZSTD_TREE_LOCATOR_KIND,
            "repo_name": repo_name,
            "repo_id": self.repo_id(),
            "tree_id": tree_id,
            "entry_count": entry_count,
            "tree_pack_id": tree_pack_id,
            "tree_pack_checksum": checksum,
            "created_at": created_at,
            "updated_at": created_at,
        });
        Ok(Some(PreparedRepositoryMetadataMutation {
            kind: BINARY_ZSTD_TREE_LOCATOR_KIND,
            value,
        }))
    }

    pub(in super::super) fn prepare_binary_zstd_snapshot(
        &self,
        repo_name: &str,
        object: &JsonMap<String, JsonValue>,
        tree_pack_indexes: &mut BTreeMap<String, JsonValue>,
        incoming_seen: &BTreeSet<String>,
        read_set: &BinaryZstdCommitReadSet,
    ) -> Result<Option<PreparedSnapshotMutation>, NativeRepositoryError> {
        let snapshot_id = required_json_text(object, "snapshot_id")
            .map_err(NativeRepositoryError::bad_request)?;
        let parent_snapshot_id = optional_json_text(object, "parent_snapshot_id");
        let root_tree_pack_id = required_json_text(object, "root_tree_pack_id")
            .map_err(NativeRepositoryError::bad_request)?;
        let root_entry_ordinal = required_i64_field(object, "root_entry_ordinal")?;
        let line_name = optional_json_text(object, "line_name").unwrap_or_else(default_main_line);
        let file_count = required_i64_field(object, "file_count")?;
        let total_bytes = required_i64_field(object, "total_bytes")?;
        if let Some(parent) = parent_snapshot_id.as_deref() {
            if read_set.snapshot(parent)?.is_none() && !incoming_seen.contains(parent) {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Snapshot {snapshot_id} parent {parent} is not present in repository {repo_name} or earlier in this zstd bulk commit"
                )));
            }
        }
        if !tree_pack_indexes.contains_key(&root_tree_pack_id) {
            let index = read_set.pack_index(&root_tree_pack_id, true)?;
            tree_pack_indexes.insert(root_tree_pack_id.clone(), index);
        }
        let root_ordinal = usize::try_from(root_entry_ordinal).map_err(|_| {
            NativeRepositoryError::bad_request(format!(
                "Tree pack {root_tree_pack_id} is missing root entry ordinal {root_entry_ordinal}"
            ))
        })?;
        validate_root_tree_locator_index(
            tree_pack_indexes
                .get(&root_tree_pack_id)
                .expect("root tree pack index should be present"),
            &root_tree_pack_id,
            root_ordinal,
        )?;
        if let Some(existing) = read_set.snapshot(&snapshot_id)? {
            binary_validate_existing_snapshot_payload(
                existing,
                repo_name,
                &line_name,
                parent_snapshot_id.as_deref(),
                file_count,
                total_bytes,
            )?;
            return Ok(None);
        }
        let created_at = optional_json_text(object, "created_at").unwrap_or_else(now_rfc3339);
        let value = json!({
            "repo_name": repo_name,
            "repo_id": self.repo_id(),
            "snapshot_id": snapshot_id,
            "parent_snapshot_id": parent_snapshot_id,
            "root_tree_pack_id": root_tree_pack_id,
            "root_entry_ordinal": root_entry_ordinal,
            "manifest_hash": optional_json_text(object, "manifest_hash").unwrap_or_default(),
            "manifest_path": format!("binary-db:zstd-snapshot/{snapshot_id}"),
            "message": optional_json_text(object, "message"),
            "line_name": line_name,
            "snapshot_kind": optional_json_text(object, "snapshot_kind").unwrap_or_else(|| "line".to_string()),
            "file_count": file_count,
            "total_bytes": total_bytes,
            "created_at": created_at,
            "zstd_snapshot": JsonValue::Object(object.clone()),
        });
        Ok(Some(PreparedSnapshotMutation { snapshot_id, value }))
    }
}

fn dependency_ordered_pack_rows(
    label: &str,
    rows: BTreeMap<String, JsonValue>,
    dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    for (pack_id, required_pack_ids) in dependencies {
        if !rows.contains_key(pack_id) {
            return Err(NativeRepositoryError::internal(format!(
                "zstd {label} pack dependency source {pack_id} is absent from its manifest"
            )));
        }
        if let Some(missing) = required_pack_ids
            .iter()
            .find(|required| !rows.contains_key(*required))
        {
            return Err(NativeRepositoryError::internal(format!(
                "zstd {label} pack {pack_id} depends on manifest-absent pack {missing}"
            )));
        }
    }

    let mut remaining = rows;
    let mut ordered = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let ready = remaining
            .keys()
            .filter(|pack_id| {
                dependencies.get(*pack_id).is_none_or(|required| {
                    required
                        .iter()
                        .all(|required_pack_id| !remaining.contains_key(required_pack_id))
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err(NativeRepositoryError::internal(format!(
                "zstd {label} pack manifest contains a dependency cycle"
            )));
        }
        for pack_id in ready {
            ordered.push(
                remaining
                    .remove(&pack_id)
                    .expect("ready zstd pack row must remain present"),
            );
        }
    }
    Ok(ordered)
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn timestamp_rfc3339(seconds: u64) -> Result<String, NativeRepositoryError> {
    let seconds = i64::try_from(seconds).map_err(|_| {
        NativeRepositoryError::internal("Binary DB timestamp exceeds RFC 3339 range")
    })?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .ok_or_else(|| {
            NativeRepositoryError::internal("Binary DB timestamp exceeds RFC 3339 range")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn tree_pack_checksum_cache_loads_each_pack_index_once() {
        let loads = Cell::new(0_u32);
        let index = json!({
            "trees": [
                {"tree_id": "TRE-ONE", "checksum": "checksum-one"},
                {"tree_id": "TRE-TWO", "checksum": "checksum-two"}
            ]
        });
        let mut cache = BinaryTreePackChecksumCache::default();

        let first = cache
            .checksum("TPK-SHARED", "TRE-ONE", || {
                loads.set(loads.get() + 1);
                Ok(index.clone())
            })
            .expect("first checksum");
        let second = cache
            .checksum("TPK-SHARED", "TRE-TWO", || {
                loads.set(loads.get() + 1);
                Ok(index.clone())
            })
            .expect("second checksum");

        assert_eq!(first, "checksum-one");
        assert_eq!(second, "checksum-two");
        assert_eq!(loads.get(), 1, "one pack index read per shared pack");
    }

    #[test]
    fn tree_pack_checksum_cache_fails_closed_for_missing_tree() {
        let mut cache = BinaryTreePackChecksumCache::default();
        let error = cache
            .checksum("TPK-MISSING", "TRE-MISSING", || Ok(json!({"trees": []})))
            .expect_err("missing tree checksum must fail");

        assert!(error
            .to_string()
            .contains("tree pack TPK-MISSING has no checksum for TRE-MISSING"));
    }
}
