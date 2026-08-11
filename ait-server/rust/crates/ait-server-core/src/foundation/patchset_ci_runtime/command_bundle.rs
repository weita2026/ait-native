use super::*;

pub(super) fn run_command_bundle_suite(
    config: &PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    admitted_cpu_tokens: i64,
) -> Result<JsonValue, String> {
    let runner = suite.runner.as_object().cloned().unwrap_or_default();
    let commands = string_array(&runner, "commands")?;
    let cargo_source_identity_sensitive =
        command_bundle_uses_cargo(&commands, &config.prewarm_commands);
    let prepared_shards = prepared_command_bundle_shards(config)?;
    if !cargo_source_identity_sensitive
        && admitted_cpu_tokens > 1
        && commands.len() > 1
        && prepared_shards.len() > 1
    {
        return run_command_bundle_sharded_suite(
            config,
            suite,
            admitted_cpu_tokens,
            commands,
            prepared_shards,
        );
    }

    let mut runner = runner;
    if cargo_source_identity_sensitive && !config.prewarm_commands.is_empty() {
        runner.insert(
            "prewarm_commands".to_string(),
            json!(&config.prewarm_commands),
        );
    }
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
    payload.insert("runner".to_string(), JsonValue::Object(runner));
    if let Some(artifacts) = suite_value(config, suite)
        .and_then(|value| value.get("artifacts"))
        .cloned()
    {
        payload.insert("artifacts".to_string(), artifacts);
    }
    let mut result = ci_command_bundle_run_json(&JsonValue::Object(payload))?;
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "server_ci_gate".to_string(),
            json!({
                "rust_command_bundle_shards": false,
                "rust_command_bundle_single_source": true,
                "cargo_source_identity_sensitive": cargo_source_identity_sensitive,
                "cargo_source_identity_policy": if cargo_source_identity_sensitive {
                    "single_workspace_prewarm"
                } else {
                    "single_workspace"
                },
                "prewarm_uses_runner_workspace": cargo_source_identity_sensitive
                    && !config.prewarm_commands.is_empty(),
                "prewarm_parallelism": admitted_cpu_tokens.max(1),
            }),
        );
    }
    Ok(result)
}

fn command_bundle_uses_cargo(commands: &[String], prewarm_commands: &[String]) -> bool {
    commands
        .iter()
        .chain(prewarm_commands.iter())
        .any(|command| shell_command_invokes_cargo(command))
}

fn shell_command_invokes_cargo(command: &str) -> bool {
    command
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .any(|token| token == "cargo")
}

pub(super) fn command_bundle_base_payload(
    config: &PatchsetCiRuntimeConfig,
    output_dir: PathBuf,
) -> JsonMap<String, JsonValue> {
    let mut payload = JsonMap::new();
    payload.insert(
        "workspace_path".to_string(),
        json!(path_string(&config.workspace_path)),
    );
    payload.insert("output_dir".to_string(), json!(path_string(&output_dir)));
    payload.insert("job_type".to_string(), json!("patchset.ci"));
    payload.insert("job_id".to_string(), json!(config.patchset_id));
    if let Some(path) = &config.temp_dir {
        payload.insert("temp_dir".to_string(), json!(path_string(path)));
    }
    if let Some(path) = &config.shared_cargo_target_dir {
        payload.insert(
            "shared_cargo_target_dir".to_string(),
            json!(path_string(path)),
        );
    }
    if let Some(path) = &config.shared_cargo_build_dir {
        payload.insert(
            "shared_cargo_build_dir".to_string(),
            json!(path_string(path)),
        );
    }
    payload.insert("env".to_string(), JsonValue::Object(config.env.clone()));
    if let Some(materialization) = &config.snapshot_materialization_result {
        payload.insert(
            "snapshot_materialization_result".to_string(),
            json!({
                "contract": "ait.server.patchset_ci.snapshot_materialization_timing.v1",
                "duration_seconds": materialization
                    .get("duration_seconds")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
                "phase_durations_seconds": materialization
                    .get("phase_durations_seconds")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            }),
        );
    }
    payload
}
