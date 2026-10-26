use super::*;

pub fn render_release_text(record: &JsonValue) {
    if string_field(record, "command").as_deref() == Some("release native-source") {
        println!(
            "ait release native-source {}",
            string_field(record, "release_id").unwrap_or_default()
        );
        for field in ["status", "version", "snapshot_id", "profile", "target"] {
            println!(
                "{}: {}",
                if field == "snapshot_id" {
                    "snapshot"
                } else {
                    field
                },
                string_field(record, field).unwrap_or_default()
            );
        }
        println!(
            "descriptor: {}",
            string_field(record, "descriptor_path").unwrap_or_default()
        );
        println!(
            "descriptor sha256: {}",
            string_field(record, "descriptor_sha256").unwrap_or_default()
        );
        return;
    }
    println!(
        "ait release {}",
        string_field(record, "release_id").unwrap_or_default()
    );
    for field in ["status", "version", "line", "snapshot_id", "profile"] {
        println!(
            "{}: {}",
            if field == "snapshot_id" {
                "snapshot"
            } else {
                field
            },
            string_field(record, field).unwrap_or_default()
        );
    }
    if let Some(summary) = release_check_summary_text(record) {
        println!("checks: {summary}");
    }
    if let Some(summary) = release_artifact_summary_text(record) {
        println!("artifacts: {summary}");
    }
    if string_field(record, "profile").as_deref() == Some(FAMILY_RELEASE_PROFILE) {
        let components = record
            .get("family")
            .and_then(|family| family.get("components"))
            .and_then(JsonValue::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        let targets = record
            .get("family")
            .and_then(|family| family.get("targets"))
            .and_then(JsonValue::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        println!(
            "family channel: {}; components: {components}; targets: {targets}",
            string_field(record, "channel").unwrap_or_default()
        );
        if string_field(record, "status").as_deref() == Some("ready_for_protected_ci") {
            println!("promotion: protected CI handoff only; registry writes: no");
        }
    } else if is_generic_release_record(record) {
        let adapter = record
            .get("metadata")
            .and_then(|metadata| metadata.get("release_adapter"));
        let components = adapter
            .and_then(|value| value.get("component_count"))
            .and_then(JsonValue::as_u64)
            .unwrap_or_default();
        let artifacts = adapter
            .and_then(|value| value.get("declared_artifact_count"))
            .and_then(JsonValue::as_u64)
            .unwrap_or_default();
        println!(
            "release adapter: {GENERIC_RELEASE_PROFILE}; components: {components}; declared artifacts: {artifacts}"
        );
    } else if let Some(readiness) = record.get("native_distribution") {
        let built = readiness
            .get("built_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or_default();
        let configured = readiness
            .get("configured_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or_default();
        let ready = bool_field(readiness, "multi_ecosystem_ready");
        println!(
            "native matrix: {built}/{configured} built; ready: {}",
            if ready { "yes" } else { "no" }
        );
        let missing = readiness
            .get("missing_targets")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            println!("native missing: {}", missing.join(", "));
        }
    }
    if let Some(detail) = record
        .get("next_action")
        .and_then(|value| value.get("detail"))
        .and_then(JsonValue::as_str)
    {
        println!("next: {detail}");
    }
    let detail_rows = release_check_detail_rows(record);
    if !detail_rows.is_empty() {
        println!();
        println!("release check detail");
        for row in detail_rows {
            println!(
                "- {} [{}]: {}",
                string_field(&row, "check_id").unwrap_or_default(),
                string_field(&row, "status").unwrap_or_default(),
                string_field(&row, "details").unwrap_or_default()
            );
        }
    }
    if let Some(path) = record
        .get("formula")
        .and_then(|value| value.get("path"))
        .and_then(JsonValue::as_str)
    {
        if !path.trim().is_empty() {
            println!("formula draft: {path}");
        }
    }
}

pub(super) fn release_check_summary_text(record: &JsonValue) -> Option<String> {
    let checks = record.get("checks").and_then(JsonValue::as_array)?;
    if checks.is_empty() {
        return None;
    }
    let summary = record.get("check_summary").and_then(JsonValue::as_object);
    let total = summary
        .and_then(|obj| obj.get("total"))
        .and_then(JsonValue::as_i64)
        .unwrap_or(checks.len() as i64);
    let blocking = summary
        .and_then(|obj| obj.get("blocking"))
        .and_then(JsonValue::as_i64)
        .unwrap_or_else(|| {
            checks
                .iter()
                .filter(|row| bool_field(row, "blocking"))
                .count() as i64
        });
    let decision = summary
        .and_then(|obj| obj.get("decision"))
        .and_then(JsonValue::as_str)
        .unwrap_or(if blocking > 0 { "fail" } else { "pass" });
    if blocking > 0 {
        Some(format!(
            "{decision} ({blocking} blocking / {total} recorded)"
        ))
    } else {
        Some(format!("{decision} ({total} recorded)"))
    }
}

pub(super) fn release_artifact_summary_text(record: &JsonValue) -> Option<String> {
    let artifacts = record.get("artifacts").and_then(JsonValue::as_array)?;
    if artifacts.is_empty() {
        return None;
    }
    let parts = artifacts
        .iter()
        .filter_map(|row| {
            let kind = string_field(row, "kind")?;
            let display = string_field(row, "download_url")
                .or_else(|| string_field(row, "download_path"))
                .or_else(|| string_field(row, "path"))
                .or_else(|| string_field(row, "url"))
                .unwrap_or_default();
            Some(if display.is_empty() {
                kind
            } else {
                format!("{kind}:{display}")
            })
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("; "))
}

pub(super) fn release_check_detail_rows(record: &JsonValue) -> Vec<JsonValue> {
    record
        .get("checks")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|row| {
            bool_field(row, "blocking")
                || !matches!(
                    string_field(row, "status").unwrap_or_default().as_str(),
                    "" | "pass" | "skipped"
                )
        })
        .collect()
}

pub(super) fn recursive_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

pub(super) fn bundle_entry_to_json(entry: &BundleEntry) -> JsonValue {
    let digest = sha256_hex(&entry.data);
    json!({
        "path": entry.path,
        "mode": entry.mode,
        "size_bytes": entry.data.len(),
        "sha256": digest,
        "content_entry_name": release_source_entry_name(&entry.path, &entry.data),
    })
}

pub(super) fn workspace_matches_release_source(
    repo: &RepoRuntime,
    line_name: &str,
    snapshot_id: &str,
) -> bool {
    repo.current_line_name().ok().as_deref() == Some(line_name)
        && release_local_line_row(repo, line_name)
            .ok()
            .and_then(|line| string_field(&line, "head_snapshot_id"))
            .as_deref()
            == Some(snapshot_id)
}

pub(super) fn workspace_matches_release_source_loose(repo: &RepoRuntime) -> bool {
    let Ok(line_name) = repo.current_line_name() else {
        return false;
    };
    release_local_line_row(repo, &line_name).is_ok()
}

pub(super) fn release_local_line_row(
    repo: &RepoRuntime,
    line_name: &str,
) -> Result<JsonValue, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    store.get_line(line_name)
}

pub(super) fn release_epoch(bundle: &JsonValue) -> Result<i64, String> {
    let value = bundle.get("created_at").ok_or_else(|| {
        "Release source Snapshot is missing required `created_at`; a reproducible release epoch cannot be derived."
            .to_string()
    })?;
    let epoch = match value {
        JsonValue::Number(number) => number
            .as_u64()
            .and_then(|seconds| i64::try_from(seconds).ok()),
        JsonValue::String(text) => text
            .parse::<u64>()
            .ok()
            .and_then(|seconds| i64::try_from(seconds).ok())
            .or_else(|| {
                DateTime::parse_from_rfc3339(text)
                    .ok()
                    .map(|timestamp| timestamp.timestamp())
                    .filter(|seconds| *seconds >= 0)
            }),
        _ => None,
    };
    epoch.ok_or_else(|| {
        "Release source Snapshot `created_at` must be a non-negative Unix-seconds integer or an RFC3339 timestamp within the i64 range."
            .to_string()
    })
}

pub(super) fn json_file(
    file_map: &BTreeMap<String, BundleEntry>,
    path: &str,
) -> Result<JsonValue, String> {
    let entry = file_map
        .get(path)
        .ok_or_else(|| format!("Release source snapshot is missing required file: {path}"))?;
    parse_slice_value(&entry.data, &format!("{path} must contain valid JSON"))
        .map_err(|err| format!("{err}."))
}

pub(super) fn parse_json_or_array(text: &str) -> JsonValue {
    parse_value_or(text, json!([]))
}

pub(super) fn parse_json_or_object(text: &str) -> JsonValue {
    parse_value_or(text, json!({}))
}

pub(super) fn toml_string(value: Option<&toml::Value>) -> Option<String> {
    match value {
        Some(toml::Value::String(text)) => normalized_text(Some(text)),
        Some(value) => normalized_text(Some(&value.to_string())),
        None => None,
    }
}

pub(super) fn toml_string_list(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| toml_string(Some(item)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn toml_to_json(value: Option<&toml::Value>) -> Option<JsonValue> {
    let toml_value = value?;
    match toml_value {
        toml::Value::String(text) => Some(JsonValue::String(text.clone())),
        toml::Value::Table(table) => Some(JsonValue::Object(
            table
                .iter()
                .filter_map(|(key, value)| {
                    toml_to_json(Some(value)).map(|value| (key.clone(), value))
                })
                .collect(),
        )),
        toml::Value::Array(items) => Some(JsonValue::Array(
            items
                .iter()
                .filter_map(|item| toml_to_json(Some(item)))
                .collect(),
        )),
        toml::Value::Boolean(flag) => Some(JsonValue::Bool(*flag)),
        toml::Value::Integer(number) => Some(JsonValue::Number((*number).into())),
        toml::Value::Float(_) | toml::Value::Datetime(_) => {
            Some(JsonValue::String(toml_value.to_string()))
        }
    }
}

pub(super) fn readme_declared_file(value: Option<&JsonValue>) -> Option<String> {
    match value {
        Some(JsonValue::String(text)) => normalized_text(Some(text)),
        Some(JsonValue::Object(obj)) => obj
            .get("file")
            .and_then(JsonValue::as_str)
            .and_then(|text| normalized_text(Some(text))),
        _ => None,
    }
}

pub(super) fn normalized_list(values: &[&str]) -> Vec<String> {
    let mut values = values
        .iter()
        .filter_map(|value| normalized_text(Some(value)))
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

pub(super) fn normalized_json_list(value: Option<&JsonValue>) -> Vec<String> {
    let mut values = value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().and_then(|text| normalized_text(Some(text))))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    values.sort();
    values.dedup();
    values
}

pub(super) fn format_py_list(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!("'{value}'"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn path_matches_any(path: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| glob_match(pattern, path))
}

pub(super) fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), text.as_bytes())
}

pub(super) fn glob_match_bytes(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    if pattern.starts_with(b"**") {
        return glob_match_bytes(&pattern[2..], text)
            || (!text.is_empty() && glob_match_bytes(pattern, &text[1..]));
    }
    if pattern[0] == b'*' {
        return glob_match_bytes(&pattern[1..], text)
            || (!text.is_empty() && text[0] != b'/' && glob_match_bytes(pattern, &text[1..]));
    }
    !text.is_empty() && pattern[0] == text[0] && glob_match_bytes(&pattern[1..], &text[1..])
}

pub(super) fn normalize_relative_path(path: PathBuf) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            _ => {}
        }
    }
    parts.join("/")
}

pub(super) fn distribution_name(name: &str, wheel_safe: bool) -> String {
    if wheel_safe {
        name.replace(['-', '.'], "_")
    } else {
        name.to_string()
    }
}

pub(super) fn formula_class_name(name: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if capitalize {
                out.push(ch.to_ascii_uppercase());
                capitalize = false;
            } else {
                out.push(ch);
            }
        } else {
            capitalize = true;
        }
    }
    if out.is_empty() {
        "Ait".to_string()
    } else {
        out
    }
}

pub(super) fn homebrew_license_literal(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return ":cannot_represent".to_string();
    }
    if value.contains(" AND ") && !value.contains(['(', ')']) {
        return format!(
            "all_of: [{}]",
            value
                .split(" AND ")
                .map(|part| json_quote(part.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if value.contains(" OR ") && !value.contains(['(', ')']) {
        return format!(
            "any_of: [{}]",
            value
                .split(" OR ")
                .map(|part| json_quote(part.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    json_quote(value)
}

pub(super) fn package_homepage(package: &JsonMap<String, JsonValue>, repo_name: &str) -> String {
    if let Some(urls) = package.get("urls").and_then(JsonValue::as_object) {
        for label in ["Homepage", "Documentation", "Source"] {
            if let Some(value) = urls.get(label).and_then(JsonValue::as_str) {
                if !value.trim().is_empty() {
                    return value.trim().to_string();
                }
            }
        }
    }
    format!("https://example.invalid/{repo_name}")
}

pub(super) fn artifact_download_name(artifact: &JsonValue) -> Result<String, String> {
    for field in ["path", "url"] {
        if let Some(value) = artifact.get(field).and_then(JsonValue::as_str) {
            if let Some(name) = Path::new(value).file_name().and_then(OsStr::to_str) {
                if !name.trim().is_empty() {
                    return Ok(name.trim().to_string());
                }
            }
        }
    }
    Err("Release artifact is missing a usable download filename.".to_string())
}

pub(super) fn relative_or_absolute(repo: &RepoRuntime, path: &Path) -> String {
    path.strip_prefix(repo.workspace_root())
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

pub(super) fn resolve_artifact_path(repo: &RepoRuntime, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        repo.workspace_root().join(candidate)
    }
}

pub(super) fn file_url(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", path.to_string_lossy())
}

pub(super) fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

pub(super) fn current_timestamp() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(super) fn validate_release_json_text(text: &str, field_name: &str) -> Result<(), String> {
    parse_value(text, &format!("{field_name} must contain valid JSON")).map(|_| ())
}

pub(super) fn required_string_field(value: &JsonValue, key: &str) -> Result<String, String> {
    string_field(value, key).ok_or_else(|| format!("payload is missing required field `{key}`"))
}

pub(super) fn string_field(value: &JsonValue, key: &str) -> Option<String> {
    value.get(key).and_then(|value| match value {
        JsonValue::String(text) => normalized_text(Some(text)),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(flag) => Some(flag.to_string()),
        _ => None,
    })
}

pub(super) fn bool_field(value: &JsonValue, key: &str) -> bool {
    value.get(key).and_then(JsonValue::as_bool).unwrap_or(false)
}

pub(super) fn normalized_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn json_quote(value: &str) -> String {
    encode_string_or(value, "\"\"")
}

pub(super) fn io_error(err: io::Error) -> String {
    err.to_string()
}

pub(super) fn zip_error(err: zip::result::ZipError) -> String {
    err.to_string()
}

pub(super) fn plan_http_error_message(err: PlanHttpClientError) -> String {
    err.to_string()
}
