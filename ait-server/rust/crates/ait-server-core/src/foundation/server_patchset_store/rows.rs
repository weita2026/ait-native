use super::*;

pub(super) fn patchset_row_json(row: &Row) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = JsonMap::new();
    insert_text(&mut out, "patchset_id", row_text(row, "patchset_id"));
    insert_text(&mut out, "repo_id", row_text(row, "repo_id"));
    insert_text(&mut out, "change_id", row_text(row, "change_id"));
    insert_i64(&mut out, "patchset_number", row_i64(row, "patchset_number"));
    insert_text(
        &mut out,
        "base_snapshot_id",
        row_text(row, "base_snapshot_id"),
    );
    insert_text(
        &mut out,
        "revision_snapshot_id",
        row_text(row, "revision_snapshot_id"),
    );
    insert_text(&mut out, "summary", row_text(row, "summary"));
    insert_text(&mut out, "author_mode", row_text(row, "author_mode"));
    insert_text(&mut out, "publish_state", row_text(row, "publish_state"));
    let diff_stats_json = row_text(row, "diff_stats_json").unwrap_or_else(|| "{}".to_string());
    out.insert("diff_stats_json".to_string(), json!(diff_stats_json));
    out.insert(
        "diff_stats".to_string(),
        serde_json::from_str::<JsonValue>(&diff_stats_json).unwrap_or_else(|_| json!({})),
    );
    insert_text(
        &mut out,
        "evaluation_state",
        row_text(row, "evaluation_state"),
    );
    insert_text(&mut out, "created_at", row_text(row, "created_at"));
    Ok(out)
}

pub(super) fn change_row_json(row: &Row) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = JsonMap::new();
    insert_text(&mut out, "change_id", row_text(row, "change_id"));
    insert_text(&mut out, "repo_name", row_text(row, "repo_name"));
    insert_text(&mut out, "repo_id", row_text(row, "repo_id"));
    insert_text(&mut out, "task_id", row_text(row, "task_id"));
    insert_text(&mut out, "status", row_text(row, "status"));
    insert_text(&mut out, "base_line", row_text(row, "base_line"));
    insert_i64(
        &mut out,
        "current_patchset_number",
        row_i64(row, "current_patchset_number"),
    );
    insert_i64(
        &mut out,
        "selected_patchset_number",
        row_i64(row, "selected_patchset_number"),
    );
    insert_text(&mut out, "updated_at", row_text(row, "updated_at"));
    Ok(out)
}

pub(super) fn attestation_row_json(row: &Row) -> Result<JsonMap<String, JsonValue>, String> {
    let evaluation = row_text(row, "evaluation_summary_json").unwrap_or_else(|| "{}".to_string());
    let provenance = row_text(row, "provenance_summary_json").unwrap_or_else(|| "{}".to_string());
    let detail = row_text(row, "detail_json").unwrap_or_else(|| "{}".to_string());
    let mut out = JsonMap::new();
    insert_text(&mut out, "attestation_id", row_text(row, "attestation_id"));
    insert_text(&mut out, "repo_id", row_text(row, "repo_id"));
    insert_text(&mut out, "patchset_id", row_text(row, "patchset_id"));
    insert_text(&mut out, "author_mode", row_text(row, "author_mode"));
    out.insert(
        "evaluation_summary".to_string(),
        serde_json::from_str::<JsonValue>(&evaluation).unwrap_or_else(|_| json!({})),
    );
    out.insert(
        "provenance_summary".to_string(),
        serde_json::from_str::<JsonValue>(&provenance).unwrap_or_else(|_| json!({})),
    );
    out.insert(
        "detail".to_string(),
        serde_json::from_str::<JsonValue>(&detail).unwrap_or_else(|_| json!({})),
    );
    insert_text(&mut out, "created_at", row_text(row, "created_at"));
    insert_text(&mut out, "updated_at", row_text(row, "updated_at"));
    Ok(out)
}

pub(super) fn review_row_json(row: &Row) -> JsonMap<String, JsonValue> {
    let mut out = JsonMap::new();
    insert_i64(&mut out, "review_id", row_i64(row, "review_id"));
    insert_text(&mut out, "reviewer", row_text(row, "reviewer"));
    insert_text(&mut out, "action", row_text(row, "action"));
    out.insert(
        "blocking".to_string(),
        row_bool(row, "blocking").map_or(JsonValue::Null, JsonValue::Bool),
    );
    insert_text(&mut out, "comment", row_text(row, "comment"));
    insert_text(&mut out, "created_at", row_text(row, "created_at"));
    insert_text(&mut out, "patchset_id", row_text(row, "patchset_id"));
    out
}
