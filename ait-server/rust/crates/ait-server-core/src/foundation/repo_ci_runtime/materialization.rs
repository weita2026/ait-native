use super::*;

#[derive(Debug)]
pub(super) struct MaterializedFile {
    path: PathBuf,
    content: Vec<u8>,
    mode: Option<u32>,
}

pub(super) fn materialize_workspace(config: &RepoCiRuntimeConfig) -> Result<(), String> {
    fs::create_dir_all(&config.workspace_path).map_err(|exc| {
        format!(
            "Failed to create repo CI workspace `{}`: {exc}",
            path_string(&config.workspace_path)
        )
    })?;
    for file in &config.materialized_files {
        if path_has_parent_escape(&file.path) || file.path.is_absolute() {
            return Err(
                "materialized file paths must be relative and stay inside workspace.".to_string(),
            );
        }
        let path = config.workspace_path.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|exc| {
                format!(
                    "Failed to create materialized file parent `{}`: {exc}",
                    path_string(parent)
                )
            })?;
        }
        fs::write(&path, &file.content)
            .map_err(|exc| format!("Failed to materialize `{}`: {exc}", path_string(&path)))?;
        #[cfg(unix)]
        if let Some(mode) = file.mode {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).map_err(|exc| {
                format!(
                    "Failed to set mode on materialized file `{}`: {exc}",
                    path_string(&path)
                )
            })?;
        }
    }
    Ok(())
}

pub(super) fn run_native_prewarm_once(
    config: &RepoCiRuntimeConfig,
) -> Result<Option<JsonValue>, String> {
    if config.prewarm_commands.is_empty() {
        return Ok(None);
    }
    let mut runner = JsonMap::new();
    runner.insert("kind".to_string(), json!("command_bundle"));
    runner.insert("commands".to_string(), json!([]));
    runner.insert(
        "prewarm_commands".to_string(),
        json!(&config.prewarm_commands),
    );
    let mut payload = command_bundle_base_payload(config, config.output_dir.join("prewarm"));
    payload.insert("prewarm_only".to_string(), json!(true));
    payload.insert("runner".to_string(), JsonValue::Object(runner));
    payload.insert(
        "artifacts".to_string(),
        json!({"summary_json": "prewarm-summary.json", "log_path": "prewarm.log"}),
    );
    let result = ci_command_bundle_run_json(&JsonValue::Object(payload))?;
    Ok(Some(json!({
        "contract": "ait.server.repo_ci.native_prewarm.v1",
        "status": result["status"].clone(),
        "required": true,
        "once_per_repo_ci_run": true,
        "command_count": config.prewarm_commands.len(),
        "duration_seconds": result["duration_seconds"].clone(),
        "reports": result["prewarm"]["reports"].clone(),
        "artifacts": result["artifacts"].clone(),
        "failure": result["failure"].clone(),
    })))
}

pub(super) fn materialized_files_from_request(
    request: &JsonMap<String, JsonValue>,
) -> Result<Vec<MaterializedFile>, String> {
    let values = request
        .get("materialized_files")
        .or_else(|| request.get("snapshot_files"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut files = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let object = value
            .as_object()
            .ok_or_else(|| format!("materialized_files[{index}] must be an object."))?;
        let path = PathBuf::from(required_text(object, "path")?);
        let content = materialized_file_content(object, index)?;
        let mode = optional_text(object, "mode")
            .and_then(|value| u32::from_str_radix(value.trim_start_matches("0o"), 8).ok());
        files.push(MaterializedFile {
            path,
            content,
            mode,
        });
    }
    Ok(files)
}

pub(super) fn materialized_file_content(
    object: &JsonMap<String, JsonValue>,
    index: usize,
) -> Result<Vec<u8>, String> {
    let retired_binary_key = ["content", "base64"].join("_");
    if object.contains_key(retired_binary_key.as_str()) {
        return Err(format!(
            "materialized_files[{index}] must use text content or pack-backed materialization."
        ));
    }
    let content =
        optional_raw_text(object, "content").or_else(|| optional_raw_text(object, "content_utf8"));
    Ok(content.unwrap_or_default().as_bytes().to_vec())
}

pub(super) fn optional_raw_text<'a>(
    object: &'a JsonMap<String, JsonValue>,
    key: &str,
) -> Option<&'a str> {
    object.get(key).and_then(JsonValue::as_str)
}
