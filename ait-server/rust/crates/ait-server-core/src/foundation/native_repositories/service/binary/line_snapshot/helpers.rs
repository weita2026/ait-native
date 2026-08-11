use super::*;

pub(in super::super) fn now_timestamp_s() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp())
        .expect("current system time must not precede the Unix epoch")
}

pub(super) fn timestamp_s(value: &str) -> Result<u64, NativeRepositoryError> {
    let timestamp = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|err| {
            NativeRepositoryError::bad_request(format!(
                "Binary DB timestamp {value:?} is invalid: {err}"
            ))
        })?
        .timestamp();
    u64::try_from(timestamp).map_err(|_| {
        NativeRepositoryError::bad_request(format!(
            "Binary DB timestamp {value:?} precedes the Unix epoch"
        ))
    })
}

pub(super) fn timestamp_string(value: u64) -> Result<String, NativeRepositoryError> {
    let value = i64::try_from(value).map_err(|_| {
        NativeRepositoryError::internal("Binary DB timestamp exceeds RFC 3339 range")
    })?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(value, 0)
        .map(|value| value.to_rfc3339())
        .ok_or_else(|| {
            NativeRepositoryError::internal("Binary DB timestamp exceeds RFC 3339 range")
        })
}

pub(super) fn decode_optional_sha256(
    value: Option<String>,
) -> Result<[u8; 32], NativeRepositoryError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok([0; 32]);
    };
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(NativeRepositoryError::bad_request(format!(
            "manifest_hash must be 64 hexadecimal characters, got {value:?}"
        )));
    }
    let mut bytes = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16)
            .expect("validated hexadecimal byte");
    }
    Ok(bytes)
}

pub(super) fn manifest_hash_text(value: &[u8; 32]) -> String {
    if value.iter().all(|byte| *byte == 0) {
        String::new()
    } else {
        value.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

pub(super) fn json_u32(value: &JsonValue, field: &str) -> Result<u32, NativeRepositoryError> {
    match value.get(field) {
        None | Some(JsonValue::Null) => Ok(0),
        Some(value) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Binary DB snapshot {field} must be a u32"
                ))
            }),
    }
}

pub(super) fn json_u64(value: &JsonValue, field: &str) -> Result<u64, NativeRepositoryError> {
    match value.get(field) {
        None | Some(JsonValue::Null) => Ok(0),
        Some(value) => value.as_u64().ok_or_else(|| {
            NativeRepositoryError::bad_request(format!("Binary DB snapshot {field} must be a u64"))
        }),
    }
}
