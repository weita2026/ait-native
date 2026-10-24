use super::*;
pub(in crate::pack_substrate) fn zstd_chunked_tree_pack_index_json(
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

pub(in crate::pack_substrate) fn read_zstd_chunked_tree_member(
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
        entry_count,
        byte_length: member.stored_len,
        checksum: member.checksum.clone(),
    };
    tree_pack_rows_from_zstd_chunked_binary_payload(&raw, &entry)
}

pub(in crate::pack_substrate) fn read_zstd_chunked_tree_member_with_chunk_cache(
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
        entry_count,
        byte_length: member.stored_len,
        checksum: member.checksum.clone(),
    };
    tree_pack_rows_from_zstd_chunked_binary_payload(&raw, &entry)
}
