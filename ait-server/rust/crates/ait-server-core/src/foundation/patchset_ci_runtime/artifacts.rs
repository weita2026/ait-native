use super::*;

#[derive(Debug, Clone)]
pub(super) struct MaterializedFile {
    pub(super) path: PathBuf,
    pub(super) content: Vec<u8>,
    pub(super) mode: Option<u32>,
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

pub(super) fn write_tg1_artifacts(
    output_dir: &Path,
    suite: &PatchsetSuiteManifest,
    summary: &JsonValue,
    log_text: &str,
) -> Result<JsonValue, String> {
    fs::create_dir_all(output_dir).map_err(|exc| {
        format!(
            "Failed to create TG1 output dir `{}`: {exc}",
            path_string(output_dir)
        )
    })?;
    let summary_path = output_dir.join("tg1_required.json");
    let log_path = output_dir.join("tg1_required.log");
    fs::write(
        &summary_path,
        serde_json::to_string(summary).map_err(|exc| exc.to_string())? + "\n",
    )
    .map_err(|exc| format!("Failed to write `{}`: {exc}", path_string(&summary_path)))?;
    fs::write(
        &log_path,
        format!("suite={}\n{log_text}\n", suite.suite_id.trim()),
    )
    .map_err(|exc| format!("Failed to write `{}`: {exc}", path_string(&log_path)))?;
    Ok(json!({
        "summary_json": artifact_payload(&summary_path),
        "log_path": artifact_payload(&log_path),
    }))
}

pub(super) fn tg1_artifacts_with_thread_pool(
    output_dir: &Path,
    suite: &PatchsetSuiteManifest,
    summary: &JsonValue,
    log_text: &str,
    shard_run: &JsonValue,
) -> Result<JsonValue, String> {
    let mut artifacts = write_tg1_artifacts(output_dir, suite, summary, log_text)?;
    if let Some(object) = artifacts.as_object_mut() {
        if let Some(thread_pool_artifacts) = shard_run.get("artifacts") {
            object.insert("thread_pool".to_string(), thread_pool_artifacts.clone());
        }
    }
    Ok(artifacts)
}

pub(super) fn tg1_thread_pool_log_text(shard_run: &JsonValue) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "thread_pool_status={}",
        shard_run
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown")
    ));
    lines.push(format!(
        "thread_pool_duration_seconds={}",
        shard_run
            .get("duration_seconds")
            .map(JsonValue::to_string)
            .unwrap_or_else(|| "null".to_string())
    ));
    if let Some(shards) = shard_run["thread_pool_shards"]["shards"].as_array() {
        for shard in shards {
            lines.push(format!(
                "shard={} status={} test_count={}",
                shard
                    .get("shard_id")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("unknown"),
                shard
                    .get("status")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("unknown"),
                shard
                    .get("test_count")
                    .and_then(JsonValue::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_string())
            ));
        }
    }
    lines.join("\n")
}

pub(super) fn artifact_payload(path: &Path) -> JsonValue {
    let size = fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len());
    json!({
        "path": path_string(path),
        "exists": path.is_file(),
        "size_bytes": size,
    })
}
