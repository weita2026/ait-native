use super::*;
use crate::foundation::remote_binary_db::acquire_serving_repository_pack_lock;
use crate::foundation::server_content_binary_db::ServerBinaryTreeReadCache;
use std::collections::VecDeque;

const ZSTD_PULL_MANIFEST_MAX_SNAPSHOTS: usize = 100_000;

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
        tx.commit()
            .map(|_| ())
            .map_err(binary_native_repository_store_error)
    }

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

    pub(in super::super) fn binary_zstd_pull_manifest_snapshots(
        &self,
        repo_name: &str,
        head_snapshot_id: &str,
        have_snapshot_ids: &BTreeSet<String>,
    ) -> Result<(Vec<JsonValue>, Vec<String>), NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let mut pending = VecDeque::from([head_snapshot_id.to_string()]);
        let mut queued = BTreeSet::from([head_snapshot_id.to_string()]);
        let mut boundary_snapshot_ids = BTreeSet::new();
        let mut snapshots = BTreeMap::<String, (JsonValue, Vec<String>)>::new();

        while let Some(snapshot_id) = pending.pop_front() {
            if have_snapshot_ids.contains(&snapshot_id) {
                boundary_snapshot_ids.insert(snapshot_id);
                continue;
            }
            let snapshot = self
                .latest_snapshot_value_optional_with_read(&read, repo_name, &snapshot_id)?
                .ok_or_else(|| {
                    NativeRepositoryError::not_found(format!(
                        "Unknown snapshot {snapshot_id} for repository {repo_name}"
                    ))
                })?;
            let parents = binary_zstd_snapshot_parent_ids(&snapshot)?;
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
            snapshots.insert(snapshot_id, (snapshot, parents));
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
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let content = self.repository_content();
        let mut manifest_cache = content
            .manifest_tree_read_cache_with_read(&read)
            .map_err(binary_native_repository_store_error)?;
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
        let mut selected_tree_pack_ids = BTreeSet::new();
        let mut selected_tree_ids = BTreeSet::new();
        while !pending_trees.is_empty() {
            let current_trees = std::mem::take(&mut pending_trees);
            let mut child_tree_sources = BTreeMap::<String, BTreeMap<String, String>>::new();
            for (tree_id, tree) in current_trees {
                if !visited_tree_ids.insert(tree_id.clone()) {
                    continue;
                }
                selected_tree_ids.insert(tree_id.to_ascii_uppercase());
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
                    let pack = self
                        .committed_tree_pack_metadata_with_cache(&pack_view, &mut manifest_cache)?;
                    if binary_json_text(&pack, "status").as_deref() != Some("ready") {
                        return Err(NativeRepositoryError::internal(format!(
                            "canonical Binary DB tree pack {pack_id} is not committed"
                        )));
                    }
                    tree_pack_rows.insert(
                        pack_id.clone(),
                        binary_zstd_import_manifest_pack_row(pack, true)?,
                    );
                    selected_tree_pack_ids.insert(pack_id.to_ascii_uppercase());
                    for pack_tree in manifest_cache
                        .projected_trees_for_tree_pack(&pack_view)
                        .map_err(binary_native_repository_store_error)?
                    {
                        selected_tree_ids.insert(pack_tree.tree_id.to_ascii_uppercase());
                        selected_tree_pack_ids_by_tree_id
                            .entry(pack_tree.tree_id.clone())
                            .or_default()
                            .insert(pack_tree.pack_id.clone());
                        tree_locator_rows.insert(
                            pack_tree.tree_id.clone(),
                            binary_zstd_import_manifest_tree_locator_row(
                                self.typed_tree_locator_with_manifest_cache(
                                    &pack_tree,
                                    &mut manifest_cache,
                                )?,
                            )?,
                        );
                        pending_trees
                            .entry(pack_tree.tree_id.clone())
                            .or_insert(pack_tree);
                    }
                }

                for entry in content
                    .projected_tree_entries_for_tree_with_read_cache(
                        &read,
                        &tree,
                        &mut manifest_cache,
                    )
                    .map_err(binary_native_repository_store_error)?
                {
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

        let mut selected_object_pack_ids = BTreeSet::new();
        let mut selected_blob_ids = BTreeSet::new();
        let object_content = self.binary_zstd_import_manifest_object_content_with_read(
            &manifest_cache,
            referenced_blob_ids,
            "tree closure",
            &mut selected_object_pack_ids,
            &mut selected_blob_ids,
        )?;
        content
            .validate_manifest_identity_indexes_with_read(
                &read,
                &manifest_cache,
                &selected_object_pack_ids,
                &selected_tree_pack_ids,
                &selected_blob_ids,
                &selected_tree_ids,
            )
            .map_err(binary_native_repository_store_error)?;

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
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let content_store = self.repository_content();
        let manifest_cache = content_store
            .manifest_object_read_cache_with_read(&read)
            .map_err(binary_native_repository_store_error)?;
        let mut selected_object_pack_ids = BTreeSet::new();
        let mut selected_blob_ids = BTreeSet::new();
        let content = self.binary_zstd_import_manifest_object_content_with_read(
            &manifest_cache,
            referenced_blob_ids,
            "requested Blob closure",
            &mut selected_object_pack_ids,
            &mut selected_blob_ids,
        )?;
        content_store
            .validate_manifest_identity_indexes_with_read(
                &read,
                &manifest_cache,
                &selected_object_pack_ids,
                &BTreeSet::new(),
                &selected_blob_ids,
                &BTreeSet::new(),
            )
            .map_err(binary_native_repository_store_error)?;
        Ok(json!({
            "object_packs": content.object_packs,
            "blob_locators": content.blob_locators,
        }))
    }

    fn binary_zstd_import_manifest_object_content_with_read(
        &self,
        manifest_cache: &ServerBinaryTreeReadCache,
        referenced_blob_ids: BTreeSet<String>,
        closure_label: &str,
        selected_object_pack_ids: &mut BTreeSet<String>,
        selected_blob_ids: &mut BTreeSet<String>,
    ) -> Result<BinaryZstdImportManifestContent, NativeRepositoryError> {
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
            selected_blob_ids.insert(blob.blob_id.to_ascii_uppercase());
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
            let pack = self.committed_object_pack_metadata(&pack_view)?;
            if binary_json_text(&pack, "status").as_deref() != Some("ready") {
                return Err(NativeRepositoryError::internal(format!(
                    "canonical Binary DB object pack {pack_id} is not committed"
                )));
            }
            object_pack_rows.insert(
                pack_id.clone(),
                binary_zstd_import_manifest_pack_row(pack, false)?,
            );
            selected_object_pack_ids.insert(pack_id.to_ascii_uppercase());
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
                selected_blob_ids.insert(base.blob_id.to_ascii_uppercase());
                if base.pack_id != pack_id {
                    object_pack_dependencies
                        .entry(pack_id.clone())
                        .or_default()
                        .insert(base.pack_id.clone());
                }
                pending_object_packs.insert(base.pack_id);
            }
            for blob in blobs {
                selected_blob_ids.insert(blob.blob_id.to_ascii_uppercase());
                blob_locator_rows.insert(
                    blob.blob_id.clone(),
                    binary_zstd_import_manifest_blob_locator_row(self.typed_blob_locator(&blob)?)?,
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
            "pack_sha256": sha256_hex(pack_bytes),
            "payload_total_bytes": pack_bytes.len() as i64,
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
        validate_pack_id_segment(pack_id)?;
        if pack_bytes.is_empty() {
            return Err(NativeRepositoryError::bad_request(if tree_pack {
                "zstd tree pack body is empty"
            } else {
                "zstd object pack body is empty"
            }));
        }
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
        if let Some(existing) =
            self.latest_binary_zstd_record_optional(repo_name, kind, "pack_id", pack_id)?
        {
            let existing_bytes = self.read_zstd_pack_payload(&existing, tree_pack, pack_id)?;
            if existing_bytes != pack_bytes {
                return Err(NativeRepositoryError::conflict(format!(
                    "{} pack {pack_id} already exists with different content",
                    if tree_pack { "Tree" } else { "Object" }
                )));
            }
            let pack_format = binary_json_text(&existing, "pack_format").ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Binary DB {label} {pack_id} is missing pack_format"
                ))
            })?;
            return Ok(json!({
                "repo_name": repo_name,
                "repo_id": self.repo_id(),
                "pack_id": pack_id,
                "pack_format": pack_format,
                "status": "already_present",
                "raw_binary_upload": true,
            }));
        }
        let metadata = self.binary_zstd_pack_metadata(
            repo_name,
            pack_id,
            &pack_bytes,
            tree_pack,
            false,
            None,
        )?;
        let path = if tree_pack {
            self.repository_content().tree_pack_path(pack_id)
        } else {
            self.repository_content().object_pack_path(pack_id)
        };
        let parent = path.parent().ok_or_else(|| {
            NativeRepositoryError::internal(format!(
                "Binary DB pack path has no parent: {}",
                path.display()
            ))
        })?;
        std::fs::create_dir_all(parent).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to create Binary DB pack directory {}: {error}",
                parent.display()
            ))
        })?;

        // Pack validation happened above. Prepare and durably flush the unique
        // temporary inode before taking the repository-pack namespace lock so
        // slow storage cannot block unrelated pack publishers.
        let temp = path.with_extension(format!(
            "zstpack.tmp-{}",
            new_identifier("upload-pack", pack_id)
        ));
        std::fs::write(&temp, &pack_bytes).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to write Binary DB pack temporary file {}: {error}",
                temp.display()
            ))
        })?;
        if let Err(error) = self.db.sync_file(&temp) {
            let _ = std::fs::remove_file(&temp);
            return Err(binary_native_repository_store_error(error));
        }

        let mut pack_lock = match acquire_serving_repository_pack_lock(&self.db) {
            Ok(lock) => lock,
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                return Err(binary_native_repository_store_error(error));
            }
        };
        // This is the complete RepositoryPack lock boundary: final-path
        // existence/content comparison plus the atomic namespace rename.
        let publish_result = (|| {
            if path.exists() {
                let existing = std::fs::read(&path).map_err(|read_error| {
                    NativeRepositoryError::internal(format!(
                        "failed to read locked Binary DB pack {}: {read_error}",
                        path.display()
                    ))
                })?;
                if existing != pack_bytes {
                    return Err(NativeRepositoryError::conflict(format!(
                        "{} pack {pack_id} already exists with different content",
                        if tree_pack { "Tree" } else { "Object" }
                    )));
                }
                return Ok("already_present");
            }

            match std::fs::rename(&temp, &path) {
                Ok(()) => Ok("uploaded"),
                Err(error) if path.exists() => {
                    let existing = std::fs::read(&path).map_err(|read_error| {
                        NativeRepositoryError::internal(format!(
                            "failed to read concurrent Binary DB pack {}: {read_error}",
                            path.display()
                        ))
                    })?;
                    if existing != pack_bytes {
                        return Err(NativeRepositoryError::conflict(format!(
                            "Binary DB pack {pack_id} already exists with different bytes ({error})"
                        )));
                    }
                    Ok("already_present")
                }
                Err(error) => Err(NativeRepositoryError::internal(format!(
                    "failed to publish Binary DB pack {}: {error}",
                    path.display()
                ))),
            }
        })();

        // Namespace exclusion ends at rename. Temp cleanup and the final
        // directory durability barrier intentionally happen after release.
        let release_result = pack_lock
            .release()
            .map_err(binary_native_repository_store_error);
        let cleanup_result = match std::fs::remove_file(&temp) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(NativeRepositoryError::internal(format!(
                "failed to remove Binary DB pack temporary file {}: {error}",
                temp.display()
            ))),
        };
        release_result?;
        cleanup_result?;
        let status = publish_result?;
        self.db
            .sync_directory(parent)
            .map_err(binary_native_repository_store_error)?;
        binary_zstd_pack_upload_response(
            &metadata,
            repo_name,
            self.repo_id(),
            pack_id,
            label,
            tree_pack,
            status,
        )
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
            let existing_sha = binary_json_text(&existing, "sha256").unwrap_or_default();
            if existing_sha != sha256 {
                return Err(NativeRepositoryError::conflict(format!(
                    "Blob locator {blob_id} already exists for repository {repo_name} with different sha256"
                )));
            }
            let same = existing.get("size_bytes").and_then(JsonValue::as_i64) == Some(size_bytes)
                && binary_json_text(&existing, "pack_id").as_deref() == Some(pack_id.as_str())
                && binary_json_text(&existing, "pack_entry_type").as_deref()
                    == Some(entry_type.as_str())
                && binary_json_text(&existing, "pack_entry_name").as_deref()
                    == Some(entry_name.as_str())
                && binary_json_text(&existing, "pack_base_blob_id")
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
                && binary_json_text(&existing, "tree_pack_id").as_deref()
                    == Some(tree_pack_id.as_str())
                && binary_json_text(&existing, "tree_pack_checksum").as_deref()
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
                &existing,
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

fn binary_zstd_snapshot_parent_ids(
    snapshot: &JsonValue,
) -> Result<Vec<String>, NativeRepositoryError> {
    let snapshot_id = binary_snapshot_id(snapshot).ok_or_else(|| {
        NativeRepositoryError::internal("Snapshot payload is missing snapshot_id")
    })?;
    let mut parents = match snapshot.get("parent_snapshot_ids") {
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .ok_or_else(|| {
                        NativeRepositoryError::internal(format!(
                            "Snapshot {snapshot_id} has an invalid parent_snapshot_ids entry"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(JsonValue::Null) | None => binary_json_text(snapshot, "parent_snapshot_id")
            .into_iter()
            .collect(),
        Some(_) => {
            return Err(NativeRepositoryError::internal(format!(
                "Snapshot {snapshot_id} has invalid parent_snapshot_ids"
            )))
        }
    };
    if parents.is_empty() {
        if let Some(parent) = binary_json_text(snapshot, "parent_snapshot_id") {
            parents.push(parent);
        }
    }
    let mut unique = BTreeSet::new();
    for parent in &parents {
        if parent == &snapshot_id {
            return Err(NativeRepositoryError::internal(format!(
                "Snapshot {snapshot_id} cannot be its own parent"
            )));
        }
        if !unique.insert(parent.clone()) {
            return Err(NativeRepositoryError::internal(format!(
                "Snapshot {snapshot_id} contains duplicate parent {parent}"
            )));
        }
    }
    Ok(parents)
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
