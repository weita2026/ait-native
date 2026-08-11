use super::*;

impl PostgresPatchsetStore {
    pub(super) fn upsert_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &JsonMap<String, JsonValue>,
        provenance_summary: &JsonMap<String, JsonValue>,
        detail: &JsonMap<String, JsonValue>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.transaction(|store| {
            let patchset = store.get_patchset_in_txn(patchset_id)?;
            let repo_id = optional_text(patchset.get("repo_id")).unwrap_or_default();
            let attestation_id = attestation_id_for_patchset(patchset_id)?;
            let evaluation_summary_json =
                serde_json::to_string(&JsonValue::Object(evaluation_summary.clone())).map_err(|exc| exc.to_string())?;
            let provenance_summary_json =
                serde_json::to_string(&JsonValue::Object(provenance_summary.clone())).map_err(|exc| exc.to_string())?;
            let detail_json =
                serde_json::to_string(&JsonValue::Object(detail.clone())).map_err(|exc| exc.to_string())?;
            let now = utc_now();
            let attestations = store.control_table("attestations");
            let existing = store
                .client
                .query_opt(
                    &format!("select attestation_id from {attestations} where patchset_id = $1"),
                    &[&patchset_id],
                )
                .map_err(|exc| exc.to_string())?;
            let event_type = if existing.is_some() {
                store
                    .client
                    .execute(
                        &format!("update {attestations} set author_mode = $1, evaluation_summary_json = $2, provenance_summary_json = $3, detail_json = $4, updated_at = $5::text::timestamptz where patchset_id = $6"),
                        &[&author_mode, &evaluation_summary_json, &provenance_summary_json, &detail_json, &now, &patchset_id],
                    )
                    .map_err(|exc| exc.to_string())?;
                "attestation.updated"
            } else {
                store
                    .client
                    .execute(
                        &format!("insert into {attestations}(attestation_id, repo_id, patchset_id, author_mode, evaluation_summary_json, provenance_summary_json, detail_json, created_at, updated_at) values ($1, $2, $3, $4, $5, $6, $7, $8::text::timestamptz, $8::text::timestamptz)"),
                        &[&attestation_id, &repo_id, &patchset_id, &author_mode, &evaluation_summary_json, &provenance_summary_json, &detail_json, &now],
                    )
                    .map_err(|exc| exc.to_string())?;
                "attestation.created"
            };
            store.invalidate_patchset_policy(patchset_id)?;
            store.record_event(
                event_type,
                "patchset",
                patchset_id,
                &json!({"patchset_id": patchset_id, "author_mode": author_mode}),
                &now,
            )?;
            store.get_attestation_in_txn(patchset_id)
        })
    }
    pub(super) fn get_attestation(
        &mut self,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.get_attestation_in_txn(patchset_id)
    }
    pub(super) fn get_attestation_in_txn(
        &mut self,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let attestations = self.control_table("attestations");
        let row = self
            .client
            .query_opt(
                &format!("select attestation_id, repo_id, patchset_id, author_mode, evaluation_summary_json, provenance_summary_json, detail_json, created_at::text as created_at, updated_at::text as updated_at from {attestations} where patchset_id = $1"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("No attestation for patchset: {patchset_id}"))?;
        attestation_row_json(&row)
    }
}
