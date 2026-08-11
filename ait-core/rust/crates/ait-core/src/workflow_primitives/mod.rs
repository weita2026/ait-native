use getrandom::getrandom;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_ID_NAMESPACE_PREFIX: &str = "";
pub const LEGACY_ID_NAMESPACE_PREFIX: &str = "AIT";
pub const LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX: &str = "L";
pub const REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX: &str = "R";
pub const WORKFLOW_ID_FAMILIES: &[&str] = &[
    "T", "C", "P", "R", "S", "PS", "K", "PL", "PR", "SK", "HP", "AM", "AN", "AMU", "STH",
];
pub const WORKFLOW_TASK_CHANGE_ORIGIN_NAMESPACE_PREFIXES: &[&str] = &[
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX,
    REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX,
];
pub const RESERVED_WORKFLOW_TOKENS: &[&str] = &["AT", "LAND", "W"];
const CROCKFORD_BASE32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckboxState {
    Open,
    Done,
    None,
}

impl CheckboxState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
            Self::None => "none",
        }
    }

    pub fn from_normalized_state(value: &str) -> Option<Self> {
        match value.trim() {
            "open" => Some(Self::Open),
            "done" => Some(Self::Done),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn from_markdown_checked(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("x") | Some("X") => Self::Done,
            Some(_) => Self::Open,
            None => Self::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowMode {
    SoloLocal,
    SoloRemote,
    TeamRemote,
}

impl WorkflowMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SoloLocal => "solo_local",
            Self::SoloRemote => "solo_remote",
            Self::TeamRemote => "team_remote",
        }
    }

    pub fn parse(value: Option<&str>) -> Result<Option<Self>, String> {
        let Some(normalized) = normalize_optional_text(value) else {
            return Ok(None);
        };
        match normalized.as_str() {
            "solo_local" => Ok(Some(Self::SoloLocal)),
            "solo_remote" => Ok(Some(Self::SoloRemote)),
            "team_remote" => Ok(Some(Self::TeamRemote)),
            _ => Err(format!(
                "Unsupported workflow mode: {}. Expected one of: solo_local, solo_remote, team_remote.",
                normalized
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicationState {
    LocalDraft,
    Published,
}

impl PublicationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LocalDraft => "local_draft",
            Self::Published => "published",
        }
    }

    pub fn parse(value: Option<&str>) -> Result<Option<Self>, String> {
        let Some(normalized) = normalize_optional_text(value) else {
            return Ok(None);
        };
        match normalized.as_str() {
            "local_draft" => Ok(Some(Self::LocalDraft)),
            "published" => Ok(Some(Self::Published)),
            _ => Err(format!(
                "Unsupported publication state: {}. Expected one of: local_draft, published.",
                normalized
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Active,
    Completed,
    Abandoned,
    LaterPromotionExcluded,
    LegacyCanceled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
            Self::LaterPromotionExcluded => "later_promotion_excluded",
            Self::LegacyCanceled => "canceled",
        }
    }

    pub fn parse(value: Option<&str>) -> Result<Option<Self>, String> {
        let Some(normalized) = normalize_optional_text(value) else {
            return Ok(None);
        };
        match normalized.as_str() {
            "active" => Ok(Some(Self::Active)),
            "completed" => Ok(Some(Self::Completed)),
            "abandoned" => Ok(Some(Self::Abandoned)),
            "later_promotion_excluded" => Ok(Some(Self::LaterPromotionExcluded)),
            "canceled" => Ok(Some(Self::LegacyCanceled)),
            _ => Err(format!(
                "Unsupported task status: {}. Expected one of: active, completed, abandoned, later_promotion_excluded, canceled.",
                normalized
            )),
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::LaterPromotionExcluded => "later-promotion-excluded",
            Self::Abandoned | Self::LegacyCanceled => "abandoned",
            Self::Active => "active",
        }
    }

    pub fn is_closed(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Abandoned | Self::LaterPromotionExcluded | Self::LegacyCanceled
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowErrorEnvelope {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowResultEnvelope {
    pub ok: bool,
    pub kind: String,
    pub value: Option<String>,
    pub error: Option<WorkflowErrorEnvelope>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowStatusDetails {
    pub normalized_status: Option<String>,
    pub display_label: Option<String>,
    pub closed: bool,
}

pub fn workflow_mode_value(value: Option<&str>) -> Result<Option<String>, String> {
    Ok(WorkflowMode::parse(value)?.map(|mode| mode.as_str().to_string()))
}

pub fn publication_state_value(value: Option<&str>) -> Result<Option<String>, String> {
    Ok(PublicationState::parse(value)?.map(|state| state.as_str().to_string()))
}

pub fn publication_state_has_unpublished_head(value: Option<&str>) -> Result<bool, String> {
    Ok(matches!(
        PublicationState::parse(value)?,
        Some(PublicationState::LocalDraft)
    ))
}

pub fn task_status_value(value: Option<&str>) -> Result<Option<String>, String> {
    Ok(TaskStatus::parse(value)?.map(|status| status.as_str().to_string()))
}

pub fn task_status_details(value: Option<&str>) -> Result<WorkflowStatusDetails, String> {
    let status = TaskStatus::parse(value)?;
    Ok(WorkflowStatusDetails {
        normalized_status: status.as_ref().map(|value| value.as_str().to_string()),
        display_label: status
            .as_ref()
            .map(|value| value.display_label().to_string()),
        closed: status.as_ref().map(TaskStatus::is_closed).unwrap_or(false),
    })
}

pub fn normalize_id_namespace_prefix(
    value: Option<&str>,
    default: Option<&str>,
) -> Result<String, String> {
    let raw = value.or(default);
    let text = raw
        .map(|value| value.trim().to_uppercase())
        .unwrap_or_default();
    if raw.is_none() && default.is_none() {
        return Err("id namespace prefix is required".to_string());
    }
    if !text
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
    {
        return Err("id namespace prefix must contain only ASCII letters or digits".to_string());
    }
    for code in WORKFLOW_ID_FAMILIES {
        let token = if text.is_empty() {
            code.to_string()
        } else {
            format!("{text}{code}")
        };
        if RESERVED_WORKFLOW_TOKENS.contains(&token.as_str()) {
            return Err(format!(
                "id namespace prefix {:?} collides with reserved workflow token {:?}",
                text, token
            ));
        }
    }
    Ok(text)
}

pub fn workflow_id_token(family: &str, namespace_prefix: Option<&str>) -> Result<String, String> {
    let resolved_family = normalize_workflow_id_family(family)?;
    let namespace =
        normalize_id_namespace_prefix(namespace_prefix, Some(DEFAULT_ID_NAMESPACE_PREFIX))?;
    if namespace.is_empty() {
        Ok(resolved_family)
    } else {
        Ok(format!("{namespace}{resolved_family}"))
    }
}

pub fn workflow_id_matches(
    value: Option<&str>,
    family: &str,
    namespace_prefix: Option<&str>,
    include_legacy: bool,
) -> Result<bool, String> {
    let text = value.unwrap_or("").trim().to_uppercase();
    if !text.contains('-') {
        return Ok(false);
    }
    let token = text.split_once('-').map(|(left, _)| left).unwrap_or("");
    let mut tokens = vec![workflow_id_token(family, namespace_prefix)?];
    let legacy = workflow_id_token(family, Some(LEGACY_ID_NAMESPACE_PREFIX))?;
    if include_legacy && !tokens.contains(&legacy) {
        tokens.push(legacy);
    }
    Ok(tokens.iter().any(|candidate| candidate == token))
}

pub fn workflow_origin_namespace_prefix(
    origin_prefix: &str,
    namespace_prefix: Option<&str>,
) -> Result<String, String> {
    let resolved_origin = origin_prefix.trim().to_uppercase();
    if !WORKFLOW_TASK_CHANGE_ORIGIN_NAMESPACE_PREFIXES
        .iter()
        .any(|candidate| *candidate == resolved_origin)
    {
        return Err(format!(
            "Unsupported workflow origin prefix: {:?}",
            origin_prefix
        ));
    }
    let namespace =
        normalize_id_namespace_prefix(namespace_prefix, Some(DEFAULT_ID_NAMESPACE_PREFIX))?;
    if namespace.is_empty() {
        Ok(resolved_origin)
    } else {
        Ok(format!("{resolved_origin}{namespace}"))
    }
}

pub fn workflow_id_tokens(
    family: &str,
    namespace_prefix: Option<&str>,
    include_legacy: bool,
) -> Result<Vec<String>, String> {
    let mut tokens = vec![workflow_id_token(family, namespace_prefix)?];
    let legacy = workflow_id_token(family, Some(LEGACY_ID_NAMESPACE_PREFIX))?;
    if include_legacy && !tokens.iter().any(|token| token == &legacy) {
        tokens.push(legacy);
    }
    Ok(tokens)
}

pub fn workflow_id_namespace_prefix_candidates(
    namespace_prefix: Option<&str>,
    include_legacy: bool,
    include_task_change_origins: bool,
) -> Result<Vec<String>, String> {
    let mut candidates = Vec::<String>::new();
    let mut base_prefixes = Vec::<String>::new();
    append_namespace_prefix_candidate(namespace_prefix, &mut candidates, &mut base_prefixes)?;
    if include_legacy {
        append_namespace_prefix_candidate(Some(""), &mut candidates, &mut base_prefixes)?;
        append_namespace_prefix_candidate(
            Some(LEGACY_ID_NAMESPACE_PREFIX),
            &mut candidates,
            &mut base_prefixes,
        )?;
    }
    if include_task_change_origins {
        let mut origin_candidates = Vec::new();
        for base_prefix in &base_prefixes {
            for origin_prefix in WORKFLOW_TASK_CHANGE_ORIGIN_NAMESPACE_PREFIXES {
                let derived = workflow_origin_namespace_prefix(origin_prefix, Some(base_prefix))?;
                if !origin_candidates.iter().any(|value| value == &derived) {
                    origin_candidates.push(derived);
                }
            }
        }
        candidates = origin_candidates
            .into_iter()
            .chain(candidates)
            .collect::<Vec<_>>();
    }
    Ok(candidates)
}

pub fn workflow_id_namespace_prefix_for_value(
    value: Option<&str>,
    family: &str,
    namespace_prefix: Option<&str>,
    include_legacy: bool,
    include_task_change_origins: bool,
) -> Result<Option<String>, String> {
    let text = value.unwrap_or("").trim().to_uppercase();
    if !text.contains('-') {
        return Ok(None);
    }
    let token = text.split_once('-').map(|(left, _)| left).unwrap_or("");
    for prefix in workflow_id_namespace_prefix_candidates(
        namespace_prefix,
        include_legacy,
        include_task_change_origins,
    )? {
        if token == workflow_id_token(family, Some(&prefix))? {
            return Ok(Some(prefix));
        }
    }
    Ok(None)
}

pub fn workflow_id_matches_any_namespace_prefix(
    value: Option<&str>,
    family: &str,
    namespace_prefix: Option<&str>,
    include_legacy: bool,
    include_task_change_origins: bool,
) -> Result<bool, String> {
    Ok(workflow_id_namespace_prefix_for_value(
        value,
        family,
        namespace_prefix,
        include_legacy,
        include_task_change_origins,
    )?
    .is_some())
}

pub fn generate_namespaced_sequence_id(
    family: &str,
    number: i64,
    namespace_prefix: Option<&str>,
    width: usize,
) -> Result<String, String> {
    let token = workflow_id_token(family, namespace_prefix)?;
    let resolved_width = if width == 0 { 4 } else { width };
    Ok(format!("{token}-{number:0resolved_width$}"))
}

pub fn derive_patchset_id(
    change_id: &str,
    patchset_number: i64,
    namespace_prefix: Option<&str>,
) -> Result<String, String> {
    let text = change_id.trim().to_uppercase();
    if !text.contains('-') {
        return Err(format!("Unsupported change id: {change_id:?}"));
    }
    let resolved_prefix =
        workflow_id_namespace_prefix_for_value(Some(&text), "C", namespace_prefix, true, true)?
            .ok_or_else(|| format!("Unsupported change id: {change_id:?}"))?;
    let patch_token = workflow_id_token("P", Some(&resolved_prefix))?;
    Ok(format!(
        "{patch_token}-{}-{}",
        text.split_once('-').map(|(_, right)| right).unwrap_or(""),
        patchset_number
    ))
}

pub fn generate_workflow_id(prefix: &str) -> Result<String, String> {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|exc| format!("System clock is before UNIX epoch: {exc}"))?
        .as_millis();
    let mut randomness = [0u8; 10];
    getrandom(&mut randomness)
        .map_err(|exc| format!("Failed to generate workflow id randomness: {exc}"))?;
    let randomness_value = randomness
        .iter()
        .fold(0u128, |acc, byte| (acc << 8) | (*byte as u128));
    Ok(format!(
        "{}-{}{}",
        prefix,
        encode_crockford_base32(timestamp_ms, 10)?,
        encode_crockford_base32(randomness_value, 16)?
    ))
}

pub fn workflow_success_envelope(
    kind: &str,
    value: Option<&str>,
) -> Result<WorkflowResultEnvelope, String> {
    let resolved_kind = require_non_empty(kind, "Workflow kind is required.")?;
    Ok(WorkflowResultEnvelope {
        ok: true,
        kind: resolved_kind,
        value: normalize_optional_text(value),
        error: None,
    })
}

pub fn workflow_error_envelope(
    kind: &str,
    code: &str,
    message: &str,
    detail: Option<&str>,
) -> Result<WorkflowResultEnvelope, String> {
    let resolved_kind = require_non_empty(kind, "Workflow kind is required.")?;
    let resolved_code = require_non_empty(code, "Workflow error code is required.")?;
    let resolved_message = require_non_empty(message, "Workflow error message is required.")?;
    Ok(WorkflowResultEnvelope {
        ok: false,
        kind: resolved_kind,
        value: None,
        error: Some(WorkflowErrorEnvelope {
            code: resolved_code,
            message: resolved_message,
            detail: normalize_optional_text(detail),
        }),
    })
}

fn normalize_workflow_id_family(value: &str) -> Result<String, String> {
    let resolved_family = value.trim().to_uppercase();
    if WORKFLOW_ID_FAMILIES
        .iter()
        .any(|candidate| *candidate == resolved_family)
    {
        Ok(resolved_family)
    } else {
        Err(format!("Unsupported workflow id family: {:?}", value))
    }
}

fn append_namespace_prefix_candidate(
    value: Option<&str>,
    candidates: &mut Vec<String>,
    base_prefixes: &mut Vec<String>,
) -> Result<(), String> {
    if value.is_none() {
        return Ok(());
    }
    let normalized = normalize_id_namespace_prefix(value, Some(DEFAULT_ID_NAMESPACE_PREFIX))?;
    if !base_prefixes
        .iter()
        .any(|candidate| candidate == &normalized)
    {
        base_prefixes.push(normalized.clone());
    }
    if !candidates.iter().any(|candidate| candidate == &normalized) {
        candidates.push(normalized);
    }
    Ok(())
}

fn encode_crockford_base32(value: u128, length: usize) -> Result<String, String> {
    let mut remaining = value;
    let mut chars = vec!['0'; length];
    for index in (0..length).rev() {
        chars[index] = CROCKFORD_BASE32[(remaining & 0b11111) as usize] as char;
        remaining >>= 5;
    }
    if remaining != 0 {
        return Err("Value does not fit requested Crockford base32 length".to_string());
    }
    Ok(chars.into_iter().collect())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    let normalized = value.unwrap_or("").trim().to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn require_non_empty(value: &str, message: &str) -> Result<String, String> {
    normalize_optional_text(Some(value)).ok_or_else(|| message.to_string())
}

#[cfg(test)]
mod tests;
