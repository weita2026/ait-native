use super::*;

impl PostgresPolicyStore {
    pub(super) fn get_policy(
        &mut self,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let inputs = self.fetch_policy_inputs(patchset_id)?;
        let latest = self.latest_policy_status(patchset_id)?;
        let status = effective_policy_status(&inputs.patchset, latest.as_ref())?;
        let mut status = json_object(status.as_object(), "effective policy status")?;
        let mut policy_context = policy_context_for_patchset(
            &inputs.repo_policy,
            &inputs.patchset,
            inputs.attestation.as_ref(),
        )?;
        let requires_summary = requires_code_review_summary(&policy_context);
        ensure_effective_requirements(&mut policy_context).insert(
            "require_code_review_summary".to_string(),
            json!(requires_summary),
        );
        enrich_status_with_policy_context(&mut status, &inputs.repo_policy, &policy_context);
        Ok(status)
    }

    pub(super) fn evaluate_policy(
        &mut self,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.client
            .batch_execute("begin")
            .map_err(|exc| exc.to_string())?;
        let result = self.evaluate_policy_in_txn(patchset_id);
        match result {
            Ok(policy) => {
                self.client
                    .batch_execute("commit")
                    .map_err(|exc| exc.to_string())?;
                Ok(policy)
            }
            Err(err) => {
                let _ = self.client.batch_execute("rollback");
                Err(err)
            }
        }
    }

    pub(super) fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        inline: bool,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.client
            .batch_execute("begin")
            .map_err(|exc| exc.to_string())?;
        let result = self.create_waiver_in_txn(patchset_id, rule_name, reason, expires_at);
        let mut waiver = match result {
            Ok(waiver) => {
                self.client
                    .batch_execute("commit")
                    .map_err(|exc| exc.to_string())?;
                waiver
            }
            Err(err) => {
                let _ = self.client.batch_execute("rollback");
                return Err(err);
            }
        };
        if inline {
            waiver.insert(
                "policy".to_string(),
                JsonValue::Object(self.evaluate_policy(patchset_id)?),
            );
        }
        Ok(waiver)
    }

    pub(super) fn evaluate_policy_in_txn(
        &mut self,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let inputs = self.fetch_policy_inputs(patchset_id)?;
        let waivers = self.active_waiver_rules(patchset_id)?;
        let mut policy_context = policy_context_for_patchset(
            &inputs.repo_policy,
            &inputs.patchset,
            inputs.attestation.as_ref(),
        )?;
        let requires_summary = requires_code_review_summary(&policy_context);
        let effective_requirements = ensure_effective_requirements(&mut policy_context);
        effective_requirements.insert(
            "require_code_review_summary".to_string(),
            json!(requires_summary),
        );
        let input_fingerprint = self.input_fingerprint(&inputs, &waivers)?;
        let latest = self.latest_policy_status(patchset_id)?;
        let evaluation_state = optional_text(inputs.patchset.get("evaluation_state"))
            .unwrap_or_else(|| "pending".to_string());
        if evaluation_state != "pending" {
            if let Some(latest_status) = latest.as_ref() {
                if optional_text(latest_status.get("decision")).as_deref()
                    == Some(evaluation_state.as_str())
                    && optional_text(latest_status.get("input_fingerprint")).as_deref()
                        == Some(input_fingerprint.as_str())
                {
                    let mut cached = latest_status.clone();
                    enrich_status_with_policy_context(
                        &mut cached,
                        &inputs.repo_policy,
                        &policy_context,
                    );
                    return Ok(cached);
                }
            }
        }

        let review = self.review_summary(
            required_text(inputs.change.get("change_id"), "change.change_id")?.as_str(),
            patchset_id,
        )?;
        let mut evaluation_payload = JsonMap::new();
        evaluation_payload.insert("patchset_id".to_string(), json!(patchset_id));
        evaluation_payload.insert(
            "patchset".to_string(),
            JsonValue::Object(inputs.patchset.clone()),
        );
        evaluation_payload.insert(
            "policy_context".to_string(),
            JsonValue::Object(policy_context.clone()),
        );
        evaluation_payload.insert(
            "effective_requirements".to_string(),
            policy_context
                .get("effective_requirements")
                .cloned()
                .unwrap_or_else(|| json!({})),
        );
        evaluation_payload.insert(
            "requires_code_review_summary".to_string(),
            json!(requires_summary),
        );
        evaluation_payload.insert(
            "attestation".to_string(),
            inputs
                .attestation
                .as_ref()
                .map(|row| JsonValue::Object(row.clone()))
                .unwrap_or(JsonValue::Null),
        );
        evaluation_payload.insert("review_summary".to_string(), JsonValue::Object(review));
        evaluation_payload.insert("active_waiver_rules".to_string(), json!(waivers));
        evaluation_payload.insert("required_approvals".to_string(), json!(REQUIRED_APPROVALS));

        let gate_evaluation = policy_gate_evaluation(&evaluation_payload);
        let decision =
            optional_text(gate_evaluation.get("decision")).unwrap_or_else(|| "pending".to_string());
        let checks = gate_evaluation
            .get("checks")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let checks_json = serde_json::to_string(&checks).map_err(|exc| exc.to_string())?;
        let now = utc_now();
        let repo_id = optional_text(inputs.patchset.get("repo_id"))
            .or_else(|| optional_text(inputs.change.get("repo_id")))
            .unwrap_or_default();
        let policy_decisions = self.control_table("policy_decisions");
        self.client
            .execute(
                &format!("insert into {policy_decisions}(repo_id, patchset_id, decision, checks_json, input_fingerprint, created_at) values ($1, $2, $3, $4, $5, $6::text::timestamptz)"),
                &[&repo_id, &patchset_id, &decision, &checks_json, &input_fingerprint, &now],
            )
            .map_err(|exc| exc.to_string())?;
        let patchsets = self.control_table("patchsets");
        self.client
            .execute(
                &format!("update {patchsets} set evaluation_state = $1 where patchset_id = $2"),
                &[&decision, &patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        let change_id = required_text(inputs.change.get("change_id"), "change.change_id")?;
        let changes = self.control_table("changes");
        self.client
            .execute(
                &format!(
                    "update {changes} set updated_at = $1::text::timestamptz where change_id = $2"
                ),
                &[&now, &change_id],
            )
            .map_err(|exc| exc.to_string())?;
        self.record_event(
            "policy.evaluated",
            "patchset",
            patchset_id,
            &json!({"patchset_id": patchset_id, "decision": decision}),
            &now,
        )?;
        self.refresh_change_state(&change_id, &now)?;

        let mut policy = JsonMap::new();
        policy.insert("patchset_id".to_string(), json!(patchset_id));
        policy.insert("decision".to_string(), json!(decision));
        policy.insert("checks".to_string(), JsonValue::Array(checks));
        policy.insert("evaluated_at".to_string(), json!(now));
        if let Some(value) = gate_evaluation.get("effective_requirements") {
            policy_context.insert("effective_requirements".to_string(), value.clone());
        }
        enrich_status_with_policy_context(&mut policy, &inputs.repo_policy, &policy_context);
        Ok(policy)
    }

    pub(super) fn create_waiver_in_txn(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let patchsets = self.control_table("patchsets");
        let row = self
            .client
            .query_opt(
                &format!("select change_id, repo_id from {patchsets} where patchset_id = $1"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Unknown patchset: {patchset_id}"))?;
        let change_id: String = row.get("change_id");
        let repo_id: Option<String> = row.get("repo_id");
        let waivers = self.control_table("waivers");
        let count_row = self
            .client
            .query_one(
                &format!("select count(*)::bigint as c from {waivers} where patchset_id = $1"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        let count: i64 = count_row.get("c");
        let now = utc_now();
        let waiver_payload = json!({
            "patchset_id": patchset_id,
            "rule_name": rule_name,
            "reason": reason,
            "expires_at": expires_at,
            "existing_waiver_count": count,
            "created_at": now,
            "change_id": change_id,
        });
        let waiver = policy_waiver_request(waiver_payload.as_object().unwrap())?;
        let waiver_id = required_text(waiver.get("waiver_id"), "waiver.waiver_id")?;
        let shaped_rule = required_text(waiver.get("rule_name"), "waiver.rule_name")?;
        let shaped_reason = optional_text(waiver.get("reason")).unwrap_or_default();
        let shaped_expires = optional_text(waiver.get("expires_at"));
        let shaped_created = required_text(waiver.get("created_at"), "waiver.created_at")?;
        let repo_id = repo_id.unwrap_or_default();
        self.client
            .execute(
                &format!("insert into {waivers}(waiver_id, repo_id, patchset_id, rule_name, reason, expires_at, created_at) values ($1, $2, $3, $4, $5, $6::text::timestamptz, $7::text::timestamptz)"),
                &[
                    &waiver_id,
                    &repo_id,
                    &patchset_id,
                    &shaped_rule,
                    &shaped_reason,
                    &shaped_expires,
                    &shaped_created,
                ],
            )
            .map_err(|exc| exc.to_string())?;
        self.record_event(
            "policy.waived",
            "patchset",
            patchset_id,
            &json!({"patchset_id": patchset_id, "rule_name": shaped_rule, "reason": shaped_reason}),
            &shaped_created,
        )?;
        Ok(waiver)
    }
}
