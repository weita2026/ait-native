use super::*;
use crate::json_support::{
    optional_text_field as json_optional_text_field, required_array_value, required_object_value,
    JsonCodec, JsonEncodeOptions,
};

pub(in crate::pack_substrate) fn tree_payload_bytes(
    tree_id: &str,
    tree_entry_rows: &[JsonValue],
) -> Result<Vec<u8>, String> {
    let mut entries = tree_entry_rows
        .iter()
        .map(|row| {
            let row_obj = as_object(row, "tree entry row")?;
            Ok(json!({
                "entry_name": required_text_field(row_obj, "entry_name")?,
                "entry_type": required_text_field(row_obj, "entry_type")?,
                "target_id": required_text_field(row_obj, "target_id")?,
                "size_bytes": row_obj.get("size_bytes").cloned().unwrap_or(JsonValue::Null),
                "mode": required_text_field(row_obj, "mode")?,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    entries.sort_by_key(|row| {
        row.get("entry_name")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string()
    });
    json_bytes_compact_sorted(&json!({
        "tree_id": tree_id,
        "entries": entries,
    }))
}

pub(in crate::pack_substrate) fn member_to_json(member: &ObjectPackWriteMember) -> JsonValue {
    let mut payload = Map::new();
    payload.insert(
        "entry_name".to_string(),
        JsonValue::String(member.entry_name.clone()),
    );
    payload.insert(
        "blob_id".to_string(),
        JsonValue::String(member.blob_id.clone()),
    );
    payload.insert(
        "data".to_string(),
        JsonValue::Array(
            member
                .data
                .iter()
                .map(|byte| JsonValue::Number(Number::from(*byte)))
                .collect(),
        ),
    );
    if let Some(logical_data) = &member.logical_data {
        payload.insert(
            "logical_data".to_string(),
            JsonValue::Array(
                logical_data
                    .iter()
                    .map(|byte| JsonValue::Number(Number::from(*byte)))
                    .collect(),
            ),
        );
    }
    payload.insert(
        "entry_type".to_string(),
        JsonValue::String(member.entry_type.clone()),
    );
    payload.insert(
        "base_blob_id".to_string(),
        member
            .base_blob_id
            .as_ref()
            .map(|value| JsonValue::String(value.clone()))
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "chain_depth".to_string(),
        JsonValue::Number(Number::from(member.chain_depth as u64)),
    );
    if let Some(value) = &member.delta_algorithm {
        payload.insert(
            "delta_algorithm".to_string(),
            JsonValue::String(value.clone()),
        );
    }
    JsonValue::Object(payload)
}

pub(in crate::pack_substrate) fn parse_pack_candidates(
    value: &JsonValue,
) -> Result<Vec<PackCandidate>, String> {
    let rows = as_array(value, "blob_items")?;
    rows.iter()
        .map(|row| {
            let row_obj = as_object(row, "blob item")?;
            Ok(PackCandidate {
                entry_name: required_text_field(row_obj, "entry_name")?,
                blob_id: required_text_field(row_obj, "blob_id")?,
                data: required_bytes_field(row_obj, "data")?,
                path_hint: optional_text_field(row_obj, "path_hint"),
                chain_depth: optional_usize_field(row_obj, "chain_depth")?.unwrap_or(0),
            })
        })
        .collect()
}

pub(in crate::pack_substrate) fn parse_initial_by_path(
    value: Option<&JsonValue>,
) -> Result<BTreeMap<String, PackCandidate>, String> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let value_obj = as_object(value, "initial_by_path")?;
    let mut out = BTreeMap::new();
    for (path_hint, row) in value_obj {
        if path_hint.trim().is_empty() {
            continue;
        }
        let row_obj = as_object(row, "initial_by_path row")?;
        out.insert(
            path_hint.clone(),
            PackCandidate {
                entry_name: format!("blobs/{}", required_text_field(row_obj, "blob_id")?),
                blob_id: required_text_field(row_obj, "blob_id")?,
                data: required_bytes_field(row_obj, "data")?,
                path_hint: Some(path_hint.clone()),
                chain_depth: optional_usize_field(row_obj, "chain_depth")?.unwrap_or(0),
            },
        );
    }
    Ok(out)
}

pub(in crate::pack_substrate) fn parse_pack_members(
    value: &JsonValue,
) -> Result<Vec<PackMember>, String> {
    let rows = as_array(value, "members")?;
    rows.iter()
        .map(|row| {
            let row_obj = as_object(row, "pack member")?;
            let data = required_bytes_field(row_obj, "data")?;
            Ok(PackMember {
                entry_name: required_text_field(row_obj, "entry_name")?,
                blob_id: required_text_field(row_obj, "blob_id")?,
                logical_data: optional_bytes_field(row_obj, "logical_data")?
                    .unwrap_or_else(|| data.clone()),
                data,
                entry_type: optional_text_field(row_obj, "entry_type")
                    .unwrap_or_else(|| "full".to_string()),
                base_blob_id: optional_text_field(row_obj, "base_blob_id"),
                chain_depth: optional_usize_field(row_obj, "chain_depth")?.unwrap_or(0),
                delta_algorithm: optional_text_field(row_obj, "delta_algorithm"),
            })
        })
        .collect()
}

pub(in crate::pack_substrate) fn parse_tree_pack_members(
    value: &JsonValue,
) -> Result<Vec<TreePackMember>, String> {
    let rows = as_array(value, "tree pack members")?;
    rows.iter()
        .map(|row| {
            let row_obj = as_object(row, "tree pack member")?;
            let data = required_bytes_field(row_obj, "data")?;
            Ok(TreePackMember {
                tree_id: required_text_field(row_obj, "tree_id")?,
                entry_name: required_text_field(row_obj, "entry_name")?,
                entry_count: required_usize_field(row_obj, "entry_count")?,
                checksum: optional_text_field(row_obj, "checksum")
                    .unwrap_or_else(|| sha256_hex(&data)),
                data,
            })
        })
        .collect()
}

pub(in crate::pack_substrate) fn pack_entries_by_name(
    pack_index: &JsonValue,
) -> Result<BTreeMap<String, PackIndexEntry>, String> {
    let pack_index_obj = as_object(pack_index, "pack index")?;
    let entries = as_array(
        pack_index_obj
            .get("entries")
            .ok_or_else(|| "Invalid pack index: missing entries list".to_string())?,
        "entries",
    )?;
    let mut out = BTreeMap::new();
    for entry in entries {
        let entry_obj = as_object(entry, "pack index entry")?;
        let entry_name = required_text_field(entry_obj, "entry_name")?;
        let parsed = PackIndexEntry {
            entry_name: entry_name.clone(),
            blob_id: required_text_field(entry_obj, "blob_id")?,
            entry_type: required_text_field(entry_obj, "entry_type")?,
            byte_length: required_usize_field(entry_obj, "byte_length")?,
            uncompressed_byte_length: optional_usize_field(entry_obj, "uncompressed_byte_length")?
                .unwrap_or(required_usize_field(entry_obj, "byte_length")?),
            base_blob_id: optional_text_field(entry_obj, "base_blob_id"),
            chain_depth: optional_usize_field(entry_obj, "chain_depth")?.unwrap_or(0),
            checksum: required_text_field(entry_obj, "checksum")?,
            delta_algorithm: optional_text_field(entry_obj, "delta_algorithm"),
        };
        if let Some(existing) = out.get(&entry_name) {
            if pack_index_entries_equivalent(existing, &parsed) {
                continue;
            }
            return Err(format!(
                "Invalid pack index: duplicate entry_name {entry_name}"
            ));
        }
        out.insert(entry_name, parsed);
    }
    Ok(out)
}

pub(in crate::pack_substrate) fn tree_entries_by_ordinal_relaxed(
    pack_index: &JsonValue,
) -> Result<BTreeMap<usize, TreePackIndexEntry>, String> {
    let pack_index_obj = as_object(pack_index, "tree pack index")?;
    let trees = as_array(
        pack_index_obj
            .get("trees")
            .ok_or_else(|| "Invalid tree pack index: missing trees list".to_string())?,
        "trees",
    )?;
    let mut out = BTreeMap::new();
    for (fallback_ordinal, entry) in trees.iter().enumerate() {
        let entry_obj = as_object(entry, "tree pack index entry")?;
        let parsed = TreePackIndexEntry {
            tree_id: required_text_field(entry_obj, "tree_id")?,
            entry_ordinal: optional_usize_field(entry_obj, "entry_ordinal")?
                .unwrap_or(fallback_ordinal),
            entry_count: required_usize_field(entry_obj, "entry_count")?,
            byte_length: required_usize_field(entry_obj, "byte_length")?,
            checksum: required_text_field(entry_obj, "checksum")?,
        };
        if out.contains_key(&parsed.entry_ordinal) {
            return Err(format!(
                "Invalid tree pack index: duplicate entry_ordinal {}",
                parsed.entry_ordinal
            ));
        }
        out.insert(parsed.entry_ordinal, parsed);
    }
    Ok(out)
}

pub(in crate::pack_substrate) fn pack_index_entries_equivalent(
    left: &PackIndexEntry,
    right: &PackIndexEntry,
) -> bool {
    left.entry_name == right.entry_name
        && left.blob_id == right.blob_id
        && left.entry_type == right.entry_type
        && left.byte_length == right.byte_length
        && left.uncompressed_byte_length == right.uncompressed_byte_length
        && left.base_blob_id == right.base_blob_id
        && left.chain_depth == right.chain_depth
        && left.checksum == right.checksum
        && left.delta_algorithm == right.delta_algorithm
}

pub(in crate::pack_substrate) fn delta_match_granularity(
    base_len: usize,
    target_len: usize,
) -> usize {
    let max_size = base_len.max(target_len);
    if max_size < 8 * 1024 {
        1
    } else if max_size < 64 * 1024 {
        8
    } else {
        32
    }
}

pub(in crate::pack_substrate) fn chunk_bytes(data: &[u8], unit: usize) -> Vec<Vec<u8>> {
    if unit <= 1 {
        return data.iter().map(|byte| vec![*byte]).collect();
    }
    data.chunks(unit).map(|chunk| chunk.to_vec()).collect()
}

pub(in crate::pack_substrate) fn encode_size_varint(mut value: usize) -> Vec<u8> {
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

pub(in crate::pack_substrate) fn decode_size_varint(
    data: &[u8],
    mut offset: usize,
) -> Result<(usize, usize), String> {
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

pub(in crate::pack_substrate) fn append_copy_instructions(
    out: &mut Vec<u8>,
    offset: usize,
    size: usize,
) {
    let mut remaining = size;
    let mut current_offset = offset;
    while remaining > 0 {
        let chunk_size = remaining.min(0xFFFF);
        out.extend(encode_copy_instruction(current_offset, chunk_size));
        current_offset += chunk_size;
        remaining -= chunk_size;
    }
}

pub(in crate::pack_substrate) fn encode_copy_instruction(offset: usize, size: usize) -> Vec<u8> {
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

pub(in crate::pack_substrate) fn append_insert_instructions(out: &mut Vec<u8>, data: &[u8]) {
    let mut cursor = 0usize;
    while cursor < data.len() {
        let end = (cursor + 0x7f).min(data.len());
        let chunk = &data[cursor..end];
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
        cursor = end;
    }
}

pub(in crate::pack_substrate) fn sha256_hex(data: &[u8]) -> String {
    hex_string(&sha256_bytes(data))
}

pub(in crate::pack_substrate) fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    digest.into()
}

pub(in crate::pack_substrate) fn hex_string(data: &[u8]) -> String {
    data.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(in crate::pack_substrate) fn json_bytes_compact_sorted(
    value: &JsonValue,
) -> Result<Vec<u8>, String> {
    let normalized = sorted_json_value(value);
    JsonCodec::encode_value_to_vec_with_error_prefix(
        &normalized,
        JsonEncodeOptions::compact(),
        "failed to serialize JSON payload",
    )
    .map_err(String::from)
}

pub(in crate::pack_substrate) fn sorted_json_value(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Array(entries) => {
            JsonValue::Array(entries.iter().map(sorted_json_value).collect())
        }
        JsonValue::Object(entries) => {
            let mut keys = entries.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                if let Some(entry) = entries.get(&key) {
                    out.insert(key, sorted_json_value(entry));
                }
            }
            JsonValue::Object(out)
        }
        _ => value.clone(),
    }
}

pub(in crate::pack_substrate) fn ensure_available(
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

pub(in crate::pack_substrate) fn increment_u64(
    map: &mut Map<String, JsonValue>,
    key: &str,
    amount: u64,
) {
    let current = map.get(key).and_then(JsonValue::as_u64).unwrap_or(0);
    map.insert(
        key.to_string(),
        JsonValue::Number(Number::from(current + amount)),
    );
}

pub(in crate::pack_substrate) fn path_to_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(|value| value.to_string())
        .ok_or_else(|| "path must be valid UTF-8".to_string())
}

pub(in crate::pack_substrate) fn as_array<'a>(
    value: &'a JsonValue,
    field_name: &str,
) -> Result<&'a [JsonValue], String> {
    required_array_value(value, field_name).map_err(|_| format!("{field_name} must be a list."))
}

pub(in crate::pack_substrate) fn as_object<'a>(
    value: &'a JsonValue,
    field_name: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    required_object_value(value, field_name).map_err(|_| format!("{field_name} must be an object."))
}

pub(in crate::pack_substrate) fn required_text_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<String, String> {
    optional_text_field(obj, field_name)
        .ok_or_else(|| format!("missing required field: {field_name}"))
}

pub(in crate::pack_substrate) fn optional_text_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Option<String> {
    json_optional_text_field(obj, field_name)
        .ok()
        .flatten()
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.to_string())
}

pub(in crate::pack_substrate) fn required_usize_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<usize, String> {
    optional_usize_field(obj, field_name)?
        .ok_or_else(|| format!("missing required integer field: {field_name}"))
}

pub(in crate::pack_substrate) fn optional_usize_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = obj.get(field_name) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(number) = value.as_u64() {
        return Ok(Some(number as usize));
    }
    if let Some(number) = value.as_i64() {
        if number < 0 {
            return Err(format!("{field_name} must be non-negative"));
        }
        return Ok(Some(number as usize));
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let parsed = trimmed
            .parse::<u64>()
            .map_err(|_| format!("{field_name} must be an integer"))?;
        return Ok(Some(parsed as usize));
    }
    Err(format!("{field_name} must be an integer"))
}

pub(in crate::pack_substrate) fn required_bytes_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<Vec<u8>, String> {
    optional_bytes_field(obj, field_name)?
        .ok_or_else(|| format!("missing required bytes field: {field_name}"))
}

pub(in crate::pack_substrate) fn optional_bytes_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let Some(value) = obj.get(field_name) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(text) = value.as_str() {
        return Ok(Some(text.as_bytes().to_vec()));
    }
    if let Some(values) = value.as_array() {
        let mut out = Vec::with_capacity(values.len());
        for entry in values {
            let Some(byte) = entry.as_u64() else {
                return Err(format!("{field_name} byte arrays must contain integers"));
            };
            if byte > 255 {
                return Err(format!("{field_name} byte arrays must stay within 0..=255"));
            }
            out.push(byte as u8);
        }
        return Ok(Some(out));
    }
    Err(format!(
        "{field_name} must be bytes encoded as string or byte array"
    ))
}
