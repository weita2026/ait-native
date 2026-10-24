use super::*;

impl<const WRITE_LAYOUT: u32> LocalContentBinaryDb<WRITE_LAYOUT> {
    pub(super) fn tree_pack_locator_for_tree(
        &self,
        tree_id: &str,
    ) -> Result<Option<(String, i64)>, String> {
        let read = self.trees.begin_read_txn();
        let Some(tree) = self.trees.get_tree_view(&read, tree_id)? else {
            return Ok(None);
        };
        let Some(tree_pack_id) = tree
            .tree_pack_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            return Ok(None);
        };
        let tree_pack = self
            .tree_packs
            .get_tree_pack_view(&read, &tree_pack_id)?
            .ok_or_else(|| format!("Binary DB tree pack {tree_pack_id} is missing."))?;
        let ordinal = tree
            .tree_index
            .checked_sub(tree_pack.record.first_tree_index)
            .ok_or_else(|| format!("Binary DB tree {tree_id} is outside {tree_pack_id}."))?;
        Ok(Some((tree_pack_id, i64::from(ordinal))))
    }
}

pub(super) fn object_pack_member_inputs(
    file_entries: &[SnapshotFileEntry],
    members: &[ObjectPackWriteMember],
    created_at: &str,
) -> Result<Vec<BinaryDbObjectPackMemberWriteInput>, String> {
    let members_by_blob_id = members
        .iter()
        .map(|member| (member.blob_id.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for entry in file_entries {
        if !seen.insert(entry.blob_id.clone()) {
            continue;
        }
        let Some(member) = members_by_blob_id.get(entry.blob_id.as_str()) else {
            continue;
        };
        out.push(BinaryDbObjectPackMemberWriteInput {
            blob_id: entry.blob_id.clone(),
            sha256: entry.sha256.clone(),
            size_bytes: entry.size_bytes,
            pack_entry_type: member.entry_type.clone(),
            pack_base_blob_id: member.base_blob_id.clone(),
            pack_chain_depth: i64::try_from(member.chain_depth).map_err(|_| {
                format!(
                    "Object pack member {} chain depth exceeds i64.",
                    member.blob_id
                )
            })?,
            created_at: created_at.to_string(),
        });
    }
    Ok(out)
}

pub(super) fn tree_rows_json(rows: &[TreeRow]) -> JsonValue {
    JsonValue::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "tree_id": row.tree_id,
                    "entry_count": row.entry_count,
                })
            })
            .collect(),
    )
}

pub(super) fn tree_entry_rows_json(
    rows: &[TreeEntryRow],
    file_entries: &[SnapshotFileEntry],
) -> JsonValue {
    let size_by_blob_id = file_entries
        .iter()
        .map(|entry| (entry.blob_id.clone(), entry.size_bytes))
        .collect::<BTreeMap<_, _>>();
    JsonValue::Array(
        rows.iter()
            .map(|row| {
                let mut payload = json!({
                    "tree_id": row.tree_id,
                    "entry_name": row.entry_name,
                    "entry_type": row.entry_type,
                    "target_id": row.target_id,
                    "mode": row.mode,
                });
                if row.entry_type == "blob" {
                    if let Some(size_bytes) = size_by_blob_id.get(&row.target_id) {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert("size_bytes".to_string(), json!(*size_bytes));
                        }
                    }
                }
                payload
            })
            .collect(),
    )
}
