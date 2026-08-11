use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::config::DiscoveryShardedConfig;
use super::paths::{optional_text, path_string};

pub(super) fn write_artifacts(
    config: &DiscoveryShardedConfig,
    summary: &JsonValue,
) -> Result<JsonValue, String> {
    if let Some(parent) = config.summary_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create summary artifact parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    if let Some(parent) = config.log_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create log artifact parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    fs::write(
        &config.summary_path,
        serde_json::to_string_pretty(summary).map_err(|exc| exc.to_string())? + "\n",
    )
    .map_err(|exc| {
        format!(
            "Failed to write summary artifact `{}`: {exc}",
            path_string(&config.summary_path)
        )
    })?;
    fs::write(&config.log_path, merged_log_text(summary)).map_err(|exc| {
        format!(
            "Failed to write merged log artifact `{}`: {exc}",
            path_string(&config.log_path)
        )
    })?;
    Ok(json!({
        "summary_json": artifact_payload(&config.summary_path),
        "log_path": artifact_payload(&config.log_path),
    }))
}

fn merged_log_text(summary: &JsonValue) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "status={}",
        summary
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown")
    ));
    lines.push(format!(
        "runner_kind={}",
        summary["runner"]["kind"].as_str().unwrap_or("unknown")
    ));
    lines.push(format!(
        "adapter={}",
        summary["runner"]["adapter"].as_str().unwrap_or("unknown")
    ));
    lines.push(format!(
        "executable_count={}",
        summary["discovery"]["executable_count"]
            .as_u64()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "0".to_string())
    ));
    lines.push(format!(
        "test_case_count={}",
        summary["discovery"]["test_case_count"]
            .as_u64()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "0".to_string())
    ));
    for shard in summary["test_shards"]["shards"]
        .as_array()
        .into_iter()
        .flatten()
    {
        lines.push(format!(
            "shard={} status={} executable_count={} test_case_count={} fallback_executable_count={}",
            shard["shard_id"].as_str().unwrap_or("unknown"),
            shard["status"].as_str().unwrap_or("unknown"),
            shard["executable_count"]
                .as_u64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "0".to_string()),
            shard["test_case_count"]
                .as_u64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "0".to_string()),
            shard["fallback_executable_count"]
                .as_u64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "0".to_string())
        ));
    }
    lines.join("\n") + "\n"
}

pub(super) fn report_passed(report: &JsonValue) -> bool {
    report.get("status").and_then(JsonValue::as_str) == Some("pass")
}

fn artifact_payload(path: &Path) -> JsonValue {
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

pub(super) fn artifact_path(
    output_dir: &Path,
    artifacts: &JsonMap<String, JsonValue>,
    key: &str,
    default_name: &str,
) -> Result<PathBuf, String> {
    let rel = optional_text(artifacts, key).unwrap_or_else(|| default_name.to_string());
    if path_has_parent_escape(&rel) || Path::new(&rel).is_absolute() {
        return Err(format!(
            "Artifact path `{key}` must be relative and stay inside output_dir."
        ));
    }
    Ok(output_dir.join(rel))
}

fn path_has_parent_escape(value: &str) -> bool {
    Path::new(value)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}
