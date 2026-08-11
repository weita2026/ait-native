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
        for (field, expected) in [
            ("contract", "history-promotion-prepare/v1"),
            ("repo_name", repo_name),
        ] {
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
            for field in ["local_task_id", "local_change_id", "local_change_ref"] {
                let expected = requested
                    .get(field)
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        PlanHttpClientError::Invalid(format!(
                            "History promotion request entry {ordinal} requires string {field}."
                        ))
                    })?;
                let actual = returned.get(field).and_then(Value::as_str).ok_or_else(|| {
                    PlanHttpClientError::Invalid(format!(
                        "History promotion response entry {ordinal} requires string {field}."
                    ))
                })?;
                if actual != expected {
                    return Err(PlanHttpClientError::Invalid(format!(
                        "History promotion response entry {ordinal} {field} `{actual}` does not match `{expected}`."
                    )));
                }
            }
            for (field, seen) in [
                ("task_id", &mut remote_task_ids),
                ("change_ref", &mut remote_change_refs),
                ("receipt_patchset_id", &mut receipt_patchset_ids),
            ] {
                let Some(value) = returned
                    .get(field)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return Err(PlanHttpClientError::Invalid(format!(
                        "History promotion response entry {ordinal} requires non-empty {field}."
                    )));
                };
                if !seen.insert(value.to_string()) {
                    return Err(PlanHttpClientError::Invalid(format!(
                        "History promotion response repeats canonical {field} `{value}`."
                    )));
                }
            }
        }
        let aggregate = response_object
            .get("aggregate")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(
                    "History promotion response requires aggregate object.".to_string(),
                )
            })?;
        for field in ["task_id", "change_ref", "patchset_id"] {
            if aggregate
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty)
            {
                return Err(PlanHttpClientError::Invalid(format!(
                    "History promotion aggregate requires non-empty {field}."
                )));
            }
        }
        let final_mapping = response_entries.last().ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "History promotion response requires at least one mapping.".to_string(),
            )
        })?;
        for field in ["task_id", "change_ref"] {
            if aggregate.get(field).and_then(Value::as_str)
                != final_mapping.get(field).and_then(Value::as_str)
            {
                return Err(PlanHttpClientError::Invalid(format!(
                    "History promotion aggregate {field} does not match the final history entry."
                )));
            }
        }
        let aggregate_patchset_id = aggregate.get("patchset_id").and_then(Value::as_str);
        if receipt_patchset_ids.contains(aggregate_patchset_id.unwrap_or_default()) {
            return Err(PlanHttpClientError::Invalid(
                "History promotion aggregate Patchset repeats a receipt Patchset identity."
                    .to_string(),
            ));
        }
        let aggregate_patchset = aggregate
            .get("patchset")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(
                    "History promotion aggregate requires Patchset projection.".to_string(),
                )
            })?;
        if aggregate_patchset
            .get("patchset_id")
            .and_then(Value::as_str)
            != aggregate_patchset_id
        {
            return Err(PlanHttpClientError::Invalid(
                "History promotion aggregate Patchset projection disagrees with patchset_id."
                    .to_string(),
            ));
        }
        for field in ["base_snapshot_id", "revision_snapshot_id"] {
            if aggregate_patchset.get(field).and_then(Value::as_str)
                != request.get(field).and_then(Value::as_str)
            {
                return Err(PlanHttpClientError::Invalid(format!(
                    "History promotion aggregate Patchset {field} does not match the request."
                )));
            }
        }
        if aggregate_patchset
            .get("source_kind")
            .and_then(Value::as_str)
            != Some("history_promotion_aggregate")
            || aggregate_patchset
                .get("governance_authority")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(PlanHttpClientError::Invalid(
                "History promotion aggregate Patchset lacks sole governance authority.".to_string(),
            ));
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
