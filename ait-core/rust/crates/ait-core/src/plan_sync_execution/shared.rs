use super::*;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct LocalPlanId(String);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RemotePlanId(String);

impl LocalPlanId {
    pub(super) fn from_plan(plan: &JsonValue) -> Result<Self, String> {
        require_plan_id(plan).map(Self)
    }

    pub(super) fn from_raw(value: impl Into<String>) -> Result<Self, String> {
        authority_plan_id(value.into(), "local").map(Self)
    }

    pub(super) fn raw(&self) -> &str {
        &self.0
    }

    pub(super) fn reference(&self) -> String {
        authority_plan_reference("LPR", self.raw())
    }
}

impl RemotePlanId {
    pub(super) fn from_plan(plan: &JsonValue) -> Result<Self, String> {
        require_plan_id(plan).map(Self)
    }

    pub(super) fn from_raw(value: impl Into<String>) -> Result<Self, String> {
        authority_plan_id(value.into(), "remote").map(Self)
    }

    pub(super) fn raw(&self) -> &str {
        &self.0
    }

    pub(super) fn reference(&self) -> String {
        authority_plan_reference("RPR", self.raw())
    }
}

fn authority_plan_id(value: String, authority: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(format!("{authority} Plan identity cannot be empty."));
    }
    Ok(normalized.to_string())
}

fn authority_plan_reference(prefix: &str, raw: &str) -> String {
    raw.strip_prefix("PR-")
        .map(|suffix| format!("{prefix}-{suffix}"))
        .unwrap_or_else(|| format!("{prefix}[{raw}]"))
}

pub(super) fn plan_lineage_identity_matches(
    left_path: &str,
    left_selector: Option<&str>,
    right_path: &str,
    right_selector: Option<&str>,
) -> bool {
    let left_selector = normalize_optional_text(left_selector);
    let right_selector = normalize_optional_text(right_selector);
    match (left_selector, right_selector) {
        (Some(left), Some(right)) => left == right,
        (None, None) => left_path == right_path,
        _ => false,
    }
}

pub(super) fn plan_revisions_share_lineage(revisions: &[JsonValue]) -> Result<bool, String> {
    let Some(first_revision) = revisions.first() else {
        return Ok(true);
    };
    let first_path = text_field(first_revision, "artifact_path").ok_or_else(|| {
        format!(
            "Plan revision {:?} has no artifact path.",
            text_field(first_revision, "plan_revision_id")
        )
    })?;
    let first_selector = text_field(first_revision, "artifact_selector");
    for revision in revisions.iter().skip(1) {
        let path = text_field(revision, "artifact_path").ok_or_else(|| {
            format!(
                "Plan revision {:?} has no artifact path.",
                text_field(revision, "plan_revision_id")
            )
        })?;
        let selector = text_field(revision, "artifact_selector");
        if !plan_lineage_identity_matches(
            &first_path,
            first_selector.as_deref(),
            &path,
            selector.as_deref(),
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>()
}

pub(super) fn json_i64_field(value: &JsonValue, key: &str) -> Result<i64, String> {
    value_get(value, key)
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| format!("Missing required integer field `{key}`."))
}

pub(super) fn json_string_field(value: &JsonValue, key: &str) -> Result<String, String> {
    value_get(value, key)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("Missing required string field `{key}`."))
}

pub(super) fn remote_head_revision_id(plan: &JsonValue) -> Option<String> {
    head_text(plan, "plan_revision_id").or_else(|| text_field(plan, "head_revision_id"))
}

pub(super) fn local_head_revision_id(plan: &JsonValue) -> Option<String> {
    head_text(plan, "plan_revision_id").or_else(|| text_field(plan, "head_revision_id"))
}

pub(super) fn require_plan_id(plan: &JsonValue) -> Result<String, String> {
    text_field(plan, "plan_id").ok_or_else(|| "Plan payload is missing plan_id.".to_string())
}

pub(super) fn require_plan_revision_id(revision: &JsonValue) -> Result<String, String> {
    text_field(revision, "plan_revision_id")
        .ok_or_else(|| "Plan revision payload is missing plan_revision_id.".to_string())
}

pub(super) fn head_text(plan: &JsonValue, key: &str) -> Option<String> {
    plan.as_object()
        .and_then(|object| object.get("head_revision"))
        .and_then(|value| value.as_object())
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn head_value(plan: &JsonValue, key: &str) -> Option<JsonValue> {
    plan.as_object()
        .and_then(|object| object.get("head_revision"))
        .and_then(|value| value.as_object())
        .and_then(|value| value.get(key))
        .cloned()
}

pub(super) fn text_field(value: &JsonValue, key: &str) -> Option<String> {
    value_get(value, key)
        .and_then(|entry| entry.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub(super) fn raw_string_field(value: &JsonValue, key: &str) -> Option<String> {
    value_get(value, key)
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
}

pub(super) fn required_string_field(value: &JsonValue, key: &str) -> Result<String, String> {
    text_field(value, key).ok_or_else(|| format!("Missing required string field `{key}`."))
}

pub(super) fn source_text(value: Option<&JsonValue>) -> JsonValue {
    value
        .and_then(|entry| entry.as_str())
        .map(|entry| JsonValue::String(entry.to_string()))
        .unwrap_or(JsonValue::Null)
}

pub(super) fn value_get<'a>(value: &'a JsonValue, key: &str) -> Option<&'a JsonValue> {
    value.as_object().and_then(|object| object.get(key))
}

pub(super) fn as_array(value: Option<&JsonValue>) -> Result<&[JsonValue], String> {
    value
        .ok_or_else(|| "Expected array payload field.".to_string())?
        .as_array()
        .map(|rows| rows.as_slice())
        .ok_or_else(|| "Expected array payload field.".to_string())
}

pub(super) fn require_nonempty_text(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<String, String> {
    optional_text(value)?.ok_or_else(|| {
        format!("Plan sync command execution field `{field_name}` must be a non-empty string.")
    })
}

pub(super) fn optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let text = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "Plan sync command execution text fields must be strings.".to_string())?;
    Ok(Some(text.to_string()))
}

pub(super) fn optional_text_allow_empty(
    value: Option<&JsonValue>,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|text| Some(text.to_string()))
        .ok_or_else(|| "Plan sync command execution text fields must be strings.".to_string())
}

pub(super) fn optional_bool(value: Option<&JsonValue>, default: bool) -> Result<bool, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(default),
        Some(JsonValue::Bool(flag)) => Ok(*flag),
        Some(_) => Err("Plan sync command execution boolean fields must be booleans.".to_string()),
    }
}

pub(super) fn optional_u32(value: Option<&JsonValue>) -> Result<Option<u32>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let number = value
        .as_i64()
        .ok_or_else(|| "Plan sync command execution u32 fields must be numbers.".to_string())?;
    u32::try_from(number)
        .map(Some)
        .map_err(|_| format!("Plan sync command execution u32 field is out of range: {number}."))
}

pub(super) fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub(super) fn build_http_client_manager(
    request: &SyncRequest,
    base_url: &str,
) -> Result<PlanHttpClientManager, String> {
    PlanHttpClientManager::new(PlanHttpClientConfig {
        base_url: base_url.to_string(),
        repository_index: request.repository_index,
        headers: plan_sync_http_headers(request),
        default_timeout_ms: DEFAULT_TIMEOUT_MS,
        retry_attempts: 0,
        retry_backoff_ms: 0,
        pool_max_idle_per_host: 1,
    })
    .map_err(|err| err.to_string())
}

pub(super) fn plan_sync_http_headers(request: &SyncRequest) -> BTreeMap<String, String> {
    plan_sync_http_headers_from_values(
        env_first(&[crate::environment_contract::names::AIT_NATIVE_ACTOR]),
        request.created_by.as_deref(),
    )
}

pub(super) fn plan_sync_http_headers_from_values(
    actor: Option<String>,
    fallback_actor: Option<&str>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    if let Some(value) = actor.or_else(|| normalize_optional_text(fallback_actor)) {
        headers.insert("X-AIT-Actor".to_string(), value);
    }
    headers
}

pub(super) fn env_first(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        env::var(name)
            .ok()
            .and_then(|value| normalize_optional_text(Some(&value)))
    })
}

pub(super) fn is_historical_status(value: Option<&JsonValue>) -> bool {
    value
        .and_then(|entry| entry.as_str())
        .map(|text| {
            matches!(
                text.trim().to_ascii_lowercase().as_str(),
                "archived" | "superseded"
            )
        })
        .unwrap_or(false)
}

pub(super) fn is_forbidden_sync_markdown_path(path_value: &str) -> bool {
    let normalized = path_value
        .replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase();
    normalized == "docs/sprints/readme.md"
}

pub(super) fn forbidden_sync_markdown_message(path_value: &str) -> String {
    let normalized = path_value.replace('\\', "/").trim_matches('/').to_string();
    format!(
        "{normalized} is reserved and cannot be used with `ait plan sync`. Use a real sprint artifact path such as docs/sprints/<card>.md instead."
    )
}

pub(super) fn plan_artifact_identity_label(
    artifact_path: &str,
    artifact_selector: Option<&str>,
) -> String {
    artifact_selector
        .map(|selector| format!("{artifact_path} [{selector}]"))
        .unwrap_or_else(|| artifact_path.to_string())
}

pub(super) fn plan_fs_error(err: PlanFilesystemError) -> String {
    match err {
        PlanFilesystemError::Invalid(message)
        | PlanFilesystemError::NotFound(message)
        | PlanFilesystemError::MissingEntry(message)
        | PlanFilesystemError::Io(message) => message,
    }
}
