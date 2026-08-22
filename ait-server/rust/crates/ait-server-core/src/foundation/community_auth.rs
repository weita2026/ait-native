use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{Duration, SecondsFormat, TimeZone, Utc};
use scrypt::{scrypt, Params as ScryptParams};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub const COMMUNITY_AUTH_CONTRACT_VERSION: &str = "ait.server.community_auth.v1";
pub const COMMUNITY_AUTH_REFERENCE_MODULE: &str = "../ait/src/ait_native/community_auth.py";

pub const PASSWORD_SCRYPT_N: u64 = 1 << 14;
pub const PASSWORD_SCRYPT_R: u32 = 8;
pub const PASSWORD_SCRYPT_P: u32 = 1;
pub const PASSWORD_SCRYPT_DKLEN: usize = 64;
pub const COMMUNITY_SESSION_TTL_DAYS: i64 = 14;

const SALT_BYTE_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityPasswordParams {
    pub salt: Vec<u8>,
    pub n: u64,
    pub r: u32,
    pub p: u32,
    pub dklen: usize,
}

impl CommunityPasswordParams {
    pub fn default_with_salt(salt: Vec<u8>) -> Self {
        Self {
            salt,
            n: PASSWORD_SCRYPT_N,
            r: PASSWORD_SCRYPT_R,
            p: PASSWORD_SCRYPT_P,
            dklen: PASSWORD_SCRYPT_DKLEN,
        }
    }

    pub fn to_python_json(&self) -> String {
        format!(
            "{{\"dklen\":{},\"n\":{},\"p\":{},\"r\":{},\"salt_b64\":\"{}\"}}",
            self.dklen,
            self.n,
            self.p,
            self.r,
            BASE64_STANDARD.encode(&self.salt)
        )
    }
}

pub fn community_auth_contract() -> JsonValue {
    json!({
        "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
        "reference_modules": [COMMUNITY_AUTH_REFERENCE_MODULE],
        "password": {
            "algorithm": "scrypt",
            "n": PASSWORD_SCRYPT_N,
            "r": PASSWORD_SCRYPT_R,
            "p": PASSWORD_SCRYPT_P,
            "dklen": PASSWORD_SCRYPT_DKLEN,
            "salt_bytes": SALT_BYTE_LEN,
        },
        "session": {
            "ttl_days": COMMUNITY_SESSION_TTL_DAYS,
            "actor_type": "community_user",
            "default_session_source": "password",
        },
        "operations": [
            "normalize-email",
            "actor-identity",
            "validate-password",
            "password-params",
            "hash-password",
            "verify-password",
            "expires-at",
            "session-payload",
        ],
        "compatibility_notes": {
            "python_reference": "ait_native.community_auth is the compatibility caller for this Rust contract after the ait_server module deletion.",
            "persistence": "Community account writes, credential table writes, web-session table writes, lookup, revocation, and role-binding queries remain follow-up service scope.",
            "task_dag": "Task DAG is retired and is not a community auth surface.",
        },
    })
}

pub fn community_auth_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "community auth payload must be a JSON object.".to_string())?;
    match operation {
        "contract" => Ok(community_auth_contract()),
        "normalize-email" => Ok(json!({
            "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
            "email_normalized": normalize_community_email(value_text(payload.get("email")).as_deref()),
        })),
        "actor-identity" => {
            let account_id = required_text(payload.get("account_id"), "account_id")?;
            Ok(json!({
                "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
                "actor_identity": community_actor_identity(&account_id),
            }))
        }
        "validate-password" => {
            let password = validate_password(value_text(payload.get("password")).as_deref())?;
            Ok(json!({
                "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
                "password": password,
            }))
        }
        "password-params" => {
            let salt = salt_from_payload(payload)?;
            let params = CommunityPasswordParams::default_with_salt(salt);
            Ok(json!({
                "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
                "password_algo": "scrypt",
                "password_params_json": params.to_python_json(),
            }))
        }
        "hash-password" => {
            let password = required_text(payload.get("password"), "password")?;
            let salt = salt_from_payload(payload)?;
            let (hash, algo, params_json) = hash_community_password_with_salt(&password, &salt)?;
            Ok(json!({
                "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
                "password_hash": hash,
                "password_algo": algo,
                "password_params_json": params_json,
            }))
        }
        "verify-password" => {
            let password = required_text(payload.get("password"), "password")?;
            let password_hash = required_text(payload.get("password_hash"), "password_hash")?;
            let password_algo = required_text(payload.get("password_algo"), "password_algo")?;
            let password_params_json =
                required_text(payload.get("password_params_json"), "password_params_json")?;
            Ok(json!({
                "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
                "verified": verify_community_password(
                    &password,
                    &password_hash,
                    &password_algo,
                    &password_params_json,
                )?,
            }))
        }
        "expires-at" => {
            let now = required_text(payload.get("now"), "now")?;
            Ok(json!({
                "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
                "expires_at": expires_at_text(&now)?,
            }))
        }
        "session-payload" => {
            let account_row = required_object(payload.get("account_row"), "account_row")?;
            let session_row = required_object(payload.get("session_row"), "session_row")?;
            Ok(json!({
                "contract": COMMUNITY_AUTH_CONTRACT_VERSION,
                "session": session_payload(account_row, session_row)?,
            }))
        }
        other => Err(format!("Unsupported community auth operation `{other}`.")),
    }
}

pub fn normalize_community_email(value: Option<&str>) -> Option<String> {
    normalize_optional_text(value).map(|text| text.to_lowercase())
}

pub fn community_actor_identity(account_id: &str) -> String {
    format!("community:{account_id}")
}

pub fn validate_password(value: Option<&str>) -> Result<String, String> {
    let text = normalize_optional_text(value).ok_or_else(|| "Password is required.".to_string())?;
    if text.len() < 10 {
        return Err("Password must be at least 10 characters.".to_string());
    }
    Ok(text)
}

pub fn hash_community_password_with_salt(
    password: &str,
    salt: &[u8],
) -> Result<(String, String, String), String> {
    let params = CommunityPasswordParams::default_with_salt(salt.to_vec());
    let digest = hash_password_bytes(password, &params)?;
    Ok((
        hex_encode(&digest),
        "scrypt".to_string(),
        params.to_python_json(),
    ))
}

pub fn verify_community_password(
    password: &str,
    password_hash: &str,
    password_algo: &str,
    password_params_json: &str,
) -> Result<bool, String> {
    if password_algo.trim().to_lowercase() != "scrypt" {
        return Err(format!(
            "Unsupported Community password algorithm: {:?}",
            password_algo
        ));
    }
    let params = load_password_params(password_params_json)?;
    let expected = hex_decode(password_hash)?;
    let actual = hash_password_bytes(password, &params)?;
    Ok(constant_time_eq(&expected, &actual))
}

pub fn expires_at_text(now: &str) -> Result<String, String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(now)
        .map_err(|exc| format!("Field `now` must be an RFC3339 timestamp: {exc}"))?
        .with_timezone(&Utc);
    Ok((parsed + Duration::days(COMMUNITY_SESSION_TTL_DAYS))
        .to_rfc3339_opts(SecondsFormat::AutoSi, false))
}

pub fn session_payload(
    account_row: &JsonMap<String, JsonValue>,
    session_row: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let account_id = required_text(account_row.get("account_id"), "account_row.account_id")?;
    let email_normalized =
        normalize_json_text(account_row.get("email_normalized")).unwrap_or_default();
    let display_name = normalize_json_text(account_row.get("display_name"))
        .or_else(|| normalize_json_text(account_row.get("full_name")))
        .unwrap_or_else(|| email_normalized.clone());
    let full_name =
        normalize_json_text(account_row.get("full_name")).unwrap_or_else(|| display_name.clone());
    Ok(json!({
        "account_id": account_id,
        "actor_identity": community_actor_identity(&account_id),
        "actor_type": "community_user",
        "display_name": display_name,
        "full_name": full_name,
        "email_normalized": email_normalized,
        "organization": normalize_json_text(account_row.get("organization")),
        "role_title": normalize_json_text(account_row.get("role_title")),
        "status": normalize_json_text(account_row.get("status")).unwrap_or_else(|| "active".to_string()),
        "primary_auth_method": normalize_json_text(account_row.get("primary_auth_method")).unwrap_or_else(|| "password".to_string()),
        "web_session_id": required_text(session_row.get("web_session_id"), "session_row.web_session_id")?,
        "session_source": normalize_json_text(session_row.get("session_source")).unwrap_or_else(|| "password".to_string()),
        "created_at": normalize_json_text(session_row.get("created_at")).unwrap_or_default(),
        "expires_at": normalize_json_text(session_row.get("expires_at")).unwrap_or_default(),
        "revoked_at": normalize_json_text(session_row.get("revoked_at")),
        "last_seen_at": normalize_json_text(session_row.get("last_seen_at")),
    }))
}

fn salt_from_payload(payload: &JsonMap<String, JsonValue>) -> Result<Vec<u8>, String> {
    if let Some(text) = value_text(payload.get("salt_hex")) {
        return hex_decode(&text);
    }
    let mut salt = [0_u8; SALT_BYTE_LEN];
    getrandom::fill(&mut salt)
        .map_err(|exc| format!("Failed to read secure password salt: {exc}"))?;
    Ok(salt.to_vec())
}

fn load_password_params(raw: &str) -> Result<CommunityPasswordParams, String> {
    let payload: JsonValue = serde_json::from_str(raw)
        .map_err(|exc| format!("Invalid password parameter JSON: {exc}"))?;
    let payload = payload
        .as_object()
        .ok_or_else(|| "Password parameter payload must be a JSON object.".to_string())?;
    let salt_b64 = value_text(payload.get("salt_b64")).unwrap_or_default();
    let salt = BASE64_STANDARD
        .decode(salt_b64.as_bytes())
        .map_err(|exc| format!("Invalid password salt base64: {exc}"))?;
    Ok(CommunityPasswordParams {
        salt,
        n: optional_u64(payload.get("n")).unwrap_or(PASSWORD_SCRYPT_N),
        r: optional_u64(payload.get("r"))
            .unwrap_or(u64::from(PASSWORD_SCRYPT_R))
            .try_into()
            .map_err(|_| "Password scrypt r does not fit u32.".to_string())?,
        p: optional_u64(payload.get("p"))
            .unwrap_or(u64::from(PASSWORD_SCRYPT_P))
            .try_into()
            .map_err(|_| "Password scrypt p does not fit u32.".to_string())?,
        dklen: optional_u64(payload.get("dklen"))
            .unwrap_or(PASSWORD_SCRYPT_DKLEN as u64)
            .try_into()
            .map_err(|_| "Password scrypt dklen does not fit usize.".to_string())?,
    })
}

fn hash_password_bytes(
    password: &str,
    params: &CommunityPasswordParams,
) -> Result<Vec<u8>, String> {
    let log_n = scrypt_log_n(params.n)?;
    let scrypt_params = ScryptParams::new(log_n, params.r, params.p, params.dklen)
        .map_err(|exc| format!("Invalid scrypt parameters: {exc}"))?;
    let mut output = vec![0_u8; params.dklen];
    scrypt(
        password.as_bytes(),
        params.salt.as_slice(),
        &scrypt_params,
        &mut output,
    )
    .map_err(|exc| format!("Failed to hash Community password: {exc}"))?;
    Ok(output)
}

fn scrypt_log_n(n: u64) -> Result<u8, String> {
    if n == 0 || !n.is_power_of_two() {
        return Err("Password scrypt n must be a power of two.".to_string());
    }
    n.trailing_zeros()
        .try_into()
        .map_err(|_| "Password scrypt n is too large.".to_string())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    let text = value?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn normalize_json_text(value: Option<&JsonValue>) -> Option<String> {
    value_text(value).and_then(|text| normalize_optional_text(Some(&text)))
}

fn value_text(value: Option<&JsonValue>) -> Option<String> {
    match value? {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(true) => Some("True".to_string()),
        JsonValue::Bool(false) => Some("False".to_string()),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn required_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Field `{field}` must be a JSON object."))
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    normalize_json_text(value).ok_or_else(|| format!("Field `{field}` must be non-empty."))
}

fn optional_u64(value: Option<&JsonValue>) -> Option<u64> {
    match value? {
        JsonValue::Number(number) => number.as_u64(),
        JsonValue::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err("hex payload must contain an even number of characters.".to_string());
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for idx in (0..text.len()).step_by(2) {
        bytes
            .push(u8::from_str_radix(&text[idx..idx + 2], 16).map_err(|_| {
                "hex payload must contain only hexadecimal characters.".to_string()
            })?);
    }
    Ok(bytes)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

#[allow(dead_code)]
fn _fixed_utc_for_docs(year: i32, month: u32, day: u32) -> String {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0)
        .unwrap()
        .to_rfc3339_opts(SecondsFormat::Secs, false)
}
