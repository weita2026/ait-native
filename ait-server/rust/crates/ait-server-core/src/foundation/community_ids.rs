use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value as JsonValue};

pub const COMMUNITY_IDS_CONTRACT_VERSION: &str = "ait.server.community_ids.v1";
pub const CROCKFORD_BASE32: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

const RANDOMNESS_BYTE_LEN: usize = 10;
const TIMESTAMP_TEXT_LEN: usize = 10;
const RANDOMNESS_TEXT_LEN: usize = 16;

pub fn community_ids_contract() -> JsonValue {
    json!({
        "contract": COMMUNITY_IDS_CONTRACT_VERSION,
        "reference_modules": [],
        "migration_status": "python_wrapper_removed_rust_owned",
        "alphabet": CROCKFORD_BASE32,
        "ulid_shape": {
            "timestamp_ms_base32_length": TIMESTAMP_TEXT_LEN,
            "randomness_base32_length": RANDOMNESS_TEXT_LEN,
            "randomness_bytes": RANDOMNESS_BYTE_LEN,
            "ulid_length": TIMESTAMP_TEXT_LEN + RANDOMNESS_TEXT_LEN,
        },
        "operations": [
            "encode-crockford-base32",
            "build",
            "generate",
        ],
        "compatibility_notes": {
            "python_reference": "The former community ID Python wrapper has been removed; callers should use this Rust contract through explicit server APIs.",
            "community_auth": "Community account, password, and web-session persistence remain separate security follow-up scope.",
            "task_dag": "Task DAG is retired and is not a community ID surface.",
        },
    })
}

pub fn community_ids_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "community id payload must be a JSON object.".to_string())?;
    match operation {
        "contract" => Ok(community_ids_contract()),
        "encode-crockford-base32" => {
            let value = required_u128(payload.get("value"), "value")?;
            let length = required_usize(payload.get("length"), "length")?;
            Ok(json!({
                "contract": COMMUNITY_IDS_CONTRACT_VERSION,
                "encoded": encode_crockford_base32(value, length)?,
            }))
        }
        "build" => {
            let prefix = required_text(payload.get("prefix"), "prefix")?;
            let timestamp_ms = required_u64(payload.get("timestamp_ms"), "timestamp_ms")?;
            let randomness = randomness_from_hex(&required_text(
                payload.get("randomness_hex"),
                "randomness_hex",
            )?)?;
            Ok(json!({
                "contract": COMMUNITY_IDS_CONTRACT_VERSION,
                "id": build_community_id(&prefix, timestamp_ms, &randomness)?,
            }))
        }
        "generate" => {
            let prefix = required_text(payload.get("prefix"), "prefix")?;
            Ok(json!({
                "contract": COMMUNITY_IDS_CONTRACT_VERSION,
                "id": generate_community_id(&prefix)?,
            }))
        }
        other => Err(format!("Unsupported community id operation `{other}`.")),
    }
}

pub fn generate_community_id(prefix: &str) -> Result<String, String> {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|exc| format!("System time is before UNIX epoch: {exc}"))?
        .as_millis()
        .try_into()
        .map_err(|_| "Current timestamp does not fit u64 milliseconds.".to_string())?;
    let mut randomness = [0_u8; RANDOMNESS_BYTE_LEN];
    getrandom::fill(&mut randomness)
        .map_err(|exc| format!("Failed to read secure randomness: {exc}"))?;
    build_community_id(prefix, timestamp_ms, &randomness)
}

pub fn build_community_id(
    prefix: &str,
    timestamp_ms: u64,
    randomness: &[u8; RANDOMNESS_BYTE_LEN],
) -> Result<String, String> {
    Ok(format!(
        "{prefix}-{}{}",
        encode_crockford_base32(u128::from(timestamp_ms), TIMESTAMP_TEXT_LEN)?,
        encode_crockford_base32(randomness_to_u128(randomness), RANDOMNESS_TEXT_LEN)?
    ))
}

pub fn encode_crockford_base32(mut value: u128, length: usize) -> Result<String, String> {
    let alphabet = CROCKFORD_BASE32.as_bytes();
    let mut chars = vec![b'0'; length];
    for idx in (0..length).rev() {
        chars[idx] = alphabet[(value & 0b11111) as usize];
        value >>= 5;
    }
    if value != 0 {
        return Err("Value does not fit requested Crockford base32 length".to_string());
    }
    String::from_utf8(chars).map_err(|exc| format!("Crockford base32 output is invalid: {exc}"))
}

pub fn randomness_from_hex(value: &str) -> Result<[u8; RANDOMNESS_BYTE_LEN], String> {
    let text = value.trim();
    if text.len() != RANDOMNESS_BYTE_LEN * 2 {
        return Err(format!(
            "randomness_hex must contain exactly {} hex characters.",
            RANDOMNESS_BYTE_LEN * 2
        ));
    }
    let mut bytes = [0_u8; RANDOMNESS_BYTE_LEN];
    for idx in 0..RANDOMNESS_BYTE_LEN {
        bytes[idx] = u8::from_str_radix(&text[idx * 2..idx * 2 + 2], 16)
            .map_err(|_| "randomness_hex must contain only hexadecimal characters.".to_string())?;
    }
    Ok(bytes)
}

fn randomness_to_u128(randomness: &[u8; RANDOMNESS_BYTE_LEN]) -> u128 {
    randomness
        .iter()
        .fold(0_u128, |acc, byte| (acc << 8) | u128::from(*byte))
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    let text = match value {
        Some(JsonValue::String(text)) => text.trim().to_string(),
        Some(JsonValue::Number(number)) => number.to_string(),
        _ => String::new(),
    };
    if text.is_empty() {
        Err(format!("Field `{field}` must be non-empty."))
    } else {
        Ok(text)
    }
}

fn required_u64(value: Option<&JsonValue>, field: &str) -> Result<u64, String> {
    let value = required_u128(value, field)?;
    value
        .try_into()
        .map_err(|_| format!("Field `{field}` must fit u64."))
}

fn required_usize(value: Option<&JsonValue>, field: &str) -> Result<usize, String> {
    let value = required_u128(value, field)?;
    value
        .try_into()
        .map_err(|_| format!("Field `{field}` must fit usize."))
}

fn required_u128(value: Option<&JsonValue>, field: &str) -> Result<u128, String> {
    match value {
        Some(JsonValue::Number(number)) => number
            .as_u64()
            .map(u128::from)
            .ok_or_else(|| format!("Field `{field}` must be a non-negative integer.")),
        Some(JsonValue::String(text)) => text
            .trim()
            .parse::<u128>()
            .map_err(|_| format!("Field `{field}` must be a non-negative integer.")),
        _ => Err(format!("Field `{field}` must be a non-negative integer.")),
    }
}
