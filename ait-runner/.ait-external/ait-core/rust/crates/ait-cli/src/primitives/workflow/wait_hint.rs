use super::*;

fn workflow_wait_hint_cache_key(kind: &str) -> Option<&'static str> {
    match kind {
        "ready" => Some(WORKFLOW_READY_POLL_SECONDS_KEY),
        "land" => Some(WORKFLOW_LAND_POLL_SECONDS_KEY),
        _ => None,
    }
}

fn workflow_wait_hint_kind_for_pending_code(code: &str) -> Option<&'static str> {
    match code {
        "waiting_for_ci" => Some("ready"),
        "waiting_for_land" => Some("land"),
        _ => None,
    }
}

fn workflow_bounded_wait_hint_seconds(value: f64) -> i64 {
    value.round().clamp(
        WORKFLOW_WAIT_HINT_MIN_SECONDS as f64,
        WORKFLOW_WAIT_HINT_MAX_SECONDS as f64,
    ) as i64
}

pub(in crate::primitives) fn workflow_coerce_wait_hint_seconds(
    value: Option<&JsonValue>,
) -> Option<i64> {
    let numeric = value.and_then(JsonValue::as_f64)?;
    if !numeric.is_finite() || numeric <= 0.0 {
        return None;
    }
    Some(workflow_bounded_wait_hint_seconds(numeric))
}

fn workflow_parse_wait_hint_datetime(value: Option<&JsonValue>) -> Option<DateTime<FixedOffset>> {
    if let Some(seconds) = value.and_then(JsonValue::as_i64) {
        return DateTime::<Utc>::from_timestamp(seconds, 0).map(|value| value.fixed_offset());
    }
    let text = workflow_json_text(value)?;
    DateTime::parse_from_rfc3339(&text.replace('Z', "+00:00")).ok()
}

fn workflow_wait_hint_duration_seconds(
    start_value: Option<&JsonValue>,
    end_value: Option<&JsonValue>,
) -> Option<i64> {
    let start = workflow_parse_wait_hint_datetime(start_value)?;
    let end = workflow_parse_wait_hint_datetime(end_value)?;
    let seconds = (end - start).num_seconds();
    if seconds <= 0 {
        return None;
    }
    Some(workflow_bounded_wait_hint_seconds(seconds as f64))
}

fn workflow_history_wait_hint_sample(detail: &JsonValue, kind: &str) -> Option<i64> {
    let patchset = detail
        .get("selected_patchset")
        .and_then(JsonValue::as_object)
        .or_else(|| {
            detail
                .get("current_patchset")
                .and_then(JsonValue::as_object)
        });
    let patchset_ci_status = detail
        .get("patchset_ci_status")
        .and_then(JsonValue::as_object);
    let change = detail.get("change").and_then(JsonValue::as_object);
    match kind {
        "ready" => workflow_wait_hint_duration_seconds(
            patchset.and_then(|value| value.get("created_at")),
            patchset_ci_status.and_then(|value| value.get("ci_completed_at_s")),
        ),
        "land" => workflow_wait_hint_duration_seconds(
            patchset_ci_status.and_then(|value| value.get("ci_completed_at_s")),
            change.and_then(|value| value.get("landed_at")),
        ),
        _ => None,
    }
}

fn workflow_load_cached_wait_hint_seconds(repo: &RepoRuntime, kind: &str) -> (Option<i64>, bool) {
    let Some(key) = workflow_wait_hint_cache_key(kind) else {
        return (None, false);
    };
    if !repo.config.contains_key(key) {
        return (None, false);
    }
    (
        workflow_coerce_wait_hint_seconds(repo.config.get(key)),
        true,
    )
}

fn workflow_write_wait_hint_seconds(
    repo: &RepoRuntime,
    kind: &str,
    seconds: Option<i64>,
    mark_bootstrap_attempt: bool,
) -> Result<Option<i64>, String> {
    let Some(key) = workflow_wait_hint_cache_key(kind) else {
        return Ok(None);
    };
    let normalized = seconds.map(|seconds| workflow_bounded_wait_hint_seconds(seconds as f64));
    update_root_config(repo, |config| {
        if let Some(seconds) = normalized {
            config.insert(key.to_string(), JsonValue::Number(seconds.into()));
        } else if mark_bootstrap_attempt {
            config.insert(
                key.to_string(),
                JsonValue::Number(WORKFLOW_WAIT_HINT_BOOTSTRAP_MISS.into()),
            );
        } else {
            config.remove(key);
        }
    })?;
    Ok(normalized)
}

fn workflow_update_wait_hint_ema(
    repo: &RepoRuntime,
    kind: &str,
    sample_seconds: i64,
) -> Result<i64, String> {
    let Some(key) = workflow_wait_hint_cache_key(kind) else {
        return Err(format!("Unsupported workflow wait-hint kind: {kind}"));
    };
    let sample = workflow_bounded_wait_hint_seconds(sample_seconds as f64);
    let config_path = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("config.json");
    let config = read_json_object_value(&config_path);
    let previous = workflow_coerce_wait_hint_seconds(config.get(key));
    let updated = match previous {
        Some(previous) => workflow_bounded_wait_hint_seconds(
            (previous as f64 * (1.0 - WORKFLOW_WAIT_HINT_ALPHA))
                + (sample as f64 * WORKFLOW_WAIT_HINT_ALPHA),
        ),
        None => sample,
    };
    update_root_config(repo, |config| {
        config.insert(key.to_string(), JsonValue::Number(updated.into()));
    })?;
    Ok(updated)
}

fn workflow_bootstrap_wait_hint_seconds_from_history(
    repo: &RepoRuntime,
    kind: &str,
    remote_name: Option<&str>,
) -> Result<Option<i64>, String> {
    let Ok((remote_row, repo_name)) = remote_context(repo, remote_name, None) else {
        return Ok(None);
    };
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    workflow_bootstrap_wait_hint_seconds_from_history_with_task_remote(
        &mut task_remote,
        &repo_name,
        kind,
    )
}

pub(in crate::primitives) fn workflow_bootstrap_wait_hint_seconds_from_history_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    kind: &str,
) -> Result<Option<i64>, String>
where
    R: TaskWorkflowRemoteChangeLister + TaskWorkflowRemoteChangeDetailReader + ?Sized,
{
    let Ok(change_rows) = workflow_wait_hint_change_rows_with_task_remote(task_remote, repo_name)
    else {
        return Ok(None);
    };
    let mut candidates = Vec::new();
    for row in change_rows {
        if string_field(&row, "status").as_deref() != Some("landed") {
            continue;
        }
        let current_patchset_number = row
            .get("current_patchset_number")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let selected_patchset_number = row
            .get("selected_patchset_number")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        if current_patchset_number != 1 || selected_patchset_number != 1 {
            continue;
        }
        let Some(landed_at) = workflow_parse_wait_hint_datetime(row.get("landed_at")) else {
            continue;
        };
        let Some(change_id) = string_field(&row, "change_id") else {
            continue;
        };
        candidates.push((landed_at, change_id));
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    candidates.sort_by_key(|left| left.0);
    let mut samples = Vec::new();
    let start_index = candidates
        .len()
        .saturating_sub(WORKFLOW_WAIT_HINT_HISTORY_LIMIT);
    for (_, change_id) in &candidates[start_index..] {
        let Ok(detail) =
            workflow_wait_hint_change_detail_with_task_remote(task_remote, repo_name, change_id)
        else {
            continue;
        };
        if let Some(sample) = workflow_history_wait_hint_sample(&detail, kind) {
            samples.push(sample);
        }
    }
    if samples.is_empty() {
        return Ok(None);
    }
    let sample_start = samples
        .len()
        .saturating_sub(WORKFLOW_WAIT_HINT_SAMPLE_LIMIT);
    let window = &samples[sample_start..];
    let mut ema = window[0] as f64;
    for sample in &window[1..] {
        ema =
            (ema * (1.0 - WORKFLOW_WAIT_HINT_ALPHA)) + (*sample as f64 * WORKFLOW_WAIT_HINT_ALPHA);
    }
    Ok(Some(workflow_bounded_wait_hint_seconds(ema)))
}

pub(in crate::primitives) fn workflow_wait_hint_change_rows_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
) -> Result<Vec<JsonValue>, String>
where
    R: TaskWorkflowRemoteChangeLister + ?Sized,
{
    task_remote
        .list_changes(repo_name)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_wait_hint_change_detail_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    change_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeDetailReader + ?Sized,
{
    task_remote
        .get_change_detail(change_id, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_resolve_wait_hint_seconds(
    repo: &RepoRuntime,
    kind: &str,
    state: &JsonValue,
) -> Result<Option<i64>, String> {
    let (cached, cache_present) = workflow_load_cached_wait_hint_seconds(repo, kind);
    if cached.is_some() {
        return Ok(cached);
    }
    if cache_present {
        return Ok(None);
    }
    let remote_name = state
        .get("resolved_remote_name")
        .and_then(JsonValue::as_str);
    let seeded = workflow_bootstrap_wait_hint_seconds_from_history(repo, kind, remote_name)?;
    workflow_write_wait_hint_seconds(repo, kind, seeded, true)
}

pub(in crate::primitives) fn workflow_wait_seconds_hint(
    repo: &RepoRuntime,
    code: &str,
    state: &JsonValue,
) -> Result<f64, String> {
    let Some(kind) = workflow_wait_hint_kind_for_pending_code(code) else {
        return Ok(WORKFLOW_APPLY_FOREGROUND_WAIT_MAX_SECONDS);
    };
    Ok(workflow_resolve_wait_hint_seconds(repo, kind, state)?
        .map(|seconds| seconds as f64)
        .unwrap_or(WORKFLOW_APPLY_FOREGROUND_WAIT_MAX_SECONDS))
}

pub(in crate::primitives) fn workflow_maybe_record_ready_wait_hint_sample(
    repo: &RepoRuntime,
    final_state: &JsonValue,
    applied_actions: &[JsonValue],
    helper_elapsed_seconds: f64,
) -> Result<Option<i64>, String> {
    let next_action_done = final_state
        .get("next_action")
        .and_then(JsonValue::as_object)
        .and_then(|next_action| workflow_json_text(next_action.get("code")))
        .as_deref()
        == Some("done");
    let action_codes = applied_actions
        .iter()
        .filter_map(|row| string_field(row, "code"))
        .collect::<Vec<_>>();
    if !next_action_done
        || action_codes
            != [
                "publish_patchset".to_string(),
                "run_patchset_ci".to_string(),
            ]
        || !helper_elapsed_seconds.is_finite()
        || helper_elapsed_seconds <= 0.0
    {
        return Ok(None);
    }
    let sample = workflow_bounded_wait_hint_seconds(helper_elapsed_seconds);
    workflow_update_wait_hint_ema(repo, "ready", sample).map(Some)
}
