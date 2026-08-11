use crate::foundation::ci_command_bundle::ci_command_bundle_run_json;
use crate::foundation::ci_process_env::{
    apply_clean_ci_process_env, ci_process_environment_report, clean_ci_process_env,
};
use crate::foundation::ci_process_stream::{
    run_streamed_command, validated_ci_process_timeout_seconds, CiProcessExecutionOptions,
    CiProcessStdoutCapture,
};
use crate::foundation::ci_runtime_json::RepoCiRunJson;
use crate::foundation::ci_runtime_temp::{
    acquire_cargo_build_dir_lease, ci_runtime_paths_from_request,
};
use crate::foundation::ci_workspace_cleanup::finalize_runtime_workspace_cleanup;
use crate::foundation::main_seed_prewarm::ci_main_seed_prewarm_json;
use crate::foundation::patchset_ci::PatchsetSuiteManifest;
use crate::foundation::test_shard_runner::ci_test_shard_run_json;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const DEFAULT_REPO_CI_PLANE: &str = "nightly";
const REPO_CI_PLANES: [&str; 3] = ["nightly", "release", "post_land_regression"];
pub fn repo_ci_run_json(request: &JsonValue) -> Result<JsonValue, String> {
    RepoCiRunJson::stateless().run(request)
}

pub(crate) fn repo_ci_run_json_impl(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "repo-ci-run payload must be a JSON object.".to_string())?;
    let config = match RepoCiRuntimeConfig::from_request(request) {
        Ok(config) => config,
        Err(error) => {
            let cleanup_workspace = request
                .get("cleanup_workspace")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            let cleanup_path = request
                .get("workspace_path")
                .and_then(JsonValue::as_str)
                .map(PathBuf::from);
            return match cleanup_path {
                Some(path) if cleanup_workspace => {
                    finalize_runtime_workspace_cleanup("repo", &path, true, Err(error))
                }
                _ => Err(error),
            };
        }
    };
    let _cargo_build_lease = config
        .shared_cargo_build_dir
        .as_deref()
        .map(acquire_cargo_build_dir_lease)
        .transpose();
    let result = match _cargo_build_lease.as_ref() {
        Ok(_) => run_repo_ci_with_config(&config),
        Err(error) => Err(error.clone()),
    };
    finalize_runtime_workspace_cleanup(
        "repo",
        &config.workspace_path,
        config.cleanup_workspace,
        result,
    )
}

fn run_repo_ci_with_config(config: &RepoCiRuntimeConfig) -> Result<JsonValue, String> {
    materialize_workspace(&config)?;

    let selected_suites = selected_repo_suites(&config)?;
    if selected_suites.is_empty() {
        return Err(format!(
            "No repo CI suites matched plane `{}`.",
            config.plane
        ));
    }

    let full_test_only = selected_suites.iter().all(is_full_test_suite);
    let mut native_prewarm = if full_test_only {
        None
    } else {
        run_native_prewarm_once(&config)?
    };
    let mut suite_results = Vec::new();
    if native_prewarm
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(JsonValue::as_str)
        == Some("fail")
    {
        let detail = build_repo_ci_detail(&config, &suite_results, native_prewarm.clone(), "fail");
        let result = build_result(&config, detail, suite_results, native_prewarm);
        return Ok(result);
    }

    for suite in &selected_suites {
        suite_results.push(run_one_suite(&config, suite)?);
    }

    if native_prewarm.is_none() {
        native_prewarm = native_prewarm_from_full_test_suite_results(&suite_results);
    }

    let tests_status = if suite_results
        .iter()
        .any(|suite| suite.get("status").and_then(JsonValue::as_str) != Some("pass"))
    {
        "fail"
    } else {
        "pass"
    };
    let detail = build_repo_ci_detail(
        &config,
        &suite_results,
        native_prewarm.clone(),
        tests_status,
    );
    let result = build_result(&config, detail, suite_results, native_prewarm);
    Ok(result)
}

#[path = "repo_ci_runtime/config.rs"]
mod config;
use config::*;
#[path = "repo_ci_runtime/suite_selection.rs"]
mod suite_selection;
use suite_selection::*;
#[path = "repo_ci_runtime/materialization.rs"]
mod materialization;
use materialization::*;
#[path = "repo_ci_runtime/suite_runner.rs"]
mod suite_runner;
use suite_runner::*;
#[path = "repo_ci_runtime/full_test.rs"]
mod full_test;
use full_test::*;
#[path = "repo_ci_runtime/artifacts.rs"]
mod artifacts;
use artifacts::*;
#[path = "repo_ci_runtime/paths.rs"]
mod paths;
use paths::*;
