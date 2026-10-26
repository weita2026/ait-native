use super::*;
use crate::foundation::remote_binary_db::{BinaryDbFsyncPolicy, BinaryDbWriteTxn};
use crate::foundation::workflow_package::WorkflowPackage;

/// A read-prepared content append. Applying it never opens or commits a writer.
pub struct PreparedWorkflowPackageContent {
    object_packs: Vec<JsonValue>,
    tree_packs: Vec<JsonValue>,
    blob_locators: Vec<JsonValue>,
    tree_locators: Vec<JsonValue>,
    snapshots: Vec<(String, JsonValue)>,
    expected_snapshots: Vec<JsonValue>,
}

impl<D> BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub fn prepare_workflow_package_content(
        &self,
        repository: &str,
        package: &WorkflowPackage,
    ) -> Result<PreparedWorkflowPackageContent, NativeRepositoryError> {
        package
            .manifest
            .validate()
            .map_err(NativeRepositoryError::bad_request)?;
        let content = &package.manifest.content;
        let objects = content.object_packs.iter().collect::<Vec<_>>();
        let trees = content.tree_packs.iter().collect::<Vec<_>>();
        let blobs = content.blob_locators.iter().collect::<Vec<_>>();
        let tree_locators = content.tree_locators.iter().collect::<Vec<_>>();
        let snapshots = content.snapshots.iter().collect::<Vec<_>>();
        let mut read = self.prefetch_binary_zstd_commit_read_set(
            repository,
            &objects,
            &trees,
            &blobs,
            &tree_locators,
            &snapshots,
            None,
        )?;
        for (rows, directory, tree) in [
            (&content.object_packs, "objects", false),
            (&content.tree_packs, "trees", true),
        ] {
            for row in rows {
                let id = required_json_text(json_object(row, "package pack")?, "pack_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                let bytes = std::fs::read(package.inputs.join(directory).join(&id))
                    .map_err(|error| NativeRepositoryError::internal(error.to_string()))?;
                self.stage_package_pack_read(repository, row, bytes, tree, &mut read)?;
            }
        }
        let mut object_indexes = BTreeMap::new();
        let mut tree_indexes = BTreeMap::new();
        let mut prepared = PreparedWorkflowPackageContent {
            object_packs: Vec::new(),
            tree_packs: Vec::new(),
            blob_locators: Vec::new(),
            tree_locators: Vec::new(),
            snapshots: Vec::new(),
            expected_snapshots: content.snapshots.clone(),
        };
        for (rows, tree) in [(&objects, false), (&trees, true)] {
            for row in rows {
                let object = json_object(row, "package pack")?;
                let id = required_json_text(object, "pack_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                let (index, mutation) =
                    self.prepare_binary_zstd_pack_from_commit(repository, object, tree, &read)?;
                if tree {
                    tree_indexes.insert(id, index);
                } else {
                    object_indexes.insert(id, index);
                }
                if let Some(mutation) = mutation {
                    if tree {
                        prepared.tree_packs.push(mutation.value);
                    } else {
                        prepared.object_packs.push(mutation.value);
                    }
                }
            }
        }
        for row in blobs {
            if let Some(mutation) = self.prepare_binary_zstd_blob_locator(
                repository,
                json_object(row, "package blob")?,
                &mut object_indexes,
                &read,
            )? {
                prepared.blob_locators.push(mutation.value);
            }
        }
        for row in tree_locators {
            if let Some(mutation) = self.prepare_binary_zstd_tree_locator(
                repository,
                json_object(row, "package tree")?,
                &mut tree_indexes,
                &read,
            )? {
                prepared.tree_locators.push(mutation.value);
            }
        }
        let mut incoming = BTreeSet::new();
        for row in snapshots {
            let object = json_object(row, "package snapshot")?;
            let id = required_json_text(object, "snapshot_id")
                .map_err(NativeRepositoryError::bad_request)?;
            let created = object
                .get("created_at")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    NativeRepositoryError::bad_request(
                        "Package Snapshot requires its original created_at",
                    )
                })?;
            let created = chrono::DateTime::parse_from_rfc3339(created)
                .map_err(|error| {
                    NativeRepositoryError::bad_request(format!(
                        "Invalid package Snapshot time: {error}"
                    ))
                })?
                .timestamp();
            if created <= 0 {
                return Err(NativeRepositoryError::bad_request(
                    "Package Snapshot requires a real original time",
                ));
            }
            if let Some(existing) = read.snapshot(&id)? {
                let hash = |value: &JsonValue| {
                    value
                        .get("manifest_hash")
                        .and_then(JsonValue::as_str)
                        .filter(|text| !text.is_empty())
                        .unwrap_or(
                            "0000000000000000000000000000000000000000000000000000000000000000",
                        )
                        .to_ascii_lowercase()
                };
                let previous = existing
                    .get("created_at")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| {
                        NativeRepositoryError::internal("Existing Snapshot has no immutable time")
                    })?;
                let previous = chrono::DateTime::parse_from_rfc3339(previous)
                    .map_err(|error| NativeRepositoryError::internal(error.to_string()))?
                    .timestamp();
                if existing.get("root_tree_pack_id") != row.get("root_tree_pack_id")
                    || existing.get("root_entry_ordinal") != row.get("root_entry_ordinal")
                    || hash(existing) != hash(row)
                    || previous != created
                    || snapshot_parent_ids(existing)? != snapshot_parent_ids(row)?
                    || existing
                        .get("message")
                        .and_then(JsonValue::as_str)
                        .map(str::to_string)
                        != optional_json_text(object, "message")
                {
                    return Err(NativeRepositoryError::conflict(format!(
                        "Package Snapshot {id} differs from immutable canonical content"
                    )));
                }
            }
            if let Some(mutation) = self.prepare_binary_zstd_snapshot(
                repository,
                object,
                &mut tree_indexes,
                &incoming,
                &read,
            )? {
                prepared
                    .snapshots
                    .push((mutation.snapshot_id, mutation.value));
            }
            incoming.insert(id);
        }
        let canonical = BinaryDbReadTxn::new(&self.db);
        let parent_count = prepared
            .snapshots
            .iter()
            .try_fold(0usize, |count, (_, value)| {
                count
                    .checked_add(snapshot_parent_ids(value)?.len())
                    .ok_or_else(|| {
                        NativeRepositoryError::bad_request("Snapshot parent count overflows")
                    })
            })?;
        for (file, added, label) in [
            (ServerBinarySnapshotCodec::<2>::record_file(), prepared.snapshots.len(), "Snapshot"),
            (crate::foundation::server_content_binary_db::ServerBinarySnapshotParentEdgeCodec::<1>::record_file(), parent_count, "Snapshot parent edge"),
        ] {
            let added = u32::try_from(added).map_err(|_| NativeRepositoryError::bad_request(format!("{label} count exceeds u32")))?;
            canonical.record_count(file).map_err(binary_native_repository_store_error)?.checked_add(added).ok_or_else(|| NativeRepositoryError::bad_request(format!("{label} physical u32 capacity exhausted")))?;
        }
        drop(canonical);
        let object_groups =
            package_pack_groups(&prepared.object_packs, &prepared.blob_locators, "pack_id")?;
        let tree_groups = package_pack_groups(
            &prepared.tree_packs,
            &prepared.tree_locators,
            "tree_pack_id",
        )?;
        self.repository_content()
            .validate_package_batches(
                &object_groups
                    .iter()
                    .map(|(p, l)| (*p, l.as_slice()))
                    .collect::<Vec<_>>(),
                &tree_groups
                    .iter()
                    .map(|(p, l)| (*p, l.as_slice()))
                    .collect::<Vec<_>>(),
                &package.inputs,
            )
            .map_err(binary_native_repository_store_error)?;
        self.validate_workflow_package_closure(repository, package)
            .map_err(NativeRepositoryError::bad_request)?;
        Ok(prepared)
    }

    pub fn stage_workflow_package_packs(
        &self,
        repository: &str,
        package: &WorkflowPackage,
    ) -> Result<(), NativeRepositoryError> {
        let content = &package.manifest.content;
        // This is the existing immutable raw-pack upload path only. No fixed
        // content/workflow records become visible before the composite commit.
        for (rows, directory, tree) in [
            (&content.object_packs, "objects", false),
            (&content.tree_packs, "trees", true),
        ] {
            for row in rows {
                let id = required_json_text(json_object(row, "package pack")?, "pack_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                let bytes =
                    std::fs::read(package.inputs.join(directory).join(&id)).map_err(|error| {
                        NativeRepositoryError::internal(format!(
                            "Read sealed package member {id}: {error}"
                        ))
                    })?;
                self.put_binary_zstd_pack(repository, &id, bytes, tree)?;
            }
        }
        Ok(())
    }

    pub fn apply_workflow_package_content_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        repository: &str,
        prepared: PreparedWorkflowPackageContent,
    ) -> Result<(), NativeRepositoryError> {
        let content = self.repository_content();
        content
            .prepare_remote_sync_write_set(
                tx,
                !prepared.object_packs.is_empty(),
                !prepared.tree_packs.is_empty(),
                !prepared.snapshots.is_empty(),
                None,
            )
            .map_err(binary_native_repository_store_error)?;
        let object_pack_groups = prepared
            .object_packs
            .iter()
            .map(|pack| {
                let id = binary_json_text(pack, "pack_id").ok_or_else(|| {
                    NativeRepositoryError::internal("Prepared package pack has no ID")
                })?;
                let locators = prepared
                    .blob_locators
                    .iter()
                    .filter(|row| row["pack_id"].as_str() == Some(&id))
                    .cloned()
                    .collect::<Vec<_>>();
                Ok((pack, locators))
            })
            .collect::<Result<Vec<_>, NativeRepositoryError>>()?;
        let object_pack_batch = object_pack_groups
            .iter()
            .map(|(pack, locators)| (*pack, locators.as_slice()))
            .collect::<Vec<_>>();
        content
            .append_object_packs_in_tx(tx, &object_pack_batch)
            .map_err(binary_native_repository_store_error)?;
        let tree_pack_groups = prepared
            .tree_packs
            .iter()
            .map(|pack| {
                let id = binary_json_text(pack, "pack_id").ok_or_else(|| {
                    NativeRepositoryError::internal("Prepared package tree pack has no ID")
                })?;
                let locators = prepared
                    .tree_locators
                    .iter()
                    .filter(|row| row["tree_pack_id"].as_str() == Some(&id))
                    .cloned()
                    .collect::<Vec<_>>();
                Ok((pack, locators))
            })
            .collect::<Result<Vec<_>, NativeRepositoryError>>()?;
        let tree_pack_batch = tree_pack_groups
            .iter()
            .map(|(pack, locators)| (*pack, locators.as_slice()))
            .collect::<Vec<_>>();
        content
            .append_tree_packs_in_tx(tx, &tree_pack_batch)
            .map_err(binary_native_repository_store_error)?;
        for (id, value) in prepared.snapshots {
            self.upsert_snapshot_value_in_tx(tx, repository, &id, &value, 0)?;
        }
        for expected in prepared.expected_snapshots {
            self.require_package_snapshot_in_write(tx, &expected)?;
        }
        self.invalidate_zstd_pull_catalog()?;
        Ok(())
    }
}

fn package_pack_groups<'a>(
    packs: &'a [JsonValue],
    locators: &[JsonValue],
    field: &str,
) -> Result<Vec<(&'a JsonValue, Vec<JsonValue>)>, NativeRepositoryError> {
    packs
        .iter()
        .map(|pack| {
            let id = binary_json_text(pack, "pack_id")
                .ok_or_else(|| NativeRepositoryError::internal("Prepared pack ID missing"))?;
            Ok((
                pack,
                locators
                    .iter()
                    .filter(|row| row[field].as_str() == Some(&id))
                    .cloned()
                    .collect(),
            ))
        })
        .collect()
}
