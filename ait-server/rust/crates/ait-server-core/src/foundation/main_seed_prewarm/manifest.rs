use serde_json::{json, Value as JsonValue};
use std::fs;
use std::path::Path;
use std::time::Instant;

use super::config::PrewarmConfig;
use super::helpers::{duration_seconds, now_epoch_seconds, path_string};
use super::PREWARM_MANIFEST_FILE;
use crate::foundation::ci_process_env::ci_process_environment_report;

pub(super) fn manifest_json(
    config: &PrewarmConfig,
    status: &str,
    steps: &[JsonValue],
    required_paths: &[JsonValue],
    started: Instant,
) -> JsonValue {
    json!({
        "contract": "ait.server.main_seed_prewarm_manifest.v1",
        "status": status,
        "repo_name": config.repo_name,
        "generation_key": config.generation_key,
        "fingerprint": config.fingerprint(),
        "main_seed_path": path_string(&config.main_seed_path),
        "source_repo_path": config.source_repo_path.as_ref().map(|path| path_string(path)),
        "parallelism": config.parallelism,
        "timeout_seconds": config.timeout_seconds,
        "process_environment": ci_process_environment_report(),
        "step_count": config.steps.len(),
        "steps": steps,
        "required_paths": required_paths,
        "prewarm_once": true,
        "updated_at_epoch_seconds": now_epoch_seconds(),
        "duration_seconds": duration_seconds(started)
    })
}

pub(super) fn write_manifest(seed_path: &Path, manifest: &JsonValue) -> Result<(), String> {
    let manifest_path = seed_path.join(PREWARM_MANIFEST_FILE);
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create prewarm manifest parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|exc| format!("Failed to encode prewarm manifest: {exc}"))?;
    fs::write(&manifest_path, format!("{content}\n")).map_err(|exc| {
        format!(
            "Failed to write main-seed prewarm manifest `{}`: {exc}",
            path_string(&manifest_path)
        )
    })
}
