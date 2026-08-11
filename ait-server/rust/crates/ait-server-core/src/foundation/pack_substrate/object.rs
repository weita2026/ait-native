use super::*;

pub fn build_pack_members(
    blob_items: &JsonValue,
    max_delta_chain_depth: usize,
    initial_by_path: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    let candidates = parse_pack_candidates(blob_items)?;
    let members = build_pack_members_from_candidates(
        candidates,
        max_delta_chain_depth,
        parse_initial_by_path(initial_by_path)?,
    );
    Ok(JsonValue::Array(
        members.iter().map(member_to_json).collect(),
    ))
}

fn build_pack_members_from_candidates(
    candidates: Vec<PackCandidate>,
    max_delta_chain_depth: usize,
    mut latest_by_path: BTreeMap<String, PackCandidate>,
) -> Vec<PackMember> {
    let mut members = Vec::new();

    for item in candidates {
        let mut member = PackMember {
            entry_name: item.entry_name.clone(),
            blob_id: item.blob_id.clone(),
            data: item.data.clone(),
            logical_data: item.data.clone(),
            entry_type: "full".to_string(),
            base_blob_id: None,
            chain_depth: 0,
            delta_algorithm: None,
        };

        let base_candidate = item
            .path_hint
            .as_ref()
            .and_then(|path| latest_by_path.get(path))
            .cloned();
        if let Some(base) = base_candidate {
            if base.blob_id != item.blob_id
                && base.chain_depth < max_delta_chain_depth
                && (MIN_DELTA_BLOB_BYTES..=MAX_DELTA_BLOB_BYTES).contains(&item.data.len())
                && (MIN_DELTA_BLOB_BYTES..=MAX_DELTA_BLOB_BYTES).contains(&base.data.len())
            {
                let delta_data = build_git_binary_delta(&base.data, &item.data);
                if delta_data.len() + MIN_DELTA_SAVINGS_BYTES < item.data.len() {
                    member = PackMember {
                        entry_name: item.entry_name.clone(),
                        blob_id: item.blob_id.clone(),
                        data: delta_data,
                        logical_data: item.data.clone(),
                        entry_type: "delta".to_string(),
                        base_blob_id: Some(base.blob_id.clone()),
                        chain_depth: base.chain_depth + 1,
                        delta_algorithm: Some(PACK_DELTA_GIT_BINARY_V1.to_string()),
                    };
                }
            }
        }
        if let Some(path_hint) = item.path_hint {
            latest_by_path.insert(
                path_hint,
                PackCandidate {
                    entry_name: item.entry_name.clone(),
                    blob_id: item.blob_id.clone(),
                    data: item.data.clone(),
                    path_hint: None,
                    chain_depth: member.chain_depth,
                },
            );
        }
        members.push(member);
    }

    members
}

/// Rebuilds a zstd object pack directly from logical blob bytes. This is the
/// bounded-memory path for migrations and GC: unlike `build_pack_members`, it
/// does not materialize byte arrays as `serde_json::Value` nodes.
pub fn write_rebuilt_zstd_pack_archive(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    blobs: Vec<ObjectPackRewriteBlob>,
    max_delta_chain_depth: usize,
) -> Result<JsonValue, String> {
    let candidates = blobs
        .into_iter()
        .map(|blob| PackCandidate {
            entry_name: blob.entry_name,
            blob_id: blob.blob_id,
            data: blob.data,
            path_hint: blob.path_hint,
            chain_depth: 0,
        })
        .collect();
    let members =
        build_pack_members_from_candidates(candidates, max_delta_chain_depth, BTreeMap::new());
    write_zstd_chunked_pack_archive_from_members(pack_path, pack_id, created_at, &members)
}

pub fn read_zstd_object_pack_blob_from_bytes(
    pack_bytes: &[u8],
    blob_id: &str,
    resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
    max_chain_depth: usize,
) -> Result<Vec<u8>, String> {
    let pack_index = read_zstd_chunked_container_index_from_bytes(
        pack_bytes,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    let entry_name = pack_index
        .members
        .iter()
        .find(|member| member.content_id == blob_id)
        .map(|member| member.entry_name.clone())
        .ok_or_else(|| format!("missing pack blob: {blob_id}"))?;
    read_zstd_chunked_object_entry_from_bytes(
        pack_bytes,
        &pack_index,
        &entry_name,
        resolve_base_blob_map,
        max_chain_depth,
        &mut BTreeSet::new(),
        0,
    )
}

pub fn object_pack_backend(
    format_kind: PackFormatKind,
) -> Result<&'static dyn ObjectPackBackend, String> {
    match format_kind {
        PackFormatKind::ZstdChunkedV1 => Ok(&ZSTD_CHUNKED_OBJECT_PACK_BACKEND),
    }
}

pub fn object_pack_backend_from_persisted_format(
    persisted_pack_format: &str,
) -> Result<&'static dyn ObjectPackBackend, String> {
    object_pack_backend(PackFormatKind::from_persisted(persisted_pack_format)?)
}

pub fn write_pack_archive(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    members: &JsonValue,
) -> Result<JsonValue, String> {
    write_pack_archive_with_format(
        pack_path,
        pack_id,
        created_at,
        members,
        DEFAULT_OBJECT_PACK_WRITE_FORMAT,
    )
}

pub fn write_pack_archive_with_format(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    members: &JsonValue,
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    object_pack_backend_from_persisted_format(persisted_pack_format)?
        .write_pack_archive(pack_path, pack_id, created_at, members)
}

pub(super) fn write_zstd_chunked_pack_archive(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    members: &JsonValue,
) -> Result<JsonValue, String> {
    let parsed = parse_pack_members(members)?;
    write_zstd_chunked_pack_archive_from_members(pack_path, pack_id, created_at, &parsed)
}

fn write_zstd_chunked_pack_archive_from_members(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    parsed: &[PackMember],
) -> Result<JsonValue, String> {
    let inputs = parsed
        .iter()
        .enumerate()
        .map(|(ordinal, member)| ZstdChunkedMemberInput {
            member_ordinal: ordinal,
            entry_name: member.entry_name.clone(),
            content_id: member.blob_id.clone(),
            entry_type: member.entry_type.clone(),
            entry_count: None,
            base_content_id: member.base_blob_id.clone(),
            delta_algorithm: member.delta_algorithm.clone(),
            chain_depth: member.chain_depth,
            data: member.data.clone(),
            logical_len: member.logical_data.len(),
            checksum: sha256_hex(&member.logical_data),
        })
        .collect::<Vec<_>>();
    let pack_index = write_zstd_chunked_container(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
        ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME,
        pack_id,
        created_at,
        &inputs,
        zstd_chunked_chunk_bytes_from_env("AIT_OBJECT_PACK_CHUNK_MIB"),
    )?;
    let pack_index_json = ObjectPackIndexJson::stateless().zstd_chunked_index_json(&pack_index)?;
    let index_bytes = encode_zstd_chunked_index(&pack_index, ZSTD_CHUNKED_INDEX_KIND_OBJECT)?;
    let index_checksum = sha256_hex(&index_bytes);
    let archive_bytes = std::fs::metadata(pack_path)
        .map_err(|err| format!("failed to stat zstd pack archive: {err}"))?
        .len();
    Ok(json!({
        "member_count": pack_index_json.get("member_count").cloned().unwrap_or(JsonValue::Number(Number::from(0))),
        "total_bytes": pack_index_json.get("total_bytes").cloned().unwrap_or(JsonValue::Number(Number::from(0))),
        "archive_bytes": archive_bytes,
        "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
        "pack_index_entry_name": ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME,
        "pack_index_checksum": index_checksum,
        "pack_index": pack_index_json,
    }))
}

pub fn read_pack_index(pack_path: &str) -> Result<JsonValue, String> {
    read_pack_index_with_format(pack_path, DEFAULT_OBJECT_PACK_WRITE_FORMAT)
}

pub fn read_pack_index_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    object_pack_backend_from_persisted_format(persisted_pack_format)?.read_pack_index(pack_path)
}

pub fn read_pack_index_checksum_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<String, String> {
    PackFormatKind::from_persisted(persisted_pack_format)?;
    let pack_index = read_zstd_chunked_container_index(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    let index_bytes = encode_zstd_chunked_index(&pack_index, ZSTD_CHUNKED_INDEX_KIND_OBJECT)?;
    Ok(sha256_hex(&index_bytes))
}

pub(super) fn read_zstd_chunked_pack_index(pack_path: &str) -> Result<JsonValue, String> {
    let pack_index = read_zstd_chunked_container_index(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    ObjectPackIndexJson::stateless().zstd_chunked_index_json(&pack_index)
}

pub fn summarize_pack_archives(
    pack_root: &str,
    pack_rows: &JsonValue,
) -> Result<JsonValue, String> {
    let rows = as_array(pack_rows, "pack_rows")?;
    let root = PathBuf::from(pack_root);
    let mut summary = json!({
        "pack_archive_bytes": 0,
        "indexed_pack_count": 0,
        "index_error_count": 0,
        "pack_indexed_blob_count": 0,
        "pack_member_bytes": 0,
        "pack_full_member_bytes": 0,
        "pack_delta_member_bytes": 0,
        "pack_member_logical_bytes": 0,
        "pack_full_logical_bytes": 0,
        "pack_delta_logical_bytes": 0,
    });
    let Some(summary_obj) = summary.as_object_mut() else {
        return Err("summary payload must be an object".to_string());
    };
    for row in rows {
        let row_obj = as_object(row, "pack row")?;
        let Some(pack_path) = optional_text_field(row_obj, "pack_path") else {
            continue;
        };
        let Some(pack_format) = optional_text_field(row_obj, "pack_format") else {
            increment_u64(summary_obj, "index_error_count", 1);
            continue;
        };
        let pack_abs = root.join(pack_path);
        if !pack_abs.exists() {
            continue;
        }
        increment_u64(
            summary_obj,
            "pack_archive_bytes",
            pack_abs.metadata().map_err(|err| err.to_string())?.len(),
        );
        let pack_index =
            match read_pack_index_with_format(path_to_string(&pack_abs)?.as_str(), &pack_format) {
                Ok(value) => value,
                Err(_) => {
                    increment_u64(summary_obj, "index_error_count", 1);
                    continue;
                }
            };
        increment_u64(summary_obj, "indexed_pack_count", 1);
        let entries = as_array(&pack_index["entries"], "entries")?;
        increment_u64(summary_obj, "pack_indexed_blob_count", entries.len() as u64);
        for entry in entries {
            let entry_obj = as_object(entry, "pack index entry")?;
            let member_bytes =
                u64::try_from(required_usize_field(entry_obj, "byte_length")?).unwrap_or(0);
            let logical_bytes = u64::try_from(
                optional_usize_field(entry_obj, "uncompressed_byte_length")?
                    .unwrap_or(member_bytes as usize),
            )
            .unwrap_or(member_bytes);
            let entry_type = required_text_field(entry_obj, "entry_type")?;
            increment_u64(summary_obj, "pack_member_bytes", member_bytes);
            increment_u64(summary_obj, "pack_member_logical_bytes", logical_bytes);
            if entry_type == "delta" {
                increment_u64(summary_obj, "pack_delta_member_bytes", member_bytes);
                increment_u64(summary_obj, "pack_delta_logical_bytes", logical_bytes);
            } else {
                increment_u64(summary_obj, "pack_full_member_bytes", member_bytes);
                increment_u64(summary_obj, "pack_full_logical_bytes", logical_bytes);
            }
        }
    }
    Ok(summary)
}

pub fn build_storage_validation_summary(
    packed_blob_count: usize,
    packed_full_blob_count: usize,
    packed_delta_blob_count: usize,
    pack_count: usize,
    pack_index_error_count: usize,
    tree_pack_index_error_count: usize,
    storage_savings_ratio: f64,
    unreferenced_blob_count: usize,
    unreferenced_tree_count: usize,
    signals_summary: Option<&JsonValue>,
) -> JsonValue {
    let mut issues = Vec::<String>::new();
    let mut reasons = Vec::<String>::new();
    let mut next_actions = Vec::<String>::new();
    let mut recommended_action = "none".to_string();
    let state: String;
    let mut needs_attention = false;

    let drift_count = signals_summary
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("drift_count"))
        .and_then(JsonValue::as_u64)
        .unwrap_or(0) as usize;
    let repairable_drift_count = signals_summary
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("repairable_drift_count"))
        .and_then(JsonValue::as_u64)
        .unwrap_or(0) as usize;
    let nonrepairable_drift_count = drift_count.saturating_sub(repairable_drift_count);

    if pack_index_error_count > 0 {
        issues.push("pack_index_errors".to_string());
    }
    if tree_pack_index_error_count > 0 {
        issues.push("tree_pack_index_errors".to_string());
    }
    if drift_count > 0 {
        issues.push("storage_drift".to_string());
    }
    if !issues.is_empty() {
        state = "attention_required".to_string();
        needs_attention = true;
        if repairable_drift_count > 0
            && pack_index_error_count == 0
            && tree_pack_index_error_count == 0
            && nonrepairable_drift_count == 0
        {
            recommended_action = "optimize".to_string();
            next_actions.push("optimize".to_string());
            reasons.push("repairable storage drift is present".to_string());
        } else {
            recommended_action = "inspect".to_string();
            next_actions.push("inspect".to_string());
            reasons.push("storage metadata requires operator attention".to_string());
        }
        if pack_index_error_count > 0 {
            reasons.push("one or more pack indexes could not be read".to_string());
        }
        if tree_pack_index_error_count > 0 {
            reasons.push("one or more tree pack indexes could not be read".to_string());
        }
        return json!({
            "state": state,
            "recommended_action": recommended_action,
            "next_actions": next_actions,
            "issues": issues,
            "reasons": reasons,
            "needs_attention": needs_attention,
            "has_pack_optimization": packed_blob_count > 0,
            "has_delta_optimization": packed_delta_blob_count > 0,
            "storage_savings_ratio": storage_savings_ratio,
        });
    }

    if packed_blob_count == 0 {
        state = "unoptimized".to_string();
        reasons.push("no packed blob payloads are currently tracked".to_string());
    } else if packed_delta_blob_count == 0 && packed_full_blob_count > 0 {
        state = "packed_full_only".to_string();
        next_actions.push("repack".to_string());
        reasons.push("pack data exists but no delta entries have been produced yet".to_string());
        if unreferenced_tree_count > 0 {
            if !next_actions.iter().any(|action| action == "gc") {
                next_actions.push("gc".to_string());
            }
            reasons.push("unreachable tree metadata remains to be cleaned up".to_string());
        }
    } else if pack_count > 1 || unreferenced_blob_count > 0 || unreferenced_tree_count > 0 {
        state = "partially_optimized".to_string();
        if pack_count > 1 {
            next_actions.push("repack".to_string());
            reasons.push(
                "multiple live packs remain and repack can still consolidate layout".to_string(),
            );
            next_actions.push("gc".to_string());
            reasons.push("old pack archives should be cleaned after repack".to_string());
        } else {
            next_actions.push("gc".to_string());
        }
        if unreferenced_blob_count > 0 {
            if !next_actions.iter().any(|action| action == "gc") {
                next_actions.push("gc".to_string());
            }
            reasons.push("unreferenced blob payloads remain to be cleaned up".to_string());
        }
        if unreferenced_tree_count > 0 {
            if !next_actions.iter().any(|action| action == "gc") {
                next_actions.push("gc".to_string());
            }
            reasons.push("unreachable tree metadata remains to be cleaned up".to_string());
        }
    } else {
        state = "delta_optimized".to_string();
        reasons.push("delta-capable packing is active and no cleanup signals remain".to_string());
    }

    if let Some(action) = next_actions.first() {
        recommended_action = action.clone();
    }

    json!({
        "state": state,
        "recommended_action": recommended_action,
        "next_actions": next_actions,
        "issues": issues,
        "reasons": reasons,
        "needs_attention": needs_attention,
        "has_pack_optimization": packed_blob_count > 0,
        "has_delta_optimization": packed_delta_blob_count > 0,
        "storage_savings_ratio": storage_savings_ratio,
    })
}

pub fn read_pack_entry(
    pack_path: &str,
    entry_name: &str,
    resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
    max_chain_depth: usize,
) -> Result<Vec<u8>, String> {
    read_pack_entry_with_format(
        pack_path,
        entry_name,
        resolve_base_blob_map,
        max_chain_depth,
        DEFAULT_OBJECT_PACK_WRITE_FORMAT,
    )
}

pub fn read_pack_entry_with_format(
    pack_path: &str,
    entry_name: &str,
    resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
    max_chain_depth: usize,
    persisted_pack_format: &str,
) -> Result<Vec<u8>, String> {
    object_pack_backend_from_persisted_format(persisted_pack_format)?.read_pack_entry(
        pack_path,
        entry_name,
        resolve_base_blob_map,
        max_chain_depth,
    )
}

pub(super) fn read_zstd_chunked_pack_entry(
    pack_path: &str,
    entry_name: &str,
    resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
    max_chain_depth: usize,
) -> Result<Vec<u8>, String> {
    let pack_index = read_zstd_chunked_container_index(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    read_zstd_chunked_object_entry(
        pack_path,
        &pack_index,
        entry_name,
        resolve_base_blob_map,
        max_chain_depth,
        &mut BTreeSet::new(),
        0,
    )
}

pub fn pack_has_entry(pack_path: &str, entry_name: &str) -> bool {
    pack_has_entry_with_format(pack_path, entry_name, DEFAULT_OBJECT_PACK_WRITE_FORMAT)
        .unwrap_or(false)
}

pub fn pack_has_entry_with_format(
    pack_path: &str,
    entry_name: &str,
    persisted_pack_format: &str,
) -> Result<bool, String> {
    object_pack_backend_from_persisted_format(persisted_pack_format)?
        .pack_has_entry(pack_path, entry_name)
}

pub(super) fn zstd_chunked_pack_has_entry(
    pack_path: &str,
    entry_name: &str,
) -> Result<bool, String> {
    if !Path::new(pack_path).exists() {
        return Ok(false);
    }
    let pack_index = read_zstd_chunked_container_index(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    Ok(pack_index
        .members
        .iter()
        .any(|entry| entry.entry_name == entry_name))
}

impl PackEntryArchive {
    pub fn open(pack_path: &str) -> Result<Self, String> {
        Self::open_with_format(pack_path, DEFAULT_OBJECT_PACK_WRITE_FORMAT)
    }

    pub fn open_with_format(pack_path: &str, persisted_pack_format: &str) -> Result<Self, String> {
        PackFormatKind::from_persisted(persisted_pack_format)?;
        let pack_index = read_zstd_chunked_container_index(
            pack_path,
            ZSTD_CHUNKED_INDEX_KIND_OBJECT,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )?;
        let entries_by_name = ObjectPackIndexJson::stateless().entries_by_name(
            &ObjectPackIndexJson::stateless().zstd_chunked_index_json(&pack_index)?,
        )?;
        Ok(Self {
            pack_path: pack_path.to_string(),
            pack_index,
            raw_chunk_cache: BTreeMap::new(),
            entries_by_name,
        })
    }

    pub fn has_entry(&mut self, entry_name: &str) -> bool {
        if !self.entries_by_name.contains_key(entry_name) {
            return false;
        }
        self.pack_index
            .members
            .iter()
            .any(|entry| entry.entry_name == entry_name)
    }

    pub fn index_json_and_checksum(&self) -> Result<(JsonValue, String), String> {
        let index = ObjectPackIndexJson::stateless().zstd_chunked_index_json(&self.pack_index)?;
        let encoded = encode_zstd_chunked_index(&self.pack_index, ZSTD_CHUNKED_INDEX_KIND_OBJECT)?;
        Ok((index, sha256_hex(&encoded)))
    }

    pub fn read_entry(
        &mut self,
        entry_name: &str,
        resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
        max_chain_depth: usize,
    ) -> Result<Vec<u8>, String> {
        read_zstd_chunked_object_entry_with_chunk_cache(
            &self.pack_path,
            &self.pack_index,
            entry_name,
            resolve_base_blob_map,
            max_chain_depth,
            &mut BTreeSet::new(),
            0,
            &mut self.raw_chunk_cache,
        )
    }

    #[cfg(test)]
    pub(crate) fn cached_zstd_chunk_count(&self) -> usize {
        self.raw_chunk_cache.len()
    }
}

impl TreePackEntryArchive {
    pub fn open_with_format(pack_path: &str, persisted_pack_format: &str) -> Result<Self, String> {
        TreePackFormatKind::from_persisted(persisted_pack_format)?;
        let pack_index = read_zstd_chunked_container_index(
            pack_path,
            ZSTD_CHUNKED_INDEX_KIND_TREE,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )?;
        Ok(Self {
            pack_path: pack_path.to_string(),
            pack_index,
            raw_chunk_cache: BTreeMap::new(),
        })
    }

    pub fn read_tree_by_entry_name(
        &mut self,
        tree_id: &str,
        entry_name: &str,
        entry_count: usize,
        checksum: &str,
    ) -> Result<JsonValue, String> {
        let member = self
            .pack_index
            .members
            .iter()
            .find(|entry| entry.content_id == tree_id && entry.entry_name == entry_name)
            .cloned()
            .ok_or_else(|| format!("missing tree-pack entry: {entry_name}"))?;
        if member.entry_count != Some(entry_count) {
            return Err(format!("Tree pack entry count mismatch for {tree_id}"));
        }
        if member.checksum != checksum {
            return Err(format!("Tree pack entry checksum mismatch for {tree_id}"));
        }
        read_zstd_chunked_tree_member_with_chunk_cache(
            &self.pack_path,
            &self.pack_index,
            &member,
            &mut self.raw_chunk_cache,
        )
    }

    pub fn index_json_and_checksum(&self) -> Result<(JsonValue, String), String> {
        let index = TreePackIndexJson::stateless().zstd_chunked_index_json(&self.pack_index)?;
        let encoded = encode_zstd_chunked_index(&self.pack_index, ZSTD_CHUNKED_INDEX_KIND_TREE)?;
        Ok((index, sha256_hex(&encoded)))
    }

    pub fn tree_checksum(&self, tree_id: &str) -> Option<&str> {
        self.pack_index
            .members
            .iter()
            .find(|member| member.content_id == tree_id)
            .map(|member| member.checksum.as_str())
    }

    pub fn read_tree_by_ordinal(&mut self, entry_ordinal: usize) -> Result<JsonValue, String> {
        let member = self
            .pack_index
            .members
            .iter()
            .find(|entry| entry.member_ordinal == entry_ordinal)
            .cloned()
            .ok_or_else(|| format!("missing tree pack entry_ordinal: {entry_ordinal}"))?;
        let rows = read_zstd_chunked_tree_member_with_chunk_cache(
            &self.pack_path,
            &self.pack_index,
            &member,
            &mut self.raw_chunk_cache,
        )?;
        Ok(json!({
            "tree_id": member.content_id,
            "entry_ordinal": member.member_ordinal,
            "rows": rows,
        }))
    }

    pub fn tree_ordinals(&self) -> Result<Vec<(String, usize, usize)>, String> {
        self.pack_index
            .members
            .iter()
            .map(|entry| {
                Ok((
                    entry.content_id.clone(),
                    entry.member_ordinal,
                    entry.entry_count.ok_or_else(|| {
                        format!("tree pack member {} has no entry count", entry.content_id)
                    })?,
                ))
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn cached_zstd_chunk_count(&self) -> usize {
        self.raw_chunk_cache.len()
    }
}

impl ObjectPackBackend for ZstdChunkedObjectPackBackend {
    fn format_kind(&self) -> PackFormatKind {
        PackFormatKind::ZstdChunkedV1
    }

    fn write_pack_archive(
        &self,
        pack_path: &str,
        pack_id: &str,
        created_at: &str,
        members: &JsonValue,
    ) -> Result<JsonValue, String> {
        write_zstd_chunked_pack_archive(pack_path, pack_id, created_at, members)
    }

    fn read_pack_index(&self, pack_path: &str) -> Result<JsonValue, String> {
        read_zstd_chunked_pack_index(pack_path)
    }

    fn pack_has_entry(&self, pack_path: &str, entry_name: &str) -> Result<bool, String> {
        zstd_chunked_pack_has_entry(pack_path, entry_name)
    }

    fn read_pack_entry(
        &self,
        pack_path: &str,
        entry_name: &str,
        resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
        max_chain_depth: usize,
    ) -> Result<Vec<u8>, String> {
        read_zstd_chunked_pack_entry(
            pack_path,
            entry_name,
            resolve_base_blob_map,
            max_chain_depth,
        )
    }
}
