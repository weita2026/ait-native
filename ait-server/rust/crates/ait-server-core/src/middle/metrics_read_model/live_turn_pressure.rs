use super::helpers::*;
use super::*;

pub fn normalize_live_turn_metrics(metrics: &JsonValue) -> Result<JsonValue, String> {
    let metrics_obj = metrics.as_object().cloned().unwrap_or_default();
    let summary = object_field(&metrics_obj, "summary");
    let active_repositories = active_repository_counts(&metrics_obj, &summary);
    let repo_activity = active_repositories
        .iter()
        .map(|(repo_name, count)| json!({"repo_name": repo_name, "active_turns": count}))
        .collect::<Vec<_>>();
    let recent_completed_turns = object_array(&metrics_obj, "recent_completed_turns");
    let recent_failed_turns = object_array(&metrics_obj, "recent_failed_turns");
    let active_turns = first_present_i64(
        &[&metrics_obj, &summary],
        &["active_turns", "active_turn_count"],
    )
    .unwrap_or(0);
    let recent_completed_turn_count =
        first_present_i64(&[&metrics_obj, &summary], &["recent_completed_turn_count"])
            .unwrap_or(recent_completed_turns.len() as i64);
    let recent_failed_turn_count =
        first_present_i64(&[&metrics_obj, &summary], &["recent_failed_turn_count"])
            .unwrap_or(recent_failed_turns.len() as i64);
    let oldest_active_turn_age_seconds = first_present_f64(
        &[&metrics_obj, &summary],
        &["oldest_active_turn_age_seconds"],
    )
    .map(|value| round_f64(value, 3));
    let recent_completed_p95_seconds =
        first_present_f64(&[&metrics_obj, &summary], &["recent_completed_p95_seconds"])
            .map(|value| round_f64(value, 3));
    let oldest_active_turn_started_at = first_present_value(
        &[&metrics_obj, &summary],
        &[
            "oldest_active_turn_started_at",
            "oldest_active_turn_started_at_epoch_seconds",
        ],
    );
    Ok(json!({
        "summary": {
            "active_turns": active_turns,
            "active_repositories": active_repositories.len(),
            "oldest_active_turn_started_at": oldest_active_turn_started_at.clone().unwrap_or(JsonValue::Null),
            "oldest_active_turn_age_seconds": oldest_active_turn_age_seconds,
            "recent_completed_turns": recent_completed_turn_count,
            "recent_failed_turns": recent_failed_turn_count,
            "recent_completed_p95_seconds": recent_completed_p95_seconds,
        },
        "repo_activity": repo_activity,
        "active_turns": active_turns,
        "active_repositories": active_repositories,
        "oldest_active_turn_started_at": oldest_active_turn_started_at.unwrap_or(JsonValue::Null),
        "oldest_active_turn_age_seconds": oldest_active_turn_age_seconds,
        "recent_completed_turns": recent_completed_turns,
        "recent_failed_turns": recent_failed_turns,
        "recent_completed_p95_seconds": recent_completed_p95_seconds,
        "recent_completed_turn_count": recent_completed_turn_count,
        "recent_failed_turn_count": recent_failed_turn_count,
        "snapshot_at_epoch_seconds": first_present_value(&[&metrics_obj, &summary], &["snapshot_at_epoch_seconds"]).unwrap_or(JsonValue::Null),
    }))
}

pub fn live_turn_pressure_summary_from_normalized(normalized: &JsonValue) -> JsonValue {
    let summary = object_value(normalized, "summary");
    let in_flight_turns = int_field(&summary, "active_turns");
    let queued_turns = 0;
    let oldest_age = optional_f64_field(&summary, "oldest_active_turn_age_seconds")
        .map(|value| round_f64(value, 3));
    let pressure_state = if in_flight_turns <= 0 && queued_turns <= 0 {
        "idle"
    } else if queued_turns > 0 || in_flight_turns >= 4 || oldest_age.unwrap_or(0.0) >= 300.0 {
        "saturated"
    } else if in_flight_turns >= 2 || oldest_age.unwrap_or(0.0) >= 120.0 {
        "busy"
    } else {
        "ok"
    };
    json!({
        "pressure_state": pressure_state,
        "in_flight_turns": in_flight_turns,
        "queued_turns": queued_turns,
        "active_repositories": int_field(&summary, "active_repositories"),
        "active_repositories_by_name": normalized.get("active_repositories").cloned().unwrap_or_else(|| json!({})),
        "oldest_in_flight_turn_started_at": summary.get("oldest_active_turn_started_at").cloned().unwrap_or(JsonValue::Null),
        "oldest_in_flight_turn_age_seconds": oldest_age,
        "oldest_queued_turn_age_seconds": JsonValue::Null,
        "recent_completed_turns": int_field(&summary, "recent_completed_turns"),
        "recent_failed_turns": int_field(&summary, "recent_failed_turns"),
        "recent_completed_p95_seconds": summary.get("recent_completed_p95_seconds").cloned().unwrap_or(JsonValue::Null),
    })
}

fn active_repository_counts(
    metrics: &JsonMap<String, JsonValue>,
    summary: &JsonMap<String, JsonValue>,
) -> BTreeMap<String, i64> {
    for obj in [metrics, summary] {
        for field in ["active_repositories", "active_turns_by_repo"] {
            if let Some(map) = obj.get(field).and_then(JsonValue::as_object) {
                return map
                    .iter()
                    .filter_map(|(repo_name, count)| {
                        let count = int_value(count).unwrap_or(0);
                        (!repo_name.trim().is_empty() && count > 0)
                            .then(|| (repo_name.clone(), count))
                    })
                    .collect();
            }
        }
    }
    object_array(metrics, "repo_activity")
        .into_iter()
        .filter_map(|row| {
            let repo_name = object_text(&row, "repo_name")?;
            let active_turns = int_field(&row, "active_turns");
            (active_turns > 0).then_some((repo_name, active_turns))
        })
        .collect()
}
