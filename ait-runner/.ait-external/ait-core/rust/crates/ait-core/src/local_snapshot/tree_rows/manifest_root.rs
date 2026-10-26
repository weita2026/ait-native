use super::*;

pub(crate) fn build_tree_records(
    file_entries: &[SnapshotFileEntry],
) -> Result<(String, Vec<TreeRow>, Vec<TreeEntryRow>), String> {
    let root = build_tree_nodes(file_entries)?;
    let mut tree_rows = BTreeMap::new();
    let mut tree_entry_rows = BTreeMap::new();
    let root_tree_id = materialize_tree(&root, &mut tree_rows, &mut tree_entry_rows)?;
    Ok((
        root_tree_id,
        tree_rows.into_values().collect(),
        tree_entry_rows.into_values().collect(),
    ))
}

pub(crate) fn build_tree_root_id(file_entries: &[SnapshotFileEntry]) -> Result<String, String> {
    materialize_tree_id_only(&build_tree_nodes(file_entries)?)
}

fn build_tree_nodes(
    file_entries: &[SnapshotFileEntry],
) -> Result<BTreeMap<String, TreeNode>, String> {
    let mut root: BTreeMap<String, TreeNode> = BTreeMap::new();
    for entry in file_entries {
        let parts = entry
            .path
            .split('/')
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        let mut cursor = &mut root;
        for part in &parts[..parts.len() - 1] {
            cursor = match cursor
                .entry((*part).to_string())
                .or_insert_with(|| TreeNode::Tree {
                    children: BTreeMap::new(),
                }) {
                TreeNode::Tree { children } => children,
                TreeNode::Blob { .. } => {
                    return Err(format!(
                        "Path collision while building tree metadata at {:?}",
                        entry.path
                    ))
                }
            };
        }
        cursor.insert(
            parts.last().unwrap().to_string(),
            TreeNode::Blob {
                blob_id: entry.blob_id.clone(),
                size_bytes: entry.size_bytes,
                mode: entry.mode.clone(),
            },
        );
    }

    Ok(root)
}

fn materialize_tree_id_only(children: &BTreeMap<String, TreeNode>) -> Result<String, String> {
    let mut serialized_entries = Vec::with_capacity(children.len());
    for (name, node) in children {
        match node {
            TreeNode::Tree { children } => {
                serialized_entries.push(json!({
                    "name": name,
                    "type": "tree",
                    "target_id": materialize_tree_id_only(children)?,
                }));
            }
            TreeNode::Blob {
                blob_id,
                size_bytes,
                mode,
            } => {
                serialized_entries.push(json!({
                    "name": name,
                    "type": "blob",
                    "target_id": blob_id,
                    "size_bytes": size_bytes,
                    "mode": mode,
                }));
            }
        }
    }
    let digest = sha256_hex(
        JsonCodec::encode_serializable(&serialized_entries, JsonEncodeOptions::compact())
            .map_err(|err| err.to_string())?
            .as_bytes(),
    );
    Ok(format!("TRE-{}", digest[..20].to_ascii_uppercase()))
}

pub(in crate::local_snapshot) fn materialize_tree(
    children: &BTreeMap<String, TreeNode>,
    tree_rows: &mut BTreeMap<String, TreeRow>,
    tree_entry_rows: &mut BTreeMap<(String, String), TreeEntryRow>,
) -> Result<String, String> {
    let mut serialized_entries = Vec::new();
    let mut pending_entries = Vec::new();
    for (name, node) in children {
        match node {
            TreeNode::Tree { children } => {
                let child_tree_id = materialize_tree(children, tree_rows, tree_entry_rows)?;
                serialized_entries.push(json!({
                    "name": name,
                    "type": "tree",
                    "target_id": child_tree_id,
                }));
                pending_entries.push(TreeEntryRow {
                    tree_id: String::new(),
                    entry_name: name.clone(),
                    entry_type: "tree".to_string(),
                    target_id: child_tree_id,
                    mode: "tree".to_string(),
                });
            }
            TreeNode::Blob {
                blob_id,
                size_bytes,
                mode,
            } => {
                serialized_entries.push(json!({
                    "name": name,
                    "type": "blob",
                    "target_id": blob_id,
                    "size_bytes": size_bytes,
                    "mode": mode,
                }));
                pending_entries.push(TreeEntryRow {
                    tree_id: String::new(),
                    entry_name: name.clone(),
                    entry_type: "blob".to_string(),
                    target_id: blob_id.clone(),
                    mode: mode.clone(),
                });
            }
        }
    }
    let digest = sha256_hex(
        JsonCodec::encode_serializable(&serialized_entries, JsonEncodeOptions::compact())
            .map_err(|err| err.to_string())?
            .as_bytes(),
    );
    let tree_id = format!("TRE-{}", digest[..20].to_ascii_uppercase());
    tree_rows.entry(tree_id.clone()).or_insert_with(|| TreeRow {
        tree_id: tree_id.clone(),
        entry_count: serialized_entries.len() as i64,
    });
    for mut row in pending_entries {
        row.tree_id = tree_id.clone();
        tree_entry_rows.insert((row.tree_id.clone(), row.entry_name.clone()), row);
    }
    Ok(tree_id)
}

#[cfg(test)]
pub(crate) fn build_snapshot_id(
    repo_name: &str,
    line_name: &str,
    parent_snapshot_id: Option<&str>,
    message: Option<&str>,
    root_tree_id: &str,
) -> (String, String) {
    let parent_snapshot_ids = parent_snapshot_id
        .map(str::to_string)
        .into_iter()
        .collect::<Vec<_>>();
    build_snapshot_id_with_parents(
        repo_name,
        line_name,
        &parent_snapshot_ids,
        message,
        root_tree_id,
    )
}

pub(crate) fn build_snapshot_id_with_parents(
    repo_name: &str,
    line_name: &str,
    parent_snapshot_ids: &[String],
    message: Option<&str>,
    root_tree_id: &str,
) -> (String, String) {
    let payload = if parent_snapshot_ids.len() <= 1 {
        json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "parent_snapshot_id": parent_snapshot_ids.first(),
            "message": message,
            "root_tree_id": root_tree_id,
            "snapshot_kind": "line",
        })
    } else {
        json!({
            "snapshot_manifest_contract": "ait.snapshot.manifest.dag.v2",
            "repo_name": repo_name,
            "line_name": line_name,
            "parent_snapshot_ids": parent_snapshot_ids,
            "primary_parent_snapshot_id": parent_snapshot_ids.first(),
            "message": message,
            "root_tree_id": root_tree_id,
            "snapshot_kind": "line",
        })
    };
    let revision_hash = sha256_hex(
        JsonCodec::encode_value(&payload, JsonEncodeOptions::compact())
            .unwrap_or_default()
            .as_bytes(),
    );
    (
        format!("SNP-{}", revision_hash[..12].to_ascii_uppercase()),
        revision_hash,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_only_materialization_matches_full_tree_records() {
        let entries = vec![
            SnapshotFileEntry {
                path: "alpha.txt".to_string(),
                blob_id: "BLB-ALPHA".to_string(),
                size_bytes: 5,
                mode: "0o644".to_string(),
                sha256: "alpha".to_string(),
                data: b"alpha".to_vec(),
                data_reused: false,
                cache_fingerprint: None,
            },
            SnapshotFileEntry {
                path: "nested/beta.txt".to_string(),
                blob_id: "BLB-BETA".to_string(),
                size_bytes: 4,
                mode: "0o755".to_string(),
                sha256: "beta".to_string(),
                data: b"beta".to_vec(),
                data_reused: false,
                cache_fingerprint: None,
            },
        ];
        let root_only = build_tree_root_id(&entries).unwrap();
        let (full_root, _, _) = build_tree_records(&entries).unwrap();
        assert_eq!(root_only, full_root);
    }

    #[test]
    fn root_and_linear_snapshot_identity_remain_legacy_compatible() {
        let root = build_snapshot_id("repo", "main", None, Some("root"), "TRE-ROOT");
        assert_eq!(
            root,
            build_snapshot_id_with_parents("repo", "main", &[], Some("root"), "TRE-ROOT")
        );
        let linear = build_snapshot_id(
            "repo",
            "main",
            Some("SNP-PARENT"),
            Some("child"),
            "TRE-CHILD",
        );
        assert_eq!(
            linear,
            build_snapshot_id_with_parents(
                "repo",
                "main",
                &["SNP-PARENT".to_string()],
                Some("child"),
                "TRE-CHILD"
            )
        );
    }

    #[test]
    fn multi_parent_snapshot_identity_includes_immutable_parent_order() {
        let left_first = build_snapshot_id_with_parents(
            "repo",
            "main",
            &["SNP-LEFT".to_string(), "SNP-RIGHT".to_string()],
            Some("merge"),
            "TRE-MERGE",
        );
        let right_first = build_snapshot_id_with_parents(
            "repo",
            "main",
            &["SNP-RIGHT".to_string(), "SNP-LEFT".to_string()],
            Some("merge"),
            "TRE-MERGE",
        );
        assert_ne!(left_first, right_first);
        assert_eq!(
            left_first,
            build_snapshot_id_with_parents(
                "repo",
                "main",
                &["SNP-LEFT".to_string(), "SNP-RIGHT".to_string()],
                Some("merge"),
                "TRE-MERGE",
            )
        );
    }
}
