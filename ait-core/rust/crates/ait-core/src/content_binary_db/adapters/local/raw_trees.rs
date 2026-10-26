use super::*;
use crate::content_binary_db::{
    BinaryTreeCodec, BinaryTreePackCodec, TREE_PACK_BIN, TREE_PACK_RECORD_SIZE,
};

#[cfg(test)]
mod replay_tests;

#[derive(Clone, Debug)]
pub struct RawTreeLocator {
    pub tree_id: String,
    pub tree_index: u32,
    pub tree_pack_index: u32,
    pub root_entry_ordinal: u32,
}

#[derive(Default)]
pub struct RawTreeSession {
    scanned_packs: u32,
    locators: BTreeMap<String, RawTreeLocator>,
}

impl RawTreeSession {
    fn refresh<const W: u32>(&mut self, content: &LocalContentBinaryDb<W>) -> Result<(), String> {
        let read = BinaryDbReadTxn::new(content.db());
        let file = crate::binary_db::BinaryFileId::new(TREE_PACK_BIN, W, TREE_PACK_RECORD_SIZE);
        let count = read.record_count(file.clone())?;
        if count < self.scanned_packs {
            return Err("Tree packs changed; reopen metadata session".into());
        }
        for pack_index in self.scanned_packs..count {
            let pack = BinaryTreePackCodec::<W>::decode_record(
                &read.read_record(file.clone(), pack_index)?,
            )?;
            if !pack.is_ready() || pack.is_tombstone() {
                return Err("unready Tree pack in raw artifact authority".into());
            }
            for ordinal in 0..pack.tree_count {
                let index = pack
                    .first_tree_index
                    .checked_add(ordinal)
                    .ok_or("Tree ordinal overflow")?;
                let tree = BinaryTreeCodec::<W>::decode_record(
                    &read.read_record(BinaryTreeCodec::<W>::record_file(), index)?,
                )?;
                if tree.is_tombstone() {
                    return Err("tombstoned raw artifact Tree".into());
                }
                let id = crate::content_binary_db::tree_id_from_hash80(&tree.tree_hash80);
                if self
                    .locators
                    .insert(
                        id.clone(),
                        RawTreeLocator {
                            tree_id: id,
                            tree_index: index,
                            tree_pack_index: pack_index,
                            root_entry_ordinal: ordinal,
                        },
                    )
                    .is_some()
                {
                    return Err("duplicate Tree identity in native packs".into());
                }
            }
        }
        self.scanned_packs = count;
        Ok(())
    }
}

impl<const W: u32> LocalContentBinaryDb<W> {
    /// Materialize a validated completed-history plan without changing source
    /// workspaces, Lines, terminal owners, or their successful finish receipts.
    /// Repeated preparation reuses an exact prior replay identity and time.
    pub fn replay_completed_snapshot_history<M>(
        &self,
        repo_name: &str,
        namespace: &str,
        steps: &[crate::snapshot_rebase::SnapshotHistoryRebaseStep<'_, M>],
    ) -> Result<Vec<SnapshotRecord>, String> {
        use crate::content_binary_db::BinaryDbSnapshotReplayWriteInput;
        use crate::snapshot_rebase::SnapshotReplayParent;
        use crate::workflow_binary_db::BinaryDbWorkflowStore;
        let workflow = BinaryDbWorkflowStore::<_, W>::new_with_namespace(
            self.db().clone(),
            repo_name,
            namespace,
        );
        let mut results = Vec::<SnapshotRecord>::with_capacity(steps.len());
        let mut pending = Vec::new();
        let mut session = RawTreeSession::default();
        for (ordinal, step) in steps.iter().enumerate() {
            let source = self
                .snapshots
                .snapshot_by_id(&step.source.snapshot_id)?
                .ok_or("Replay source Snapshot missing")?;
            let owners = workflow
                .snapshot_ownership_rows(std::slice::from_ref(&source.snapshot_id))
                .map_err(|error| error.to_string())?;
            let owner = owners
                .first()
                .filter(|_| owners.len() == 1)
                .ok_or("Replay source has no exact Snapshot owner")?;
            if owner["ownership_source"].as_str() != Some("durable_snapshot_binding")
                || owner["task_id"].as_str() != Some(step.source.owner.task_id.as_str())
                || owner["change_id"].as_str()
                    != (!step.source.owner.change_id.is_empty())
                        .then_some(step.source.owner.change_id.as_str())
                || source.line_name != step.source.owner.line_name
            {
                return Err("Replay plan does not match immutable source ownership".into());
            }
            let task =
                crate::task_store::TaskStore::get_task(&workflow, &step.source.owner.task_id)
                    .map_err(|error| error.to_string())?;
            if task["status"].as_str() != Some("completed") || task["local_finish"].is_null() {
                return Err("Replay source Task has no completed successful receipt".into());
            }
            let parent = match &step.parent {
                SnapshotReplayParent::Existing(id) => id.clone(),
                SnapshotReplayParent::Replayed(index) if *index < ordinal => {
                    results[*index].snapshot_id.clone()
                }
                _ => return Err("Replay parent does not precede its child".into()),
            };
            if let Some(reused) = step.reuse_snapshot_id {
                let actual_files = self
                    .snapshot_tree_file_rows(Some(&source.snapshot_id))?
                    .into_iter()
                    .map(|row| (row.path.clone(), row))
                    .collect::<BTreeMap<_, _>>();
                if reused != source.snapshot_id
                    || source.parent_snapshot_ids != [parent]
                    || step.files != actual_files
                {
                    return Err(
                        "Replay reuse differs from immutable source ancestry or content".into(),
                    );
                }
                results.push(source);
                continue;
            }
            let mut files = Vec::with_capacity(step.files.len());
            let read = BinaryDbReadTxn::new(self.db());
            let mut total_bytes = 0i64;
            for (path, row) in &step.files {
                if path != &row.path {
                    return Err("Replay tree path disagrees with its file row".into());
                }
                if path
                    .split('/')
                    .scan(String::new(), |prefix, part| {
                        if !prefix.is_empty() {
                            prefix.push('/');
                        }
                        prefix.push_str(part);
                        Some(prefix.clone())
                    })
                    .any(|prefix| prefix != *path && step.files.contains_key(&prefix))
                {
                    return Err(format!(
                        "History rebase conflict between file and directory at {path}"
                    ));
                }
                let blob = self
                    .blobs
                    .get_blob_view(&read, &row.blob_id)?
                    .ok_or("Replay file Blob is missing")?;
                if i64::try_from(blob.size_bytes).ok() != Some(row.size_bytes)
                    || blob.sha256 != row.sha256
                {
                    return Err("Replay file content differs from immutable Blob metadata".into());
                }
                total_bytes = total_bytes
                    .checked_add(row.size_bytes)
                    .ok_or("Replay tree byte count overflow")?;
                let mode = u16::from_str_radix(row.mode.strip_prefix("0o").unwrap_or(&row.mode), 8)
                    .map_err(|error| error.to_string())?;
                files.push((path.clone(), blob.blob_index, mode));
            }
            drop(read);
            let root = self
                .ensure_raw_trees_with_mode_encoding(&[files], &mut session, true)?
                .remove(0);
            let parents = vec![parent];
            let (snapshot_id, manifest_hash) = build_snapshot_id_with_parents(
                repo_name,
                &source.line_name,
                &parents,
                source.message.as_deref(),
                &root.tree_id,
            );
            if let Some(existing) = self.snapshots.snapshot_by_id(&snapshot_id)? {
                if existing.parent_snapshot_ids != parents
                    || existing.manifest_hash != manifest_hash
                    || existing.line_name != source.line_name
                    || existing.message != source.message
                    || existing.file_count != step.files.len() as i64
                    || existing.total_bytes != total_bytes
                    || self.snapshot_tree_root_locator(&snapshot_id)?.root_tree_id != root.tree_id
                {
                    return Err("Existing replay Snapshot has conflicting immutable content".into());
                }
                let owned = workflow
                    .snapshot_ownership_rows(std::slice::from_ref(&snapshot_id))
                    .map_err(|error| error.to_string())?;
                if owned.len() != 1
                    || owned[0]["task_id"] != owner["task_id"]
                    || owned[0]["change_id"] != owner["change_id"]
                    || owned[0]["ownership_source"] != "durable_snapshot_binding"
                {
                    return Err("Existing replay Snapshot belongs to another Task/Change".into());
                }
                results.push(existing);
                continue;
            }
            let read = BinaryDbReadTxn::new(self.db());
            let pack = BinaryTreePackCodec::<W>::decode_record(&read.read_record(
                BinaryTreePackCodec::<W>::record_file(),
                root.tree_pack_index,
            )?)?;
            let pack_id = tree_pack_id_from_hash48(
                (u64::from(pack.pack_hash_hi16) << 32) | u64::from(pack.pack_hash_lo32),
            );
            drop(read);
            let created_at = current_timestamp();
            pending.push(BinaryDbSnapshotReplayWriteInput {
                source_snapshot_id: source.snapshot_id.clone(),
                snapshot: BinaryDbSnapshotWriteInput {
                    snapshot_id: snapshot_id.clone(),
                    parent_snapshot_ids: parents.clone(),
                    root_tree_pack_id: pack_id.clone(),
                    root_entry_ordinal: i64::from(root.root_entry_ordinal),
                    manifest_hash: manifest_hash.clone(),
                    message: source.message.clone(),
                    line_name: source.line_name.clone(),
                    snapshot_kind: "line".into(),
                    file_count: step.files.len() as i64,
                    total_bytes,
                    created_at: created_at.clone(),
                },
            });
            results.push(SnapshotRecord {
                snapshot_id,
                parent_snapshot_ids: parents.clone(),
                primary_parent_snapshot_id: parents.first().cloned(),
                parent_snapshot_id: parents.first().cloned(),
                root_tree_pack_id: Some(pack_id),
                root_entry_ordinal: Some(i64::from(root.root_entry_ordinal)),
                manifest_hash,
                message: source.message,
                line_name: source.line_name,
                snapshot_kind: "line".into(),
                file_count: step.files.len() as i64,
                total_bytes,
                created_at,
            });
        }
        BinaryDbContentWriteCoordinator::new(
            &self.blobs,
            &self.object_packs,
            &self.tree_packs,
            &self.trees,
            &self.snapshots,
        )
        .record_replayed_snapshots(
            BinaryDbCommandScope::ContentWrite,
            &pending,
            repo_name,
            namespace,
        )
        .map_err(|error| error.to_string())?;
        results
            .into_iter()
            .map(|record| {
                self.snapshots
                    .snapshot_by_id(&record.snapshot_id)?
                    .ok_or_else(|| "Replay Snapshot missing after atomic capture".to_string())
            })
            .collect()
    }

    /// Git capture supplies already verified native Blob ordinals. Preserve
    /// the canonical Tree/Snapshot algorithms without filesystem hash caches
    /// or an expanded file JSON report for this disposable capture.
    pub fn create_git_code_snapshot(
        &self,
        repo_name: &str,
        line_name: &str,
        parents: &[String],
        message: Option<&str>,
        files: &[(String, u32, u16)],
        session: &mut RawTreeSession,
    ) -> Result<crate::content_binary_db::BinarySnapshotView, String> {
        if files
            .iter()
            .any(|(path, _, _)| path.to_ascii_lowercase().ends_with(".md"))
        {
            return Err("code Snapshot cannot own Markdown artifacts".into());
        }
        let (parents, _, _) =
            normalize_snapshot_parent_set(None, Some(parents.to_vec()), None, None)?;
        let root = self
            .ensure_raw_trees_with_mode_encoding(&[files.to_vec()], session, true)?
            .remove(0);
        if parents.len() == 1
            && self.snapshot_tree_root_locator(&parents[0])?.root_tree_id == root.tree_id
        {
            return self
                .snapshots
                .get_snapshot_view(&BinaryDbReadTxn::new(self.db()), &parents[0])?
                .ok_or_else(|| "projected parent Snapshot missing".into());
        }
        let message = normalize_optional_text(message);
        let (snapshot_id, manifest_hash) = build_snapshot_id_with_parents(
            repo_name,
            line_name,
            &parents,
            message.as_deref(),
            &root.tree_id,
        );
        let read = BinaryDbReadTxn::new(self.db());
        let total_bytes = files.iter().try_fold(0i64, |total, (_, index, _)| {
            let blob = self.blobs.blob_view_at(&read, *index)?;
            let size = i64::try_from(blob.size_bytes).map_err(|_| "code Blob size exceeds i64")?;
            total.checked_add(size).ok_or_else(|| {
                crate::binary_db::BinaryDbError::invalid_domain_data("code byte count overflow")
            })
        })?;
        let pack =
            crate::content_binary_db::BinaryTreePackCodec::<W>::decode_record(&read.read_record(
                crate::content_binary_db::BinaryTreePackCodec::<W>::record_file(),
                root.tree_pack_index,
            )?)?;
        let pack_id = tree_pack_id_from_hash48(
            (u64::from(pack.pack_hash_hi16) << 32) | u64::from(pack.pack_hash_lo32),
        );
        drop(read);
        BinaryDbContentWriteCoordinator::new(
            &self.blobs,
            &self.object_packs,
            &self.tree_packs,
            &self.trees,
            &self.snapshots,
        )
        .record_snapshot(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: snapshot_id.clone(),
                parent_snapshot_ids: parents,
                root_tree_pack_id: pack_id,
                root_entry_ordinal: i64::from(root.root_entry_ordinal),
                manifest_hash,
                message,
                line_name: line_name.to_string(),
                snapshot_kind: "line".into(),
                file_count: i64::try_from(files.len())
                    .map_err(|_| "code file count exceeds i64")?,
                total_bytes,
                created_at: current_timestamp(),
            },
        )
        .map_err(|error| error.to_string())?;
        self.snapshots
            .get_snapshot_view(&BinaryDbReadTxn::new(self.db()), &snapshot_id)?
            .ok_or_else(|| "Git code Snapshot absent after native commit".into())
    }

    /// Paths and native modes are preflighted by the source-artifact boundary.
    /// Multiple roots share one existing Tree pack and native Tree ID algorithm.
    pub fn ensure_raw_trees(
        &self,
        roots: &[Vec<(String, u32, u16)>],
        session: &mut RawTreeSession,
    ) -> Result<Vec<RawTreeLocator>, String> {
        self.ensure_raw_trees_with_mode_encoding(roots, session, false)
    }
    fn ensure_raw_trees_with_mode_encoding(
        &self,
        roots: &[Vec<(String, u32, u16)>],
        session: &mut RawTreeSession,
        snapshot_modes: bool,
    ) -> Result<Vec<RawTreeLocator>, String> {
        session.refresh(self)?;
        let read = BinaryDbReadTxn::new(self.db());
        let mut files = Vec::new();
        let mut all_rows = BTreeMap::new();
        let mut all_entries = BTreeMap::new();
        let mut root_ids = Vec::new();
        for root in roots {
            let mut entries = Vec::new();
            let mut paths = BTreeSet::new();
            for (path, index, mode) in root {
                crate::git_binary_db::validate_git_artifact_path(path)?;
                if !paths.insert(path.clone()) {
                    return Err("duplicate raw Tree path".into());
                }
                if !matches!(*mode, 0o644 | 0o755 | 0o120000)
                    && !(snapshot_modes
                        && (*mode <= 0o777 || matches!(*mode & !0o777, 0o100000 | 0o120000)))
                {
                    return Err("unsupported native artifact mode".into());
                }
                let blob = self.blobs.blob_view_at(&read, *index)?;
                entries.push(SnapshotFileEntry {
                    path: path.clone(),
                    blob_id: blob.blob_id,
                    size_bytes: i64::try_from(blob.size_bytes)
                        .map_err(|_| "raw artifact size exceeds i64")?,
                    mode: if snapshot_modes {
                        format!("0o{mode:o}")
                    } else if *mode == 0o120000 {
                        "120000".into()
                    } else {
                        format!("{:04o}", mode)
                    },
                    sha256: blob.sha256,
                    data: Vec::new(),
                    data_reused: true,
                    cache_fingerprint: None,
                });
            }
            let (id, rows, tree_entries) = build_tree_records(&entries)?;
            root_ids.push(id);
            for row in rows {
                all_rows.insert(row.tree_id.clone(), row);
            }
            for entry in tree_entries {
                all_entries.insert((entry.tree_id.clone(), entry.entry_name.clone()), entry);
            }
            files.extend(entries);
        }
        drop(read);
        let missing = all_rows
            .into_values()
            .filter(|row| !session.locators.contains_key(&row.tree_id))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let ids = missing
                .iter()
                .map(|row| row.tree_id.clone())
                .collect::<BTreeSet<_>>();
            let entries = all_entries
                .into_values()
                .filter(|entry| ids.contains(&entry.tree_id))
                .collect::<Vec<_>>();
            let seed = format!(
                "native-raw-trees/v1\n{}",
                ids.iter().cloned().collect::<Vec<_>>().join("\n")
            );
            let pack_id = format!(
                "TPK-{}",
                hex_lower(&sha256_array(seed.as_bytes()))[..12].to_ascii_uppercase()
            );
            let pack_rel_path = default_tree_pack_relative_path(&pack_id);
            let path = self.pack_root.as_path().join(&pack_rel_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            let members = build_tree_pack_members(
                &tree_rows_json(&missing),
                &tree_entry_rows_json(&entries, &files),
            )?;
            let stats = write_tree_pack_archive_with_format(
                path.to_string_lossy().as_ref(),
                &pack_id,
                CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
                &members,
                zstd_only_tree_pack_write_format(),
            )?;
            BinaryDbContentWriteCoordinator::new(
                &self.blobs,
                &self.object_packs,
                &self.tree_packs,
                &self.trees,
                &self.snapshots,
            )
            .record_tree_pack_metadata_with_entries(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbTreePackWriteInput {
                    pack_id,
                    pack_rel_path,
                    pack_format: json_string_field(&stats, "pack_format")?,
                    tree_count: json_i64_field(&stats, "tree_count")?,
                    total_bytes: json_i64_field(&stats, "total_bytes")?,
                    created_at: current_timestamp(),
                    trees: missing
                        .iter()
                        .map(|row| BinaryDbTreePackTreeWriteInput {
                            tree_id: row.tree_id.clone(),
                            entry_count: row.entry_count,
                        })
                        .collect(),
                },
                &entries
                    .iter()
                    .map(|entry| BinaryDbTreeEntryWriteInput {
                        tree_id: entry.tree_id.clone(),
                        entry_name: entry.entry_name.clone(),
                        entry_type: entry.entry_type.clone(),
                        target_id: entry.target_id.clone(),
                        mode: entry.mode.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| error.to_string())?;
            session.refresh(self)?;
        }
        root_ids
            .iter()
            .map(|id| {
                session
                    .locators
                    .get(id)
                    .cloned()
                    .ok_or_else(|| format!("raw Tree {id} missing from native pack"))
            })
            .collect()
    }
}
