use super::*;
pub(in crate::pack_substrate) fn read_zstd_chunked_container_index(
    pack_path: &str,
    kind: u8,
    expected_pack_format: &str,
) -> Result<ZstdChunkedPackIndex, String> {
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

pub(in crate::pack_substrate) fn read_zstd_chunked_container_index_from_bytes(
    pack_bytes: &[u8],
    kind: u8,
    expected_pack_format: &str,
) -> Result<ZstdChunkedPackIndex, String> {
    if pack_bytes.len() < ZSTD_CHUNKED_TRAILER_LEN {
        return Err("Invalid zstd chunked pack: truncated trailer".to_string());
    }
    let data_end = pack_bytes.len() - ZSTD_CHUNKED_TRAILER_LEN;
    let trailer = &pack_bytes[data_end..];
    let (index_offset, index_len, expected_checksum) = decode_zstd_chunked_trailer(trailer)?;
    let index_start = usize::try_from(index_offset)
        .map_err(|_| "Invalid zstd chunked pack: index offset overflow".to_string())?;
    let index_end = index_start
        .checked_add(index_len)
        .ok_or_else(|| "Invalid zstd chunked pack: index range overflow".to_string())?;
    if index_end > data_end {
        return Err("Invalid zstd chunked pack: index range exceeds container".to_string());
    }
    let index_bytes = &pack_bytes[index_start..index_end];
    if sha256_bytes(index_bytes) != expected_checksum {
        return Err("Invalid zstd chunked pack: index checksum mismatch".to_string());
    }
    let pack_index = decode_zstd_chunked_index(index_bytes, kind)?;
    validate_zstd_chunked_index(&pack_index, kind, expected_pack_format, Some(index_offset))?;
    Ok(pack_index)
}

pub(in crate::pack_substrate) fn zstd_chunked_container_index_checksum(
    pack_path: &str,
    kind: u8,
    expected_pack_format: &str,
) -> Result<String, String> {
    let pack_index = read_zstd_chunked_container_index(pack_path, kind, expected_pack_format)?;
    let index_bytes = encode_zstd_chunked_index(&pack_index, kind)?;
    Ok(sha256_hex(&index_bytes))
}

pub(in crate::pack_substrate) fn encode_zstd_chunked_index(
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

pub(in crate::pack_substrate) fn decode_zstd_chunked_index(
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

pub(in crate::pack_substrate) fn write_zstd_chunked_trailer(
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

pub(in crate::pack_substrate) fn decode_zstd_chunked_trailer(
    data: &[u8],
) -> Result<(u64, usize, [u8; 32]), String> {
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

pub(in crate::pack_substrate) fn push_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

pub(in crate::pack_substrate) fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(in crate::pack_substrate) fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(in crate::pack_substrate) fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    push_u32(out, usize_to_u32(value.len(), "string_len")?);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

pub(in crate::pack_substrate) fn push_option_string(
    out: &mut Vec<u8>,
    value: Option<&str>,
) -> Result<(), String> {
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

pub(in crate::pack_substrate) fn push_option_usize(
    out: &mut Vec<u8>,
    value: Option<usize>,
) -> Result<(), String> {
    match value {
        Some(value) => {
            push_u8(out, 1);
            push_u64(out, usize_to_u64(value, "optional_usize")?);
        }
        None => push_u8(out, 0),
    }
    Ok(())
}

pub(in crate::pack_substrate) fn push_checksum_hex(
    out: &mut Vec<u8>,
    value: &str,
) -> Result<(), String> {
    out.extend_from_slice(&checksum_bytes_from_hex(value)?);
    Ok(())
}

pub(in crate::pack_substrate) fn read_u8(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<u8, String> {
    Ok(read_bytes(data, cursor, 1, label)?[0])
}

pub(in crate::pack_substrate) fn read_u32(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<u32, String> {
    let bytes = read_bytes(data, cursor, 4, label)?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

pub(in crate::pack_substrate) fn read_u64(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<u64, String> {
    let bytes = read_bytes(data, cursor, 8, label)?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

pub(in crate::pack_substrate) fn read_string(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<String, String> {
    let len = read_u32(data, cursor, label)? as usize;
    let bytes = read_bytes(data, cursor, len, label)?;
    String::from_utf8(bytes.to_vec())
        .map_err(|err| format!("Invalid zstd chunked pack: {label} is not UTF-8: {err}"))
}

pub(in crate::pack_substrate) fn read_option_string(
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

pub(in crate::pack_substrate) fn read_option_usize(
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

pub(in crate::pack_substrate) fn read_checksum_hex(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<String, String> {
    Ok(hex_string(&read_checksum_bytes(data, cursor, label)?))
}

pub(in crate::pack_substrate) fn read_checksum_bytes(
    data: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<[u8; 32], String> {
    let bytes = read_bytes(data, cursor, 32, label)?;
    Ok(bytes.try_into().unwrap())
}

pub(in crate::pack_substrate) fn read_bytes<'a>(
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

pub(in crate::pack_substrate) fn usize_to_u32(value: usize, label: &str) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{label} exceeds u32 capacity"))
}

pub(in crate::pack_substrate) fn usize_to_u64(value: usize, label: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("{label} exceeds u64 capacity"))
}

pub(in crate::pack_substrate) fn checksum_bytes_from_hex(value: &str) -> Result<[u8; 32], String> {
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
