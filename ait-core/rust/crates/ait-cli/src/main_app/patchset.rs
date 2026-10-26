fn run_patchset(repo: RepoRuntime, command: PatchsetCommand) -> Result<(), String> {
    match command {
        PatchsetCommand::Publish(args) => {
            let payload = crate::binding_changes::publish_patchset(
                &repo,
                &args.change,
                &args.summary,
                args.author_mode.map(ConfigAuthorModeArg::as_str),
                args.remote.as_deref(),
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_patchset_publish(&payload)?
                );
            }
            Ok(())
        }
        PatchsetCommand::List(args) => {
            let payload = crate::binding_changes::list_patchsets(
                &repo,
                &args.change,
                args.remote.as_deref(),
            )?;
            if args.json {
                print_json(&payload)?;
            } else if payload.is_array() {
                println!("{}", crate::binding_output::format_patchset_list(&payload)?);
            }
            Ok(())
        }
        PatchsetCommand::Show(args) => {
            let payload = patchset_show_cmd(&repo, &args.patchset_id, args.remote.as_deref())?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_patchset_show(&payload)?);
            }
            Ok(())
        }
        PatchsetCommand::Select(args) => {
            let payload = patchset_select_cmd(&repo, &args.patchset_id, args.remote.as_deref())?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_patchset_select(&payload)?
                );
            }
            Ok(())
        }
        PatchsetCommand::CiStatus(args) => {
            let payload = patchset_ci_status_cmd(&repo, &args.patchset_id, args.remote.as_deref())?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_patchset_ci_status(&payload)?
                );
            }
            Ok(())
        }
        PatchsetCommand::RerunCi(args) => {
            let payload = patchset_rerun_ci_cmd(&repo, &args.patchset_id, args.remote.as_deref())?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_patchset_rerun_ci(&payload)?
                );
            }
            Ok(())
        }
    }
}

pub(crate) fn patchset_show_display_payload(payload: &JsonValue) -> JsonValue {
    let Some(obj) = payload.as_object() else {
        return payload.clone();
    };
    let mut display = obj.clone();
    let readable_summary = obj
        .get("summary")
        .and_then(JsonValue::as_str)
        .and_then(|summary| summary.find('{').map(|start| &summary[start..]))
        .and_then(crate::json_support::parse_value_option)
        .and_then(|summary| {
            summary
                .get("display_summary")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        });
    if let Some(readable_summary) = readable_summary {
        display.insert("summary".to_string(), JsonValue::String(readable_summary));
    }
    JsonValue::Object(display)
}

pub(crate) fn patchset_ci_status_display_payload(payload: &JsonValue) -> JsonValue {
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
    if let Some(action) = payload
        .get("recommended_action")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
    {
        display.insert(
            "recommended_action".to_string(),
            JsonValue::String(action.to_string()),
        );
    } else {
        display.remove("recommended_action");
    }
    JsonValue::Object(display)
}

pub(crate) fn patchset_ci_status_display_fields(payload: &JsonValue) -> Vec<&'static str> {
    let mut fields = vec!["patchset_id", "tests_status"];
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
