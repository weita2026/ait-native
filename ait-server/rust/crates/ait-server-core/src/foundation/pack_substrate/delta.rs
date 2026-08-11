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

pub(super) fn delta_match_granularity(base_len: usize, target_len: usize) -> usize {
    let max_size = base_len.max(target_len);
    if max_size < 8 * 1024 {
        1
    } else if max_size < 64 * 1024 {
        8
    } else {
        32
    }
}

pub(super) fn chunk_bytes(data: &[u8], unit: usize) -> Vec<Vec<u8>> {
    if unit <= 1 {
        return data.iter().map(|byte| vec![*byte]).collect();
    }
    data.chunks(unit).map(|chunk| chunk.to_vec()).collect()
}

pub(super) fn encode_size_varint(mut value: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value > 0 {
            out.push(byte | 0x80);
        } else {
            out.push(byte);
            break;
        }
    }
    out
}

pub(super) fn decode_size_varint(data: &[u8], mut offset: usize) -> Result<(usize, usize), String> {
    let mut value = 0usize;
    let mut shift = 0usize;
    loop {
        if offset >= data.len() {
            return Err("Invalid pack delta payload: truncated size header".to_string());
        }
        let byte = data[offset];
        offset += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if (byte & 0x80) == 0 {
            return Ok((value, offset));
        }
        shift += 7;
        if shift > 63 {
            return Err("Invalid pack delta payload: size header is too large".to_string());
        }
    }
}

pub(super) fn append_copy_instructions(out: &mut Vec<u8>, offset: usize, size: usize) {
    let mut remaining = size;
    let mut current_offset = offset;
    while remaining > 0 {
        let chunk_size = remaining.min(0xFFFF);
        out.extend(encode_copy_instruction(current_offset, chunk_size));
        current_offset += chunk_size;
        remaining -= chunk_size;
    }
}

pub(super) fn encode_copy_instruction(offset: usize, size: usize) -> Vec<u8> {
    let mut command = 0x80u8;
    let mut payload = Vec::new();
    for (bit, shift) in [0usize, 8, 16, 24].iter().enumerate() {
        let byte = ((offset >> shift) & 0xFF) as u8;
        if byte != 0 {
            command |= 1 << bit;
            payload.push(byte);
        }
    }
    for (bit, shift) in [0usize, 8, 16].iter().enumerate() {
        let byte = ((size >> shift) & 0xFF) as u8;
        if byte != 0 {
            command |= 1 << (bit + 4);
            payload.push(byte);
        }
    }
    let mut out = vec![command];
    out.extend(payload);
    out
}

pub(super) fn append_insert_instructions(out: &mut Vec<u8>, data: &[u8]) {
    let mut cursor = 0usize;
    while cursor < data.len() {
        let end = (cursor + 0x7f).min(data.len());
        let chunk = &data[cursor..end];
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
        cursor = end;
    }
}

pub(super) fn ensure_available(
    data: &[u8],
    cursor: usize,
    needed: usize,
    reason: &str,
) -> Result<(), String> {
    if cursor + needed > data.len() {
        Err(format!("Invalid pack delta payload: {reason}"))
    } else {
        Ok(())
    }
}
