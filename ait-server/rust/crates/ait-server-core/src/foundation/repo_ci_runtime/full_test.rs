use super::*;

const FULL_TEST_COLLECTION_STDOUT_CAPTURE_LIMIT_BYTES: u64 = 8 * 1024 * 1024;

struct FullTestItemResolution {
    test_items: Vec<JsonValue>,
    test_collection: Option<JsonValue>,
}

pub(super) fn run_full_test_shard_suite(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let runner = suite.runner.as_object().cloned().unwrap_or_default();
    let main_seed_path = config
        .main_seed_path
        .clone()
        .map(Ok)
        .unwrap_or_else(|| default_main_seed_path(config))?;
    let required_paths = full_test_required_paths(config, suite)?;

    let prewarm = match ci_main_seed_prewarm_json(&JsonValue::Object(main_seed_prewarm_payload(
        config,
        &main_seed_path,
        &required_paths,
    ))) {
        Ok(value) => value,
        Err(message) => {
            return Ok(full_test_failure_result(
                started,
                "main_seed_prewarm",
                message,
                &main_seed_path,
                &required_paths,
                None,
            ));
        }
    };
    let item_resolution = match resolve_full_test_items(config, suite, &runner, &main_seed_path) {
        Ok(value) => value,
        Err(message) => {
            return Ok(full_test_failure_result(
                started,
                "test_collection",
                message,
                &main_seed_path,
                &required_paths,
                Some(prewarm),
            ));
        }
    };

    let artifacts = suite_value(config, suite)
        .and_then(|value| value.get("artifacts"))
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();

    let mut shard_payload = JsonMap::new();
    shard_payload.insert(
        "job_id".to_string(),
        json!(format!("repo-ci-{}", config.snapshot_id)),
    );
    shard_payload.insert("job_type".to_string(), json!("repo.ci"));
    shard_payload.insert(
        "main_seed_path".to_string(),
        json!(path_string(&main_seed_path)),
    );
    shard_payload.insert(
        "merged_output_dir".to_string(),
        json!(path_string(&config.output_dir.join(suite.suite_id.trim()))),
    );
    shard_payload.insert(
        "test_items".to_string(),
        JsonValue::Array(item_resolution.test_items),
    );
    shard_payload.insert(
        "payload".to_string(),
        json!({
            "repo_name": config.repo_name,
            "repo_id": config.repo_id,
            "suite_ids": [suite.suite_id.trim()],
            "plane": config.plane,
            "target_line": config.target_line,
            "snapshot_id": config.snapshot_id,
        }),
    );
    shard_payload.insert(
        "runner".to_string(),
        explicit_shard_runner_payload(config, &runner)?,
    );
    shard_payload.insert(
        "artifacts".to_string(),
        json!({
            "summary_json": optional_text(&artifacts, "summary_json")
                .unwrap_or_else(|| "full_repo_shards.json".to_string()),
            "log_path": optional_text(&artifacts, "log_path")
                .unwrap_or_else(|| "full_repo_shards.log".to_string()),
        }),
    );
    if let Some(path) = &config.main_seed_root {
        shard_payload.insert("main_seed_root".to_string(), json!(path_string(path)));
    }
    if let Some(path) = &config.ram_shard_root {
        shard_payload.insert("ram_shard_root".to_string(), json!(path_string(path)));
    }
    if let Some(value) = config.admitted_cpu_tokens {
        shard_payload.insert("admitted_cpu_tokens".to_string(), json!(value));
    }
    if let Some(value) = config.host_cpu_cores {
        shard_payload.insert("host_cpu_cores".to_string(), json!(value));
    }
    if let Some(value) = &config.scheduler_posture {
        shard_payload.insert("scheduler_posture".to_string(), json!(value));
    }
    if let Some(value) = &config.platform {
        shard_payload.insert("platform".to_string(), json!(value));
    }
    if let Some(value) = &config.materialization_strategy {
        shard_payload.insert("materialization_strategy".to_string(), json!(value));
    }

    let mut run = ci_test_shard_run_json(&JsonValue::Object(shard_payload))?;
    run["runner_kind"] = json!("rust_repo_full_test_shards");
    run["main_seed_prewarm"] = prewarm;
    if let Some(collection) = item_resolution.test_collection {
        run["test_collection"] = collection;
    }
    run["shard_run"] = run.clone();
    Ok(run)
}

fn resolve_full_test_items(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    runner: &JsonMap<String, JsonValue>,
    main_seed_path: &Path,
) -> Result<FullTestItemResolution, String> {
    let explicit_items = string_array(runner, "test_items")?;
    if !explicit_items.is_empty() {
        return Ok(FullTestItemResolution {
            test_items: explicit_items
                .into_iter()
                .map(JsonValue::String)
                .collect::<Vec<_>>(),
            test_collection: None,
        });
    }

    let source = full_test_items_source(config, suite, runner);
    if source.as_deref() != Some("server_collect_once_artifact") {
        return Err(format!(
            "Full-test suite `{}` requires explicit runner.test_items.",
            suite.suite_id.trim()
        ));
    }
    let collection = full_test_collection_object(config, suite, runner).ok_or_else(|| {
        format!(
            "Full-test suite `{}` declares server_collect_once_artifact but has no collection config.",
            suite.suite_id.trim()
        )
    })?;
    if !optional_bool(collection, "collect_once_before_sharding")?.unwrap_or(true) {
        return Err(format!(
            "Full-test suite `{}` collection must set collect_once_before_sharding=true.",
            suite.suite_id.trim()
        ));
    }

    let report = run_full_test_collection(config, suite, collection, main_seed_path)?;
    let items = string_array_from_value(report.get("test_items"));
    if items.is_empty() {
        return Err(format!(
            "Full-test suite `{}` server collection produced no test items.",
            suite.suite_id.trim()
        ));
    }
    Ok(FullTestItemResolution {
        test_items: items.into_iter().map(JsonValue::String).collect(),
        test_collection: Some(report),
    })
}

pub(super) fn full_test_items_source(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    runner: &JsonMap<String, JsonValue>,
) -> Option<String> {
    optional_text(runner, "test_items_source").or_else(|| {
        full_test_collection_object(config, suite, runner)
            .and_then(|collection| optional_text(collection, "test_items_source"))
    })
}

pub(super) fn full_test_collection_object<'a>(
    config: &'a RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    runner: &'a JsonMap<String, JsonValue>,
) -> Option<&'a JsonMap<String, JsonValue>> {
    runner
        .get("collection")
        .and_then(JsonValue::as_object)
        .or_else(|| {
            suite_value(config, suite)
                .and_then(|value| value.get("collection"))
                .and_then(JsonValue::as_object)
        })
}

pub(super) fn run_full_test_collection(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    collection: &JsonMap<String, JsonValue>,
    main_seed_path: &Path,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let output_dir = config
        .output_dir
        .join(suite.suite_id.trim())
        .join("collection");
    fs::create_dir_all(&output_dir).map_err(|exc| {
        format!(
            "Failed to create full-test collection output dir `{}`: {exc}",
            path_string(&output_dir)
        )
    })?;
    let output_path = collection_output_path(&output_dir, collection)?;
    let (program, args) = collection_program_and_args(collection)?;
    let env_map = collection_command_env(
        config,
        collection,
        main_seed_path,
        &output_dir,
        &output_path,
    )?;
    let resolved_program = expand_command_program(&program, &env_map, "collection.program")?;
    let runner_timeout_seconds = suite
        .runner
        .as_object()
        .map(|runner| optional_i64(runner, "timeout_seconds"))
        .transpose()?
        .flatten();
    let timeout_seconds = validated_ci_process_timeout_seconds(
        optional_i64(collection, "timeout_seconds")?.or(runner_timeout_seconds),
        "timeout_seconds",
    )?;

    let mut command = Command::new(&resolved_program);
    command.current_dir(main_seed_path);
    command.args(&args);
    apply_clean_ci_process_env(&mut command, &env_map);
    let log_path = output_dir.join("collection.log");
    let command_text = rendered_command(&resolved_program, &args);
    let mut output = run_streamed_command(
        &mut command,
        &log_path,
        &command_text,
        main_seed_path,
        CiProcessStdoutCapture::Optional(FULL_TEST_COLLECTION_STDOUT_CAPTURE_LIMIT_BYTES),
        CiProcessExecutionOptions::from_timeout_seconds(timeout_seconds),
    )
    .map_err(|exc| {
        format!(
            "Failed to execute full-test collection program `{program}` for suite `{}`: {exc}",
            suite.suite_id.trim()
        )
    })?;

    if output.timed_out {
        return Err(format!(
            "Full-test collection for suite `{}` timed out after {timeout_seconds} seconds; log: {}",
            suite.suite_id.trim(),
            path_string(&log_path)
        ));
    }
    if !output.status.success() {
        return Err(format!(
            "Full-test collection for suite `{}` failed with exit code {}; log: {}",
            suite.suite_id.trim(),
            output.status.code().unwrap_or(-1),
            path_string(&log_path)
        ));
    }

    let output_format = optional_text(collection, "output_format")
        .or_else(|| optional_text(collection, "format"))
        .unwrap_or_else(|| "json_array".to_string());
    let raw_items = if output_path.is_file() {
        fs::read_to_string(&output_path).map_err(|exc| {
            format!(
                "Failed to read full-test collection artifact `{}`: {exc}",
                path_string(&output_path)
            )
        })?
    } else {
        if output.stdout_capture_truncated {
            return Err(format!(
                "Full-test collection for suite `{}` did not write its output artifact and stdout exceeded the bounded {}-byte parser limit; log: {}",
                suite.suite_id.trim(),
                FULL_TEST_COLLECTION_STDOUT_CAPTURE_LIMIT_BYTES,
                path_string(&log_path)
            ));
        }
        output.captured_stdout.take().unwrap_or_default()
    };
    let items = parse_collected_test_items(&raw_items, &output_format)?;
    if !output_path.is_file() {
        fs::write(
            &output_path,
            serde_json::to_string_pretty(&items).map_err(|exc| exc.to_string())? + "\n",
        )
        .map_err(|exc| {
            format!(
                "Failed to write full-test collection artifact `{}`: {exc}",
                path_string(&output_path)
            )
        })?;
    }

    let summary_path = output_dir.join("collection-summary.json");
    let report = json!({
        "contract": "ait.server.repo_ci.full_test_collection.v1",
        "status": "pass",
        "source": "server_collect_once_artifact",
        "collect_once_before_sharding": true,
        "suite_id": suite.suite_id.trim(),
        "program": program,
        "args": args,
        "timeout_seconds": timeout_seconds,
        "timed_out": output.timed_out,
        "process_environment": ci_process_environment_report(),
        "stdout_bytes": output.stdout_bytes,
        "stderr_bytes": output.stderr_bytes,
        "stdout_tail": output.stdout_tail,
        "stderr_tail": output.stderr_tail,
        "output_format": output_format,
        "test_count": items.len(),
        "test_items": items,
        "artifacts": {
            "test_items_json": artifact_payload(&output_path),
            "summary_json": artifact_payload(&summary_path),
            "log_path": artifact_payload(&log_path),
        },
        "duration_seconds": duration_seconds(started),
    });
    fs::write(
        &summary_path,
        serde_json::to_string_pretty(&report).map_err(|exc| exc.to_string())? + "\n",
    )
    .map_err(|exc| {
        format!(
            "Failed to write full-test collection summary `{}`: {exc}",
            path_string(&summary_path)
        )
    })?;
    let mut report = report;
    report["artifacts"]["summary_json"] = artifact_payload(&summary_path);
    Ok(report)
}

pub(super) fn collection_program_and_args(
    collection: &JsonMap<String, JsonValue>,
) -> Result<(String, Vec<String>), String> {
    if let Some(program) = optional_text(collection, "program") {
        return Ok((program, string_array(collection, "args")?));
    }
    if let Some(command) = optional_text(collection, "command") {
        return Ok(("/bin/sh".to_string(), vec!["-c".to_string(), command]));
    }
    Err(
        "Full-test server collection requires collection.program or collection.command."
            .to_string(),
    )
}

pub(super) fn collection_output_path(
    output_dir: &Path,
    collection: &JsonMap<String, JsonValue>,
) -> Result<PathBuf, String> {
    let relative = optional_text(collection, "output_path")
        .or_else(|| optional_text(collection, "test_items_path"))
        .unwrap_or_else(|| "test_items.json".to_string());
    let path = Path::new(&relative);
    if path.is_absolute() || path_has_parent_escape(path) {
        return Err(
            "Full-test collection output_path must be relative and stay inside output_dir."
                .to_string(),
        );
    }
    Ok(output_dir.join(path))
}

pub(super) fn collection_command_env(
    config: &RepoCiRuntimeConfig,
    collection: &JsonMap<String, JsonValue>,
    main_seed_path: &Path,
    output_dir: &Path,
    output_path: &Path,
) -> Result<BTreeMap<String, String>, String> {
    let mut explicit_env = BTreeMap::new();
    for (key, value) in &config.env {
        if let Some(text) = value.as_str() {
            explicit_env.insert(key.clone(), text.to_string());
        }
    }
    explicit_env.remove("PYTHONPATH");
    if let Some(raw) = collection.get("env").and_then(JsonValue::as_object) {
        for (key, value) in raw {
            let text = value
                .as_str()
                .ok_or_else(|| "Full-test collection env values must be strings.".to_string())?;
            explicit_env.insert(key.clone(), text.to_string());
        }
    }
    let mut env_map = clean_ci_process_env(&explicit_env);
    let main_seed = path_string(main_seed_path);
    env_map.insert("AIT_REPO_ROOT".to_string(), main_seed.clone());
    env_map.insert("AIT_WORKSPACE_ROOT".to_string(), main_seed.clone());
    env_map.insert("AIT_NATIVE_WORKSPACE_ROOT".to_string(), main_seed);
    env_map.insert(
        "AIT_FULL_TEST_COLLECTION_OUTPUT_DIR".to_string(),
        path_string(output_dir),
    );
    env_map.insert(
        "AIT_TEST_COLLECTION_OUTPUT_PATH".to_string(),
        path_string(output_path),
    );
    if let Some(path) = &config.shared_cargo_target_dir {
        let text = path_string(path);
        env_map.insert("CARGO_TARGET_DIR".to_string(), text.clone());
        env_map.insert("AIT_SHARED_CARGO_TARGET_DIR".to_string(), text);
    }
    if let Some(path) = &config.shared_cargo_build_dir {
        let text = path_string(path);
        env_map.insert("CARGO_BUILD_BUILD_DIR".to_string(), text.clone());
        env_map.insert("AIT_SHARED_CARGO_BUILD_DIR".to_string(), text);
    }
    Ok(env_map)
}

pub(super) fn parse_collected_test_items(
    raw: &str,
    output_format: &str,
) -> Result<Vec<String>, String> {
    match output_format {
        "json_array" | "json" => {
            let value: JsonValue = serde_json::from_str(raw)
                .map_err(|exc| format!("Full-test collection JSON is invalid: {exc}"))?;
            strict_string_array_from_value(&value, "full-test collection JSON")
        }
        "json_object" => {
            let value: JsonValue = serde_json::from_str(raw)
                .map_err(|exc| format!("Full-test collection JSON object is invalid: {exc}"))?;
            let object = value.as_object().ok_or_else(|| {
                "Full-test collection output_format=json_object requires an object.".to_string()
            })?;
            let items = object
                .get("test_items")
                .or_else(|| object.get("items"))
                .ok_or_else(|| {
                    "Full-test collection JSON object requires test_items or items.".to_string()
                })?;
            strict_string_array_from_value(items, "full-test collection JSON object")
        }
        "lines" | "newline" | "text" => {
            let items = raw
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if items.is_empty() {
                return Err("Full-test collection lines output is empty.".to_string());
            }
            Ok(items)
        }
        value => Err(format!(
            "Unsupported full-test collection output_format `{value}`."
        )),
    }
}

pub(super) fn strict_string_array_from_value(
    value: &JsonValue,
    field: &str,
) -> Result<Vec<String>, String> {
    let values = value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array of non-empty strings."))?;
    let mut items = Vec::with_capacity(values.len());
    for item in values {
        let text = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{field} must contain only non-empty strings."))?;
        items.push(text.to_string());
    }
    Ok(items)
}

pub(super) fn expand_command_program(
    program: &str,
    env_map: &BTreeMap<String, String>,
    field: &str,
) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = program.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            output.push(ch);
            continue;
        }
        match chars.peek() {
            Some('{') => {
                chars.next();
                let mut variable = String::new();
                while let Some(&next_char) = chars.peek() {
                    if next_char == '}' {
                        break;
                    }
                    variable.push(next_char);
                    chars.next();
                }
                if chars.next() != Some('}') {
                    return Err(format!(
                        "{field} has unterminated ${{...}} variable reference."
                    ));
                }
                if variable.is_empty() {
                    return Err(format!("{field} has empty ${{}} variable reference."));
                }
                let value = command_env_value(&variable, env_map).ok_or_else(|| {
                    format!("{field} references missing environment variable `{variable}`.")
                })?;
                output.push_str(&value);
            }
            Some(ch) if is_env_var_name_start(*ch) => {
                let mut variable = String::new();
                while let Some(&next_char) = chars.peek() {
                    if is_env_var_name_char(next_char) {
                        variable.push(next_char);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let value = command_env_value(&variable, env_map).ok_or_else(|| {
                    format!("{field} references missing environment variable `{variable}`.")
                })?;
                output.push_str(&value);
            }
            Some(_) | None => output.push('$'),
        }
    }
    Ok(output)
}

pub(super) fn command_env_value(
    variable: &str,
    env_map: &BTreeMap<String, String>,
) -> Option<String> {
    env_map.get(variable).cloned()
}

pub(super) fn is_env_var_name_start(value: char) -> bool {
    value == '_' || value.is_ascii_alphabetic()
}

pub(super) fn is_env_var_name_char(value: char) -> bool {
    is_env_var_name_start(value) || value.is_ascii_digit()
}

pub(super) fn rendered_command(program: &str, args: &[String]) -> String {
    let mut parts = Vec::from([program.to_string()]);
    parts.extend(args.iter().cloned());
    parts.join(" ")
}

pub(super) fn explicit_shard_runner_payload(
    config: &RepoCiRuntimeConfig,
    runner: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let program = optional_text(runner, "program")
        .ok_or_else(|| "Full-test test_shard runner requires runner.program.".to_string())?;
    let mut payload = JsonMap::new();
    payload.insert("kind".to_string(), json!("command"));
    payload.insert("program".to_string(), json!(program));
    payload.insert("args".to_string(), json!(string_array(runner, "args")?));
    payload.insert(
        "append_test_items".to_string(),
        json!(optional_bool(runner, "append_test_items")?.unwrap_or(false)),
    );
    let timeout_seconds = validated_ci_process_timeout_seconds(
        optional_i64(runner, "timeout_seconds")?,
        "runner.timeout_seconds",
    )?;
    payload.insert("timeout_seconds".to_string(), json!(timeout_seconds));
    let mut env = JsonMap::new();
    for (key, value) in &config.env {
        if let Some(text) = value.as_str() {
            env.insert(key.clone(), json!(text));
        }
    }
    env.remove("PYTHONPATH");
    if let Some(raw) = runner.get("env").and_then(JsonValue::as_object) {
        for (key, value) in raw {
            if let Some(text) = value.as_str() {
                env.insert(key.clone(), json!(text));
            }
        }
    }
    if let Some(path) = &config.shared_cargo_target_dir {
        let text = path_string(path);
        env.insert("CARGO_TARGET_DIR".to_string(), json!(text.clone()));
        env.insert("AIT_SHARED_CARGO_TARGET_DIR".to_string(), json!(text));
    }
    if let Some(path) = &config.shared_cargo_build_dir {
        let text = path_string(path);
        env.insert("CARGO_BUILD_BUILD_DIR".to_string(), json!(text.clone()));
        env.insert("AIT_SHARED_CARGO_BUILD_DIR".to_string(), json!(text));
    }
    if !env.is_empty() {
        payload.insert("env".to_string(), JsonValue::Object(env));
    }
    Ok(JsonValue::Object(payload))
}

pub(super) fn is_full_test_suite(suite: &PatchsetSuiteManifest) -> bool {
    matches!(
        suite.suite_id.trim().to_ascii_lowercase().as_str(),
        "full"
            | "full-test"
            | "full_test"
            | "full-repo"
            | "full_repo"
            | "full_repo_zstd_only"
            | "all"
    )
}

pub(super) fn default_main_seed_path(config: &RepoCiRuntimeConfig) -> Result<PathBuf, String> {
    Ok(config
        .main_seed_root
        .clone()
        .map(Ok)
        .unwrap_or_else(default_main_seed_root)?
        .join(safe_path_segment(&config.repo_name))
        .join("main-seed"))
}

pub(super) fn main_seed_prewarm_payload(
    config: &RepoCiRuntimeConfig,
    main_seed_path: &Path,
    required_paths: &[String],
) -> JsonMap<String, JsonValue> {
    let parallelism = config.admitted_cpu_tokens.unwrap_or(1).max(1);
    let mut payload = JsonMap::new();
    payload.insert("repo_name".to_string(), json!(config.repo_name));
    payload.insert(
        "main_seed_path".to_string(),
        json!(path_string(main_seed_path)),
    );
    payload.insert(
        "source_repo_path".to_string(),
        json!(path_string(&config.workspace_path)),
    );
    payload.insert("generation_key".to_string(), json!(config.snapshot_id));
    payload.insert("parallelism".to_string(), json!(parallelism));
    payload.insert("required_paths".to_string(), json!(required_paths));
    payload.insert(
        "prewarm_steps".to_string(),
        JsonValue::Array(prewarm_steps_from_commands(
            &config.prewarm_commands,
            &config.env,
        )),
    );
    payload
}

pub(super) fn full_test_required_paths(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> Result<Vec<String>, String> {
    let Some(suite_value) = suite_value(config, suite).and_then(JsonValue::as_object) else {
        return Ok(Vec::new());
    };
    if let Some(prewarm) = suite_value
        .get("main_seed_prewarm")
        .and_then(JsonValue::as_object)
    {
        return string_array(prewarm, "required_paths");
    }
    string_array(suite_value, "required_paths")
}

pub(super) fn full_test_failure_result(
    started: Instant,
    phase: &str,
    message: String,
    main_seed_path: &Path,
    required_paths: &[String],
    prewarm: Option<JsonValue>,
) -> JsonValue {
    let failure = json!({
        "phase": phase,
        "message": message,
        "main_seed_path": path_string(main_seed_path),
        "required_paths": required_paths,
    });
    let main_seed_prewarm = prewarm.unwrap_or_else(|| {
        json!({
            "contract": "ait.server.main_seed_prewarm.v1",
            "status": "fail",
            "failure": failure.clone(),
            "main_seed_path": path_string(main_seed_path),
            "required_paths": required_paths,
        })
    });
    json!({
        "status": "fail",
        "duration_seconds": duration_seconds(started),
        "failure": failure,
        "main_seed_prewarm": main_seed_prewarm,
    })
}

pub(super) fn prewarm_steps_from_commands(
    commands: &[String],
    env: &JsonMap<String, JsonValue>,
) -> Vec<JsonValue> {
    commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            let mut step_env = JsonMap::new();
            for (key, value) in env {
                if let Some(text) = value.as_str() {
                    step_env.insert(key.clone(), json!(text));
                }
            }
            step_env.insert("AIT_REPO_ROOT".to_string(), json!("."));
            step_env.insert(
                "AIT_SHARED_CARGO_TARGET_DIR".to_string(),
                json!(".ait/cargo-target"),
            );
            step_env.insert("CARGO_TARGET_DIR".to_string(), json!(".ait/cargo-target"));
            step_env.insert(
                "AIT_SHARED_CARGO_BUILD_DIR".to_string(),
                json!(".ait/cargo-build"),
            );
            step_env.insert(
                "CARGO_BUILD_BUILD_DIR".to_string(),
                json!(".ait/cargo-build"),
            );
            json!({
                "step_id": format!("repo-ci-prewarm-{:03}", index + 1),
                "program": "/bin/sh",
                "args": ["-c", command],
                "env": step_env,
            })
        })
        .collect()
}

pub(super) fn native_prewarm_from_full_test_suite_results(
    suite_results: &[JsonValue],
) -> Option<JsonValue> {
    let prewarm = suite_results
        .iter()
        .find_map(|suite| suite.get("main_seed_prewarm"))?;
    let prewarm_status = prewarm
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    let status = if matches!(prewarm_status, "prewarmed" | "reused") {
        "pass"
    } else {
        "fail"
    };
    Some(json!({
        "contract": "ait.server.repo_ci.native_prewarm.v1",
        "status": status,
        "required": true,
        "once_per_repo_ci_run": true,
        "once_per_main_seed_generation": true,
        "main_seed_status": prewarm_status,
        "command_count": prewarm.get("step_count").cloned().unwrap_or(JsonValue::Null),
        "duration_seconds": prewarm.get("duration_seconds").cloned().unwrap_or(JsonValue::Null),
        "reports": prewarm.get("steps").cloned().unwrap_or_else(|| json!([])),
        "artifacts": prewarm.get("artifacts").cloned().unwrap_or_else(|| json!({})),
        "failure": prewarm.get("failure").cloned().unwrap_or(JsonValue::Null),
        "main_seed_prewarm": prewarm,
    }))
}
