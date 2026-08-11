use super::*;

impl<const WRITE_LAYOUT: u32> LocalContentBinaryDb<WRITE_LAYOUT> {
    pub fn create_no_parent_snapshot_content(
        &self,
        repo_name: &str,
        line_name: &str,
        message: Option<&str>,
        is_worktree: bool,
    ) -> Result<JsonValue, String> {
        self.create_snapshot_content(repo_name, line_name, None, message, is_worktree)
    }

    pub fn create_snapshot_content(
        &self,
        repo_name: &str,
        line_name: &str,
        parent_snapshot_id: Option<&str>,
        message: Option<&str>,
        is_worktree: bool,
    ) -> Result<JsonValue, String> {
        let parent_snapshot_ids = normalize_optional_text(parent_snapshot_id)
            .into_iter()
            .collect::<Vec<_>>();
        self.create_snapshot_content_with_parents(
            repo_name,
            line_name,
            &parent_snapshot_ids,
            message,
            is_worktree,
        )
    }

    pub fn create_snapshot_content_with_parents(
        &self,
        repo_name: &str,
        line_name: &str,
        parent_snapshot_ids: &[String],
        message: Option<&str>,
        is_worktree: bool,
    ) -> Result<JsonValue, String> {
        self.create_snapshot_content_with_parents_and_options(
            repo_name,
            line_name,
            parent_snapshot_ids,
            message,
            is_worktree,
            SnapshotAuthoringOptions::default(),
        )
    }

    pub fn create_snapshot_content_with_parents_and_options(
        &self,
        repo_name: &str,
        line_name: &str,
        parent_snapshot_ids: &[String],
        message: Option<&str>,
        is_worktree: bool,
        options: SnapshotAuthoringOptions,
    ) -> Result<JsonValue, String> {
        let _snapshot_range = crate::perfetto_range!("ait.core.snapshot.create");
        let total_started = Instant::now();
        let repo_root = {
            let _range = crate::perfetto_range!("ait.core.snapshot.repository_discovery");
            self.workspace_root
                .as_path()
                .canonicalize()
                .map_err(io_error)?
        };
        let pack_root = self.pack_root.as_path();
        let normalized_repo_name = require_non_empty(repo_name, "repo_name")?;
        let normalized_line_name = require_non_empty(line_name, "line_name")?;
        let (normalized_parent_snapshot_ids, normalized_primary_parent_snapshot_id, _) =
            normalize_snapshot_parent_set(None, Some(parent_snapshot_ids.to_vec()), None, None)?;
        for parent_snapshot_id in &normalized_parent_snapshot_ids {
            if !self.snapshots.snapshot_exists(parent_snapshot_id)? {
                return Err(format!(
                    "Snapshot parent {parent_snapshot_id} is missing before authoring line {normalized_line_name}."
                ));
            }
        }
        let normalized_message = normalize_optional_text(message);
        let runtime_root = active_workspace_runtime_root(&repo_root);
        let visible_list_started = Instant::now();
        let visible_entries = {
            let _range = crate::perfetto_range!("ait.core.snapshot.workspace_scan");
            list_visible_workspace_entries(
                repo_root.to_string_lossy().as_ref(),
                None,
                runtime_root.as_deref(),
            )
            .map_err(|err| format!("{err:?}"))?
        };
        let ignore_policy = workspace_ignore_policy(
            &repo_root,
            runtime_root.as_deref(),
            &visible_entries.operational_external_roots,
        );
        let mut phase_timings_ms = JsonMap::new();
        phase_timings_ms.insert(
            "workspace_scan".to_string(),
            number_json(elapsed_ms(visible_list_started)),
        );
        phase_timings_ms.insert("ignore_filtering".to_string(), number_json(0.0));

        let primary_parent_root = normalized_primary_parent_snapshot_id
            .as_deref()
            .map(|snapshot_id| self.snapshots.snapshot_tree_root_locator(snapshot_id))
            .transpose()?;
        let cache_read_started = Instant::now();
        let workspace_hash_cache = {
            let _range = crate::perfetto_range!("ait.core.snapshot.hash_cache_read");
            normalized_primary_parent_snapshot_id
                .as_deref()
                .map(|snapshot_id| load_workspace_hash_cache(&repo_root, snapshot_id))
        };
        let cache_root_matches = workspace_hash_cache
            .as_ref()
            .and_then(|load| load.cache())
            .zip(primary_parent_root.as_ref())
            .is_some_and(|(cache, parent)| cache.root_tree_id == parent.root_tree_id);
        let cache_read_state = if workspace_hash_cache
            .as_ref()
            .and_then(|load| load.cache())
            .is_some()
            && !cache_root_matches
        {
            "root_mismatch_fallback"
        } else {
            workspace_hash_cache
                .as_ref()
                .map(|cache| cache.state())
                .unwrap_or("no_parent")
        };
        let cache_read_elapsed = elapsed_ms(cache_read_started);
        let projection_started = Instant::now();
        let mut file_entries = Vec::new();
        let mut reused_paths = 0_usize;
        let mut rehashed_paths = 0_usize;
        let visible_file_metadata = visible_entries.file_metadata;
        {
            let _range = crate::perfetto_range!("ait.core.snapshot.workspace_projection_hash");
            for rel_path in visible_entries.files {
                if path_is_projected_out_for_workspace(
                    repo_root.to_string_lossy().as_ref(),
                    &rel_path,
                    is_worktree,
                ) {
                    continue;
                }
                let abs_path = repo_root.join(&rel_path);
                let fingerprint_before = match visible_file_metadata.get(&rel_path) {
                    Some(metadata) => workspace_file_fingerprint_from_visible_metadata(metadata),
                    None => workspace_file_fingerprint(&abs_path)?,
                };
                let is_symlink = fingerprint_before.file_kind == "symlink";
                if let Some(cached_entry) = workspace_hash_cache
                    .as_ref()
                    .and_then(|load| load.cache())
                    .filter(|_| cache_root_matches)
                    .and_then(|cache| cache.entries.get(&rel_path))
                    .filter(|entry| entry.fingerprint == fingerprint_before)
                {
                    file_entries.push(SnapshotFileEntry {
                        path: rel_path,
                        blob_id: cached_entry.blob_id.clone(),
                        size_bytes: cached_entry.size_bytes as i64,
                        mode: cached_entry.mode.clone(),
                        sha256: cached_entry.sha256.clone(),
                        data: Vec::new(),
                        data_reused: true,
                        cache_fingerprint: Some(fingerprint_before),
                    });
                    reused_paths = reused_paths.saturating_add(1);
                    continue;
                }
                let data = if is_symlink {
                    let target = fs::read_link(&abs_path).map_err(io_error)?;
                    #[cfg(unix)]
                    {
                        target.as_os_str().as_bytes().to_vec()
                    }
                    #[cfg(windows)]
                    {
                        target
                            .to_str()
                            .ok_or_else(|| {
                                format!(
                                    "Windows symlink target is not valid Unicode: {}",
                                    target.display()
                                )
                            })?
                            .as_bytes()
                            .to_vec()
                    }
                } else {
                    fs::read(&abs_path).map_err(io_error)?
                };
                let fingerprint_after = workspace_file_fingerprint(&abs_path)?;
                if fingerprint_before != fingerprint_after {
                    return Err(format!(
                    "Workspace path {rel_path} changed while Snapshot content was being read; retry Snapshot creation."
                ));
                }
                let sha256 = sha256_array(&data);
                let sha256_hex = hex_lower(&sha256);
                let blob_id = blob_id_from_sha256(&sha256);
                let permission_bits = fingerprint_before.mode_bits & 0o777;
                let mode_bits = if is_symlink {
                    0o120000 | permission_bits
                } else {
                    permission_bits
                };
                let mode = format!("{:#o}", mode_bits);
                file_entries.push(SnapshotFileEntry {
                    path: rel_path,
                    blob_id,
                    size_bytes: data.len() as i64,
                    mode,
                    sha256: sha256_hex,
                    data,
                    data_reused: false,
                    cache_fingerprint: Some(fingerprint_after),
                });
                rehashed_paths = rehashed_paths.saturating_add(1);
            }
        }
        self.preserve_parent_worktree_cargo_config_entry(
            normalized_primary_parent_snapshot_id.as_deref(),
            is_worktree,
            &repo_root,
            &mut file_entries,
        )?;
        let projection_elapsed = elapsed_ms(projection_started);
        phase_timings_ms.insert(
            "workspace_projection_filter".to_string(),
            number_json(projection_elapsed),
        );
        phase_timings_ms.insert("hashing".to_string(), number_json(projection_elapsed));
        phase_timings_ms.insert(
            "hashing_cache".to_string(),
            json!({
                "reused_paths": reused_paths,
                "rehashed_paths": rehashed_paths,
                "state_read": cache_read_state,
                "state_write": "pending",
                "read_ms": cache_read_elapsed,
            }),
        );

        let tree_build_started = Instant::now();
        let (root_tree_id, tree_rows, tree_entry_rows) = {
            let _range = crate::perfetto_range!("ait.core.snapshot.tree_build");
            build_tree_records(&file_entries)?
        };
        phase_timings_ms.insert(
            "tree_build".to_string(),
            number_json(elapsed_ms(tree_build_started)),
        );
        if !options.allow_unchanged_tree && normalized_parent_snapshot_ids.len() <= 1 {
            if let Some(parent_snapshot_id) = normalized_primary_parent_snapshot_id.as_deref() {
                let parent_root = primary_parent_root
                    .as_ref()
                    .expect("normalized primary parent root");
                if parent_root.root_tree_id == root_tree_id {
                    let cache_entries = file_entries.iter().filter_map(|entry| {
                        entry.cache_fingerprint.clone().map(|fingerprint| {
                            workspace_hash_cache_entry(
                                &entry.path,
                                &entry.blob_id,
                                &entry.sha256,
                                entry.size_bytes as u64,
                                &entry.mode,
                                fingerprint,
                            )
                        })
                    });
                    let _ = write_workspace_hash_cache(
                        &repo_root,
                        parent_snapshot_id,
                        &root_tree_id,
                        cache_entries,
                    );
                    return Err(format!(
                    "Refusing to create snapshot for line {normalized_line_name}: workspace tree is unchanged from parent snapshot {parent_snapshot_id}; no snapshot created."
                ));
                }
            }
        }
        let (snapshot_id, revision_hash) = build_snapshot_id_with_parents(
            &normalized_repo_name,
            &normalized_line_name,
            &normalized_parent_snapshot_ids,
            normalized_message.as_deref(),
            &root_tree_id,
        );
        let created_at = current_timestamp();
        let coordinator = BinaryDbContentWriteCoordinator::new(
            &self.blobs,
            &self.object_packs,
            &self.tree_packs,
            &self.trees,
            &self.snapshots,
        );

        let hashing_started = Instant::now();
        let _blob_lookup_range = crate::perfetto_range!("ait.core.snapshot.blob_lookup");
        let mut blob_candidates = Vec::new();
        let mut new_blob_paths = BTreeSet::new();
        let blob_read = self.blobs.begin_read_txn();
        let mut seen_blob_ids = BTreeSet::new();
        for entry in &file_entries {
            if self
                .blobs
                .get_blob_view(&blob_read, &entry.blob_id)?
                .is_some()
                || !seen_blob_ids.insert(entry.blob_id.clone())
            {
                continue;
            }
            if entry.data_reused {
                return Err(format!(
                    "Workspace hash cache referenced missing authoritative blob {} for {}; retry after invalidating the derived cache.",
                    entry.blob_id, entry.path
                ));
            }
            if !entry.path.trim().is_empty() {
                new_blob_paths.insert(entry.path.clone());
            }
            blob_candidates.push(PackCandidate {
                entry_name: format!("blobs/{}", entry.blob_id),
                blob_id: entry.blob_id.clone(),
                data: entry.data.clone(),
                path_hint: Some(entry.path.clone()),
                chain_depth: 0,
            });
        }
        drop(blob_read);
        #[cfg(feature = "perfetto-tracing")]
        drop(_blob_lookup_range);
        phase_timings_ms.insert(
            "blob_lookup".to_string(),
            number_json(elapsed_ms(hashing_started)),
        );

        let mut blob_pack_elapsed = 0.0;
        let mut blob_delta_lookup_elapsed = 0.0;
        let mut blob_pack_assembly_elapsed = 0.0;
        let mut blob_pack_archive_elapsed = 0.0;
        let mut blob_pack_metadata_elapsed = 0.0;
        if !blob_candidates.is_empty() {
            let _range = crate::perfetto_range!("ait.core.snapshot.blob_pack_write");
            let blob_delta_lookup_started = Instant::now();
            let _delta_lookup_range = crate::perfetto_range!("ait.core.snapshot.blob_delta_lookup");
            let initial_by_path = self.parent_delta_candidates(
                normalized_primary_parent_snapshot_id.as_deref(),
                &new_blob_paths,
            )?;
            #[cfg(feature = "perfetto-tracing")]
            drop(_delta_lookup_range);
            blob_delta_lookup_elapsed = elapsed_ms(blob_delta_lookup_started);
            let pack_id = build_snapshot_object_pack_id(&snapshot_id, &blob_candidates)?;
            let pack_rel_path = default_object_pack_relative_path(&pack_id);
            let pack_abs = pack_root.join(&pack_rel_path);
            let blob_pack_started = Instant::now();
            let blob_pack_assembly_started = Instant::now();
            let _blob_assembly_range =
                crate::perfetto_range!("ait.core.snapshot.blob_pack_assembly");
            let members = build_typed_pack_members(
                blob_candidates,
                DEFAULT_MAX_DELTA_CHAIN_DEPTH,
                Some(&initial_by_path),
            );
            #[cfg(feature = "perfetto-tracing")]
            drop(_blob_assembly_range);
            blob_pack_assembly_elapsed = elapsed_ms(blob_pack_assembly_started);
            let blob_pack_archive_started = Instant::now();
            let _blob_archive_range =
                crate::perfetto_range!("ait.core.snapshot.blob_pack_archive_write");
            let archive_stats = write_typed_pack_archive_with_format(
                pack_abs.to_string_lossy().as_ref(),
                &pack_id,
                CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
                &members,
                zstd_only_object_pack_write_format(),
            )?;
            #[cfg(feature = "perfetto-tracing")]
            drop(_blob_archive_range);
            blob_pack_archive_elapsed = elapsed_ms(blob_pack_archive_started);
            let blob_pack_metadata_started = Instant::now();
            let _blob_metadata_range =
                crate::perfetto_range!("ait.core.snapshot.blob_pack_metadata_commit");
            coordinator.record_object_pack_metadata(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbObjectPackWriteInput {
                    pack_id: pack_id.clone(),
                    pack_rel_path,
                    pack_format: json_string_field(&archive_stats, "pack_format")?,
                    member_count: json_i64_field(&archive_stats, "member_count")?,
                    total_bytes: json_i64_field(&archive_stats, "total_bytes")?,
                    created_at: created_at.clone(),
                    members: object_pack_member_inputs(&file_entries, &members, &created_at)?,
                },
            )?;
            #[cfg(feature = "perfetto-tracing")]
            drop(_blob_metadata_range);
            blob_pack_metadata_elapsed = elapsed_ms(blob_pack_metadata_started);
            blob_pack_elapsed = elapsed_ms(blob_pack_started);
        }

        let tree_pack_started = Instant::now();
        let _tree_pack_range = crate::perfetto_range!("ait.core.snapshot.tree_pack_write");
        let tree_lookup_started = Instant::now();
        let _tree_lookup_range = crate::perfetto_range!("ait.core.snapshot.tree_lookup");
        let tree_read = self.trees.begin_read_txn();
        let existing_tree_ids = self.trees.existing_tree_ids(&tree_read)?;
        let missing_tree_rows = tree_rows
            .iter()
            .filter(|row| !existing_tree_ids.contains(&row.tree_id))
            .cloned()
            .collect::<Vec<_>>();
        let missing_tree_ids = missing_tree_rows
            .iter()
            .map(|row| row.tree_id.clone())
            .collect::<BTreeSet<_>>();
        let missing_tree_entry_rows = tree_entry_rows
            .iter()
            .filter(|row| missing_tree_ids.contains(&row.tree_id))
            .cloned()
            .collect::<Vec<_>>();
        drop(tree_read);
        #[cfg(feature = "perfetto-tracing")]
        drop(_tree_lookup_range);
        let tree_lookup_elapsed = elapsed_ms(tree_lookup_started);
        let mut tree_pack_assembly_elapsed = 0.0;
        let mut tree_pack_archive_elapsed = 0.0;
        let mut tree_pack_metadata_elapsed = 0.0;
        if missing_tree_ids.contains(&root_tree_id) {
            let tree_pack_assembly_started = Instant::now();
            let _assembly_range = crate::perfetto_range!("ait.core.snapshot.tree_pack_assembly");
            let tree_rows_json = tree_rows_json(&missing_tree_rows);
            let tree_entry_rows_json =
                tree_entry_rows_json(&missing_tree_entry_rows, &file_entries);
            let pack_seed = format!(
                "{snapshot_id}|{root_tree_id}|{}",
                missing_tree_rows
                    .iter()
                    .map(|row| row.tree_id.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            );
            let tree_pack_id = tree_pack_id_from_hash48(hash48_from_seed(pack_seed.as_bytes()));
            let tree_pack_rel_path = default_tree_pack_relative_path(&tree_pack_id);
            let tree_pack_abs = pack_root.join(&tree_pack_rel_path);
            let tree_members = build_tree_pack_members(&tree_rows_json, &tree_entry_rows_json)?;
            #[cfg(feature = "perfetto-tracing")]
            drop(_assembly_range);
            tree_pack_assembly_elapsed = elapsed_ms(tree_pack_assembly_started);
            let tree_pack_archive_started = Instant::now();
            let _archive_range =
                crate::perfetto_range!("ait.core.snapshot.tree_pack_archive_write");
            let tree_archive_stats = write_tree_pack_archive_with_format(
                tree_pack_abs.to_string_lossy().as_ref(),
                &tree_pack_id,
                CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
                &tree_members,
                zstd_only_tree_pack_write_format(),
            )?;
            #[cfg(feature = "perfetto-tracing")]
            drop(_archive_range);
            tree_pack_archive_elapsed = elapsed_ms(tree_pack_archive_started);
            let tree_pack_metadata_started = Instant::now();
            let _tree_metadata_range =
                crate::perfetto_range!("ait.core.snapshot.tree_pack_metadata_commit");
            coordinator.record_tree_pack_metadata_with_entries(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbTreePackWriteInput {
                    pack_id: tree_pack_id.clone(),
                    pack_rel_path: tree_pack_rel_path.clone(),
                    pack_format: json_string_field(&tree_archive_stats, "pack_format")?,
                    tree_count: json_i64_field(&tree_archive_stats, "tree_count")?,
                    total_bytes: json_i64_field(&tree_archive_stats, "total_bytes")?,
                    created_at: created_at.clone(),
                    trees: missing_tree_rows
                        .iter()
                        .map(|row| BinaryDbTreePackTreeWriteInput {
                            tree_id: row.tree_id.clone(),
                            entry_count: row.entry_count,
                        })
                        .collect(),
                },
                &missing_tree_entry_rows
                    .iter()
                    .map(|row| BinaryDbTreeEntryWriteInput {
                        tree_id: row.tree_id.clone(),
                        entry_name: row.entry_name.clone(),
                        entry_type: row.entry_type.clone(),
                        target_id: row.target_id.clone(),
                        mode: row.mode.clone(),
                    })
                    .collect::<Vec<_>>(),
            )?;
            #[cfg(feature = "perfetto-tracing")]
            drop(_tree_metadata_range);
            tree_pack_metadata_elapsed = elapsed_ms(tree_pack_metadata_started);
        } else if !missing_tree_rows.is_empty() {
            let missing = missing_tree_rows
                .iter()
                .map(|row| row.tree_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Binary DB snapshot create found pre-existing root tree {root_tree_id} with missing subtree metadata: {missing}."
            ));
        }
        let tree_locator_started = Instant::now();
        let _tree_locator_range = crate::perfetto_range!("ait.core.snapshot.tree_pack_locator");
        let (tree_pack_id, root_entry_ordinal) =
            self.tree_pack_locator_for_tree(&root_tree_id)?.ok_or_else(|| {
                format!(
                    "Snapshot {snapshot_id} is missing tree-pack root locator metadata for tree {root_tree_id}."
                )
            })?;
        #[cfg(feature = "perfetto-tracing")]
        drop(_tree_locator_range);
        let tree_locator_elapsed = elapsed_ms(tree_locator_started);
        let tree_pack_rel_path = default_tree_pack_relative_path(&tree_pack_id);
        let tree_pack_elapsed = elapsed_ms(tree_pack_started);
        #[cfg(feature = "perfetto-tracing")]
        drop(_tree_pack_range);
        phase_timings_ms.insert("tree_record_stage".to_string(), number_json(0.0));
        phase_timings_ms.insert(
            "pack_archive_write".to_string(),
            json!({
                "blob_pack_write": {
                    "archive_write": blob_pack_archive_elapsed,
                    "assembly": blob_pack_assembly_elapsed,
                    "delta_lookup": blob_delta_lookup_elapsed,
                    "metadata_commit": blob_pack_metadata_elapsed,
                    "total": blob_pack_elapsed,
                },
                "tree_pack_write": {
                    "archive_write": tree_pack_archive_elapsed,
                    "assembly": tree_pack_assembly_elapsed,
                    "lookup": tree_lookup_elapsed,
                    "metadata_commit": tree_pack_metadata_elapsed,
                    "root_locator": tree_locator_elapsed,
                    "total": tree_pack_elapsed,
                },
                "total": number_json(blob_pack_elapsed + tree_pack_elapsed),
            }),
        );

        let metadata_commit_started = Instant::now();
        let _metadata_range = crate::perfetto_range!("ait.core.snapshot.metadata_transaction");
        for entry in &file_entries {
            let Some(expected) = entry.cache_fingerprint.as_ref() else {
                continue;
            };
            let actual = workspace_file_fingerprint(&repo_root.join(&entry.path))?;
            if &actual != expected {
                return Err(format!(
                    "Workspace path {} changed before the Snapshot metadata transaction; retry Snapshot creation.",
                    entry.path
                ));
            }
        }
        coordinator.record_snapshot(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: snapshot_id.clone(),
                parent_snapshot_ids: normalized_parent_snapshot_ids.clone(),
                root_tree_pack_id: tree_pack_id.clone(),
                root_entry_ordinal,
                manifest_hash: revision_hash.clone(),
                message: normalized_message.clone(),
                line_name: normalized_line_name.clone(),
                snapshot_kind: "line".to_string(),
                file_count: file_entries.len() as i64,
                total_bytes: file_entries
                    .iter()
                    .map(|entry| entry.size_bytes)
                    .sum::<i64>(),
                created_at: created_at.clone(),
            },
        )?;
        #[cfg(feature = "perfetto-tracing")]
        drop(_metadata_range);
        phase_timings_ms.insert(
            "metadata_commit".to_string(),
            number_json(elapsed_ms(metadata_commit_started)),
        );
        let cache_write_started = Instant::now();
        let cache_write_state = {
            let _range = crate::perfetto_range!("ait.core.snapshot.hash_cache_write");
            let cache_entries = file_entries.iter().filter_map(|entry| {
                entry.cache_fingerprint.clone().map(|fingerprint| {
                    workspace_hash_cache_entry(
                        &entry.path,
                        &entry.blob_id,
                        &entry.sha256,
                        entry.size_bytes as u64,
                        &entry.mode,
                        fingerprint,
                    )
                })
            });
            match write_workspace_hash_cache(&repo_root, &snapshot_id, &root_tree_id, cache_entries)
            {
                Ok(_) => "written",
                Err(_) => "write_failed_fallback",
            }
        };
        if let Some(cache_timing) = phase_timings_ms
            .get_mut("hashing_cache")
            .and_then(JsonValue::as_object_mut)
        {
            cache_timing.insert(
                "state_write".to_string(),
                JsonValue::String(cache_write_state.to_string()),
            );
            cache_timing.insert(
                "write_ms".to_string(),
                number_json(elapsed_ms(cache_write_started)),
            );
        }
        phase_timings_ms.insert("total".to_string(), number_json(elapsed_ms(total_started)));

        let mut files = file_entries
            .iter()
            .map(snapshot_file_entry_json)
            .collect::<Vec<_>>();
        files.sort_by(|left, right| {
            left.get("path")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .cmp(
                    right
                        .get("path")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default(),
                )
        });
        Ok(json!({
            "snapshot_id": snapshot_id,
            "parent_snapshot_id": normalized_primary_parent_snapshot_id,
            "primary_parent_snapshot_id": normalized_primary_parent_snapshot_id,
            "parent_snapshot_ids": normalized_parent_snapshot_ids,
            "root_tree_pack_id": tree_pack_id,
            "root_entry_ordinal": root_entry_ordinal,
            "manifest_hash": revision_hash,
            "manifest_path": tree_pack_manifest_path(
                &tree_pack_rel_path,
                &format!("trees/{root_tree_id}.json")
            ),
            "message": normalized_message,
            "line_name": normalized_line_name,
            "snapshot_kind": "line",
            "file_count": file_entries.len() as i64,
            "total_bytes": file_entries.iter().map(|entry| entry.size_bytes).sum::<i64>(),
            "created_at": created_at,
            "files": files,
            "ignore_policy": ignore_policy,
            "phase_timings_ms": JsonValue::Object(phase_timings_ms),
        }))
    }

    pub fn ensure_blob_bytes_content(
        &self,
        data: &[u8],
        path_hint: Option<&str>,
    ) -> Result<String, String> {
        let digest = sha256_array(data);
        let digest_hex = hex_lower(&digest);
        let blob_id = blob_id_from_sha256(&digest);
        if let Some(existing) = self.blobs.get_blob(&blob_id)? {
            if existing.sha256 != digest_hex {
                return Err(format!(
                    "Binary DB blob {blob_id} sha256 metadata drifted during blob ensure."
                ));
            }
            if existing.size_bytes != data.len() as i64 {
                return Err(format!(
                    "Binary DB blob {blob_id} size metadata drifted during blob ensure."
                ));
            }
            return Ok(blob_id);
        }

        let created_at = current_timestamp();
        let pack_seed = format!("BLOB-{blob_id}|[\"{blob_id}\"]");
        let pack_id = format!(
            "PCK-{}",
            hex_lower(&sha256_array(pack_seed.as_bytes()))[..12].to_ascii_uppercase()
        );
        let pack_rel_path = default_object_pack_relative_path(&pack_id);
        let pack_abs_path = self.pack_root.as_path().join(&pack_rel_path);
        if let Some(parent) = pack_abs_path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }

        let members = build_pack_members(
            &json!([{
                "entry_name": format!("blobs/{blob_id}"),
                "blob_id": blob_id.clone(),
                "data": data,
                "path_hint": path_hint.unwrap_or(""),
            }]),
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            None,
        )?;
        let member_obj = members
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(JsonValue::as_object)
            .cloned()
            .ok_or_else(|| "Failed to build Binary DB blob pack member.".to_string())?;
        let archive_stats = write_pack_archive_with_format(
            pack_abs_path.to_string_lossy().as_ref(),
            &pack_id,
            CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
            &members,
            zstd_only_object_pack_write_format(),
        )?;
        let coordinator = BinaryDbContentWriteCoordinator::new(
            &self.blobs,
            &self.object_packs,
            &self.tree_packs,
            &self.trees,
            &self.snapshots,
        );
        coordinator
            .record_object_pack_metadata(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbObjectPackWriteInput {
                    pack_id,
                    pack_rel_path,
                    pack_format: json_string_field(&archive_stats, "pack_format")?,
                    member_count: json_i64_field(&archive_stats, "member_count")?,
                    total_bytes: json_i64_field(&archive_stats, "total_bytes")?,
                    created_at: created_at.clone(),
                    members: vec![BinaryDbObjectPackMemberWriteInput {
                        blob_id: blob_id.clone(),
                        sha256: digest_hex,
                        size_bytes: data.len() as i64,
                        pack_entry_type: member_obj
                            .get("entry_type")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("full")
                            .to_string(),
                        pack_base_blob_id: member_obj
                            .get("base_blob_id")
                            .and_then(JsonValue::as_str)
                            .map(str::to_string),
                        pack_chain_depth: member_obj
                            .get("chain_depth")
                            .and_then(JsonValue::as_i64)
                            .unwrap_or(0),
                        created_at,
                    }],
                },
            )
            .map_err(|err| err.to_string())?;
        Ok(blob_id)
    }
}
