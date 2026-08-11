use crate::foundation::ci_command_bundle::ci_command_bundle_run_json;
use crate::foundation::ci_runtime_json::PatchsetCiRunJson;
use crate::foundation::ci_runtime_temp::{
    acquire_cargo_build_dir_lease, ci_runtime_paths_from_request,
};
use crate::foundation::ci_test_discovery_sharded::ci_test_discovery_sharded_run_json;
use crate::foundation::ci_workspace_cleanup::finalize_runtime_workspace_cleanup;
use crate::foundation::patchset_ci::{
    workflow_ready_server_evidence_from_manifest_values, PatchsetSuiteManifest,
};
use crate::foundation::test_shard_runner::ci_test_shard_run_json;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::VecDeque;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

#[path = "patchset_ci_runtime/artifacts.rs"]
mod artifacts;
use artifacts::*;
#[path = "patchset_ci_runtime/helpers.rs"]
mod helpers;
use helpers::*;
#[path = "patchset_ci_runtime/config.rs"]
mod config;
use config::*;
#[path = "patchset_ci_runtime/flow.rs"]
mod flow;
use flow::*;
#[path = "patchset_ci_runtime/materialization.rs"]
mod materialization;
use materialization::*;
#[path = "patchset_ci_runtime/prewarm.rs"]
mod prewarm;
use prewarm::*;
#[path = "patchset_ci_runtime/command_bundle.rs"]
mod command_bundle;
use command_bundle::*;
#[path = "patchset_ci_runtime/tg1.rs"]
mod tg1;
use tg1::*;
#[path = "patchset_ci_runtime/discovery.rs"]
mod discovery;
use discovery::*;
#[path = "patchset_ci_runtime/shards.rs"]
mod shards;
use shards::*;
#[path = "patchset_ci_runtime/status.rs"]
mod status;
use status::*;

const PATCHSET_CI_PROFILE_FULL: &str = "full";
const PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND: &str = "workflow_ready_foreground";
const PATCHSET_CI_PROFILE_TG1_FLOW: &str = "tg1_patchset_ci";
const TG1_PATCHSET_CI_FLOW_CONTRACT: &str = "ait.server.patchset_ci.tg1_flow.v1";
const TG1_REQUIRED_SUITE_ID: &str = "tg1_required";
const TG1_DEFAULT_MINIMUM_COUNT: i64 = 33;
const TG1_DEFAULT_REQUESTED_CPU_TOKENS: i64 = 10;
const TG1_REQUIRED_CPU_TOKENS: i64 = 10;
const PATCHSET_CI_DEFAULT_SUITE_POOL_TOKENS: i64 = 10;
const TG1_NATIVE_DEFAULT_ARGS: &[&str] = &["test", "patchset-ci", "tg1-required", "--json"];

pub fn patchset_ci_run_json(request: &JsonValue) -> Result<JsonValue, String> {
    PatchsetCiRunJson::stateless().run(request)
}

pub(crate) fn patchset_ci_run_json_impl(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "patchset-ci-run payload must be a JSON object.".to_string())?;
    let config = match PatchsetCiRuntimeConfig::from_request(request) {
        Ok(config) => config,
        Err(error) => return finalize_runtime_config_error("patchset", request, error),
    };
    config.validate_flow()?;
    let _cargo_build_lease = config
        .shared_cargo_build_dir
        .as_deref()
        .map(acquire_cargo_build_dir_lease)
        .transpose();
    let result = match _cargo_build_lease.as_ref() {
        Ok(_) => run_patchset_ci_with_config(&config),
        Err(error) => Err(error.clone()),
    };
    finalize_runtime_workspace_cleanup(
        "patchset",
        &config.runtime_cleanup_workspace_path,
        config.cleanup_workspace,
        result,
    )
}

fn finalize_runtime_config_error(
    kind: &str,
    request: &JsonMap<String, JsonValue>,
    error: String,
) -> Result<JsonValue, String> {
    let cleanup_workspace = request
        .get("cleanup_workspace")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let cleanup_path = optional_path(request, "runtime_cleanup_workspace_path")
        .or_else(|| optional_path(request, "workspace_path"));
    match cleanup_path {
        Some(path) if cleanup_workspace => {
            finalize_runtime_workspace_cleanup(kind, &path, true, Err(error))
        }
        _ => Err(error),
    }
}

fn run_patchset_ci_with_config(config: &PatchsetCiRuntimeConfig) -> Result<JsonValue, String> {
    materialize_workspace(&config)?;

    let all_patchset_suites = selected_patchset_suites(&config)?;
    if all_patchset_suites.is_empty() {
        return Err(format!(
            "Patchset {} does not expose any runnable patchset gate manifests in the configured CI suite catalog.",
            config.patchset_id
        ));
    }
    let suites = suites_for_execution_profile(&all_patchset_suites, &config.execution_profile)?;

    let native_prewarm = run_native_prewarm_once(&config)?;
    let suite_results = Vec::new();
    if native_prewarm
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(JsonValue::as_str)
        == Some("fail")
    {
        let mut tests_status = "fail".to_string();
        let mut detail = build_patchset_ci_detail(
            &config,
            &all_patchset_suites,
            &suite_results,
            native_prewarm.clone(),
            None,
            &tests_status,
        );
        attach_flow_finish_evidence(&config, &mut detail, 0);
        attach_workflow_ready_evidence(&config, &mut detail, &mut tests_status)?;
        let mut result = build_result(&config, detail.clone(), suite_results, native_prewarm, None);
        result["patchset_ci_completion"] =
            build_patchset_ci_completion(&config, suites.len(), &detail, &tests_status);
        return Ok(result);
    }

    let (suite_results, suite_pool) = run_suites_with_bounded_pool(&config, &suites)?;

    let mut tests_status = if suite_results.iter().any(|suite| {
        suite.get("blocking").and_then(JsonValue::as_bool) == Some(true)
            && suite.get("status").and_then(JsonValue::as_str) != Some("pass")
    }) {
        "fail".to_string()
    } else {
        "pass".to_string()
    };
    let mut detail = build_patchset_ci_detail(
        &config,
        &all_patchset_suites,
        &suite_results,
        native_prewarm.clone(),
        Some(&suite_pool),
        &tests_status,
    );
    attach_flow_finish_evidence(&config, &mut detail, suite_results.len());
    attach_workflow_ready_evidence(&config, &mut detail, &mut tests_status)?;
    let policy_job_payload = if config.policy_mode == "async" {
        Some(policy_job_payload(&config))
    } else {
        None
    };
    let mut result = build_result(
        &config,
        detail.clone(),
        suite_results.clone(),
        native_prewarm,
        policy_job_payload,
    );
    result["patchset_ci_completion"] =
        build_patchset_ci_completion(&config, suites.len(), &detail, &tests_status);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialization_changed_paths_ignore_copy_up_paths() {
        let materialization = json!({
            "copy_up_paths": ["rust/Cargo.toml", "rust/crates/ait-server/src/lib.rs"],
            "revision_overlay_paths": ["README.md"],
            "deleted_paths": ["docs/old.md"],
            "revision_overlay_entries": [
                {"path": "ci/patch_ci.json"},
                {"path": "README.md"}
            ]
        });

        let paths = materialization_changed_paths(&materialization);

        assert_eq!(
            paths,
            vec![
                "README.md".to_string(),
                "docs/old.md".to_string(),
                "ci/patch_ci.json".to_string()
            ]
        );
    }

    #[test]
    fn sanitize_cache_path_component_keeps_manifest_cache_paths_file_safe() {
        assert_eq!(
            sanitize_cache_path_component("rust/Cargo.toml"),
            "rust-Cargo-toml"
        );
        assert_eq!(sanitize_cache_path_component(""), "default");
    }
}
