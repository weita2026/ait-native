use super::*;
use crate::json_support::JsonCodec;

pub fn tree_pack_manifest_path(pack_path: &str, entry_name: &str) -> String {
    format!("{pack_path}#{entry_name}")
}

impl TreePackEntryArchive {
    pub fn open_with_format(pack_path: &str, persisted_pack_format: &str) -> Result<Self, String> {
        let _trace = crate::perfetto_range!("ait.core.tree_pack.archive.open_with_format");
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
        let _trace = crate::perfetto_range!("ait.core.tree_pack.archive.read_tree_by_entry_name");
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

    pub fn read_tree_by_ordinal(&mut self, entry_ordinal: usize) -> Result<JsonValue, String> {
        let _trace = crate::perfetto_range!("ait.core.tree_pack.archive.read_tree_by_ordinal");
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

    #[cfg(test)]
    pub(crate) fn cached_zstd_chunk_count(&self) -> usize {
        self.raw_chunk_cache.len()
    }
}

pub fn build_tree_pack_members(
    tree_rows: &JsonValue,
    tree_entry_rows: &JsonValue,
) -> Result<JsonValue, String> {
    let trees = as_array(tree_rows, "tree_rows")?;
    let entries = as_array(tree_entry_rows, "tree_entry_rows")?;
    let mut entry_rows_by_tree: BTreeMap<String, Vec<JsonValue>> = BTreeMap::new();
    for row in entries {
        let row_obj = as_object(row, "tree entry row")?;
        let tree_id = required_text_field(row_obj, "tree_id")?;
        entry_rows_by_tree
            .entry(tree_id)
            .or_default()
            .push(row.clone());
    }
    let mut members = Vec::new();
    let mut sorted_trees = trees.iter().collect::<Vec<_>>();
    sorted_trees.sort_by_key(|row| {
        as_object(row, "tree row")
            .ok()
            .and_then(|obj| optional_text_field(obj, "tree_id"))
            .unwrap_or_default()
    });
    for row in sorted_trees {
        let row_obj = as_object(row, "tree row")?;
        let tree_id = required_text_field(row_obj, "tree_id")?;
        let member_entries = entry_rows_by_tree
            .get(&tree_id)
            .cloned()
            .unwrap_or_default();
        let data = tree_payload_bytes(&tree_id, &member_entries)?;
        members.push(json!({
            "tree_id": tree_id,
            "entry_name": format!("trees/{tree_id}.json"),
            "entry_count": required_usize_field(row_obj, "entry_count")?,
            "data": data,
            "checksum": sha256_hex(&data),
        }));
    }
    Ok(JsonValue::Array(members))
}

pub fn write_tree_pack_archive(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    members: &JsonValue,
) -> Result<JsonValue, String> {
    write_tree_pack_archive_with_format(
        pack_path,
        pack_id,
        created_at,
        members,
        DEFAULT_TREE_PACK_WRITE_FORMAT,
    )
}

pub fn write_tree_pack_archive_with_format(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    members: &JsonValue,
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    tree_pack_backend_from_persisted_format(persisted_pack_format)?
        .write_tree_pack_archive(pack_path, pack_id, created_at, members)
}

pub fn tree_pack_checksums_by_tree_id(
    archive_stats: &JsonValue,
) -> Result<BTreeMap<String, String>, String> {
    let entries = archive_stats
        .get("pack_index")
        .and_then(|pack_index| pack_index.get("trees"))
        .or_else(|| archive_stats.get("trees"))
        .or_else(|| archive_stats.get("entries"))
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Tree pack archive stats are missing tree entries.".to_string())?;
    let mut out = BTreeMap::new();
    for entry in entries {
        let obj = entry
            .as_object()
            .ok_or_else(|| "Tree pack archive entry must be an object.".to_string())?;
        let tree_id = obj
            .get("tree_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "Tree pack archive entry is missing tree_id.".to_string())?;
        let checksum = obj
            .get("checksum")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "Tree pack archive entry is missing checksum.".to_string())?;
        out.insert(tree_id.to_string(), checksum.to_string());
    }
    Ok(out)
}

pub(in crate::pack_substrate) fn write_zstd_chunked_tree_pack_archive(
    pack_path: &str,
    pack_id: &str,
    created_at: &str,
    members: &JsonValue,
) -> Result<JsonValue, String> {
    let parsed = parse_tree_pack_members(members)?;
    let inputs = parsed
        .iter()
        .enumerate()
        .map(|(ordinal, member)| {
            let data = zstd_chunked_tree_member_payload_bytes(member)?;
            Ok(ZstdChunkedMemberInput {
                member_ordinal: ordinal,
                entry_name: member.entry_name.clone(),
                content_id: member.tree_id.clone(),
                entry_type: "tree".to_string(),
                entry_count: Some(member.entry_count),
                base_content_id: None,
                delta_algorithm: None,
                chain_depth: 0,
                logical_len: data.len(),
                checksum: sha256_hex(&data),
                data,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let pack_index = write_zstd_chunked_container(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_TREE,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME,
        pack_id,
        created_at,
        &inputs,
        ZSTD_CHUNKED_DEFAULT_CHUNK_BYTES,
    )?;
    let pack_index_json = TreePackIndexJson::stateless().zstd_chunked_index_json(&pack_index)?;
    let index_bytes = encode_zstd_chunked_index(&pack_index, ZSTD_CHUNKED_INDEX_KIND_TREE)?;
    let index_checksum = sha256_hex(&index_bytes);
    let archive_bytes = std::fs::metadata(pack_path)
        .map_err(|err| format!("failed to stat zstd tree-pack archive: {err}"))?
        .len();
    Ok(json!({
        "tree_count": pack_index_json.get("tree_count").cloned().unwrap_or(JsonValue::Number(Number::from(0))),
        "total_bytes": pack_index_json.get("total_bytes").cloned().unwrap_or(JsonValue::Number(Number::from(0))),
        "archive_bytes": archive_bytes,
        "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        "pack_index_entry_name": ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME,
        "pack_index_checksum": index_checksum,
        "pack_index": pack_index_json,
    }))
}

pub fn read_tree_pack_index(pack_path: &str) -> Result<JsonValue, String> {
    read_tree_pack_index_with_format(pack_path, DEFAULT_TREE_PACK_WRITE_FORMAT)
}

pub fn read_tree_pack_index_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    tree_pack_backend_from_persisted_format(persisted_pack_format)?.read_tree_pack_index(pack_path)
}

pub fn tree_pack_index_checksum_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<Option<String>, String> {
    TreePackFormatKind::from_persisted(persisted_pack_format)?;
    zstd_chunked_container_index_checksum(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_TREE,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .map(Some)
}

pub fn validate_tree_pack_archive_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<(), String> {
    TreePackFormatKind::from_persisted(persisted_pack_format)?;
    validate_zstd_chunked_container_chunks(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_TREE,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
}

pub(in crate::pack_substrate) fn read_zstd_chunked_tree_pack_index(
    pack_path: &str,
) -> Result<JsonValue, String> {
    let pack_index = read_zstd_chunked_container_index(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_TREE,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    TreePackIndexJson::stateless().zstd_chunked_index_json(&pack_index)
}

pub fn read_tree_pack_index_without_ordinals(pack_path: &str) -> Result<JsonValue, String> {
    read_tree_pack_index_without_ordinals_with_format(pack_path, DEFAULT_TREE_PACK_WRITE_FORMAT)
}

pub fn read_tree_pack_index_without_ordinals_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    tree_pack_backend_from_persisted_format(persisted_pack_format)?
        .read_tree_pack_index_without_ordinals(pack_path)
}

pub fn read_tree_pack_physical_ordinals_with_format(
    pack_path: &str,
    persisted_pack_format: &str,
) -> Result<BTreeMap<String, u32>, String> {
    let index =
        read_tree_pack_index_without_ordinals_with_format(pack_path, persisted_pack_format)?;
    TreePackIndexJson::stateless()
        .entries_by_ordinal_relaxed(&index)?
        .into_iter()
        .map(|(ordinal, entry)| {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| format!("Tree pack physical ordinal exceeds u32: {ordinal}"))?;
            Ok((entry.tree_id.to_ascii_lowercase(), ordinal))
        })
        .collect()
}

pub(in crate::pack_substrate) fn read_zstd_chunked_tree_pack_index_without_ordinals(
    pack_path: &str,
) -> Result<JsonValue, String> {
    read_zstd_chunked_tree_pack_index(pack_path)
}

pub fn read_tree_pack_tree(pack_path: &str, tree_id: &str) -> Result<JsonValue, String> {
    read_tree_pack_tree_with_format(pack_path, tree_id, DEFAULT_TREE_PACK_WRITE_FORMAT)
}

pub fn read_tree_pack_tree_with_format(
    pack_path: &str,
    tree_id: &str,
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    tree_pack_backend_from_persisted_format(persisted_pack_format)?
        .read_tree_pack_tree(pack_path, tree_id)
}

pub(in crate::pack_substrate) fn read_zstd_chunked_tree_pack_tree(
    pack_path: &str,
    tree_id: &str,
) -> Result<JsonValue, String> {
    let pack_index = read_zstd_chunked_container_index(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_TREE,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    let member = pack_index
        .members
        .iter()
        .find(|entry| entry.content_id == tree_id)
        .cloned()
        .ok_or_else(|| format!("missing tree_id: {tree_id}"))?;
    read_zstd_chunked_tree_member(pack_path, &pack_index, &member)
}

pub fn read_tree_pack_tree_by_ordinal(
    pack_path: &str,
    entry_ordinal: usize,
) -> Result<JsonValue, String> {
    read_tree_pack_tree_by_ordinal_with_format(
        pack_path,
        entry_ordinal,
        DEFAULT_TREE_PACK_WRITE_FORMAT,
    )
}

pub fn read_tree_pack_tree_by_ordinal_with_format(
    pack_path: &str,
    entry_ordinal: usize,
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    tree_pack_backend_from_persisted_format(persisted_pack_format)?
        .read_tree_pack_tree_by_ordinal(pack_path, entry_ordinal)
}

pub fn read_tree_pack_tree_by_entry_name_with_format(
    pack_path: &str,
    tree_id: &str,
    entry_name: &str,
    entry_count: usize,
    checksum: &str,
    persisted_pack_format: &str,
) -> Result<JsonValue, String> {
    tree_pack_backend_from_persisted_format(persisted_pack_format)?
        .read_tree_pack_tree_by_entry_name(pack_path, tree_id, entry_name, entry_count, checksum)
}

pub(in crate::pack_substrate) fn read_zstd_chunked_tree_pack_tree_by_ordinal(
    pack_path: &str,
    entry_ordinal: usize,
) -> Result<JsonValue, String> {
    let pack_index = read_zstd_chunked_container_index(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_TREE,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    let member = pack_index
        .members
        .iter()
        .find(|entry| entry.member_ordinal == entry_ordinal)
        .cloned()
        .ok_or_else(|| format!("missing tree pack entry_ordinal: {entry_ordinal}"))?;
    let rows = read_zstd_chunked_tree_member(pack_path, &pack_index, &member)?;
    Ok(json!({
        "tree_id": member.content_id,
        "entry_ordinal": member.member_ordinal,
        "rows": rows,
    }))
}

pub(in crate::pack_substrate) fn read_zstd_chunked_tree_pack_tree_by_entry_name(
    pack_path: &str,
    tree_id: &str,
    entry_name: &str,
    entry_count: usize,
    checksum: &str,
) -> Result<JsonValue, String> {
    let pack_index = read_zstd_chunked_container_index(
        pack_path,
        ZSTD_CHUNKED_INDEX_KIND_TREE,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )?;
    let member = pack_index
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
    read_zstd_chunked_tree_member(pack_path, &pack_index, &member)
}

pub(in crate::pack_substrate) fn tree_pack_rows_from_payload_bytes(
    raw: &[u8],
    entry: &TreePackIndexEntry,
) -> Result<JsonValue, String> {
    if raw.len() != entry.byte_length {
        return Err(format!(
            "Tree pack entry byte length mismatch for {}",
            entry.tree_id
        ));
    }
    if sha256_hex(raw) != entry.checksum {
        return Err(format!(
            "Tree pack entry checksum mismatch for {}",
            entry.tree_id
        ));
    }
    let payload = JsonCodec::parse_slice_with_error_prefix(raw, "Invalid tree pack entry payload")
        .map_err(String::from)?;
    let payload_obj = as_object(&payload, "tree pack payload")?;
    if optional_text_field(payload_obj, "tree_id").as_deref() != Some(entry.tree_id.as_str()) {
        return Err(format!(
            "Tree pack entry tree_id mismatch for {}",
            entry.tree_id
        ));
    }
    let rows = as_array(
        payload_obj.get("entries").ok_or_else(|| {
            format!(
                "Tree pack entry payload missing entries for {}",
                entry.tree_id
            )
        })?,
        "entries",
    )?;
    if rows.len() != entry.entry_count {
        return Err(format!(
            "Tree pack entry count mismatch for {}",
            entry.tree_id
        ));
    }
    let mut out = Vec::new();
    for row in rows {
        let row_obj = as_object(row, "tree pack row")?;
        out.push(json!({
            "tree_id": entry.tree_id,
            "entry_name": required_text_field(row_obj, "entry_name")?,
            "entry_type": required_text_field(row_obj, "entry_type")?,
            "target_id": required_text_field(row_obj, "target_id")?,
            "size_bytes": row_obj.get("size_bytes").cloned().unwrap_or(JsonValue::Null),
            "mode": required_text_field(row_obj, "mode")?,
        }));
    }
    Ok(JsonValue::Array(out))
}

pub(in crate::pack_substrate) fn zstd_chunked_tree_member_payload_bytes(
    member: &TreePackMember,
) -> Result<Vec<u8>, String> {
    let entry = TreePackIndexEntry {
        tree_id: member.tree_id.clone(),
        entry_ordinal: 0,
        entry_count: member.entry_count,
        byte_length: member.data.len(),
        checksum: member.checksum.clone(),
    };
    let rows = tree_pack_rows_from_payload_bytes(&member.data, &entry)?;
    encode_zstd_chunked_tree_payload(&member.tree_id, member.entry_count, &rows)
}

pub(in crate::pack_substrate) fn encode_zstd_chunked_tree_payload(
    tree_id: &str,
    entry_count: usize,
    rows: &JsonValue,
) -> Result<Vec<u8>, String> {
    let rows = as_array(rows, "tree pack rows")?;
    if rows.len() != entry_count {
        return Err(format!("Tree pack entry count mismatch for {tree_id}"));
    }
    let mut out = Vec::new();
    out.extend_from_slice(ZSTD_CHUNKED_TREE_MEMBER_MAGIC);
    push_u32(&mut out, ZSTD_CHUNKED_VERSION);
    push_string(&mut out, tree_id)?;
    push_u32(&mut out, usize_to_u32(entry_count, "tree_entry_count")?);
    for row in rows {
        let row_obj = as_object(row, "tree pack row")?;
        if required_text_field(row_obj, "tree_id")? != tree_id {
            return Err(format!("Tree pack entry tree_id mismatch for {tree_id}"));
        }
        push_string(&mut out, &required_text_field(row_obj, "entry_name")?)?;
        push_string(&mut out, &required_text_field(row_obj, "entry_type")?)?;
        push_string(&mut out, &required_text_field(row_obj, "target_id")?)?;
        push_option_usize(&mut out, optional_usize_field(row_obj, "size_bytes")?)?;
        push_string(&mut out, &required_text_field(row_obj, "mode")?)?;
    }
    Ok(out)
}

pub(in crate::pack_substrate) fn tree_pack_rows_from_zstd_chunked_binary_payload(
    raw: &[u8],
    entry: &TreePackIndexEntry,
) -> Result<JsonValue, String> {
    if raw.len() != entry.byte_length {
        return Err(format!(
            "Tree pack entry byte length mismatch for {}",
            entry.tree_id
        ));
    }
    if sha256_hex(raw) != entry.checksum {
        return Err(format!(
            "Tree pack entry checksum mismatch for {}",
            entry.tree_id
        ));
    }
    let mut cursor = 0usize;
    if read_bytes(
        raw,
        &mut cursor,
        ZSTD_CHUNKED_TREE_MEMBER_MAGIC.len(),
        "tree member magic",
    )? != ZSTD_CHUNKED_TREE_MEMBER_MAGIC
    {
        return Err("Invalid zstd tree-pack member: bad payload magic".to_string());
    }
    let version = read_u32(raw, &mut cursor, "tree member version")?;
    if version != ZSTD_CHUNKED_VERSION {
        return Err(format!(
            "Invalid zstd tree-pack member: unsupported payload version {version}"
        ));
    }
    let tree_id = read_string(raw, &mut cursor, "tree_id")?;
    if tree_id != entry.tree_id {
        return Err(format!(
            "Tree pack entry tree_id mismatch for {}",
            entry.tree_id
        ));
    }
    let entry_count = read_u32(raw, &mut cursor, "tree_entry_count")? as usize;
    if entry_count != entry.entry_count {
        return Err(format!(
            "Tree pack entry count mismatch for {}",
            entry.tree_id
        ));
    }
    let mut out = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let entry_name = read_string(raw, &mut cursor, "entry_name")?;
        let entry_type = read_string(raw, &mut cursor, "entry_type")?;
        let target_id = read_string(raw, &mut cursor, "target_id")?;
        let size_bytes = read_option_usize(raw, &mut cursor, "size_bytes")?
            .map(|size| JsonValue::Number(Number::from(size as u64)))
            .unwrap_or(JsonValue::Null);
        let mode = read_string(raw, &mut cursor, "mode")?;
        out.push(json!({
            "tree_id": entry.tree_id,
            "entry_name": entry_name,
            "entry_type": entry_type,
            "target_id": target_id,
            "size_bytes": size_bytes,
            "mode": mode,
        }));
    }
    if cursor != raw.len() {
        return Err("Invalid zstd tree-pack member: trailing payload bytes".to_string());
    }
    Ok(JsonValue::Array(out))
}

impl TreePackBackend for ZstdChunkedTreePackBackend {
    fn format_kind(&self) -> TreePackFormatKind {
        TreePackFormatKind::ZstdChunkedTreeV1
    }

    fn write_tree_pack_archive(
        &self,
        pack_path: &str,
        pack_id: &str,
        created_at: &str,
        members: &JsonValue,
    ) -> Result<JsonValue, String> {
        write_zstd_chunked_tree_pack_archive(pack_path, pack_id, created_at, members)
    }

    fn read_tree_pack_index(&self, pack_path: &str) -> Result<JsonValue, String> {
        read_zstd_chunked_tree_pack_index(pack_path)
    }

    fn read_tree_pack_index_without_ordinals(&self, pack_path: &str) -> Result<JsonValue, String> {
        read_zstd_chunked_tree_pack_index_without_ordinals(pack_path)
    }

    fn read_tree_pack_tree(&self, pack_path: &str, tree_id: &str) -> Result<JsonValue, String> {
        read_zstd_chunked_tree_pack_tree(pack_path, tree_id)
    }

    fn read_tree_pack_tree_by_ordinal(
        &self,
        pack_path: &str,
        entry_ordinal: usize,
    ) -> Result<JsonValue, String> {
        read_zstd_chunked_tree_pack_tree_by_ordinal(pack_path, entry_ordinal)
    }

    fn read_tree_pack_tree_by_entry_name(
        &self,
        pack_path: &str,
        tree_id: &str,
        entry_name: &str,
        entry_count: usize,
        checksum: &str,
    ) -> Result<JsonValue, String> {
        read_zstd_chunked_tree_pack_tree_by_entry_name(
            pack_path,
            tree_id,
            entry_name,
            entry_count,
            checksum,
        )
    }
}

pub fn summarize_tree_pack_archives(
    root: &str,
    pack_rows: &JsonValue,
) -> Result<JsonValue, String> {
    let rows = as_array(pack_rows, "pack_rows")?;
    let root = PathBuf::from(root);
    let mut summary = json!({
        "pack_count": 0,
        "archive_bytes": 0,
        "indexed_tree_count": 0,
        "indexed_entry_count": 0,
        "index_error_count": 0,
    });
    let Some(summary_obj) = summary.as_object_mut() else {
        return Err("summary payload must be an object".to_string());
    };
    for row in rows {
        increment_u64(summary_obj, "pack_count", 1);
        let row_obj = as_object(row, "tree pack row")?;
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
            "archive_bytes",
            pack_abs.metadata().map_err(|err| err.to_string())?.len(),
        );
        let pack_index = match read_tree_pack_index_with_format(
            path_to_string(&pack_abs)?.as_str(),
            &pack_format,
        ) {
            Ok(value) => value,
            Err(_) => {
                increment_u64(summary_obj, "index_error_count", 1);
                continue;
            }
        };
        increment_u64(
            summary_obj,
            "indexed_tree_count",
            pack_index
                .get("tree_count")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0),
        );
        let trees = as_array(
            pack_index
                .get("trees")
                .ok_or_else(|| "tree pack index missing trees".to_string())?,
            "trees",
        )?;
        let total_entries = trees
            .iter()
            .filter_map(|entry| {
                as_object(entry, "tree pack index entry")
                    .ok()
                    .and_then(|obj| optional_usize_field(obj, "entry_count").ok().flatten())
                    .map(|value| value as u64)
            })
            .sum::<u64>();
        increment_u64(summary_obj, "indexed_entry_count", total_entries);
    }
    Ok(summary)
}
