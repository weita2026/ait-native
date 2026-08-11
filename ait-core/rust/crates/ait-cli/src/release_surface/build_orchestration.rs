use super::*;

pub fn release_check(
    repo: &RepoRuntime,
    release_id: &str,
    tests_command: Option<&str>,
    skip_tests_reason: Option<&str>,
) -> Result<JsonValue, String> {
    release_check_with_compileall_policy(repo, release_id, tests_command, skip_tests_reason, None)
}

pub(super) fn release_check_with_compileall_policy(
    repo: &RepoRuntime,
    release_id: &str,
    tests_command: Option<&str>,
    skip_tests_reason: Option<&str>,
    compileall_skip_reason: Option<&str>,
) -> Result<JsonValue, String> {
    if normalized_text(tests_command).is_some() && normalized_text(skip_tests_reason).is_some() {
        return Err("Use either `--tests-command` or `--skip-tests-reason`, not both.".to_string());
    }
    let record = get_release_candidate(repo, release_id)?;
    let profile = require_profile(&required_string_field(&record, "profile")?)?;
    let snapshot_id = required_string_field(&record, "snapshot_id")?;
    let line_name = required_string_field(&record, "line")?;
    let bundle = release_source_bundle(repo, &snapshot_id, &profile)?;
    let package = package_metadata_from_bundle(&bundle)?;
    let file_map = &bundle.files;
    let mut checks = Vec::new();

    let workspace_matches = workspace_matches_release_source(repo, &line_name, &snapshot_id);
    checks.push(check_result(
        "workspace_clean",
        "Workspace is clean against the selected line head",
        if workspace_matches { "pass" } else { "fail" },
        if workspace_matches {
            format!("Workspace is clean on line {line_name} at snapshot {snapshot_id}.")
        } else {
            format!("Current workspace is not on line {line_name} at snapshot {snapshot_id}.")
        },
        !workspace_matches,
    ));

    let version_match =
        package.version == required_string_field(&record, "version").unwrap_or_default();
    checks.push(check_result(
        "version_matches_pyproject",
        "Release version matches pyproject.toml",
        if version_match { "pass" } else { "fail" },
        format!("pyproject.toml version is {:?}.", package.version),
        !version_match,
    ));

    let export_ok = materialize_bundle_to_temp(&bundle, "ait-release-check-")
        .map(|temp| temp.source_dir().join("pyproject.toml").exists());
    checks.push(check_result(
        "snapshot_export",
        "Selected snapshot can be exported into an isolated source tree",
        if export_ok.as_ref().copied().unwrap_or(false) {
            "pass"
        } else {
            "fail"
        },
        match &export_ok {
            Ok(true) => "Snapshot exported to an isolated source tree with pyproject.toml present."
                .to_string(),
            Ok(false) => "Snapshot export did not include pyproject.toml.".to_string(),
            Err(err) => format!("Snapshot export failed: {err}"),
        },
        !export_ok.as_ref().copied().unwrap_or(false),
    ));

    let (missing_docs, broken_links) = markdown_link_audit(file_map, profile.release_docs);
    let docs_ok = missing_docs.is_empty() && broken_links.is_empty();
    let mut doc_details = Vec::new();
    if !missing_docs.is_empty() {
        doc_details.push(format!("missing docs: {}", missing_docs.join(", ")));
    }
    if !broken_links.is_empty() {
        doc_details.push(format!(
            "broken links: {}",
            broken_links
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    checks.push(check_result(
        "release_docs_links",
        "Release-facing Markdown docs have valid local links",
        if docs_ok { "pass" } else { "fail" },
        if docs_ok {
            "Release-facing Markdown links resolved cleanly.".to_string()
        } else {
            doc_details.join("; ")
        },
        !docs_ok,
    ));

    let private_findings = scan_private_surface(repo, file_map, profile.release_docs);
    checks.push(check_result(
        "public_surface_private_paths",
        "Release-facing docs do not expose private machine paths or local runtime defaults",
        if private_findings.is_empty() {
            "pass"
        } else {
            "fail"
        },
        if private_findings.is_empty() {
            "No private path or loopback runtime strings were detected in the release-facing docs."
                .to_string()
        } else {
            private_findings
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        },
        !private_findings.is_empty(),
    ));

    checks.push(match normalized_text(compileall_skip_reason) {
        Some(reason) => check_result(
            "compileall",
            "`compileall` status is explicitly recorded",
            "skipped",
            reason,
            false,
        ),
        None => run_compileall_check(&bundle),
    });
    checks.push(run_tests_check(&bundle, tests_command, skip_tests_reason));
    checks.push(presence_check(
        "license_readiness",
        "License and notice artifacts are present for the selected profile",
        profile.license_files,
        file_map,
    ));
    checks.push(presence_any_check(
        "contributor_readiness",
        "Contributor guidance exists for the selected profile",
        profile.contributor_files,
        file_map,
        "Missing CONTRIBUTING.md.",
    ));
    checks.push(presence_any_check(
        "quickstart_readiness",
        "Quickstart guidance exists for the selected profile",
        profile.quickstart_files,
        file_map,
        "Missing release-facing quickstart docs.",
    ));
    checks.push(package_targets_check(&profile, &package));
    checks.push(native_distribution_contract_check(&record));
    checks.push(future_repo_boundary_check(&profile, file_map));
    checks.push(future_repo_prep_check(&profile, file_map));
    checks.push(future_repo_split_dry_run_check(&profile, file_map));
    if profile.requires_package_metadata() {
        checks.push(package_metadata_check(&profile, &package));
        if profile.readme_file.is_some() {
            checks.push(package_readme_links_check(&package, file_map));
        }
    }
    if profile.publish_support {
        checks.push(publish_automation_check(file_map));
    }
    if let Some(check) = release_external_readiness_check(repo)? {
        checks.push(check);
    }

    let failed = checks
        .iter()
        .filter(|row| string_field(row, "status").as_deref() == Some("fail"))
        .count();
    let blocking = checks
        .iter()
        .filter(|row| bool_field(row, "blocking"))
        .count();
    let decision = if failed == 0 { "pass" } else { "fail" };
    let mut metadata = record
        .get("metadata")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.insert("package".to_string(), package.to_json());
    metadata.insert(
        "check_summary".to_string(),
        json!({
            "decision": decision,
            "failed": failed,
            "blocking": blocking,
            "checked_at": current_timestamp(),
        }),
    );
    update_release(
        repo,
        release_id,
        if decision == "pass" {
            Some("checked")
        } else {
            None
        },
        Some(JsonValue::Array(checks)),
        None,
        None,
        Some(JsonValue::Object(metadata)),
    )?;
    get_release_candidate(repo, release_id)
}

pub(super) fn native_agent_artifact_filename(command: &str, version: &str) -> String {
    format!(
        "{command}-{version}-{}-{}{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::EXE_SUFFIX
    )
}

pub(super) fn native_agent_command_source_dirs(repo: &RepoRuntime) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("AIT_RELEASE_NATIVE_COMMAND_DIR") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            candidates.push(path);
        }
    }
    if let Ok(current_executable) = std::env::current_exe() {
        if let Some(parent) = current_executable.parent() {
            candidates.push(parent.to_path_buf());
        }
    }
    let authoritative_root = repo.authoritative_repo_root();
    if let Some(workspace_parent) = authoritative_root.parent() {
        candidates.push(
            workspace_parent
                .join("ait-core")
                .join(".ait")
                .join("cargo-target")
                .join("release"),
        );
    }
    let mut seen = BTreeSet::new();
    candidates.retain(|path| seen.insert(path.clone()));
    candidates
}

pub(super) fn resolve_native_agent_command_source_dir(
    repo: &RepoRuntime,
) -> Result<PathBuf, String> {
    let candidates = native_agent_command_source_dirs(repo);
    for directory in &candidates {
        let normalized = directory.to_string_lossy().replace('\\', "/");
        if normalized.ends_with("/debug")
            || normalized.contains("/target/debug/")
            || normalized.contains("/cargo-target/debug/")
        {
            continue;
        }
        if REQUIRED_NATIVE_AGENT_COMMANDS.iter().all(|command| {
            directory
                .join(format!("{command}{}", std::env::consts::EXE_SUFFIX))
                .is_file()
        }) {
            return Ok(directory.clone());
        }
    }
    let attempted = candidates
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Rust release generation requires the paired ait-agent and ait-agent-worker release artifacts, but no complete release-profile pair was found in: {attempted}. Run `../ait-core/ait.sh core build` from the stable ait repository root (or `./ait.sh core build` inside ait-core) and retry."
    ))
}

pub(super) fn copy_native_agent_command_artifacts_from_dir(
    repo: &RepoRuntime,
    source_dir: &Path,
    dist_dir: &Path,
    version: &str,
) -> Result<Vec<JsonValue>, String> {
    let normalized_source = source_dir.to_string_lossy().replace('\\', "/");
    if normalized_source.ends_with("/debug")
        || normalized_source.contains("/target/debug/")
        || normalized_source.contains("/cargo-target/debug/")
    {
        return Err(format!(
            "Refusing debug-profile native agent release artifacts from {}. Build the canonical release profile first.",
            source_dir.display()
        ));
    }
    let mut artifacts = Vec::new();
    for command in REQUIRED_NATIVE_AGENT_COMMANDS {
        let source = source_dir.join(format!("{command}{}", std::env::consts::EXE_SUFFIX));
        if !source.is_file() {
            return Err(format!(
                "Rust release generation requires `{command}` at {}, but the release artifact is missing. Run the canonical core build and retry.",
                source.display()
            ));
        }
        let destination = dist_dir.join(native_agent_artifact_filename(command, version));
        fs::copy(&source, &destination).map_err(io_error)?;
        let source_permissions = fs::metadata(&source).map_err(io_error)?.permissions();
        fs::set_permissions(&destination, source_permissions).map_err(io_error)?;
        let mut artifact = artifact_info(repo, &destination)?;
        let row = artifact
            .as_object_mut()
            .ok_or_else(|| "Native command artifact projection must be an object.".to_string())?;
        row.insert(
            "kind".to_string(),
            JsonValue::String("native-command".to_string()),
        );
        row.insert(
            "command".to_string(),
            JsonValue::String((*command).to_string()),
        );
        row.insert(
            "runtime_authority".to_string(),
            JsonValue::String("rust".to_string()),
        );
        row.insert("python_fallback".to_string(), JsonValue::Bool(false));
        row.insert(
            "target_os".to_string(),
            JsonValue::String(std::env::consts::OS.to_string()),
        );
        row.insert(
            "target_arch".to_string(),
            JsonValue::String(std::env::consts::ARCH.to_string()),
        );
        row.insert(
            "cargo_profile".to_string(),
            JsonValue::String("release".to_string()),
        );
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

pub(super) fn assert_native_agent_artifact_pair(artifacts: &[JsonValue]) -> Result<(), String> {
    let native_rows = artifacts
        .iter()
        .filter(|row| string_field(row, "kind").as_deref() == Some("native-command"))
        .collect::<Vec<_>>();
    let mut commands = BTreeSet::new();
    for row in native_rows {
        let command = required_string_field(row, "command")?;
        if !commands.insert(command.clone()) {
            return Err(format!(
                "Release native command artifacts contain duplicate `{command}` entries."
            ));
        }
        if string_field(row, "runtime_authority").as_deref() != Some("rust")
            || bool_field(row, "python_fallback")
        {
            return Err(format!(
                "Release native command `{command}` must declare Rust authority with Python fallback disabled."
            ));
        }
        if string_field(row, "cargo_profile").as_deref() != Some("release") {
            return Err(format!(
                "Release native command `{command}` must come from the Cargo release profile."
            ));
        }
    }
    let missing = REQUIRED_NATIVE_AGENT_COMMANDS
        .iter()
        .filter(|command| !commands.contains(**command))
        .copied()
        .collect::<Vec<_>>();
    let unexpected = commands
        .iter()
        .filter(|command| !REQUIRED_NATIVE_AGENT_COMMANDS.contains(&command.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unexpected.is_empty() {
        return Err(format!(
            "Release native agent artifact pair is invalid (missing: {}; unexpected: {}).",
            if missing.is_empty() {
                "none".to_string()
            } else {
                missing.join(", ")
            },
            if unexpected.is_empty() {
                "none".to_string()
            } else {
                unexpected.join(", ")
            }
        ));
    }
    Ok(())
}

pub fn release_build(repo: &RepoRuntime, release_id: &str) -> Result<JsonValue, String> {
    release_build_with_native_matrix(repo, release_id, None)
}

pub fn release_build_with_native_matrix(
    repo: &RepoRuntime,
    release_id: &str,
    native_matrix_dir: Option<&Path>,
) -> Result<JsonValue, String> {
    let record = get_release_candidate(repo, release_id)?;
    let profile = require_profile(&required_string_field(&record, "profile")?)?;
    let snapshot_id = required_string_field(&record, "snapshot_id")?;
    let mut bundle = release_source_bundle(repo, &snapshot_id, &profile)?;
    apply_release_notes(repo, &record, &mut bundle)?;
    let package = package_metadata_from_bundle(&bundle)?;
    let dist_dir = repo.workspace_root().join("dist");
    fs::create_dir_all(&dist_dir).map_err(io_error)?;
    let epoch = release_epoch(&bundle.raw)?;
    let temp_root = materialize_bundle_to_temp(&bundle, "ait-release-build-")?;
    let source_dir = temp_root.source_dir();
    let sdist_path = build_sdist(source_dir, &dist_dir, &package, epoch)?;
    let wheel_path = build_wheel(source_dir, &dist_dir, &package, epoch)?;
    let mut artifacts = vec![
        artifact_info(repo, &sdist_path)?,
        artifact_info(repo, &wheel_path)?,
    ];
    let native_source_dir = resolve_native_agent_command_source_dir(repo)?;
    let native_commands = copy_native_agent_command_artifacts_from_dir(
        repo,
        &native_source_dir,
        &dist_dir,
        &required_string_field(&record, "version")?,
    )?;
    assert_native_agent_artifact_pair(&native_commands)?;
    artifacts.extend(native_commands.clone());
    let (native_bundles, native_distribution) = build_native_distribution(
        repo,
        &record,
        &bundle,
        &profile,
        &dist_dir,
        epoch,
        native_matrix_dir,
    )?;
    artifacts.extend(native_bundles);
    let manifest_path = dist_dir.join(format!(
        "ait-release-{}.manifest.json",
        required_string_field(&record, "version")?
    ));
    let manifest_payload = json!({
        "release_id": release_id,
        "repo_name": required_string_field(&record, "repo_name")?,
        "version": required_string_field(&record, "version")?,
        "line": required_string_field(&record, "line")?,
        "snapshot_id": snapshot_id,
        "manifest_hash": required_string_field(&record, "manifest_hash")?,
        "profile": profile.id,
        "package": package.to_json(),
        "built_at": current_timestamp(),
        "source_date_epoch": epoch,
        "native_commands": native_commands,
        "native_distribution": native_distribution.clone(),
        "artifacts": artifacts,
    });
    fs::write(
        &manifest_path,
        encode_value_pretty_with_newline_error_string(&manifest_payload)?,
    )
    .map_err(io_error)?;
    let manifest_artifact = artifact_info(repo, &manifest_path)?;
    artifacts.push(manifest_artifact.clone());
    let checksum_path = dist_dir.join(format!(
        "ait-release-{}.sha256",
        required_string_field(&record, "version")?
    ));
    let checksum_text = artifacts
        .iter()
        .map(|artifact| {
            format!(
                "{}  {}",
                required_string_field(artifact, "sha256").unwrap_or_default(),
                Path::new(&required_string_field(artifact, "path").unwrap_or_default())
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&checksum_path, checksum_text).map_err(io_error)?;
    artifacts.push(artifact_info(repo, &checksum_path)?);
    artifacts.sort_by_key(|row| required_string_field(row, "kind").unwrap_or_default());
    assert_native_agent_artifact_pair(&artifacts)?;
    assert_release_artifact_paths_are_publishable(release_id, &artifacts)?;

    let mut metadata = record
        .get("metadata")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.insert("package".to_string(), package.to_json());
    metadata.insert(
        "native_distribution".to_string(),
        native_distribution.clone(),
    );
    metadata.insert(
        "build".to_string(),
        json!({
            "dist_dir": relative_or_absolute(repo, &dist_dir),
            "manifest_path": relative_or_absolute(repo, &manifest_path),
            "checksum_path": relative_or_absolute(repo, &checksum_path),
            "built_at": current_timestamp(),
            "source_date_epoch": epoch,
            "builder": "ait_rust_internal_sdist_and_wheel",
            "native_command_source_dir": native_source_dir.to_string_lossy(),
            "native_commands": REQUIRED_NATIVE_AGENT_COMMANDS,
            "rust_release_profile": rust_release_profile_contract(),
            "rust_ci_profile": rust_ci_profile_contract(),
        }),
    );
    update_release(
        repo,
        release_id,
        Some("built"),
        None,
        Some(JsonValue::Array(artifacts)),
        Some(json!({})),
        Some(JsonValue::Object(metadata)),
    )?;
    get_release_candidate(repo, release_id)
}

pub(super) fn replace_native_bundle_artifacts(
    record: &JsonValue,
    mut native_bundles: Vec<JsonValue>,
) -> Vec<JsonValue> {
    let mut artifacts = record
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    artifacts.retain(|artifact| string_field(artifact, "kind").as_deref() != Some("native-bundle"));
    artifacts.append(&mut native_bundles);
    artifacts.sort_by_key(|artifact| {
        (
            string_field(artifact, "kind").unwrap_or_default(),
            string_field(artifact, "target").unwrap_or_default(),
            string_field(artifact, "path").unwrap_or_default(),
        )
    });
    artifacts
}

pub fn release_native_bundle(
    repo: &RepoRuntime,
    release_id: &str,
    native_matrix_dir: &Path,
) -> Result<JsonValue, String> {
    if !native_matrix_dir.is_dir() {
        return Err(format!(
            "Native matrix directory does not exist or is not a directory: {}",
            native_matrix_dir.display()
        ));
    }
    let record = get_release_candidate(repo, release_id)?;
    let profile = require_profile(&required_string_field(&record, "profile")?)?;
    let snapshot_id = required_string_field(&record, "snapshot_id")?;
    let mut source_bundle = release_source_bundle(repo, &snapshot_id, &profile)?;
    apply_release_notes(repo, &record, &mut source_bundle)?;
    let dist_dir = repo.workspace_root().join("dist");
    fs::create_dir_all(&dist_dir).map_err(io_error)?;
    let epoch = release_epoch(&source_bundle.raw)?;
    let (native_bundles, native_distribution) = build_native_distribution(
        repo,
        &record,
        &source_bundle,
        &profile,
        &dist_dir,
        epoch,
        Some(native_matrix_dir),
    )?;
    assert_release_artifact_paths_are_publishable(release_id, &native_bundles)?;
    let artifacts = replace_native_bundle_artifacts(&record, native_bundles);
    let mut metadata = record
        .get("metadata")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.insert(
        "native_distribution".to_string(),
        native_distribution.clone(),
    );
    metadata.insert(
        "native_bundle_build".to_string(),
        json!({
            "builder": "ait_rust_native_bundle_only",
            "matrix_dir": native_matrix_dir.to_string_lossy(),
            "dist_dir": relative_or_absolute(repo, &dist_dir),
            "built_at": current_timestamp(),
            "source_date_epoch": epoch,
            "command_profile": native_distribution.get("command_profile").cloned().unwrap_or(JsonValue::Null),
            "built_targets": native_distribution.get("built_targets").cloned().unwrap_or(json!([])),
            "missing_targets": native_distribution.get("missing_targets").cloned().unwrap_or(json!([])),
            "rejected_targets": native_distribution.get("rejected_targets").cloned().unwrap_or(json!([])),
            "python_distribution_built": false,
            "native_agent_pair_required": false,
            "public_publish": false,
        }),
    );
    update_release(
        repo,
        release_id,
        None,
        None,
        Some(JsonValue::Array(artifacts)),
        None,
        Some(JsonValue::Object(metadata)),
    )?;
    get_release_candidate(repo, release_id)
}

pub fn release_artifact_smoke(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let line_name = repo.current_line_name()?;
    let line = release_local_line_row(repo, &line_name)?;
    let snapshot_id = required_string_field(&line, "head_snapshot_id")?;
    let profile = require_profile("local-cli")?;
    let bundle = release_source_bundle(repo, &snapshot_id, &profile)?;
    let package = package_metadata_from_bundle(&bundle)?;
    let version = package.version.clone();

    let candidate = release_candidate_create(repo, &version, &line_name, profile.id)?;
    let release_id = required_string_field(&candidate, "release_id")?;
    let checked = release_check_with_compileall_policy(
        repo,
        &release_id,
        None,
        Some("The native artifact smoke is an orchestration check; repository test suites already own executable test coverage."),
        Some(NATIVE_RELEASE_SMOKE_COMPILEALL_SKIP_REASON),
    )?;
    let check_decision = checked
        .get("check_summary")
        .and_then(|summary| summary.get("decision"))
        .and_then(JsonValue::as_str)
        .unwrap_or("fail");
    if check_decision != "pass" {
        let blocking = checked
            .get("check_summary")
            .and_then(|summary| summary.get("blocking"))
            .and_then(JsonValue::as_i64)
            .unwrap_or_default();
        return Err(format!(
            "Native release artifact smoke checks failed for {release_id} with decision {check_decision} and {blocking} blocking checks."
        ));
    }

    let built = release_build(repo, &release_id)?;
    let native_readiness = native_distribution_readiness(&built);
    let publish_ready = assert_publish_ready(&built).is_ok();
    if !publish_ready
        && string_field(&native_readiness, "state").as_deref() != Some("partial")
        && string_field(&native_readiness, "state").as_deref() != Some("configured_unbuilt")
    {
        return Err(format!(
            "Native release artifact smoke produced an invalid distribution state: {}",
            native_readiness
        ));
    }
    let artifact_kinds = built
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artifact| string_field(artifact, "kind"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    Ok(json!({
        "contract": "AT.patchset_ci.release_artifact_smoke.v1",
        "status": "pass",
        "release_id": release_id,
        "release_status": string_field(&built, "status").unwrap_or_default(),
        "version": version,
        "line": line_name,
        "snapshot_id": snapshot_id,
        "check_decision": check_decision,
        "artifact_count": artifact_kinds.len(),
        "artifact_kinds": artifact_kinds,
        "native_distribution": native_readiness,
        "publish_ready": publish_ready,
        "python_process_count": 0,
    }))
}

pub(super) fn release_snapshot_bundle(
    repo: &RepoRuntime,
    snapshot_id: &str,
) -> Result<ReleaseBundle, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let raw = export_snapshot_source_manifest_with_store(snapshot_id, &repo.repo_name(), &store)?;
    let blob_ids = raw
        .get("files")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Snapshot source manifest is missing files.".to_string())?
        .iter()
        .map(|row| required_string_field(row, "blob_id"))
        .collect::<Result<Vec<_>, _>>()?;
    let bytes_by_blob_id = store.read_blob_bytes_batch(&blob_ids)?;
    let files = bundle_file_map(&raw, &bytes_by_blob_id)?;
    Ok(ReleaseBundle { raw, files })
}

pub(super) fn release_source_bundle(
    repo: &RepoRuntime,
    snapshot_id: &str,
    profile: &ReleaseProfile,
) -> Result<ReleaseBundle, String> {
    let mut bundle = release_snapshot_bundle(repo, snapshot_id)?;
    let mut files = bundle.files;
    supplement_workspace_file(repo, &mut files, "pyproject.toml");
    for path in profile
        .release_docs
        .iter()
        .chain(profile.license_files)
        .chain(profile.contributor_files)
        .chain(profile.quickstart_files)
    {
        supplement_workspace_file(repo, &mut files, path);
    }
    supplement_release_markdown_dependencies(repo, &mut files, profile.release_docs);
    shape_bundle_files(&mut files, profile)?;
    let file_values = files
        .values()
        .map(bundle_entry_to_json)
        .collect::<Vec<JsonValue>>();
    bundle
        .raw
        .as_object_mut()
        .ok_or_else(|| "release bundle payload must be an object".to_string())?
        .insert("files".to_string(), JsonValue::Array(file_values));
    bundle.files = files;
    Ok(bundle)
}

pub(super) fn bundle_file_map(
    bundle: &JsonValue,
    bytes_by_blob_id: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, BundleEntry>, String> {
    let mut files = BTreeMap::new();
    for row in bundle
        .get("files")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Snapshot source manifest is missing files.".to_string())?
    {
        let path = required_string_field(row, "path")?;
        let blob_id = required_string_field(row, "blob_id")?;
        let data = bytes_by_blob_id.get(&blob_id).cloned().ok_or_else(|| {
            format!("Snapshot source file {path:?} is missing blob {blob_id} content.")
        })?;
        let mode = string_field(row, "mode").unwrap_or_else(|| "0644".to_string());
        files.insert(path.clone(), BundleEntry { path, data, mode });
    }
    Ok(files)
}

pub(super) fn release_external_closure_from_bundle(
    bundle: &ReleaseBundle,
) -> Result<Option<JsonValue>, String> {
    let Some(lockfile_entry) = bundle.files.get(EXTERNAL_LOCKFILE_PATH) else {
        return Ok(None);
    };
    external_release_closure_metadata_from_lockfile_bytes(&lockfile_entry.data)
        .map(Some)
        .map_err(|err| err.to_string())
}

pub(super) fn supplement_workspace_file(
    repo: &RepoRuntime,
    files: &mut BTreeMap<String, BundleEntry>,
    path: &str,
) {
    if files.contains_key(path) || !workspace_matches_release_source_loose(repo) {
        return;
    }
    let Some(target) = supplemental_source_file_path(repo, path) else {
        return;
    };
    let Ok(data) = fs::read(&target) else {
        return;
    };
    let mode = target
        .metadata()
        .map(|meta| format!("{:04o}", filesystem_mode(&meta, 0o644)))
        .unwrap_or_else(|_| "0644".to_string());
    files.insert(
        path.to_string(),
        BundleEntry {
            path: path.to_string(),
            data,
            mode,
        },
    );
}

pub(super) fn supplemental_source_file_path(repo: &RepoRuntime, path: &str) -> Option<PathBuf> {
    let workspace_root = repo.workspace_root();
    let authoritative_root = repo.authoritative_repo_root();
    [workspace_root, authoritative_root]
        .into_iter()
        .map(|root| root.join(path))
        .find(|candidate| candidate.is_file())
}

pub(super) fn supplement_release_markdown_dependencies(
    repo: &RepoRuntime,
    files: &mut BTreeMap<String, BundleEntry>,
    release_docs: &[&str],
) {
    let mut pending = release_docs
        .iter()
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let Some(data) = files.get(&path).map(|entry| entry.data.clone()) else {
            continue;
        };
        let source_dir = Path::new(&path).parent().unwrap_or_else(|| Path::new(""));
        for target in markdown_links(&String::from_utf8_lossy(&data)) {
            if target.is_empty()
                || target.starts_with('#')
                || target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
            {
                continue;
            }
            let target_path = target.split('#').next().unwrap_or("").trim();
            if target_path.is_empty() {
                continue;
            }
            let normalized = normalize_relative_path(source_dir.join(target_path));
            if normalized.starts_with("../") || Path::new(&normalized).is_absolute() {
                continue;
            }
            supplement_workspace_file(repo, files, &normalized);
            if normalized.ends_with(".md") && files.contains_key(&normalized) {
                pending.push(normalized);
            }
        }
    }
}

pub(super) fn shape_bundle_files(
    files: &mut BTreeMap<String, BundleEntry>,
    profile: &ReleaseProfile,
) -> Result<(), String> {
    let excluded = files
        .keys()
        .filter(|path| path_matches_any(path, profile.excluded_paths))
        .cloned()
        .collect::<Vec<_>>();
    for path in excluded {
        files.remove(&path);
    }
    if let Some(pyproject) = files.get("pyproject.toml").cloned() {
        let shaped = shape_pyproject(&String::from_utf8_lossy(&pyproject.data), profile)?;
        files.insert(
            "pyproject.toml".to_string(),
            BundleEntry {
                path: "pyproject.toml".to_string(),
                data: shaped.into_bytes(),
                mode: pyproject.mode,
            },
        );
    }
    if profile.readme_file == Some("README.pypi.md") && !files.contains_key("README.pypi.md") {
        files.insert(
            "README.pypi.md".to_string(),
            BundleEntry {
                path: "README.pypi.md".to_string(),
                data: public_pypi_readme().as_bytes().to_vec(),
                mode: "0644".to_string(),
            },
        );
    }
    Ok(())
}

pub(super) struct MaterializedBundleTemp {
    _root: TempDir,
    source_dir: PathBuf,
    external_materialization: Option<JsonValue>,
}

impl MaterializedBundleTemp {
    pub(super) fn source_dir(&self) -> &Path {
        &self.source_dir
    }

    pub(super) fn external_materialization(&self) -> Option<&JsonValue> {
        self.external_materialization.as_ref()
    }
}

pub(super) fn materialize_bundle_to_temp(
    bundle: &ReleaseBundle,
    prefix: &str,
) -> Result<MaterializedBundleTemp, String> {
    let temp_root = TempDirBuilder::new()
        .prefix(prefix)
        .tempdir()
        .map_err(io_error)?;
    let source_dir = temp_root.path().join("source");
    fs::create_dir_all(&source_dir).map_err(io_error)?;
    materialize_bundle(bundle, &source_dir)?;
    Ok(MaterializedBundleTemp {
        _root: temp_root,
        source_dir,
        external_materialization: None,
    })
}

pub(super) fn materialize_release_bundle_to_temp(
    repo: &RepoRuntime,
    bundle: &ReleaseBundle,
    prefix: &str,
) -> Result<MaterializedBundleTemp, String> {
    let mut temp = materialize_bundle_to_temp(bundle, prefix)?;
    if let Some(lockfile) = bundle.files.get(EXTERNAL_LOCKFILE_PATH) {
        temp.external_materialization = Some(
            crate::external_surface::materialize_locked_external_release_sources(
                repo,
                &lockfile.data,
                temp.source_dir(),
            )?,
        );
    }
    Ok(temp)
}

pub(super) fn materialize_bundle(bundle: &ReleaseBundle, destination: &Path) -> Result<(), String> {
    release_epoch(&bundle.raw)?;
    for entry in bundle.files.values() {
        let target = destination.join(&entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        fs::write(&target, &entry.data).map_err(io_error)?;
        if let Ok(mode) = u32::from_str_radix(entry.mode.trim_start_matches("0o"), 8) {
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

pub(super) fn run_compileall_check(bundle: &ReleaseBundle) -> JsonValue {
    match materialize_bundle_to_temp(bundle, "ait-release-compileall-") {
        Ok(temp_root) => {
            let source_dir = temp_root.source_dir();
            let targets = ["src", "tests"]
                .iter()
                .filter(|path| source_dir.join(path).exists())
                .copied()
                .collect::<Vec<_>>();
            if targets.is_empty() {
                return check_result(
                    "compileall",
                    "`compileall` passes for the exported release source",
                    "fail",
                    "No `src/` or `tests/` tree was present in the release snapshot.",
                    true,
                );
            }
            let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string());
            let mut command = Command::new(python);
            command.arg("-m").arg("compileall");
            for target in targets {
                command.arg(target);
            }
            let output = command.current_dir(source_dir).output();
            match output {
                Ok(output) if output.status.success() => check_result(
                    "compileall",
                    "`compileall` passes for the exported release source",
                    "pass",
                    output_text(&output, "compileall completed."),
                    false,
                ),
                Ok(output) => check_result(
                    "compileall",
                    "`compileall` passes for the exported release source",
                    "fail",
                    output_text(&output, "compileall failed."),
                    true,
                ),
                Err(err) => check_result(
                    "compileall",
                    "`compileall` passes for the exported release source",
                    "fail",
                    err.to_string(),
                    true,
                ),
            }
        }
        Err(err) => check_result(
            "compileall",
            "`compileall` passes for the exported release source",
            "fail",
            err,
            true,
        ),
    }
}

pub(super) fn run_tests_check(
    bundle: &ReleaseBundle,
    tests_command: Option<&str>,
    skip_tests_reason: Option<&str>,
) -> JsonValue {
    if let Some(reason) = normalized_text(skip_tests_reason) {
        return check_result(
            "tests",
            "Release test status is explicitly recorded",
            "skipped",
            reason,
            false,
        );
    }
    let Some(command_text) = normalized_text(tests_command) else {
        return check_result(
            "tests",
            "Release test status is explicitly recorded",
            "fail",
            "No `--tests-command` or `--skip-tests-reason` was supplied.",
            true,
        );
    };
    match materialize_bundle_to_temp(bundle, "ait-release-tests-") {
        Ok(temp_root) => {
            let source_dir = temp_root.source_dir();
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command_text)
                .current_dir(source_dir)
                .output();
            match output {
                Ok(output) if output.status.success() => check_result(
                    "tests",
                    "Release test status is explicitly recorded",
                    "pass",
                    output_text(&output, &command_text),
                    false,
                ),
                Ok(output) => check_result(
                    "tests",
                    "Release test status is explicitly recorded",
                    "fail",
                    output_text(&output, &command_text),
                    true,
                ),
                Err(err) => check_result(
                    "tests",
                    "Release test status is explicitly recorded",
                    "fail",
                    err.to_string(),
                    true,
                ),
            }
        }
        Err(err) => check_result(
            "tests",
            "Release test status is explicitly recorded",
            "fail",
            err,
            true,
        ),
    }
}

pub(super) fn output_text(output: &std::process::Output, fallback: &str) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    fallback.to_string()
}

pub(super) fn package_targets_check(
    profile: &ReleaseProfile,
    package: &PackageMetadata,
) -> JsonValue {
    let missing = profile
        .required_scripts
        .iter()
        .filter(|name| !package.scripts.contains_key(**name))
        .copied()
        .collect::<Vec<_>>();
    let forbidden = profile
        .forbidden_scripts
        .iter()
        .filter(|name| package.scripts.contains_key(**name))
        .copied()
        .collect::<Vec<_>>();
    let ok = missing.is_empty() && forbidden.is_empty();
    check_result(
        "package_targets",
        "Package targets required by the selected profile are present",
        if ok { "pass" } else { "fail" },
        if ok {
            "All required console scripts are declared in pyproject.toml and forbidden scripts are absent."
                .to_string()
        } else {
            [
                (!missing.is_empty()).then(|| format!("Missing scripts: {}.", missing.join(", "))),
                (!forbidden.is_empty())
                    .then(|| format!("Forbidden scripts still present: {}.", forbidden.join(", "))),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; ")
        },
        !ok,
    )
}

pub(super) fn future_repo_boundary_check(
    profile: &ReleaseProfile,
    file_map: &BTreeMap<String, BundleEntry>,
) -> JsonValue {
    match future_repo_boundary(profile, file_map) {
        Ok((mix, failures)) => check_result(
            "future_repo_boundary_contract",
            "Release profile matches the future repo boundary contract",
            if failures.is_empty() { "pass" } else { "fail" },
            if failures.is_empty() {
                format!(
                    "Contract targets and future repository mix match the selected profile: {}.",
                    mix.join(", ")
                )
            } else {
                failures.join("; ")
            },
            !failures.is_empty(),
        ),
        Err(err) => check_result(
            "future_repo_boundary_contract",
            "Release profile matches the future repo boundary contract",
            "fail",
            err,
            true,
        ),
    }
}

pub(super) fn future_repo_boundary(
    profile: &ReleaseProfile,
    file_map: &BTreeMap<String, BundleEntry>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let payload = json_file(file_map, PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH)?;
    let profile_row = payload
        .get("artifact_profiles")
        .and_then(JsonValue::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|row| row.get("profile_id").and_then(JsonValue::as_str) == Some(profile.id))
        })
        .ok_or_else(|| {
            format!(
                "{PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH} is missing artifact profile {:?}.",
                profile.id
            )
        })?;
    let included = normalized_json_list(profile_row.get("included_targets"));
    let excluded = normalized_json_list(profile_row.get("excluded_targets"));
    let declared_mix = normalized_json_list(profile_row.get("future_repository_mix"));
    let required = normalized_list(profile.required_scripts);
    let forbidden = normalized_list(profile.forbidden_scripts);
    let mut failures = Vec::new();
    if required != included {
        failures.push(format!(
            "required_scripts drift from contract included_targets: expected {}, got {}.",
            format_py_list(&included),
            format_py_list(&required)
        ));
    }
    if forbidden != excluded {
        failures.push(format!(
            "forbidden_scripts drift from contract excluded_targets: expected {}, got {}.",
            format_py_list(&excluded),
            format_py_list(&forbidden)
        ));
    }
    let target_rows = payload
        .get("package_targets")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut owners = BTreeSet::new();
    for target in &included {
        if let Some(owner) = target_rows.iter().find_map(|row| {
            (row.get("target_id").and_then(JsonValue::as_str) == Some(target.as_str()))
                .then(|| {
                    row.get("future_repository_owner")
                        .and_then(JsonValue::as_str)
                })
                .flatten()
        }) {
            owners.insert(owner.to_string());
        }
    }
    let derived = owners.into_iter().collect::<Vec<_>>();
    if declared_mix != derived {
        failures.push(format!(
            "artifact profile future_repository_mix does not match the included target owners: expected {}, got {}.",
            format_py_list(&derived),
            format_py_list(&declared_mix)
        ));
    }
    Ok((derived, failures))
}

pub(super) fn future_repo_prep_check(
    profile: &ReleaseProfile,
    file_map: &BTreeMap<String, BundleEntry>,
) -> JsonValue {
    match future_repo_prep(profile, file_map) {
        Ok((repo_ids, failures)) => check_result(
            "future_repo_extraction_prep_contract",
            "Future repository packaging and CI prep contract matches package-target ownership",
            if failures.is_empty() { "pass" } else { "fail" },
            if failures.is_empty() {
                format!(
                    "Prep contract matches future repo ownership and profile mixes for {}.",
                    repo_ids.join(", ")
                )
            } else {
                failures.join("; ")
            },
            !failures.is_empty(),
        ),
        Err(err) => check_result(
            "future_repo_extraction_prep_contract",
            "Future repository packaging and CI prep contract matches package-target ownership",
            "fail",
            err,
            true,
        ),
    }
}

pub(super) fn future_repo_prep(
    _profile: &ReleaseProfile,
    file_map: &BTreeMap<String, BundleEntry>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let package = json_file(file_map, PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH)?;
    let prep = json_file(file_map, PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH)?;
    let mut failures = Vec::new();
    if prep.get("guide_path").and_then(JsonValue::as_str)
        != Some(PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH)
    {
        failures.push(format!("future repo extraction prep contract must declare guide_path `{PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH}`."));
    }
    let package_profiles = package
        .get("artifact_profiles")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let prep_repos = prep
        .get("future_repositories")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let checked_repo_ids = prep_repos
        .iter()
        .filter_map(|row| {
            row.get("repo_id")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    for profile_row in package_profiles {
        let Some(profile_id) = profile_row.get("profile_id").and_then(JsonValue::as_str) else {
            continue;
        };
        let expected = normalized_json_list(profile_row.get("future_repository_mix"));
        let actual = prep_repos
            .iter()
            .filter(|repo_row| {
                normalized_json_list(repo_row.get("artifact_profiles"))
                    .iter()
                    .any(|value| value == profile_id)
            })
            .filter_map(|repo_row| {
                repo_row
                    .get("repo_id")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        if expected != actual {
            failures.push(format!(
                "{profile_id} future repo mix should be {}, got {}.",
                format_py_list(&expected),
                format_py_list(&actual)
            ));
        }
    }
    Ok((checked_repo_ids, failures))
}

pub(super) fn future_repo_split_dry_run_check(
    profile: &ReleaseProfile,
    file_map: &BTreeMap<String, BundleEntry>,
) -> JsonValue {
    match future_repo_split(profile, file_map) {
        Ok((repo_ids, failures)) => check_result(
            "future_repo_split_dry_run_contract",
            "Future repository split dry-run contract matches package-target and prep ownership",
            if failures.is_empty() { "pass" } else { "fail" },
            if failures.is_empty() {
                format!(
                    "Split dry-run contract matches future repo profiles and repo rows for {}.",
                    repo_ids.join(", ")
                )
            } else {
                failures.join("; ")
            },
            !failures.is_empty(),
        ),
        Err(err) => check_result(
            "future_repo_split_dry_run_contract",
            "Future repository split dry-run contract matches package-target and prep ownership",
            "fail",
            err,
            true,
        ),
    }
}

pub(super) fn future_repo_split(
    profile: &ReleaseProfile,
    file_map: &BTreeMap<String, BundleEntry>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let split = json_file(file_map, PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_CONTRACT_PATH)?;
    let mut failures = Vec::new();
    if split.get("guide_path").and_then(JsonValue::as_str)
        != Some(PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH)
    {
        failures.push(format!(
            "future repo split dry-run contract must declare guide_path `{PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH}`."
        ));
    }
    let profile_row = split
        .get("dry_run_profiles")
        .and_then(JsonValue::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|row| row.get("profile_id").and_then(JsonValue::as_str) == Some(profile.id))
        })
        .ok_or_else(|| {
            format!(
                "Split dry-run contract is missing profile {:?}.",
                profile.id
            )
        })?;
    let required = normalized_list(profile.required_scripts);
    let actual_required = normalized_json_list(profile_row.get("required_scripts"));
    if required != actual_required {
        failures.push(format!(
            "{} required_scripts should be {}, got {}.",
            profile.id,
            format_py_list(&required),
            format_py_list(&actual_required)
        ));
    }
    let forbidden = normalized_list(profile.forbidden_scripts);
    let actual_forbidden = normalized_json_list(profile_row.get("forbidden_scripts"));
    if forbidden != actual_forbidden {
        failures.push(format!(
            "{} forbidden_scripts should be {}, got {}.",
            profile.id,
            format_py_list(&forbidden),
            format_py_list(&actual_forbidden)
        ));
    }
    let repo_ids = split
        .get("future_repositories")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| {
            row.get("repo_id")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    Ok((repo_ids, failures))
}

pub(super) fn package_metadata_check(
    profile: &ReleaseProfile,
    package: &PackageMetadata,
) -> JsonValue {
    let mut failures = Vec::new();
    if package.license.as_deref() != Some(profile.license) {
        failures.push(format!("license should be {:?}", profile.license));
    }
    if let Some(readme_file) = profile.readme_file {
        let actual = readme_declared_file(package.readme.as_ref());
        if actual.as_deref() != Some(readme_file) {
            failures.push(format!("readme should target {readme_file}"));
        }
    }
    for label in profile.required_package_urls {
        if !package.urls.contains_key(*label) {
            failures.push(format!("missing project URL: {label}"));
        }
    }
    if package.keywords.len() < 3 {
        failures.push(format!(
            "keywords count {} is below 3",
            package.keywords.len()
        ));
    }
    if package.classifiers.len() < 5 {
        failures.push(format!(
            "classifier count {} is below 5",
            package.classifiers.len()
        ));
    }
    check_result(
        "package_metadata",
        "Public package metadata is ready for PyPI-facing publication",
        if failures.is_empty() { "pass" } else { "fail" },
        if failures.is_empty() {
            "Project URLs, readme target, keywords, classifiers, and license expression are present."
                .to_string()
        } else {
            failures.join("; ")
        },
        !failures.is_empty(),
    )
}

pub(super) fn package_readme_links_check(
    package: &PackageMetadata,
    file_map: &BTreeMap<String, BundleEntry>,
) -> JsonValue {
    let readme = readme_declared_file(package.readme.as_ref()).unwrap_or_default();
    let mut findings = Vec::new();
    if let Some(entry) = file_map.get(&readme) {
        for target in markdown_links(&String::from_utf8_lossy(&entry.data)) {
            if !target.starts_with('#')
                && !target.starts_with("http://")
                && !target.starts_with("https://")
                && !target.starts_with("mailto:")
            {
                findings.push(target);
            }
        }
    } else {
        findings.push(format!("missing readme file: {readme}"));
    }
    check_result(
        "package_readme_links",
        "PyPI-facing package readme avoids local relative links",
        if findings.is_empty() { "pass" } else { "fail" },
        if findings.is_empty() {
            "The public package readme uses only absolute or fragment links.".to_string()
        } else {
            format!(
                "Relative or missing readme targets: {}",
                findings
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        !findings.is_empty(),
    )
}

pub(super) fn publish_automation_check(file_map: &BTreeMap<String, BundleEntry>) -> JsonValue {
    let checks: &[(&str, &[&str])] = &[
        (
            ".github/workflows/pypi-publish.yml",
            &[
                "workflow_dispatch:",
                "push:",
                "tags:",
                "\"v*\"",
                "pypa/gh-action-pypi-publish@release/v1",
                "id-token: write",
                "name: pypi",
                "https://pypi.org/p/ait-native",
            ],
        ),
        (
            ".github/workflows/github-release-publish.yml",
            &[
                "workflow_dispatch:",
                "push:",
                "tags:",
                "\"v*\"",
                "contents: write",
                "gh release create",
                "gh release upload",
                "release-assets-",
            ],
        ),
        (
            "release/PYPI_PUBLISHING.md",
            &[
                "weita2026/ait-native",
                ".github/workflows/pypi-publish.yml",
                "matching `v*` tag",
                "Trusted Publisher",
                "twine upload dist/*",
                "GITHUB_RELEASE_PUBLISHING.md",
            ],
        ),
        (
            "release/GITHUB_RELEASE_PUBLISHING.md",
            &[
                "scripts/github_release_publish.sh",
                ".github/workflows/github-release-publish.yml",
                "release-assets-v*",
                "workflow_dispatch",
                "GITHUB_TOKEN",
            ],
        ),
        ("scripts/github_release_publish.sh", &[]),
    ];
    let mut failures = Vec::new();
    for (path, fragments) in checks {
        let Some(entry) = file_map.get(*path) else {
            failures.push(format!("missing file: {path}"));
            continue;
        };
        let text = String::from_utf8_lossy(&entry.data);
        for fragment in *fragments {
            if !text.contains(fragment) {
                failures.push(format!("{path} missing fragments: {fragment}"));
            }
        }
    }
    check_result(
        "publish_automation",
        "GitHub Release plus PyPI publication workflows and operator handoff are present",
        if failures.is_empty() { "pass" } else { "fail" },
        if failures.is_empty() {
            "Public release workflows, asset helper script, and operator docs are bundled for the clean public repo."
                .to_string()
        } else {
            failures.join("; ")
        },
        !failures.is_empty(),
    )
}

pub(super) fn presence_check(
    check_id: &str,
    label: &str,
    paths: &[&str],
    file_map: &BTreeMap<String, BundleEntry>,
) -> JsonValue {
    let found = paths
        .iter()
        .filter(|path| file_map.contains_key(**path))
        .copied()
        .collect::<Vec<_>>();
    let missing = paths
        .iter()
        .filter(|path| !file_map.contains_key(**path))
        .copied()
        .collect::<Vec<_>>();
    check_result(
        check_id,
        label,
        if missing.is_empty() { "pass" } else { "fail" },
        if missing.is_empty() {
            format!("Found {}.", found.join(", "))
        } else {
            format!("Missing: {}.", missing.join(", "))
        },
        !missing.is_empty(),
    )
}

pub(super) fn presence_any_check(
    check_id: &str,
    label: &str,
    paths: &[&str],
    file_map: &BTreeMap<String, BundleEntry>,
    missing_detail: &str,
) -> JsonValue {
    let found = paths
        .iter()
        .filter(|path| file_map.contains_key(**path))
        .copied()
        .collect::<Vec<_>>();
    check_result(
        check_id,
        label,
        if found.is_empty() { "fail" } else { "pass" },
        if found.is_empty() {
            missing_detail.to_string()
        } else {
            format!("Found {}.", found.join(", "))
        },
        found.is_empty(),
    )
}

pub(super) fn markdown_link_audit(
    file_map: &BTreeMap<String, BundleEntry>,
    paths: &[&str],
) -> (Vec<String>, Vec<String>) {
    let mut missing_docs = Vec::new();
    let mut broken_links = Vec::new();
    for path in paths {
        let Some(entry) = file_map.get(*path) else {
            missing_docs.push((*path).to_string());
            continue;
        };
        let source_dir = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
        for target in markdown_links(&String::from_utf8_lossy(&entry.data)) {
            if target.is_empty()
                || target.starts_with('#')
                || target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
            {
                continue;
            }
            let target_path = target.split('#').next().unwrap_or("").trim();
            if target_path.is_empty() {
                continue;
            }
            let normalized = normalize_relative_path(source_dir.join(target_path));
            if !file_map.contains_key(&normalized) {
                broken_links.push(format!("{path} -> {target}"));
            }
        }
    }
    (missing_docs, broken_links)
}

pub(super) fn markdown_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!' {
            i += 1;
            continue;
        }
        if bytes[i] == b'[' {
            if let Some(close) = text[i..].find("](") {
                let start = i + close + 2;
                if let Some(end) = text[start..].find(')') {
                    links.push(text[start..start + end].trim().to_string());
                    i = start + end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    links
}

pub(super) fn scan_private_surface(
    repo: &RepoRuntime,
    file_map: &BTreeMap<String, BundleEntry>,
    paths: &[&str],
) -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let repo_root = repo.workspace_root().to_string_lossy().to_string();
    let patterns = [
        ("home_path", home.as_str()),
        ("repo_root_path", repo_root.as_str()),
        ("mac_user_path", "/Users/"),
        ("loopback_runtime", "127.0.0.1:8088"),
        ("localhost_runtime", "localhost:8088"),
    ];
    let mut findings = Vec::new();
    for path in paths {
        let Some(entry) = file_map.get(*path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&entry.data);
        for (label, needle) in patterns {
            if !needle.is_empty() && text.contains(needle) {
                findings.push(format!("{path}: {label} ({needle})"));
            }
        }
    }
    findings
}

pub(super) fn check_result(
    check_id: &str,
    label: &str,
    status: &str,
    details: impl Into<String>,
    blocking: bool,
) -> JsonValue {
    json!({
        "check_id": check_id,
        "label": label,
        "status": status,
        "details": details.into(),
        "blocking": blocking,
    })
}

pub(super) fn release_external_readiness_check(
    repo: &RepoRuntime,
) -> Result<Option<JsonValue>, String> {
    let Some(readiness) = external_readiness_report_for_repo(repo)? else {
        return Ok(None);
    };
    Ok(Some(external_readiness_check_row(&readiness)))
}

pub(super) fn release_require_external_readiness(repo: &RepoRuntime) -> Result<(), String> {
    let Some(readiness) = external_readiness_report_for_repo(repo)? else {
        return Ok(());
    };
    if readiness.ready {
        return Ok(());
    }
    Err(format!(
        "Cannot create release candidate because external readiness failed. Run `ait external update --locked` and `ait external doctor --json` before release. {}",
        external_readiness_blocker_details(&readiness)
    ))
}

pub(super) fn external_readiness_check_row(readiness: &ExternalReadinessReport) -> JsonValue {
    let mut row = check_result(
        "external_readiness",
        "External materialization is ready for release and remote workflows",
        if readiness.ready { "pass" } else { "fail" },
        if readiness.ready {
            "External materialization is ready.".to_string()
        } else {
            external_readiness_blocker_details(readiness)
        },
        !readiness.ready,
    );
    if let Some(object) = row.as_object_mut() {
        object.insert("external_readiness".to_string(), readiness.to_json_value());
    }
    row
}
