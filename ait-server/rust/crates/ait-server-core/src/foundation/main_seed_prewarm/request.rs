use crate::foundation::ci_runtime_json::MainSeedPrewarmJson;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::fs;
use std::time::Instant;

use super::config::PrewarmConfig;
use super::copy_repo::copy_source_repo;
use super::helpers::{duration_seconds, optional_text, path_string};
use super::lock::SeedPrewarmLock;
use super::manifest::{manifest_json, write_manifest};
use super::paths::{lock_path_for_seed, staging_path_for_seed, verify_required_paths};
use super::reuse::reuse_if_current;
use super::steps::run_steps_parallel;
use super::PREWARM_MANIFEST_FILE;
use crate::foundation::ci_process_env::ci_process_environment_report;

pub fn request_has_main_seed_prewarm(request: &JsonMap<String, JsonValue>) -> bool {
    request.get("main_seed_prewarm").is_some()
        || request.get("prewarm").is_some()
        || optional_text(request, "source_repo_path").is_some()
        || request.get("prewarm_steps").is_some()
        || request.get("cargo_packages").is_some()
}

pub fn ci_main_seed_prewarm_for_plan_json(
    request: &JsonValue,
    plan: &JsonValue,
) -> Result<JsonValue, String> {
    MainSeedPrewarmJson::stateless().prewarm_for_plan(request, plan)
}

pub(crate) fn ci_main_seed_prewarm_for_plan_json_impl(
    request: &JsonValue,
    plan: &JsonValue,
) -> Result<JsonValue, String> {
    let request_object = request
        .as_object()
        .ok_or_else(|| "ci-main-seed-prewarm payload must be a JSON object.".to_string())?;
    let mut effective = JsonMap::new();
    for (key, value) in request_object {
        effective.insert(key.clone(), value.clone());
    }
    if !effective.contains_key("main_seed_path") {
        effective.insert(
            "main_seed_path".to_string(),
            plan["main_seed"]["path"].clone(),
        );
    }
    if !effective.contains_key("repo_name") {
        effective.insert("repo_name".to_string(), plan["repo_name"].clone());
    }
    ci_main_seed_prewarm_json_impl(&JsonValue::Object(effective))
}

pub fn ci_main_seed_prewarm_json(request: &JsonValue) -> Result<JsonValue, String> {
    MainSeedPrewarmJson::stateless().prewarm(request)
}

pub(crate) fn ci_main_seed_prewarm_json_impl(request: &JsonValue) -> Result<JsonValue, String> {
    let request_object = request
        .as_object()
        .ok_or_else(|| "ci-main-seed-prewarm payload must be a JSON object.".to_string())?;
    let config = PrewarmConfig::from_request(request_object)?;
    let started = Instant::now();
    let lock_path = lock_path_for_seed(&config.main_seed_path);
    let _lock = SeedPrewarmLock::acquire(&lock_path, config.lock_timeout_ms)?;

    if !config.force {
        if let Some(reused) = reuse_if_current(&config, started)? {
            return Ok(reused);
        }
    }
    if config.reuse_only {
        return Err(format!(
            "main_seed path `{}` is not current for generation `{}`.",
            path_string(&config.main_seed_path),
            config.generation_key
        ));
    }

    let prewarm_target = if config.source_repo_path.is_some() {
        let staging = staging_path_for_seed(&config.main_seed_path);
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|exc| {
                format!(
                    "Failed to remove stale main-seed staging dir `{}`: {exc}",
                    path_string(&staging)
                )
            })?;
        }
        fs::create_dir_all(&staging).map_err(|exc| {
            format!(
                "Failed to create main-seed staging dir `{}`: {exc}",
                path_string(&staging)
            )
        })?;
        copy_source_repo(&config, &staging)?;
        staging
    } else {
        if !config.main_seed_path.is_dir() {
            return Err(format!(
                "main_seed path `{}` is missing and source_repo_path was not provided.",
                path_string(&config.main_seed_path)
            ));
        }
        config.main_seed_path.clone()
    };

    let run_result = run_steps_parallel(&config, &prewarm_target);
    if let Err(message) = run_result {
        if prewarm_target != config.main_seed_path {
            let _ = fs::remove_dir_all(&prewarm_target);
        }
        return Err(message);
    }
    let step_results = run_result?;
    let required_paths = verify_required_paths(&config, &prewarm_target)?;
    let manifest = manifest_json(
        &config,
        "prewarmed",
        &step_results,
        &required_paths,
        started,
    );
    write_manifest(&prewarm_target, &manifest)?;

    let replaced_existing_seed = if prewarm_target != config.main_seed_path {
        if config.main_seed_path.exists() {
            fs::remove_dir_all(&config.main_seed_path).map_err(|exc| {
                format!(
                    "Failed to replace stale main_seed `{}`: {exc}",
                    path_string(&config.main_seed_path)
                )
            })?;
            true
        } else {
            false
        }
    } else {
        false
    };
    if prewarm_target != config.main_seed_path {
        if let Some(parent) = config.main_seed_path.parent() {
            fs::create_dir_all(parent).map_err(|exc| {
                format!(
                    "Failed to create main_seed parent `{}`: {exc}",
                    path_string(parent)
                )
            })?;
        }
        fs::rename(&prewarm_target, &config.main_seed_path).map_err(|exc| {
            format!(
                "Failed to promote main-seed staging dir `{}` to `{}`: {exc}",
                path_string(&prewarm_target),
                path_string(&config.main_seed_path)
            )
        })?;
    }

    Ok(json!({
        "contract": "ait.server.main_seed_prewarm.v1",
        "status": "prewarmed",
        "reused": false,
        "generation_key": config.generation_key,
        "fingerprint": config.fingerprint(),
        "repo_name": config.repo_name,
        "main_seed_path": path_string(&config.main_seed_path),
        "source_repo_path": config.source_repo_path.as_ref().map(|path| path_string(path)),
        "parallelism": config.parallelism,
        "timeout_seconds": config.timeout_seconds,
        "process_environment": ci_process_environment_report(),
        "step_count": config.steps.len(),
        "steps": step_results,
        "required_paths": required_paths,
        "manifest_path": path_string(&config.main_seed_path.join(PREWARM_MANIFEST_FILE)),
        "lock_path": path_string(&lock_path),
        "replaced_existing_seed": replaced_existing_seed,
        "duration_seconds": duration_seconds(started),
        "prewarm_once": true
    }))
}
