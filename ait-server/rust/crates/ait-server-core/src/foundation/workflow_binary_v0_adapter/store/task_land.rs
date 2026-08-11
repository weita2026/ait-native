use super::*;

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn resolve_task_land_change_ref_in_read<A: ReadV0>(
        &self,
        read: &A,
        task_or_change_ref: &str,
    ) -> Result<String, String> {
        let requested = task_or_change_ref.trim();
        if requested.is_empty() {
            return Err("Atomic Task Land requires task_or_change_ref.".to_string());
        }
        if requested.contains("/C-") {
            let change_index = self.change_index_for_ref(read, requested)?;
            let change = self
                .read_change(read, change_index)
                .map_err(|error| Self::error("Atomic Task Land Change resolve", error))?;
            return Ok(self.change_ref(change.task_index, change.change_ordinal));
        }

        let task_index = self.task_index_for_id(read, requested)?;
        let owner = self.owner_ordinal_index(read, "task_change_index.bin", task_index)?;
        let mut cursor = owner.latest_index_plus1.checked_sub(1);
        let mut candidates = Vec::new();
        let mut visited = BTreeSet::new();
        while let Some(change_index) = cursor {
            if !visited.insert(change_index) {
                return Err(format!(
                    "Task {} Change inventory contains a cycle at index {change_index}.",
                    self.task_id(task_index)
                ));
            }
            let change = self
                .read_change(read, change_index)
                .map_err(|error| Self::error("Atomic Task Land Change inventory", error))?;
            if change.task_index != task_index {
                return Err(format!(
                    "Task {} Change inventory points at another Task.",
                    self.task_id(task_index)
                ));
            }
            if change.remote_meta & 1 == 0
                && change.change_state & CHANGE_STATE_CANCELED == 0
                && change.lifecycle() != CHANGE_LIFECYCLE_ARCHIVED
            {
                candidates.push(self.change_ref(task_index, change.change_ordinal));
            }
            cursor = change.previous_change_index_plus1.checked_sub(1);
        }
        candidates.sort();
        match candidates.as_slice() {
            [change_ref] => Ok(change_ref.clone()),
            [] => Err(format!(
                "Task {} has no landable Change.",
                self.task_id(task_index)
            )),
            _ => Err(format!(
                "Task {} has multiple landable Changes ({}); submit the exact Change reference.",
                self.task_id(task_index),
                candidates.join(", ")
            )),
        }
    }

    pub(super) fn resolve_task_land_change_ref_from_store(
        &self,
        task_or_change_ref: &str,
    ) -> Result<String, String> {
        let read = BinaryDbReadTxn::new(&self.db);
        self.resolve_task_land_change_ref_in_read(&read, task_or_change_ref)
    }

    pub(super) fn prepare_atomic_task_land_payload(
        &self,
        task_or_change_ref: &str,
        payload: &JsonValue,
    ) -> Result<(String, JsonValue), String> {
        const CONTRACT: &str = "task-land-atomic/v1";
        let payload = Self::required_object(payload, "atomic Task Land payload")?;
        if Self::required_text(payload, "contract")? != CONTRACT {
            return Err(format!("Atomic Task Land contract must be {CONTRACT:?}."));
        }
        let idempotency_key = Self::required_text(payload, "idempotency_key")?;
        if idempotency_key.len() > 256 {
            return Err("Atomic Task Land idempotency_key exceeds 256 bytes.".to_string());
        }
        if let Some(requested) = Self::optional_text(payload, "task_or_change_ref") {
            if requested != task_or_change_ref.trim() {
                return Err(
                    "Atomic Task Land path and payload task_or_change_ref disagree.".to_string(),
                );
            }
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let change_ref = self.resolve_task_land_change_ref_in_read(&read, task_or_change_ref)?;
        let change_index = self.change_index_for_ref(&read, &change_ref)?;
        let change = self.change_at(&read, change_index)?;
        let mut prepared = payload.clone();
        prepared.insert(
            "task_or_change_ref".to_string(),
            JsonValue::String(task_or_change_ref.trim().to_string()),
        );
        if Self::optional_text(&prepared, "target_line").is_none() {
            prepared.insert(
                "target_line".to_string(),
                change
                    .get("base_line")
                    .cloned()
                    .ok_or_else(|| "Atomic Task Land Change is missing base_line.".to_string())?,
            );
        }
        Ok((change_ref, JsonValue::Object(prepared)))
    }

    pub(super) fn atomic_task_land_result_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        change_index: u32,
        patchset_index: u32,
        land_index: u32,
        idempotency_key: &str,
        replayed: bool,
    ) -> Result<JsonValue, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let change_record = self
            .read_change(write, change_index)
            .map_err(|error| Self::error("Atomic Task Land Change result", error))?;
        let task = self.task_at_in_write(write, change_record.task_index)?;
        let change = self.change_at(write, change_index)?;
        let patchset = self.patchset_at_in_write(write, patchset_index)?;
        let land = self.land_at(write, land_index)?;
        let history_promotion = self
            .history_manifest_for_patchset(write, patchset_index)?
            .unwrap_or(JsonValue::Null);
        Ok(json!({
            "contract": "task-land-atomic/v1",
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "idempotency_key": idempotency_key,
            "replayed": replayed,
            "status": land.get("status").cloned().unwrap_or(JsonValue::Null),
            "task_id": task.get("task_id").cloned().unwrap_or(JsonValue::Null),
            "task_status": task.get("status").cloned().unwrap_or(JsonValue::Null),
            "change_id": change.get("change_id").cloned().unwrap_or(JsonValue::Null),
            "change_ref": change.get("change_ref").cloned().unwrap_or(JsonValue::Null),
            "change_status": change.get("status").cloned().unwrap_or(JsonValue::Null),
            "patchset_id": patchset.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
            "target_line": land.get("target_line").cloned().unwrap_or(JsonValue::Null),
            "landed_snapshot_id": land.get("landed_snapshot_id").cloned().unwrap_or(JsonValue::Null),
            "history_promotion": history_promotion,
            "task": task,
            "change": change,
            "patchset": patchset,
            "land": land,
        }))
    }
}
