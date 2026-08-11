use super::*;

impl PostgresPatchsetStore {
    pub(super) fn latest_policy_status(
        &mut self,
        patchset_id: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let policy_decisions = self.control_table("policy_decisions");
        self.client
            .query_opt(
                &format!("select decision, checks_json, input_fingerprint, created_at::text as created_at from {policy_decisions} where patchset_id = $1 order by policy_decision_id desc limit 1"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?
            .map(|row| {
                let checks_json = row_text(&row, "checks_json").unwrap_or_else(|| "[]".to_string());
                Ok(JsonMap::from_iter([
                    ("patchset_id".to_string(), json!(patchset_id)),
                    ("decision".to_string(), row_text(&row, "decision").map_or(JsonValue::Null, JsonValue::String)),
                    (
                        "checks".to_string(),
                        serde_json::from_str::<JsonValue>(&checks_json).unwrap_or_else(|_| json!([])),
                    ),
                    ("input_fingerprint".to_string(), row_text(&row, "input_fingerprint").map_or(JsonValue::Null, JsonValue::String)),
                    ("evaluated_at".to_string(), row_text(&row, "created_at").map_or(JsonValue::Null, JsonValue::String)),
                ]))
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
}
