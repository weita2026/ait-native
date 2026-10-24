use super::*;

pub(super) fn parse_request_map(
    request: JsonMap<String, JsonValue>,
) -> Result<SyncRequest, String> {
    Ok(SyncRequest {
        root_path: require_nonempty_text(request.get("root_path"), "root_path")?,
        repo_name: require_nonempty_text(request.get("repo_name"), "repo_name")?,
        repository_index: optional_u32(request.get("repository_index"))?.map(RepositoryIndex::new),
        id_namespace_prefix: optional_text_allow_empty(request.get("id_namespace_prefix"))?,
        created_by: optional_text(request.get("created_by"))?,
        target: require_nonempty_text(request.get("target"), "target")?,
        plan_ref: optional_text(request.get("plan_ref"))?,
        prune: optional_bool(request.get("prune"), false)?,
        local: optional_bool(request.get("local"), false)?,
        remote_name: optional_text(request.get("remote_name"))?,
        remote_repo_name: optional_text(request.get("remote_repo_name"))?,
        base_url: optional_text(request.get("base_url"))?,
        rebase: optional_bool(request.get("rebase"), false)?,
        reconcile: optional_bool(request.get("reconcile"), false)?,
        history_publish_plan_id: optional_text(request.get("history_publish_plan_id"))?,
        plan_storage: parse_plan_storage_request(request.get("plan_storage"))?,
        task_start: parse_task_start_request(request.get("task_start"))?,
    })
}

pub(super) fn validate_runtime_flags(request: &SyncRequest) -> Result<(), String> {
    if request.local && request.base_url.is_some() {
        return Err("`--local` cannot be combined with `--remote`; omit `--local` to sync locally and publish.".to_string());
    }
    if request.rebase && request.reconcile {
        return Err("Choose either `--rebase` or `--reconcile`, not both.".to_string());
    }
    if request.rebase && request.base_url.is_none() {
        return Err(
            "`--rebase` requires `--remote <name>` because it replaces a divergent local head from shared Plan authority."
                .to_string(),
        );
    }
    if request.reconcile && request.base_url.is_none() && !request.local {
        return Err(
            "`--reconcile` without `--remote <name>` requires explicit `--local`.".to_string(),
        );
    }
    if let Some(plan_id) = request.history_publish_plan_id.as_deref() {
        LocalPlanId::from_raw(plan_id.to_string())?;
        if request.local || request.base_url.is_none() {
            return Err(
                "Internal history Plan publication requires remote publish mode and cannot run with `--local`."
                    .to_string(),
            );
        }
        if request.prune
            || request.rebase
            || request.reconcile
            || request.task_start.is_some()
            || request.plan_ref.is_some()
        {
            return Err(
                "Internal history Plan publication cannot be combined with prune, rebase, reconcile, task_start, or plan_ref."
                    .to_string(),
            );
        }
    }
    if request.task_start.is_some() {
        if request.local || request.base_url.is_none() {
            return Err(
                "Plan sync task_start requires remote publish mode and cannot run with `--local`."
                    .to_string(),
            );
        }
        if request.prune || request.rebase || request.reconcile {
            return Err(
                "Plan sync task_start does not support prune, rebase, or reconcile.".to_string(),
            );
        }
        if request.plan_ref.is_none() {
            return Err(
                "Plan sync task_start requires an exact Plan selector in plan_ref.".to_string(),
            );
        }
    }
    request.plan_storage.require_binary_layout()?;
    Ok(())
}

fn parse_task_start_request(
    value: Option<&JsonValue>,
) -> Result<Option<PlanSyncTaskStartRequest>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let object = value
        .as_object()
        .ok_or_else(|| "task_start must be a JSON object.".to_string())?;
    const FIELDS: &[&str] = &[
        "contract",
        "idempotency_key",
        "plan_item_ref",
        "task",
        "change",
    ];
    if let Some(field) = object
        .keys()
        .find(|field| !FIELDS.contains(&field.as_str()))
    {
        return Err(format!(
            "Plan sync does not support task_start field `{field}`."
        ));
    }
    let contract = require_nonempty_text(object.get("contract"), "task_start.contract")?;
    if contract != "task-start-atomic/v1" {
        return Err("task_start.contract must be `task-start-atomic/v1`.".to_string());
    }
    let idempotency_key =
        require_nonempty_text(object.get("idempotency_key"), "task_start.idempotency_key")?;
    if idempotency_key.len() > 256 {
        return Err("task_start.idempotency_key exceeds 256 bytes.".to_string());
    }
    let plan_item_ref =
        require_nonempty_text(object.get("plan_item_ref"), "task_start.plan_item_ref")?;
    let task = parse_task_start_child(
        object.get("task"),
        "task_start.task",
        &["task_id", "title", "intent"],
        &["title", "intent"],
    )?;
    let change = parse_task_start_child(
        object.get("change"),
        "task_start.change",
        &[
            "change_id",
            "title",
            "base_line",
            "fork_snapshot_id",
            "forked_from_line",
        ],
        &["title", "base_line"],
    )?;
    Ok(Some(PlanSyncTaskStartRequest {
        contract,
        idempotency_key,
        plan_item_ref,
        task,
        change,
    }))
}

fn parse_task_start_child(
    value: Option<&JsonValue>,
    field: &str,
    supported_fields: &[&str],
    required_fields: &[&str],
) -> Result<JsonValue, String> {
    let object = value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("{field} must be a JSON object."))?;
    if let Some(child_field) = object
        .keys()
        .find(|child_field| !supported_fields.contains(&child_field.as_str()))
    {
        return Err(format!(
            "Plan sync does not support {field} field `{child_field}`."
        ));
    }
    for required in required_fields {
        require_nonempty_text(object.get(*required), &format!("{field}.{required}"))?;
    }
    for optional in supported_fields
        .iter()
        .filter(|candidate| !required_fields.contains(candidate))
    {
        if object.get(*optional).is_some_and(|value| !value.is_null()) {
            require_nonempty_text(object.get(*optional), &format!("{field}.{optional}"))?;
        }
    }
    Ok(JsonValue::Object(object.clone()))
}

pub(super) fn parse_plan_storage_request(
    value: Option<&JsonValue>,
) -> Result<PlanSyncStorageRequest, String> {
    let value =
        value.ok_or_else(|| "Plan sync requires a Binary DB `plan_storage` object.".to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "plan_storage must be a JSON object.".to_string())?;
    const FIELDS: &[&str] = &[
        "write_layout",
        "authority_root",
        "activation_pointer",
        "pack_root",
        "repo_root",
        "local_authority_id",
        "current_line_state_scope",
    ];
    if let Some(field) = object
        .keys()
        .find(|field| !FIELDS.contains(&field.as_str()))
    {
        return Err(format!(
            "Plan sync does not support plan_storage field `{field}`."
        ));
    }
    Ok(PlanSyncStorageRequest {
        write_layout: optional_u32(object.get("write_layout"))?,
        authority_root: optional_text(object.get("authority_root"))?,
        activation_pointer: optional_text(object.get("activation_pointer"))?,
        pack_root: optional_text(object.get("pack_root"))?,
        repo_root: optional_text(object.get("repo_root"))?,
        local_authority_id: optional_text(object.get("local_authority_id"))?,
        current_line_state_scope: optional_text(object.get("current_line_state_scope"))?
            .map(|value| parse_local_state_scope(value.as_str()))
            .transpose()?,
    })
}

pub(super) fn parse_local_state_scope(value: &str) -> Result<LocalStateScope, String> {
    match value {
        "repository" => Ok(LocalStateScope::Repository),
        "line" => Ok(LocalStateScope::Line),
        "task" => Ok(LocalStateScope::Task),
        "remote_cache" => Ok(LocalStateScope::RemoteCache),
        other => Err(format!(
            "Unsupported plan_storage.current_line_state_scope `{other}`."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_payload() -> JsonMap<String, JsonValue> {
        json!({
            "root_path": "/repo",
            "repo_name": "repo",
            "target": "docs/sprints/card.md",
            "plan_ref": "card",
            "local": false,
            "remote_name": "origin",
            "remote_repo_name": "repo",
            "base_url": "https://server.test",
            "plan_storage": {"write_layout": 1},
            "task_start": {
                "contract": "task-start-atomic/v1",
                "idempotency_key": "start-1",
                "plan_item_ref": "card/item",
                "task": {"title": "Task", "intent": "Intent"},
                "change": {"title": "Change", "base_line": "main"}
            }
        })
        .as_object()
        .cloned()
        .unwrap()
    }

    #[test]
    fn task_start_request_requires_remote_exact_file_safe_flags() {
        let parsed = parse_request_map(request_payload()).unwrap();
        validate_runtime_flags(&parsed).unwrap();
        assert_eq!(
            parsed
                .task_start
                .as_ref()
                .map(|request| request.plan_item_ref.as_str()),
            Some("card/item")
        );

        for (field, value, expected) in [
            ("local", JsonValue::Bool(true), "cannot be combined"),
            ("prune", JsonValue::Bool(true), "does not support prune"),
            (
                "reconcile",
                JsonValue::Bool(true),
                "does not support prune, rebase, or reconcile",
            ),
            (
                "plan_ref",
                JsonValue::Null,
                "requires an exact Plan selector",
            ),
        ] {
            let mut payload = request_payload();
            payload.insert(field.to_string(), value);
            let parsed = parse_request_map(payload).unwrap();
            assert!(
                validate_runtime_flags(&parsed)
                    .unwrap_err()
                    .contains(expected),
                "{field}"
            );
        }
    }

    #[test]
    fn explicit_local_reconcile_is_admitted_without_weakening_retry_flag_guards() {
        let mut local_reconcile = request_payload();
        local_reconcile["local"] = JsonValue::Bool(true);
        local_reconcile["remote_name"] = JsonValue::Null;
        local_reconcile["remote_repo_name"] = JsonValue::Null;
        local_reconcile["base_url"] = JsonValue::Null;
        local_reconcile.insert("reconcile".to_string(), JsonValue::Bool(true));
        local_reconcile["task_start"] = JsonValue::Null;
        let parsed = parse_request_map(local_reconcile).unwrap();
        validate_runtime_flags(&parsed).expect("explicit local reconcile should be admitted");

        let mut local_rebase = request_payload();
        local_rebase["local"] = JsonValue::Bool(true);
        local_rebase["remote_name"] = JsonValue::Null;
        local_rebase["remote_repo_name"] = JsonValue::Null;
        local_rebase["base_url"] = JsonValue::Null;
        local_rebase.insert("rebase".to_string(), JsonValue::Bool(true));
        local_rebase["task_start"] = JsonValue::Null;
        let parsed = parse_request_map(local_rebase).unwrap();
        assert!(validate_runtime_flags(&parsed)
            .unwrap_err()
            .contains("`--rebase` requires `--remote <name>`"));

        let mut ambiguous_reconcile = request_payload();
        ambiguous_reconcile["remote_name"] = JsonValue::Null;
        ambiguous_reconcile["remote_repo_name"] = JsonValue::Null;
        ambiguous_reconcile["base_url"] = JsonValue::Null;
        ambiguous_reconcile.insert("reconcile".to_string(), JsonValue::Bool(true));
        ambiguous_reconcile["task_start"] = JsonValue::Null;
        let parsed = parse_request_map(ambiguous_reconcile).unwrap();
        assert!(validate_runtime_flags(&parsed)
            .unwrap_err()
            .contains("requires explicit `--local`"));

        let mut mixed_retry_modes = request_payload();
        mixed_retry_modes.insert("rebase".to_string(), JsonValue::Bool(true));
        mixed_retry_modes.insert("reconcile".to_string(), JsonValue::Bool(true));
        mixed_retry_modes["task_start"] = JsonValue::Null;
        let parsed = parse_request_map(mixed_retry_modes).unwrap();
        assert!(validate_runtime_flags(&parsed)
            .unwrap_err()
            .contains("Choose either `--rebase` or `--reconcile`"));
    }

    #[test]
    fn internal_history_plan_publication_is_remote_only_and_mutually_exclusive() {
        let mut payload = request_payload();
        payload["task_start"] = JsonValue::Null;
        payload["plan_ref"] = JsonValue::Null;
        payload.insert(
            "history_publish_plan_id".to_string(),
            JsonValue::String("PR-649".to_string()),
        );
        let parsed = parse_request_map(payload.clone()).unwrap();
        validate_runtime_flags(&parsed).expect("exact history publication should be admitted");
        assert_eq!(parsed.history_publish_plan_id.as_deref(), Some("PR-649"));

        for (field, value, expected) in [
            ("local", JsonValue::Bool(true), "cannot be combined"),
            ("prune", JsonValue::Bool(true), "cannot be combined"),
            ("rebase", JsonValue::Bool(true), "cannot be combined"),
            ("reconcile", JsonValue::Bool(true), "cannot be combined"),
            (
                "plan_ref",
                JsonValue::String("card".to_string()),
                "cannot be combined",
            ),
        ] {
            let mut invalid = payload.clone();
            invalid.insert(field.to_string(), value);
            let parsed = parse_request_map(invalid).unwrap();
            assert!(
                validate_runtime_flags(&parsed)
                    .unwrap_err()
                    .contains(expected),
                "{field}"
            );
        }

        let mut without_remote = payload.clone();
        without_remote["base_url"] = JsonValue::Null;
        without_remote["remote_name"] = JsonValue::Null;
        without_remote["remote_repo_name"] = JsonValue::Null;
        let parsed = parse_request_map(without_remote).unwrap();
        assert!(validate_runtime_flags(&parsed)
            .unwrap_err()
            .contains("remote publish mode"));

        let mut with_task_start = payload;
        with_task_start["task_start"] = request_payload()["task_start"].clone();
        let parsed = parse_request_map(with_task_start).unwrap();
        assert!(validate_runtime_flags(&parsed)
            .unwrap_err()
            .contains("cannot be combined"));
    }

    #[test]
    fn task_start_request_rejects_server_derived_or_unknown_fields() {
        let mut payload = request_payload();
        payload["task_start"]["task"]["plan_id"] = JsonValue::String("PR-1".to_string());
        assert!(parse_request_map(payload)
            .unwrap_err()
            .contains("task_start.task field `plan_id`"));

        let mut payload = request_payload();
        payload["task_start"]["unexpected"] = JsonValue::Bool(true);
        assert!(parse_request_map(payload)
            .unwrap_err()
            .contains("task_start field `unexpected`"));
    }
}
