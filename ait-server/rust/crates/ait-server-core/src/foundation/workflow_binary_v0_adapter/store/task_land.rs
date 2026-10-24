use super::*;

pub(super) const ATOMIC_CONTRACT: &str = "task-land-atomic/v2";
pub(super) const GUARDED_CONTRACT: &str = "task-land-atomic-guarded/v2";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedIdentity {
    repository_index: u32,
    patchset_id: String,
    revision_snapshot_id: String,
    base_snapshot_id: String,
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn validate_expected_patchset_in_read<A: ReadV0>(
        &self,
        read: &A,
        patchset_index: u32,
        expected: Option<&JsonValue>,
    ) -> Result<(), String> {
        let Some(value) = expected else {
            return Ok(());
        };
        let expected: ExpectedIdentity = serde_json::from_value(value.clone())
            .map_err(|e| format!("Invalid guarded expected_identity: {e}"))?;
        if self.repository_index != Some(expected.repository_index) {
            return Err("EXPECTED_CANDIDATE_MISMATCH: numeric Repository authority differs or is unavailable".to_string());
        }
        let patchset = self
            .read_patchset(read, patchset_index)
            .map_err(|e| e.to_string())?;
        self.patchset_owner(read, patchset)?;
        if self.patchset_index_for_id(read, &expected.patchset_id).ok() != Some(patchset_index)
            || self.content_snapshot_id(read, patchset.base_snapshot_index)?
                != expected.base_snapshot_id
            || self.content_snapshot_id(read, patchset.revision_snapshot_index)?
                != expected.revision_snapshot_id
        {
            return Err(
                "EXPECTED_CANDIDATE_MISMATCH: Patchset, revision or base changed".to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn validate_expected_identity_in_read<A: ReadV0>(
        &self,
        read: &A,
        task_index: u32,
        expected: Option<&JsonValue>,
    ) -> Result<(), String> {
        let (index, patchset) = self
            .current_task_patchset(read, task_index)?
            .ok_or("EXPECTED_CANDIDATE_MISMATCH: Task has no current Patchset")?;
        self.require_current_patchset(read, index, patchset)?;
        self.validate_expected_patchset_in_read(read, index, expected)
    }

    pub(super) fn resolve_task_land_change_ref_in_read<A: ReadV0>(
        &self,
        read: &A,
        task_or_change_ref: &str,
    ) -> Result<String, String> {
        Ok(self.owner_ref(self.workflow_owner(read, task_or_change_ref.trim())?))
    }

    pub(super) fn resolve_task_land_change_ref_from_store(
        &self,
        reference: &str,
    ) -> Result<String, String> {
        self.resolve_task_land_change_ref_in_read(&BinaryDbReadTxn::new(&self.db), reference)
    }

    #[cfg(test)]
    pub(crate) fn submit_task_land_after_prepare_for_test<F>(
        &self,
        reference: &str,
        payload: &JsonValue,
        competing_update: F,
    ) -> Result<JsonValue, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let (owner_ref, prepared) = self.prepare_atomic_task_land_payload(reference, payload)?;
        competing_update()?;
        self.submit_land(&owner_ref, &prepared)
    }

    pub(super) fn prepare_atomic_task_land_payload(
        &self,
        task_or_change_ref: &str,
        payload: &JsonValue,
    ) -> Result<(String, JsonValue), String> {
        let payload = Self::required_object(payload, "atomic Task Land payload")?;
        self.request_actor(payload)?;
        let contract = Self::required_text(payload, "contract")?;
        if contract != ATOMIC_CONTRACT && contract != GUARDED_CONTRACT {
            return Err(format!(
                "Atomic Task Land contract must be {ATOMIC_CONTRACT:?} or {GUARDED_CONTRACT:?}"
            ));
        }
        let expected = if contract == GUARDED_CONTRACT {
            Some(
                payload
                    .get("expected_identity")
                    .ok_or("Guarded atomic Task Land requires expected_identity")?,
            )
        } else {
            if payload.contains_key("expected_identity") {
                return Err(format!("expected_identity requires {GUARDED_CONTRACT}"));
            }
            None
        };
        let key = Self::required_text(payload, "idempotency_key")?;
        if key.len() > 256 {
            return Err("Atomic Task Land idempotency_key exceeds 256 bytes".to_string());
        }
        if Self::optional_text(payload, "task_or_change_ref")
            .is_some_and(|reference| reference != task_or_change_ref.trim())
        {
            return Err(
                "Atomic Task Land path and payload task_or_change_ref disagree".to_string(),
            );
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let requested = self.workflow_owner(&read, task_or_change_ref.trim())?;
        // Resolve accepted identity before current candidate or target freshness.
        let index = match self.task_successful_receipt(&read, requested.task_index)? {
            Some((_, receipt)) => receipt.patchset_index,
            None => {
                self.current_task_patchset(&read, requested.task_index)?
                    .ok_or("Task has no current Patchset")?
                    .0
            }
        };
        let patchset = self
            .read_patchset(&read, index)
            .map_err(|e| e.to_string())?;
        self.require_patchset_owner(&read, task_or_change_ref, patchset)?;
        self.validate_expected_patchset_in_read(&read, index, expected)?;
        let mut prepared = payload.clone();
        prepared.insert(
            "task_or_change_ref".to_string(),
            json!(task_or_change_ref.trim()),
        );
        if Self::optional_text(&prepared, "target_line").is_none() {
            let Some((change_index, _)) = requested.change else {
                return Err("Task-only atomic Land requires target_line".to_string());
            };
            prepared.insert(
                "target_line".to_string(),
                self.change_at(&read, change_index)?["base_line"].clone(),
            );
        }
        Ok((self.owner_ref(requested), JsonValue::Object(prepared)))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn atomic_task_land_result_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        task_index: u32,
        patchset_index: u32,
        land_index: u32,
        key: &str,
        replayed: bool,
        expected: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        let record = self
            .read_patchset(write, patchset_index)
            .map_err(|e| e.to_string())?;
        let owner = self.patchset_owner(write, record)?;
        if owner.task_index != task_index {
            return Err("Receipt Task identity disagrees".to_string());
        }
        let task = self.task_at_in_write(write, task_index)?;
        let change = owner
            .change
            .map(|(index, _)| self.change_at(write, index))
            .transpose()?
            .unwrap_or(JsonValue::Null);
        let patchset = self.patchset_at_in_write(write, patchset_index)?;
        let land = self.land_at(write, land_index)?;
        self.validate_expected_patchset_in_read(write, patchset_index, expected)?;
        let mut result = json!({
            "contract": ATOMIC_CONTRACT, "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(), "idempotency_key": key,
            "replayed": replayed, "status": land["status"], "task_id": task["task_id"],
            "task_status": task["status"], "change_id": change["change_id"],
            "change_ref": change["change_ref"], "change_status": change["status"],
            "patchset_id": patchset["patchset_id"], "target_line": land["target_line"],
            "landed_snapshot_id": land["landed_snapshot_id"],
            "history_promotion": self.history_manifest_for_patchset(write, patchset_index)?,
            "task": task, "change": change, "patchset": patchset, "land": land,
        });
        if let Some(expected) = expected {
            result["contract"] = json!(GUARDED_CONTRACT);
            result["repository_index"] = json!(self.repository_index);
            result["expected_identity"] = expected.clone();
        }
        Ok(result)
    }
}
