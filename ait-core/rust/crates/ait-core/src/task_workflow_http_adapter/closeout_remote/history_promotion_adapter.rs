use super::*;
use std::collections::BTreeSet;

impl HttpWorkflowCloseoutRemote {
    pub fn prepare_history_promotion(
        &mut self,
        repo_name: &str,
        payload: &Value,
    ) -> TaskWorkflowHttpClientResult<Value> {
        prepare_history_promotion_with_task_workflow_remote(self, repo_name, payload)
    }

    fn prepare_history_promotion_once(
        &mut self,
        repo_name: &str,
        payload: &Value,
        timeout_ms: Option<u64>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let response = self.with_temporary_default_timeout(timeout_ms, |remote| {
            remote.manager.prepare_history_promotion(repo_name, payload)
        })?;
        self.validate_history_promotion_response(repo_name, payload, response)
    }

    fn validate_history_promotion_response(
        &self,
        repo_name: &str,
        request: &Value,
        response: Value,
    ) -> TaskWorkflowHttpClientResult<Value> {
        if request["contract"].as_str() == Some("history-promotion-correction-prepare/v2") {
            return self.validate_history_correction_response(repo_name, request, response);
        }
        self.validate_history_projection(repo_name, request, response, None)
    }

    fn validate_history_projection(
        &self,
        repo_name: &str,
        request: &Value,
        response: Value,
        prior_candidate: Option<&crate::workflow_expected_identity::WorkflowExpectedIdentity>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let request = request.as_object().ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "History promotion request must remain an object.".to_string(),
            )
        })?;
        let response_object = response.as_object().ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "History promotion response must be an object.".to_string(),
            )
        })?;
        if response_object
            .get("replayed")
            .and_then(Value::as_bool)
            .is_none()
        {
            return Err(PlanHttpClientError::Invalid(
                "History promotion response requires boolean replayed.".to_string(),
            ));
        }
        let request_contract = request
            .get("contract")
            .and_then(Value::as_str)
            .filter(|contract| matches!(*contract, "history-promotion-prepare/v3"))
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(
                    "History promotion request has an unsupported contract.".to_string(),
                )
            })?;
        for (field, expected) in [("contract", request_contract), ("repo_name", repo_name)] {
            let actual = response_object
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid(format!(
                        "History promotion response requires non-empty {field}."
                    ))
                })?;
            if actual != expected {
                return Err(PlanHttpClientError::Invalid(format!(
                    "History promotion response {field} `{actual}` does not match `{expected}`."
                )));
            }
        }
        for field in [
            "idempotency_key",
            "target_line",
            "base_snapshot_id",
            "revision_snapshot_id",
        ] {
            let expected = request.get(field).and_then(Value::as_str).ok_or_else(|| {
                PlanHttpClientError::Invalid(format!(
                    "History promotion request requires string {field}."
                ))
            })?;
            let actual = response_object
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid(format!(
                        "History promotion response requires string {field}."
                    ))
                })?;
            if actual != expected {
                return Err(PlanHttpClientError::Invalid(format!(
                    "History promotion response {field} `{actual}` does not match request `{expected}`."
                )));
            }
        }
        let staged = request.get("stage_ordinal").is_some();
        if staged {
            for field in [
                "promotion_id",
                "stage_base_snapshot_id",
                "stage_revision_snapshot_id",
            ] {
                let expected = request.get(field).and_then(Value::as_str).ok_or_else(|| {
                    PlanHttpClientError::Invalid(format!(
                        "Staged history promotion request requires string {field}."
                    ))
                })?;
                if response_object.get(field).and_then(Value::as_str) != Some(expected) {
                    return Err(PlanHttpClientError::Invalid(format!(
                        "Staged history promotion response {field} does not match the request."
                    )));
                }
            }
            for field in ["stage_ordinal", "total_entry_count"] {
                let expected = request.get(field).and_then(Value::as_u64).ok_or_else(|| {
                    PlanHttpClientError::Invalid(format!(
                        "Staged history promotion request requires unsigned {field}."
                    ))
                })?;
                if response_object.get(field).and_then(Value::as_u64) != Some(expected) {
                    return Err(PlanHttpClientError::Invalid(format!(
                        "Staged history promotion response {field} does not match the request."
                    )));
                }
            }
            let expected_final = request
                .get("final_stage")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid(
                        "Staged history promotion request requires boolean final_stage."
                            .to_string(),
                    )
                })?;
            if response_object.get("final_stage").and_then(Value::as_bool) != Some(expected_final) {
                return Err(PlanHttpClientError::Invalid(
                    "Staged history promotion response final_stage does not match the request."
                        .to_string(),
                ));
            }
            let expected_previous = request
                .get("previous_stage_patchset_id")
                .cloned()
                .unwrap_or(Value::Null);
            let actual_previous = response_object
                .get("previous_stage_patchset_id")
                .cloned()
                .unwrap_or(Value::Null);
            if actual_previous != expected_previous {
                return Err(PlanHttpClientError::Invalid(
                    "Staged history promotion response predecessor does not match the request."
                        .to_string(),
                ));
            }
        }
        let request_entries = request
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(
                    "History promotion request requires entries array.".to_string(),
                )
            })?;
        let response_entries = response_object
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(
                    "History promotion response requires entries array.".to_string(),
                )
            })?;
        if response_entries.len() != request_entries.len() {
            return Err(PlanHttpClientError::Invalid(format!(
                "History promotion response has {} entries, expected {}.",
                response_entries.len(),
                request_entries.len()
            )));
        }
        let mut remote_task_ids = BTreeSet::new();
        let mut remote_change_refs = BTreeSet::new();
        let mut receipt_patchset_ids = BTreeSet::new();
        for (ordinal, (requested, returned)) in request_entries
            .iter()
            .zip(response_entries.iter())
            .enumerate()
        {
            let local_task = requested["local_task_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid("History entry requires local_task_id".into())
                })?;
            if returned["local_task_id"].as_str() != Some(local_task) {
                return Err(PlanHttpClientError::Invalid(
                    "History response local_task_id differs".into(),
                ));
            }
            let optional = |row: &Value,
                            field: &str|
             -> TaskWorkflowHttpClientResult<Option<String>> {
                match row.get(field) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(s)) if !s.is_empty() && s.trim() == s => Ok(Some(s.clone())),
                    _ => Err(PlanHttpClientError::Invalid(format!(
                        "History {field} must be exact text or null"
                    ))),
                }
            };
            for field in ["local_change_id", "local_change_ref"] {
                if optional(requested, field)? != optional(returned, field)? {
                    return Err(PlanHttpClientError::Invalid(format!(
                        "History response entry {ordinal} {field} differs"
                    )));
                }
            }
            let local_change = optional(requested, "local_change_ref")?;
            if local_change.is_some() != optional(requested, "local_change_id")?.is_some() {
                return Err(PlanHttpClientError::Invalid(
                    "History optional Change identity is incomplete".into(),
                ));
            }
            let task = returned["task_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid("History response requires Task".into())
                })?;
            let change = optional(returned, "change_ref")?;
            if local_change.is_some() != change.is_some() {
                return Err(PlanHttpClientError::Invalid(
                    "History response changed optional Change ownership".into(),
                ));
            }
            let receipt = returned["receipt_patchset_id"].as_str().ok_or_else(|| {
                PlanHttpClientError::Invalid("History response requires receipt_patchset_id".into())
            })?;
            crate::workflow_expected_identity::validate_patchset_reference_owner(
                receipt,
                task,
                change.as_deref(),
            )
            .map_err(PlanHttpClientError::Invalid)?;
            if !remote_task_ids.insert(task.to_string())
                || !receipt_patchset_ids.insert(receipt.to_string())
                || change.is_some_and(|id| !remote_change_refs.insert(id))
            {
                return Err(PlanHttpClientError::Invalid(
                    "History response repeats an owner or receipt Patchset".into(),
                ));
            }
            if returned.get("land").is_some_and(|value| !value.is_null()) {
                return Err(PlanHttpClientError::Invalid(
                    "Imported local history cannot synthesize Remote Land".into(),
                ));
            }
        }

        let final_stage = !staged
            || request
                .get("final_stage")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let authority_field = if staged { "stage" } else { "aggregate" };
        let authority = response_object
            .get(authority_field)
            .and_then(Value::as_object)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(format!(
                    "History promotion response requires {authority_field} object."
                ))
            })?;
        for field in ["task_id", "patchset_id"] {
            if authority
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty)
            {
                return Err(PlanHttpClientError::Invalid(format!(
                    "History promotion {authority_field} requires non-empty {field}."
                )));
            }
        }
        let final_mapping = response_entries.last().ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "History promotion response requires at least one mapping.".to_string(),
            )
        })?;
        for field in ["task_id", "change_ref"] {
            if field == "change_ref" && prior_candidate.is_some() {
                continue;
            }
            if authority.get(field).and_then(Value::as_str)
                != final_mapping.get(field).and_then(Value::as_str)
            {
                return Err(PlanHttpClientError::Invalid(format!(
                    "History promotion {authority_field} {field} does not match the final history entry."
                )));
            }
        }
        let authority_task = authority["task_id"].as_str().expect("validated Task");
        let authority_change = authority
            .get("change_ref")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str().ok_or_else(|| {
                    PlanHttpClientError::Invalid("Aggregate Change must be text or null".into())
                })
            })
            .transpose()?;
        crate::workflow_expected_identity::validate_patchset_reference_owner(
            authority["patchset_id"]
                .as_str()
                .expect("validated Patchset"),
            authority_task,
            authority_change,
        )
        .map_err(PlanHttpClientError::Invalid)?;
        if let Some(prior) = prior_candidate {
            if prior.patchset_id.split('/').next() != Some(authority_task)
                || (prior.patchset_id.contains("/C-") && authority_change.is_none())
            {
                return Err(PlanHttpClientError::Invalid(
                    "Correction changed Task or dropped its real Change".into(),
                ));
            }
        }
        let authority_patchset_id = authority.get("patchset_id").and_then(Value::as_str);
        if receipt_patchset_ids.contains(authority_patchset_id.unwrap_or_default()) {
            return Err(PlanHttpClientError::Invalid(
                "History promotion stage or aggregate Patchset repeats a receipt Patchset identity."
                    .to_string(),
            ));
        }
        let authority_patchset = authority
            .get("patchset")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(format!(
                    "History promotion {authority_field} requires Patchset projection."
                ))
            })?;
        if authority_patchset
            .get("patchset_id")
            .and_then(Value::as_str)
            != authority_patchset_id
        {
            return Err(PlanHttpClientError::Invalid(
                "History promotion stage or aggregate Patchset projection disagrees with patchset_id."
                    .to_string(),
            ));
        }
        if authority_patchset.get("task_id").and_then(Value::as_str) != Some(authority_task)
            || authority_patchset
                .get("change_ref")
                .filter(|v| !v.is_null())
                .and_then(Value::as_str)
                != authority_change
        {
            return Err(PlanHttpClientError::Invalid(
                "Aggregate Patchset has a different Task/Change association".into(),
            ));
        }
        let expected_base_field = if staged && !final_stage {
            "stage_base_snapshot_id"
        } else {
            "base_snapshot_id"
        };
        let expected_revision_field = if staged && !final_stage {
            "stage_revision_snapshot_id"
        } else {
            "revision_snapshot_id"
        };
        for (patchset_field, request_field) in [
            ("base_snapshot_id", expected_base_field),
            ("revision_snapshot_id", expected_revision_field),
        ] {
            if authority_patchset
                .get(patchset_field)
                .and_then(Value::as_str)
                != request.get(request_field).and_then(Value::as_str)
            {
                return Err(PlanHttpClientError::Invalid(format!(
                    "History promotion {authority_field} Patchset {patchset_field} does not match request {request_field}."
                )));
            }
        }
        let expected_source_kind = if staged && !final_stage {
            "history_promotion_stage"
        } else {
            "history_promotion_aggregate"
        };
        if authority_patchset
            .get("source_kind")
            .and_then(Value::as_str)
            != Some(expected_source_kind)
            || authority_patchset
                .get("governance_authority")
                .and_then(Value::as_bool)
                != Some(final_stage)
        {
            return Err(PlanHttpClientError::Invalid(
                "History promotion stage/aggregate Patchset governance authority is invalid."
                    .to_string(),
            ));
        }
        if staged {
            if final_stage {
                if response_object.get("aggregate") != response_object.get("stage") {
                    return Err(PlanHttpClientError::Invalid(
                        "Final history promotion stage and aggregate authority disagree."
                            .to_string(),
                    ));
                }
            } else if response_object
                .get("aggregate")
                .is_none_or(|value| !value.is_null())
            {
                return Err(PlanHttpClientError::Invalid(
                    "Intermediate history promotion stage must not expose aggregate authority."
                        .to_string(),
                ));
            }
        }
        Ok(response)
    }
}

impl TaskWorkflowHistoryPromotionPreparer for HttpWorkflowCloseoutRemote {
    fn prepare_history_promotion(
        &mut self,
        repo_name: &str,
        payload: &Value,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let response_deadline_ms = remote_mutation_response_deadline_timeout_ms()
            .map(|timeout_ms| timeout_ms.min(self.manager.config.default_timeout_ms));
        match self.prepare_history_promotion_once(repo_name, payload, response_deadline_ms) {
            Ok(result) => Ok(result),
            Err(error) if is_remote_mutation_timeout(&error) => {
                self.prepare_history_promotion_once(repo_name, payload, response_deadline_ms)
            }
            Err(error) => Err(error),
        }
    }
}

impl HttpWorkflowCloseoutRemote {
    fn validate_history_correction_response(
        &self,
        repo_name: &str,
        request: &Value,
        response: Value,
    ) -> TaskWorkflowHttpClientResult<Value> {
        use crate::workflow_expected_identity::canonical_patchset_id;
        use sha2::{Digest, Sha256};
        let invalid = || {
            PlanHttpClientError::Invalid(
                "Aggregate correction response differs from the pinned history or candidate."
                    .to_string(),
            )
        };
        if response["contract"].as_str() != Some("history-promotion-correction-prepare/v2")
            || response["repo_name"].as_str() != Some(repo_name)
            || response["replayed"].as_bool().is_none()
        {
            return Err(invalid());
        }
        for field in ["idempotency_key", "target_line", "revision_snapshot_id"] {
            if request[field].as_str().is_none_or(str::is_empty)
                || response[field] != request[field]
            {
                return Err(invalid());
            }
        }
        let expected = &request["expected_identity"];
        let prior = expected["patchset_id"].as_str().ok_or_else(invalid)?;
        let returned_prior = response["correction"]["prior_patchset_id"]
            .as_str()
            .ok_or_else(invalid)?;
        let original = response["correction"]["original_patchset_id"]
            .as_str()
            .ok_or_else(invalid)?;
        let canonical_prior = canonical_patchset_id(prior);
        let canonical_original = canonical_patchset_id(original);
        let returned_id = response["aggregate"]["patchset_id"]
            .as_str()
            .ok_or_else(invalid)?;
        let canonical_returned = canonical_patchset_id(returned_id);
        let prior_owner = canonical_prior.split('/').next().ok_or_else(invalid)?;
        if canonical_original.split('/').next() != Some(prior_owner)
            || canonical_returned.split('/').next() != Some(prior_owner)
            || canonical_returned
                .rsplit_once("/P-")
                .and_then(|p| p.1.parse::<u8>().ok())
                <= canonical_prior
                    .rsplit_once("/P-")
                    .and_then(|p| p.1.parse::<u8>().ok())
            || request["revision_snapshot_id"] == expected["revision_snapshot_id"]
            || request["snapshots"]
                .as_array()
                .and_then(|v| v.last())
                .and_then(|v| v.get("snapshot_id"))
                != Some(&request["revision_snapshot_id"])
        {
            return Err(invalid());
        }
        if response["base_snapshot_id"] != expected["base_snapshot_id"]
            || canonical_patchset_id(prior) != canonical_patchset_id(returned_prior)
            || original.is_empty()
            || canonical_patchset_id(original).split('/').next()
                != canonical_patchset_id(prior).split('/').next()
            || response["correction"]["snapshots"] != request["snapshots"]
            || request["snapshots"].as_array().is_none_or(Vec::is_empty)
        {
            return Err(invalid());
        }
        let entries = response["entries"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 64)
            .ok_or_else(invalid)?;
        let bytes =
            serde_json::to_vec(entries).map_err(|e| PlanHttpClientError::Invalid(e.to_string()))?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        if request["expected_receipts_sha256"].as_str() != Some(digest.as_str())
            || response["correction"]["receipt_revision_snapshot_id"]
                .as_str()
                .is_none()
            || entries.last().unwrap()["landed_snapshot_id"]
                != response["correction"]["receipt_revision_snapshot_id"]
        {
            return Err(invalid());
        }
        // Reuse the strict existing mapping, owner, Patchset and governance
        // validation after the digest binds every returned receipt field.
        let mut normalized_request = request.clone();
        normalized_request["contract"] = Value::String("history-promotion-prepare/v3".to_string());
        normalized_request["base_snapshot_id"] = expected["base_snapshot_id"].clone();
        normalized_request["entries"] = response["entries"].clone();
        let mut normalized_response = response.clone();
        normalized_response["contract"] = Value::String("history-promotion-prepare/v3".to_string());
        let prior =
            crate::workflow_expected_identity::WorkflowExpectedIdentity::from_value(expected)
                .map_err(PlanHttpClientError::Invalid)?;
        self.validate_history_projection(
            repo_name,
            &normalized_request,
            normalized_response,
            Some(&prior),
        )?;
        Ok(response)
    }
}
