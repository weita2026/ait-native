use super::*;

pub(super) fn run_test_discovery_sharded_suite(
    config: &PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    admitted_cpu_tokens: i64,
) -> Result<JsonValue, String> {
    let runner = suite.runner.as_object().cloned().unwrap_or_default();
    let mut payload =
        command_bundle_base_payload(config, config.output_dir.join(suite.suite_id.trim()));
    payload.insert("suite_id".to_string(), json!(suite.suite_id.trim()));
    payload.insert(
        "runner_parallelism".to_string(),
        json!(admitted_cpu_tokens.max(1)),
    );
    payload.insert(
        "admitted_cpu_tokens".to_string(),
        json!(admitted_cpu_tokens.max(1)),
    );
    if runner.get("adapter").and_then(JsonValue::as_str) != Some("command") {
        if let Some(build_cache) = test_discovery_build_cache_payload(config, suite) {
            payload.insert("build_cache".to_string(), build_cache);
        }
    }
    payload.insert("runner".to_string(), JsonValue::Object(runner));
    if let Some(artifacts) = suite_value(config, suite)
        .and_then(|value| value.get("artifacts"))
        .cloned()
    {
        payload.insert("artifacts".to_string(), artifacts);
    }
    let mut result = ci_test_discovery_sharded_run_json(&JsonValue::Object(payload))?;
    let language_adapter = result
        .get("runner")
        .and_then(|runner| runner.get("adapter"))
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown")
        .to_string();
    let cargo_build_cache_policy = result
        .get("diagnostics")
        .and_then(|value| value.get("cargo_build_cache_policy"))
        .cloned()
        .unwrap_or_else(|| json!("disabled"));
    let cargo_adapter = language_adapter == "cargo";
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "server_ci_gate".to_string(),
            json!({
                "generic_test_discovery_runner": true,
                "language_adapter": language_adapter,
                "cargo_build_once": cargo_adapter,
                "cargo_build_cache_policy": cargo_build_cache_policy,
                "command_discovery_once": !cargo_adapter,
                "test_case_shards": true,
                "test_executable_shards": false,
                "test_executable_fallback": cargo_adapter,
                "runner_parallelism": admitted_cpu_tokens.max(1),
                "runner_parallelism_source": "scheduler_admitted_cpu_tokens",
            }),
        );
    }
    Ok(result)
}

pub(super) fn test_discovery_build_cache_payload(
    config: &PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> Option<JsonValue> {
    let shared_cargo_target_dir = config.shared_cargo_target_dir.as_ref()?;
    let materialization = config.snapshot_materialization_result.as_ref()?;
    let runner = suite.runner.as_object();
    let manifest_path = runner
        .and_then(|value| value.get("manifest_path"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Cargo.toml");
    let changed_paths = materialization_changed_paths(materialization);
    let manifest_file_name = format!(
        "{}-{}-{}.json",
        sanitize_cache_path_component(&config.repo_name),
        sanitize_cache_path_component(suite.suite_id.trim()),
        sanitize_cache_path_component(manifest_path),
    );
    let executable_manifest_path = shared_cargo_target_dir
        .join("ait-patchset-ci")
        .join("test-executables")
        .join(manifest_file_name);
    Some(json!({
        "contract": "ait.server.patchset_ci.cargo_build_cache.v1",
        "policy": "reuse_when_rust_inputs_unchanged",
        "executable_manifest_path": path_string(&executable_manifest_path),
        "changed_paths": changed_paths,
        "source": "snapshot_materialization_result",
    }))
}

pub(super) fn materialization_changed_paths(materialization: &JsonValue) -> Vec<String> {
    let mut paths = Vec::new();
    for key in [
        "revision_overlay_paths",
        "deleted_paths",
        "revision_overlay_entries",
    ] {
        let Some(values) = materialization.get(key).and_then(JsonValue::as_array) else {
            continue;
        };
        for value in values {
            if let Some(text) = value.as_str() {
                push_unique_changed_path(&mut paths, text);
            } else if let Some(path) = value
                .as_object()
                .and_then(|object| object.get("path"))
                .and_then(JsonValue::as_str)
            {
                push_unique_changed_path(&mut paths, path);
            }
        }
    }
    paths
}

pub(super) fn push_unique_changed_path(paths: &mut Vec<String>, path: &str) {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return;
    }
    if !paths.iter().any(|existing| existing == trimmed) {
        paths.push(trimmed.to_string());
    }
}

pub(super) fn sanitize_cache_path_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "default".to_string()
    } else {
        sanitized
    }
}
