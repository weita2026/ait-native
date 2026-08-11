use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PayloadSyncBoundary {
    BeforeReference,
    TransactionCommit,
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn resolve_plan_binding_for_create(
        &self,
        payload: &JsonMap<String, JsonValue>,
    ) -> Result<(u32, u32, u64), String> {
        let plan_id = Self::optional_text(payload, "plan_id");
        let revision_id = Self::optional_text(payload, "origin_plan_revision_id");
        let item_ref = Self::optional_text(payload, "plan_item_ref");
        if plan_id.is_none() && revision_id.is_none() && item_ref.is_none() {
            return Ok((0, 0, 0));
        }
        let plan_id = plan_id.ok_or_else(|| "plan linkage requires plan_id".to_string())?;
        let revision_id = revision_id
            .ok_or_else(|| "plan linkage requires origin_plan_revision_id".to_string())?;
        let item_ref = item_ref
            .ok_or_else(|| "Binary DB v0 plan linkage requires plan_item_ref".to_string())?;
        let plan_index = plan_id
            .strip_prefix("PR-")
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| format!("invalid compact Plan identity: {plan_id}"))?;
        let revision_index = revision_id
            .strip_prefix("plan-revision:")
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| format!("invalid compact Plan revision identity: {revision_id}"))?;
        let revision = BinaryDbServerPlanService::new(self.db.clone())
            .get_plan_revision(&plan_id, &revision_id)?;
        let items = revision
            .get("items")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| format!("Plan revision {revision_id} has no item authority"))?;
        let item_offset = items
            .iter()
            .position(|item| {
                item.get("plan_item_ref").and_then(JsonValue::as_str) == Some(item_ref.as_str())
            })
            .ok_or_else(|| {
                format!("Plan item ref {item_ref:?} is not present in revision {revision_id}")
            })?;
        let read = BinaryDbReadTxn::new(&self.db);
        let raw = read
            .read_record(
                BinaryFileId::new("plan_revision.bin", 1, 56, BinaryDbFileFamily::Plan),
                revision_index,
            )
            .map_err(|error| Self::error("Task Plan binding", error))?;
        let persisted_plan_index = u32::from_le_bytes(raw[16..20].try_into().unwrap());
        if persisted_plan_index != plan_index {
            return Err(format!(
                "Plan revision {revision_id} belongs to PR-{persisted_plan_index}, not {plan_id}"
            ));
        }
        let item_count = usize::from(u16::from_le_bytes(raw[6..8].try_into().unwrap()));
        let item_start = u32::from_le_bytes(raw[24..28].try_into().unwrap());
        if item_offset >= item_count {
            return Err(format!(
                "Plan item ref {item_ref:?} is outside the fixed item range for {revision_id}"
            ));
        }
        let item_index = item_start
            .checked_add(u32::try_from(item_offset).map_err(|_| "Plan item offset exceeds u32")?)
            .ok_or_else(|| "Plan item index overflow".to_string())?;
        Ok((revision_index + 1, item_index + 1, Self::now_s()?))
    }

    pub(super) fn append_task_index_rows<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        expected_task_index: u32,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let inventory =
            WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord::default())?;
        let ordinal = WorkflowBinaryV0Codec::encode_ordinal_index(V0OrdinalIndexRecord::default())?;
        for (path, raw) in [
            ("task_change_index.bin", ordinal.as_slice()),
            ("task_patchset_index.bin", inventory.as_slice()),
            ("task_attest_index.bin", ordinal.as_slice()),
            ("task_review_index.bin", inventory.as_slice()),
            ("task_policy_index.bin", inventory.as_slice()),
            ("task_land_index.bin", inventory.as_slice()),
            ("task_snapshot_index.bin", ordinal.as_slice()),
            ("task_waiver_index.bin", ordinal.as_slice()),
        ] {
            let index = tx.append_record(WorkflowBinaryV0Codec::chain_index_file(path), raw)?;
            if index != expected_task_index {
                return Err(BinaryDbError::corruption(format!(
                    "{path} append index {index} disagrees with Task index {expected_task_index}"
                )));
            }
        }
        Ok(())
    }

    pub(super) fn append_change_index_rows<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        expected_change_index: u32,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let inventory =
            WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord::default())?;
        let ordinal = WorkflowBinaryV0Codec::encode_ordinal_index(V0OrdinalIndexRecord::default())?;
        for (path, raw) in [
            ("change_patchset_index.bin", ordinal.as_slice()),
            ("change_land_index.bin", ordinal.as_slice()),
            ("change_snapshot_index.bin", inventory.as_slice()),
        ] {
            let index = tx.append_record(WorkflowBinaryV0Codec::chain_index_file(path), raw)?;
            if index != expected_change_index {
                return Err(BinaryDbError::corruption(format!(
                    "{path} append index {index} disagrees with Change index {expected_change_index}"
                )));
            }
        }
        Ok(())
    }

    pub(super) fn sync_file<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        relative_path: &str,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        tx.fsync_policy().sync_file_data(
            &ServerRemoteBinaryDb::authority_root(&self.db)
                .as_path()
                .join(relative_path),
        )
    }

    fn task_change_index_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        task_index: u32,
    ) -> Result<V0OrdinalIndexRecord, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        WorkflowBinaryV0Codec::decode_ordinal_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("task_change_index.bin"),
                task_index,
            )
            .map_err(|error| Self::error("Task Change index read", error))?,
        )
        .map_err(|error| Self::error("Task Change index decode", error))
    }

    pub(super) fn append_task_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        payload: &JsonMap<String, JsonValue>,
        revision_plus1: u32,
        item_plus1: u32,
        linked_at_s: u64,
        now: u64,
        payload_sync_boundary: PayloadSyncBoundary,
    ) -> Result<(u32, String), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let operation = "ServerWorkflowTaskStore::append_task_in_write";
        let title = Self::required_text(payload, "title")?;
        let intent = Self::required_text(payload, "intent")?;
        let explicit_id = Self::optional_text(payload, "task_id");
        let payload_bytes =
            WorkflowBinaryV0Codec::encode_task_payload(&V0TaskPayload { title, intent })
                .map_err(|error| Self::error(operation, error))?;
        let task_index = tx
            .record_count(WorkflowBinaryV0Codec::task_file())
            .map_err(|error| Self::error(operation, error))?;
        let canonical_id = self.task_id(task_index);
        if let Some(explicit_id) = explicit_id.as_deref() {
            if explicit_id != canonical_id {
                return Err(format!(
                    "Binary DB v0 Task identity is derived as {canonical_id}, not {explicit_id}"
                ));
            }
        }
        let range = tx
            .append_payload(WorkflowBinaryV0Codec::task_payload_file(), &payload_bytes)
            .map_err(|error| Self::error(operation, error))?;
        if payload_sync_boundary == PayloadSyncBoundary::BeforeReference {
            self.sync_file(tx, "task_payload.bin")
                .map_err(|error| Self::error(operation, error))?;
        }
        let record = V0TaskRecord {
            task_meta: if revision_plus1 == 0 {
                0
            } else {
                TASK_META_PLANNED
            },
            remote_meta: 0,
            payload_len: u16::try_from(range.payload_len)
                .map_err(|_| "Task payload exceeds u16".to_string())?,
            payload_offset: range.payload_offset,
            origin_plan_revision_index_plus1: revision_plus1,
            plan_item_index_plus1: item_plus1,
            created_at_s: now,
            updated_at_s: now,
            plan_linked_at_s: linked_at_s,
            fetched_at_s: now,
            closed_at_s: 0,
        };
        let raw = WorkflowBinaryV0Codec::encode_task(record)
            .map_err(|error| Self::error(operation, error))?;
        let appended = tx
            .append_record(WorkflowBinaryV0Codec::task_file(), &raw)
            .map_err(|error| Self::error(operation, error))?;
        if appended != task_index {
            return Err("Binary DB v0 Task append index drift".to_string());
        }
        self.append_task_index_rows(tx, task_index)
            .map_err(|error| Self::error(operation, error))?;
        Ok((task_index, canonical_id))
    }

    fn append_change_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        payload: &JsonMap<String, JsonValue>,
        task_index: u32,
        task_id: &str,
        now: u64,
    ) -> Result<(u32, String), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        self.append_change_with_fork_in_write(
            tx,
            payload,
            task_index,
            task_id,
            now,
            None,
            PayloadSyncBoundary::BeforeReference,
        )
    }

    pub(super) fn append_change_with_fork_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        payload: &JsonMap<String, JsonValue>,
        task_index: u32,
        task_id: &str,
        now: u64,
        historical_fork_snapshot_index_plus1: Option<u32>,
        payload_sync_boundary: PayloadSyncBoundary,
    ) -> Result<(u32, String), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let operation = "ServerWorkflowChangeStore::append_change_in_write";
        let title = Self::required_text(payload, "title")?;
        let base_line = Self::required_text(payload, "base_line")?;
        let explicit_id = Self::optional_text(payload, "change_id");
        let title_bytes = WorkflowBinaryV0Codec::encode_single_text_payload(&title, "Change title")
            .map_err(|error| Self::error(operation, error))?;
        let task = self
            .read_task(tx, task_index)
            .map_err(|error| Self::error(operation, error))?;
        if task.is_terminal() {
            return Err(format!(
                "Task {task_id} is terminal and cannot accept new Changes"
            ));
        }
        let mut task_index_row = self.task_change_index_in_write(tx, task_index)?;
        if task_index_row.next_ordinal >= 64 {
            return Err(format!(
                "Task {task_id} has exhausted its v0 Change ordinals"
            ));
        }
        let change_ordinal = task_index_row.next_ordinal;
        let change_ref = self.change_ref(task_index, change_ordinal);
        if let Some(explicit) = explicit_id.as_deref() {
            let short = format!("C-{:02}", change_ordinal + 1);
            if explicit != change_ref && explicit != short {
                return Err(format!(
                    "Binary DB v0 Change identity is derived as {change_ref}, not {explicit}"
                ));
            }
        }
        let line_store =
            ServerBinaryDbLineStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let (line_index, line) = line_store
            .line_by_name_in_write(tx, &base_line)
            .map_err(|error| Self::error(operation, error))?
            .ok_or_else(|| format!("Unknown canonical base Line: {base_line}"))?;
        let fork_snapshot_index_plus1 =
            historical_fork_snapshot_index_plus1.unwrap_or(line.head_snapshot_index_plus1);
        if let Some(requested_fork) = Self::optional_text(payload, "fork_snapshot_id") {
            let actual = fork_snapshot_index_plus1
                .checked_sub(1)
                .map(|index| {
                    ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(
                        self.db.clone(),
                    )
                    .snapshot_id_at_in_write(tx, index)
                    .map_err(|error| Self::error(operation, error))
                })
                .transpose()?;
            if actual.as_deref() != Some(requested_fork.as_str()) {
                return Err(format!(
                    "Change fork_snapshot_id {requested_fork} does not equal base Line head {actual:?}"
                ));
            }
        }
        let change_index = tx
            .record_count(WorkflowBinaryV0Codec::change_file())
            .map_err(|error| Self::error(operation, error))?;
        let range = tx
            .append_payload(WorkflowBinaryV0Codec::change_payload_file(), &title_bytes)
            .map_err(|error| Self::error(operation, error))?;
        if payload_sync_boundary == PayloadSyncBoundary::BeforeReference {
            self.sync_file(tx, "change_payload.bin")
                .map_err(|error| Self::error(operation, error))?;
        }
        let record = V0ChangeRecord {
            change_meta: CHANGE_LIFECYCLE_DRAFT,
            remote_meta: 0,
            payload_len: u16::try_from(range.payload_len)
                .map_err(|_| "Change title exceeds u16".to_string())?,
            change_ordinal,
            change_state: 0,
            reserved1: 0,
            payload_offset: range.payload_offset,
            task_index,
            previous_change_index_plus1: task_index_row.latest_index_plus1,
            selected_patchset_index_plus1: 0,
            fork_snapshot_index_plus1,
            created_at_s: now,
            updated_at_s: now,
            fetched_at_s: now,
            base_line_index_plus1: line_index
                .checked_add(1)
                .ok_or_else(|| "base Line index plus-one overflow".to_string())?,
            archived_at_s: 0,
        };
        let raw = WorkflowBinaryV0Codec::encode_change(record)
            .map_err(|error| Self::error(operation, error))?;
        if tx
            .append_record(WorkflowBinaryV0Codec::change_file(), &raw)
            .map_err(|error| Self::error(operation, error))?
            != change_index
        {
            return Err("Binary DB v0 Change append index drift".to_string());
        }
        self.append_change_index_rows(tx, change_index)
            .map_err(|error| Self::error(operation, error))?;
        task_index_row.latest_index_plus1 = change_index + 1;
        task_index_row.count = task_index_row
            .count
            .checked_add(1)
            .ok_or_else(|| "Task Change count exceeds u16".to_string())?;
        task_index_row.next_ordinal = change_ordinal + 1;
        let index_raw = WorkflowBinaryV0Codec::encode_ordinal_index(task_index_row)
            .map_err(|error| Self::error(operation, error))?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_change_index.bin"),
            task_index,
            &index_raw,
        )
        .map_err(|error| Self::error(operation, error))?;
        Ok((change_index, change_ref))
    }

    pub(super) fn task_for_plan_binding_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        revision_plus1: u32,
        item_plus1: u32,
    ) -> Result<Option<u32>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let count = tx
            .record_count(WorkflowBinaryV0Codec::task_file())
            .map_err(|error| Self::error("Task Plan binding inventory", error))?;
        for task_index in 0..count {
            let task = self
                .read_task(tx, task_index)
                .map_err(|error| Self::error("Task Plan binding inventory", error))?;
            // Canceled Tasks remain immutable history but relinquish exclusive
            // ownership of their Plan revision/item binding.
            if task.remote_meta & 1 == 0
                && task.task_meta & TASK_META_CANCELED == 0
                && task.origin_plan_revision_index_plus1 == revision_plus1
                && task.plan_item_index_plus1 == item_plus1
            {
                return Ok(Some(task_index));
            }
        }
        Ok(None)
    }

    fn atomic_task_start_result(
        &self,
        binding: &crate::foundation::server_plan_binary_db::task_start::TaskStartPlanBinding,
        plan_item_ref: &str,
        idempotency_key: &str,
        task_index: u32,
        change_index: u32,
        replayed: bool,
    ) -> Result<JsonValue, String> {
        let plan = BinaryDbServerPlanService::new(self.db.clone()).get_plan(&binding.plan_id)?;
        let read = BinaryDbReadTxn::new(&self.db);
        let task = self.task_at(&read, task_index)?;
        let change = self.change_at(&read, change_index)?;
        Ok(json!({
            "contract": "task-start-atomic/v1",
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "idempotency_key": idempotency_key,
            "replayed": replayed,
            "plan_action": binding.action,
            "plan_id": binding.plan_id,
            "plan_revision_id": binding.plan_revision_id,
            "plan_item_ref": plan_item_ref,
            "plan": plan,
            "task_id": self.task_id(task_index),
            "task": task,
            "change": change,
        }))
    }

    fn replay_atomic_task_start(
        &self,
        binding: &crate::foundation::server_plan_binary_db::task_start::TaskStartPlanBinding,
        plan_item_ref: &str,
        idempotency_key: &str,
        task_payload: &JsonMap<String, JsonValue>,
        change_payload: &JsonMap<String, JsonValue>,
        task_index: u32,
    ) -> Result<JsonValue, String> {
        let read = BinaryDbReadTxn::new(&self.db);
        let task = self.task_at(&read, task_index)?;
        for field in ["title", "intent", "task_id"] {
            if let Some(requested) = task_payload.get(field).filter(|value| !value.is_null()) {
                if task.get(field) != Some(requested) {
                    return Err(format!(
                        "Atomic task-start replay conflicts with existing Task {} field {field}.",
                        self.task_id(task_index)
                    ));
                }
            }
        }
        let change_count = read
            .record_count(WorkflowBinaryV0Codec::change_file())
            .map_err(|error| Self::error("Atomic task-start replay Change inventory", error))?;
        let mut initial_change_index = None;
        for change_index in 0..change_count {
            let change = self
                .read_change(&read, change_index)
                .map_err(|error| Self::error("Atomic task-start replay Change inventory", error))?;
            if change.remote_meta & 1 == 0
                && change.task_index == task_index
                && change.change_ordinal == 0
            {
                if initial_change_index.replace(change_index).is_some() {
                    return Err(format!(
                        "Task {} has multiple initial Changes.",
                        self.task_id(task_index)
                    ));
                }
            }
        }
        let change_index = initial_change_index.ok_or_else(|| {
            format!(
                "Task {} has no initial Change for atomic replay.",
                self.task_id(task_index)
            )
        })?;
        let change = self.change_at(&read, change_index)?;
        for field in [
            "title",
            "base_line",
            "change_id",
            "fork_snapshot_id",
            "forked_from_line",
        ] {
            if let Some(requested) = change_payload.get(field).filter(|value| !value.is_null()) {
                if change.get(field) != Some(requested) {
                    return Err(format!(
                        "Atomic task-start replay conflicts with existing Change {} field {field}.",
                        change["change_ref"].as_str().unwrap_or("<unknown>")
                    ));
                }
            }
        }
        drop(read);
        self.atomic_task_start_result(
            binding,
            plan_item_ref,
            idempotency_key,
            task_index,
            change_index,
            true,
        )
    }

    pub(super) fn complete_task_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        task_index: u32,
        now: u64,
    ) -> Result<bool, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let operation = "ServerWorkflowTaskStore::complete_task_in_write";
        let mut record = self
            .read_task(tx, task_index)
            .map_err(|error| Self::error(operation, error))?;
        if record.task_meta & TASK_META_COMPLETED != 0 {
            return Ok(false);
        }
        if record.is_terminal() {
            return Err(format!(
                "Task {} is already terminal and cannot be completed by Task Land",
                self.task_id(task_index)
            ));
        }
        let mut dependency = WorkflowBinaryV0Codec::encode_task(record)
            .map_err(|error| Self::error(operation, error))?;
        dependency[52..60].copy_from_slice(&now.to_le_bytes());
        tx.overwrite_record(WorkflowBinaryV0Codec::task_file(), task_index, &dependency)
            .map_err(|error| Self::error(operation, error))?;
        self.sync_file(tx, "task.bin")
            .map_err(|error| Self::error(operation, error))?;
        record.closed_at_s = now;
        record.updated_at_s = now;
        record.task_meta |= TASK_META_COMPLETED;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::task_file(),
            task_index,
            &WorkflowBinaryV0Codec::encode_task(record)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        Ok(true)
    }
}

impl<D> ServerWorkflowTaskStore for BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn prepare_history_promotion(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.prepare_history_promotion_from_payload(repo_name, payload)
    }

    fn start_plan_bound_task(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        const CONTRACT: &str = "task-start-atomic/v1";
        let operation = "ServerWorkflowTaskStore::start_plan_bound_task";
        self.repo_scope(operation, repo_name)?;
        let payload = Self::required_object(payload, "atomic task-start payload")?;
        if Self::required_text(payload, "contract")? != CONTRACT {
            return Err(format!("Atomic task-start contract must be {CONTRACT:?}."));
        }
        let idempotency_key = Self::required_text(payload, "idempotency_key")?;
        if idempotency_key.len() > 256 {
            return Err("Atomic task-start idempotency_key exceeds 256 bytes.".to_string());
        }
        let plan_item_ref = Self::required_text(payload, "plan_item_ref")?;
        let plan_operation = payload
            .get("plan")
            .ok_or_else(|| "Atomic task-start payload requires plan.".to_string())?;
        let task_payload = payload
            .get("task")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| "Atomic task-start payload requires a task object.".to_string())?;
        let change_payload = payload
            .get("change")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| "Atomic task-start payload requires a change object.".to_string())?;
        for field in ["plan_id", "origin_plan_revision_id", "plan_item_ref"] {
            if task_payload
                .get(field)
                .is_some_and(|value| !value.is_null())
            {
                return Err(format!(
                    "Atomic task-start task.{field} is server-derived from the Plan operation."
                ));
            }
        }
        if change_payload
            .get("task_id")
            .is_some_and(|value| !value.is_null())
        {
            return Err(
                "Atomic task-start change.task_id is server-derived from the Task append."
                    .to_string(),
            );
        }

        let plan_service = BinaryDbServerPlanService::new(self.db.clone());
        let prepared =
            plan_service.prepare_task_start_plan(repo_name, plan_operation, &plan_item_ref)?;
        let now = Self::now_s()?;
        let mut tx = plan_service.begin_task_start_write(&prepared)?;
        let binding = plan_service.apply_task_start_plan(&mut tx, &prepared, now)?;
        let revision_plus1 = binding
            .revision_index
            .checked_add(1)
            .ok_or_else(|| "Atomic task-start Plan revision index overflow.".to_string())?;
        let item_plus1 = binding
            .item_index
            .checked_add(1)
            .ok_or_else(|| "Atomic task-start Plan item index overflow.".to_string())?;
        let existing_task_index = {
            let workflow_tx = tx.workflow_write()?;
            self.task_for_plan_binding_in_write(workflow_tx, revision_plus1, item_plus1)?
        };
        if let Some(existing_task_index) = existing_task_index {
            if binding.action != "existing" {
                return Err(format!(
                    "Plan item {plan_item_ref:?} is already linked to Task {}.",
                    self.task_id(existing_task_index)
                ));
            }
            drop(tx);
            return self.replay_atomic_task_start(
                &binding,
                &plan_item_ref,
                &idempotency_key,
                task_payload,
                change_payload,
                existing_task_index,
            );
        }
        let (task_index, task_id, change_index) = {
            let workflow_tx = tx.workflow_write()?;
            let (task_index, task_id) = self.append_task_in_write(
                workflow_tx,
                task_payload,
                revision_plus1,
                item_plus1,
                now,
                now,
                PayloadSyncBoundary::BeforeReference,
            )?;
            let (change_index, _) = self.append_change_in_write(
                workflow_tx,
                change_payload,
                task_index,
                &task_id,
                now,
            )?;
            (task_index, task_id, change_index)
        };
        tx.set_commit_point(ServerPlanBinaryDbCommitPoint::TaskStarted {
            plan_index: binding.plan_index,
            revision_index: binding.revision_index,
            task_index,
            change_index,
        })?;
        tx.commit()?;
        debug_assert_eq!(task_id, self.task_id(task_index));
        self.atomic_task_start_result(
            &binding,
            &plan_item_ref,
            &idempotency_key,
            task_index,
            change_index,
            false,
        )
    }

    fn create_task(&self, repo_name: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let operation = "ServerWorkflowTaskStore::create_task";
        self.repo_scope(operation, repo_name)?;
        let payload = Self::required_object(payload, "task create payload")?;
        let (revision_plus1, item_plus1, linked_at_s) =
            self.resolve_plan_binding_for_create(payload)?;
        let now = Self::now_s()?;
        let task_index = {
            let mut tx =
                BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                    .map_err(|error| Self::error(operation, error))?;
            let (task_index, _) = self.append_task_in_write(
                &mut tx,
                payload,
                revision_plus1,
                item_plus1,
                linked_at_s,
                now,
                PayloadSyncBoundary::BeforeReference,
            )?;
            tx.commit().map_err(|error| Self::error(operation, error))?;
            task_index
        };
        let read = BinaryDbReadTxn::new(&self.db);
        self.task_at(&read, task_index)
    }

    fn list_tasks(&self, repo_name: &str) -> Result<JsonValue, String> {
        self.repo_scope("ServerWorkflowTaskStore::list_tasks", repo_name)?;
        let read = BinaryDbReadTxn::new(&self.db);
        let count = read
            .record_count(WorkflowBinaryV0Codec::task_file())
            .map_err(|error| Self::error("ServerWorkflowTaskStore::list_tasks", error))?;
        let mut rows = Vec::new();
        for index in 0..count {
            let record = self
                .read_task(&read, index)
                .map_err(|error| Self::error("ServerWorkflowTaskStore::list_tasks", error))?;
            if record.remote_meta & 1 == 0 {
                rows.push(self.task_at(&read, index)?);
            }
        }
        rows.reverse();
        Ok(JsonValue::Array(rows))
    }

    fn get_task(&self, repo_name: Option<&str>, task_ref: &str) -> Result<JsonValue, String> {
        if let Some(repo_name) = repo_name {
            self.repo_scope("ServerWorkflowTaskStore::get_task", repo_name)?;
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let index = self.task_index_for_id(&read, task_ref)?;
        self.task_at(&read, index)
    }

    fn close_task(&self, task_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let operation = "ServerWorkflowTaskStore::close_task";
        let payload = Self::required_object(payload, "task close payload")?;
        let status = Self::required_text(payload, "status")?;
        let terminal_bit = match status.as_str() {
            "completed" => TASK_META_COMPLETED,
            "abandoned" | "canceled" | "cancelled" => TASK_META_CANCELED,
            _ => {
                return Err(format!(
                    "Binary DB v0 Task close does not support status {status:?}"
                ))
            }
        };
        let now = Self::now_s()?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let task_index = self.task_index_for_id(&tx, task_id)?;
        if terminal_bit == TASK_META_COMPLETED {
            let changed = self.complete_task_in_write(&mut tx, task_index, now)?;
            if changed {
                tx.commit().map_err(|error| Self::error(operation, error))?;
            } else {
                drop(tx);
            }
            let read = BinaryDbReadTxn::new(&self.db);
            return self.task_at(&read, task_index);
        }
        let mut record = self
            .read_task(&tx, task_index)
            .map_err(|error| Self::error(operation, error))?;
        if record.task_meta & terminal_bit != 0 {
            drop(tx);
            let read = BinaryDbReadTxn::new(&self.db);
            return self.task_at(&read, task_index);
        }
        if record.is_terminal() {
            return Err(format!("Task {task_id} is already terminal"));
        }
        let mut dependency = WorkflowBinaryV0Codec::encode_task(record)
            .map_err(|error| Self::error(operation, error))?;
        dependency[52..60].copy_from_slice(&now.to_le_bytes());
        tx.overwrite_record(WorkflowBinaryV0Codec::task_file(), task_index, &dependency)
            .map_err(|error| Self::error(operation, error))?;
        self.sync_file(&tx, "task.bin")
            .map_err(|error| Self::error(operation, error))?;
        record.closed_at_s = now;
        record.updated_at_s = now;
        record.task_meta |= terminal_bit;
        let committed = WorkflowBinaryV0Codec::encode_task(record)
            .map_err(|error| Self::error(operation, error))?;
        tx.overwrite_record(WorkflowBinaryV0Codec::task_file(), task_index, &committed)
            .map_err(|error| Self::error(operation, error))?;
        tx.commit().map_err(|error| Self::error(operation, error))?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.task_at(&read, task_index)
    }

    fn read_task_audit(
        &self,
        repo_name: &str,
        task_ref: &str,
        target_line: &str,
    ) -> Result<JsonValue, String> {
        self.repo_scope("ServerWorkflowTaskStore::read_task_audit", repo_name)?;
        let read = BinaryDbReadTxn::new(&self.db);
        let task_index = self.task_index_for_id(&read, task_ref)?;
        let task = self.task_at(&read, task_index)?;
        let change_count = read
            .record_count(WorkflowBinaryV0Codec::change_file())
            .map_err(|error| Self::error("ServerWorkflowTaskStore::read_task_audit", error))?;
        let mut changes = Vec::new();
        for index in 0..change_count {
            let record = self
                .read_change(&read, index)
                .map_err(|error| Self::error("ServerWorkflowTaskStore::read_task_audit", error))?;
            if record.remote_meta & 1 == 0 && record.task_index == task_index {
                changes.push(self.change_at(&read, index)?);
            }
        }
        let landed = changes
            .iter()
            .filter(|row| row.get("status").and_then(JsonValue::as_str) == Some("landed"))
            .count();
        let open = changes.len() - landed;
        let task_status = task.get("status").and_then(JsonValue::as_str);
        let verdict = match task_status {
            Some("completed") => "task_completed",
            Some("abandoned" | "canceled" | "cancelled") => "task_abandoned",
            _ if open == 0 && landed > 0 => "ready_to_close",
            _ if changes.is_empty() => "no_changes",
            _ => "in_progress",
        };
        let lines =
            ServerBinaryDbLineStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let target_head = lines
            .line_by_name(&read, target_line)
            .map_err(|error| Self::error("Task audit target Line", error))?
            .and_then(|(_, line)| line.head_snapshot_index())
            .map(|index| {
                ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(
                    self.db.clone(),
                )
                .snapshot_id_at(&read, index)
                .map_err(|error| Self::error("Task audit target Snapshot", error))
            })
            .transpose()?;
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_ref,
            "task": task,
            "target_line": target_line,
            "target_line_head": target_head,
            "changes": changes,
            "summary": {
                "verdict": verdict,
                "total_changes": landed + open,
                "landed_changes": landed,
                "open_changes": open,
            },
            "verdict": { "code": verdict, "status": verdict },
        }))
    }
}

impl<D> ServerWorkflowChangeStore for BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn create_change(&self, repo_name: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let operation = "ServerWorkflowChangeStore::create_change";
        self.repo_scope(operation, repo_name)?;
        let payload = Self::required_object(payload, "change create payload")?;
        let task_id = Self::required_text(payload, "task_id")?;
        let now = Self::now_s()?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let task_index = self.task_index_for_id(&tx, &task_id)?;
        let (change_index, _) =
            self.append_change_in_write(&mut tx, payload, task_index, &task_id, now)?;
        tx.commit().map_err(|error| Self::error(operation, error))?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.change_at(&read, change_index)
    }

    fn list_changes(&self, repo_name: &str) -> Result<JsonValue, String> {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.workflow_v0.list_changes");
        self.repo_scope("ServerWorkflowChangeStore::list_changes", repo_name)?;
        let read = BinaryDbReadTxn::new(&self.db);
        let count = read
            .record_count(WorkflowBinaryV0Codec::change_file())
            .map_err(|error| Self::error("ServerWorkflowChangeStore::list_changes", error))?;
        let latest_succeeded_lands = self.latest_succeeded_lands(
            &read,
            "ait.server.workflow_v0.list_changes.latest_succeeded_lands",
        )?;
        let mut rows = Vec::new();
        for index in 0..count {
            let record = self
                .read_change(&read, index)
                .map_err(|error| Self::error("ServerWorkflowChangeStore::list_changes", error))?;
            if record.remote_meta & 1 == 0 {
                rows.push(self.change_at_with_precomputed_latest_success(
                    &read,
                    index,
                    Some(latest_succeeded_lands.get(&index).copied()),
                )?);
            }
        }
        rows.reverse();
        Ok(JsonValue::Array(rows))
    }

    fn get_change(&self, repo_name: Option<&str>, change_ref: &str) -> Result<JsonValue, String> {
        if let Some(repo_name) = repo_name {
            self.repo_scope("ServerWorkflowChangeStore::get_change", repo_name)?;
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let index = self.change_index_for_ref(&read, change_ref)?;
        self.change_at(&read, index)
    }

    fn close_change(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let operation = "ServerWorkflowChangeStore::close_change";
        let payload = Self::required_object(payload, "change close payload")?;
        let status = Self::required_text(payload, "status")?;
        let (canceled, superseded) =
            match status.as_str() {
                "archived" => (false, false),
                "abandoned" | "canceled" | "cancelled" => (true, false),
                "superseded" => (false, true),
                "landed" => return Err(
                    "Binary DB v0 landed Change state is derived from successful Land authority"
                        .to_string(),
                ),
                _ => {
                    return Err(format!(
                        "unsupported Binary DB v0 Change close status {status:?}"
                    ))
                }
            };
        let now = Self::now_s()?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let index = self.change_index_for_ref(&tx, change_id)?;
        let mut record = self
            .read_change(&tx, index)
            .map_err(|error| Self::error(operation, error))?;
        if record.lifecycle() == CHANGE_LIFECYCLE_ARCHIVED
            && (record.change_state & CHANGE_STATE_CANCELED != 0) == canceled
            && (record.change_meta & CHANGE_META_SUPERSEDED != 0) == superseded
        {
            drop(tx);
            let read = BinaryDbReadTxn::new(&self.db);
            return self.change_at(&read, index);
        }
        if record.lifecycle() == CHANGE_LIFECYCLE_LANDED {
            return Err(format!("Change {change_id} is already landed"));
        }
        record.change_meta =
            (record.change_meta & !CHANGE_META_LIFECYCLE_MASK) | CHANGE_LIFECYCLE_ARCHIVED;
        record.change_meta &= !CHANGE_META_SUPERSEDED;
        if superseded {
            record.change_meta |= CHANGE_META_SUPERSEDED;
        }
        record.change_state = if canceled { CHANGE_STATE_CANCELED } else { 0 };
        record.updated_at_s = now;
        record.archived_at_s = now;
        let raw = WorkflowBinaryV0Codec::encode_change(record)
            .map_err(|error| Self::error(operation, error))?;
        tx.overwrite_record(WorkflowBinaryV0Codec::change_file(), index, &raw)
            .map_err(|error| Self::error(operation, error))?;
        tx.commit().map_err(|error| Self::error(operation, error))?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.change_at(&read, index)
    }
}
