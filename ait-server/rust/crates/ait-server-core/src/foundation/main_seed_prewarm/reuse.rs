use serde_json::{json, Value as JsonValue};
use std::fs;
use std::time::Instant;

use super::config::PrewarmConfig;
use super::helpers::{duration_seconds, path_string};
use super::manifest::{manifest_json, write_manifest};
use super::paths::verify_required_paths;
use super::PREWARM_MANIFEST_FILE;
use crate::foundation::ci_process_env::ci_process_environment_report;

pub(super) fn reuse_if_current(
    config: &PrewarmConfig,
    started: Instant,
) -> Result<Option<JsonValue>, String> {
    if !config.main_seed_path.is_dir() {
        return Ok(None);
    }
    let manifest_path = config.main_seed_path.join(PREWARM_MANIFEST_FILE);
    let Ok(text) = fs::read_to_string(&manifest_path) else {
        return Ok(None);
    };
    let manifest: JsonValue = serde_json::from_str(&text).map_err(|exc| {
        format!(
            "Failed to parse main-seed prewarm manifest `{}`: {exc}",
            path_string(&manifest_path)
        )
    })?;
    if manifest.get("generation_key").and_then(JsonValue::as_str)
        != Some(config.generation_key.as_str())
    {
        return Ok(None);
    }
    if manifest.get("fingerprint").and_then(JsonValue::as_str)
        != Some(config.fingerprint().as_str())
    {
        return Ok(None);
    }
    let required_paths = match verify_required_paths(config, &config.main_seed_path) {
        Ok(paths) => paths,
        Err(_) if config.source_repo_path.is_some() => {
            return Ok(None);
        }
        Err(message) => return Err(message),
    };
    let mut reused = manifest_json(config, "reused", &[], &required_paths, started);
    reused["previous_manifest_path"] = json!(path_string(&manifest_path));
    reused["reused"] = json!(true);
    write_manifest(&config.main_seed_path, &reused)?;
    Ok(Some(json!({
        "contract": "ait.server.main_seed_prewarm.v1",
        "status": "reused",
        "reused": true,
        "generation_key": config.generation_key,
        "fingerprint": config.fingerprint(),
        "repo_name": config.repo_name,
        "main_seed_path": path_string(&config.main_seed_path),
        "parallelism": config.parallelism,
        "timeout_seconds": config.timeout_seconds,
        "process_environment": ci_process_environment_report(),
        "step_count": config.steps.len(),
        "steps": [],
        "required_paths": required_paths,
        "manifest_path": path_string(&manifest_path),
        "duration_seconds": duration_seconds(started),
        "prewarm_once": true
    })))
}
