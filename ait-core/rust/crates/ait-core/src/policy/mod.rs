use crate::json_support::{json, JsonMap, JsonValue};
use regex::Regex;

use crate::text_normalization::normalize_optional_text;

pub const POLICY_REQUIREMENT_FLAGS: &[&str] = &[
    "require_attestation",
    "require_tests",
    "require_lint",
    "require_security_scan",
    "require_license_scan",
    "require_ai_provenance",
    "require_code_review_summary",
];
pub const POLICY_CONTENT_CLASSES: &[&str] = &["docs_only", "code_change"];
pub const POLICY_AUTHOR_CLASSES: &[&str] = &["human_only", "ai_related"];
pub const CODE_REVIEW_SUMMARY_TEMPLATE: &str = "Reviewed files: <paths reviewed>; Findings: <blocking/non-blocking findings>; Risks: <residual risks>; Tests: <checks run>; Recommendation: <land/defer/request changes>";
pub const CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE: &str = "1. Reviewed files\n<paths reviewed>\n2. Findings\n<blocking/non-blocking findings>\n3. Risks\n<residual risks>\n4. Tests\n<checks run>\n5. Recommendation\n<land/defer/request changes>";
pub const CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND: &str =
    "ait review code template --style numbered";

const AUTHOR_MODES: &[&str] = &[
    "human_only",
    "human_with_ai_assist",
    "ai_with_human_review",
    "ai_only_experimental",
];
const AI_RELATED_AUTHOR_MODES: &[&str] = &[
    "human_with_ai_assist",
    "ai_with_human_review",
    "ai_only_experimental",
];
const CODE_REVIEW_SUMMARY_SECTIONS: &[(&str, &[&str])] = &[
    (
        "Reviewed files",
        &[
            "reviewed files",
            "files reviewed",
            "reviewed file",
            "files",
            "paths reviewed",
        ],
    ),
    ("Findings", &["findings", "issues", "observations"]),
    (
        "Risks",
        &["risks", "risk", "residual risks", "regression risks"],
    ),
    ("Tests", &["tests", "verification", "validation", "checks"]),
    (
        "Recommendation",
        &[
            "recommendation",
            "promotion recommendation",
            "land recommendation",
            "verdict",
            "decision",
        ],
    ),
];

pub fn policy_profile_names() -> Vec<String> {
    vec![
        "prototype".to_string(),
        "release".to_string(),
        "team".to_string(),
    ]
}

pub fn author_mode_values() -> Vec<String> {
    AUTHOR_MODES
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

pub fn policy_content_class_values() -> Vec<String> {
    POLICY_CONTENT_CLASSES
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

pub fn policy_author_class_values() -> Vec<String> {
    POLICY_AUTHOR_CLASSES
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

pub fn normalize_author_mode(value: &str) -> Result<String, String> {
    let text = value.trim();
    if AUTHOR_MODES.contains(&text) {
        Ok(text.to_string())
    } else {
        Err(format!(
            "Unknown author_mode: {value}. Expected one of: {}",
            AUTHOR_MODES.join(", ")
        ))
    }
}

pub fn missing_code_review_summary_sections(value: Option<&str>) -> Vec<String> {
    let Some(text) = normalize_optional_text(value) else {
        return CODE_REVIEW_SUMMARY_SECTIONS
            .iter()
            .map(|(section, _)| (*section).to_string())
            .collect();
    };
    let labels = code_review_label_regex();
    let matches: Vec<_> = labels.captures_iter(&text).collect();
    let mut present = Vec::<String>::new();
    for (index, captures) in matches.iter().enumerate() {
        let Some(label_match) = captures.name("label") else {
            continue;
        };
        let canonical = canonical_code_review_section(label_match.as_str());
        let Some(canonical) = canonical else {
            continue;
        };
        let section_start = captures
            .get(0)
            .map(|m| m.end())
            .unwrap_or(label_match.end());
        let next_start = matches
            .get(index + 1)
            .and_then(|next| next.get(0))
            .map(|m| m.start())
            .unwrap_or(text.len());
        let body = text[section_start..next_start].trim();
        if code_review_summary_section_has_content(body) && !present.iter().any(|v| v == canonical)
        {
            present.push(canonical.to_string());
        }
    }
    CODE_REVIEW_SUMMARY_SECTIONS
        .iter()
        .filter_map(|(section, _)| {
            if present.iter().any(|value| value == section) {
                None
            } else {
                Some((*section).to_string())
            }
        })
        .collect()
}

pub fn is_structured_code_review_summary(value: Option<&str>) -> bool {
    missing_code_review_summary_sections(value).is_empty()
}

pub fn render_code_review_summary_template(style: Option<&str>) -> Result<&'static str, String> {
    match style.unwrap_or("inline").trim().to_lowercase().as_str() {
        "inline" => Ok(CODE_REVIEW_SUMMARY_TEMPLATE),
        "numbered" => Ok(CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE),
        _ => Err(
            "Unknown code review summary template style. Expected one of: inline, numbered."
                .to_string(),
        ),
    }
}

pub fn code_review_summary_requirement_text(value: Option<&str>) -> String {
    let required = CODE_REVIEW_SUMMARY_SECTIONS
        .iter()
        .map(|(section, _)| *section)
        .collect::<Vec<_>>()
        .join(", ");
    let missing = value
        .map(|text| missing_code_review_summary_sections(Some(text)))
        .unwrap_or_default();
    if missing.is_empty() {
        format!(
            "Code review summary requires sections with non-placeholder content: {required}. Run `{CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND}` for a safe scaffold."
        )
    } else {
        format!(
            "Code review summary is missing sections with non-placeholder content: {}. Required sections: {required}. Run `{CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND}` for a safe scaffold.",
            missing.join(", ")
        )
    }
}

pub fn derive_policy_content_class(changed_paths: Option<&JsonValue>) -> String {
    let paths = match changed_paths {
        Some(JsonValue::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    if !paths.is_empty()
        && paths
            .iter()
            .all(|path| path.to_lowercase().ends_with(".md"))
    {
        "docs_only".to_string()
    } else {
        "code_change".to_string()
    }
}

pub fn derive_policy_author_class(author_mode: Option<&str>) -> Option<String> {
    let normalized = normalize_optional_text(author_mode)?;
    let value = normalize_author_mode(&normalized).ok()?;
    if AI_RELATED_AUTHOR_MODES
        .iter()
        .any(|candidate| *candidate == value)
    {
        Some("ai_related".to_string())
    } else {
        Some("human_only".to_string())
    }
}

pub fn build_minimum_provenance(
    author_mode: &str,
    model_name: Option<&str>,
) -> Result<(JsonValue, JsonValue), String> {
    let author_mode_value = normalize_author_mode(author_mode)?;
    let model_name_value = normalize_optional_text(model_name);
    let required_fields: Vec<&str> = if AI_RELATED_AUTHOR_MODES
        .iter()
        .any(|candidate| *candidate == author_mode_value)
    {
        vec!["model_name"]
    } else {
        Vec::new()
    };
    let mut missing_fields = Vec::new();
    for field in &required_fields {
        let present = match *field {
            "model_name" => model_name_value.is_some(),
            _ => false,
        };
        if !present {
            missing_fields.push((*field).to_string());
        }
    }
    let evidence_readiness = if required_fields.is_empty() {
        "not_required"
    } else if missing_fields.is_empty() {
        "complete"
    } else {
        "partial"
    };
    let policy_readable = missing_fields.is_empty();
    let provenance_summary = json!({
        "model_name": model_name_value,
        "evidence_readiness": evidence_readiness,
        "missing_fields": missing_fields,
        "policy_readable": policy_readable,
    });
    let detail = json!({
        "minimum_evidence": {
            "author_mode": author_mode_value,
            "model_name": model_name_value,
            "required_fields": required_fields,
            "missing_fields": provenance_summary["missing_fields"],
            "policy_readable": policy_readable,
        }
    });
    Ok((provenance_summary, detail))
}

pub fn policy_profile(name: &str) -> Result<JsonValue, String> {
    let profile_name = name.trim().to_lowercase();
    let profile_name = if profile_name.is_empty() {
        "prototype"
    } else {
        profile_name.as_str()
    };
    match profile_name {
        "prototype" => Ok(profile_value(
            "prototype",
            false,
            false,
            false,
            false,
            false,
        )),
        "team" => Ok(profile_value("team", true, false, false, false, false)),
        "release" => Ok(profile_value("release", true, true, true, true, false)),
        _ => Err(format!("Unknown policy profile: {name}")),
    }
}

pub fn normalize_policy(
    policy: Option<&JsonValue>,
    fallback_profile: &str,
) -> Result<JsonValue, String> {
    let base = policy_profile(fallback_profile)?;
    let empty = JsonMap::new();
    let payload = policy.and_then(JsonValue::as_object).unwrap_or(&empty);
    let base_id = base
        .get("policy_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("prototype")
        .to_string();
    let policy_id = payload
        .get("policy_id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(base_id.as_str())
        .to_string();
    let mut normalized = if policy_profile_names().iter().any(|name| name == &policy_id) {
        policy_profile(&policy_id)?
    } else {
        let mut custom = base;
        custom
            .as_object_mut()
            .expect("policy profile must be an object")
            .insert("policy_id".to_string(), JsonValue::String(policy_id));
        custom
    };
    let normalized_obj = normalized
        .as_object_mut()
        .ok_or_else(|| "normalized policy must be an object".to_string())?;
    let mut defaults = normalized_obj
        .get("defaults")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let raw_defaults = payload.get("defaults").and_then(JsonValue::as_object);
    for key in POLICY_REQUIREMENT_FLAGS {
        let default = defaults
            .get(*key)
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let value = raw_defaults.and_then(|values| values.get(*key));
        defaults.insert(
            (*key).to_string(),
            JsonValue::Bool(coerce_bool(value, default)),
        );
    }
    normalized_obj.insert("defaults".to_string(), JsonValue::Object(defaults.clone()));
    let version = payload
        .get("version")
        .and_then(JsonValue::as_i64)
        .unwrap_or_else(|| {
            normalized_obj
                .get("version")
                .and_then(JsonValue::as_i64)
                .unwrap_or(1)
        });
    normalized_obj.insert("version".to_string(), JsonValue::from(version));
    let raw_overrides = payload
        .get("class_overrides")
        .or_else(|| normalized_obj.get("class_overrides"));
    normalized_obj.insert(
        "class_overrides".to_string(),
        JsonValue::Array(normalize_policy_class_overrides(raw_overrides, &defaults)),
    );
    Ok(normalized)
}

pub fn resolve_effective_policy(
    policy: Option<&JsonValue>,
    content_class: Option<&str>,
    author_class: Option<&str>,
    fallback_profile: &str,
) -> Result<JsonValue, String> {
    let normalized = normalize_policy(policy, fallback_profile)?;
    let mut effective_requirements = normalized
        .get("defaults")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let mut matched_overrides = Vec::new();
    for (index, override_value) in normalized
        .get("class_overrides")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let when = override_value
            .get("when")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        if when
            .get("content_class")
            .and_then(JsonValue::as_str)
            .is_some_and(|value| Some(value) != content_class)
        {
            continue;
        }
        if when
            .get("author_class")
            .and_then(JsonValue::as_str)
            .is_some_and(|value| Some(value) != author_class)
        {
            continue;
        }
        let set_values = override_value
            .get("set")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in &set_values {
            effective_requirements.insert(key.clone(), value.clone());
        }
        matched_overrides.push(json!({
            "index": index + 1,
            "when": when,
            "set": set_values,
        }));
    }
    Ok(json!({
        "policy": normalized,
        "content_class": content_class,
        "author_class": author_class,
        "effective_requirements": effective_requirements,
        "matched_overrides": matched_overrides,
    }))
}

pub fn policy_to_yaml(
    policy: Option<&JsonValue>,
    fallback_profile: &str,
) -> Result<String, String> {
    let normalized = normalize_policy(policy, fallback_profile)?;
    let obj = normalized
        .as_object()
        .ok_or_else(|| "normalized policy must be an object".to_string())?;
    let mut lines = vec![
        format!(
            "version: {}",
            obj.get("version").and_then(JsonValue::as_i64).unwrap_or(1)
        ),
        format!(
            "policy_id: {}",
            obj.get("policy_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("prototype")
        ),
        "defaults:".to_string(),
    ];
    let defaults = obj
        .get("defaults")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    for key in POLICY_REQUIREMENT_FLAGS {
        lines.push(format!(
            "  {key}: {}",
            if defaults
                .get(*key)
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            {
                "true"
            } else {
                "false"
            }
        ));
    }
    if let Some(overrides) = obj.get("class_overrides").and_then(JsonValue::as_array) {
        if !overrides.is_empty() {
            lines.push("class_overrides:".to_string());
            for override_value in overrides {
                lines.push("  - when:".to_string());
                if let Some(when) = override_value.get("when").and_then(JsonValue::as_object) {
                    for (key, value) in when {
                        lines.push(format!(
                            "      {key}: {}",
                            value.as_str().unwrap_or_default()
                        ));
                    }
                }
                lines.push("    set:".to_string());
                if let Some(set_values) = override_value.get("set").and_then(JsonValue::as_object) {
                    for key in POLICY_REQUIREMENT_FLAGS {
                        if let Some(value) = set_values.get(*key) {
                            lines.push(format!(
                                "      {key}: {}",
                                if value.as_bool().unwrap_or(false) {
                                    "true"
                                } else {
                                    "false"
                                }
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(format!("{}\n", lines.join("\n")))
}

pub fn parse_policy_yaml(text: &str, fallback_profile: &str) -> Result<JsonValue, String> {
    let mut payload = JsonMap::new();
    let mut defaults = JsonMap::new();
    let mut class_overrides = Vec::<JsonValue>::new();
    let mut in_defaults = false;
    let mut in_class_overrides = false;
    let mut current_override: Option<JsonMap<String, JsonValue>> = None;
    let mut current_override_section: Option<String> = None;
    for raw_line in text.lines() {
        let line = raw_line
            .split_once('#')
            .map(|(left, _)| left)
            .unwrap_or(raw_line)
            .trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let stripped = line.trim();
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent == 0 {
            push_current_override(&mut class_overrides, &mut current_override);
            if stripped == "defaults:" {
                in_defaults = true;
                in_class_overrides = false;
                current_override_section = None;
                continue;
            }
            if stripped == "class_overrides:" {
                in_defaults = false;
                in_class_overrides = true;
                current_override_section = None;
                continue;
            }
            in_defaults = false;
            in_class_overrides = false;
            current_override_section = None;
            let (key, value) = split_key_value(stripped)?;
            payload.insert(key.to_string(), parse_policy_scalar(value));
            continue;
        }
        if in_defaults {
            let (key, value) = split_key_value(stripped)?;
            defaults.insert(key.to_string(), parse_policy_scalar(value));
            continue;
        }
        if in_class_overrides {
            if indent == 2 && stripped == "- when:" {
                push_current_override(&mut class_overrides, &mut current_override);
                let mut override_map = JsonMap::new();
                override_map.insert("when".to_string(), JsonValue::Object(JsonMap::new()));
                override_map.insert("set".to_string(), JsonValue::Object(JsonMap::new()));
                current_override = Some(override_map);
                current_override_section = Some("when".to_string());
                continue;
            }
            let Some(override_map) = current_override.as_mut() else {
                continue;
            };
            if indent == 4 && matches!(stripped, "when:" | "set:") {
                current_override_section = Some(stripped.trim_end_matches(':').to_string());
                continue;
            }
            if indent >= 4
                && current_override_section
                    .as_deref()
                    .is_some_and(|section| matches!(section, "when" | "set"))
            {
                let (key, value) = split_key_value(stripped)?;
                let section = current_override_section.clone().unwrap_or_default();
                if let Some(section_obj) = override_map
                    .get_mut(&section)
                    .and_then(JsonValue::as_object_mut)
                {
                    section_obj.insert(key.to_string(), parse_policy_scalar(value));
                }
            }
        }
    }
    push_current_override(&mut class_overrides, &mut current_override);
    if !defaults.is_empty() {
        payload.insert("defaults".to_string(), JsonValue::Object(defaults));
    }
    if !class_overrides.is_empty() {
        payload.insert(
            "class_overrides".to_string(),
            JsonValue::Array(class_overrides),
        );
    }
    normalize_policy(Some(&JsonValue::Object(payload)), fallback_profile)
}

fn profile_value(
    policy_id: &str,
    require_lint: bool,
    require_security_scan: bool,
    require_license_scan: bool,
    _release_marker: bool,
    require_code_review_summary: bool,
) -> JsonValue {
    json!({
        "version": 1,
        "policy_id": policy_id,
        "defaults": {
            "require_attestation": true,
            "require_tests": true,
            "require_lint": require_lint,
            "require_security_scan": require_security_scan,
            "require_license_scan": require_license_scan,
            "require_ai_provenance": false,
            "require_code_review_summary": require_code_review_summary,
        },
        "class_overrides": default_policy_class_overrides(),
    })
}

fn default_policy_class_overrides() -> JsonValue {
    json!([
        {
            "when": {"content_class": "docs_only"},
            "set": {
                "require_tests": false,
                "require_lint": false,
                "require_security_scan": false,
                "require_license_scan": false,
            },
        },
    ])
}

fn normalize_policy_class_overrides(
    raw_overrides: Option<&JsonValue>,
    defaults: &JsonMap<String, JsonValue>,
) -> Vec<JsonValue> {
    let mut normalized = Vec::new();
    let Some(overrides) = raw_overrides.and_then(JsonValue::as_array) else {
        return normalized;
    };
    for item in overrides {
        let Some(item_obj) = item.as_object() else {
            continue;
        };
        let Some(when_raw) = item_obj.get("when").and_then(JsonValue::as_object) else {
            continue;
        };
        let Some(set_raw) = item_obj.get("set").and_then(JsonValue::as_object) else {
            continue;
        };
        let mut when = JsonMap::new();
        if let Some(content_class) = when_raw
            .get("content_class")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_optional_text(Some(value)))
        {
            if POLICY_CONTENT_CLASSES
                .iter()
                .any(|candidate| *candidate == content_class)
            {
                when.insert(
                    "content_class".to_string(),
                    JsonValue::String(content_class),
                );
            }
        }
        if let Some(author_class) = when_raw
            .get("author_class")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_optional_text(Some(value)))
        {
            if POLICY_AUTHOR_CLASSES
                .iter()
                .any(|candidate| *candidate == author_class)
            {
                when.insert("author_class".to_string(), JsonValue::String(author_class));
            }
        }
        if when.is_empty() {
            continue;
        }
        let mut set_values = JsonMap::new();
        for key in POLICY_REQUIREMENT_FLAGS {
            if let Some(value) = set_raw.get(*key) {
                let default = defaults
                    .get(*key)
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                set_values.insert(
                    (*key).to_string(),
                    JsonValue::Bool(coerce_bool(Some(value), default)),
                );
            }
        }
        if set_values.is_empty() {
            continue;
        }
        normalized.push(json!({"when": when, "set": set_values}));
    }
    normalized
}

fn code_review_label_regex() -> Regex {
    let mut labels = CODE_REVIEW_SUMMARY_SECTIONS
        .iter()
        .flat_map(|(_, aliases)| aliases.iter().copied())
        .collect::<Vec<_>>();
    labels.sort_by_key(|value| std::cmp::Reverse(value.len()));
    let label_pattern = labels
        .iter()
        .map(|label| regex::escape(label))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(
        r"(?is)(?:^|[\n;])\s*(?:(?:[-*]|\d+\.)\s*)?(?:#{{1,6}}\s*)?(?:\*\*)?(?P<label>{label_pattern})(?:\*\*)?\s*(?:(?::|-|\u{{2013}}|\u{{2014}})\s*|\n+)"
    ))
    .expect("code review summary label regex must compile")
}

fn canonical_code_review_section(label: &str) -> Option<&'static str> {
    let normalized = label.trim().to_lowercase();
    CODE_REVIEW_SUMMARY_SECTIONS
        .iter()
        .find(|(_, aliases)| aliases.iter().any(|alias| *alias == normalized))
        .map(|(section, _)| *section)
}

fn code_review_summary_section_has_content(value: &str) -> bool {
    let text = value.trim();
    let placeholder = text
        .trim_matches(|ch: char| ch == '.' || ch.is_whitespace())
        .to_lowercase();
    if text.is_empty()
        || matches!(
            placeholder.as_str(),
            "" | "todo" | "tbd" | "replace me" | "replace_me"
        )
    {
        return false;
    }
    !(text.starts_with('<') && text.ends_with('>'))
}

fn coerce_bool(value: Option<&JsonValue>, default: bool) -> bool {
    match value {
        Some(JsonValue::Bool(value)) => *value,
        None | Some(JsonValue::Null) => default,
        Some(JsonValue::Number(number)) => {
            if let Some(value) = number.as_i64() {
                value != 0
            } else if let Some(value) = number.as_f64() {
                value != 0.0
            } else {
                default
            }
        }
        Some(JsonValue::String(value)) => match value.trim().to_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => true,
            "false" | "no" | "off" | "0" => false,
            _ => default,
        },
        _ => default,
    }
}

fn split_key_value(line: &str) -> Result<(&str, &str), String> {
    line.split_once(':')
        .map(|(key, value)| (key.trim(), value.trim()))
        .ok_or_else(|| format!("Invalid policy line: {line}"))
}

fn parse_policy_scalar(value: &str) -> JsonValue {
    if value.is_empty() {
        return JsonValue::String(String::new());
    }
    match value.to_lowercase().as_str() {
        "true" => JsonValue::Bool(true),
        "false" => JsonValue::Bool(false),
        _ if value.chars().all(|ch| ch.is_ascii_digit()) => value
            .parse::<i64>()
            .map(JsonValue::from)
            .unwrap_or_else(|_| JsonValue::String(value.to_string())),
        _ if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')) =>
        {
            JsonValue::String(value[1..value.len() - 1].to_string())
        }
        _ => JsonValue::String(value.to_string()),
    }
}

fn push_current_override(
    class_overrides: &mut Vec<JsonValue>,
    current_override: &mut Option<JsonMap<String, JsonValue>>,
) {
    if let Some(value) = current_override.take() {
        class_overrides.push(JsonValue::Object(value));
    }
}

#[cfg(test)]
mod tests;
