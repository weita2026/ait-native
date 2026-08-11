use super::*;

pub(super) fn policy_context_for_patchset(
    repo_policy: &JsonMap<String, JsonValue>,
    patchset: &JsonMap<String, JsonValue>,
    attestation: Option<&JsonMap<String, JsonValue>>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let changed_paths = patchset_changed_paths(patchset);
    let content_class = derive_policy_content_class(&changed_paths);
    let author_mode = attestation
        .and_then(|row| optional_text(row.get("author_mode")))
        .or_else(|| optional_text(patchset.get("author_mode")));
    let author_class = derive_policy_author_class(author_mode.as_deref());
    let policy = normalize_policy(repo_policy);
    let mut context = resolve_effective_policy(&policy, &content_class, author_class.as_deref());
    context.insert("changed_paths".to_string(), json!(changed_paths));
    let provenance_summary = attestation
        .and_then(|row| {
            optional_text(row.get("provenance_summary_json"))
                .and_then(|raw| serde_json::from_str::<JsonValue>(&raw).ok())
        })
        .unwrap_or_else(|| json!({}));
    context.insert("provenance_summary".to_string(), provenance_summary);
    Ok(context)
}

fn derive_policy_content_class(changed_paths: &[String]) -> String {
    if !changed_paths.is_empty() && changed_paths.iter().all(|path| docs_like_path(path)) {
        "docs_only".to_string()
    } else {
        "code_change".to_string()
    }
}

fn docs_like_path(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    lower.starts_with("docs/")
        || lower == "readme"
        || lower.starts_with("readme.")
        || [".md", ".markdown", ".rst", ".txt", ".adoc"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

fn derive_policy_author_class(author_mode: Option<&str>) -> Option<String> {
    match author_mode.unwrap_or("").trim() {
        "human_only" => Some("human_only".to_string()),
        "human_with_ai_assist" | "ai_with_human_review" | "ai_only_experimental" => {
            Some("ai_related".to_string())
        }
        _ => None,
    }
}

pub(super) fn normalize_policy(raw: &JsonMap<String, JsonValue>) -> JsonMap<String, JsonValue> {
    let mut defaults = prototype_defaults();
    if let Some(raw_defaults) = raw.get("defaults").and_then(JsonValue::as_object) {
        for &key in requirement_flags() {
            if let Some(value) = raw_defaults.get(key) {
                defaults.insert(key.to_string(), json!(truthy(value)));
            }
        }
    }
    let class_overrides = raw
        .get("class_overrides")
        .and_then(JsonValue::as_array)
        .filter(|items| !items.is_empty())
        .cloned()
        .unwrap_or_else(prototype_class_overrides);
    let mut policy = JsonMap::new();
    policy.insert(
        "policy_id".to_string(),
        json!(optional_text(raw.get("policy_id")).unwrap_or_else(|| "prototype".to_string())),
    );
    policy.insert(
        "version".to_string(),
        raw.get("version").cloned().unwrap_or_else(|| json!(1)),
    );
    policy.insert("defaults".to_string(), JsonValue::Object(defaults));
    policy.insert(
        "class_overrides".to_string(),
        JsonValue::Array(class_overrides),
    );
    policy
}

fn prototype_defaults() -> JsonMap<String, JsonValue> {
    let mut defaults = JsonMap::new();
    defaults.insert("require_attestation".to_string(), json!(true));
    defaults.insert("require_tests".to_string(), json!(true));
    defaults.insert("require_lint".to_string(), json!(false));
    defaults.insert("require_security_scan".to_string(), json!(false));
    defaults.insert("require_license_scan".to_string(), json!(false));
    defaults.insert("require_ai_provenance".to_string(), json!(false));
    defaults.insert("require_code_review_summary".to_string(), json!(false));
    defaults
}

fn prototype_class_overrides() -> Vec<JsonValue> {
    vec![json!({
        "when": {"content_class": "docs_only"},
        "set": {
            "require_tests": false,
            "require_lint": false,
            "require_security_scan": false,
            "require_license_scan": false,
        }
    })]
}

fn requirement_flags() -> &'static [&'static str] {
    &[
        "require_attestation",
        "require_tests",
        "require_lint",
        "require_security_scan",
        "require_license_scan",
        "require_ai_provenance",
        "require_code_review_summary",
    ]
}

fn resolve_effective_policy(
    policy: &JsonMap<String, JsonValue>,
    content_class: &str,
    author_class: Option<&str>,
) -> JsonMap<String, JsonValue> {
    let mut effective = policy
        .get("defaults")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_else(prototype_defaults);
    let mut matched = Vec::new();
    for (index, override_value) in policy
        .get("class_overrides")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(override_object) = override_value.as_object() else {
            continue;
        };
        let when = override_object.get("when").and_then(JsonValue::as_object);
        if !policy_override_matches(when, content_class, author_class) {
            continue;
        }
        let set = override_object
            .get("set")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        for &key in requirement_flags() {
            if let Some(value) = set.get(key) {
                effective.insert(key.to_string(), json!(truthy(value)));
            }
        }
        matched.push(json!({
            "index": index + 1,
            "when": when.cloned().unwrap_or_default(),
            "set": set,
        }));
    }
    let mut context = JsonMap::new();
    context.insert("policy".to_string(), JsonValue::Object(policy.clone()));
    context.insert("content_class".to_string(), json!(content_class));
    context.insert(
        "author_class".to_string(),
        author_class
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
    );
    context.insert(
        "effective_requirements".to_string(),
        JsonValue::Object(effective),
    );
    context.insert("matched_overrides".to_string(), JsonValue::Array(matched));
    context
}

fn policy_override_matches(
    when: Option<&JsonMap<String, JsonValue>>,
    content_class: &str,
    author_class: Option<&str>,
) -> bool {
    let Some(when) = when else {
        return true;
    };
    if let Some(expected) = optional_text(when.get("content_class")) {
        if expected != content_class {
            return false;
        }
    }
    if let Some(expected) = optional_text(when.get("author_class")) {
        if Some(expected.as_str()) != author_class {
            return false;
        }
    }
    true
}
