use super::*;
pub(in crate::pack_substrate) fn zstd_chunked_object_pack_index_json(
    pack_index: &ZstdChunkedPackIndex,
) -> Result<JsonValue, String> {
    if pack_index.pack_format != PACK_FORMAT_ZSTD_CHUNKED_V1 {
        return Err(format!(
            "Invalid zstd object pack index: unsupported pack_format {}",
            pack_index.pack_format
        ));
    }
    let entries = pack_index
        .members
        .iter()
        .map(|member| {
            let mut payload = Map::new();
            payload.insert(
                "entry_name".to_string(),
                JsonValue::String(member.entry_name.clone()),
            );
            payload.insert(
                "blob_id".to_string(),
                JsonValue::String(member.content_id.clone()),
            );
            payload.insert(
                "entry_type".to_string(),
                JsonValue::String(member.entry_type.clone()),
            );
            payload.insert(
                "byte_length".to_string(),
                JsonValue::Number(Number::from(member.stored_len as u64)),
            );
            payload.insert(
                "uncompressed_byte_length".to_string(),
                JsonValue::Number(Number::from(member.logical_len as u64)),
            );
            payload.insert(
                "base_blob_id".to_string(),
                member
                    .base_content_id
                    .as_ref()
                    .map(|value| JsonValue::String(value.clone()))
                    .unwrap_or(JsonValue::Null),
            );
            payload.insert(
                "chain_depth".to_string(),
                JsonValue::Number(Number::from(member.chain_depth as u64)),
            );
            payload.insert(
                "checksum".to_string(),
                JsonValue::String(member.checksum.clone()),
            );
            if let Some(value) = &member.delta_algorithm {
                payload.insert(
                    "delta_algorithm".to_string(),
                    JsonValue::String(value.clone()),
                );
            }
            JsonValue::Object(payload)
        })
        .collect::<Vec<_>>();
    let total_bytes = pack_index
        .members
        .iter()
        .map(|member| member.stored_len as u64)
        .sum::<u64>();
    Ok(json!({
        "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
        "pack_id": pack_index.pack_id,
        "created_at": pack_index.created_at,
        "index_entry_name": pack_index.index_entry_name,
        "member_count": entries.len(),
        "total_bytes": total_bytes,
        "chunk_count": pack_index.chunks.len(),
        "entries": entries,
    }))
}

pub(in crate::pack_substrate) fn read_zstd_chunked_object_entry(
    pack_path: &str,
    pack_index: &ZstdChunkedPackIndex,
    entry_name: &str,
    resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
    max_chain_depth: usize,
    visited_blob_ids: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Vec<u8>, String> {
    read_zstd_chunked_object_entry_with_chunk_cache(
        pack_path,
        pack_index,
        entry_name,
        resolve_base_blob_map,
        max_chain_depth,
        visited_blob_ids,
        depth,
        &mut BTreeMap::new(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "recursive pack decoding keeps bounded state and caller-owned caches explicit"
)]
pub(in crate::pack_substrate) fn read_zstd_chunked_object_entry_with_chunk_cache(
    pack_path: &str,
    pack_index: &ZstdChunkedPackIndex,
    entry_name: &str,
    resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
    max_chain_depth: usize,
    visited_blob_ids: &mut BTreeSet<String>,
    depth: usize,
    raw_chunk_cache: &mut BTreeMap<usize, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    if depth > max_chain_depth {
        return Err(format!("Pack delta chain depth exceeded for {entry_name}"));
    }
    let member = pack_index
        .members
        .iter()
        .find(|entry| entry.entry_name == entry_name)
        .ok_or_else(|| format!("missing pack entry: {entry_name}"))?;
    if member.chain_depth > max_chain_depth {
        return Err(format!("Pack delta chain depth exceeded for {entry_name}"));
    }
    let data = read_zstd_chunked_member_stored_bytes_with_chunk_cache(
        pack_path,
        pack_index,
        member,
        raw_chunk_cache,
    )?;
    if member.entry_type == "full" {
        if sha256_hex(&data) != member.checksum {
            return Err(format!("Pack entry checksum mismatch for {entry_name}"));
        }
        if data.len() != member.logical_len {
            return Err(format!("Pack entry size mismatch for {entry_name}"));
        }
        return Ok(data);
    }
    if member.entry_type != "delta" {
        return Err(format!(
            "Unsupported pack entry type: {:?}",
            member.entry_type
        ));
    }
    let Some(base_blob_id) = &member.base_content_id else {
        return Err(format!("Invalid delta entry base blob for {entry_name}"));
    };
    let identity = if member.content_id.is_empty() {
        entry_name.to_string()
    } else {
        member.content_id.clone()
    };
    if visited_blob_ids.contains(&identity) {
        return Err(format!("Cyclic pack delta chain detected for {entry_name}"));
    }
    let mut next_visited = visited_blob_ids.clone();
    next_visited.insert(identity);
    let base_entry_name = format!("blobs/{base_blob_id}");
    let base_data = if pack_index
        .members
        .iter()
        .any(|entry| entry.entry_name == base_entry_name)
    {
        read_zstd_chunked_object_entry_with_chunk_cache(
            pack_path,
            pack_index,
            &base_entry_name,
            resolve_base_blob_map,
            max_chain_depth,
            &mut next_visited,
            depth + 1,
            raw_chunk_cache,
        )?
    } else if let Some(base_map) = resolve_base_blob_map {
        base_map
            .get(base_blob_id)
            .cloned()
            .ok_or_else(|| format!("Missing base blob resolver for delta entry {entry_name}"))?
    } else {
        return Err(format!(
            "Missing base blob resolver for delta entry {entry_name}"
        ));
    };
    let Some(delta_algorithm) = member.delta_algorithm.as_deref() else {
        return Err(format!("Invalid delta entry algorithm for {entry_name}"));
    };
    let resolved = apply_pack_delta(&base_data, &data, delta_algorithm)?;
    if sha256_hex(&resolved) != member.checksum {
        return Err(format!("Pack entry checksum mismatch for {entry_name}"));
    }
    if resolved.len() != member.logical_len {
        return Err(format!("Pack entry size mismatch for {entry_name}"));
    }
    Ok(resolved)
}
