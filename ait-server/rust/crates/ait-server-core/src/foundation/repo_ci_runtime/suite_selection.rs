use super::*;

pub(super) fn selected_repo_suites(
    config: &RepoCiRuntimeConfig,
) -> Result<Vec<PatchsetSuiteManifest>, String> {
    let mut selected = Vec::new();
    if !config.suite_ids.is_empty() {
        let by_id = config
            .suites
            .iter()
            .filter_map(|suite| {
                let suite_id = suite.suite_id.trim();
                if suite_id.is_empty() {
                    None
                } else {
                    Some((suite_id.to_string(), suite))
                }
            })
            .collect::<BTreeMap<_, _>>();
        let mut missing = Vec::new();
        for suite_id in &config.suite_ids {
            let Some(suite) = by_id.get(suite_id) else {
                missing.push(suite_id.clone());
                continue;
            };
            validate_repo_ci_plane(suite)?;
            selected.push((*suite).clone());
        }
        if !missing.is_empty() {
            return Err(format!(
                "Unknown repo CI suite id(s): {}.",
                missing.join(", ")
            ));
        }
    } else if let Some(configured) = configured_suite_ids_for_plane(config) {
        let by_id = config
            .suites
            .iter()
            .filter_map(|suite| {
                let suite_id = suite.suite_id.trim();
                if suite_id.is_empty() {
                    None
                } else {
                    Some((suite_id.to_string(), suite))
                }
            })
            .collect::<BTreeMap<_, _>>();
        let mut missing = Vec::new();
        for suite_id in configured {
            let Some(suite) = by_id.get(&suite_id) else {
                missing.push(suite_id);
                continue;
            };
            if !suite.plane.trim().eq_ignore_ascii_case(&config.plane) {
                return Err(format!(
                    "Suite `{}` is configured under `{}` but declares plane `{}`.",
                    suite.suite_id.trim(),
                    config.plane,
                    suite.plane
                ));
            }
            selected.push((*suite).clone());
        }
        if !missing.is_empty() {
            return Err(format!(
                "Configured repo CI suite id(s) for plane `{}` are missing manifests: {}.",
                config.plane,
                missing.join(", ")
            ));
        }
    } else {
        selected = config
            .suites
            .iter()
            .filter(|suite| suite.plane.trim().eq_ignore_ascii_case(&config.plane))
            .cloned()
            .collect();
    }
    selected.sort_by(|left, right| left.suite_id.trim().cmp(right.suite_id.trim()));
    for suite in &selected {
        if suite.suite_id.trim().is_empty() {
            return Err("repo CI suite manifest requires `suite_id`.".to_string());
        }
    }
    Ok(selected)
}

pub(super) fn configured_suite_ids_for_plane(config: &RepoCiRuntimeConfig) -> Option<Vec<String>> {
    let key = match config.plane.as_str() {
        "nightly" => "nightly_suites",
        "release" => "release_suites",
        _ => return None,
    };
    let values = config.ci_config.get(key)?.as_array()?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        let Some(text) = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if seen.insert(text.to_string()) {
            out.push(text.to_string());
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(super) fn validate_repo_ci_plane(suite: &PatchsetSuiteManifest) -> Result<(), String> {
    let plane = suite.plane.trim();
    if REPO_CI_PLANES.iter().any(|allowed| plane == *allowed) {
        Ok(())
    } else {
        Err(format!(
            "Suite `{}` cannot run through repo CI because it belongs to plane `{}`.",
            suite.suite_id.trim(),
            suite.plane
        ))
    }
}

pub(super) fn release_gate_evidence(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> JsonValue {
    let mut required_dependency_keys = string_array_from_value(
        config
            .ci_config
            .get("rollout")
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("release_evidence"))
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("dependency_keys")),
    );
    let mut required_compliance_keys = string_array_from_value(
        config
            .ci_config
            .get("rollout")
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("release_evidence"))
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("compliance_keys")),
    );
    if let Some(suite_gate) = suite_value(config, suite)
        .and_then(|value| value.get("release_gate_evidence"))
        .and_then(JsonValue::as_object)
    {
        required_dependency_keys.extend(string_array_from_value(suite_gate.get("dependency_keys")));
        required_compliance_keys.extend(string_array_from_value(suite_gate.get("compliance_keys")));
    }
    required_dependency_keys.sort();
    required_dependency_keys.dedup();
    required_compliance_keys.sort();
    required_compliance_keys.dedup();
    json!({
        "dependency_keys": required_dependency_keys,
        "compliance_keys": required_compliance_keys,
        "attached_dependency_evidence": config.dependency_evidence,
        "attached_compliance_evidence": config.compliance_evidence,
        "missing_dependency_keys": missing_keys(&required_dependency_keys, &config.dependency_evidence),
        "missing_compliance_keys": missing_keys(&required_compliance_keys, &config.compliance_evidence),
    })
}

pub(super) fn missing_keys(required: &[String], attached: &[String]) -> Vec<String> {
    let attached = attached.iter().collect::<BTreeSet<_>>();
    required
        .iter()
        .filter(|key| !attached.contains(key))
        .cloned()
        .collect()
}

pub(super) fn suite_value<'a>(
    config: &'a RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> Option<&'a JsonValue> {
    config.suite_values.iter().find(|value| {
        value
            .get("suite_id")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            == Some(suite.suite_id.trim())
    })
}
