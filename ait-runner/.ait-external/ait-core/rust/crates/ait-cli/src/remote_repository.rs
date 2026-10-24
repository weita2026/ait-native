use crate::runtime::{RemoteRow, RepoRuntime};
use ait_core::json_support::JsonValue;
use ait_core::plan_http_client::PlanHttpClientConfig;
use ait_core::task_workflow_http_adapter::{
    HttpTaskRemote, TaskWorkflowRepositoryEnsurer, TaskWorkflowRepositoryReader,
};
use std::collections::BTreeSet;
use std::fs;

pub(crate) fn ensure_or_read_remote_repository_authority_for_url(
    repo: &RepoRuntime,
    remote_url: &str,
    repo_name: &str,
) -> Result<JsonValue, String> {
    if repo.repository_index().is_some() {
        read_remote_repository_authority_for_url(repo, remote_url, repo_name)
    } else {
        ensure_remote_repository_authority_for_url(repo, remote_url, repo_name)
    }
}

pub(crate) fn ensure_remote_repository_authority_for_url(
    repo: &RepoRuntime,
    remote_url: &str,
    repo_name: &str,
) -> Result<JsonValue, String> {
    let repo_name = normalize_required_text(repo_name, "repo_name")?;
    let remote_row = RemoteRow {
        name: "new remote".to_string(),
        url: remote_url.to_string(),
        repo_name: Some(repo_name.clone()),
    };
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    ensure_remote_repository_authority(repo, &mut task_remote, &repo_name)
}

pub(crate) fn ensure_remote_repository_authority<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRepositoryEnsurer + ?Sized,
{
    let repo_name = normalize_required_text(repo_name, "repo_name")?;
    let expected_namespace = repo.id_namespace_prefix();
    let policy = local_policy_payload(repo)?;
    let remote_repository = task_remote
        .ensure_repository(
            &repo_name,
            "main",
            policy.as_ref(),
            Some(&expected_namespace),
        )
        .map_err(|err| format!("Remote Repository registration for {repo_name} failed: {err}"))?;
    let repository_index = remote_repository_index(&remote_repository)?;
    verify_remote_repository_authority(
        &remote_repository,
        repository_index,
        &expected_namespace,
        &repo_name,
    )?;
    Ok(remote_repository)
}

pub(crate) fn read_remote_repository_authority_for_url(
    repo: &RepoRuntime,
    remote_url: &str,
    repo_name: &str,
) -> Result<JsonValue, String> {
    let repo_name = normalize_required_text(repo_name, "repo_name")?;
    let remote_row = RemoteRow {
        name: "new remote".to_string(),
        url: remote_url.to_string(),
        repo_name: Some(repo_name.clone()),
    };
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    read_remote_repository_authority(repo, &mut task_remote, &repo_name)
}

pub(crate) fn read_remote_repository_authority<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRepositoryReader + ?Sized,
{
    let repo_name = normalize_required_text(repo_name, "repo_name")?;
    let expected_repository_index = repo.require_repository_index()?;
    let remote_repository = task_remote.get_repository(&repo_name).map_err(|err| {
        format!(
            "Remote Repository authority {expected_repository_index} ({repo_name}) could not be read: {err}"
        )
    })?;
    let expected_namespace = repo.id_namespace_prefix();
    verify_remote_repository_authority(
        &remote_repository,
        expected_repository_index.get(),
        &expected_namespace,
        &repo_name,
    )?;
    Ok(remote_repository)
}

pub(crate) fn remote_repository_index(remote_repository: &JsonValue) -> Result<u32, String> {
    remote_repository
        .get("repository")
        .unwrap_or(remote_repository)
        .get("repository_index")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            "Remote Repository authority response is missing numeric repository_index.".to_string()
        })
}

fn http_task_remote(repo: &RepoRuntime, remote_row: &RemoteRow) -> Result<HttpTaskRemote, String> {
    HttpTaskRemote::new(http_config(repo, remote_row)).map_err(|err| err.to_string())
}

fn http_config(repo: &RepoRuntime, remote_row: &RemoteRow) -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: remote_row.url.clone(),
        repository_index: repo.repository_index(),
        headers: repo.auth_headers(),
        ..PlanHttpClientConfig::default()
    }
}

fn verify_remote_repository_authority(
    remote_repository: &JsonValue,
    expected_repository_index: u32,
    expected_namespace: &str,
    repo_name: &str,
) -> Result<(), String> {
    let repository = remote_repository
        .get("repository")
        .unwrap_or(remote_repository);
    let remote_repository_index = remote_repository_index(remote_repository)?;
    if remote_repository_index != expected_repository_index {
        return Err(format!(
            "Remote Repository authority index mismatch: configured={expected_repository_index} remote={remote_repository_index}"
        ));
    }
    let remote_namespace = repository
        .get("namespace")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            format!(
                "Remote Repository authority {expected_repository_index} ({repo_name}) is missing string namespace."
            )
        })?;
    if remote_namespace != expected_namespace {
        return Err(format!(
            "Remote Repository authority namespace mismatch: configured={expected_namespace:?} remote={remote_namespace:?}"
        ));
    }
    if repository
        .get("tombstoned")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return Err(format!(
            "Remote Repository authority {expected_repository_index} ({repo_name}) is tombstoned."
        ));
    }
    Ok(())
}

pub(crate) fn local_policy_requires_tests(repo: &RepoRuntime) -> Result<bool, String> {
    Ok(local_policy_payload(repo)?
        .and_then(|policy| {
            policy
                .get("defaults")
                .and_then(JsonValue::as_object)
                .and_then(|defaults| defaults.get("require_tests"))
                .and_then(JsonValue::as_bool)
        })
        .unwrap_or(true))
}

pub(crate) fn local_policy_payload(repo: &RepoRuntime) -> Result<Option<JsonValue>, String> {
    let path = repo.root.join(".ait").join("policy.yaml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "Failed to read local policy {}: {err}",
                path.display()
            ))
        }
    };
    validate_local_registration_policy_yaml(&text)?;
    ait_core::policy::parse_policy_yaml(&text, "prototype")
        .map(Some)
        .map_err(|err| format!("Local policy cannot be used for Repository registration: {err}"))
}

fn validate_local_registration_policy_yaml(text: &str) -> Result<(), String> {
    const DEFAULT_FIELDS: [&str; 7] = [
        "require_attestation",
        "require_tests",
        "require_lint",
        "require_security_scan",
        "require_license_scan",
        "require_ai_provenance",
        "require_code_review_summary",
    ];
    const DOCS_OVERRIDE_FIELDS: [&str; 4] = [
        "require_tests",
        "require_lint",
        "require_security_scan",
        "require_license_scan",
    ];

    let mut root_fields = BTreeSet::new();
    let mut default_fields = BTreeSet::new();
    let mut override_set_fields = BTreeSet::new();
    let mut section = "";
    let mut override_section = "";
    let mut override_count = 0_u8;
    let mut saw_docs_content_class = false;

    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line
            .split_once('#')
            .map(|(left, _)| left)
            .unwrap_or(raw_line)
            .trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line.contains('\t') {
            return Err(format!(
                "Local policy line {line_number} contains a tab; exact registration policy requires spaces."
            ));
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        let stripped = line.trim();
        match indent {
            0 => {
                override_section = "";
                if matches!(stripped, "defaults:" | "class_overrides:") {
                    section = stripped.trim_end_matches(':');
                    if !root_fields.insert(section.to_string()) {
                        return Err(format!(
                            "Local policy contains duplicate root field {section}."
                        ));
                    }
                    continue;
                }
                section = "";
                let (key, value) = policy_key_value(stripped, line_number)?;
                if !matches!(key, "version" | "policy_id") {
                    return Err(format!(
                        "Local policy contains unknown root field {key}."
                    ));
                }
                if !root_fields.insert(key.to_string()) {
                    return Err(format!("Local policy contains duplicate root field {key}."));
                }
                match key {
                    "version" if value != "1" => {
                        return Err(
                            "Local Repository registration policy version must be exact integer 1."
                                .to_string(),
                        )
                    }
                    "policy_id" if value != "prototype" => {
                        return Err(
                            "Local Repository registration policy_id must be exact prototype."
                                .to_string(),
                        )
                    }
                    _ => {}
                }
            }
            2 if section == "defaults" => {
                let (key, value) = policy_key_value(stripped, line_number)?;
                if !DEFAULT_FIELDS.contains(&key) {
                    return Err(format!(
                        "Local policy defaults contains unknown field {key}."
                    ));
                }
                if !matches!(value, "true" | "false") {
                    return Err(format!(
                        "Local policy defaults.{key} must be exact boolean true or false."
                    ));
                }
                if !default_fields.insert(key.to_string()) {
                    return Err(format!(
                        "Local policy contains duplicate defaults field {key}."
                    ));
                }
            }
            2 if section == "class_overrides" => {
                if stripped != "- when:" {
                    return Err(format!(
                        "Local policy line {line_number} must begin the exact docs-only override with `- when:`."
                    ));
                }
                override_count = override_count.saturating_add(1);
                if override_count != 1 {
                    return Err(
                        "Local Repository registration policy permits exactly one docs-only override."
                            .to_string(),
                    );
                }
                override_section = "when";
            }
            4 if section == "class_overrides"
                && override_count == 1
                && stripped == "set:" =>
            {
                override_section = "set";
            }
            6 if section == "class_overrides"
                && override_count == 1
                && override_section == "when" =>
            {
                let (key, value) = policy_key_value(stripped, line_number)?;
                if key != "content_class" || value != "docs_only" || saw_docs_content_class {
                    return Err(
                        "Local Repository registration policy requires the exact single docs_only content-class predicate."
                            .to_string(),
                    );
                }
                saw_docs_content_class = true;
            }
            6 if section == "class_overrides"
                && override_count == 1
                && override_section == "set" =>
            {
                let (key, value) = policy_key_value(stripped, line_number)?;
                if !DOCS_OVERRIDE_FIELDS.contains(&key) || value != "false" {
                    return Err(format!(
                        "Local Repository registration docs-only override field {key} is not an exact supported false assignment."
                    ));
                }
                if !override_set_fields.insert(key.to_string()) {
                    return Err(format!(
                        "Local policy contains duplicate docs-only override field {key}."
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "Local policy line {line_number} is outside the exact prototype registration structure."
                ))
            }
        }
    }
    if !root_fields.contains("policy_id") || !root_fields.contains("defaults") {
        return Err(
            "Local Repository registration policy requires policy_id and defaults.".to_string(),
        );
    }
    if root_fields.contains("class_overrides")
        && (override_count != 1
            || !saw_docs_content_class
            || override_set_fields.len() != DOCS_OVERRIDE_FIELDS.len())
    {
        return Err(
            "Local Repository registration class_overrides must be the complete exact docs-only override."
                .to_string(),
        );
    }
    Ok(())
}

fn policy_key_value(line: &str, line_number: usize) -> Result<(&str, &str), String> {
    let (key, value) = line.split_once(':').ok_or_else(|| {
        format!("Local policy line {line_number} must contain one key/value delimiter.")
    })?;
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || value.is_empty() || value.contains(':') {
        return Err(format!(
            "Local policy line {line_number} is not an exact scalar key/value pair."
        ));
    }
    Ok((key, value))
}

fn normalize_required_text(value: &str, field: &str) -> Result<String, String> {
    normalize_optional_text(Some(value)).ok_or_else(|| format!("{field} is required."))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

#[cfg(test)]
mod tests;
