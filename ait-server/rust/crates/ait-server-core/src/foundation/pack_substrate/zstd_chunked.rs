use super::*;

#[cfg(test)]
std::thread_local! {
    static TEST_ZSTD_OBJECT_INDEX_FILE_READ_COUNT: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
    static TEST_ZSTD_TREE_INDEX_FILE_READ_COUNT: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
    static TEST_ZSTD_CHUNK_FILE_READ_COUNT: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_test_zstd_container_file_read_counts() {
    TEST_ZSTD_OBJECT_INDEX_FILE_READ_COUNT.with(|count| count.set(0));
    TEST_ZSTD_TREE_INDEX_FILE_READ_COUNT.with(|count| count.set(0));
    TEST_ZSTD_CHUNK_FILE_READ_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn test_zstd_container_file_read_counts() -> (u64, u64, u64) {
    (
        TEST_ZSTD_OBJECT_INDEX_FILE_READ_COUNT.with(std::cell::Cell::get),
        TEST_ZSTD_TREE_INDEX_FILE_READ_COUNT.with(std::cell::Cell::get),
        TEST_ZSTD_CHUNK_FILE_READ_COUNT.with(std::cell::Cell::get),
    )
}

#[derive(Clone, Debug)]
pub(super) struct ZstdChunkedMemberInput {
    pub(super) member_ordinal: usize,
    pub(super) entry_name: String,
    pub(super) content_id: String,
    pub(super) entry_type: String,
    pub(super) entry_count: Option<usize>,
    pub(super) base_content_id: Option<String>,
    pub(super) delta_algorithm: Option<String>,
    pub(super) chain_depth: usize,
    pub(super) data: Vec<u8>,
    pub(super) logical_len: usize,
    pub(super) checksum: String,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_zstd_chunked_container(
    pack_path: &str,
    kind: u8,
    pack_format: &str,
    index_entry_name: &str,
    pack_id: &str,
    created_at: &str,
    members: &[ZstdChunkedMemberInput],
    chunk_bytes: usize,
) -> Result<ZstdChunkedPackIndex, String> {
    let path = PathBuf::from(pack_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create zstd pack parent directory: {err}"))?;
    }
    let mut file =
        File::create(&path).map_err(|err| format!("failed to create zstd pack archive: {err}"))?;

    let chunk_bytes = chunk_bytes.max(1);
    let mut chunks = Vec::<ZstdChunkedChunkIndex>::new();
    let mut indexed_members = Vec::<ZstdChunkedMemberIndex>::new();
    let mut raw_chunk = Vec::<u8>::new();
    let mut raw_chunk_member_count = 0usize;
    let mut next_chunk_ordinal = 0usize;

    for member in members {
        if raw_chunk_member_count > 0
            && raw_chunk.len().saturating_add(member.data.len()) > chunk_bytes
        {
            chunks.push(write_zstd_chunked_raw_chunk(
                &mut file,
                next_chunk_ordinal,
                &raw_chunk,
            )?);
            next_chunk_ordinal += 1;
            raw_chunk.clear();
            raw_chunk_member_count = 0;
        }

        let chunk_ordinal = next_chunk_ordinal;
        let in_chunk_offset = raw_chunk.len();
        raw_chunk.extend_from_slice(&member.data);
        raw_chunk_member_count += 1;
        indexed_members.push(ZstdChunkedMemberIndex {
            member_ordinal: member.member_ordinal,
            entry_name: member.entry_name.clone(),
            content_id: member.content_id.clone(),
            entry_type: member.entry_type.clone(),
            entry_count: member.entry_count,
            base_content_id: member.base_content_id.clone(),
            delta_algorithm: member.delta_algorithm.clone(),
            chain_depth: member.chain_depth,
            chunk_ordinal,
            in_chunk_offset,
            stored_len: member.data.len(),
            logical_len: member.logical_len,
            checksum: member.checksum.clone(),
        });

        if raw_chunk.len() >= chunk_bytes {
            chunks.push(write_zstd_chunked_raw_chunk(
                &mut file,
                next_chunk_ordinal,
                &raw_chunk,
            )?);
            next_chunk_ordinal += 1;
            raw_chunk.clear();
            raw_chunk_member_count = 0;
        }
    }

    if raw_chunk_member_count > 0 {
        chunks.push(write_zstd_chunked_raw_chunk(
            &mut file,
            next_chunk_ordinal,
            &raw_chunk,
        )?);
    }

    let pack_index = ZstdChunkedPackIndex {
        pack_format: pack_format.to_string(),
        pack_id: pack_id.to_string(),
        created_at: created_at.to_string(),
        index_entry_name: index_entry_name.to_string(),
        chunks,
        members: indexed_members,
    };
    validate_zstd_chunked_index(&pack_index, kind, pack_format, None)?;
    let index_bytes = encode_zstd_chunked_index(&pack_index, kind)?;
    let index_offset = file
        .stream_position()
        .map_err(|err| format!("failed to locate zstd pack index offset: {err}"))?;
    file.write_all(&index_bytes)
        .map_err(|err| format!("failed to write zstd pack index: {err}"))?;
    write_zstd_chunked_trailer(&mut file, index_offset, index_bytes.len(), &index_bytes)?;
    file.flush()
        .map_err(|err| format!("failed to flush zstd pack archive: {err}"))?;
    Ok(pack_index)
}

pub(super) fn write_zstd_chunked_raw_chunk(
    file: &mut File,
    chunk_ordinal: usize,
    raw: &[u8],
) -> Result<ZstdChunkedChunkIndex, String> {
    let compressed = zstd::bulk::compress(raw, ZSTD_CHUNKED_LEVEL)
        .map_err(|err| format!("failed to zstd-compress pack chunk: {err}"))?;
    let compressed_offset = file
        .stream_position()
        .map_err(|err| format!("failed to locate zstd pack chunk offset: {err}"))?;
    file.write_all(&compressed)
        .map_err(|err| format!("failed to write zstd pack chunk: {err}"))?;
    Ok(ZstdChunkedChunkIndex {
        chunk_ordinal,
        compressed_offset,
        compressed_len: compressed.len(),
        raw_len: raw.len(),
        checksum: sha256_hex(raw),
    })
}

pub(super) fn read_zstd_chunked_container_index(
    pack_path: &str,
    kind: u8,
    expected_pack_format: &str,
) -> Result<ZstdChunkedPackIndex, String> {
    #[cfg(test)]
    match kind {
        ZSTD_CHUNKED_INDEX_KIND_OBJECT => {
            TEST_ZSTD_OBJECT_INDEX_FILE_READ_COUNT.with(|count| count.set(count.get() + 1));
        }
        ZSTD_CHUNKED_INDEX_KIND_TREE => {
            TEST_ZSTD_TREE_INDEX_FILE_READ_COUNT.with(|count| count.set(count.get() + 1));
        }
        _ => {}
    }
    let mut file =
        File::open(pack_path).map_err(|err| format!("failed to open zstd pack archive: {err}"))?;
    let file_len = file
        .metadata()
        .map_err(|err| format!("failed to stat zstd pack archive: {err}"))?
        .len();
    if file_len < ZSTD_CHUNKED_TRAILER_LEN as u64 {
        return Err("Invalid zstd chunked pack: truncated trailer".to_string());
    }
    file.seek(SeekFrom::End(-(ZSTD_CHUNKED_TRAILER_LEN as i64)))
        .map_err(|err| format!("failed to seek zstd pack trailer: {err}"))?;
    let mut trailer = vec![0u8; ZSTD_CHUNKED_TRAILER_LEN];
    file.read_exact(&mut trailer)
        .map_err(|err| format!("failed to read zstd pack trailer: {err}"))?;
    let (index_offset, index_len, expected_checksum) = decode_zstd_chunked_trailer(&trailer)?;
    let data_end = file_len - ZSTD_CHUNKED_TRAILER_LEN as u64;
    let index_end = index_offset
        .checked_add(index_len as u64)
        .ok_or_else(|| "Invalid zstd chunked pack: index range overflow".to_string())?;
    if index_end > data_end {
        return Err("Invalid zstd chunked pack: index range exceeds container".to_string());
    }
    file.seek(SeekFrom::Start(index_offset))
        .map_err(|err| format!("failed to seek zstd pack index: {err}"))?;
    let mut index_bytes = vec![0u8; index_len];
    file.read_exact(&mut index_bytes)
        .map_err(|err| format!("failed to read zstd pack index: {err}"))?;
    if sha256_bytes(&index_bytes) != expected_checksum {
        return Err("Invalid zstd chunked pack: index checksum mismatch".to_string());
    }
    let pack_index = decode_zstd_chunked_index(&index_bytes, kind)?;
    validate_zstd_chunked_index(&pack_index, kind, expected_pack_format, Some(index_offset))?;
    Ok(pack_index)
}

pub(super) fn read_zstd_chunked_container_index_from_bytes(
    data: &[u8],
    kind: u8,
    expected_pack_format: &str,
) -> Result<ZstdChunkedPackIndex, String> {
    if data.len() < ZSTD_CHUNKED_TRAILER_LEN {
        return Err("Invalid zstd chunked pack: truncated trailer".to_string());
    }
    let trailer_start = data.len() - ZSTD_CHUNKED_TRAILER_LEN;
    let (index_offset, index_len, expected_checksum) =
        decode_zstd_chunked_trailer(&data[trailer_start..])?;
    let index_start = usize::try_from(index_offset)
        .map_err(|_| "Invalid zstd chunked pack: index offset overflow".to_string())?;
    let index_end = index_start
        .checked_add(index_len)
        .ok_or_else(|| "Invalid zstd chunked pack: index range overflow".to_string())?;
    if index_end > trailer_start {
        return Err("Invalid zstd chunked pack: index range exceeds container".to_string());
    }
    let index_bytes = &data[index_start..index_end];
    if sha256_bytes(index_bytes) != expected_checksum {
        return Err("Invalid zstd chunked pack: index checksum mismatch".to_string());
    }
    let pack_index = decode_zstd_chunked_index(index_bytes, kind)?;
    validate_zstd_chunked_index(&pack_index, kind, expected_pack_format, Some(index_offset))?;
    Ok(pack_index)
}

pub(super) fn zstd_chunked_object_pack_index_json(
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

pub(super) fn zstd_chunked_tree_pack_index_json(
    pack_index: &ZstdChunkedPackIndex,
) -> Result<JsonValue, String> {
    if pack_index.pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
        return Err(format!(
            "Invalid zstd tree-pack index: unsupported pack_format {}",
            pack_index.pack_format
        ));
    }
    let trees = pack_index
        .members
        .iter()
        .map(|member| {
            json!({
                "tree_id": member.content_id,
                "entry_ordinal": member.member_ordinal,
                "entry_name": member.entry_name,
                "entry_count": member.entry_count.unwrap_or(0),
                "byte_length": member.stored_len,
                "checksum": member.checksum,
            })
        })
        .collect::<Vec<_>>();
    let total_bytes = pack_index
        .members
        .iter()
        .map(|member| member.stored_len as u64)
        .sum::<u64>();
    Ok(json!({
        "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        "pack_id": pack_index.pack_id,
        "created_at": pack_index.created_at,
        "index_entry_name": pack_index.index_entry_name,
        "tree_count": trees.len(),
        "total_bytes": total_bytes,
        "chunk_count": pack_index.chunks.len(),
        "trees": trees,
    }))
}

pub(super) fn read_zstd_chunked_object_entry(
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

#[allow(clippy::too_many_arguments)]
pub(super) fn read_zstd_chunked_object_entry_with_chunk_cache(
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
    let data = read_zstd_chunked_member_stored_bytes_with_chunk_cache(
        pack_path,
        pack_index,
        member,
        raw_chunk_cache,
    )?;
    if member.entry_type == "full" {
        if member.chain_depth > max_chain_depth {
            return Err(format!("Pack delta chain depth exceeded for {entry_name}"));
        }
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
    let resolved_base = resolve_base_blob_map.and_then(|base_map| base_map.get(base_blob_id));
    if resolved_base.is_none() && member.chain_depth > max_chain_depth {
        return Err(format!("Pack delta chain depth exceeded for {entry_name}"));
    }
    let base_data = if let Some(base_data) = resolved_base {
        base_data.clone()
    } else if pack_index
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

pub(super) fn read_zstd_chunked_object_entry_from_bytes(
    pack_bytes: &[u8],
    pack_index: &ZstdChunkedPackIndex,
    entry_name: &str,
    resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
    max_chain_depth: usize,
    visited_blob_ids: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Vec<u8>, String> {
    if depth > max_chain_depth {
        return Err(format!("Pack delta chain depth exceeded for {entry_name}"));
    }
    let member = pack_index
        .members
        .iter()
        .find(|entry| entry.entry_name == entry_name)
        .ok_or_else(|| format!("missing pack entry: {entry_name}"))?;
    let data = read_zstd_chunked_member_stored_bytes_from_bytes(pack_bytes, pack_index, member)?;
    if member.entry_type == "full" {
        if member.chain_depth > max_chain_depth {
            return Err(format!("Pack delta chain depth exceeded for {entry_name}"));
        }
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
    let resolved_base = resolve_base_blob_map.and_then(|base_map| base_map.get(base_blob_id));
    let base_data = if let Some(base_data) = resolved_base {
        base_data.clone()
    } else if pack_index
        .members
        .iter()
        .any(|entry| entry.content_id == *base_blob_id)
    {
        let actual_base_entry_name = pack_index
            .members
            .iter()
            .find(|entry| entry.content_id == *base_blob_id)
            .map(|entry| entry.entry_name.as_str())
            .unwrap_or(base_entry_name.as_str());
        read_zstd_chunked_object_entry_from_bytes(
            pack_bytes,
            pack_index,
            actual_base_entry_name,
            resolve_base_blob_map,
            max_chain_depth,
            &mut next_visited,
            depth + 1,
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

pub(super) fn read_zstd_chunked_tree_member(
    pack_path: &str,
    pack_index: &ZstdChunkedPackIndex,
    member: &ZstdChunkedMemberIndex,
) -> Result<JsonValue, String> {
    let raw = read_zstd_chunked_member_stored_bytes(pack_path, pack_index, member)?;
    let entry_count = member.entry_count.ok_or_else(|| {
        format!(
            "Tree pack entry {} is missing entry_count",
            member.content_id
        )
    })?;
    let entry = TreePackIndexEntry {
        tree_id: member.content_id.clone(),
        entry_ordinal: member.member_ordinal,
        entry_count,
        checksum: member.checksum.clone(),
    };
    tree_pack_rows_from_zstd_chunked_binary_payload(&raw, &entry)
}

pub(super) fn read_zstd_chunked_tree_member_with_chunk_cache(
    pack_path: &str,
    pack_index: &ZstdChunkedPackIndex,
    member: &ZstdChunkedMemberIndex,
    raw_chunk_cache: &mut BTreeMap<usize, Vec<u8>>,
) -> Result<JsonValue, String> {
    let raw = read_zstd_chunked_member_stored_bytes_with_chunk_cache(
        pack_path,
        pack_index,
        member,
        raw_chunk_cache,
    )?;
    let entry_count = member.entry_count.ok_or_else(|| {
        format!(
            "Tree pack entry {} is missing entry_count",
            member.content_id
        )
    })?;
    let entry = TreePackIndexEntry {
        tree_id: member.content_id.clone(),
        entry_ordinal: member.member_ordinal,
        entry_count,
        checksum: member.checksum.clone(),
    };
    tree_pack_rows_from_zstd_chunked_binary_payload(&raw, &entry)
}

pub(super) fn read_zstd_chunked_member_stored_bytes(
    pack_path: &str,
    pack_index: &ZstdChunkedPackIndex,
    member: &ZstdChunkedMemberIndex,
) -> Result<Vec<u8>, String> {
    let chunk = pack_index
        .chunks
        .get(member.chunk_ordinal)
        .ok_or_else(|| format!("missing zstd chunk {}", member.chunk_ordinal))?;
    let raw = read_zstd_chunked_raw_chunk(pack_path, chunk)?;
    let end = member
        .in_chunk_offset
        .checked_add(member.stored_len)
        .ok_or_else(|| "Invalid zstd chunked pack: member range overflow".to_string())?;
    if end > raw.len() {
        return Err(format!(
            "Invalid zstd chunked pack: member range exceeds chunk for {}",
            member.entry_name
        ));
    }
    Ok(raw[member.in_chunk_offset..end].to_vec())
}

pub(super) fn read_zstd_chunked_member_stored_bytes_with_chunk_cache(
    pack_path: &str,
    pack_index: &ZstdChunkedPackIndex,
    member: &ZstdChunkedMemberIndex,
    raw_chunk_cache: &mut BTreeMap<usize, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let chunk = pack_index
        .chunks
        .get(member.chunk_ordinal)
        .ok_or_else(|| format!("missing zstd chunk {}", member.chunk_ordinal))?;
    let raw = match raw_chunk_cache.entry(member.chunk_ordinal) {
        std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(read_zstd_chunked_raw_chunk(pack_path, chunk)?)
        }
    };
    let end = member
        .in_chunk_offset
        .checked_add(member.stored_len)
        .ok_or_else(|| "Invalid zstd chunked pack: member range overflow".to_string())?;
    if end > raw.len() {
        return Err(format!(
            "Invalid zstd chunked pack: member range exceeds chunk for {}",
            member.entry_name
        ));
    }
    Ok(raw[member.in_chunk_offset..end].to_vec())
}

pub(super) fn read_zstd_chunked_member_stored_bytes_from_bytes(
    pack_bytes: &[u8],
    pack_index: &ZstdChunkedPackIndex,
    member: &ZstdChunkedMemberIndex,
) -> Result<Vec<u8>, String> {
    let chunk = pack_index
        .chunks
        .get(member.chunk_ordinal)
        .ok_or_else(|| format!("missing zstd chunk {}", member.chunk_ordinal))?;
    let raw = read_zstd_chunked_raw_chunk_from_bytes(pack_bytes, chunk)?;
    let end = member
        .in_chunk_offset
        .checked_add(member.stored_len)
        .ok_or_else(|| "Invalid zstd chunked pack: member range overflow".to_string())?;
    if end > raw.len() {
        return Err(format!(
            "Invalid zstd chunked pack: member range exceeds chunk for {}",
            member.entry_name
        ));
    }
    Ok(raw[member.in_chunk_offset..end].to_vec())
}

pub(super) fn read_zstd_chunked_raw_chunk(
    pack_path: &str,
    chunk: &ZstdChunkedChunkIndex,
) -> Result<Vec<u8>, String> {
    #[cfg(test)]
    TEST_ZSTD_CHUNK_FILE_READ_COUNT.with(|count| count.set(count.get() + 1));
    let compressed = read_file_range(pack_path, chunk.compressed_offset, chunk.compressed_len)?;
    #[cfg(feature = "perfetto-tracing")]
    let _trace =
        crate::perfetto_trace::PerfettoRange::new("ait.server.content.pack.file_chunk_decompress");
    let raw = zstd::bulk::decompress(&compressed, chunk.raw_len)
        .map_err(|err| format!("failed to zstd-decompress pack chunk: {err}"))?;
    if raw.len() != chunk.raw_len {
        return Err(format!(
            "Invalid zstd chunked pack: raw chunk size mismatch for chunk {}",
            chunk.chunk_ordinal
        ));
    }
    if sha256_hex(&raw) != chunk.checksum {
        return Err(format!(
            "Invalid zstd chunked pack: chunk checksum mismatch for chunk {}",
            chunk.chunk_ordinal
        ));
    }
    Ok(raw)
}

pub(super) fn read_zstd_chunked_raw_chunk_from_bytes(
    pack_bytes: &[u8],
    chunk: &ZstdChunkedChunkIndex,
) -> Result<Vec<u8>, String> {
    let start = usize::try_from(chunk.compressed_offset)
        .map_err(|_| "Invalid zstd chunked pack: chunk offset overflow".to_string())?;
    let end = start
        .checked_add(chunk.compressed_len)
        .filter(|end| *end <= pack_bytes.len())
        .ok_or_else(|| "Invalid zstd chunked pack: chunk range exceeds container".to_string())?;
    #[cfg(feature = "perfetto-tracing")]
    let _trace =
        crate::perfetto_trace::PerfettoRange::new("ait.server.content.pack.bytes_chunk_decompress");
    let raw = zstd::bulk::decompress(&pack_bytes[start..end], chunk.raw_len)
        .map_err(|err| format!("failed to zstd-decompress pack chunk: {err}"))?;
    if raw.len() != chunk.raw_len {
        return Err(format!(
            "Invalid zstd chunked pack: raw chunk size mismatch for chunk {}",
            chunk.chunk_ordinal
        ));
    }
    if sha256_hex(&raw) != chunk.checksum {
        return Err(format!(
            "Invalid zstd chunked pack: chunk checksum mismatch for chunk {}",
            chunk.chunk_ordinal
        ));
    }
    Ok(raw)
}

pub(super) fn read_file_range(path: &str, offset: u64, len: usize) -> Result<Vec<u8>, String> {
    let mut file = File::open(path).map_err(|err| format!("failed to open pack archive: {err}"))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|err| format!("failed to seek pack archive: {err}"))?;
    let mut out = vec![0u8; len];
    file.read_exact(&mut out)
        .map_err(|err| format!("failed to read pack archive range: {err}"))?;
    Ok(out)
}

pub(super) fn encode_zstd_chunked_index(
    pack_index: &ZstdChunkedPackIndex,
    kind: u8,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.extend_from_slice(ZSTD_CHUNKED_INDEX_MAGIC);
    push_u32(&mut out, ZSTD_CHUNKED_VERSION);
    push_u8(&mut out, kind);
    push_string(&mut out, &pack_index.pack_format)?;
    push_string(&mut out, &pack_index.pack_id)?;
    push_string(&mut out, &pack_index.created_at)?;
    push_string(&mut out, &pack_index.index_entry_name)?;
    push_u32(
        &mut out,
        usize_to_u32(pack_index.chunks.len(), "chunk_count")?,
    );
    for chunk in &pack_index.chunks {
        push_u32(
            &mut out,
            usize_to_u32(chunk.chunk_ordinal, "chunk_ordinal")?,
        );
        push_u64(&mut out, chunk.compressed_offset);
        push_u64(
            &mut out,
            usize_to_u64(chunk.compressed_len, "compressed_len")?,
        );
        push_u64(&mut out, usize_to_u64(chunk.raw_len, "raw_len")?);
        push_checksum_hex(&mut out, &chunk.checksum)?;
    }
    push_u32(
        &mut out,
        usize_to_u32(pack_index.members.len(), "member_count")?,
    );
    for member in &pack_index.members {
        push_u32(
            &mut out,
            usize_to_u32(member.member_ordinal, "member_ordinal")?,
        );
        push_string(&mut out, &member.entry_name)?;
        push_string(&mut out, &member.content_id)?;
        push_string(&mut out, &member.entry_type)?;
        push_option_usize(&mut out, member.entry_count)?;
        push_option_string(&mut out, member.base_content_id.as_deref())?;
        push_option_string(&mut out, member.delta_algorithm.as_deref())?;
        push_u32(
            &mut out,
            usize_to_u32(member.chain_depth, "delta_chain_depth")?,
        );
        push_u32(
            &mut out,
            usize_to_u32(member.chunk_ordinal, "member_chunk_ordinal")?,
        );
        push_u64(
            &mut out,
            usize_to_u64(member.in_chunk_offset, "in_chunk_offset")?,
        );
        push_u64(&mut out, usize_to_u64(member.stored_len, "stored_len")?);
        push_u64(&mut out, usize_to_u64(member.logical_len, "logical_len")?);
        push_checksum_hex(&mut out, &member.checksum)?;
    }
    Ok(out)
}

pub(super) fn decode_zstd_chunked_index(
    data: &[u8],
    expected_kind: u8,
) -> Result<ZstdChunkedPackIndex, String> {
    let mut cursor = 0usize;
    if read_bytes(
        data,
        &mut cursor,
        ZSTD_CHUNKED_INDEX_MAGIC.len(),
        "index magic",
    )? != ZSTD_CHUNKED_INDEX_MAGIC
    {
        return Err("Invalid zstd chunked pack: bad index magic".to_string());
    }
    let version = read_u32(data, &mut cursor, "index version")?;
    if version != ZSTD_CHUNKED_VERSION {
        return Err(format!(
            "Invalid zstd chunked pack: unsupported index version {version}"
        ));
    }
    let kind = read_u8(data, &mut cursor, "index kind")?;
    if kind != expected_kind {
        return Err("Invalid zstd chunked pack: index kind mismatch".to_string());
    }
    let pack_format = read_string(data, &mut cursor, "pack_format")?;
    let pack_id = read_string(data, &mut cursor, "pack_id")?;
    let created_at = read_string(data, &mut cursor, "created_at")?;
    let index_entry_name = read_string(data, &mut cursor, "index_entry_name")?;
    let chunk_count = read_u32(data, &mut cursor, "chunk_count")? as usize;
    let mut chunks = Vec::with_capacity(chunk_count);
    for _ in 0..chunk_count {
        chunks.push(ZstdChunkedChunkIndex {
            chunk_ordinal: read_u32(data, &mut cursor, "chunk_ordinal")? as usize,
            compressed_offset: read_u64(data, &mut cursor, "compressed_offset")?,
            compressed_len: read_u64(data, &mut cursor, "compressed_len")? as usize,
            raw_len: read_u64(data, &mut cursor, "raw_len")? as usize,
            checksum: read_checksum_hex(data, &mut cursor, "chunk_checksum")?,
        });
    }
    let member_count = read_u32(data, &mut cursor, "member_count")? as usize;
    let mut members = Vec::with_capacity(member_count);
    for _ in 0..member_count {
        members.push(ZstdChunkedMemberIndex {
            member_ordinal: read_u32(data, &mut cursor, "member_ordinal")? as usize,
            entry_name: read_string(data, &mut cursor, "entry_name")?,
            content_id: read_string(data, &mut cursor, "content_id")?,
            entry_type: read_string(data, &mut cursor, "entry_type")?,
            entry_count: read_option_usize(data, &mut cursor, "entry_count")?,
            base_content_id: read_option_string(data, &mut cursor, "base_content_id")?,
            delta_algorithm: read_option_string(data, &mut cursor, "delta_algorithm")?,
            chain_depth: read_u32(data, &mut cursor, "delta_chain_depth")? as usize,
            chunk_ordinal: read_u32(data, &mut cursor, "member_chunk_ordinal")? as usize,
            in_chunk_offset: read_u64(data, &mut cursor, "in_chunk_offset")? as usize,
            stored_len: read_u64(data, &mut cursor, "stored_len")? as usize,
            logical_len: read_u64(data, &mut cursor, "logical_len")? as usize,
            checksum: read_checksum_hex(data, &mut cursor, "member_checksum")?,
        });
    }
    if cursor != data.len() {
        return Err("Invalid zstd chunked pack: trailing index bytes".to_string());
    }
    Ok(ZstdChunkedPackIndex {
        pack_format,
        pack_id,
        created_at,
        index_entry_name,
        chunks,
        members,
    })
}

pub(super) fn validate_zstd_chunked_index(
    pack_index: &ZstdChunkedPackIndex,
    kind: u8,
    expected_pack_format: &str,
    data_end: Option<u64>,
) -> Result<(), String> {
    if pack_index.pack_format != expected_pack_format {
        return Err(format!(
            "Invalid zstd chunked pack: unsupported pack_format {}",
            pack_index.pack_format
        ));
    }
    if kind == ZSTD_CHUNKED_INDEX_KIND_OBJECT
        && pack_index.index_entry_name != ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME
    {
        return Err("Invalid zstd object pack index: incorrect index_entry_name".to_string());
    }
    if kind == ZSTD_CHUNKED_INDEX_KIND_TREE
        && pack_index.index_entry_name != ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME
    {
        return Err("Invalid zstd tree-pack index: incorrect index_entry_name".to_string());
    }
    for (expected, chunk) in pack_index.chunks.iter().enumerate() {
        if chunk.chunk_ordinal != expected {
            return Err(format!(
                "Invalid zstd chunked pack: non-sequential chunk ordinal {}",
                chunk.chunk_ordinal
            ));
        }
        if let Some(data_end) = data_end {
            let chunk_end = chunk
                .compressed_offset
                .checked_add(chunk.compressed_len as u64)
                .ok_or_else(|| "Invalid zstd chunked pack: chunk range overflow".to_string())?;
            if chunk_end > data_end {
                return Err(format!(
                    "Invalid zstd chunked pack: chunk {} exceeds data section",
                    chunk.chunk_ordinal
                ));
            }
        }
    }
    let mut entry_names = BTreeSet::new();
    let mut content_ids = BTreeSet::new();
    let mut ordinals = BTreeSet::new();
    for (fallback_ordinal, member) in pack_index.members.iter().enumerate() {
        if member.member_ordinal != fallback_ordinal {
            return Err(format!(
                "Invalid zstd chunked pack: non-sequential member ordinal {}",
                member.member_ordinal
            ));
        }
        if !entry_names.insert(member.entry_name.clone()) {
            return Err(format!(
                "Invalid zstd chunked pack: duplicate entry_name {}",
                member.entry_name
            ));
        }
        if !ordinals.insert(member.member_ordinal) {
            return Err(format!(
                "Invalid zstd chunked pack: duplicate member ordinal {}",
                member.member_ordinal
            ));
        }
        let chunk = pack_index.chunks.get(member.chunk_ordinal).ok_or_else(|| {
            format!(
                "Invalid zstd chunked pack: member {} points at missing chunk {}",
                member.entry_name, member.chunk_ordinal
            )
        })?;
        let member_end = member
            .in_chunk_offset
            .checked_add(member.stored_len)
            .ok_or_else(|| "Invalid zstd chunked pack: member range overflow".to_string())?;
        if member_end > chunk.raw_len {
            return Err(format!(
                "Invalid zstd chunked pack: member {} exceeds raw chunk",
                member.entry_name
            ));
        }
        if kind == ZSTD_CHUNKED_INDEX_KIND_OBJECT {
            if member.entry_count.is_some() {
                return Err(
                    "Invalid zstd object pack index: object member has entry_count".to_string(),
                );
            }
            if member.entry_type == "delta" && member.base_content_id.is_none() {
                return Err(format!(
                    "Invalid zstd object pack index: delta member {} missing base",
                    member.entry_name
                ));
            }
        } else if kind == ZSTD_CHUNKED_INDEX_KIND_TREE {
            if !content_ids.insert(member.content_id.clone()) {
                return Err(format!(
                    "Invalid zstd tree-pack index: duplicate tree_id {}",
                    member.content_id
                ));
            }
            if member.entry_count.is_none() {
                return Err(format!(
                    "Invalid zstd tree-pack index: tree {} missing entry_count",
                    member.content_id
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn write_zstd_chunked_trailer(
    file: &mut File,
    index_offset: u64,
    index_len: usize,
    index_bytes: &[u8],
) -> Result<(), String> {
    let mut trailer = Vec::with_capacity(ZSTD_CHUNKED_TRAILER_LEN);
    trailer.extend_from_slice(ZSTD_CHUNKED_TRAILER_MAGIC);
    push_u32(&mut trailer, ZSTD_CHUNKED_VERSION);
    push_u64(&mut trailer, index_offset);
    push_u64(&mut trailer, usize_to_u64(index_len, "index_len")?);
    trailer.extend_from_slice(&sha256_bytes(index_bytes));
    debug_assert_eq!(trailer.len(), ZSTD_CHUNKED_TRAILER_LEN);
    file.write_all(&trailer)
        .map_err(|err| format!("failed to write zstd pack trailer: {err}"))
}

pub(super) fn decode_zstd_chunked_trailer(data: &[u8]) -> Result<(u64, usize, [u8; 32]), String> {
    let mut cursor = 0usize;
    if read_bytes(
        data,
        &mut cursor,
        ZSTD_CHUNKED_TRAILER_MAGIC.len(),
        "trailer magic",
    )? != ZSTD_CHUNKED_TRAILER_MAGIC
    {
        return Err("Invalid zstd chunked pack: bad trailer magic".to_string());
    }
    let version = read_u32(data, &mut cursor, "trailer version")?;
    if version != ZSTD_CHUNKED_VERSION {
        return Err(format!(
            "Invalid zstd chunked pack: unsupported trailer version {version}"
        ));
    }
    let index_offset = read_u64(data, &mut cursor, "index_offset")?;
    let index_len = read_u64(data, &mut cursor, "index_len")? as usize;
    let checksum = read_checksum_bytes(data, &mut cursor, "index_checksum")?;
    if cursor != data.len() {
        return Err("Invalid zstd chunked pack: trailing trailer bytes".to_string());
    }
    Ok((index_offset, index_len, checksum))
}

pub(super) fn zstd_chunked_chunk_bytes_from_env(env_name: &str) -> usize {
    std::env::var(env_name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|mib| mib.saturating_mul(1024 * 1024))
        .filter(|bytes| *bytes > 0)
        .unwrap_or(ZSTD_CHUNKED_DEFAULT_CHUNK_BYTES)
}

pub(super) fn push_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

pub(super) fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    push_u32(out, usize_to_u32(value.len(), "string_len")?);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

pub(super) fn push_option_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), String> {
    match value {
        Some(value) => {
            push_u8(out, 1);
            push_string(out, value)
        }
        None => {
            push_u8(out, 0);
            Ok(())
        }
    }
}

pub(super) fn push_option_usize(out: &mut Vec<u8>, value: Option<usize>) -> Result<(), String> {
    match value {
        Some(value) => {
            push_u8(out, 1);
            push_u64(out, usize_to_u64(value, "optional_usize")?);
        }
        None => push_u8(out, 0),
    }
    Ok(())
}

pub(super) fn push_checksum_hex(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    out.extend_from_slice(&checksum_bytes_from_hex(value)?);
    Ok(())
}

pub(super) fn read_u8(data: &[u8], cursor: &mut usize, label: &str) -> Result<u8, String> {
    Ok(read_bytes(data, cursor, 1, label)?[0])
}

pub(super) fn read_u32(data: &[u8], cursor: &mut usize, label: &str) -> Result<u32, String> {
    let bytes = read_bytes(data, cursor, 4, label)?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

pub(super) fn read_u64(data: &[u8], cursor: &mut usize, label: &str) -> Result<u64, String> {
    let bytes = read_bytes(data, cursor, 8, label)?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

pub(super) fn read_string(data: &[u8], cursor: &mut usize, label: &str) -> Result<String, String> {
    let len = read_u32(data, cursor, label)? as usize;
    let bytes = read_bytes(data, cursor, len, label)?;
    String::from_utf8(bytes.to_vec())
        .map_err(|err| format!("Invalid zstd chunked pack: {label} is not UTF-8: {err}"))
}

pub(super) fn read_option_string(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<Option<String>, String> {
    match read_u8(data, cursor, label)? {
        0 => Ok(None),
        1 => read_string(data, cursor, label).map(Some),
        other => Err(format!(
            "Invalid zstd chunked pack: {label} has invalid option tag {other}"
        )),
    }
}

pub(super) fn read_option_usize(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<Option<usize>, String> {
    match read_u8(data, cursor, label)? {
        0 => Ok(None),
        1 => Ok(Some(read_u64(data, cursor, label)? as usize)),
        other => Err(format!(
            "Invalid zstd chunked pack: {label} has invalid option tag {other}"
        )),
    }
}

pub(super) fn read_checksum_hex(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<String, String> {
    Ok(hex_string(&read_checksum_bytes(data, cursor, label)?))
}

pub(super) fn read_checksum_bytes(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<[u8; 32], String> {
    let bytes = read_bytes(data, cursor, 32, label)?;
    Ok(bytes.try_into().unwrap())
}

pub(super) fn read_bytes<'a>(
    data: &'a [u8],
    cursor: &mut usize,
    len: usize,
    label: &str,
) -> Result<&'a [u8], String> {
    let end = cursor
        .checked_add(len)
        .ok_or_else(|| format!("Invalid zstd chunked pack: {label} range overflow"))?;
    if end > data.len() {
        return Err(format!("Invalid zstd chunked pack: truncated {label}"));
    }
    let bytes = &data[*cursor..end];
    *cursor = end;
    Ok(bytes)
}

pub(super) fn usize_to_u32(value: usize, label: &str) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{label} exceeds u32 capacity"))
}

pub(super) fn usize_to_u64(value: usize, label: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("{label} exceeds u64 capacity"))
}

pub(super) fn checksum_bytes_from_hex(value: &str) -> Result<[u8; 32], String> {
    let trimmed = value.trim();
    if trimmed.len() != 64 {
        return Err("checksum must be a 64-character SHA-256 hex string".to_string());
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        let start = index * 2;
        *slot = u8::from_str_radix(&trimmed[start..start + 2], 16)
            .map_err(|_| "checksum must be lowercase SHA-256 hex".to_string())?;
    }
    Ok(out)
}
