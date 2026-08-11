use super::*;

pub(super) struct PostgresPolicyStore {
    pub(super) client: Client,
    pub(super) content_schema: String,
    pub(super) control_schema: String,
}

pub(super) struct PolicyInputs {
    pub(super) patchset: JsonMap<String, JsonValue>,
    pub(super) change: JsonMap<String, JsonValue>,
    pub(super) repo_policy: JsonMap<String, JsonValue>,
    pub(super) attestation: Option<JsonMap<String, JsonValue>>,
}

impl PostgresPolicyStore {
    pub(super) fn connect(runtime: PolicyStoreRuntime) -> Result<Self, String> {
        let client = Client::connect(&runtime.dsn, NoTls).map_err(|exc| exc.to_string())?;
        Ok(Self {
            client,
            content_schema: runtime.content_schema,
            control_schema: runtime.control_schema,
        })
    }

    pub(super) fn fetch_policy_inputs(
        &mut self,
        patchset_id: &str,
    ) -> Result<PolicyInputs, String> {
        let patchsets = self.control_table("patchsets");
        let changes = self.control_table("changes");
        let sql = format!(
            "select p.patchset_id, p.repo_id as patchset_repo_id, p.change_id, p.patchset_number::bigint as patchset_number, p.base_snapshot_id, p.revision_snapshot_id, p.summary, p.author_mode, p.publish_state, p.diff_stats_json, p.evaluation_state, p.created_at::text as patchset_created_at, c.repo_name, c.repo_id as change_repo_id, c.status as change_status, c.task_id, c.base_line, c.current_patchset_number::bigint as current_patchset_number, c.selected_patchset_number::bigint as selected_patchset_number from {patchsets} p join {changes} c on c.change_id = p.change_id where p.patchset_id = $1"
        );
        let row = self
            .client
            .query_opt(&sql, &[&patchset_id])
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Unknown patchset: {patchset_id}"))?;
        let mut patchset = JsonMap::new();
        insert_text(&mut patchset, "patchset_id", row_text(&row, "patchset_id"));
        insert_text(&mut patchset, "repo_id", row_text(&row, "patchset_repo_id"));
        insert_text(&mut patchset, "change_id", row_text(&row, "change_id"));
        insert_i64(
            &mut patchset,
            "patchset_number",
            row_i64(&row, "patchset_number"),
        );
        insert_text(
            &mut patchset,
            "base_snapshot_id",
            row_text(&row, "base_snapshot_id"),
        );
        insert_text(
            &mut patchset,
            "revision_snapshot_id",
            row_text(&row, "revision_snapshot_id"),
        );
        insert_text(&mut patchset, "summary", row_text(&row, "summary"));
        insert_text(&mut patchset, "author_mode", row_text(&row, "author_mode"));
        insert_text(
            &mut patchset,
            "publish_state",
            row_text(&row, "publish_state"),
        );
        insert_text(
            &mut patchset,
            "diff_stats_json",
            row_text(&row, "diff_stats_json"),
        );
        insert_text(
            &mut patchset,
            "evaluation_state",
            row_text(&row, "evaluation_state"),
        );
        insert_text(
            &mut patchset,
            "created_at",
            row_text(&row, "patchset_created_at"),
        );

        let mut change = JsonMap::new();
        insert_text(&mut change, "change_id", row_text(&row, "change_id"));
        insert_text(&mut change, "repo_name", row_text(&row, "repo_name"));
        insert_text(&mut change, "repo_id", row_text(&row, "change_repo_id"));
        insert_text(&mut change, "status", row_text(&row, "change_status"));
        insert_text(&mut change, "task_id", row_text(&row, "task_id"));
        insert_text(&mut change, "base_line", row_text(&row, "base_line"));
        insert_i64(
            &mut change,
            "current_patchset_number",
            row_i64(&row, "current_patchset_number"),
        );
        insert_i64(
            &mut change,
            "selected_patchset_number",
            row_i64(&row, "selected_patchset_number"),
        );

        let repo_name = required_text(change.get("repo_name"), "change.repo_name")?;
        let repo_policy = self.repository_policy(&repo_name)?;
        let attestation = self.attestation(patchset_id)?;
        Ok(PolicyInputs {
            patchset,
            change,
            repo_policy,
            attestation,
        })
    }

    pub(super) fn repository_policy(
        &mut self,
        repo_name: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let repositories = self.content_table("repositories");
        let Some(row) = self
            .client
            .query_opt(
                &format!("select policy_json from {repositories} where repo_name = $1"),
                &[&repo_name],
            )
            .map_err(|exc| exc.to_string())?
        else {
            return Ok(JsonMap::new());
        };
        let policy_json: Option<String> = row.get("policy_json");
        parse_json_object(policy_json.as_deref().unwrap_or("{}"))
    }

    pub(super) fn attestation(
        &mut self,
        patchset_id: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let attestations = self.control_table("attestations");
        let row = self
            .client
            .query_opt(
                &format!("select attestation_id, repo_id, patchset_id, author_mode, evaluation_summary_json, provenance_summary_json, detail_json, created_at::text as created_at, updated_at::text as updated_at from {attestations} where patchset_id = $1"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(row.map(|row| {
            let mut out = JsonMap::new();
            insert_text(&mut out, "attestation_id", row_text(&row, "attestation_id"));
            insert_text(&mut out, "repo_id", row_text(&row, "repo_id"));
            insert_text(&mut out, "patchset_id", row_text(&row, "patchset_id"));
            insert_text(&mut out, "author_mode", row_text(&row, "author_mode"));
            insert_text(
                &mut out,
                "evaluation_summary_json",
                row_text(&row, "evaluation_summary_json"),
            );
            insert_text(
                &mut out,
                "provenance_summary_json",
                row_text(&row, "provenance_summary_json"),
            );
            insert_text(&mut out, "detail_json", row_text(&row, "detail_json"));
            insert_text(&mut out, "created_at", row_text(&row, "created_at"));
            insert_text(&mut out, "updated_at", row_text(&row, "updated_at"));
            out
        }))
    }

    pub(super) fn active_waiver_rules(&mut self, patchset_id: &str) -> Result<Vec<String>, String> {
        let waivers = self.control_table("waivers");
        let rows = self
            .client
            .query(
                &format!("select rule_name, expires_at::text as expires_at from {waivers} where patchset_id = $1 order by created_at desc"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        let waiver_rows = rows
            .iter()
            .map(|row| {
                let mut item = JsonMap::new();
                insert_text(&mut item, "rule_name", row_text(row, "rule_name"));
                insert_text(&mut item, "expires_at", row_text(row, "expires_at"));
                JsonValue::Object(item)
            })
            .collect::<Vec<_>>();
        let payload = json!({"waivers": waiver_rows, "now": utc_now()});
        Ok(active_waiver_rules(payload.as_object().unwrap()))
    }

    pub(super) fn input_fingerprint(
        &mut self,
        inputs: &PolicyInputs,
        active_waivers: &[String],
    ) -> Result<String, String> {
        let reviews = self.control_table("reviews");
        let change_id = required_text(inputs.change.get("change_id"), "change.change_id")?;
        let patchset_id =
            required_text(inputs.patchset.get("patchset_id"), "patchset.patchset_id")?;
        let review_stamp = self
            .client
            .query_one(
                &format!("select coalesce(max(review_id), 0)::bigint as max_review_id from {reviews} where change_id = $1 and patchset_id = $2"),
                &[&change_id, &patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        let max_review_id: i64 = review_stamp.get("max_review_id");
        let payload = json!({
            "patchset": inputs.patchset,
            "repo_policy": inputs.repo_policy,
            "attestation": inputs.attestation,
            "max_review_id": max_review_id,
            "active_waiver_rules": active_waivers,
        });
        Ok(policy_input_fingerprint(payload.as_object().unwrap()))
    }

    pub(super) fn latest_policy_status(
        &mut self,
        patchset_id: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let policy_decisions = self.control_table("policy_decisions");
        let row = self
            .client
            .query_opt(
                &format!("select decision, checks_json, input_fingerprint, created_at::text as created_at from {policy_decisions} where patchset_id = $1 order by policy_decision_id desc limit 1"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        row.map(|row| {
            let checks_json: Option<String> = row.get("checks_json");
            let checks = serde_json::from_str::<JsonValue>(checks_json.as_deref().unwrap_or("[]"))
                .unwrap_or_else(|_| json!([]));
            let mut out = JsonMap::new();
            out.insert("patchset_id".to_string(), json!(patchset_id));
            insert_text(&mut out, "decision", row_text(&row, "decision"));
            out.insert("checks".to_string(), checks);
            insert_text(
                &mut out,
                "input_fingerprint",
                row_text(&row, "input_fingerprint"),
            );
            insert_text(&mut out, "evaluated_at", row_text(&row, "created_at"));
            Ok(out)
        })
        .transpose()
    }

    pub(super) fn review_summary(
        &mut self,
        change_id: &str,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let reviews = self.control_table("reviews");
        let rows = self
            .client
            .query(
                &format!("select review_id::bigint as review_id, reviewer, action, blocking, comment, created_at::text as created_at, patchset_id from {reviews} where change_id = $1 and patchset_id = $2 order by review_id asc"),
                &[&change_id, &patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        let reviews = rows.iter().map(review_row_json).collect::<Vec<_>>();
        Ok(review_summary_from_rows(&reviews, Some(patchset_id)))
    }

    pub(super) fn current_patchset_for_change(
        &mut self,
        change_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let patchsets = self.control_table("patchsets");
        let row = self
            .client
            .query_opt(
                &format!("select patchset_id, repo_id, change_id, patchset_number::bigint as patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at::text as created_at from {patchsets} where change_id = $1 order by patchset_number desc limit 1"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Change {change_id} has no published patchset"))?;
        let mut patchset = JsonMap::new();
        insert_text(&mut patchset, "patchset_id", row_text(&row, "patchset_id"));
        insert_text(&mut patchset, "repo_id", row_text(&row, "repo_id"));
        insert_text(&mut patchset, "change_id", row_text(&row, "change_id"));
        insert_i64(
            &mut patchset,
            "patchset_number",
            row_i64(&row, "patchset_number"),
        );
        insert_text(
            &mut patchset,
            "base_snapshot_id",
            row_text(&row, "base_snapshot_id"),
        );
        insert_text(
            &mut patchset,
            "revision_snapshot_id",
            row_text(&row, "revision_snapshot_id"),
        );
        insert_text(&mut patchset, "summary", row_text(&row, "summary"));
        insert_text(&mut patchset, "author_mode", row_text(&row, "author_mode"));
        insert_text(
            &mut patchset,
            "publish_state",
            row_text(&row, "publish_state"),
        );
        insert_text(
            &mut patchset,
            "diff_stats_json",
            row_text(&row, "diff_stats_json"),
        );
        insert_text(
            &mut patchset,
            "evaluation_state",
            row_text(&row, "evaluation_state"),
        );
        insert_text(&mut patchset, "created_at", row_text(&row, "created_at"));
        Ok(patchset)
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

    pub(super) fn content_table(&self, name: &str) -> String {
        schema_table(&self.content_schema, name)
    }

    pub(super) fn control_table(&self, name: &str) -> String {
        schema_table(&self.control_schema, name)
    }
}
