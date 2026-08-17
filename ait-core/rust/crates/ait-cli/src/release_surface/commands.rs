use super::*;

pub fn release_candidate_create(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    profile: &str,
) -> Result<JsonValue, String> {
    let profile = require_profile(profile)?;
    let line = release_local_line_row(repo, line_name)?;
    let snapshot_id = string_field(&line, "head_snapshot_id")
        .ok_or_else(|| format!("Line {line_name} does not have a head snapshot yet."))?;
    let bundle = release_source_bundle(repo, &snapshot_id, &profile)?;
    let package = package_metadata_from_bundle(&bundle)?;
    if version.trim() != package.version {
        return Err(format!(
            "Requested release version {version:?} does not match pyproject.toml version {:?}.",
            package.version
        ));
    }
    release_require_external_readiness(repo)?;
    let release_id = create_release_id(repo)?;
    let manifest_hash = required_string_field(&bundle.raw, "manifest_hash")?;
    let created_at = current_timestamp();
    let mut metadata = json!({
        "package": package.to_json(),
        "profile": profile.id,
        "profile_settings": profile.to_json(),
        "source_snapshot_created_at": bundle.raw.get("created_at").cloned().unwrap_or(JsonValue::Null),
        "native_distribution": native_distribution_candidate_contract(version.trim(), &release_id, &snapshot_id),
    });
    if let Some(external_closure) = release_external_closure_from_bundle(&bundle)? {
        metadata
            .as_object_mut()
            .ok_or_else(|| "release metadata must be a JSON object".to_string())?
            .insert("external_closure".to_string(), external_closure);
    }
    let release_store = workflow_release_store(repo)?;
    create_workflow_release_with_store(
        &release_store,
        &WorkflowReleaseRecord {
            release_id: release_id.clone(),
            repo_name: repo.repo_name(),
            version: version.trim().to_string(),
            line_name: line_name.trim().to_string(),
            snapshot_id: snapshot_id.clone(),
            manifest_hash: manifest_hash.clone(),
            profile: profile.id.to_string(),
            package_name: Some(package.name.clone()),
            package_version: Some(package.version.clone()),
            package_requires_python: package.requires_python.clone(),
            status: "candidate".to_string(),
            checks_json: "[]".to_string(),
            artifacts_json: "[]".to_string(),
            formula_json: "{}".to_string(),
            metadata_json: metadata.to_string(),
            created_at: created_at.clone(),
            updated_at: created_at,
        },
    )?;
    get_release_candidate(repo, &release_id)
}

pub fn release_show(
    repo: &RepoRuntime,
    release_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if remote_name.is_some() {
        let (remote_row, repo_name) = remote_context(repo, remote_name)?;
        let mut client = http_client(repo, &remote_row)?;
        return client
            .get_release(&repo_name, release_id)
            .map_err(plan_http_error_message);
    }
    get_release_candidate(repo, release_id)
}

pub fn release_publish(
    repo: &RepoRuntime,
    release_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let record = get_release_candidate(repo, release_id)?;
    assert_publish_ready(&record)?;
    let (remote_row, repo_name) = remote_context(repo, remote_name)?;
    if required_string_field(&record, "repo_name")? != repo_name {
        return Err(format!(
            "Local release {release_id} belongs to repository {}, not {repo_name}",
            required_string_field(&record, "repo_name")?
        ));
    }
    let mut client = http_client(repo, &remote_row)?;
    let snapshot_id = required_string_field(&record, "snapshot_id")?;
    client
        .get_remote_snapshot(&repo_name, &snapshot_id, false, None)
        .map_err(|err| {
            format!(
                "Remote repository {repo_name} is missing source snapshot {snapshot_id}. Run `ait push --line {}` first. ({})",
                required_string_field(&record, "line").unwrap_or_else(|_| "main".to_string()),
                plan_http_error_message(err)
            )
        })?;
    let artifacts = release_publish_artifacts(repo, &record)?;
    let remote_release = client
        .publish_release(
            &repo_name,
            release_id,
            &required_string_field(&record, "version")?,
            &required_string_field(&record, "line")?,
            &snapshot_id,
            &required_string_field(&record, "manifest_hash")?,
            &required_string_field(&record, "profile")?,
            record.get("package").cloned().unwrap_or_else(|| json!({})),
            record.get("checks").cloned().unwrap_or_else(|| json!([])),
            JsonValue::Array(artifacts),
            record.get("formula").cloned().unwrap_or_else(|| json!({})),
            release_publish_metadata(&record),
        )
        .map_err(plan_http_error_message)?;
    let mut metadata = record
        .get("metadata")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.insert(
        "remote_publish".to_string(),
        json!({
            "remote_name": remote_row.name,
            "repo_name": repo_name,
            "release_id": remote_release.get("release_id").and_then(JsonValue::as_str).unwrap_or(release_id),
            "published_at": remote_release.get("updated_at").or_else(|| remote_release.get("created_at")).cloned().unwrap_or(JsonValue::Null),
            "status": remote_release.get("status").cloned().unwrap_or(JsonValue::Null),
            "artifact_count": remote_release.get("artifacts").and_then(JsonValue::as_array).map(Vec::len).unwrap_or(0),
        }),
    );
    update_release(
        repo,
        release_id,
        Some("published"),
        None,
        None,
        None,
        Some(JsonValue::Object(metadata)),
    )?;
    get_release_candidate(repo, release_id)
}

#[allow(clippy::too_many_arguments)]
pub fn create_workflow_release_explicit(
    repo: &RepoRuntime,
    release_id: &str,
    repo_name: &str,
    version: &str,
    line_name: &str,
    snapshot_id: &str,
    manifest_hash: &str,
    profile: &str,
    package_name: Option<&str>,
    package_version: Option<&str>,
    package_requires_python: Option<&str>,
    status: Option<&str>,
    checks_json: &str,
    artifacts_json: &str,
    formula_json: &str,
    metadata_json: &str,
) -> Result<JsonValue, String> {
    validate_release_json_text(checks_json, "checks_json")?;
    validate_release_json_text(artifacts_json, "artifacts_json")?;
    validate_release_json_text(formula_json, "formula_json")?;
    validate_release_json_text(metadata_json, "metadata_json")?;
    let now = current_timestamp();
    let release_store = workflow_release_store(repo)?;
    let record = create_workflow_release_with_store(
        &release_store,
        &WorkflowReleaseRecord {
            release_id: release_id.trim().to_string(),
            repo_name: repo_name.trim().to_string(),
            version: version.trim().to_string(),
            line_name: line_name.trim().to_string(),
            snapshot_id: snapshot_id.trim().to_string(),
            manifest_hash: manifest_hash.trim().to_string(),
            profile: profile.trim().to_string(),
            package_name: normalized_text(package_name),
            package_version: normalized_text(package_version),
            package_requires_python: normalized_text(package_requires_python),
            status: normalized_text(status).unwrap_or_else(|| "candidate".to_string()),
            checks_json: checks_json.to_string(),
            artifacts_json: artifacts_json.to_string(),
            formula_json: formula_json.to_string(),
            metadata_json: metadata_json.to_string(),
            created_at: now.clone(),
            updated_at: now,
        },
    )?;
    Ok(workflow_release_record_json(&record))
}

pub fn list_workflow_releases(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let release_store = workflow_release_store(repo)?;
    Ok(JsonValue::Array(
        list_workflow_releases_with_store(&release_store)?
            .iter()
            .map(workflow_release_record_json)
            .collect(),
    ))
}

pub fn get_workflow_release(repo: &RepoRuntime, release_id: &str) -> Result<JsonValue, String> {
    let release_store = workflow_release_store(repo)?;
    get_workflow_release_with_store(&release_store, release_id)?
        .map(|record| workflow_release_record_json(&record))
        .ok_or_else(|| format!("Unknown local release: {release_id}"))
}

#[allow(clippy::too_many_arguments)]
pub fn update_workflow_release_explicit(
    repo: &RepoRuntime,
    release_id: &str,
    status: Option<&str>,
    checks_json: Option<&str>,
    artifacts_json: Option<&str>,
    formula_json: Option<&str>,
    metadata_json: Option<&str>,
) -> Result<JsonValue, String> {
    let normalized_status = normalized_text(status);
    if let Some(value) = checks_json {
        validate_release_json_text(value, "checks_json")?;
    }
    if let Some(value) = artifacts_json {
        validate_release_json_text(value, "artifacts_json")?;
    }
    if let Some(value) = formula_json {
        validate_release_json_text(value, "formula_json")?;
    }
    if let Some(value) = metadata_json {
        validate_release_json_text(value, "metadata_json")?;
    }
    let release_store = workflow_release_store(repo)?;
    update_workflow_release_with_store(
        &release_store,
        release_id,
        &WorkflowReleaseUpdate {
            status: normalized_status,
            checks_json: checks_json.map(ToString::to_string),
            artifacts_json: artifacts_json.map(ToString::to_string),
            formula_json: formula_json.map(ToString::to_string),
            metadata_json: metadata_json.map(ToString::to_string),
            updated_at: current_timestamp(),
        },
    )?
    .map(|record| workflow_release_record_json(&record))
    .ok_or_else(|| format!("Unknown local release: {release_id}"))
}

pub fn get_release_candidate(repo: &RepoRuntime, release_id: &str) -> Result<JsonValue, String> {
    let release_store = workflow_release_store(repo)?;
    let record = get_workflow_release_with_store(&release_store, release_id)?
        .ok_or_else(|| format!("Unknown release: {release_id}"))?;
    hydrate_release(release_candidate_record_json(&record))
}

pub(super) fn workflow_release_store(
    repo: &RepoRuntime,
) -> Result<impl WorkflowReleaseStore, String> {
    repo.workflow_release_store()
}

pub(super) fn release_task_store(repo: &RepoRuntime) -> Result<impl TaskStore, String> {
    repo.task_store()
}

pub(super) fn workflow_release_record_json(record: &WorkflowReleaseRecord) -> JsonValue {
    json!({
        "release_id": &record.release_id,
        "repo_name": &record.repo_name,
        "version": &record.version,
        "line_name": &record.line_name,
        "snapshot_id": &record.snapshot_id,
        "manifest_hash": &record.manifest_hash,
        "profile": &record.profile,
        "package_name": &record.package_name,
        "package_version": &record.package_version,
        "package_requires_python": &record.package_requires_python,
        "status": &record.status,
        "checks_json": &record.checks_json,
        "artifacts_json": &record.artifacts_json,
        "formula_json": &record.formula_json,
        "metadata_json": &record.metadata_json,
        "created_at": &record.created_at,
        "updated_at": &record.updated_at,
    })
}

pub(super) fn release_candidate_record_json(record: &WorkflowReleaseRecord) -> JsonValue {
    let metadata = parse_json_or_object(&record.metadata_json);
    let package = metadata.get("package").cloned().unwrap_or_else(|| {
        json!({
            "name": &record.package_name,
            "version": &record.package_version,
            "requires_python": &record.package_requires_python,
        })
    });
    json!({
        "release_id": &record.release_id,
        "repo_name": &record.repo_name,
        "version": &record.version,
        "line": &record.line_name,
        "line_name": &record.line_name,
        "snapshot_id": &record.snapshot_id,
        "manifest_hash": &record.manifest_hash,
        "profile": &record.profile,
        "package_name": &record.package_name,
        "package_version": &record.package_version,
        "package_requires_python": &record.package_requires_python,
        "status": &record.status,
        "checks": parse_json_or_array(&record.checks_json),
        "artifacts": parse_json_or_array(&record.artifacts_json),
        "formula": parse_json_or_object(&record.formula_json),
        "metadata": metadata,
        "package": package,
        "created_at": &record.created_at,
        "updated_at": &record.updated_at,
    })
}

pub(super) fn hydrate_release(mut record: JsonValue) -> Result<JsonValue, String> {
    let native_distribution = native_distribution_readiness(&record);
    let next_action = release_next_action(&record)?;
    let checks = record
        .get("checks")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let passed = checks
        .iter()
        .filter(|row| string_field(row, "status").as_deref() == Some("pass"))
        .count();
    let warned = checks
        .iter()
        .filter(|row| string_field(row, "status").as_deref() == Some("warn"))
        .count();
    let failed = checks
        .iter()
        .filter(|row| string_field(row, "status").as_deref() == Some("fail"))
        .count();
    let skipped = checks
        .iter()
        .filter(|row| string_field(row, "status").as_deref() == Some("skipped"))
        .count();
    let blocking = checks
        .iter()
        .filter(|row| bool_field(row, "blocking"))
        .count();
    let obj = record
        .as_object_mut()
        .ok_or_else(|| "release row must be an object".to_string())?;
    obj.insert("next_action".to_string(), next_action);
    obj.insert("native_distribution".to_string(), native_distribution);
    obj.insert(
        "check_summary".to_string(),
        json!({
            "total": checks.len(),
            "passed": passed,
            "warned": warned,
            "failed": failed,
            "skipped": skipped,
            "blocking": blocking,
            "decision": if failed > 0 { "fail" } else if warned > 0 { "warn" } else { "pass" },
        }),
    );
    Ok(record)
}

pub(super) fn release_next_action(record: &JsonValue) -> Result<JsonValue, String> {
    let release_id = required_string_field(record, "release_id")?;
    let status = string_field(record, "status").unwrap_or_default();
    let checks = record
        .get("checks")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let blocking = checks.iter().any(|row| bool_field(row, "blocking"));
    let artifact_kinds = record
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| string_field(row, "kind"))
        .collect::<BTreeSet<_>>();
    let formula_path = record
        .get("formula")
        .and_then(|value| value.get("path"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let remote_publish = record
        .get("metadata")
        .and_then(|value| value.get("remote_publish"))
        .and_then(JsonValue::as_object);
    if status == "published" || remote_publish.is_some() {
        let label = remote_publish
            .and_then(|obj| obj.get("remote_name").or_else(|| obj.get("repo_name")))
            .and_then(JsonValue::as_str)
            .unwrap_or("ait-server");
        return Ok(json!({
            "code": "published_remote",
            "detail": format!("Release is already published to {label}. Reuse `ait release show {release_id} --remote <name>` to inspect the shared record."),
        }));
    }
    if checks.is_empty() {
        return Ok(json!({
            "code": "run_checks",
            "detail": format!("Run `ait release check {release_id}` to record the first structured readiness checks."),
        }));
    }
    if blocking {
        return Ok(json!({
            "code": "resolve_checks",
            "detail": format!("Resolve the blocking release checks, then rerun `ait release check {release_id}`."),
        }));
    }
    if !(artifact_kinds.contains("sdist") && artifact_kinds.contains("wheel")) {
        return Ok(json!({
            "code": "build_candidate",
            "detail": format!("Run `ait release build {release_id}` to produce deterministic release artifacts."),
        }));
    }
    let native_readiness = native_distribution_readiness(record);
    if !bool_field(&native_readiness, "multi_ecosystem_ready") {
        let missing = native_readiness
            .get("missing_targets")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .collect::<Vec<_>>();
        return Ok(json!({
            "code": "complete_native_matrix",
            "detail": format!(
                "Build the complete native target matrix with `ait release native-bundle {release_id} --native-matrix-dir <dir>`; missing targets: {}.",
                if missing.is_empty() { "inspect native_distribution.blockers".to_string() } else { missing.join(", ") }
            ),
        }));
    }
    if formula_path.trim().is_empty() {
        let formula_name = record
            .get("package")
            .and_then(|value| value.get("name"))
            .and_then(JsonValue::as_str)
            .unwrap_or("ait");
        return Ok(json!({
            "code": "generate_formula",
            "detail": format!("Run `ait release formula {release_id} --name {formula_name}` to draft the Homebrew formula surface."),
        }));
    }
    Ok(json!({
        "code": "publish_remote",
        "detail": format!("Run `ait release publish {release_id} --remote <name>` to publish this private release candidate to ait-server."),
    }))
}

pub(super) fn update_release(
    repo: &RepoRuntime,
    release_id: &str,
    status: Option<&str>,
    checks: Option<JsonValue>,
    artifacts: Option<JsonValue>,
    formula: Option<JsonValue>,
    metadata: Option<JsonValue>,
) -> Result<(), String> {
    let current = get_release_candidate(repo, release_id)?;
    let next_status = status
        .map(str::to_string)
        .or_else(|| string_field(&current, "status"))
        .unwrap_or_else(|| "candidate".to_string());
    let next_checks = checks.unwrap_or_else(|| current.get("checks").cloned().unwrap_or(json!([])));
    let next_artifacts =
        artifacts.unwrap_or_else(|| current.get("artifacts").cloned().unwrap_or(json!([])));
    let next_formula =
        formula.unwrap_or_else(|| current.get("formula").cloned().unwrap_or(json!({})));
    let next_metadata =
        metadata.unwrap_or_else(|| current.get("metadata").cloned().unwrap_or(json!({})));
    let release_store = workflow_release_store(repo)?;
    update_workflow_release_with_store(
        &release_store,
        release_id,
        &WorkflowReleaseUpdate {
            status: Some(next_status.clone()),
            checks_json: Some(next_checks.to_string()),
            artifacts_json: Some(next_artifacts.to_string()),
            formula_json: Some(next_formula.to_string()),
            metadata_json: Some(next_metadata.to_string()),
            updated_at: current_timestamp(),
        },
    )?
    .ok_or_else(|| format!("Unknown release: {release_id}"))?;
    Ok(())
}

pub(super) fn assert_publish_ready(record: &JsonValue) -> Result<(), String> {
    let release_id = required_string_field(record, "release_id")?;
    let checks = record
        .get("checks")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if checks.is_empty() {
        return Err(format!(
            "Release {release_id} has no recorded checks. Run `ait release check {release_id}` first."
        ));
    }
    if checks.iter().any(|row| bool_field(row, "blocking")) {
        return Err(format!(
            "Release {release_id} still has blocking checks. Resolve them and rerun `ait release check {release_id}`."
        ));
    }
    let artifacts = record
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let kinds = artifacts
        .iter()
        .filter_map(|row| string_field(row, "kind"))
        .collect::<BTreeSet<_>>();
    let missing = ["sdist", "wheel", "manifest", "checksum"]
        .iter()
        .filter(|kind| !kinds.contains(**kind))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "Release {release_id} is missing built artifacts: {}. Run `ait release build {release_id}` first.",
            missing.join(", ")
        ));
    }
    assert_release_metadata_has_build_profile_contracts(record)?;
    assert_release_artifact_paths_are_publishable(&release_id, &artifacts)?;
    assert_native_agent_artifact_pair(&artifacts)?;
    assert_native_distribution_publish_ready(record)?;
    Ok(())
}

pub(super) fn remote_context(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
) -> Result<(RemoteRow, String), String> {
    let remote_row = repo.remote_row(remote_name)?;
    let repo_name = remote_row
        .repo_name
        .clone()
        .and_then(|value| normalized_text(Some(&value)))
        .unwrap_or_else(|| repo.repo_name());
    Ok((remote_row, repo_name))
}

pub(super) fn http_client(
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
) -> Result<PlanHttpClientManager, String> {
    PlanHttpClientManager::new(PlanHttpClientConfig {
        base_url: remote_row.url.clone(),
        repository_index: repo.repository_index(),
        headers: repo.auth_headers(),
        ..PlanHttpClientConfig::default()
    })
    .map_err(plan_http_error_message)
}

pub(super) fn create_release_id(repo: &RepoRuntime) -> Result<String, String> {
    let prefix = repo.id_namespace_prefix();
    let raw = format!(
        "{:X}",
        Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_else(|| Utc::now().timestamp_micros())
    );
    let id = format!("R-{raw}");
    if prefix.trim().is_empty() {
        Ok(id)
    } else {
        Ok(format!("{}{}", prefix.trim(), id))
    }
}
