use super::*;

pub(super) fn run_native_prewarm_once(
    config: &PatchsetCiRuntimeConfig,
) -> Result<Option<JsonValue>, String> {
    if let Some(main_seed_prewarm) = &config.main_seed_prewarm {
        let main_seed_status = main_seed_prewarm
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or("fail");
        let status = if matches!(main_seed_status, "prewarmed" | "reused") {
            "pass"
        } else {
            "fail"
        };
        let command_count = main_seed_prewarm
            .get("step_count")
            .cloned()
            .unwrap_or_else(|| json!(0));
        let required = config.flow.prewarm_required || command_count.as_i64().unwrap_or(0) > 0;
        return Ok(Some(json!({
            "contract": "ait.server.patchset_ci.native_prewarm.v1",
            "status": status,
            "required": required,
            "once_per_patchset_ci_run": false,
            "once_per_main_seed_generation": true,
            "main_seed_status": main_seed_status,
            "main_seed_path": main_seed_prewarm.get("main_seed_path").cloned().unwrap_or(JsonValue::Null),
            "generation_key": main_seed_prewarm.get("generation_key").cloned().unwrap_or(JsonValue::Null),
            "command_count": command_count,
            "duration_seconds": main_seed_prewarm.get("duration_seconds").cloned().unwrap_or(JsonValue::Null),
            "reports": main_seed_prewarm.get("steps").cloned().unwrap_or_else(|| json!([])),
            "artifacts": {
                "manifest_path": main_seed_prewarm.get("manifest_path").cloned().unwrap_or(JsonValue::Null),
            },
            "failure": main_seed_prewarm.get("failure").cloned().unwrap_or(JsonValue::Null),
            "main_seed_prewarm": main_seed_prewarm,
        })));
    }
    if config.prewarm_commands.is_empty() {
        return Ok(None);
    }
    let mut runner = JsonMap::new();
    runner.insert("kind".to_string(), json!("command_bundle"));
    runner.insert("commands".to_string(), json!([]));
    runner.insert(
        "prewarm_commands".to_string(),
        json!(&config.prewarm_commands),
    );
    let mut payload = command_bundle_base_payload(config, config.output_dir.join("prewarm"));
    payload.insert("prewarm_only".to_string(), json!(true));
    payload.insert("runner".to_string(), JsonValue::Object(runner));
    payload.insert(
        "artifacts".to_string(),
        json!({"summary_json": "prewarm-summary.json", "log_path": "prewarm.log"}),
    );
    let result = ci_command_bundle_run_json(&JsonValue::Object(payload))?;
    Ok(Some(json!({
        "contract": "ait.server.patchset_ci.native_prewarm.v1",
        "status": result["status"].clone(),
        "required": true,
        "once_per_patchset_ci_run": true,
        "command_count": config.prewarm_commands.len(),
        "duration_seconds": result["duration_seconds"].clone(),
        "reports": result["prewarm"]["reports"].clone(),
        "artifacts": result["artifacts"].clone(),
        "failure": result["failure"].clone(),
    })))
}
