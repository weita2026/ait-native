use super::*;

impl PostgresPolicyStore {
    pub(super) fn refresh_change_state(
        &mut self,
        change_id: &str,
        now: &str,
    ) -> Result<String, String> {
        let changes = self.control_table("changes");
        let change = self
            .client
            .query_opt(
                &format!("select change_id, repo_name, repo_id, status, base_line, current_patchset_number::bigint as current_patchset_number from {changes} where change_id = $1"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Unknown change: {change_id}"))?;
        let existing_state = row_text(&change, "status").unwrap_or_default();
        if existing_state == "landed" || existing_state == "archived" {
            return Ok(existing_state);
        }
        let current_patchset_number = row_i64(&change, "current_patchset_number").unwrap_or(0);
        let new_state = if current_patchset_number == 0 {
            "draft".to_string()
        } else {
            let patchset = self.current_patchset_for_change(change_id)?;
            let latest = self.latest_policy_status(
                required_text(patchset.get("patchset_id"), "patchset.patchset_id")?.as_str(),
            )?;
            let policy = json_object(
                effective_policy_status(&patchset, latest.as_ref())?.as_object(),
                "effective policy status",
            )?;
            let review = self.review_summary(
                change_id,
                required_text(patchset.get("patchset_id"), "patchset.patchset_id")?.as_str(),
            )?;
            let repo_name = row_text(&change, "repo_name").unwrap_or_default();
            let base_line = row_text(&change, "base_line").unwrap_or_default();
            let base_line_head = self.content_line_head(&repo_name, &base_line)?;
            let base_snapshot_id =
                optional_text(patchset.get("base_snapshot_id")).unwrap_or_default();
            let stale = base_line_head
                .as_deref()
                .is_some_and(|head| !head.is_empty() && head != base_snapshot_id);
            let blocking_count = review
                .get("blocking_count")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0);
            let approval_count = review
                .get("approval_count")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0);
            let decision =
                optional_text(policy.get("decision")).unwrap_or_else(|| "pending".to_string());
            if blocking_count > 0 || decision == "hard_fail" || stale {
                "blocked".to_string()
            } else if decision == "pass" && approval_count >= REQUIRED_APPROVALS && !stale {
                "landable".to_string()
            } else if approval_count >= REQUIRED_APPROVALS && decision == "pass" {
                "approved".to_string()
            } else if decision == "pending" || decision == "soft_fail" {
                "gated".to_string()
            } else {
                "review".to_string()
            }
        };
        if new_state != existing_state {
            self.client
                .execute(
                    &format!("update {changes} set status = $1, updated_at = $2::text::timestamptz where change_id = $3"),
                    &[&new_state, &now, &change_id],
                )
                .map_err(|exc| exc.to_string())?;
        }
        Ok(new_state)
    }
}

pub(super) fn enrich_status_with_policy_context(
    status: &mut JsonMap<String, JsonValue>,
    repo_policy: &JsonMap<String, JsonValue>,
    policy_context: &JsonMap<String, JsonValue>,
) {
    let normalized = normalize_policy(repo_policy);
    status.insert(
        "policy_id".to_string(),
        normalized
            .get("policy_id")
            .cloned()
            .unwrap_or_else(|| json!("prototype")),
    );
    for key in [
        "content_class",
        "author_class",
        "effective_requirements",
        "matched_overrides",
    ] {
        status.insert(
            key.to_string(),
            policy_context.get(key).cloned().unwrap_or(JsonValue::Null),
        );
    }
}

pub(super) fn ensure_effective_requirements(
    policy_context: &mut JsonMap<String, JsonValue>,
) -> &mut JsonMap<String, JsonValue> {
    let needs_insert = !policy_context
        .get("effective_requirements")
        .is_some_and(JsonValue::is_object);
    if needs_insert {
        policy_context.insert("effective_requirements".to_string(), json!({}));
    }
    policy_context
        .get_mut("effective_requirements")
        .and_then(JsonValue::as_object_mut)
        .expect("effective_requirements object should exist")
}
