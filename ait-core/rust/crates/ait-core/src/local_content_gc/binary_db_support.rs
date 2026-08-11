use super::*;
use crate::content_binary_db::BinaryDbTreeReadCache;

pub(super) fn binary_db_local_content_storage_stats<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    options: LocalContentStatsOptions,
) -> Result<JsonValue, String> {
    let _stats_range = crate::perfetto_range!("ait.core.gc.stats");
    let read = content.blobs().begin_read_txn();
    let counts = {
        let _range = crate::perfetto_range!("ait.core.gc.stats.blob_counts");
        binary_db_blob_counts(content, &read)?
    };
    let snapshot_views = {
        let _range = crate::perfetto_range!("ait.core.gc.stats.snapshot_views");
        content.snapshots().list_snapshot_views(&read)?
    };
    let snapshot_count = snapshot_views.len() as i64;
    let mut tree_read_cache = BinaryDbTreeReadCache::default();

    let reachability_computed = options.compute_reachability;
    let (reachable, unreachable, tree_stats, tree_reachability_error) = if reachability_computed {
        let _range = crate::perfetto_range!("ait.core.gc.stats.reachability");
        match binary_db_reachable_tree_and_blob_state(
            content,
            &read,
            &snapshot_views,
            &mut tree_read_cache,
        )
        .and_then(|state| {
            let reachable = state.blob_ids.len() as i64;
            let unreachable = (counts.total_blobs - reachable).max(0);
            Ok((
                reachable,
                unreachable,
                binary_db_tree_metadata_stats(content, &read, &state, &mut tree_read_cache)?,
                None,
            ))
        }) {
            Ok(value) => value,
            Err(err) => {
                let fallback = binary_db_fallback_tree_stats(content, &read, &mut tree_read_cache)?;
                (0, counts.total_blobs, fallback, Some(err))
            }
        }
    } else {
        let _range = crate::perfetto_range!("ait.core.gc.stats.tree_metadata");
        (
            0,
            counts.total_blobs,
            binary_db_fallback_tree_stats(content, &read, &mut tree_read_cache)?,
            None,
        )
    };

    let pack_rows = {
        let _range = crate::perfetto_range!("ait.core.gc.stats.object_pack_rows");
        binary_db_pack_rows(content, &read)?
    };
    let tree_pack_rows = {
        let _range = crate::perfetto_range!("ait.core.gc.stats.tree_pack_rows");
        binary_db_tree_pack_rows(content, &read)?
    };
    let pack_count = pack_rows.len();
    let pack_inventory = JsonValue::Array(pack_rows);
    let tree_pack_inventory = JsonValue::Array(tree_pack_rows);
    let repo_root = path_to_string(content.repo_root().as_path())?;
    let (pack_summary, tree_pack_summary) = {
        let _range = crate::perfetto_range!("ait.core.gc.stats.archive_summaries");
        (
            summarize_pack_archives(&repo_root, &pack_inventory)?,
            summarize_tree_pack_archives(&repo_root, &tree_pack_inventory)?,
        )
    };

    let logical_tracked_blob_bytes = counts.packed_blob_bytes;
    let physical_storage_bytes = json_i64(&pack_summary, "pack_archive_bytes")?;
    let storage_savings_bytes = logical_tracked_blob_bytes - physical_storage_bytes;
    let pack_delta_logical_bytes = json_i64(&pack_summary, "pack_delta_logical_bytes")?;
    let pack_delta_member_bytes = json_i64(&pack_summary, "pack_delta_member_bytes")?;
    let delta_pre_archive_savings_bytes = pack_delta_logical_bytes - pack_delta_member_bytes;
    let tracked_blob_count = counts.packed_blob_count;
    let storage_savings_ratio = ratio(storage_savings_bytes, logical_tracked_blob_bytes);
    let delta_pre_archive_savings_ratio =
        ratio(delta_pre_archive_savings_bytes, pack_delta_logical_bytes);
    let pack_member_bytes = json_i64(&pack_summary, "pack_member_bytes")?;
    let archive_compression_savings_bytes =
        pack_member_bytes - json_i64(&pack_summary, "pack_archive_bytes")?;
    let archive_compression_savings_ratio =
        ratio(archive_compression_savings_bytes, pack_member_bytes);

    let validation_summary = if reachability_computed {
        build_storage_validation_summary(
            counts.packed_blob_count as usize,
            counts.packed_full_blob_count as usize,
            counts.packed_delta_blob_count as usize,
            pack_count,
            json_i64(&pack_summary, "index_error_count")? as usize,
            json_i64(&tree_pack_summary, "index_error_count")? as usize,
            storage_savings_ratio,
            unreachable as usize,
            (tree_stats.unreachable_tree_count + tree_stats.orphan_tree_pack_count) as usize,
            None,
        )
    } else {
        json!({
            "state": "reachability_not_computed",
            "needs_attention": null,
            "recommended_action": "run_deep_validation",
            "reasons": ["Default stats skip retained tree-payload traversal."],
            "next_actions": ["ait gc validate", "ait gc stats --deep"],
        })
    };

    let _range = crate::perfetto_range!("ait.core.gc.stats.payload");
    let mut payload = json!({
        "storage_backend": "binary_db",
        "snapshot_count": snapshot_count,
        "reachable_blob_count": reachability_computed.then_some(reachable),
        "unreachable_blob_count": reachability_computed.then_some(unreachable),
        "tree_count": tree_stats.tree_count,
        "tree_entry_count": tree_stats.tree_entry_count,
        "reachable_tree_count": reachability_computed.then_some(tree_stats.reachable_tree_count),
        "reachable_tree_entry_count": reachability_computed.then_some(tree_stats.reachable_tree_entry_count),
        "unreachable_tree_count": reachability_computed.then_some(tree_stats.unreachable_tree_count),
        "unreachable_tree_entry_count": reachability_computed.then_some(tree_stats.unreachable_tree_entry_count),
        "tree_pack_count": tree_stats.tree_pack_count,
        "reachable_tree_pack_count": reachability_computed.then_some(tree_stats.reachable_tree_pack_count),
        "orphan_tree_pack_count": tree_stats.orphan_tree_pack_count,
        "total_blobs": counts.total_blobs,
        "packed_blob_count": counts.packed_blob_count,
        "packed_full_blob_count": counts.packed_full_blob_count,
        "packed_delta_blob_count": counts.packed_delta_blob_count,
        "total_blob_bytes": counts.total_blob_bytes,
        "packed_blob_bytes": counts.packed_blob_bytes,
        "packed_full_blob_bytes": counts.packed_full_blob_bytes,
        "packed_delta_blob_bytes": counts.packed_delta_blob_bytes,
        "pack_count": pack_count,
        "pack_archive_bytes": json_i64(&pack_summary, "pack_archive_bytes")?,
        "tree_pack_archive_bytes": json_i64(&tree_pack_summary, "archive_bytes")?,
        "reachability_summary": {
            "computed": reachability_computed,
            "detail": if reachability_computed {
                "Exact retained-snapshot reachability was computed."
            } else {
                "Skipped retained tree-payload traversal; use `ait gc stats --deep` or `ait gc validate` for exact reachability."
            },
        },
        "schema_cleanup_summary": {
            "storage_backend": "binary_db",
            "skipped": true,
            "reason": "Relational schema cleanup does not apply to Binary DB content metadata.",
        },
        "optimization_summary": {
            "tracked_blob_count": tracked_blob_count,
            "storage_kind_counts": {
                "pack_full": counts.packed_full_blob_count,
                "pack_delta": counts.packed_delta_blob_count,
            },
            "packed_blob_ratio": ratio(counts.packed_blob_count, tracked_blob_count),
            "packed_delta_ratio": ratio(counts.packed_delta_blob_count, tracked_blob_count),
            "delta_within_packed_ratio": ratio(counts.packed_delta_blob_count, counts.packed_blob_count),
        },
        "efficiency_summary": {
            "logical_tracked_blob_bytes": logical_tracked_blob_bytes,
            "physical_storage_bytes": physical_storage_bytes,
            "storage_savings_bytes": storage_savings_bytes,
            "storage_savings_ratio": storage_savings_ratio,
            "pack_archive_bytes": json_i64(&pack_summary, "pack_archive_bytes")?,
            "pack_member_bytes": pack_member_bytes,
            "pack_full_member_bytes": json_i64(&pack_summary, "pack_full_member_bytes")?,
            "pack_delta_member_bytes": pack_delta_member_bytes,
            "pack_member_logical_bytes": json_i64(&pack_summary, "pack_member_logical_bytes")?,
            "pack_delta_logical_bytes": pack_delta_logical_bytes,
            "delta_pre_archive_savings_bytes": delta_pre_archive_savings_bytes,
            "delta_pre_archive_savings_ratio": delta_pre_archive_savings_ratio,
            "archive_compression_savings_bytes": archive_compression_savings_bytes,
            "archive_compression_savings_ratio": archive_compression_savings_ratio,
            "indexed_pack_count": json_i64(&pack_summary, "indexed_pack_count")?,
            "pack_indexed_blob_count": json_i64(&pack_summary, "pack_indexed_blob_count")?,
            "pack_index_error_count": json_i64(&pack_summary, "index_error_count")?,
        },
        "metadata_summary": {
            "tree_count": tree_stats.tree_count,
            "tree_entry_count": tree_stats.tree_entry_count,
            "reachable_tree_count": reachability_computed.then_some(tree_stats.reachable_tree_count),
            "reachable_tree_entry_count": reachability_computed.then_some(tree_stats.reachable_tree_entry_count),
            "unreachable_tree_count": reachability_computed.then_some(tree_stats.unreachable_tree_count),
            "unreachable_tree_entry_count": reachability_computed.then_some(tree_stats.unreachable_tree_entry_count),
            "tree_pack_count": tree_stats.tree_pack_count,
            "reachable_tree_pack_count": reachability_computed.then_some(tree_stats.reachable_tree_pack_count),
            "orphan_tree_pack_count": tree_stats.orphan_tree_pack_count,
            "tree_pack_archive_bytes": json_i64(&tree_pack_summary, "archive_bytes")?,
            "tree_pack_index_error_count": json_i64(&tree_pack_summary, "index_error_count")?,
            "tree_reachability_error": tree_reachability_error,
        },
        "validation_summary": validation_summary,
        "inventory_included": options.include_inventory,
    });
    if options.include_inventory {
        let payload_obj = payload
            .as_object_mut()
            .ok_or_else(|| "GC stats payload must be an object".to_string())?;
        payload_obj.insert("packs".to_string(), pack_inventory);
        payload_obj.insert("tree_packs".to_string(), tree_pack_inventory);
    }
    Ok(payload)
}

pub(super) fn binary_db_blob_counts<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
) -> Result<BlobCounts, String> {
    let mut counts = BlobCounts::default();
    let blob_count = read
        .record_count(BinaryDbBlobStore::<LocalBinaryDbFs, WRITE_LAYOUT>::blob_file())
        .map_err(|err| err.to_string())?;
    for blob_index in 0..blob_count {
        let view = content
            .blobs()
            .blob_view_at(read, blob_index)
            .map_err(|err| err.to_string())?;
        if view.record.is_tombstone() {
            continue;
        }
        let size_bytes = i64_from_u64(view.size_bytes, "blob size_bytes")?;
        counts.total_blobs += 1;
        counts.total_blob_bytes += size_bytes;
        let Some(member_index) = view.record.pack_member_index() else {
            continue;
        };
        let member = content
            .object_packs()
            .object_pack_member_view_at(read, member_index)
            .map_err(|err| err.to_string())?;
        if member.record.is_tombstone() {
            continue;
        }
        counts.packed_blob_count += 1;
        counts.packed_blob_bytes += size_bytes;
        match member.record.member_kind() {
            BinaryObjectPackMemberKind::Delta => {
                counts.packed_delta_blob_count += 1;
                counts.packed_delta_blob_bytes += size_bytes;
            }
            BinaryObjectPackMemberKind::Full | BinaryObjectPackMemberKind::Reserved(_) => {
                counts.packed_full_blob_count += 1;
                counts.packed_full_blob_bytes += size_bytes;
            }
        }
    }
    Ok(counts)
}

pub(super) fn binary_db_reachable_tree_and_blob_state<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    snapshot_views: &[BinarySnapshotView],
    tree_read_cache: &mut BinaryDbTreeReadCache,
) -> Result<ReachableState, String> {
    let parent_map = snapshot_views
        .iter()
        .map(|snapshot| {
            (
                snapshot.snapshot_id.clone(),
                snapshot.parent_snapshot_ids.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    // Reachability currently retains every admitted immutable Snapshot. Still
    // validate the full DAG before walking payloads so a corrupt alternate
    // parent can never be ignored by GC or deep validation.
    topological_snapshot_order(&parent_map, &BTreeSet::new())?;
    let mut state = ReachableState {
        blob_ids: BTreeSet::new(),
        tree_ids: BTreeSet::new(),
    };
    for snapshot in snapshot_views {
        let Some(root_tree_id) = snapshot.root_tree_id.as_deref() else {
            continue;
        };
        binary_db_walk_tree(content, read, root_tree_id, &mut state, tree_read_cache)?;
    }
    Ok(state)
}

pub(super) fn binary_db_walk_tree<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    tree_id: &str,
    state: &mut ReachableState,
    tree_read_cache: &mut BinaryDbTreeReadCache,
) -> Result<(), String> {
    if !state.tree_ids.insert(tree_id.to_string()) {
        return Ok(());
    }
    let entries = content
        .trees()
        .list_tree_entry_views_with_cache(read, tree_id, tree_read_cache)
        .map_err(|err| err.to_string())?;
    tree_read_cache.clear_tree_entries();
    for entry in entries {
        if entry.entry_type == "tree" {
            binary_db_walk_tree(content, read, &entry.target_id, state, tree_read_cache)?;
        } else {
            state.blob_ids.insert(entry.target_id);
        }
    }
    Ok(())
}

pub(super) fn binary_db_tree_metadata_stats<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    reachable: &ReachableState,
    tree_read_cache: &mut BinaryDbTreeReadCache,
) -> Result<TreeStats, String> {
    let tree_rows = binary_db_tree_views(content, read, tree_read_cache)?;
    let tree_count = tree_rows.len() as i64;
    let tree_entry_count = tree_rows
        .iter()
        .map(|tree| i64::from(tree.record.entry_count))
        .sum::<i64>();
    let reachable_tree_count = reachable.tree_ids.len() as i64;
    let reachable_tree_entry_count = tree_rows
        .iter()
        .filter(|tree| reachable.tree_ids.contains(&tree.tree_id))
        .map(|tree| i64::from(tree.record.entry_count))
        .sum::<i64>();
    let referenced_tree_pack_ids = tree_rows
        .iter()
        .filter_map(|tree| tree.tree_pack_id.clone())
        .collect::<BTreeSet<_>>();
    let reachable_tree_pack_count = tree_rows
        .iter()
        .filter(|tree| reachable.tree_ids.contains(&tree.tree_id))
        .filter_map(|tree| tree.tree_pack_id.clone())
        .collect::<BTreeSet<_>>()
        .len() as i64;
    let tree_pack_rows = binary_db_tree_pack_views(content, read)?;
    let orphan_tree_pack_count = tree_pack_rows
        .iter()
        .filter(|pack| !referenced_tree_pack_ids.contains(&pack.pack_id))
        .count() as i64;
    Ok(TreeStats {
        tree_count,
        tree_entry_count,
        reachable_tree_count,
        reachable_tree_entry_count,
        unreachable_tree_count: (tree_count - reachable_tree_count).max(0),
        unreachable_tree_entry_count: (tree_entry_count - reachable_tree_entry_count).max(0),
        tree_pack_count: referenced_tree_pack_ids.len() as i64,
        reachable_tree_pack_count,
        orphan_tree_pack_count,
    })
}

pub(super) fn binary_db_fallback_tree_stats<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    tree_read_cache: &mut BinaryDbTreeReadCache,
) -> Result<TreeStats, String> {
    let tree_rows = binary_db_tree_views(content, read, tree_read_cache)?;
    let tree_count = tree_rows.len() as i64;
    let tree_entry_count = tree_rows
        .iter()
        .map(|tree| i64::from(tree.record.entry_count))
        .sum::<i64>();
    let referenced_tree_pack_ids = tree_rows
        .iter()
        .filter_map(|tree| tree.tree_pack_id.clone())
        .collect::<BTreeSet<_>>();
    let tree_pack_rows = binary_db_tree_pack_views(content, read)?;
    let orphan_tree_pack_count = tree_pack_rows
        .iter()
        .filter(|pack| !referenced_tree_pack_ids.contains(&pack.pack_id))
        .count() as i64;
    Ok(TreeStats {
        tree_count,
        tree_entry_count,
        reachable_tree_count: 0,
        reachable_tree_entry_count: 0,
        unreachable_tree_count: tree_count,
        unreachable_tree_entry_count: tree_entry_count,
        tree_pack_count: referenced_tree_pack_ids.len() as i64,
        reachable_tree_pack_count: 0,
        orphan_tree_pack_count,
    })
}

pub(super) fn binary_db_tree_views<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    tree_read_cache: &mut BinaryDbTreeReadCache,
) -> Result<Vec<crate::content_binary_db::BinaryTreeView>, String> {
    let tree_count = read
        .record_count(BinaryDbTreeStore::<LocalBinaryDbFs, WRITE_LAYOUT>::tree_file())
        .map_err(|err| err.to_string())?;
    let mut trees = Vec::new();
    for tree_index in 0..tree_count {
        let view = content
            .trees()
            .tree_view_at_with_cache(read, tree_index, tree_read_cache)
            .map_err(|err| err.to_string())?;
        if !view.record.is_tombstone() {
            trees.push(view);
        }
    }
    Ok(trees)
}

pub(super) fn binary_db_tree_pack_views<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
) -> Result<Vec<crate::content_binary_db::BinaryTreePackView>, String> {
    let tree_pack_count = read
        .record_count(BinaryDbTreePackStore::<LocalBinaryDbFs, WRITE_LAYOUT>::tree_pack_file())
        .map_err(|err| err.to_string())?;
    let mut packs = Vec::new();
    for tree_pack_index in 0..tree_pack_count {
        let view = content
            .tree_packs()
            .tree_pack_view_at(read, tree_pack_index)
            .map_err(|err| err.to_string())?;
        if !view.record.is_tombstone() {
            packs.push(view);
        }
    }
    Ok(packs)
}

pub(super) fn binary_db_pack_rows<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
) -> Result<Vec<JsonValue>, String> {
    let mut rows = Vec::new();
    for pack in content
        .object_packs()
        .list_object_pack_views(read)
        .map_err(|err| err.to_string())?
    {
        if pack.record.is_tombstone() {
            continue;
        }
        rows.push(json!({
            "pack_id": pack.pack_id,
            "status": if pack.record.is_ready() { "ready" } else { "pending" },
            "member_count": i64::from(pack.record.member_count),
            "total_bytes": i64_from_u64(pack.record.total_bytes, "object pack total_bytes")?,
            "pack_path": pack.pack_path,
            "pack_format": pack.pack_format,
            "created_at": i64_from_u64(pack.record.created_at_s, "object pack created_at_s")?,
        }));
    }
    Ok(rows)
}

pub(super) fn binary_db_tree_pack_rows<const WRITE_LAYOUT: u32>(
    content: &LocalContentBinaryDb<WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
) -> Result<Vec<JsonValue>, String> {
    let mut rows = Vec::new();
    for pack in binary_db_tree_pack_views(content, read)? {
        rows.push(json!({
            "pack_id": pack.pack_id,
            "status": if pack.record.is_ready() { "ready" } else { "pending" },
            "tree_count": i64::from(pack.record.tree_count),
            "total_bytes": i64_from_u64(pack.record.total_bytes, "tree pack total_bytes")?,
            "pack_path": pack.pack_path,
            "pack_format": pack.pack_format,
            "created_at": i64_from_u64(pack.record.created_at_s, "tree pack created_at_s")?,
        }));
    }
    Ok(rows)
}

pub(super) fn i64_from_u64(value: u64, label: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{label} overflows i64: {value}"))
}
