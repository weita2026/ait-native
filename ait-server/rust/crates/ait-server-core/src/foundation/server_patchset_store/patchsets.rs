use super::*;

impl PostgresPatchsetStore {
    pub(super) fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.transaction(|store| {
            let change = store.change_row(change_id)?;
            ensure_change_mutable(&change, "publish patchsets")?;
            let repo_name = required_text(change.get("repo_name"), "change.repo_name")?;
            let base_repo = store.snapshot_repo(base_snapshot_id)?;
            if base_repo.as_deref() != Some(repo_name.as_str()) {
                return Err(format!("Unknown base snapshot: {base_snapshot_id}"));
            }
            let revision_repo = store.snapshot_repo(revision_snapshot_id)?;
            if revision_repo.as_deref() != Some(repo_name.as_str()) {
                return Err(format!("Unknown revision snapshot: {revision_snapshot_id}"));
            }
            if !store.snapshot_is_ancestor(base_snapshot_id, revision_snapshot_id)? {
                return Err(format!(
                    "Revision snapshot `{revision_snapshot_id}` does not descend from base snapshot `{base_snapshot_id}` for change `{change_id}`."
                ));
            }
            if let Some(mut existing) =
                store.existing_patchset(change_id, base_snapshot_id, revision_snapshot_id)?
            {
                existing.insert(
                    "idempotency".to_string(),
                    json!({
                        "state": "reused_existing_patchset",
                        "change_id": change_id,
                        "base_snapshot_id": base_snapshot_id,
                        "revision_snapshot_id": revision_snapshot_id,
                    }),
                );
                return Ok(existing);
            }

            let current_patchset_number = int_value(change.get("current_patchset_number")).unwrap_or(0);
            let next_num = current_patchset_number + 1;
            let repo_id = optional_text(change.get("repo_id"))
                .or_else(|| store.repo_id_for_repo(&repo_name).ok().flatten())
                .ok_or_else(|| format!("Repository {repo_name} is missing repo_id"))?;
            let namespace_prefix = store
                .repo_namespace_prefix(&repo_name)?
                .unwrap_or_else(|| "AIT".to_string());
            let patchset_id = derive_patchset_id(change_id, next_num, Some(&namespace_prefix));
            let diff_stats = store.diff_stats(base_snapshot_id, revision_snapshot_id)?;
            let diff_stats_json =
                serde_json::to_string(&JsonValue::Object(diff_stats.clone())).map_err(|exc| exc.to_string())?;
            let now = utc_now();
            let patchsets = store.control_table("patchsets");
            if next_num > 1 {
                store
                    .client
                    .execute(
                        &format!(
                            "update {patchsets} set publish_state = 'superseded' where change_id = $1 and patchset_number = $2"
                        ),
                        &[&change_id, &((next_num - 1) as i32)],
                    )
                    .map_err(|exc| exc.to_string())?;
            }
            store
                .client
                .execute(
                    &format!("insert into {patchsets}(patchset_id, repo_id, change_id, patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at) values ($1, $2, $3, $4, $5, $6, $7, $8, 'published', $9, 'pending', $10::text::timestamptz)"),
                    &[&patchset_id, &repo_id, &change_id, &(next_num as i32), &base_snapshot_id, &revision_snapshot_id, &summary, &author_mode, &diff_stats_json, &now],
                )
                .map_err(|exc| exc.to_string())?;
            let changes = store.control_table("changes");
            store
                .client
                .execute(
                    &format!("update {changes} set current_patchset_number = $1, status = 'review', updated_at = $2::text::timestamptz, selected_patchset_number = coalesce(selected_patchset_number, $1) where change_id = $3"),
                    &[&(next_num as i32), &now, &change_id],
                )
                .map_err(|exc| exc.to_string())?;
            store.record_event(
                "patchset.published",
                "patchset",
                &patchset_id,
                &json!({
                    "change_id": change_id,
                    "patchset_number": next_num,
                    "base_snapshot_id": base_snapshot_id,
                    "revision_snapshot_id": revision_snapshot_id,
                }),
                &now,
            )?;
            store.refresh_change_state(change_id, &now)?;
            let mut out = store.get_patchset_in_txn(&patchset_id)?;
            out.insert("diff_stats".to_string(), JsonValue::Object(diff_stats));
            Ok(out)
        })
    }
    pub(super) fn list_patchsets(
        &mut self,
        change_id: &str,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let changes = self.control_table("changes");
        if self
            .client
            .query_opt(
                &format!("select 1 from {changes} where change_id = $1"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?
            .is_none()
        {
            return Err(format!("Unknown change: {change_id}"));
        }
        self.list_patchsets_for_change(change_id)
    }
    pub(super) fn list_patchsets_for_repo(
        &mut self,
        repo_name: &str,
        change_ref: &str,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let change = self.get_change_for_repo(repo_name, change_ref)?;
        let change_id = required_text(change.get("change_id"), "change.change_id")?;
        self.list_patchsets(&change_id)
    }
    pub(super) fn get_patchset(
        &mut self,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.get_patchset_in_txn(patchset_id)
    }
    pub(super) fn get_patchset_for_repo(
        &mut self,
        repo_name: &str,
        patchset_ref: &str,
        change_ref: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let repo_id = self
            .repo_id_for_repo(repo_name)?
            .ok_or_else(|| format!("Unknown repository: {repo_name}"))?;
        let patchsets = self.control_table("patchsets");
        let changes = self.control_table("changes");
        if let Some(row) = self
            .client
            .query_opt(
                &format!("select p.patchset_id, p.repo_id, p.change_id, p.patchset_number::bigint as patchset_number, p.base_snapshot_id, p.revision_snapshot_id, p.summary, p.author_mode, p.publish_state, p.diff_stats_json, p.evaluation_state, p.created_at::text as created_at from {patchsets} p join {changes} c on c.change_id = p.change_id where c.repo_id = $1 and p.patchset_id = $2"),
                &[&repo_id, &patchset_ref],
            )
            .map_err(|exc| exc.to_string())?
        {
            return patchset_row_json(&row);
        }
        if let Some(number) = repo_scoped_sequence_ref(patchset_ref) {
            let Some(change_ref) = change_ref else {
                return Err(format!(
                    "Patchset ref {patchset_ref} for repository {repo_name} requires change_ref when using a local patchset number"
                ));
            };
            let change = self.get_change_for_repo(repo_name, change_ref)?;
            let change_id = required_text(change.get("change_id"), "change.change_id")?;
            if let Some(row) = self
                .client
                .query_opt(
                    &format!("select patchset_id, repo_id, change_id, patchset_number::bigint as patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at::text as created_at from {patchsets} where change_id = $1 and patchset_number = $2"),
                    &[&change_id, &(number as i32)],
                )
                .map_err(|exc| exc.to_string())?
            {
                return patchset_row_json(&row);
            }
        }
        Err(format!(
            "Unknown patchset {patchset_ref} for repository {repo_name}"
        ))
    }
    pub(super) fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.transaction(|store| {
            let change = store.change_row(change_id)?;
            ensure_change_mutable(&change, "select patchsets")?;
            let patchset = store.get_patchset_in_txn(patchset_id)?;
            if optional_text(patchset.get("change_id")).as_deref() != Some(change_id) {
                return Err(format!("Patchset {patchset_id} does not belong to change {change_id}"));
            }
            let patchset_number = int_value(patchset.get("patchset_number")).unwrap_or(0);
            let patchsets = store.control_table("patchsets");
            store
                .client
                .execute(
                    &format!("update {patchsets} set publish_state = case when patchset_id = $1 then 'selected_for_landing' when publish_state = 'selected_for_landing' then 'published' else publish_state end where change_id = $2"),
                    &[&patchset_id, &change_id],
                )
                .map_err(|exc| exc.to_string())?;
            let now = utc_now();
            let changes = store.control_table("changes");
            store
                .client
                .execute(
                    &format!("update {changes} set selected_patchset_number = $1, updated_at = $2::text::timestamptz where change_id = $3"),
                    &[&(patchset_number as i32), &now, &change_id],
                )
                .map_err(|exc| exc.to_string())?;
            store.record_event(
                "patchset.selected",
                "patchset",
                patchset_id,
                &json!({"change_id": change_id, "patchset_number": patchset_number}),
                &now,
            )?;
            store.refresh_change_state(change_id, &now)?;
            let mut out = store.change_row(change_id)?;
            out.insert("selected_patchset_id".to_string(), json!(patchset_id));
            Ok(out)
        })
    }
    pub(super) fn list_patchsets_for_change(
        &mut self,
        change_id: &str,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let patchsets = self.control_table("patchsets");
        let rows = self
            .client
            .query(
                &format!("select patchset_id, repo_id, change_id, patchset_number::bigint as patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at::text as created_at from {patchsets} where change_id = $1 order by patchset_number desc"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?;
        rows.iter().map(patchset_row_json).collect()
    }
    pub(super) fn get_patchset_in_txn(
        &mut self,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let patchsets = self.control_table("patchsets");
        let row = self
            .client
            .query_opt(
                &format!("select patchset_id, repo_id, change_id, patchset_number::bigint as patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at::text as created_at from {patchsets} where patchset_id = $1"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Unknown patchset: {patchset_id}"))?;
        patchset_row_json(&row)
    }
    pub(super) fn existing_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let patchsets = self.control_table("patchsets");
        self.client
            .query_opt(
                &format!("select patchset_id, repo_id, change_id, patchset_number::bigint as patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at::text as created_at from {patchsets} where change_id = $1 and base_snapshot_id = $2 and revision_snapshot_id = $3 order by patchset_number desc limit 1"),
                &[&change_id, &base_snapshot_id, &revision_snapshot_id],
            )
            .map_err(|exc| exc.to_string())?
            .map(|row| patchset_row_json(&row))
            .transpose()
    }
    pub(super) fn diff_stats(
        &mut self,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let base_map = self.snapshot_manifest_map(base_snapshot_id)?;
        let revision_map = self.snapshot_manifest_map(revision_snapshot_id)?;
        let base_paths = base_map.keys().cloned().collect::<HashSet<_>>();
        let revision_paths = revision_map.keys().cloned().collect::<HashSet<_>>();
        let mut added = revision_paths
            .difference(&base_paths)
            .cloned()
            .collect::<Vec<_>>();
        let mut deleted = base_paths
            .difference(&revision_paths)
            .cloned()
            .collect::<Vec<_>>();
        let mut modified = base_paths
            .intersection(&revision_paths)
            .filter(|path| base_map.get(*path) != revision_map.get(*path))
            .cloned()
            .collect::<Vec<_>>();
        added.sort();
        deleted.sort();
        modified.sort();
        let changed = added.len() + deleted.len() + modified.len();
        Ok(json!({
            "files_added": added.len(),
            "files_deleted": deleted.len(),
            "files_modified": modified.len(),
            "files_changed": changed,
            "paths": {
                "added": added,
                "deleted": deleted,
                "modified": modified,
            }
        })
        .as_object()
        .cloned()
        .unwrap())
    }
    pub(super) fn snapshot_manifest_map(
        &mut self,
        snapshot_id: &str,
    ) -> Result<BTreeMap<String, JsonValue>, String> {
        let rows = self.snapshot_blob_rows(snapshot_id)?;
        let mut out = BTreeMap::new();
        for row in rows {
            let path = required_text(row.get("path"), "snapshot row.path")?;
            let blob_id = required_text(row.get("blob_id"), "snapshot row.blob_id")?;
            let mode = required_text(row.get("mode"), "snapshot row.mode")?;
            let sha256 = required_text(row.get("sha256"), "snapshot row.sha256")?;
            out.insert(
                path,
                json!({
                    "blob_id": blob_id,
                    "size_bytes": row.get("size_bytes").cloned().unwrap_or(JsonValue::Null),
                    "mode": mode,
                    "sha256": sha256,
                }),
            );
        }
        Ok(out)
    }
    pub(super) fn snapshot_blob_rows(
        &mut self,
        snapshot_id: &str,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let root = self
            .root
            .clone()
            .ok_or_else(|| "patchset-store publish-patchset requires server_data/root for snapshot manifest diffing.".to_string())?;
        let snapshots = self.content_table("snapshots");
        let snapshot = self
            .client
            .query_opt(
                &format!("select root_tree_pack_id, root_entry_ordinal::bigint as root_entry_ordinal from {snapshots} where snapshot_id = $1"),
                &[&snapshot_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Unknown snapshot: {snapshot_id}"))?;
        let pack_id = row_text(&snapshot, "root_tree_pack_id")
            .ok_or_else(|| format!("Snapshot {snapshot_id} is missing root tree pack locator metadata; repair the snapshot metadata before reading manifests."))?;
        let ordinal = row_i64(&snapshot, "root_entry_ordinal")
            .ok_or_else(|| format!("Snapshot {snapshot_id} is missing root tree pack locator metadata; repair the snapshot metadata before reading manifests."))?;
        let pack_path = self
            .tree_pack_path_by_id(&pack_id)?
            .ok_or_else(|| format!("Tree pack {pack_id} is missing pack_path metadata."))?;
        let root_payload = read_tree_pack_tree_by_ordinal(
            root.join(&pack_path).to_string_lossy().as_ref(),
            ordinal as usize,
        )?;
        let root_obj = root_payload
            .as_object()
            .ok_or_else(|| "root tree pack payload must be an object".to_string())?;
        let root_tree_id = required_text(root_obj.get("tree_id"), "root tree_id")?;
        let mut tree_cache = BTreeMap::new();
        tree_cache.insert(
            root_tree_id.clone(),
            root_obj
                .get("rows")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        let mut stack = vec![(String::new(), root_tree_id)];
        let mut tree_rows = Vec::new();
        while let Some((prefix, tree_id)) = stack.pop() {
            let rows = if let Some(rows) = tree_cache.get(&tree_id) {
                rows.clone()
            } else {
                let tree_pack_path = self.tree_pack_path_for_tree(&tree_id)?.ok_or_else(|| {
                    format!("Tree {tree_id} is missing tree-pack metadata required for snapshot traversal.")
                })?;
                let payload = read_tree_pack_tree(
                    root.join(&tree_pack_path).to_string_lossy().as_ref(),
                    &tree_id,
                )?;
                let rows = payload.as_array().cloned().unwrap_or_default();
                tree_cache.insert(tree_id.clone(), rows.clone());
                rows
            };
            for row in rows {
                let row_obj = row
                    .as_object()
                    .ok_or_else(|| "tree pack row must be an object".to_string())?;
                let entry_name = optional_text(row_obj.get("entry_name")).unwrap_or_default();
                let next_path = format!("{prefix}{entry_name}");
                let entry_type = optional_text(row_obj.get("entry_type")).unwrap_or_default();
                let target_id = optional_text(row_obj.get("target_id")).unwrap_or_default();
                if entry_type == "blob" {
                    let sha256 = self.blob_sha256(&target_id)?.unwrap_or_default();
                    tree_rows.push(JsonMap::from_iter([
                        ("path".to_string(), json!(next_path)),
                        ("blob_id".to_string(), json!(target_id)),
                        (
                            "size_bytes".to_string(),
                            row_obj
                                .get("size_bytes")
                                .cloned()
                                .unwrap_or(JsonValue::Null),
                        ),
                        (
                            "mode".to_string(),
                            row_obj.get("mode").cloned().unwrap_or(JsonValue::Null),
                        ),
                        ("sha256".to_string(), json!(sha256)),
                    ]));
                } else if entry_type == "tree" && !target_id.is_empty() {
                    stack.push((format!("{next_path}/"), target_id));
                }
            }
        }
        tree_rows.sort_by(|left, right| {
            optional_text(left.get("path"))
                .unwrap_or_default()
                .cmp(&optional_text(right.get("path")).unwrap_or_default())
        });
        Ok(tree_rows)
    }
    pub(super) fn tree_pack_path_by_id(&mut self, pack_id: &str) -> Result<Option<String>, String> {
        let tree_packs = self.content_table("tree_packs");
        self.client
            .query_opt(
                &format!("select pack_path from {tree_packs} where pack_id = $1"),
                &[&pack_id],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "pack_path")))
    }
    pub(super) fn tree_pack_path_for_tree(
        &mut self,
        tree_id: &str,
    ) -> Result<Option<String>, String> {
        let trees = self.content_table("trees");
        let tree_packs = self.content_table("tree_packs");
        self.client
            .query_opt(
                &format!("select tp.pack_path from {trees} t join {tree_packs} tp on tp.pack_id = t.tree_pack_id where t.tree_id = $1 and coalesce(t.tree_pack_id, '') != ''"),
                &[&tree_id],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "pack_path")))
    }
    pub(super) fn blob_sha256(&mut self, blob_id: &str) -> Result<Option<String>, String> {
        let blobs = self.content_table("blobs");
        self.client
            .query_opt(
                &format!("select sha256 from {blobs} where blob_id = $1"),
                &[&blob_id],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "sha256")))
    }
    pub(super) fn current_patchset_for_change(
        &mut self,
        change_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let patchsets = self.control_table("patchsets");
        let row = self
            .client
            .query_opt(
                &format!("select patchset_id, repo_id, change_id, patchset_number::bigint as patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at::text as created_at from {patchsets} where change_id = $1 order by patchset_number desc limit 1"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Change {change_id} has no published patchset"))?;
        patchset_row_json(&row)
    }
    pub(super) fn invalidate_patchset_policy(&mut self, patchset_id: &str) -> Result<(), String> {
        let patchsets = self.control_table("patchsets");
        self.client
            .execute(
                &format!(
                    "update {patchsets} set evaluation_state = 'pending' where patchset_id = $1"
                ),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(())
    }
}
