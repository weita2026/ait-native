use super::*;

impl PostgresPatchsetStore {
    pub(super) fn change_row(
        &mut self,
        change_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let changes = self.control_table("changes");
        let row = self
            .client
            .query_opt(
                &format!("select change_id, repo_name, repo_id, task_id, status, base_line, current_patchset_number::bigint as current_patchset_number, selected_patchset_number::bigint as selected_patchset_number, updated_at::text as updated_at from {changes} where change_id = $1"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Unknown change: {change_id}"))?;
        let mut out = JsonMap::new();
        insert_text(&mut out, "change_id", row_text(&row, "change_id"));
        insert_text(&mut out, "repo_name", row_text(&row, "repo_name"));
        insert_text(&mut out, "repo_id", row_text(&row, "repo_id"));
        insert_text(&mut out, "task_id", row_text(&row, "task_id"));
        insert_text(&mut out, "status", row_text(&row, "status"));
        insert_text(&mut out, "base_line", row_text(&row, "base_line"));
        insert_i64(
            &mut out,
            "current_patchset_number",
            row_i64(&row, "current_patchset_number"),
        );
        insert_i64(
            &mut out,
            "selected_patchset_number",
            row_i64(&row, "selected_patchset_number"),
        );
        insert_text(&mut out, "updated_at", row_text(&row, "updated_at"));
        Ok(out)
    }
    pub(super) fn get_change_for_repo(
        &mut self,
        repo_name: &str,
        change_ref: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let repo_id = self
            .repo_id_for_repo(repo_name)?
            .ok_or_else(|| format!("Unknown repository: {repo_name}"))?;
        let changes = self.control_table("changes");
        if let Some(row) = self
            .client
            .query_opt(
                &format!("select change_id, repo_name, repo_id, task_id, status, base_line, current_patchset_number::bigint as current_patchset_number, selected_patchset_number::bigint as selected_patchset_number, updated_at::text as updated_at from {changes} where repo_id = $1 and change_id = $2"),
                &[&repo_id, &change_ref],
            )
            .map_err(|exc| exc.to_string())?
        {
            return change_row_json(&row);
        }
        if let Some(number) = repo_scoped_sequence_ref(change_ref) {
            if let Some(row) = self
                .client
                .query_opt(
                    &format!("select change_id, repo_name, repo_id, task_id, status, base_line, current_patchset_number::bigint as current_patchset_number, selected_patchset_number::bigint as selected_patchset_number, updated_at::text as updated_at from {changes} where repo_id = $1 and change_seq = $2"),
                    &[&repo_id, &(number as i32)],
                )
                .map_err(|exc| exc.to_string())?
            {
                return change_row_json(&row);
            }
        }
        Err(format!(
            "Unknown change {change_ref} for repository {repo_name}"
        ))
    }
    pub(super) fn snapshot_repo(&mut self, snapshot_id: &str) -> Result<Option<String>, String> {
        let snapshots = self.content_table("snapshots");
        let repositories = self.content_table("repositories");
        self.client
            .query_opt(
                &format!("select coalesce(r.repo_name, s.repo_name) as repo_name from {snapshots} s left join {repositories} r on r.repo_id = s.repo_id where s.snapshot_id = $1"),
                &[&snapshot_id],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "repo_name")))
    }
    pub(super) fn snapshot_is_ancestor(
        &mut self,
        ancestor_snapshot_id: &str,
        descendant_snapshot_id: &str,
    ) -> Result<bool, String> {
        if ancestor_snapshot_id.trim().is_empty() || descendant_snapshot_id.trim().is_empty() {
            return Ok(false);
        }
        if ancestor_snapshot_id == descendant_snapshot_id {
            return Ok(true);
        }
        let snapshots = self.content_table("snapshots");
        let mut seen = HashSet::new();
        let mut current = descendant_snapshot_id.to_string();
        while !current.is_empty() && !seen.contains(&current) {
            seen.insert(current.clone());
            let row = self
                .client
                .query_opt(
                    &format!("select parent_snapshot_id from {snapshots} where snapshot_id = $1"),
                    &[&current],
                )
                .map_err(|exc| exc.to_string())?;
            let Some(row) = row else {
                return Ok(false);
            };
            let parent = row_text(&row, "parent_snapshot_id").unwrap_or_default();
            if parent == ancestor_snapshot_id {
                return Ok(true);
            }
            current = parent;
        }
        Ok(false)
    }
    pub(super) fn repo_id_for_repo(&mut self, repo_name: &str) -> Result<Option<String>, String> {
        let repositories = self.content_table("repositories");
        self.client
            .query_opt(
                &format!("select repo_id from {repositories} where repo_name = $1"),
                &[&repo_name],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "repo_id")))
    }
    pub(super) fn repo_namespace_prefix(
        &mut self,
        repo_name: &str,
    ) -> Result<Option<String>, String> {
        let repositories = self.content_table("repositories");
        self.client
            .query_opt(
                &format!("select id_namespace_prefix from {repositories} where repo_name = $1"),
                &[&repo_name],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "id_namespace_prefix")))
    }
    pub(super) fn refresh_change_state(
        &mut self,
        change_id: &str,
        now: &str,
    ) -> Result<String, String> {
        let change = self.change_row(change_id)?;
        let existing_state = optional_text(change.get("status")).unwrap_or_default();
        if existing_state == "landed" || existing_state == "archived" {
            return Ok(existing_state);
        }
        let current_patchset_number = int_value(change.get("current_patchset_number")).unwrap_or(0);
        let new_state = if current_patchset_number == 0 {
            "draft".to_string()
        } else {
            let patchset = self.current_patchset_for_change(change_id)?;
            let latest = self.latest_policy_status(
                required_text(patchset.get("patchset_id"), "patchset.patchset_id")?.as_str(),
            )?;
            let policy = effective_policy_status(&patchset, latest.as_ref())?;
            let policy = payload_object(Some(&policy), "effective policy status")?;
            let patchset_id = required_text(patchset.get("patchset_id"), "patchset.patchset_id")?;
            let review = self.review_summary(change_id, &patchset_id)?;
            let repo_name = required_text(change.get("repo_name"), "change.repo_name")?;
            let base_line = optional_text(change.get("base_line")).unwrap_or_default();
            let base_line_head = self.content_line_head(&repo_name, &base_line)?;
            let base_snapshot_id =
                optional_text(patchset.get("base_snapshot_id")).unwrap_or_default();
            let stale = base_line_head
                .as_deref()
                .is_some_and(|head| !head.is_empty() && head != base_snapshot_id);
            let blocking_count = int_value(review.get("blocking_count")).unwrap_or(0);
            let approval_count = int_value(review.get("approval_count")).unwrap_or(0);
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
            let changes = self.control_table("changes");
            self.client
                .execute(
                    &format!("update {changes} set status = $1, updated_at = $2::text::timestamptz where change_id = $3"),
                    &[&new_state, &now, &change_id],
                )
                .map_err(|exc| exc.to_string())?;
        }
        Ok(new_state)
    }
    pub(super) fn content_line_head(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> Result<Option<String>, String> {
        if repo_name.trim().is_empty() || line_name.trim().is_empty() {
            return Ok(None);
        }
        let lines = self.content_table("lines");
        self.client
            .query_opt(
                &format!("select head_snapshot_id from {lines} where repo_name = $1 and line_name = $2 limit 1"),
                &[&repo_name, &line_name],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "head_snapshot_id")))
    }
    pub(super) fn record_event(
        &mut self,
        event_type: &str,
        entity_type: &str,
        entity_id: &str,
        payload: &JsonValue,
        created_at: &str,
    ) -> Result<(), String> {
        let events = self.control_table("events");
        let payload_json = serde_json::to_string(payload).map_err(|exc| exc.to_string())?;
        self.client
            .execute(
                &format!("insert into {events}(event_type, entity_type, entity_id, payload_json, actor_identity, actor_type, created_at) values ($1, $2, $3, $4, 'system', 'system_worker', $5::text::timestamptz)"),
                &[&event_type, &entity_type, &entity_id, &payload_json, &created_at],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(())
    }
}
