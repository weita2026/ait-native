use super::*;

pub fn build_git_binary_delta(base_data: &[u8], target_data: &[u8]) -> Vec<u8> {
    let unit = delta_match_granularity(base_data.len(), target_data.len());
    let base_chunks = chunk_bytes(base_data, unit);
    let target_chunks = chunk_bytes(target_data, unit);
    let ops = capture_diff_slices(Algorithm::Myers, &base_chunks, &target_chunks);
    let mut out = Vec::new();
    out.extend(encode_size_varint(base_data.len()));
    out.extend(encode_size_varint(target_data.len()));

    for op in ops {
        match op {
            DiffOp::Equal { old_index, len, .. } => {
                let base_start = old_index * unit;
                let base_end = std::cmp::min((old_index + len) * unit, base_data.len());
                append_copy_instructions(&mut out, base_start, base_end - base_start);
            }
            DiffOp::Insert {
                new_index, new_len, ..
            }
            | DiffOp::Replace {
                new_index, new_len, ..
            } => {
                let start = new_index * unit;
                let end = std::cmp::min((new_index + new_len) * unit, target_data.len());
                append_insert_instructions(&mut out, &target_data[start..end]);
            }
            DiffOp::Delete { .. } => {}
        }
    }

    out
}

pub fn build_git_binary_delta_member(
    entry_name: &str,
    blob_id: &str,
    base_blob_id: &str,
    base_data: &[u8],
    target_data: &[u8],
    chain_depth: usize,
) -> JsonValue {
    let delta = build_git_binary_delta(base_data, target_data);
    json!({
        "entry_name": entry_name,
        "blob_id": blob_id,
        "data": delta,
        "logical_data": target_data,
        "entry_type": "delta",
        "base_blob_id": base_blob_id,
        "chain_depth": chain_depth,
        "delta_algorithm": PACK_DELTA_GIT_BINARY_V1,
    })
}

pub fn build_pack_members(
    blob_items: &JsonValue,
    max_delta_chain_depth: usize,
    initial_by_path: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    let candidates = parse_pack_candidates(blob_items)?;
    let initial_by_path = parse_initial_by_path(initial_by_path)?;
    Ok(JsonValue::Array(
        build_typed_pack_members(candidates, max_delta_chain_depth, Some(&initial_by_path))
            .iter()
            .map(member_to_json)
            .collect(),
    ))
}

pub(crate) fn build_typed_pack_members(
    candidates: Vec<PackCandidate>,
    max_delta_chain_depth: usize,
    initial_by_path: Option<&BTreeMap<String, PackCandidate>>,
) -> Vec<ObjectPackWriteMember> {
    let mut members = Vec::with_capacity(candidates.len());
    let mut latest_by_path = initial_by_path.cloned().unwrap_or_default();

    for item in candidates {
        let mut member = ObjectPackWriteMember {
            entry_name: item.entry_name.clone(),
            blob_id: item.blob_id.clone(),
            data: item.data.clone(),
            logical_data: None,
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
                    member = ObjectPackWriteMember {
                        entry_name: item.entry_name.clone(),
                        blob_id: item.blob_id.clone(),
                        data: delta_data,
                        logical_data: Some(item.data.clone()),
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

pub fn apply_git_binary_delta(base_data: &[u8], delta_data: &[u8]) -> Result<Vec<u8>, String> {
    let (expected_base_size, mut cursor) = decode_size_varint(delta_data, 0)?;
    if expected_base_size != base_data.len() {
        return Err("Invalid pack delta payload: base size mismatch".to_string());
    }
    let (expected_target_size, next_cursor) = decode_size_varint(delta_data, cursor)?;
    cursor = next_cursor;
    let mut out = Vec::new();

    while cursor < delta_data.len() {
        let command = delta_data[cursor];
        cursor += 1;
        if (command & 0x80) != 0 {
            let mut offset = 0usize;
            let mut size = 0usize;
            if (command & 0x01) != 0 {
                ensure_available(delta_data, cursor, 1, "copy offset truncated")?;
                offset |= usize::from(delta_data[cursor]);
                cursor += 1;
            }
            if (command & 0x02) != 0 {
                ensure_available(delta_data, cursor, 1, "copy offset truncated")?;
                offset |= usize::from(delta_data[cursor]) << 8;
                cursor += 1;
            }
            if (command & 0x04) != 0 {
                ensure_available(delta_data, cursor, 1, "copy offset truncated")?;
                offset |= usize::from(delta_data[cursor]) << 16;
                cursor += 1;
            }
            if (command & 0x08) != 0 {
                ensure_available(delta_data, cursor, 1, "copy offset truncated")?;
                offset |= usize::from(delta_data[cursor]) << 24;
                cursor += 1;
            }
            if (command & 0x10) != 0 {
                ensure_available(delta_data, cursor, 1, "copy size truncated")?;
                size |= usize::from(delta_data[cursor]);
                cursor += 1;
            }
            if (command & 0x20) != 0 {
                ensure_available(delta_data, cursor, 1, "copy size truncated")?;
                size |= usize::from(delta_data[cursor]) << 8;
                cursor += 1;
            }
            if (command & 0x40) != 0 {
                ensure_available(delta_data, cursor, 1, "copy size truncated")?;
                size |= usize::from(delta_data[cursor]) << 16;
                cursor += 1;
            }
            if size == 0 {
                size = 0x10000;
            }
            let end = offset + size;
            if end > base_data.len() {
                return Err("Invalid pack delta payload: copy range out of bounds".to_string());
            }
            out.extend_from_slice(&base_data[offset..end]);
            continue;
        }
        if command == 0 {
            return Err("Invalid pack delta payload: zero instruction is reserved".to_string());
        }
        let end = cursor + usize::from(command);
        if end > delta_data.len() {
            return Err("Invalid pack delta payload: insert data truncated".to_string());
        }
        out.extend_from_slice(&delta_data[cursor..end]);
        cursor = end;
    }

    if out.len() != expected_target_size {
        return Err("Invalid pack delta payload: target size mismatch".to_string());
    }
    Ok(out)
}

pub fn apply_pack_delta(
    base_data: &[u8],
    delta_data: &[u8],
    algorithm: &str,
) -> Result<Vec<u8>, String> {
    if algorithm == PACK_DELTA_GIT_BINARY_V1 {
        return apply_git_binary_delta(base_data, delta_data);
    }
    Err(format!("Unsupported pack delta algorithm: '{algorithm}'"))
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

pub fn write_typed_pack_archive_with_format(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    members: &[ObjectPackWriteMember],
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    if persisted_pack_format != PACK_FORMAT_ZSTD_CHUNKED_V1 {
        return Err(format!(
            "Typed object-pack writes require {PACK_FORMAT_ZSTD_CHUNKED_V1}, got {persisted_pack_format}"
        ));
    }
    let mut entry_names = BTreeSet::new();
    let mut blob_ids = BTreeSet::new();
    let inputs = members
        .iter()
        .enumerate()
        .map(|(ordinal, member)| {
            if member.entry_name != format!("blobs/{}", member.blob_id)
                || !entry_names.insert(member.entry_name.as_str())
                || !blob_ids.insert(member.blob_id.as_str())
            {
                return Err(format!(
                    "Typed object-pack member {} has a duplicate or noncanonical identity",
                    member.blob_id
                ));
            }
            let logical_data = member.logical_data.as_deref().unwrap_or(&member.data);
            match member.entry_type.as_str() {
                "full"
                    if member.base_blob_id.is_none()
                        && member.chain_depth == 0
                        && member.delta_algorithm.is_none()
                        && logical_data == member.data => {}
                "delta"
                    if member.base_blob_id.is_some()
                        && member.chain_depth > 0
                        && member.delta_algorithm.as_deref() == Some(PACK_DELTA_GIT_BINARY_V1)
                        && member.logical_data.is_some() => {}
                _ => {
                    return Err(format!(
                        "Typed object-pack member {} has inconsistent {} metadata",
                        member.blob_id, member.entry_type
                    ))
                }
            }
            Ok(ZstdChunkedMemberInput {
                member_ordinal: ordinal,
                entry_name: member.entry_name.clone(),
                content_id: member.blob_id.clone(),
                entry_type: member.entry_type.clone(),
                entry_count: None,
                base_content_id: member.base_blob_id.clone(),
                delta_algorithm: member.delta_algorithm.clone(),
                chain_depth: member.chain_depth,
                data: member.data.clone(),
                logical_len: logical_data.len(),
                checksum: sha256_hex(logical_data),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    write_zstd_chunked_pack_inputs(pack_path, pack_id, created_at, &inputs)
}

pub(in crate::pack_substrate) fn write_zstd_chunked_pack_archive(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    members: &JsonValue,
) -> Result<JsonValue, String> {
    let parsed = parse_pack_members(members)?;
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
    write_zstd_chunked_pack_inputs(pack_path, pack_id, created_at, &inputs)
}

fn write_zstd_chunked_pack_inputs(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    inputs: &[ZstdChunkedMemberInput],
) -> Result<JsonValue, String> {
    let pack_index = write_zstd_chunked_container(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
        ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME,
        pack_id,
        created_at,
        inputs,
        ZSTD_CHUNKED_DEFAULT_CHUNK_BYTES,
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

pub fn pack_index_checksum_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<Option<String>, String> {
    PackFormatKind::from_persisted(persisted_pack_format)?;
    zstd_chunked_container_index_checksum(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .map(Some)
}

pub fn validate_pack_archive_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<(), String> {
    PackFormatKind::from_persisted(persisted_pack_format)?;
    validate_zstd_chunked_container_chunks(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
}

pub(in crate::pack_substrate) fn read_zstd_chunked_pack_index(
    pack_path: &str,
) -> Result<JsonValue, String> {
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
            increment_u64(summary_obj, "index_error_count", 1);
            continue;
        };
        let Some(pack_format) = optional_text_field(row_obj, "pack_format") else {
            increment_u64(summary_obj, "index_error_count", 1);
            continue;
        };
        let pack_abs = root.join(pack_path);
        if !pack_abs.exists() {
            increment_u64(summary_obj, "index_error_count", 1);
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
        let entries_by_name = ObjectPackIndexJson::stateless().entries_by_name(&pack_index)?;
        increment_u64(summary_obj, "indexed_pack_count", 1);
        increment_u64(
            summary_obj,
            "pack_indexed_blob_count",
            entries_by_name.len() as u64,
        );
        for entry in entries_by_name.values() {
            let member_bytes = u64::try_from(entry.byte_length).unwrap_or(0);
            let logical_bytes =
                u64::try_from(entry.uncompressed_byte_length).unwrap_or(member_bytes);
            let entry_type = entry.entry_type.as_str();
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

#[expect(
    clippy::too_many_arguments,
    reason = "arguments are independent counters in the stable validation payload"
)]
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
    let state;
    let mut needs_attention = false;

    let drift_count = signals_summary
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("drift_count"))
        .and_then(JsonValue::as_u64)
        .unwrap_or(0) as usize;
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
        recommended_action = "inspect".to_string();
        next_actions.push("inspect".to_string());
        reasons.push("storage metadata requires operator attention".to_string());
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
        reasons.push("pack data exists but no delta entries have been produced yet".to_string());
        if unreferenced_tree_count > 0 {
            reasons.push("unreachable tree metadata remains to be cleaned up".to_string());
        }
    } else if pack_count > 1 || unreferenced_blob_count > 0 || unreferenced_tree_count > 0 {
        state = "partially_optimized".to_string();
        if pack_count > 1 {
            reasons.push("multiple live packs remain".to_string());
        }
        if unreferenced_blob_count > 0 {
            reasons.push("unreferenced blob payloads remain".to_string());
        }
        if unreferenced_tree_count > 0 {
            reasons.push("unreachable tree metadata remains".to_string());
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

pub(in crate::pack_substrate) fn read_zstd_chunked_pack_entry(
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

pub(in crate::pack_substrate) fn zstd_chunked_pack_has_entry(
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
        let contract = ObjectPackIndexJson::stateless();
        let entries_by_name =
            contract.entries_by_name(&contract.zstd_chunked_index_json(&pack_index)?)?;
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
