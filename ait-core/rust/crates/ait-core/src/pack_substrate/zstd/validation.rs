use super::*;
pub(in crate::pack_substrate) fn validate_zstd_chunked_container_chunks(
    pack_path: &str,
    kind: u8,
    expected_pack_format: &str,
) -> Result<(), String> {
    let pack_index = read_zstd_chunked_container_index(pack_path, kind, expected_pack_format)?;
    for chunk in &pack_index.chunks {
        read_zstd_chunked_raw_chunk(pack_path, chunk)?;
    }
    Ok(())
}

pub(in crate::pack_substrate) fn validate_zstd_chunked_index(
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
