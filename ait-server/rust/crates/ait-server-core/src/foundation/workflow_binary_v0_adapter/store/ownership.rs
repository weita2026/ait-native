use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct WorkflowOwner {
    pub task_index: u32,
    pub change: Option<(u32, V0ChangeRecord)>,
}

impl WorkflowOwner {
    pub fn change_ordinal_plus1(self) -> u8 {
        self.change
            .map_or(0, |(_, change)| change.change_ordinal + 1)
    }
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub(super) fn workflow_owner<A: ReadV0>(
        &self,
        read: &A,
        task_or_change_ref: &str,
    ) -> Result<WorkflowOwner, String> {
        if task_or_change_ref.contains("/C-") {
            let index = self.change_index_for_ref(read, task_or_change_ref)?;
            let change = self.read_change(read, index).map_err(|e| e.to_string())?;
            Ok(WorkflowOwner {
                task_index: change.task_index,
                change: Some((index, change)),
            })
        } else {
            Ok(WorkflowOwner {
                task_index: self.task_index_for_id(read, task_or_change_ref)?,
                change: None,
            })
        }
    }

    pub(super) fn patchset_owner<A: ReadV0>(
        &self,
        read: &A,
        patchset: V0PatchsetRecord,
    ) -> Result<WorkflowOwner, String> {
        self.task_index_for_id(read, &self.task_id(patchset.task_index))?;
        let change = patchset
            .change_ordinal_plus1
            .checked_sub(1)
            .map(|ordinal| {
                let index = self
                    .change_index_for_ref(read, &self.change_ref(patchset.task_index, ordinal))?;
                let change = self.read_change(read, index).map_err(|e| e.to_string())?;
                Ok::<_, String>((index, change))
            })
            .transpose()?;
        Ok(WorkflowOwner {
            task_index: patchset.task_index,
            change,
        })
    }

    pub(super) fn owner_ref(&self, owner: WorkflowOwner) -> String {
        owner.change.map_or_else(
            || self.task_id(owner.task_index),
            |(_, change)| self.change_ref(owner.task_index, change.change_ordinal),
        )
    }

    pub(super) fn patchset_public_id(&self, patchset: V0PatchsetRecord) -> String {
        let parent = patchset.change_ordinal_plus1.checked_sub(1).map_or_else(
            || self.task_id(patchset.task_index),
            |ordinal| self.change_ref(patchset.task_index, ordinal),
        );
        format!("{parent}/P-{:02}", patchset.patch_ordinal + 1)
    }

    pub(super) fn new_patchset_ref_matches(
        &self,
        reference: &str,
        owner: WorkflowOwner,
        ordinal: u8,
    ) -> bool {
        Self::parse_owned_ordinal(reference, &self.task_id(owner.task_index), "P").ok()
            == Some(ordinal)
            || Self::parse_owned_ordinal(reference, &self.owner_ref(owner), "P").ok()
                == Some(ordinal)
    }

    pub(super) fn require_patchset_owner<A: ReadV0>(
        &self,
        read: &A,
        requested: &str,
        patchset: V0PatchsetRecord,
    ) -> Result<WorkflowOwner, String> {
        let expected = self.workflow_owner(read, requested)?;
        let actual = self.patchset_owner(read, patchset)?;
        if expected.task_index != actual.task_index
            || expected.change.is_some()
                && expected.change_ordinal_plus1() != actual.change_ordinal_plus1()
        {
            return Err("Patchset belongs to a different Task or explicit Change".to_string());
        }
        Ok(actual)
    }

    pub(super) fn task_inventory<A: ReadV0>(
        &self,
        read: &A,
        path: &'static str,
        task_index: u32,
    ) -> Result<V0InventoryIndexRecord, String> {
        WorkflowBinaryV0Codec::decode_inventory_index(
            &read
                .read_record_v0(WorkflowBinaryV0Codec::chain_index_file(path), task_index)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }

    pub(super) fn current_task_patchset<A: ReadV0>(
        &self,
        read: &A,
        task_index: u32,
    ) -> Result<Option<(u32, V0PatchsetRecord)>, String> {
        let inventory = self.task_inventory(read, "task_patchset_index.bin", task_index)?;
        let Some(index) = inventory.latest_index_plus1.checked_sub(1) else {
            if inventory.count != 0 {
                return Err("Task Patchset inventory is incomplete".to_string());
            }
            return Ok(None);
        };
        let patchset = self.read_patchset(read, index).map_err(|e| e.to_string())?;
        if inventory.count == 0 || patchset.task_index != task_index {
            return Err("Task latest Patchset has invalid ownership".to_string());
        }
        self.patchset_owner(read, patchset)?;
        Ok(Some((index, patchset)))
    }

    pub(super) fn require_current_patchset<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
        patchset: V0PatchsetRecord,
    ) -> Result<(), String> {
        if self
            .current_task_patchset(read, patchset.task_index)?
            .map(|(i, _)| i)
            != Some(index)
            || patchset.patchset_meta & 0b11 != 0
            || patchset.patchset_meta & PATCHSET_PUBLISH_STATE_MASK != 0
        {
            return Err(
                "EXPECTED_CANDIDATE_MISMATCH: Patchset is not the valid current Task candidate"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn next_task_patchset_ordinal<A: ReadV0>(
        &self,
        read: &A,
        task_index: u32,
    ) -> Result<u8, String> {
        let next = self.allocated_task_patchset_ordinals(read, task_index)?;
        if next >= 64 {
            return Err("Task has exhausted its Patchset ordinals".into());
        }
        Ok(next)
    }

    pub(super) fn allocated_task_patchset_ordinals<A: ReadV0>(
        &self,
        read: &A,
        task_index: u32,
    ) -> Result<u8, String> {
        let inventory = self.task_inventory(read, "task_patchset_index.bin", task_index)?;
        let mut cursor = inventory.latest_index_plus1.checked_sub(1);
        let mut seen = BTreeSet::new();
        let mut ordinals = BTreeSet::new();
        while let Some(index) = cursor {
            if !seen.insert(index) {
                return Err("Task Patchset inventory has a cycle".to_string());
            }
            let patchset = self.read_patchset(read, index).map_err(|e| e.to_string())?;
            self.patchset_owner(read, patchset)?;
            if patchset.task_index != task_index || !ordinals.insert(patchset.patch_ordinal) {
                return Err("Task Patchset owner or allocated ordinal is invalid".to_string());
            }
            cursor = patchset.previous_task_patchset_index_plus1.checked_sub(1);
            if cursor.is_some_and(|previous| previous >= index) {
                return Err("Task Patchset inventory is not in physical append order".to_string());
            }
        }
        if seen.len() != usize::from(inventory.count) {
            return Err("Task Patchset count disagrees".to_string());
        }
        let next = ordinals.last().map_or(0, |ordinal| ordinal + 1);
        Ok(next)
    }

    pub(super) fn task_successful_receipt<A: ReadV0>(
        &self,
        read: &A,
        task_index: u32,
    ) -> Result<Option<(u32, V0LandRecord)>, String> {
        let inventory = self.task_inventory(read, "task_land_index.bin", task_index)?;
        let mut cursor = inventory.latest_index_plus1.checked_sub(1);
        let mut seen = BTreeSet::new();
        let mut success = None;
        while let Some(index) = cursor {
            if !seen.insert(index) {
                return Err("Task Land inventory has a cycle".to_string());
            }
            let land = WorkflowBinaryV0Codec::decode_land(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::land_file(), index)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if land.task_index != task_index {
                return Err("Task Land inventory points at another Task".to_string());
            }
            if land.land_meta & LAND_TOMBSTONE == 0
                && land.land_meta & LAND_STATUS_MASK == LAND_STATUS_SUCCEEDED
            {
                let patchset = self
                    .read_patchset(read, land.patchset_index)
                    .map_err(|e| e.to_string())?;
                self.patchset_owner(read, patchset)?;
                if patchset.task_index != task_index
                    || patchset.change_ordinal_plus1 != land.change_ordinal_plus1
                {
                    return Err("Successful receipt candidate ownership disagrees".to_string());
                }
                if success.replace((index, land)).is_some() {
                    return Err("Task has multiple successful receipts; explicit historical disposition is required".to_string());
                }
            }
            cursor = land.previous_task_land_index_plus1.checked_sub(1);
            if cursor.is_some_and(|previous| previous >= index) {
                return Err("Task Land inventory is not in physical append order".to_string());
            }
        }
        if seen.len() != usize::from(inventory.count) {
            return Err("Task Land count disagrees".to_string());
        }
        Ok(success)
    }

    pub(super) fn land_at<A: ReadV0>(&self, read: &A, index: u32) -> Result<JsonValue, String> {
        let record = WorkflowBinaryV0Codec::decode_land(
            &read
                .read_record_v0(WorkflowBinaryV0Codec::land_file(), index)
                .map_err(|error| Self::error("Land read", error))?,
        )
        .map_err(|error| Self::error("Land decode", error))?;
        if record.land_meta & LAND_TOMBSTONE != 0 {
            return Err(format!("Unknown Land index {index}"));
        }
        let patchset = self
            .read_patchset(read, record.patchset_index)
            .map_err(|e| e.to_string())?;
        let owner = self.patchset_owner(read, patchset)?;
        if record.task_index != patchset.task_index
            || record.change_ordinal_plus1 != patchset.change_ordinal_plus1
        {
            return Err("Binary DB v0 Land ownership relation disagrees".to_string());
        }
        let line_raw = read
            .read_record_v0(
                crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::record_file(),
                record.target_line_index_plus1 - 1,
            )
            .map_err(|error| Self::error("Land target Line read", error))?;
        let line_record =
            crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::decode_record(
                &line_raw,
            )
            .map_err(|error| Self::error("Land target Line decode", error))?;
        let target_line = self.content_line_name(read, &line_record)?;
        let snapshot_id = |plus1: u32| -> Result<JsonValue, String> {
            plus1
                .checked_sub(1)
                .map(|snapshot_index| {
                    self.content_snapshot_id(read, snapshot_index)
                        .map(JsonValue::String)
                        .map_err(|error| format!("Land Snapshot read failed: {error}"))
                })
                .transpose()
                .map(|value| value.unwrap_or(JsonValue::Null))
        };
        let pre_target = snapshot_id(record.pre_land_target_snapshot_index_plus1)?;
        let landed_snapshot = snapshot_id(record.landed_snapshot_index_plus1)?;
        let base_snapshot = self
            .content_snapshot_id(read, patchset.base_snapshot_index)
            .map_err(|error| format!("Land base Snapshot read failed: {error}"))?;
        let revision_snapshot = self
            .content_snapshot_id(read, patchset.revision_snapshot_index)
            .map_err(|error| format!("Land revision Snapshot read failed: {error}"))?;
        let patchset_summary_raw = read
            .read_payload_v0(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                patchset.summary_offset,
                u32::from(patchset.summary_len),
            )
            .map_err(|error| Self::error("Land Patchset summary read", error))?;
        let patchset_summary = WorkflowBinaryV0Codec::decode_single_text_payload(
            &patchset_summary_raw,
            "Land Patchset summary",
        )
        .map_err(|error| Self::error("Land Patchset summary decode", error))?;
        let (source_kind, governance_authority) =
            history_promotion::source_kind_for_summary(patchset_summary);
        let status = match record.land_meta & LAND_STATUS_MASK {
            LAND_STATUS_QUEUED => "queued",
            LAND_STATUS_RUNNING => "running",
            LAND_STATUS_SUCCEEDED => "succeeded",
            LAND_STATUS_BLOCKED => "blocked",
            LAND_STATUS_FAILED => "failed",
            LAND_STATUS_CANCELED => "canceled",
            LAND_STATUS_UPDATING => "updating",
            _ => return Err("Binary DB v0 Land status is reserved".to_string()),
        };
        let mode = match (record.land_meta & LAND_MODE_MASK) >> 5 {
            LAND_MODE_DIRECT => "direct",
            LAND_MODE_MERGE => "merge",
            LAND_MODE_FF_ONLY => "ff-only",
            _ => return Err("Binary DB v0 Land mode is reserved".to_string()),
        };
        let failure = match record.failure_kind {
            0 => None,
            1 => Some("BASE_STALE"),
            2 => Some("POLICY_BLOCKED"),
            3 => Some("REVIEW_BLOCKED"),
            4 => Some("CI_BLOCKED"),
            5 => Some("CONFLICT"),
            6 => Some("TARGET_UPDATE_FAILED"),
            7 => Some("INTERNAL_ERROR"),
            _ => return Err("Binary DB v0 Land failure kind is reserved".to_string()),
        };
        let line_action = if status != "succeeded" {
            JsonValue::Null
        } else if landed_snapshot == pre_target
            && landed_snapshot.as_str() == Some(revision_snapshot.as_str())
        {
            json!("already_at_selected_patchset_revision")
        } else if landed_snapshot == pre_target {
            json!("already_contains_selected_patchset_revision")
        } else {
            json!("moved")
        };
        let result = if status == "succeeded" {
            json!({
                "landed_snapshot_id": landed_snapshot,
                "selected_revision_snapshot_id": revision_snapshot,
                "target_line": target_line,
                "base_snapshot_id": base_snapshot,
                "target_line_head": pre_target,
                "line_action": line_action,
                "current_head_contains_selected_revision": line_action == json!("already_contains_selected_patchset_revision"),
                "snapshot_action": if line_action == json!("already_contains_selected_patchset_revision") { "selected_patchset_revision_already_contained" } else { "selected_patchset_revision" },
            })
        } else {
            json!({
                "blocker_class": failure,
                "target_line": target_line,
                "target_line_head": pre_target,
                "expected_base_snapshot_id": base_snapshot,
                "selected_revision_snapshot_id": revision_snapshot,
                "current_head_contains_selected_revision": false,
            })
        };
        let change_ref = owner
            .change
            .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal));
        let patchset_id = self.patchset_public_id(patchset);
        Ok(json!({
            "submission_id": self.land_id(&self.task_id(owner.task_index), record.land_ordinal),
            "actor_key_index": record.actor_index_plus1.checked_sub(1),
            "repository_index": self.repository_index,
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "land_seq": index + 1,
            "task_id": self.task_id(owner.task_index),
            "change_id": owner.change.map(|(_, change)| format!("C-{:02}", change.change_ordinal + 1)),
            "change_ref": change_ref,
            "patchset_id": patchset_id,
            "base_snapshot_id": base_snapshot,
            "revision_snapshot_id": revision_snapshot,
            "pre_land_target_snapshot_id": pre_target,
            "target_line_index": record.target_line_index_plus1 - 1,
            "source_kind": source_kind,
            "governance_authority": governance_authority,
            "target_line": target_line,
            "mode": mode,
            "status": status,
            "failure_kind": failure,
            "result": result,
            "result_json": serde_json::to_string(&result)
                .map_err(|error| format!("failed to render derived Land result: {error}"))?,
            "landed_snapshot_id": landed_snapshot,
            "line_action": line_action,
            "created_at": Self::timestamp(record.submitted_at_s)?,
            "updated_at": Self::timestamp(record.updated_at_s)?,
        }))
    }

    pub(super) fn require_active_owner<A: ReadV0>(
        &self,
        read: &A,
        owner: WorkflowOwner,
    ) -> Result<(), String> {
        let task = self
            .read_task(read, owner.task_index)
            .map_err(|e| e.to_string())?;
        if task.is_terminal()
            || owner.change.is_some_and(|(_, change)| {
                change.change_state & CHANGE_STATE_CANCELED != 0
                    || matches!(
                        change.lifecycle(),
                        CHANGE_LIFECYCLE_LANDED | CHANGE_LIFECYCLE_ARCHIVED
                    )
            })
        {
            return Err("Task or explicit Change is terminal".to_string());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_owner_state<F, C>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        owner: WorkflowOwner,
        task_set: u8,
        task_clear: u8,
        now: u64,
        actor_index_plus1: Option<u32>,
        update_change: C,
    ) -> Result<(), String>
    where
        F: BinaryDbFsyncPolicy,
        C: FnOnce(&mut V0ChangeRecord),
    {
        let mut task = self
            .read_task(tx, owner.task_index)
            .map_err(|e| e.to_string())?;
        task.task_meta = (task.task_meta & !task_clear) | task_set;
        task.updated_at_s = task.updated_at_s.max(now);
        if let Some(actor) = actor_index_plus1 {
            task.actor_index_plus1 = actor;
        }
        tx.overwrite_record(
            WorkflowBinaryV0Codec::task_file(),
            owner.task_index,
            &WorkflowBinaryV0Codec::encode_task(task).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if let Some((index, mut change)) = owner.change {
            update_change(&mut change);
            change.updated_at_s = change.updated_at_s.max(now);
            if let Some(actor) = actor_index_plus1 {
                change.actor_index_plus1 = actor;
            }
            tx.overwrite_record(
                WorkflowBinaryV0Codec::change_file(),
                index,
                &WorkflowBinaryV0Codec::encode_change(change).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
