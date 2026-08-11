use super::*;

#[allow(dead_code)]
pub struct BinaryDbRemoteSyncZstdImportTransactionStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    snapshots: BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
    blobs: Option<BinaryDbBlobStore<B, WRITE_LAYOUT>>,
    object_packs: Option<BinaryDbObjectPackStore<B, WRITE_LAYOUT>>,
    tree_packs: Option<BinaryDbTreePackStore<B, WRITE_LAYOUT>>,
    trees: Option<BinaryDbTreeStore<B, WRITE_LAYOUT>>,
}

#[allow(dead_code)]
impl<B, const WRITE_LAYOUT: u32> BinaryDbRemoteSyncZstdImportTransactionStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn new(snapshots: BinaryDbSnapshotStore<B, WRITE_LAYOUT>) -> Self {
        Self {
            snapshots,
            blobs: None,
            object_packs: None,
            tree_packs: None,
            trees: None,
        }
    }

    pub fn with_content_stores(
        blobs: BinaryDbBlobStore<B, WRITE_LAYOUT>,
        object_packs: BinaryDbObjectPackStore<B, WRITE_LAYOUT>,
        tree_packs: BinaryDbTreePackStore<B, WRITE_LAYOUT>,
        trees: BinaryDbTreeStore<B, WRITE_LAYOUT>,
        snapshots: BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
    ) -> Self {
        Self {
            snapshots,
            blobs: Some(blobs),
            object_packs: Some(object_packs),
            tree_packs: Some(tree_packs),
            trees: Some(trees),
        }
    }
}

#[allow(dead_code)]
pub struct BinaryDbRemoteSyncZstdImportStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb + Clone,
{
    transaction: BinaryDbRemoteSyncZstdImportTransactionStore<B, WRITE_LAYOUT>,
    object_packs: BinaryDbObjectPackStore<B, WRITE_LAYOUT>,
    tree_packs: BinaryDbTreePackStore<B, WRITE_LAYOUT>,
}

#[allow(dead_code)]
impl<B, const WRITE_LAYOUT: u32> BinaryDbRemoteSyncZstdImportStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + Clone,
{
    pub fn with_content_stores(
        blobs: BinaryDbBlobStore<B, WRITE_LAYOUT>,
        object_packs: BinaryDbObjectPackStore<B, WRITE_LAYOUT>,
        tree_packs: BinaryDbTreePackStore<B, WRITE_LAYOUT>,
        trees: BinaryDbTreeStore<B, WRITE_LAYOUT>,
        snapshots: BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
    ) -> Self {
        let transaction = BinaryDbRemoteSyncZstdImportTransactionStore::with_content_stores(
            blobs,
            object_packs.clone(),
            tree_packs.clone(),
            trees,
            snapshots,
        );
        Self {
            transaction,
            object_packs,
            tree_packs,
        }
    }
}

impl<B, const WRITE_LAYOUT: u32> RemoteSyncZstdImportTransactionStore
    for BinaryDbRemoteSyncZstdImportTransactionStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    fn zstd_import_snapshot_exists(
        &self,
        _ctx: &RemoteSyncLocalStoreContext,
        snapshot_id: &str,
    ) -> Result<bool, String> {
        self.snapshots.snapshot_exists(snapshot_id)
    }

    fn commit_zstd_import_metadata(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        manifest: &ZstdImportManifestPayload,
        history_mode: ZstdImportHistoryMode,
    ) -> Result<ZstdImportMetadataCommitResult, String> {
        let blobs = self.blobs.as_ref().ok_or_else(|| {
            "BinaryDbRemoteSyncZstdImportTransactionStore::commit_zstd_import_metadata requires Binary DB blob store wiring".to_string()
        })?;
        let object_packs = self.object_packs.as_ref().ok_or_else(|| {
            "BinaryDbRemoteSyncZstdImportTransactionStore::commit_zstd_import_metadata requires Binary DB object-pack store wiring".to_string()
        })?;
        let tree_packs = self.tree_packs.as_ref().ok_or_else(|| {
            "BinaryDbRemoteSyncZstdImportTransactionStore::commit_zstd_import_metadata requires Binary DB tree-pack store wiring".to_string()
        })?;
        let trees = self.trees.as_ref().ok_or_else(|| {
            "BinaryDbRemoteSyncZstdImportTransactionStore::commit_zstd_import_metadata requires Binary DB tree store wiring".to_string()
        })?;
        let coordinator = BinaryDbContentWriteCoordinator::new(
            blobs,
            object_packs,
            tree_packs,
            trees,
            &self.snapshots,
        );
        let snapshot = manifest
            .snapshots
            .first()
            .ok_or_else(|| "Zstd import manifest is missing snapshot row.".to_string())?;
        let mut snapshot_input = binary_db_snapshot_write_input(snapshot)?;

        let mut pending_object_packs = BTreeMap::new();
        for pack in &manifest.object_packs {
            let read = object_packs.begin_read_txn();
            let already_recorded = object_packs
                .get_object_pack_view(&read, &pack.pack_id)?
                .is_some();
            drop(read);
            if already_recorded {
                continue;
            }
            let input = binary_db_object_pack_write_input_from_index(ctx, manifest, pack)?;
            if pending_object_packs
                .insert(pack.pack_id.clone(), input)
                .is_some()
            {
                return Err(format!(
                    "Zstd import manifest contains duplicate object pack {}.",
                    pack.pack_id
                ));
            }
        }
        let selected_object_pack_by_blob_id = manifest
            .blob_locators
            .iter()
            .map(|locator| {
                let pack_id = locator.pack_id.as_ref().ok_or_else(|| {
                    format!(
                        "Remote-head Blob locator {} is missing its selected object pack.",
                        locator.blob_id
                    )
                })?;
                Ok((locator.blob_id.to_ascii_lowercase(), pack_id.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let object_pack_order = object_pack_dependency_order(
            blobs,
            &pending_object_packs,
            &selected_object_pack_by_blob_id,
        )?;
        let mut ordered_object_packs = Vec::with_capacity(object_pack_order.len());
        for pack_id in object_pack_order {
            let input = pending_object_packs.remove(&pack_id).ok_or_else(|| {
                format!("Object-pack dependency order returned unknown pack {pack_id}.")
            })?;
            ordered_object_packs.push(input);
        }
        coordinator.record_object_pack_metadata_batch(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &ordered_object_packs,
        )?;
        let selected_tree_pack_by_tree_id = manifest
            .tree_locators
            .iter()
            .map(|locator| {
                let pack_id = locator.tree_pack_id.as_ref().ok_or_else(|| {
                    format!(
                        "Remote-head Tree locator {} is missing its selected tree pack.",
                        locator.tree_id
                    )
                })?;
                Ok((locator.tree_id.to_ascii_lowercase(), pack_id.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let mut all_tree_packs = BTreeMap::new();
        for pack in &manifest.tree_packs {
            let (pack_input, tree_entries) = binary_db_tree_pack_write_inputs(ctx, manifest, pack)?;
            if all_tree_packs
                .insert(pack.pack_id.clone(), (pack_input, tree_entries))
                .is_some()
            {
                return Err(format!(
                    "Zstd import manifest contains duplicate tree pack {}.",
                    pack.pack_id
                ));
            }
        }
        let (reachable_tree_ids, boundary_root_tree_id) = match history_mode {
            ZstdImportHistoryMode::CompleteAncestry => (None, None),
            ZstdImportHistoryMode::RemoteHeadBoundary => {
                let (reachable, root_tree_id) = remote_head_reachable_tree_ids(
                    blobs,
                    trees,
                    &all_tree_packs,
                    &snapshot_input.root_tree_pack_id,
                    snapshot_input.root_entry_ordinal,
                    &selected_tree_pack_by_tree_id,
                )?;
                (Some(reachable), Some(root_tree_id))
            }
        };
        let mut pending_tree_packs = BTreeMap::new();
        for (pack_id, input) in all_tree_packs {
            let read = tree_packs.begin_read_txn();
            let existing_pack = tree_packs.get_tree_pack_view(&read, &pack_id)?;
            drop(read);
            if let Some(existing_pack) = existing_pack {
                if history_mode == ZstdImportHistoryMode::RemoteHeadBoundary
                    && i64::from(existing_pack.record.tree_count) != input.0.tree_count
                {
                    coordinator.mark_tree_pack_sparse_physical_ordinals(
                        BinaryDbCommandScope::RemoteSyncLocalImport,
                        &pack_id,
                    )?;
                }
            } else {
                pending_tree_packs.insert(pack_id, input);
            }
        }
        let tree_pack_order = tree_pack_dependency_order(
            trees,
            &pending_tree_packs,
            reachable_tree_ids.as_ref(),
            &selected_tree_pack_by_tree_id,
        )?;
        let mut ordered_tree_packs = Vec::with_capacity(tree_pack_order.len());
        for pack_id in tree_pack_order {
            ordered_tree_packs.push(pending_tree_packs.remove(&pack_id).ok_or_else(|| {
                format!("Tree-pack dependency order returned unknown pack {pack_id}.")
            })?);
        }
        match reachable_tree_ids.as_ref() {
            Some(reachable) => {
                for (pack_input, tree_entries) in ordered_tree_packs {
                    coordinator.record_tree_pack_metadata_with_reachable_entries(
                        BinaryDbCommandScope::RemoteSyncLocalImport,
                        &pack_input,
                        &tree_entries,
                        reachable,
                    )?;
                }
            }
            None => coordinator.record_tree_pack_metadata_batch_with_entries(
                BinaryDbCommandScope::RemoteSyncLocalImport,
                &ordered_tree_packs,
            )?,
        }
        if let Some(root_tree_id) = boundary_root_tree_id {
            let read = trees.begin_read_txn();
            let root_tree = trees
                .get_tree_view(&read, &root_tree_id)?
                .ok_or_else(|| {
                    format!(
                        "Remote-head root tree {root_tree_id} is missing after verified tree-pack import."
                    )
                })?;
            let root_tree_pack_id = root_tree.tree_pack_id.clone().ok_or_else(|| {
                format!(
                    "Remote-head root tree {root_tree_id} has no Binary DB tree-pack owner after import."
                )
            })?;
            let root_tree_pack = tree_packs
                .get_tree_pack_view(&read, &root_tree_pack_id)?
                .ok_or_else(|| {
                    format!(
                        "Remote-head root tree pack {root_tree_pack_id} is missing after import."
                    )
                })?;
            let root_local_ordinal = root_tree
                .tree_index
                .checked_sub(root_tree_pack.record.first_tree_index)
                .ok_or_else(|| {
                    format!(
                        "Remote-head root tree {root_tree_id} precedes its local tree-pack range."
                    )
                })?;
            snapshot_input.root_tree_pack_id = root_tree_pack_id;
            snapshot_input.root_entry_ordinal = i64::from(root_local_ordinal);
            drop(read);
        }
        let imported_snapshot = match history_mode {
            ZstdImportHistoryMode::CompleteAncestry => coordinator
                .record_snapshot(BinaryDbCommandScope::RemoteSyncLocalImport, &snapshot_input)?,
            ZstdImportHistoryMode::RemoteHeadBoundary => coordinator
                .record_snapshot_at_remote_head_history_boundary(
                    BinaryDbCommandScope::RemoteSyncLocalImport,
                    &snapshot_input,
                )?,
        };
        Ok(ZstdImportMetadataCommitResult {
            imported_snapshot,
            upserted_blob_locators: manifest.blob_locators.len() as i64,
            upserted_tree_locators: manifest.tree_locators.len() as i64,
        })
    }
}

fn binary_db_snapshot_write_input(
    snapshot: &ZstdBulkSnapshotRow,
) -> Result<BinaryDbSnapshotWriteInput, String> {
    validate_zstd_import_manifest_snapshot_row(snapshot)?;
    Ok(BinaryDbSnapshotWriteInput {
        snapshot_id: snapshot.snapshot_id.clone(),
        parent_snapshot_ids: snapshot.parent_snapshot_ids.clone(),
        root_tree_pack_id: snapshot.root_tree_pack_id.clone().ok_or_else(|| {
            format!(
                "Snapshot {} is missing root_tree_pack_id.",
                snapshot.snapshot_id
            )
        })?,
        root_entry_ordinal: snapshot.root_entry_ordinal.ok_or_else(|| {
            format!(
                "Snapshot {} is missing root_entry_ordinal.",
                snapshot.snapshot_id
            )
        })?,
        manifest_hash: snapshot.manifest_hash.clone().ok_or_else(|| {
            format!(
                "Snapshot {} is missing manifest_hash.",
                snapshot.snapshot_id
            )
        })?,
        message: snapshot.message.clone(),
        line_name: snapshot.line_name.clone().unwrap_or_default(),
        snapshot_kind: snapshot
            .snapshot_kind
            .clone()
            .unwrap_or_else(|| "line".to_string()),
        file_count: snapshot.file_count.unwrap_or(0),
        total_bytes: snapshot.total_bytes.unwrap_or(0),
        created_at: snapshot.created_at.clone().unwrap_or_default(),
    })
}

fn object_pack_dependency_order<B, const WRITE_LAYOUT: u32>(
    blobs: &BinaryDbBlobStore<B, WRITE_LAYOUT>,
    inputs: &BTreeMap<String, BinaryDbObjectPackWriteInput>,
    selected_pack_by_blob_id: &BTreeMap<String, String>,
) -> Result<Vec<String>, String>
where
    B: BinaryDb,
{
    let (owners_by_blob_id, members_by_pack, mut dependencies) =
        object_pack_dependency_index(inputs, selected_pack_by_blob_id)?;

    let read = blobs.begin_read_txn();
    for (blob_id, owners) in &owners_by_blob_id {
        let Some(selected_pack_id) = selected_pack_by_blob_id.get(blob_id) else {
            return Err(format!(
                "Remote-head object-pack closure has no selected locator for repeated blob {blob_id}."
            ));
        };
        if owners.len() > 1
            && !inputs.contains_key(selected_pack_id)
            && blobs.get_blob_view(&read, blob_id)?.is_none()
        {
            return Err(format!(
                "Remote-head selected object pack {selected_pack_id} for repeated blob {blob_id} is neither pending nor present in the local Binary DB."
            ));
        }
    }

    for (pack_id, input) in inputs {
        let own_members = members_by_pack
            .get(pack_id)
            .ok_or_else(|| format!("Missing object-pack member index for {pack_id}."))?;
        let pack_dependencies = dependencies
            .get_mut(pack_id)
            .ok_or_else(|| format!("Missing object-pack dependency index for {pack_id}."))?;
        for member in &input.members {
            let Some(base_blob_id) = member
                .pack_base_blob_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let normalized_base = base_blob_id.to_ascii_lowercase();
            if own_members.contains(&normalized_base)
                || blobs.get_blob_view(&read, base_blob_id)?.is_some()
            {
                continue;
            }
            let owners = owners_by_blob_id.get(&normalized_base).ok_or_else(|| {
                format!(
                    "Remote-head object-pack closure is incomplete: pack {pack_id} requires base blob {base_blob_id}, but no downloaded pack or existing Binary DB record owns it."
                )
            })?;
            let owner = selected_pack_by_blob_id
                .get(&normalized_base)
                .filter(|selected| owners.contains(*selected))
                .or_else(|| owners.iter().next())
                .ok_or_else(|| {
                    format!(
                        "Remote-head object-pack closure has no pending owner for base blob {base_blob_id}."
                    )
                })?;
            if owner != pack_id {
                pack_dependencies.insert(owner.clone());
            }
        }
    }
    drop(read);
    dependency_order("object-pack", inputs.len(), &dependencies)
}

type ObjectPackOwners = BTreeMap<String, BTreeSet<String>>;
type ObjectPackMembers = BTreeMap<String, BTreeSet<String>>;
type ObjectPackDependencies = BTreeMap<String, BTreeSet<String>>;

fn object_pack_dependency_index(
    inputs: &BTreeMap<String, BinaryDbObjectPackWriteInput>,
    selected_pack_by_blob_id: &BTreeMap<String, String>,
) -> Result<(ObjectPackOwners, ObjectPackMembers, ObjectPackDependencies), String> {
    let mut owners_by_blob_id = ObjectPackOwners::new();
    let mut members_by_pack = BTreeMap::new();
    for (pack_id, input) in inputs {
        let mut member_ids = BTreeSet::new();
        for member in &input.members {
            let blob_id = member.blob_id.to_ascii_lowercase();
            if !member_ids.insert(blob_id.clone()) {
                return Err(format!(
                    "Object pack {pack_id} contains duplicate blob {}.",
                    member.blob_id
                ));
            }
            owners_by_blob_id
                .entry(blob_id)
                .or_default()
                .insert(pack_id.clone());
        }
        members_by_pack.insert(pack_id.clone(), member_ids);
    }

    let mut dependencies = inputs
        .keys()
        .map(|pack_id| (pack_id.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for (blob_id, owners) in &owners_by_blob_id {
        if owners.len() < 2 {
            continue;
        }
        let selected_pack_id = selected_pack_by_blob_id.get(blob_id).ok_or_else(|| {
            format!(
                "Remote-head object-pack closure has no selected locator for repeated blob {blob_id}."
            )
        })?;
        if !owners.contains(selected_pack_id) {
            continue;
        }
        for owner in owners.iter().filter(|owner| *owner != selected_pack_id) {
            dependencies
                .get_mut(owner)
                .ok_or_else(|| format!("Missing object-pack dependency index for {owner}."))?
                .insert(selected_pack_id.clone());
        }
    }
    Ok((owners_by_blob_id, members_by_pack, dependencies))
}

fn remote_head_reachable_tree_ids<B, const WRITE_LAYOUT: u32>(
    blobs: &BinaryDbBlobStore<B, WRITE_LAYOUT>,
    trees: &BinaryDbTreeStore<B, WRITE_LAYOUT>,
    inputs: &BTreeMap<String, (BinaryDbTreePackWriteInput, Vec<BinaryDbTreeEntryWriteInput>)>,
    root_tree_pack_id: &str,
    root_entry_ordinal: i64,
    selected_pack_by_tree_id: &BTreeMap<String, String>,
) -> Result<(BTreeSet<String>, String), String>
where
    B: BinaryDb,
{
    let root_ordinal = usize::try_from(root_entry_ordinal)
        .map_err(|_| format!("Remote-head root tree ordinal is invalid: {root_entry_ordinal}."))?;
    let (root_pack, _) = inputs.get(root_tree_pack_id).ok_or_else(|| {
        format!("Remote-head root tree pack {root_tree_pack_id} is missing from the manifest.")
    })?;
    let root_tree_id = root_pack
        .trees
        .get(root_ordinal)
        .map(|tree| tree.tree_id.to_ascii_lowercase())
        .ok_or_else(|| {
            format!(
                "Remote-head root tree pack {root_tree_pack_id} is missing ordinal {root_entry_ordinal}."
            )
        })?;

    let mut owners_by_tree_id = BTreeMap::<String, BTreeSet<String>>::new();
    let mut entries_by_tree_id = BTreeMap::<String, Vec<&BinaryDbTreeEntryWriteInput>>::new();
    for (pack_id, (input, _)) in inputs {
        for tree in &input.trees {
            let tree_id = tree.tree_id.to_ascii_lowercase();
            owners_by_tree_id
                .entry(tree_id.clone())
                .or_default()
                .insert(pack_id.clone());
            // A verified empty tree has no tree-entry rows. Keep an explicit
            // empty adjacency list so it remains a valid reachable leaf.
            entries_by_tree_id.entry(tree_id).or_default();
        }
    }
    let mut owner_by_tree_id = BTreeMap::new();
    for (tree_id, owners) in &owners_by_tree_id {
        let selected_pack_id = selected_pack_by_tree_id.get(tree_id).ok_or_else(|| {
            format!(
                "Remote-head tree-pack closure has no selected locator for physical tree {tree_id}."
            )
        })?;
        if !owners.contains(selected_pack_id) {
            return Err(format!(
                "Remote-head selected tree pack {selected_pack_id} does not contain physical tree {tree_id}."
            ));
        }
        owner_by_tree_id.insert(tree_id.clone(), selected_pack_id.as_str());
    }
    for (pack_id, (_, entries)) in inputs {
        for entry in entries {
            let tree_id = entry.tree_id.to_ascii_lowercase();
            if selected_pack_by_tree_id.get(&tree_id) == Some(pack_id) {
                entries_by_tree_id.entry(tree_id).or_default().push(entry);
            }
        }
    }

    let read = trees.begin_read_txn();
    let mut reachable = BTreeSet::from([root_tree_id.clone()]);
    let mut pending = vec![root_tree_id.clone()];
    let mut cursor = 0_usize;
    while cursor < pending.len() {
        let tree_id = pending[cursor].clone();
        cursor += 1;
        let entries = entries_by_tree_id.get(&tree_id).ok_or_else(|| {
            format!("Remote-head reachable tree {tree_id} has no verified pack entries.")
        })?;
        for entry in entries {
            match entry.entry_type.as_str() {
                "tree" => {
                    let target_id = entry.target_id.to_ascii_lowercase();
                    if owner_by_tree_id.contains_key(&target_id) {
                        if reachable.insert(target_id.clone()) {
                            pending.push(target_id);
                        }
                    } else if trees.get_tree_view(&read, &entry.target_id)?.is_none() {
                        return Err(format!(
                            "Remote-head reachable tree {tree_id} entry {} requires tree {}, but no downloaded pack or existing Binary DB record owns it.",
                            entry.entry_name, entry.target_id
                        ));
                    }
                }
                "blob" => {
                    if blobs.get_blob_view(&read, &entry.target_id)?.is_none() {
                        return Err(format!(
                            "Remote-head reachable tree {tree_id} entry {} requires blob {}, but no downloaded pack or existing Binary DB record owns it.",
                            entry.entry_name, entry.target_id
                        ));
                    }
                }
                other => {
                    return Err(format!(
                        "Remote-head reachable tree {tree_id} entry {} has unsupported kind {other}.",
                        entry.entry_name
                    ));
                }
            }
        }
    }
    drop(read);
    Ok((reachable, root_tree_id))
}

fn tree_pack_dependency_order<B, const WRITE_LAYOUT: u32>(
    trees: &BinaryDbTreeStore<B, WRITE_LAYOUT>,
    inputs: &BTreeMap<String, (BinaryDbTreePackWriteInput, Vec<BinaryDbTreeEntryWriteInput>)>,
    reachable_tree_ids: Option<&BTreeSet<String>>,
    selected_pack_by_tree_id: &BTreeMap<String, String>,
) -> Result<Vec<String>, String>
where
    B: BinaryDb,
{
    let (owners_by_tree_id, trees_by_pack, mut dependencies) =
        tree_pack_dependency_index(inputs, selected_pack_by_tree_id)?;

    let read = trees.begin_read_txn();
    for (tree_id, owners) in &owners_by_tree_id {
        let Some(selected_pack_id) = selected_pack_by_tree_id.get(tree_id) else {
            return Err(format!(
                "Remote-head tree-pack closure has no selected locator for repeated tree {tree_id}."
            ));
        };
        if owners.len() > 1
            && !inputs.contains_key(selected_pack_id)
            && trees.get_tree_view(&read, tree_id)?.is_none()
        {
            return Err(format!(
                "Remote-head selected tree pack {selected_pack_id} for repeated tree {tree_id} is neither pending nor present in the local Binary DB."
            ));
        }
    }
    for (pack_id, (_, entries)) in inputs {
        let own_trees = trees_by_pack
            .get(pack_id)
            .ok_or_else(|| format!("Missing tree-pack tree index for {pack_id}."))?;
        let pack_dependencies = dependencies
            .get_mut(pack_id)
            .ok_or_else(|| format!("Missing tree-pack dependency index for {pack_id}."))?;
        for entry in entries.iter().filter(|entry| {
            entry.entry_type == "tree"
                && reachable_tree_ids
                    .is_none_or(|reachable| reachable.contains(&entry.tree_id.to_ascii_lowercase()))
        }) {
            let normalized_target = entry.target_id.to_ascii_lowercase();
            if own_trees.contains(&normalized_target)
                || trees.get_tree_view(&read, &entry.target_id)?.is_some()
            {
                continue;
            }
            let owners = owners_by_tree_id.get(&normalized_target).ok_or_else(|| {
                format!(
                    "Remote-head tree-pack closure is incomplete: pack {pack_id} entry {} requires tree {}, but no downloaded pack or existing Binary DB record owns it.",
                    entry.entry_name, entry.target_id
                )
            })?;
            let owner = selected_pack_by_tree_id
                .get(&normalized_target)
                .filter(|selected| owners.contains(*selected))
                .or_else(|| owners.iter().next())
                .ok_or_else(|| {
                    format!(
                        "Remote-head tree-pack closure has no pending owner for tree {}.",
                        entry.target_id
                    )
                })?;
            if owner != pack_id {
                pack_dependencies.insert(owner.clone());
            }
        }
    }
    drop(read);
    dependency_order("tree-pack", inputs.len(), &dependencies)
}

type TreePackOwners = BTreeMap<String, BTreeSet<String>>;
type TreePackMembers = BTreeMap<String, BTreeSet<String>>;
type TreePackDependencies = BTreeMap<String, BTreeSet<String>>;

fn tree_pack_dependency_index(
    inputs: &BTreeMap<String, (BinaryDbTreePackWriteInput, Vec<BinaryDbTreeEntryWriteInput>)>,
    selected_pack_by_tree_id: &BTreeMap<String, String>,
) -> Result<(TreePackOwners, TreePackMembers, TreePackDependencies), String> {
    let mut owners_by_tree_id = TreePackOwners::new();
    let mut trees_by_pack = TreePackMembers::new();
    for (pack_id, (input, _)) in inputs {
        let mut tree_ids = BTreeSet::new();
        for tree in &input.trees {
            let tree_id = tree.tree_id.to_ascii_lowercase();
            if !tree_ids.insert(tree_id.clone()) {
                return Err(format!(
                    "Tree pack {pack_id} contains duplicate tree {}.",
                    tree.tree_id
                ));
            }
            owners_by_tree_id
                .entry(tree_id)
                .or_default()
                .insert(pack_id.clone());
        }
        trees_by_pack.insert(pack_id.clone(), tree_ids);
    }

    let mut dependencies = inputs
        .keys()
        .map(|pack_id| (pack_id.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for (tree_id, owners) in &owners_by_tree_id {
        if owners.len() < 2 {
            continue;
        }
        let selected_pack_id = selected_pack_by_tree_id.get(tree_id).ok_or_else(|| {
            format!(
                "Remote-head tree-pack closure has no selected locator for repeated tree {tree_id}."
            )
        })?;
        if !owners.contains(selected_pack_id) {
            continue;
        }
        for owner in owners.iter().filter(|owner| *owner != selected_pack_id) {
            dependencies
                .get_mut(owner)
                .ok_or_else(|| format!("Missing tree-pack dependency index for {owner}."))?
                .insert(selected_pack_id.clone());
        }
    }
    Ok((owners_by_tree_id, trees_by_pack, dependencies))
}

fn dependency_order(
    family: &str,
    item_count: usize,
    dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let mut ordered = Vec::with_capacity(item_count);
    let mut resolved = BTreeSet::new();
    while ordered.len() < item_count {
        let next = dependencies
            .iter()
            .filter(|(item_id, _)| !resolved.contains(*item_id))
            .find(|(_, required)| required.iter().all(|item_id| resolved.contains(item_id)))
            .map(|(item_id, _)| item_id.clone());
        let Some(item_id) = next else {
            let blocked = dependencies
                .iter()
                .filter(|(item_id, _)| !resolved.contains(*item_id))
                .map(|(item_id, required)| {
                    let unresolved = required
                        .iter()
                        .filter(|dependency| !resolved.contains(*dependency))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("{item_id}<-[{unresolved}]")
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!(
                "Remote-head {family} dependency cycle prevents import: {blocked}."
            ));
        };
        resolved.insert(item_id.clone());
        ordered.push(item_id);
    }
    Ok(ordered)
}

impl<B, const WRITE_LAYOUT: u32> RemoteSyncZstdImportSource
    for BinaryDbRemoteSyncZstdImportStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender + Clone,
{
    fn zstd_import_download_plan(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        manifest: &ZstdImportManifestPayload,
    ) -> Result<ZstdImportDownloadPlan, String> {
        ZstdImportManifestJson::stateless().validate_domain(manifest)?;
        let mut plan = ZstdImportDownloadPlan::default();
        for pack in &manifest.object_packs {
            if let Some(stamp) = local_object_pack_validation_stamp(ctx, &self.object_packs, pack)?
            {
                plan.reusable_object_pack_ids.push(pack.pack_id.clone());
                plan.reusable_object_pack_stamps
                    .insert(pack.pack_id.clone(), stamp);
            } else {
                plan.missing_object_pack_ids.push(pack.pack_id.clone());
            }
        }
        for pack in &manifest.tree_packs {
            if let Some(stamp) = local_tree_pack_validation_stamp(ctx, &self.tree_packs, pack)? {
                plan.reusable_tree_pack_ids.push(pack.pack_id.clone());
                plan.reusable_tree_pack_stamps
                    .insert(pack.pack_id.clone(), stamp);
            } else {
                plan.missing_tree_pack_ids.push(pack.pack_id.clone());
            }
        }
        Ok(plan)
    }

    fn import_zstd_manifest(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        manifest: &ZstdImportManifestPayload,
        history_mode: ZstdImportHistoryMode,
        plan: &ZstdImportDownloadPlan,
        object_pack_bytes: &BTreeMap<String, Vec<u8>>,
        tree_pack_bytes: &BTreeMap<String, Vec<u8>>,
    ) -> Result<ZstdImportApplyResult, String> {
        ZstdImportManifestJson::stateless().validate_domain(manifest)?;
        let snapshot_id = manifest.snapshot_id.clone();

        let mut downloaded_object_packs = 0_i64;
        let mut reused_object_packs = 0_i64;
        for pack in &manifest.object_packs {
            let verified_unchanged = plan
                .reusable_object_pack_stamps
                .get(&pack.pack_id)
                .map(|stamp| object_pack_validation_stamp_is_current(ctx, pack, stamp))
                .transpose()?
                .unwrap_or(false);
            if verified_unchanged
                || local_object_pack_matches_manifest(ctx, &self.object_packs, pack)?
            {
                reused_object_packs += 1;
            } else {
                let bytes = object_pack_bytes.get(&pack.pack_id).ok_or_else(|| {
                    format!(
                        "Zstd import manifest requires object pack {}, but downloaded bytes were not provided.",
                        pack.pack_id
                    )
                })?;
                write_downloaded_object_pack(ctx, pack, bytes)?;
                downloaded_object_packs += 1;
            }
        }

        let mut downloaded_tree_packs = 0_i64;
        let mut reused_tree_packs = 0_i64;
        for pack in &manifest.tree_packs {
            let verified_unchanged = plan
                .reusable_tree_pack_stamps
                .get(&pack.pack_id)
                .map(|stamp| tree_pack_validation_stamp_is_current(ctx, pack, stamp))
                .transpose()?
                .unwrap_or(false);
            if verified_unchanged || local_tree_pack_matches_manifest(ctx, &self.tree_packs, pack)?
            {
                reused_tree_packs += 1;
            } else {
                let bytes = tree_pack_bytes.get(&pack.pack_id).ok_or_else(|| {
                    format!(
                        "Zstd import manifest requires tree pack {}, but downloaded bytes were not provided.",
                        pack.pack_id
                    )
                })?;
                write_downloaded_tree_pack(ctx, pack, bytes)?;
                downloaded_tree_packs += 1;
            }
        }

        validate_manifest_locators_against_pack_indexes(ctx, manifest)?;
        let commit = commit_zstd_import_metadata_with_remote_sync_zstd_import_transaction_store(
            &self.transaction,
            ctx,
            manifest,
            history_mode,
        )?;

        Ok(ZstdImportApplyResult {
            snapshot_id,
            imported_snapshot: commit.imported_snapshot,
            downloaded_object_packs,
            reused_object_packs,
            downloaded_tree_packs,
            reused_tree_packs,
            upserted_blob_locators: commit.upserted_blob_locators,
            upserted_tree_locators: commit.upserted_tree_locators,
        })
    }

    fn stage_zstd_import_pack_batch(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        object_packs: &[ZstdBulkObjectPackRow],
        tree_packs: &[ZstdBulkTreePackRow],
        object_pack_bytes: &BTreeMap<String, Vec<u8>>,
        tree_pack_bytes: &BTreeMap<String, Vec<u8>>,
    ) -> Result<ZstdImportPackStageResult, String> {
        let expected_object_pack_ids = object_packs
            .iter()
            .map(|pack| pack.pack_id.as_str())
            .collect::<BTreeSet<_>>();
        let expected_tree_pack_ids = tree_packs
            .iter()
            .map(|pack| pack.pack_id.as_str())
            .collect::<BTreeSet<_>>();
        if object_pack_bytes
            .keys()
            .any(|pack_id| !expected_object_pack_ids.contains(pack_id.as_str()))
        {
            return Err(
                "Zstd import pack batch contains unrequested object-pack bytes.".to_string(),
            );
        }
        if tree_pack_bytes
            .keys()
            .any(|pack_id| !expected_tree_pack_ids.contains(pack_id.as_str()))
        {
            return Err("Zstd import pack batch contains unrequested tree-pack bytes.".to_string());
        }

        let mut result = ZstdImportPackStageResult::default();
        for pack in object_packs {
            validate_zstd_import_manifest_object_pack_row(pack)?;
            if local_object_pack_matches_manifest(ctx, &self.object_packs, pack)? {
                result.reused_object_packs += 1;
                continue;
            }
            let bytes = object_pack_bytes.get(&pack.pack_id).ok_or_else(|| {
                format!(
                    "Zstd import pack batch requires object pack {}, but downloaded bytes were not provided.",
                    pack.pack_id
                )
            })?;
            write_downloaded_object_pack(ctx, pack, bytes)?;
            result.downloaded_object_packs += 1;
        }
        for pack in tree_packs {
            validate_zstd_import_manifest_tree_pack_row(pack)?;
            if local_tree_pack_matches_manifest(ctx, &self.tree_packs, pack)? {
                result.reused_tree_packs += 1;
                continue;
            }
            let bytes = tree_pack_bytes.get(&pack.pack_id).ok_or_else(|| {
                format!(
                    "Zstd import pack batch requires tree pack {}, but downloaded bytes were not provided.",
                    pack.pack_id
                )
            })?;
            write_downloaded_tree_pack(ctx, pack, bytes)?;
            result.downloaded_tree_packs += 1;
        }
        Ok(result)
    }

    fn import_zstd_snapshot_rows(
        &self,
        _ctx: &RemoteSyncLocalStoreContext,
        snapshots: &[ZstdBulkSnapshotRow],
    ) -> Result<Vec<String>, String> {
        let blobs = self.transaction.blobs.as_ref().ok_or_else(|| {
            "BinaryDbRemoteSyncZstdImportStore::import_zstd_snapshot_rows requires Binary DB blob store wiring".to_string()
        })?;
        let object_packs = self.transaction.object_packs.as_ref().ok_or_else(|| {
            "BinaryDbRemoteSyncZstdImportStore::import_zstd_snapshot_rows requires Binary DB object-pack store wiring".to_string()
        })?;
        let tree_packs = self.transaction.tree_packs.as_ref().ok_or_else(|| {
            "BinaryDbRemoteSyncZstdImportStore::import_zstd_snapshot_rows requires Binary DB tree-pack store wiring".to_string()
        })?;
        let trees = self.transaction.trees.as_ref().ok_or_else(|| {
            "BinaryDbRemoteSyncZstdImportStore::import_zstd_snapshot_rows requires Binary DB tree store wiring".to_string()
        })?;
        let coordinator = BinaryDbContentWriteCoordinator::new(
            blobs,
            object_packs,
            tree_packs,
            trees,
            &self.transaction.snapshots,
        );
        let inputs = snapshots
            .iter()
            .map(binary_db_snapshot_write_input)
            .collect::<Result<Vec<_>, _>>()?;
        let imported =
            coordinator.record_snapshots(BinaryDbCommandScope::RemoteSyncLocalImport, &inputs)?;
        let imported_snapshot_ids = snapshots
            .iter()
            .zip(imported)
            .filter(|(_, wrote)| *wrote)
            .map(|(snapshot, _)| snapshot.snapshot_id.clone())
            .collect();
        Ok(imported_snapshot_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_pack_member(blob_id: &str) -> BinaryDbObjectPackMemberWriteInput {
        BinaryDbObjectPackMemberWriteInput {
            blob_id: blob_id.to_string(),
            sha256: "00".repeat(32),
            size_bytes: 1,
            pack_entry_type: "full".to_string(),
            pack_base_blob_id: None,
            pack_chain_depth: 0,
            created_at: "2026-08-04T00:00:00Z".to_string(),
        }
    }

    fn object_pack_input(
        pack_id: &str,
        members: Vec<BinaryDbObjectPackMemberWriteInput>,
    ) -> BinaryDbObjectPackWriteInput {
        BinaryDbObjectPackWriteInput {
            pack_id: pack_id.to_string(),
            pack_rel_path: format!(".ait/objects/packs/{pack_id}.zstpack"),
            pack_format: PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
            member_count: members.len() as i64,
            total_bytes: 1,
            created_at: "2026-08-04T00:00:00Z".to_string(),
            members,
        }
    }

    fn tree_pack_tree(tree_id: &str) -> BinaryDbTreePackTreeWriteInput {
        BinaryDbTreePackTreeWriteInput {
            tree_id: tree_id.to_string(),
            entry_count: 0,
        }
    }

    fn tree_pack_input(
        pack_id: &str,
        trees: Vec<BinaryDbTreePackTreeWriteInput>,
    ) -> (BinaryDbTreePackWriteInput, Vec<BinaryDbTreeEntryWriteInput>) {
        (
            BinaryDbTreePackWriteInput {
                pack_id: pack_id.to_string(),
                pack_rel_path: format!(".ait/objects/tree-packs/{pack_id}.zstpack"),
                pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: trees.len() as i64,
                total_bytes: 1,
                created_at: "2026-08-04T00:00:00Z".to_string(),
                trees,
            },
            Vec::new(),
        )
    }

    #[test]
    fn repeated_blob_orders_selected_pack_before_overlapping_physical_pack() {
        let blob_id = "BLB-3710db9ee7993ad1420b";
        let selected_pack_id = "PCK-Z-SELECTED";
        let overlapping_pack_id = "PCK-A-OVERLAP";
        let inputs = BTreeMap::from([
            (
                overlapping_pack_id.to_string(),
                object_pack_input(overlapping_pack_id, vec![object_pack_member(blob_id)]),
            ),
            (
                selected_pack_id.to_string(),
                object_pack_input(selected_pack_id, vec![object_pack_member(blob_id)]),
            ),
        ]);
        let selected =
            BTreeMap::from([(blob_id.to_ascii_lowercase(), selected_pack_id.to_string())]);

        let (_, _, dependencies) =
            object_pack_dependency_index(&inputs, &selected).expect("overlap should index");
        assert_eq!(
            dependencies[overlapping_pack_id],
            BTreeSet::from([selected_pack_id.to_string()])
        );
        assert!(dependencies[selected_pack_id].is_empty());
        assert_eq!(
            dependency_order("object-pack", inputs.len(), &dependencies)
                .expect("selected precedence should be acyclic"),
            vec![
                selected_pack_id.to_string(),
                overlapping_pack_id.to_string()
            ]
        );
    }

    #[test]
    fn duplicate_blob_inside_one_object_pack_remains_rejected() {
        let blob_id = "BLB-3710db9ee7993ad1420b";
        let pack_id = "PCK-DUPLICATE";
        let inputs = BTreeMap::from([(
            pack_id.to_string(),
            object_pack_input(
                pack_id,
                vec![object_pack_member(blob_id), object_pack_member(blob_id)],
            ),
        )]);
        let selected = BTreeMap::from([(blob_id.to_ascii_lowercase(), pack_id.to_string())]);

        let error = object_pack_dependency_index(&inputs, &selected)
            .expect_err("one pack must not contain a duplicate Blob row");
        assert!(error.contains("contains duplicate blob"), "{error}");
    }

    #[test]
    fn repeated_tree_orders_selected_pack_before_overlapping_physical_pack() {
        let tree_id = "TRE-0102030405060708090A";
        let selected_pack_id = "TPK-Z-SELECTED";
        let overlapping_pack_id = "TPK-A-OVERLAP";
        let inputs = BTreeMap::from([
            (
                overlapping_pack_id.to_string(),
                tree_pack_input(overlapping_pack_id, vec![tree_pack_tree(tree_id)]),
            ),
            (
                selected_pack_id.to_string(),
                tree_pack_input(selected_pack_id, vec![tree_pack_tree(tree_id)]),
            ),
        ]);
        let selected =
            BTreeMap::from([(tree_id.to_ascii_lowercase(), selected_pack_id.to_string())]);

        let (_, _, dependencies) =
            tree_pack_dependency_index(&inputs, &selected).expect("overlap should index");
        assert_eq!(
            dependencies[overlapping_pack_id],
            BTreeSet::from([selected_pack_id.to_string()])
        );
        assert!(dependencies[selected_pack_id].is_empty());
        assert_eq!(
            dependency_order("tree-pack", inputs.len(), &dependencies)
                .expect("selected precedence should be acyclic"),
            vec![
                selected_pack_id.to_string(),
                overlapping_pack_id.to_string()
            ]
        );
    }

    #[test]
    fn duplicate_tree_inside_one_tree_pack_remains_rejected() {
        let tree_id = "TRE-0102030405060708090A";
        let pack_id = "TPK-DUPLICATE";
        let inputs = BTreeMap::from([(
            pack_id.to_string(),
            tree_pack_input(
                pack_id,
                vec![tree_pack_tree(tree_id), tree_pack_tree(tree_id)],
            ),
        )]);
        let selected = BTreeMap::from([(tree_id.to_ascii_lowercase(), pack_id.to_string())]);

        let error = tree_pack_dependency_index(&inputs, &selected)
            .expect_err("one pack must not contain a duplicate Tree row");
        assert!(error.contains("contains duplicate tree"), "{error}");
    }
}
