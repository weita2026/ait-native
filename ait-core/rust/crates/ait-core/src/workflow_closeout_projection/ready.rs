use super::*;

pub(crate) fn workflow_ready_steps(
    facts: &JsonValue,
    command_hints: &JsonValue,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
) -> Vec<JsonValue> {
    let mut steps = Vec::new();
    let workspace = field_obj(facts, "workspace");
    let patchset = optional_obj_field(facts, "patchset");
    let patchset_id = patchset
        .as_ref()
        .and_then(|value| optional_string_field(value, "patchset_id"))
        .unwrap_or_default();
    let attestation = optional_obj_field(facts, "attestation");
    let external_readiness = optional_obj_field(facts, "external_readiness");
    let patchset_refresh = optional_obj_field(facts, "patchset_refresh");
    let publish_command = command_hint(command_hints, "publish_command");
    let patchset_ci_required = command_hint(command_hints, "patchset_ci_command").is_some();
    if ignore_workspace_authoring {
        steps.push(workflow_land_step("snapshot", "Snapshot", "done", "Final-snapshot remote promotion is using the completed local target-line snapshot and does not require repo-root authoring state.", None));
    } else if !bool_field(&workspace, "clean") {
        steps.push(workflow_land_step(
            "snapshot",
            "Snapshot",
            "pending",
            "Workspace changes are still dirty, so ready state needs a fresh snapshot first.",
            Some("ait snapshot create --message \"reviewable snapshot\"".to_string()),
        ));
    } else {
        steps.push(workflow_land_step(
            "snapshot",
            "Snapshot",
            "done",
            &format!(
                "The current line `{}` is already captured at `{}`.",
                optional_string_field(&workspace, "current_line")
                    .unwrap_or_else(|| "unknown".to_string()),
                optional_string_field(&workspace, "head_snapshot_id")
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            None,
        ));
    }
    if patchset.is_none() {
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "pending",
            "No published patchset exists yet for this change.",
            publish_command,
        ));
    } else if patchset_is_authoritative {
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "done",
            &format!(
                "Patchset `{}` is already prepared for the review and land phase.",
                patchset_id
            ),
            None,
        ));
    } else if let Some(refresh) = patchset_refresh {
        let detail = optional_string_field(&refresh, "detail").unwrap_or_else(|| {
            format!(
                "Patchset `{}` needs a refresh from the current line.",
                patchset_id
            )
        });
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "stale",
            &detail,
            publish_command,
        ));
    } else {
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "done",
            &format!(
                "Patchset `{}` is published for `{}`.",
                patchset_id,
                optional_string_field(&field_obj(facts, "change"), "change_id")
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            None,
        ));
    }
    if let Some(readiness) = external_readiness.as_ref() {
        if external_readiness_is_ready(Some(readiness)) {
            steps.push(workflow_land_step(
                "external",
                "External",
                "done",
                "External materialization is ready for CI and remote land.",
                None,
            ));
        } else {
            steps.push(workflow_land_step(
                "external",
                "External",
                "blocked",
                &external_readiness_blocker_detail(Some(readiness)),
                Some("ait external doctor".to_string()),
            ));
            return steps;
        }
    }
    if patchset.is_none() {
        steps.push(workflow_land_step(
            "attestation",
            "Attestation",
            "waiting",
            "Attestation starts after a patchset exists.",
            None,
        ));
    } else if attestation.is_none() {
        let detail = if patchset_ci_required {
            format!("No attestation is recorded yet for patchset `{patchset_id}`. Workflow ready apply owns Patchset CI completion and then records the compact Attestation gate statement.")
        } else {
            format!("No attestation is recorded yet for patchset `{patchset_id}`.")
        };
        steps.push(workflow_land_step(
            "attestation",
            "Attestation",
            "pending",
            &detail,
            if patchset_ci_required {
                command_hint(command_hints, "apply_command")
            } else {
                command_hint(command_hints, "attestation_command")
                    .or_else(|| command_hint(command_hints, "attest_command"))
            },
        ));
    } else if !string_field(facts, "tests_state").is_empty()
        && !matches!(
            string_field(facts, "tests_state").as_str(),
            "pass" | "not_required"
        )
    {
        let tests_state = string_field(facts, "tests_state");
        let detail = if command_hint(command_hints, "patchset_ci_command").is_some() {
            format!(
                "Patchset CI currently reports tests `{tests_state}` for patchset `{patchset_id}`."
            )
        } else {
            format!("Tests are currently `{tests_state}` for patchset `{patchset_id}`.")
        };
        steps.push(workflow_land_step(
            "attestation",
            "Attestation",
            if tests_state == "pending" {
                "pending"
            } else {
                "blocked"
            },
            &detail,
            command_hint(command_hints, "attestation_command")
                .or_else(|| command_hint(command_hints, "attest_command")),
        ));
    } else {
        let attestation_id = optional_string_field(attestation.as_ref().unwrap(), "attestation_id")
            .unwrap_or_default();
        let tests_state = string_field(facts, "tests_state");
        let suffix = if tests_state.is_empty() {
            ".".to_string()
        } else {
            format!(" with tests `{tests_state}`.")
        };
        steps.push(workflow_land_step(
            "attestation",
            "Attestation",
            "done",
            &format!("Attestation `{attestation_id}` is recorded{suffix}"),
            None,
        ));
    }
    steps
}

pub(crate) fn workflow_ready_suggested_commands(
    commands: &JsonValue,
    next_action: &JsonValue,
    apply_owned_continuation: bool,
) -> Vec<JsonValue> {
    let next_action_code = optional_string_field(next_action, "code").unwrap_or_default();
    let candidates = if next_action_code == "external_readiness_blocked" {
        vec![optional_string_field(next_action, "command")]
    } else if apply_owned_continuation
        && WORKFLOW_READY_APPLY_OWNED_CODES.contains(&next_action_code.as_str())
    {
        vec![
            command_hint(commands, "apply_command"),
            optional_string_field(next_action, "command"),
        ]
    } else {
        vec![
            optional_string_field(next_action, "command"),
            command_hint(commands, "publish_command"),
            command_hint(commands, "attestation_command")
                .or_else(|| command_hint(commands, "attest_command")),
            if next_action_code == "done" {
                command_hint(commands, "land_command")
            } else {
                None
            },
        ]
    };
    unique_command_values(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_facts(attestation: JsonValue) -> JsonValue {
        json!({
            "change": {"change_id": "RCC-1"},
            "workspace": {
                "clean": true,
                "current_line": "feature/rct-1",
                "head_snapshot_id": "SNP-1"
            },
            "patchset": {"patchset_id": "RCP-1"},
            "attestation": attestation,
            "tests_state": "pass"
        })
    }

    fn command_hints() -> JsonValue {
        json!({
            "apply_command": "ait workflow ready RCC-1 --apply",
            "patchset_ci_command": "ait patchset rerun-ci RCP-1",
            "attestation_command": "ait patchset rerun-ci RCP-1",
        })
    }

    fn attestation_step(steps: &[JsonValue]) -> &JsonValue {
        steps
            .iter()
            .find(|step| step["code"] == json!("attestation"))
            .expect("attestation step")
    }

    #[test]
    fn ready_steps_mark_compact_attestation_done_for_ci_contract() {
        let facts = ready_facts(json!({
            "attestation_id": "AT-1",
            "evaluation_summary": {"tests": "pass"}
        }));

        let steps = workflow_ready_steps(&facts, &command_hints(), false, false);
        let attestation = attestation_step(&steps);

        assert_eq!(attestation["status"], json!("done"));
        assert!(attestation["command"].is_null());
    }

    #[test]
    fn ready_steps_mark_missing_compact_attestation_pending() {
        let facts = ready_facts(JsonValue::Null);

        let steps = workflow_ready_steps(&facts, &command_hints(), false, false);
        let attestation = attestation_step(&steps);

        assert_eq!(attestation["status"], json!("pending"));
        assert_eq!(
            attestation["command"],
            json!("ait workflow ready RCC-1 --apply")
        );
        assert!(attestation["detail"]
            .as_str()
            .unwrap()
            .contains("compact Attestation gate statement"));
    }
}
