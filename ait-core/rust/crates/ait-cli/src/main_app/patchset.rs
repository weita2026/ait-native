fn run_patchset(repo: RepoRuntime, command: PatchsetCommand) -> Result<(), String> {
    match command {
        PatchsetCommand::Publish(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli patchset publish", || {
                patchset_publish(
                    &repo,
                    &args.change,
                    &args.summary,
                    args.author_mode.map(ConfigAuthorModeArg::as_str),
                    args.remote.as_deref(),
                )
            })?;
            emit_result(
                "ait-cli patchset publish",
                &payload,
                args.json,
                &[
                    "change_id",
                    "current_line",
                    "base_snapshot_id",
                    "revision_snapshot_id",
                    "author_mode",
                ],
            )?;
            Ok(())
        }
        PatchsetCommand::List(args) => {
            let payload = patchset_list_cmd(&repo, &args.change, args.remote.as_deref())?;
            if args.json {
                print_json(&payload)?;
            } else if let Some(rows) = payload.as_array() {
                print_list(
                    rows,
                    &[
                        "patchset_id",
                        "patchset_number",
                        "base_snapshot_id",
                        "revision_snapshot_id",
                        "publish_state",
                        "evaluation_state",
                    ],
                );
            }
            Ok(())
        }
        PatchsetCommand::Show(args) => {
            let payload =
                patchset_show_cmd(&repo, &args.patchset_id, args.remote.as_deref())?;
            emit_result(
                "ait-cli patchset show",
                &payload,
                args.json,
                &[
                    "patchset_id",
                    "patchset_number",
                    "change_id",
                    "author_mode",
                    "base_snapshot_id",
                    "revision_snapshot_id",
                    "evaluation_state",
                    "publish_state",
                    "summary",
                ],
            )?;
            Ok(())
        }
        PatchsetCommand::Select(args) => {
            let payload =
                patchset_select_cmd(&repo, &args.patchset_id, args.remote.as_deref())?;
            emit_result(
                "ait-cli patchset select",
                &payload,
                args.json,
                &["change_id", "selected_patchset_id"],
            )?;
            Ok(())
        }
        PatchsetCommand::CiStatus(args) => {
            let payload =
                patchset_ci_status_cmd(&repo, &args.patchset_id, args.remote.as_deref())?;
            if args.json {
                print_json(&payload)?;
            } else {
                let display_payload = patchset_ci_status_display_payload(&payload);
                let fields = patchset_ci_status_display_fields(&display_payload);
                emit_result(
                    "ait-cli patchset ci-status",
                    &display_payload,
                    false,
                    &fields,
                )?;
            }
            Ok(())
        }
        PatchsetCommand::RerunCi(args) => {
            let payload =
                patchset_rerun_ci_cmd(&repo, &args.patchset_id, args.remote.as_deref())?;
            emit_result(
                "ait-cli patchset rerun-ci",
                &payload,
                args.json,
                &["patchset_id", "queued", "trigger"],
            )?;
            Ok(())
        }
    }
}

fn patchset_ci_status_display_payload(payload: &JsonValue) -> JsonValue {
    let Some(obj) = payload.as_object() else {
        return payload.clone();
    };
    let mut display = obj.clone();
    if let Some(message) = payload
        .get("status_notice")
        .and_then(|notice| notice.get("message"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        display.insert(
            "status_message".to_string(),
            JsonValue::String(message.to_string()),
        );
    }
    JsonValue::Object(display)
}

fn patchset_ci_status_display_fields(payload: &JsonValue) -> Vec<&'static str> {
    let mut fields = vec!["patchset_id", "change_id", "tests_status"];
    for optional_field in ["recommended_action", "status_message"] {
        if patchset_ci_status_has_display_field(payload, optional_field) {
            fields.push(optional_field);
        }
    }
    fields
}

fn patchset_ci_status_has_display_field(payload: &JsonValue, field: &str) -> bool {
    payload
        .get(field)
        .map(|value| string_field(Some(value)))
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}
