use super::*;

pub(super) fn tree_payload_bytes(
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

pub(super) fn member_to_json(member: &PackMember) -> JsonValue {
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
    if member.logical_data != member.data {
        payload.insert(
            "logical_data".to_string(),
            JsonValue::Array(
                member
                    .logical_data
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

pub(super) fn parse_pack_candidates(value: &JsonValue) -> Result<Vec<PackCandidate>, String> {
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

pub(super) fn parse_initial_by_path(
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

pub(super) fn parse_pack_members(value: &JsonValue) -> Result<Vec<PackMember>, String> {
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

pub(super) fn parse_tree_pack_members(value: &JsonValue) -> Result<Vec<TreePackMember>, String> {
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

pub(super) fn pack_entries_by_name(
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
            uncompressed_byte_length: required_usize_field(entry_obj, "uncompressed_byte_length")?,
            base_blob_id: optional_text_field(entry_obj, "base_blob_id"),
            chain_depth: required_usize_field(entry_obj, "chain_depth")?,
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

pub(super) fn tree_entries_by_id(
    pack_index: &JsonValue,
) -> Result<BTreeMap<String, TreePackIndexEntry>, String> {
    let pack_index_obj = as_object(pack_index, "tree pack index")?;
    let trees = as_array(
        pack_index_obj
            .get("trees")
            .ok_or_else(|| "Invalid tree pack index: missing trees list".to_string())?,
        "trees",
    )?;
    let mut out = BTreeMap::new();
    for entry in trees {
        let entry_obj = as_object(entry, "tree pack index entry")?;
        let tree_id = required_text_field(entry_obj, "tree_id")?;
        required_text_field(entry_obj, "entry_name")?;
        required_usize_field(entry_obj, "byte_length")?;
        let parsed = TreePackIndexEntry {
            tree_id: tree_id.clone(),
            entry_ordinal: required_usize_field(entry_obj, "entry_ordinal")?,
            entry_count: required_usize_field(entry_obj, "entry_count")?,
            checksum: required_text_field(entry_obj, "checksum")?,
        };
        if out.contains_key(&tree_id) {
            return Err(format!(
                "Invalid tree pack index: duplicate tree_id {tree_id}"
            ));
        }
        out.insert(tree_id, parsed);
    }
    Ok(out)
}

pub(super) fn validate_current_pack_index_header(
    pack_index: &JsonValue,
    expected_format: &str,
    expected_index_entry_name: &str,
    label: &str,
) -> Result<(), String> {
    let object = as_object(pack_index, label)?;
    let pack_format = required_text_field(object, "pack_format")?;
    if pack_format != expected_format {
        return Err(format!(
            "Invalid {label} index: unsupported pack_format '{pack_format}'"
        ));
    }
    let index_entry_name = required_text_field(object, "index_entry_name")?;
    if index_entry_name != expected_index_entry_name {
        return Err(format!(
            "Invalid {label} index: unsupported index_entry_name '{index_entry_name}'"
        ));
    }
    Ok(())
}

pub(super) fn pack_index_entries_equivalent(left: &PackIndexEntry, right: &PackIndexEntry) -> bool {
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

pub(super) fn sha256_hex(data: &[u8]) -> String {
    hex_string(&sha256_bytes(data))
}

pub(super) fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    digest.into()
}

pub(super) fn hex_string(data: &[u8]) -> String {
    data.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn json_bytes_compact_sorted(value: &JsonValue) -> Result<Vec<u8>, String> {
    let normalized = sorted_json_value(value);
    serde_json::to_vec(&normalized)
        .map_err(|err| format!("failed to serialize JSON payload: {err}"))
}

pub(super) fn sorted_json_value(value: &JsonValue) -> JsonValue {
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

pub(super) fn increment_u64(map: &mut Map<String, JsonValue>, key: &str, amount: u64) {
    let current = map.get(key).and_then(JsonValue::as_u64).unwrap_or(0);
    map.insert(
        key.to_string(),
        JsonValue::Number(Number::from(current + amount)),
    );
}

pub(super) fn path_to_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(|value| value.to_string())
        .ok_or_else(|| "path must be valid UTF-8".to_string())
}

pub(super) fn as_array<'a>(
    value: &'a JsonValue,
    field_name: &str,
) -> Result<&'a [JsonValue], String> {
    value
        .as_array()
        .map(|value| value.as_slice())
        .ok_or_else(|| format!("{field_name} must be a list."))
}

pub(super) fn as_object<'a>(
    value: &'a JsonValue,
    field_name: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{field_name} must be an object."))
}

pub(super) fn required_text_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<String, String> {
    optional_text_field(obj, field_name)
        .ok_or_else(|| format!("missing required field: {field_name}"))
}

pub(super) fn optional_text_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Option<String> {
    obj.get(field_name)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.to_string())
}

pub(super) fn required_usize_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<usize, String> {
    optional_usize_field(obj, field_name)?
        .ok_or_else(|| format!("missing required integer field: {field_name}"))
}

pub(super) fn optional_usize_field(
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

pub(super) fn required_bytes_field(
    obj: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<Vec<u8>, String> {
    optional_bytes_field(obj, field_name)?
        .ok_or_else(|| format!("missing required bytes field: {field_name}"))
}

pub(super) fn optional_bytes_field(
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
