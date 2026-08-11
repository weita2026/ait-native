use super::*;

impl<const WRITE_LAYOUT: u32> LocalContentBinaryDb<WRITE_LAYOUT> {
    pub(super) fn parent_delta_candidates(
        &self,
        parent_snapshot_id: Option<&str>,
        paths: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, PackCandidate>, String> {
        let Some(parent_snapshot_id) = parent_snapshot_id else {
            return Ok(BTreeMap::new());
        };
        if paths.is_empty() || !self.snapshots.snapshot_exists(parent_snapshot_id)? {
            return Ok(BTreeMap::new());
        }
        let requested_paths = paths.iter().cloned().collect::<Vec<_>>();
        let rows = self
            .snapshots
            .snapshot_tree_path_file_rows(parent_snapshot_id, &requested_paths)?;
        let read = self.blobs.begin_read_txn();
        let mut chain_depth_by_blob_id = BTreeMap::new();
        for row in rows.values() {
            if chain_depth_by_blob_id.contains_key(&row.blob_id) {
                continue;
            }
            if let Some(chain_depth) = self.validated_blob_chain_depth(&read, &row.blob_id)? {
                chain_depth_by_blob_id.insert(row.blob_id.clone(), chain_depth);
            }
        }
        let blob_ids = chain_depth_by_blob_id.keys().cloned().collect::<Vec<_>>();
        let bytes_by_blob_id = self.blobs.read_blob_bytes_batch(&blob_ids)?;
        let mut candidates = BTreeMap::new();
        for path in paths {
            let Some(row) = rows.get(path) else {
                continue;
            };
            let Some(chain_depth) = chain_depth_by_blob_id.get(&row.blob_id).copied() else {
                continue;
            };
            let bytes = bytes_by_blob_id.get(&row.blob_id).ok_or_else(|| {
                format!(
                    "Snapshot `{parent_snapshot_id}` is missing blob payload {} for {path}.",
                    row.blob_id
                )
            })?;
            let chain_depth = usize::try_from(chain_depth).map_err(|_| {
                format!(
                    "Snapshot `{parent_snapshot_id}` has invalid negative delta depth for {}.",
                    row.blob_id
                )
            })?;
            candidates.insert(
                path.clone(),
                PackCandidate {
                    entry_name: format!("blobs/{}", row.blob_id),
                    blob_id: row.blob_id.clone(),
                    data: bytes.clone(),
                    path_hint: Some(path.clone()),
                    chain_depth,
                },
            );
        }
        Ok(candidates)
    }

    fn validated_blob_chain_depth(
        &self,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
        blob_id: &str,
    ) -> Result<Option<i64>, String> {
        fn visit<const WRITE_LAYOUT: u32>(
            store: &LocalContentBinaryDb<WRITE_LAYOUT>,
            read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
            blob_id: &str,
            visiting: &mut BTreeSet<String>,
        ) -> Result<Option<i64>, String> {
            if !visiting.insert(blob_id.to_string()) {
                return Ok(None);
            }
            let result = match store.blobs.get_blob_view(read, blob_id)? {
                Some(blob) if !blob.record.is_tombstone() && !blob.record.is_pruned() => {
                    match blob.record.pack_member_index() {
                        Some(member_index) => {
                            let member = store
                                .object_packs
                                .object_pack_member_view_at(read, member_index)?;
                            if member.record.is_tombstone() || member.blob_id != blob_id {
                                None
                            } else {
                                let depth = i64::from(member.record.delta_chain_depth);
                                match member.record.member_kind() {
                                    BinaryObjectPackMemberKind::Full
                                        if member.base_blob_id.is_none() && depth == 0 =>
                                    {
                                        Some(0)
                                    }
                                    BinaryObjectPackMemberKind::Delta
                                        if depth > 0
                                            && depth <= DEFAULT_MAX_DELTA_CHAIN_DEPTH as i64 =>
                                    {
                                        match member.base_blob_id.as_deref() {
                                            Some(base_blob_id) => {
                                                match visit(store, read, base_blob_id, visiting)? {
                                                    Some(base_depth) if depth == base_depth + 1 => {
                                                        Some(depth)
                                                    }
                                                    _ => None,
                                                }
                                            }
                                            None => None,
                                        }
                                    }
                                    _ => None,
                                }
                            }
                        }
                        None => None,
                    }
                }
                _ => None,
            };
            visiting.remove(blob_id);
            Ok(result)
        }

        visit(self, read, blob_id, &mut BTreeSet::new())
    }

    pub(super) fn preserve_parent_worktree_cargo_config_entry(
        &self,
        parent_snapshot_id: Option<&str>,
        is_worktree: bool,
        repo_root: &Path,
        file_entries: &mut Vec<SnapshotFileEntry>,
    ) -> Result<(), String> {
        if !is_worktree
            || file_entries
                .iter()
                .any(|entry| entry.path == WORKTREE_CARGO_CONFIG_RELATIVE_PATH)
        {
            return Ok(());
        }
        let rel_path = Path::new(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
        let abs_path = repo_root.join(rel_path);
        if !is_generated_worktree_cargo_config(repo_root, rel_path, &abs_path) {
            return Ok(());
        }
        let Some(parent_snapshot_id) = parent_snapshot_id else {
            return Ok(());
        };
        let rows = self.snapshots.snapshot_tree_path_file_rows(
            parent_snapshot_id,
            &[WORKTREE_CARGO_CONFIG_RELATIVE_PATH.to_string()],
        )?;
        let Some(parent_row) = rows.get(WORKTREE_CARGO_CONFIG_RELATIVE_PATH) else {
            return Ok(());
        };
        let data = self.blobs.read_blob_bytes(&parent_row.blob_id)?;
        let sha256 = hex_lower(&sha256_array(&data));
        if !parent_row.sha256.is_empty() && parent_row.sha256 != sha256 {
            return Err(format!(
                "Snapshot `{parent_snapshot_id}` has inconsistent blob metadata for {WORKTREE_CARGO_CONFIG_RELATIVE_PATH}."
            ));
        }
        let size_bytes = data.len() as i64;
        if parent_row.size_bytes != 0 && parent_row.size_bytes != size_bytes {
            return Err(format!(
                "Snapshot `{parent_snapshot_id}` has inconsistent size metadata for {WORKTREE_CARGO_CONFIG_RELATIVE_PATH}."
            ));
        }
        file_entries.push(SnapshotFileEntry {
            path: parent_row.path.clone(),
            blob_id: parent_row.blob_id.clone(),
            size_bytes,
            mode: parent_row.mode.clone(),
            sha256,
            data,
            data_reused: false,
            cache_fingerprint: Some(workspace_file_fingerprint(&abs_path)?),
        });
        Ok(())
    }
}
