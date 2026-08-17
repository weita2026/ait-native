use super::*;
#[expect(
    clippy::too_many_arguments,
    reason = "container writer receives explicit format and compression contract fields"
)]
pub(in crate::pack_substrate) fn write_zstd_chunked_container(
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

pub(in crate::pack_substrate) fn write_zstd_chunked_raw_chunk(
    file: &mut File,
    chunk_ordinal: usize,
    raw: &[u8],
) -> Result<ZstdChunkedChunkIndex, String> {
    let compressed = ::zstd::bulk::compress(raw, ZSTD_CHUNKED_LEVEL)
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

pub(in crate::pack_substrate) fn read_zstd_chunked_member_stored_bytes(
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

pub(in crate::pack_substrate) fn read_zstd_chunked_member_stored_bytes_from_bytes(
    pack_bytes: &[u8],
    pack_index: &ZstdChunkedPackIndex,
    member: &ZstdChunkedMemberIndex,
) -> Result<Vec<u8>, String> {
    let chunk = pack_index
        .chunks
        .get(member.chunk_ordinal)
        .ok_or_else(|| format!("missing zstd chunk {}", member.chunk_ordinal))?;
    let compressed_start = usize::try_from(chunk.compressed_offset)
        .map_err(|_| "Invalid zstd chunked pack: chunk offset overflow".to_string())?;
    let compressed_end = compressed_start
        .checked_add(chunk.compressed_len)
        .ok_or_else(|| "Invalid zstd chunked pack: chunk range overflow".to_string())?;
    if compressed_end > pack_bytes.len() {
        return Err(format!(
            "Invalid zstd chunked pack: compressed range exceeds container for chunk {}",
            chunk.chunk_ordinal
        ));
    }
    let raw =
        ::zstd::bulk::decompress(&pack_bytes[compressed_start..compressed_end], chunk.raw_len)
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
    let member_end = member
        .in_chunk_offset
        .checked_add(member.stored_len)
        .ok_or_else(|| "Invalid zstd chunked pack: member range overflow".to_string())?;
    if member_end > raw.len() {
        return Err(format!(
            "Invalid zstd chunked pack: member range exceeds chunk for {}",
            member.entry_name
        ));
    }
    Ok(raw[member.in_chunk_offset..member_end].to_vec())
}

pub(in crate::pack_substrate) fn read_zstd_chunked_member_stored_bytes_with_chunk_cache(
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

pub(in crate::pack_substrate) fn read_zstd_chunked_raw_chunk(
    pack_path: &str,
    chunk: &ZstdChunkedChunkIndex,
) -> Result<Vec<u8>, String> {
    let _trace = crate::perfetto_range!("ait.core.pack.zstd.read_and_decompress_raw_chunk");
    let compressed = read_file_range(pack_path, chunk.compressed_offset, chunk.compressed_len)?;
    let raw = ::zstd::bulk::decompress(&compressed, chunk.raw_len)
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

pub(in crate::pack_substrate) fn read_file_range(
    path: &str,
    offset: u64,
    len: usize,
) -> Result<Vec<u8>, String> {
    let _trace = crate::perfetto_range!("ait.core.pack.file_range.open_seek_read");
    let mut file = File::open(path).map_err(|err| format!("failed to open pack archive: {err}"))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|err| format!("failed to seek pack archive: {err}"))?;
    let mut out = vec![0u8; len];
    file.read_exact(&mut out)
        .map_err(|err| format!("failed to read pack archive range: {err}"))?;
    Ok(out)
}
