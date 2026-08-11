use super::*;

impl ConvertedZstdInventory {
    pub fn validate_conversion_contract(&self) -> Result<ConvertedZstdVerificationSummary, String> {
        self.inventory.validate_zstd_only()?;

        let snapshot_order =
            repository_snapshot_parent_topological_order(&self.inventory.snapshots)?;
        validate_snapshot_conversion_order(&self.snapshot_conversion_order, &snapshot_order)?;

        let snapshot_path_blobs = validate_snapshot_path_blobs(
            &self.inventory.snapshots,
            &self.inventory.blob_locators,
            &self.snapshot_path_blobs,
        )?;
        validate_delta_blob_bases(
            &self.inventory.snapshots,
            &self.inventory.blob_locators,
            &snapshot_path_blobs,
        )?;
        let unreachable_packed_blob_count = validate_source_packed_blob_retention(
            &self.inventory.object_packs,
            &self.inventory.blob_locators,
            &snapshot_path_blobs,
            &self.source_packed_blob_ids,
            &self.orphan_object_pack_ids,
        )?;

        Ok(ConvertedZstdVerificationSummary {
            snapshot_order,
            source_packed_blob_count: normalized_unique_texts(&self.source_packed_blob_ids).len(),
            unreachable_packed_blob_count,
            orphan_pack_count: normalized_unique_texts(&self.orphan_object_pack_ids).len(),
        })
    }
}

pub fn repository_snapshot_parent_topological_order(
    snapshots: &[RepositorySnapshotInventoryRow],
) -> Result<Vec<String>, String> {
    let mut by_id = BTreeMap::new();
    for snapshot in snapshots {
        require_non_empty(&snapshot.snapshot_id, "snapshot id")?;
        crate::snapshot_store::validate_snapshot_parent_set(
            Some(&snapshot.snapshot_id),
            &snapshot.parent_snapshot_ids,
            snapshot.primary_parent_snapshot_id.as_deref(),
            snapshot.parent_snapshot_id.as_deref(),
        )?;
        if by_id
            .insert(snapshot.snapshot_id.clone(), snapshot)
            .is_some()
        {
            return Err(format!("Duplicate snapshot id {}.", snapshot.snapshot_id));
        }
    }

    let mut pending = snapshots.iter().collect::<Vec<_>>();
    pending.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.snapshot_id.cmp(&right.snapshot_id))
    });

    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::with_capacity(snapshots.len());
    while !pending.is_empty() {
        let mut next_pending = Vec::new();
        let mut progressed = false;
        for snapshot in pending {
            let parents_ready = snapshot
                .parent_snapshot_ids
                .iter()
                .all(|parent_id| emitted.contains(parent_id) || !by_id.contains_key(parent_id));
            if parents_ready {
                emitted.insert(snapshot.snapshot_id.clone());
                ordered.push(snapshot.snapshot_id.clone());
                progressed = true;
            } else {
                next_pending.push(snapshot);
            }
        }
        if !progressed {
            let blocked = next_pending
                .iter()
                .map(|snapshot| snapshot.snapshot_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Snapshot parent chain is cyclic or blocked before conversion order can be built: {blocked}."
            ));
        }
        pending = next_pending;
    }

    Ok(ordered)
}

pub(super) fn validate_snapshot_conversion_order(
    actual: &[String],
    expected: &[String],
) -> Result<(), String> {
    if actual.is_empty() && !expected.is_empty() {
        return Err("Converted zstd inventory requires snapshot_conversion_order.".to_string());
    }
    let actual_normalized = normalized_unique_texts(actual);
    if actual_normalized.len() != actual.len() {
        return Err("Converted zstd snapshot_conversion_order contains duplicates.".to_string());
    }
    if actual_normalized != expected.iter().cloned().collect::<BTreeSet<_>>() {
        return Err(
            "Converted zstd snapshot_conversion_order must contain every inventory snapshot exactly once."
                .to_string(),
        );
    }
    if actual != expected {
        return Err(format!(
            "Converted zstd snapshot_conversion_order must be parent-topological with created_at tiebreak: expected {}.",
            expected.join(", ")
        ));
    }
    Ok(())
}

pub(super) fn validate_snapshot_path_blobs(
    snapshots: &[RepositorySnapshotInventoryRow],
    blob_locators: &[RepositoryBlobLocatorInventoryRow],
    rows: &[RepositorySnapshotPathBlobInventoryRow],
) -> Result<BTreeMap<(String, String), String>, String> {
    let snapshot_ids = snapshots
        .iter()
        .map(|snapshot| snapshot.snapshot_id.as_str())
        .collect::<BTreeSet<_>>();
    let blob_ids = blob_locators
        .iter()
        .map(|locator| locator.blob_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut snapshot_path_blobs = BTreeMap::new();
    for row in rows {
        require_non_empty(&row.snapshot_id, "snapshot path blob snapshot id")?;
        require_non_empty(&row.path, "snapshot path blob path")?;
        require_non_empty(&row.blob_id, "snapshot path blob blob id")?;
        if !snapshot_ids.contains(row.snapshot_id.as_str()) {
            return Err(format!(
                "Snapshot path blob row references unknown snapshot {}.",
                row.snapshot_id
            ));
        }
        if !blob_ids.contains(row.blob_id.as_str()) {
            return Err(format!(
                "Snapshot path blob row references unknown blob {}.",
                row.blob_id
            ));
        }
        let key = (row.snapshot_id.clone(), row.path.clone());
        if snapshot_path_blobs
            .insert(key, row.blob_id.clone())
            .is_some()
        {
            return Err(format!(
                "Duplicate snapshot path blob row for snapshot {} path {}.",
                row.snapshot_id, row.path
            ));
        }
    }
    Ok(snapshot_path_blobs)
}

pub(super) fn validate_delta_blob_bases(
    snapshots: &[RepositorySnapshotInventoryRow],
    blob_locators: &[RepositoryBlobLocatorInventoryRow],
    snapshot_path_blobs: &BTreeMap<(String, String), String>,
) -> Result<(), String> {
    let snapshots_by_id = snapshots
        .iter()
        .map(|snapshot| (snapshot.snapshot_id.as_str(), snapshot))
        .collect::<BTreeMap<_, _>>();
    let locators_by_blob_id = blob_locators
        .iter()
        .map(|locator| (locator.blob_id.as_str(), locator))
        .collect::<BTreeMap<_, _>>();
    let mut paths_by_blob_id: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for ((snapshot_id, path), blob_id) in snapshot_path_blobs {
        paths_by_blob_id
            .entry(blob_id.as_str())
            .or_default()
            .push((snapshot_id.as_str(), path.as_str()));
    }

    for locator in blob_locators {
        let entry_type =
            require_non_empty(&locator.pack_entry_type, "blob locator pack entry type")?;
        if locator.pack_chain_depth as usize > DEFAULT_MAX_DELTA_CHAIN_DEPTH {
            return Err(format!(
                "Blob {} delta chain depth {} exceeds DEFAULT_MAX_DELTA_CHAIN_DEPTH {}.",
                locator.blob_id, locator.pack_chain_depth, DEFAULT_MAX_DELTA_CHAIN_DEPTH
            ));
        }
        match entry_type {
            "full" => {
                if locator.pack_base_blob_id.is_some() || locator.pack_chain_depth != 0 {
                    return Err(format!(
                        "Full blob {} must not carry delta base metadata.",
                        locator.blob_id
                    ));
                }
            }
            "delta" => {
                let base_blob_id = locator
                    .pack_base_blob_id
                    .as_deref()
                    .and_then(normalize_optional_text)
                    .ok_or_else(|| {
                        format!("Delta blob {} requires a base blob id.", locator.blob_id)
                    })?;
                if locator.pack_chain_depth <= 0 {
                    return Err(format!(
                        "Delta blob {} requires positive chain depth.",
                        locator.blob_id
                    ));
                }
                let base_locator = locators_by_blob_id.get(base_blob_id).ok_or_else(|| {
                    format!(
                        "Delta blob {} references unknown base blob {}.",
                        locator.blob_id, base_blob_id
                    )
                })?;
                if locator.pack_chain_depth != base_locator.pack_chain_depth + 1 {
                    return Err(format!(
                        "Delta blob {} chain depth must equal base chain depth plus one.",
                        locator.blob_id
                    ));
                }
                let mut parent_same_path_match = false;
                for (snapshot_id, path) in paths_by_blob_id
                    .get(locator.blob_id.as_str())
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                {
                    let Some(snapshot) = snapshots_by_id.get(snapshot_id) else {
                        continue;
                    };
                    let Some(parent_id) =
                        normalized_option_text(snapshot.primary_parent_snapshot_id.as_deref())
                    else {
                        continue;
                    };
                    if snapshot_path_blobs
                        .get(&(parent_id.to_string(), (*path).to_string()))
                        .is_some_and(|parent_blob_id| parent_blob_id == base_blob_id)
                    {
                        parent_same_path_match = true;
                        break;
                    }
                }
                if !parent_same_path_match {
                    return Err(format!(
                        "Delta blob {} must use the parent snapshot same-path blob {} as its base.",
                        locator.blob_id, base_blob_id
                    ));
                }
            }
            other => {
                return Err(format!(
                    "Blob {} has unsupported pack entry type {}.",
                    locator.blob_id, other
                ));
            }
        }
    }

    Ok(())
}

pub(super) fn validate_source_packed_blob_retention(
    object_packs: &[RepositoryObjectPackInventoryRow],
    blob_locators: &[RepositoryBlobLocatorInventoryRow],
    snapshot_path_blobs: &BTreeMap<(String, String), String>,
    source_packed_blob_ids: &[String],
    orphan_object_pack_ids: &[String],
) -> Result<usize, String> {
    let source_blob_ids = normalized_unique_texts(source_packed_blob_ids);
    let orphan_pack_ids = normalized_unique_texts(orphan_object_pack_ids);
    let object_pack_ids = object_packs
        .iter()
        .map(|pack| pack.pack_id.as_str())
        .collect::<BTreeSet<_>>();
    for orphan_pack_id in &orphan_pack_ids {
        if !object_pack_ids.contains(orphan_pack_id.as_str()) {
            return Err(format!(
                "Converted zstd orphan object pack {} is not in inventory.",
                orphan_pack_id
            ));
        }
    }

    let reachable_blob_ids = snapshot_path_blobs
        .values()
        .map(|blob_id| blob_id.as_str())
        .collect::<BTreeSet<_>>();
    let locators_by_blob_id = blob_locators
        .iter()
        .map(|locator| (locator.blob_id.as_str(), locator))
        .collect::<BTreeMap<_, _>>();

    let mut unreachable_packed_blob_count = 0;
    for source_blob_id in &source_blob_ids {
        let locator = locators_by_blob_id
            .get(source_blob_id.as_str())
            .ok_or_else(|| {
                format!(
                    "Converted zstd inventory dropped source packed blob {}.",
                    source_blob_id
                )
            })?;
        if !reachable_blob_ids.contains(source_blob_id.as_str()) {
            unreachable_packed_blob_count += 1;
            if !orphan_pack_ids.contains(&locator.pack_id) {
                return Err(format!(
                    "Unreachable source packed blob {} must be retained in an orphan zstd object pack.",
                    source_blob_id
                ));
            }
        }
    }

    Ok(unreachable_packed_blob_count)
}

pub(super) fn normalized_unique_texts(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .filter_map(|value| normalize_optional_text(value).map(ToOwned::to_owned))
        .collect()
}

pub(super) fn normalized_option_text(value: Option<&str>) -> Option<&str> {
    value.and_then(normalize_optional_text)
}
