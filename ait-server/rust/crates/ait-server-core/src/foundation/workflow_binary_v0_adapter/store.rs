use crate::foundation::remote_binary_db::{
    binary_db_runtime_error, BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbFileFamily,
    BinaryDbFsyncPolicy, BinaryDbIndexAppender, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId,
    BinaryIndexId, BinaryPayloadFileId, ServerRemoteBinaryDb, StoreResult,
};
use crate::foundation::server_content_binary_db::{
    server_snapshot_id_from_hash48, ServerBinaryDbLineStore, ServerBinaryDbSnapshotStore,
    ServerBinaryLineCodec, ServerBinaryRepositoryContentStore, ServerBinarySnapshotCodec,
    ServerBinarySnapshotFileIdentity, ServerBinarySnapshotParentEdgeCodec,
    ServerBinarySnapshotRecord, SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use crate::foundation::server_plan_binary_db::{
    BinaryDbServerPlanService, ServerPlanBinaryDbCommitPoint,
};
use crate::foundation::server_workflow_store::{
    patchset_ci_trigger_requests_new_run, ServerWorkflowAttestationStore,
    ServerWorkflowChangeStore, ServerWorkflowLandStore, ServerWorkflowPatchsetStore,
    ServerWorkflowPolicyStore, ServerWorkflowReviewStore, ServerWorkflowStore,
    ServerWorkflowTaskStore,
};
use crate::foundation::workflow_binary_v0::*;
use chrono::{DateTime, Utc};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PatchsetCodecMode {
    Transitional,
    Frozen,
}

type TaskPlanProjection = (JsonValue, JsonValue, JsonValue, JsonValue, JsonValue);

#[derive(Clone, Debug)]
pub struct BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + Clone,
{
    db: D,
    task_prefix: String,
    patchset_codec: PatchsetCodecMode,
    // Immutable numeric routing authority supplied by the native Repository owner.
    repository_index: Option<u32>,
}

#[path = "store/actors.rs"]
mod actors;
#[path = "store/history_promotion.rs"]
mod history_promotion;
pub use actors::UserActorIdentity;
#[path = "store/ownership.rs"]
mod ownership;
#[path = "store/package_evidence.rs"]
mod package_evidence;
#[path = "store/patch_review.rs"]
mod patch_review;
#[path = "store/policy_land.rs"]
mod policy_land;
#[path = "store/snapshot_capture.rs"]
mod snapshot_capture;
#[path = "store/task_change.rs"]
mod task_change;
#[path = "store/task_land.rs"]
mod task_land;

use ownership::WorkflowOwner;

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub fn new(db: D) -> Self {
        Self {
            db,
            task_prefix: String::new(),
            patchset_codec: PatchsetCodecMode::Transitional,
            repository_index: None,
        }
    }

    pub fn new_remote(db: D, namespace_prefix: &str) -> Result<Self, String> {
        Self::new_remote_with_patchset_codec(db, namespace_prefix, PatchsetCodecMode::Transitional)
    }

    pub fn new_frozen(db: D) -> Self {
        Self {
            db,
            task_prefix: String::new(),
            patchset_codec: PatchsetCodecMode::Frozen,
            repository_index: None,
        }
    }

    pub fn new_remote_frozen(db: D, namespace_prefix: &str) -> Result<Self, String> {
        Self::new_remote_with_patchset_codec(db, namespace_prefix, PatchsetCodecMode::Frozen)
    }

    fn new_remote_with_patchset_codec(
        db: D,
        namespace_prefix: &str,
        patchset_codec: PatchsetCodecMode,
    ) -> Result<Self, String> {
        let namespace = namespace_prefix.trim().to_ascii_uppercase();
        if !namespace
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        {
            return Err(
                "Binary DB workflow id_namespace_prefix must contain only ASCII letters or digits"
                    .to_string(),
            );
        }
        let task_prefix = format!("R{namespace}");
        Ok(Self {
            db,
            task_prefix,
            patchset_codec,
            repository_index: None,
        })
    }

    pub fn db(&self) -> &D {
        &self.db
    }

    pub fn with_repository_index(mut self, repository_index: u32) -> Self {
        self.repository_index = Some(repository_index);
        self
    }

    pub fn origin_namespace_prefix(&self) -> &str {
        &self.task_prefix
    }

    pub fn physical_patchset_index(&self, patchset_id: &str) -> Result<u32, String> {
        let read = BinaryDbReadTxn::new(&self.db);
        self.patchset_index_for_id(&read, patchset_id)
    }

    pub fn into_arc(self) -> Arc<dyn ServerWorkflowStore>
    where
        D: BinaryDbIndexAppender + Send + Sync + 'static,
    {
        Arc::new(self)
    }

    fn error(operation: &str, error: BinaryDbError) -> String {
        binary_db_runtime_error(
            &format!("Binary DB v0 workflow adapter {operation} failed"),
            error,
        )
    }

    fn repo_scope(&self, operation: &str, repo_name: &str) -> Result<(), String> {
        if repo_name == self.db.repo_name().as_str() {
            Ok(())
        } else {
            Err(format!(
                "Binary DB v0 workflow adapter {operation} is bound to repository {}, not {repo_name}",
                self.db.repo_name().as_str()
            ))
        }
    }

    fn task_id(&self, task_index: u32) -> String {
        let sequence = task_index + 1;
        format!("{}T-{sequence:04}", self.task_prefix)
    }

    fn change_ref(&self, task_index: u32, ordinal: u8) -> String {
        format!("{}/C-{:02}", self.task_id(task_index), ordinal + 1)
    }

    fn current_change_patchset_identity<A: ReadV0>(
        &self,
        read: &A,
        change: V0ChangeRecord,
    ) -> Result<(Option<String>, Option<u64>), String> {
        let current = self
            .current_task_patchset(read, change.task_index)?
            .filter(|(_, patchset)| patchset.change_ordinal_plus1 == change.change_ordinal + 1);
        Ok(current.map_or((None, None), |(_, patchset)| {
            (
                Some(self.patchset_public_id(patchset)),
                Some(u64::from(patchset.patch_ordinal) + 1),
            )
        }))
    }

    fn review_id(&self, patchset_id: &str, ordinal: u8) -> String {
        format!("{patchset_id}/R-{:02}", ordinal + 1)
    }

    fn policy_id(&self, patchset_id: &str, ordinal: u8) -> String {
        format!("{patchset_id}/K-{:02}", ordinal + 1)
    }

    fn land_id(&self, change_ref: &str, ordinal: u8) -> String {
        format!("{change_ref}/L-{:02}", ordinal + 1)
    }

    fn attestation_id(&self, task_index: u32, ordinal: u8) -> String {
        format!("{}/A-{:02}", self.task_id(task_index), ordinal + 1)
    }

    fn now_s() -> Result<u64, String> {
        u64::try_from(Utc::now().timestamp())
            .map_err(|_| "system time precedes the Unix epoch".to_string())
            .and_then(|value| {
                if value == 0 {
                    Err("native Binary DB v0 writes require a non-zero timestamp".to_string())
                } else {
                    Ok(value)
                }
            })
    }

    fn timestamp(value: u64) -> Result<JsonValue, String> {
        if value == 0 {
            Ok(JsonValue::Null)
        } else {
            let seconds = i64::try_from(value)
                .map_err(|_| format!("Binary DB timestamp {value} exceeds RFC 3339 range"))?;
            DateTime::<Utc>::from_timestamp(seconds, 0)
                .map(|value| JsonValue::String(value.to_rfc3339()))
                .ok_or_else(|| format!("Binary DB timestamp {value} exceeds RFC 3339 range"))
        }
    }

    fn required_object<'a>(
        value: &'a JsonValue,
        label: &str,
    ) -> Result<&'a JsonMap<String, JsonValue>, String> {
        value
            .as_object()
            .ok_or_else(|| format!("{label} must be a JSON object"))
    }

    fn required_text(object: &JsonMap<String, JsonValue>, field: &str) -> Result<String, String> {
        object
            .get(field)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("{field} must be a non-empty string"))
    }

    fn optional_text(object: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
        object
            .get(field)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn optional_bool(object: &JsonMap<String, JsonValue>, field: &str) -> Option<bool> {
        object.get(field).and_then(JsonValue::as_bool)
    }

    fn parse_owned_ordinal(value: &str, parent: &str, kind: &str) -> Result<u8, String> {
        let prefix = format!("{parent}/{kind}-");
        let digits = value
            .strip_prefix(&prefix)
            .ok_or_else(|| format!("{value:?} is not owned by {parent}"))?;
        if digits.len() != 2 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("{value:?} is not a normalized {kind} identity"));
        }
        let ordinal = digits
            .parse::<u8>()
            .ok()
            .filter(|value| (1..=64).contains(value))
            .ok_or_else(|| format!("{value:?} has an invalid {kind} ordinal"))?;
        Ok(ordinal - 1)
    }

    fn parse_task_index(&self, task_id: &str) -> Result<u32, String> {
        let marker = format!("{}T-", self.task_prefix);
        let digits = task_id
            .strip_prefix(&marker)
            .ok_or_else(|| format!("{task_id:?} is not a Task in this repository namespace"))?;
        if digits.len() < 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("{task_id:?} is not a normalized Task identity"));
        }
        let sequence = digits
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("{task_id:?} has an invalid Task sequence"))?;
        Ok(sequence - 1)
    }

    fn read_task<A: ReadV0>(&self, read: &A, index: u32) -> StoreResult<V0TaskRecord> {
        WorkflowBinaryV0Codec::decode_task(
            &read.read_record_v0(WorkflowBinaryV0Codec::task_file(), index)?,
        )
    }

    fn read_change<A: ReadV0>(&self, read: &A, index: u32) -> StoreResult<V0ChangeRecord> {
        WorkflowBinaryV0Codec::decode_change(
            &read.read_record_v0(WorkflowBinaryV0Codec::change_file(), index)?,
        )
    }

    fn read_patchset<A: ReadV0>(&self, read: &A, index: u32) -> StoreResult<V0PatchsetRecord> {
        let raw = read.read_record_v0(WorkflowBinaryV0Codec::patchset_file(), index)?;
        self.decode_patchset(&raw)
    }

    fn decode_patchset(&self, raw: &[u8]) -> StoreResult<V0PatchsetRecord> {
        match self.patchset_codec {
            PatchsetCodecMode::Transitional => WorkflowBinaryV0Codec::decode_patchset(raw),
            PatchsetCodecMode::Frozen => WorkflowBinaryV0Codec::decode_frozen_patchset(raw)
                .map(Self::logical_patchset_from_frozen),
        }
    }

    fn logical_patchset_from_frozen(record: V0FrozenPatchsetRecord) -> V0PatchsetRecord {
        V0PatchsetRecord {
            patchset_meta: record.patchset_meta,
            patch_ordinal: record.patch_ordinal,
            change_ordinal_plus1: record.change_ordinal_plus1,
            reserved0: record.reserved0,
            task_index: record.task_index,
            previous_task_patchset_index_plus1: record.previous_task_patchset_index_plus1,
            previous_change_patchset_index_plus1: record.previous_change_patchset_index_plus1,
            base_snapshot_index: record.base_snapshot_index,
            revision_snapshot_index: record.revision_snapshot_index,
            created_at_s: record.created_at_s,
            ci_completed_at_s: record.ci_completed_at_s,
            ci_run_seq: record.ci_run_seq,
            ci_selected_suite_count: record.ci_selected_suite_count,
            ci_suite_result_count: record.ci_suite_result_count,
            ci_blocking_failure_count: record.ci_blocking_failure_count,
            ci_status_bits: record.ci_status_bits,
            summary_offset: record.summary_offset,
            summary_len: record.summary_len,
            ci_worker_job_index_plus1: record.ci_worker_job_index_plus1,
        }
    }

    fn frozen_patchset_from_logical(
        record: V0PatchsetRecord,
        ci_worker_job_index_plus1: u32,
    ) -> StoreResult<V0FrozenPatchsetRecord> {
        Ok(V0FrozenPatchsetRecord {
            patchset_meta: record.patchset_meta,
            patch_ordinal: record.patch_ordinal,
            change_ordinal_plus1: record.change_ordinal_plus1,
            reserved0: record.reserved0,
            task_index: record.task_index,
            previous_task_patchset_index_plus1: record.previous_task_patchset_index_plus1,
            previous_change_patchset_index_plus1: record.previous_change_patchset_index_plus1,
            base_snapshot_index: record.base_snapshot_index,
            revision_snapshot_index: record.revision_snapshot_index,
            created_at_s: record.created_at_s,
            ci_completed_at_s: record.ci_completed_at_s,
            ci_run_seq: record.ci_run_seq,
            ci_selected_suite_count: record.ci_selected_suite_count,
            ci_suite_result_count: record.ci_suite_result_count,
            ci_blocking_failure_count: record.ci_blocking_failure_count,
            ci_status_bits: record.ci_status_bits,
            summary_offset: record.summary_offset,
            summary_len: record.summary_len,
            ci_worker_job_index_plus1,
        })
    }

    fn encode_new_patchset(&self, record: V0PatchsetRecord) -> StoreResult<Vec<u8>> {
        match self.patchset_codec {
            PatchsetCodecMode::Transitional => WorkflowBinaryV0Codec::encode_patchset(record),
            PatchsetCodecMode::Frozen => WorkflowBinaryV0Codec::encode_frozen_patchset(
                Self::frozen_patchset_from_logical(record, 0)?,
            ),
        }
    }

    fn encode_patchset_replacement<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
        record: V0PatchsetRecord,
    ) -> StoreResult<Vec<u8>> {
        match self.patchset_codec {
            PatchsetCodecMode::Transitional => WorkflowBinaryV0Codec::encode_patchset(record),
            PatchsetCodecMode::Frozen => {
                let current = WorkflowBinaryV0Codec::decode_frozen_patchset(
                    &read.read_record_v0(WorkflowBinaryV0Codec::patchset_file(), index)?,
                )?;
                WorkflowBinaryV0Codec::encode_frozen_patchset(Self::frozen_patchset_from_logical(
                    record,
                    current.ci_worker_job_index_plus1,
                )?)
            }
        }
    }

    fn patchset_summary_is_valid<A: ReadV0>(read: &A, record: V0PatchsetRecord) -> StoreResult<()> {
        let summary = read.read_payload_v0(
            WorkflowBinaryV0Codec::patchset_summary_file(),
            record.summary_offset,
            u32::from(record.summary_len),
        )?;
        WorkflowBinaryV0Codec::decode_single_text_payload(&summary, "Patchset summary").map(|_| ())
    }

    pub fn repair_frozen_patchsets_for_activation(
        &self,
        expected_ci_worker_jobs: &BTreeMap<u32, u32>,
    ) -> Result<u32, String> {
        if self.patchset_codec != PatchsetCodecMode::Frozen {
            return Err(
                "Patchset activation repair requires the frozen Binary DB v0 codec".to_string(),
            );
        }
        let operation = "frozen Patchset locator normalization";
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let count = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| Self::error(operation, error))?;
        for (&patchset_index, &worker_job_index_plus1) in expected_ci_worker_jobs {
            if patchset_index >= count || worker_job_index_plus1 == 0 {
                return Err(format!(
                    "Binary DB v0 Patchset CI Job locator map contains an invalid row ({patchset_index}, {worker_job_index_plus1})"
                ));
            }
        }

        let mut repaired = 0_u32;
        for index in 0..count {
            let raw = tx
                .read_record(WorkflowBinaryV0Codec::patchset_file(), index)
                .map_err(|error| Self::error(operation, error))?;
            let expected_locator = expected_ci_worker_jobs.get(&index).copied().unwrap_or(0);
            let mut next = WorkflowBinaryV0Codec::decode_frozen_patchset(&raw)
                .map_err(|error| Self::error(operation, error))?;
            Self::patchset_summary_is_valid(&tx, Self::logical_patchset_from_frozen(next))
                .map_err(|error| Self::error(operation, error))?;
            if next.ci_worker_job_index_plus1 > expected_locator {
                return Err(format!(
                    "Binary DB v0 Patchset {index} selects missing Worker Job {}",
                    next.ci_worker_job_index_plus1 - 1
                ));
            }
            next.ci_worker_job_index_plus1 = expected_locator;
            let next_raw = WorkflowBinaryV0Codec::encode_frozen_patchset(next)
                .map_err(|error| Self::error(operation, error))?;
            if next_raw != raw {
                tx.overwrite_record(WorkflowBinaryV0Codec::patchset_file(), index, &next_raw)
                    .map_err(|error| Self::error(operation, error))?;
                repaired = repaired
                    .checked_add(1)
                    .ok_or_else(|| "Patchset repair count exceeds u32".to_string())?;
            }
        }
        tx.commit().map_err(|error| Self::error(operation, error))?;
        Ok(repaired)
    }

    fn task_at(&self, read: &BinaryDbReadTxn<'_, D>, task_index: u32) -> Result<JsonValue, String> {
        let record = self
            .read_task(read, task_index)
            .map_err(|error| Self::error("Task read", error))?;
        let projection = self.task_plan_projection(read, record)?;
        self.task_at_with_projection(read, task_index, record, projection)
    }

    fn task_at_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        task_index: u32,
    ) -> Result<JsonValue, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let record = self
            .read_task(write, task_index)
            .map_err(|error| Self::error("Task read", error))?;
        let projection = self.task_plan_projection_in_write(write, record)?;
        self.task_at_with_projection(write, task_index, record, projection)
    }

    fn task_at_with_projection<A: ReadV0>(
        &self,
        read: &A,
        task_index: u32,
        record: V0TaskRecord,
        projection: TaskPlanProjection,
    ) -> Result<JsonValue, String> {
        if record.remote_meta & 1 != 0 {
            return Err(format!("Unknown task: {}", self.task_id(task_index)));
        }
        let raw = read
            .read_payload_v0(
                WorkflowBinaryV0Codec::task_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| Self::error("Task payload read", error))?;
        let payload = WorkflowBinaryV0Codec::decode_task_payload(&raw)
            .map_err(|error| Self::error("Task payload decode", error))?;
        let (plan_id, revision_id, item_ref, section_ref, drift_state) = projection;
        let status = if record.task_meta & TASK_META_CANCELED != 0 {
            "abandoned"
        } else if record.task_meta & TASK_META_COMPLETED != 0 {
            "completed"
        } else {
            "active"
        };
        let workflow_state = if record.is_terminal() {
            status
        } else if record.task_meta & TASK_META_BLOCKED != 0 {
            "attention_required"
        } else if record.task_meta & TASK_META_REVIEW_PENDING != 0 {
            "in_review"
        } else if record.task_meta & TASK_META_VALIDATION_PENDING != 0 {
            "validation_pending"
        } else if record.task_meta & TASK_META_READY_TO_LAND != 0 {
            "ready_to_land"
        } else {
            "in_progress"
        };
        let current_patchset_id = self
            .current_task_patchset(read, task_index)?
            .map(|(_, patchset)| self.patchset_public_id(patchset));
        let successful_land = self
            .task_successful_receipt(read, task_index)?
            .map(|(index, _)| self.land_at(read, index))
            .transpose()?;
        Ok(json!({
            "task_id": self.task_id(task_index),
            "repository_index": self.repository_index,
            "actor_key_index": record.actor_index_plus1.checked_sub(1),
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "task_seq": task_index + 1,
            "current_patchset_id": current_patchset_id,
            "successful_land": successful_land,
            "title": payload.title,
            "intent": payload.intent,
            "planning_state": if record.task_meta & TASK_META_PLANNED != 0 { "planned" } else { "unplanned" },
            "plan_id": plan_id,
            "origin_plan_revision_id": revision_id,
            "plan_item_ref": item_ref,
            "plan_section_ref": section_ref,
            "plan_drift_state": drift_state,
            "plan_linked_at": Self::timestamp(record.plan_linked_at_s)?,
            "status": status,
            "workflow_state": workflow_state,
            "created_at": Self::timestamp(record.created_at_s)?,
            "updated_at": Self::timestamp(record.updated_at_s)?,
            "closed_at": Self::timestamp(record.closed_at_s)?,
        }))
    }

    fn task_plan_projection(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        task: V0TaskRecord,
    ) -> Result<TaskPlanProjection, String> {
        let Some(revision_index) = task.origin_plan_revision_index_plus1.checked_sub(1) else {
            return Ok((
                JsonValue::Null,
                JsonValue::Null,
                JsonValue::Null,
                JsonValue::Null,
                JsonValue::Null,
            ));
        };
        let item_index = task.plan_item_index_plus1.checked_sub(1).ok_or_else(|| {
            "Binary DB v0 Task has an incomplete Plan revision/item binding".to_string()
        })?;
        let (plan_index, item_ref, heading_path) = BinaryDbServerPlanService::new(self.db.clone())
            .task_binding_projection_with_read(read, revision_index, item_index)?;
        let plan_id = format!("PR-{plan_index}");
        let revision_id = format!("plan-revision:{revision_index}");
        let section = (!heading_path.is_empty()).then(|| heading_path.join(" / "));
        Ok((
            json!(plan_id),
            json!(revision_id),
            json!(item_ref),
            section.map(JsonValue::String).unwrap_or(JsonValue::Null),
            JsonValue::Null,
        ))
    }

    fn task_plan_projection_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        task: V0TaskRecord,
    ) -> Result<TaskPlanProjection, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let Some(revision_index) = task.origin_plan_revision_index_plus1.checked_sub(1) else {
            return Ok((
                JsonValue::Null,
                JsonValue::Null,
                JsonValue::Null,
                JsonValue::Null,
                JsonValue::Null,
            ));
        };
        let item_index = task.plan_item_index_plus1.checked_sub(1).ok_or_else(|| {
            "Binary DB v0 Task has an incomplete Plan revision/item binding".to_string()
        })?;
        let (plan_index, item_ref, heading_path) = BinaryDbServerPlanService::new(self.db.clone())
            .task_binding_projection_in_write(write, revision_index, item_index)?;
        let plan_id = format!("PR-{plan_index}");
        let revision_id = format!("plan-revision:{revision_index}");
        let section = (!heading_path.is_empty()).then(|| heading_path.join(" / "));
        Ok((
            json!(plan_id),
            json!(revision_id),
            json!(item_ref),
            section.map(JsonValue::String).unwrap_or(JsonValue::Null),
            JsonValue::Null,
        ))
    }

    fn task_index_for_id<A: ReadV0>(&self, read: &A, task_id: &str) -> Result<u32, String> {
        let index = self.parse_task_index(task_id)?;
        let count = read
            .record_count_v0(WorkflowBinaryV0Codec::task_file())
            .map_err(|error| Self::error("Task identity read", error))?;
        if index >= count {
            return Err(format!("Unknown task: {task_id}"));
        }
        let record = self
            .read_task(read, index)
            .map_err(|error| Self::error("Task identity read", error))?;
        if record.remote_meta & 1 != 0 {
            return Err(format!("Unknown task: {task_id}"));
        }
        Ok(index)
    }

    fn change_index_for_ref<A: ReadV0>(&self, read: &A, change_ref: &str) -> Result<u32, String> {
        let change_ref = change_ref.trim();
        let exact_owner = if let Some((task_id, _)) = change_ref.rsplit_once("/C-") {
            Some((
                self.task_index_for_id(read, task_id)?,
                Self::parse_owned_ordinal(change_ref, task_id, "C")?,
            ))
        } else {
            None
        };
        if let Some((task_index, ordinal)) = exact_owner {
            let inventory = self.owner_ordinal_index(read, "task_change_index.bin", task_index)?;
            let mut cursor = inventory.latest_index_plus1.checked_sub(1);
            let mut visited = BTreeSet::new();
            let mut found = None;
            while let Some(index) = cursor {
                if !visited.insert(index) {
                    return Err("Task Change inventory has a cycle".to_string());
                }
                let change = self.read_change(read, index).map_err(|e| e.to_string())?;
                if change.task_index != task_index {
                    return Err("Task Change inventory has a foreign owner".to_string());
                }
                if change.change_ordinal == ordinal
                    && change.remote_meta & 1 == 0
                    && found.replace(index).is_some()
                {
                    return Err(format!(
                        "Duplicate Binary DB v0 Change identity: {change_ref}"
                    ));
                }
                cursor = change.previous_change_index_plus1.checked_sub(1);
            }
            if visited.len() != usize::from(inventory.count) {
                return Err("Task Change inventory count disagrees".to_string());
            }
            return found.ok_or_else(|| format!("Unknown change: {change_ref}"));
        }
        let short_ordinal = if exact_owner.is_none() {
            let digits = change_ref
                .strip_prefix("C-")
                .ok_or_else(|| format!("{change_ref:?} is not a normalized Change identity"))?;
            if digits.len() != 2 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(format!(
                    "{change_ref:?} is not a normalized Change identity"
                ));
            }
            Some(
                digits
                    .parse::<u8>()
                    .ok()
                    .filter(|value| (1..=64).contains(value))
                    .map(|value| value - 1)
                    .ok_or_else(|| format!("{change_ref:?} has an invalid Change ordinal"))?,
            )
        } else {
            None
        };
        let count = read
            .record_count_v0(WorkflowBinaryV0Codec::change_file())
            .map_err(|error| Self::error("Change identity read", error))?;
        let mut found: Option<(u32, V0ChangeRecord)> = None;
        for index in 0..count {
            let record = self
                .read_change(read, index)
                .map_err(|error| Self::error("Change identity read", error))?;
            let matches = record.remote_meta & 1 == 0
                && match exact_owner {
                    Some((task_index, ordinal)) => {
                        record.task_index == task_index && record.change_ordinal == ordinal
                    }
                    None => Some(record.change_ordinal) == short_ordinal,
                };
            if matches {
                if let Some((_, previous)) = found {
                    if previous.task_index != record.task_index {
                        return Err(format!(
                            "Ambiguous short Change selector {change_ref:?}: Tasks {} and {} both own {change_ref}; use a contextual <task-id>/{change_ref} change_ref",
                            self.task_id(previous.task_index),
                            self.task_id(record.task_index),
                        ));
                    }
                    return Err(format!(
                        "Duplicate Binary DB v0 Change identity: {change_ref}"
                    ));
                }
                found = Some((index, record));
            }
        }
        found
            .map(|(index, _)| index)
            .ok_or_else(|| format!("Unknown change: {change_ref}"))
    }

    fn patchset_index_for_id<A: ReadV0>(&self, read: &A, patchset_id: &str) -> Result<u32, String> {
        let (parent, _) = patchset_id
            .rsplit_once("/P-")
            .ok_or_else(|| format!("Invalid Patchset identity {patchset_id:?}"))?;
        let owner = self.workflow_owner(read, parent)?;
        let ordinal = Self::parse_owned_ordinal(patchset_id, parent, "P")?;
        let inventory = self.task_inventory(read, "task_patchset_index.bin", owner.task_index)?;
        let mut cursor = inventory.latest_index_plus1.checked_sub(1);
        let mut seen = BTreeSet::new();
        let mut ordinals = BTreeSet::new();
        let mut found = None;
        while let Some(index) = cursor {
            if !seen.insert(index) {
                return Err("Task Patchset inventory has a cycle".to_string());
            }
            let patchset = self.read_patchset(read, index).map_err(|e| e.to_string())?;
            if patchset.task_index != owner.task_index || !ordinals.insert(patchset.patch_ordinal) {
                return Err("Task Patchset owner or allocated ordinal disagrees".to_string());
            }
            self.patchset_owner(read, patchset)?;
            if patchset.patch_ordinal == ordinal
                && (owner.change.is_none() || self.patchset_public_id(patchset) == patchset_id)
            {
                found = Some(index);
            }
            cursor = patchset.previous_task_patchset_index_plus1.checked_sub(1);
            if cursor.is_some_and(|previous| previous >= index) {
                return Err("Task Patchset inventory is not in physical append order".to_string());
            }
        }
        if seen.len() != usize::from(inventory.count) {
            return Err("Task Patchset count disagrees".to_string());
        }
        found.ok_or_else(|| format!("Unknown Patchset {patchset_id}"))
    }

    fn content_line_name<A: ReadV0>(
        &self,
        read: &A,
        record: &crate::foundation::server_content_binary_db::ServerBinaryLineRecord,
    ) -> Result<String, String> {
        let raw = read
            .read_payload_v0(
                ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::payload_file(),
                record.line_name_offset,
                u32::from(record.line_name_len),
            )
            .map_err(|error| Self::error("Line name read", error))?;
        String::from_utf8(raw).map_err(|error| format!("Line name is not UTF-8: {error}"))
    }

    fn content_snapshot_id<A: ReadV0>(
        &self,
        read: &A,
        snapshot_index: u32,
    ) -> Result<String, String> {
        let raw = read
            .read_record_v0(
                ServerBinarySnapshotCodec::<2>::record_file(),
                snapshot_index,
            )
            .map_err(|error| Self::error("Snapshot read", error))?;
        let record = ServerBinarySnapshotCodec::<2>::decode_record(&raw)
            .map_err(|error| Self::error("Snapshot decode", error))?;
        if record.is_tombstone() {
            return Err(format!("Snapshot index {snapshot_index} is tombstoned"));
        }
        Ok(server_snapshot_id_from_hash48(record.snapshot_hash48))
    }

    fn change_at<A: ReadV0>(&self, read: &A, index: u32) -> Result<JsonValue, String> {
        self.change_at_with_precomputed_latest_success(read, index, None)
    }

    fn change_at_with_precomputed_latest_success<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
        precomputed_latest_success: Option<Option<(u32, V0LandRecord)>>,
    ) -> Result<JsonValue, String> {
        let record = self
            .read_change(read, index)
            .map_err(|error| Self::error("Change read", error))?;
        if record.remote_meta & 1 != 0 {
            return Err("Unknown change".to_string());
        }
        let title_raw = read
            .read_payload_v0(
                WorkflowBinaryV0Codec::change_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| Self::error("Change payload read", error))?;
        let title = WorkflowBinaryV0Codec::decode_single_text_payload(&title_raw, "Change title")
            .map_err(|error| Self::error("Change payload decode", error))?;
        let line_record = read
            .read_record_v0(
                crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::record_file(),
                record.base_line_index_plus1 - 1,
            )
            .map_err(|error| Self::error("Change base Line read", error))?;
        let line_record =
            crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::decode_record(
                &line_record,
            )
            .map_err(|error| Self::error("Change base Line decode", error))?;
        let base_line = self.content_line_name(read, &line_record)?;
        let fork_snapshot_id = record
            .fork_snapshot_index_plus1
            .checked_sub(1)
            .map(|snapshot_index| {
                self.content_snapshot_id(read, snapshot_index)
                    .map_err(|error| format!("Change fork Snapshot read failed: {error}"))
            })
            .transpose()?;
        let (current_patchset_id, current_patchset_number) =
            self.current_change_patchset_identity(read, record)?;
        let current_patchset_number = current_patchset_number.unwrap_or(0);
        let latest_success = match precomputed_latest_success {
            Some(latest_success) => latest_success,
            None => self.latest_succeeded_land(read, index)?,
        };
        let (target_line, landed_at, landed_snapshot_id) = match latest_success {
            Some((_land_index, land)) => {
                let line_raw_record = read.read_record_v0(
                    crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::record_file(),
                    land.target_line_index_plus1 - 1,
                ).map_err(|error| Self::error("Land target Line read", error))?;
                let line_record = crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::decode_record(&line_raw_record)
                    .map_err(|error| Self::error("Land target Line decode", error))?;
                let landed_snapshot_index = land
                    .landed_snapshot_index_plus1
                    .checked_sub(1)
                    .ok_or_else(|| {
                        "Binary DB v0 successful Land is missing its landed Snapshot reference"
                            .to_string()
                    })?;
                (
                    json!(self.content_line_name(read, &line_record)?),
                    Self::timestamp(land.updated_at_s)?,
                    json!(self.content_snapshot_id(read, landed_snapshot_index)?),
                )
            }
            None => (JsonValue::Null, JsonValue::Null, JsonValue::Null),
        };
        let status = if record.change_state & CHANGE_STATE_CANCELED != 0 {
            "abandoned"
        } else {
            match record.lifecycle() {
                CHANGE_LIFECYCLE_DRAFT => "draft",
                CHANGE_LIFECYCLE_ACTIVE if record.change_meta & CHANGE_META_REVIEW_PENDING != 0 => {
                    "review"
                }
                CHANGE_LIFECYCLE_ACTIVE => "active",
                CHANGE_LIFECYCLE_LANDED => "landed",
                CHANGE_LIFECYCLE_ARCHIVED if record.change_meta & CHANGE_META_SUPERSEDED != 0 => {
                    "superseded"
                }
                CHANGE_LIFECYCLE_ARCHIVED => "archived",
                _ => return Err("Binary DB v0 Change lifecycle is reserved".to_string()),
            }
        };
        let change_ref = self.change_ref(record.task_index, record.change_ordinal);
        Ok(json!({
            "change_id": format!("C-{:02}", record.change_ordinal + 1),
            "change_ref": change_ref,
            "repo_name": self.db.repo_name().as_str(),
            "actor_key_index": record.actor_index_plus1.checked_sub(1),
            "repo_id": self.db.repo_id().as_str(),
            "task_id": self.task_id(record.task_index),
            "title": title,
            "base_line": base_line,
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": base_line,
            "status": status,
            "current_patchset_number": current_patchset_number,
            "current_patchset_id": current_patchset_id,
            "target_line": target_line,
            "landed_snapshot_id": landed_snapshot_id,
            "created_at": Self::timestamp(record.created_at_s)?,
            "updated_at": Self::timestamp(record.updated_at_s)?,
            "landed_at": landed_at,
            "archived_at": Self::timestamp(record.archived_at_s)?,
        }))
    }

    fn patchset_at(&self, read: &BinaryDbReadTxn<'_, D>, index: u32) -> Result<JsonValue, String> {
        let record = self
            .read_patchset(read, index)
            .map_err(|error| Self::error("Patchset read", error))?;
        let diff_stats = self.patchset_diff_stats(
            read,
            record.base_snapshot_index,
            record.revision_snapshot_index,
        )?;
        self.patchset_at_with_diff_stats(read, index, diff_stats)
    }

    fn patchset_at_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        index: u32,
    ) -> Result<JsonValue, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let record = self
            .read_patchset(write, index)
            .map_err(|error| Self::error("Patchset read", error))?;
        let diff_stats = self.patchset_diff_stats_in_write(
            write,
            record.base_snapshot_index,
            record.revision_snapshot_index,
        )?;
        self.patchset_at_with_diff_stats(write, index, diff_stats)
    }

    fn patchset_at_with_diff_stats<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
        diff_stats: JsonValue,
    ) -> Result<JsonValue, String> {
        let record = self
            .read_patchset(read, index)
            .map_err(|error| Self::error("Patchset read", error))?;
        let owner = self.patchset_owner(read, record)?;
        let summary_raw = read
            .read_payload_v0(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                record.summary_offset,
                u32::from(record.summary_len),
            )
            .map_err(|error| Self::error("Patchset summary read", error))?;
        let summary =
            WorkflowBinaryV0Codec::decode_single_text_payload(&summary_raw, "Patchset summary")
                .map_err(|error| Self::error("Patchset summary decode", error))?;
        let (source_kind, governance_authority) =
            history_promotion::source_kind_for_summary(summary);
        let base_snapshot_id = self
            .content_snapshot_id(read, record.base_snapshot_index)
            .map_err(|error| format!("Patchset base Snapshot read failed: {error}"))?;
        let revision_snapshot_id = self
            .content_snapshot_id(read, record.revision_snapshot_index)
            .map_err(|error| format!("Patchset revision Snapshot read failed: {error}"))?;
        let author_mode = match (record.patchset_meta & PATCHSET_AUTHOR_MODE_MASK) >> 2 {
            0 => "human_only",
            1 => "human_with_ai_assist",
            2 => "ai_with_human_review",
            3 => "ai_only_experimental",
            4 => "agent",
            5 => "codex",
            6 => "xhigh",
            _ => return Err("Binary DB v0 Patchset author mode is reserved".to_string()),
        };
        let publish_state = match (record.patchset_meta & PATCHSET_PUBLISH_STATE_MASK) >> 5 {
            0 => "published",
            2 => "superseded",
            _ => return Err("Binary DB v0 Patchset publish state is reserved".to_string()),
        };
        let evaluation_state = if record.patchset_meta & PATCHSET_EVALUATION_PENDING != 0 {
            "pending"
        } else {
            self.latest_policy_decision(read, index)?
                .map(|(_, policy)| policy_decision_name(policy.policy_meta & POLICY_DECISION_MASK))
                .transpose()?
                .ok_or_else(|| "non-pending Patchset has no live Policy Decision".to_string())?
        };
        let ci_status = |shift| ci_status_name(record.ci_status(shift));
        let change_ref = owner
            .change
            .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal));
        let patchset_id = self.patchset_public_id(record);
        let diff_stats_json =
            serde_json::to_string(&diff_stats).map_err(|error| error.to_string())?;
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "task_id": self.task_id(owner.task_index),
            "change_id": owner.change.map(|(_, change)| format!("C-{:02}", change.change_ordinal + 1)),
            "change_ref": change_ref,
            "patchset_number": u64::from(record.patch_ordinal) + 1,
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "source_kind": source_kind,
            "governance_authority": governance_authority,
            "author_mode": author_mode,
            "publish_state": publish_state,
            "withdrawn": record.patchset_meta & 1 != 0,
            "invalidated": record.patchset_meta & 2 != 0,
            "diff_stats": diff_stats,
            "diff_stats_json": diff_stats_json,
            "evaluation_state": evaluation_state,
            "created_at": Self::timestamp(record.created_at_s)?,
            "ci_run_seq": record.ci_run_seq,
            "ci_completed_at_s": record.ci_completed_at_s,
            "ci": {
                "completed_at": if record.ci_completed_at_s == 0 { JsonValue::Null } else { DateTime::<Utc>::from_timestamp(record.ci_completed_at_s as i64, 0).map(|value| json!(value.to_rfc3339())).unwrap_or(JsonValue::Null) },
                "run_seq": record.ci_run_seq,
                "selected_suite_count": record.ci_selected_suite_count,
                "suite_result_count": record.ci_suite_result_count,
                "blocking_failure_count": record.ci_blocking_failure_count,
                "overall_status": ci_status(CI_STATUS_OVERALL_SHIFT),
                "tests_status": ci_status(CI_STATUS_TESTS_SHIFT),
                "lint_status": ci_status(CI_STATUS_LINT_SHIFT),
            },
        }))
    }

    fn patchset_diff_stats(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        base_snapshot_index: u32,
        revision_snapshot_index: u32,
    ) -> Result<JsonValue, String> {
        let base = self.snapshot_file_map(read, base_snapshot_index)?;
        let revision = self.snapshot_file_map(read, revision_snapshot_index)?;
        Self::diff_stats_from_snapshot_maps(&base, &revision)
    }

    fn patchset_diff_stats_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        base_snapshot_index: u32,
        revision_snapshot_index: u32,
    ) -> Result<JsonValue, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let base = self.snapshot_file_map_in_write(write, base_snapshot_index)?;
        let revision = self.snapshot_file_map_in_write(write, revision_snapshot_index)?;
        Self::diff_stats_from_snapshot_maps(&base, &revision)
    }

    fn diff_stats_from_snapshot_maps(
        base: &BTreeMap<String, ServerBinarySnapshotFileIdentity>,
        revision: &BTreeMap<String, ServerBinarySnapshotFileIdentity>,
    ) -> Result<JsonValue, String> {
        let base_paths = base.keys().cloned().collect::<BTreeSet<_>>();
        let revision_paths = revision.keys().cloned().collect::<BTreeSet<_>>();
        let added = revision_paths
            .difference(&base_paths)
            .cloned()
            .collect::<Vec<_>>();
        let deleted = base_paths
            .difference(&revision_paths)
            .cloned()
            .collect::<Vec<_>>();
        let modified = base_paths
            .intersection(&revision_paths)
            .filter(|path| base.get(*path) != revision.get(*path))
            .cloned()
            .collect::<Vec<_>>();
        Ok(json!({
            "files_added": added.len(),
            "files_deleted": deleted.len(),
            "files_modified": modified.len(),
            "files_changed": added.len() + deleted.len() + modified.len(),
            "paths": {
                "added": added,
                "deleted": deleted,
                "modified": modified,
            }
        }))
    }

    fn snapshot_file_map_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        snapshot_index: u32,
    ) -> Result<BTreeMap<String, ServerBinarySnapshotFileIdentity>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let record = ServerBinarySnapshotCodec::<2>::decode_record(
            &write
                .read_record(
                    ServerBinarySnapshotCodec::<2>::record_file(),
                    snapshot_index,
                )
                .map_err(|error| Self::error("Patchset Snapshot read", error))?,
        )
        .map_err(|error| Self::error("Patchset Snapshot decode", error))?;
        if record.is_tombstone() {
            return Err(format!(
                "Binary DB v0 Patchset references tombstoned Snapshot {snapshot_index}"
            ));
        }
        ServerBinaryRepositoryContentStore::new(self.db.clone())
            .snapshot_file_map_in_write(write, &record)
            .map_err(|error| Self::error("Patchset Snapshot Tree comparison", error))
    }

    fn snapshot_file_map(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        snapshot_index: u32,
    ) -> Result<BTreeMap<String, ServerBinarySnapshotFileIdentity>, String> {
        let record = ServerBinarySnapshotCodec::<2>::decode_record(
            &read
                .read_record(
                    ServerBinarySnapshotCodec::<2>::record_file(),
                    snapshot_index,
                )
                .map_err(|error| Self::error("Patchset Snapshot read", error))?,
        )
        .map_err(|error| Self::error("Patchset Snapshot decode", error))?;
        if record.is_tombstone() {
            return Err(format!(
                "Binary DB v0 Patchset references tombstoned Snapshot {snapshot_index}"
            ));
        }
        ServerBinaryRepositoryContentStore::new(self.db.clone())
            .snapshot_file_map_with_read(read, &record)
            .map_err(|error| Self::error("Patchset Snapshot Tree comparison", error))
    }

    fn queue_patchset_at(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        index: u32,
    ) -> Result<JsonValue, String> {
        let record = self
            .read_patchset(read, index)
            .map_err(|error| Self::error("queue Patchset read", error))?;
        let owner = self.patchset_owner(read, record)?;
        let raw_summary = read
            .read_payload_v0(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                record.summary_offset,
                u32::from(record.summary_len),
            )
            .map_err(|e| e.to_string())?;
        let summary = WorkflowBinaryV0Codec::decode_single_text_payload(
            &raw_summary,
            "queue Patchset summary",
        )
        .map_err(|e| e.to_string())?;
        let (source_kind, governance_authority) =
            history_promotion::source_kind_for_summary(summary);
        let snapshots =
            ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let base_snapshot_id = snapshots
            .snapshot_id_at(read, record.base_snapshot_index)
            .map_err(|error| Self::error("queue Patchset base Snapshot read", error))?;
        let revision_snapshot_id = snapshots
            .snapshot_id_at(read, record.revision_snapshot_index)
            .map_err(|error| Self::error("queue Patchset revision Snapshot read", error))?;
        let change_ref = owner
            .change
            .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal));
        Ok(json!({
                "patchset_id": self.patchset_public_id(record),
                "repo_name": self.db.repo_name().as_str(),
                "repo_id": self.db.repo_id().as_str(),
                "task_id": self.task_id(owner.task_index),
                "change_id": owner.change.map(|(_, change)| format!("C-{:02}", change.change_ordinal + 1)),
                "change_ref": change_ref,
        "patchset_number": u64::from(record.patch_ordinal) + 1,
        "source_kind":source_kind,"governance_authority":governance_authority,
        "withdrawn":record.patchset_meta & 1 != 0,"invalidated":record.patchset_meta & 2 != 0,
        "publish_state":if record.patchset_meta & PATCHSET_PUBLISH_STATE_MASK == 0 {"published"}else{"superseded"},
        "ci":{"run_seq":record.ci_run_seq,"completed_at_s":record.ci_completed_at_s,
        "overall_status":ci_status_name(record.ci_status(CI_STATUS_OVERALL_SHIFT)),
        "tests_status":ci_status_name(record.ci_status(CI_STATUS_TESTS_SHIFT)),
        "lint_status":ci_status_name(record.ci_status(CI_STATUS_LINT_SHIFT)),
        "selected_suite_count":record.ci_selected_suite_count,"suite_result_count":record.ci_suite_result_count,
        "blocking_failure_count":record.ci_blocking_failure_count},
                "base_snapshot_id": base_snapshot_id,
                "revision_snapshot_id": revision_snapshot_id,
                "created_at": Self::timestamp(record.created_at_s)?,
            }))
    }

    fn owner_ordinal_index<A: ReadV0>(
        &self,
        read: &A,
        path: &'static str,
        owner_index: u32,
    ) -> Result<V0OrdinalIndexRecord, String> {
        let raw = read
            .read_record_v0(WorkflowBinaryV0Codec::chain_index_file(path), owner_index)
            .map_err(|error| Self::error("owner ordinal index read", error))?;
        (if path == "task_snapshot_index.bin" {
            WorkflowBinaryV0Codec::decode_task_snapshot_index(&raw)
        } else {
            WorkflowBinaryV0Codec::decode_ordinal_index(&raw)
        })
        .map_err(|error| Self::error("owner ordinal index decode", error))
    }

    fn latest_succeeded_land<A: ReadV0>(
        &self,
        read: &A,
        change_index: u32,
    ) -> Result<Option<(u32, V0LandRecord)>, String> {
        let change = self
            .read_change(read, change_index)
            .map_err(|e| e.to_string())?;
        Ok(self
            .task_successful_receipt(read, change.task_index)?
            .filter(|(_, receipt)| receipt.change_ordinal_plus1 == change.change_ordinal + 1))
    }

    pub(super) fn latest_succeeded_lands_from_records(
        &self,
        records: &[Vec<u8>],
    ) -> Result<BTreeMap<u32, (u32, V0LandRecord)>, String> {
        let mut latest = BTreeMap::new();
        for (index, raw) in records.iter().enumerate() {
            let land = WorkflowBinaryV0Codec::decode_land(raw)
                .map_err(|error| Self::error("queue Land decode", error))?;
            if land.land_meta & LAND_TOMBSTONE != 0
                || land.land_meta & LAND_STATUS_MASK != LAND_STATUS_SUCCEEDED
            {
                continue;
            }
            let index = u32::try_from(index)
                .map_err(|_| "Binary DB v0 queue Land index exceeds u32".to_string())?;
            if latest.insert(land.task_index, (index, land)).is_some() {
                return Err(
                    "Task has multiple successful receipts; historical disposition required"
                        .to_string(),
                );
            }
        }
        Ok(latest)
    }

    fn latest_succeeded_lands(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        trace_name: &'static str,
    ) -> Result<BTreeMap<u32, (u32, V0LandRecord)>, String> {
        #[cfg(feature = "perfetto-tracing")]
        let _trace = crate::perfetto_trace::PerfettoRange::new(trace_name);
        #[cfg(not(feature = "perfetto-tracing"))]
        let _ = trace_name;
        let file = WorkflowBinaryV0Codec::land_file();
        let count = read
            .record_count(file.clone())
            .map_err(|error| Self::error("queue Land count", error))?;
        let records = read
            .read_records(file, 0, count)
            .map_err(|error| Self::error("queue Land range read", error))?;
        self.latest_succeeded_lands_from_records(&records)
    }

    fn latest_policy_decision<A: ReadV0>(
        &self,
        read: &A,
        patchset_index: u32,
    ) -> Result<Option<(u32, V0PolicyRecord)>, String> {
        let count = read
            .record_count_v0(WorkflowBinaryV0Codec::policy_file())
            .map_err(|error| Self::error("Policy read", error))?;
        let mut latest = None;
        for index in 0..count {
            let policy = WorkflowBinaryV0Codec::decode_policy(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::policy_file(), index)
                    .map_err(|error| Self::error("Policy read", error))?,
            )
            .map_err(|error| Self::error("Policy decode", error))?;
            if policy.patchset_index == patchset_index
                && policy.policy_meta & POLICY_TOMBSTONE == 0
                && latest
                    .as_ref()
                    .is_none_or(|(_, current): &(u32, V0PolicyRecord)| {
                        policy.policy_ordinal > current.policy_ordinal
                    })
            {
                latest = Some((index, policy));
            }
        }
        Ok(latest)
    }
}

trait ReadV0 {
    fn lookup_index_v0(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>>;
    fn read_record_v0(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>>;
    fn record_count_v0(&self, file: BinaryFileId) -> StoreResult<u32>;
    fn read_payload_v0(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>>;
}

impl<'a, B> ReadV0 for BinaryDbReadTxn<'a, B>
where
    B: BinaryDb + ?Sized,
{
    fn lookup_index_v0(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>> {
        self.lookup_index(index, key)
    }
    fn read_record_v0(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>> {
        self.read_record(file, index)
    }

    fn record_count_v0(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.record_count(file)
    }

    fn read_payload_v0(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.read_payload(file, offset, len)
    }
}

impl<B, F> ReadV0 for BinaryDbWriteTxn<'_, B, F>
where
    B: BinaryDb + ?Sized,
    F: BinaryDbFsyncPolicy,
{
    fn lookup_index_v0(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>> {
        self.lookup_index(index, key)
    }
    fn read_record_v0(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>> {
        self.read_record(file, index)
    }

    fn record_count_v0(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.record_count(file)
    }

    fn read_payload_v0(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.read_payload(file, offset, len)
    }
}

fn ci_status_name(value: u8) -> &'static str {
    match value {
        CI_STATUS_PASS => "pass",
        CI_STATUS_FAIL => "fail",
        CI_STATUS_ERROR => "error",
        _ => "none",
    }
}

fn policy_decision_name(value: u8) -> Result<&'static str, String> {
    match value {
        0 => Ok("pending"),
        1 => Ok("pass"),
        2 => Ok("soft_fail"),
        3 => Ok("hard_fail"),
        4 => Ok("waived"),
        _ => Err("Binary DB v0 Policy decision kind is reserved".to_string()),
    }
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(crate) fn queue_projection_values_nonblocking(&self) -> Result<JsonValue, String> {
        #[cfg(feature = "perfetto-tracing")]
        let _trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.queue_projection.workflow_v0_values",
        );
        let read = BinaryDbReadTxn::new(&self.db);

        #[cfg(feature = "perfetto-tracing")]
        let phase_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.queue_projection.workflow_v0_tasks",
        );
        let task_count = read
            .record_count(WorkflowBinaryV0Codec::task_file())
            .map_err(|error| Self::error("queue Task count", error))?;
        let mut tasks = Vec::new();
        for index in 0..task_count {
            let record = self
                .read_task(&read, index)
                .map_err(|error| Self::error("queue Task read", error))?;
            if record.remote_meta & 1 == 0 {
                tasks.push(self.task_at(&read, index)?);
            }
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(phase_trace);

        #[cfg(feature = "perfetto-tracing")]
        let phase_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.queue_projection.workflow_v0_changes",
        );
        let change_count = read
            .record_count(WorkflowBinaryV0Codec::change_file())
            .map_err(|error| Self::error("queue Change count", error))?;
        let latest_succeeded_lands = self.latest_succeeded_lands(
            &read,
            "ait.server.queue_projection.workflow_v0_latest_succeeded_lands",
        )?;
        let mut changes = Vec::new();
        for index in 0..change_count {
            let record = self
                .read_change(&read, index)
                .map_err(|error| Self::error("queue Change read", error))?;
            if record.remote_meta & 1 == 0 {
                changes.push(
                    self.change_at_with_precomputed_latest_success(
                        &read,
                        index,
                        Some(
                            latest_succeeded_lands
                                .get(&record.task_index)
                                .copied()
                                .filter(|(_, land)| {
                                    land.change_ordinal_plus1 == record.change_ordinal + 1
                                }),
                        ),
                    )?,
                );
            }
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(phase_trace);

        #[cfg(feature = "perfetto-tracing")]
        let phase_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.queue_projection.workflow_v0_patchsets",
        );
        let patchset_count = read
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| Self::error("queue Patchset count", error))?;
        let mut patchsets = Vec::with_capacity(patchset_count as usize);
        for index in 0..patchset_count {
            patchsets.push(self.queue_patchset_at(&read, index)?);
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(phase_trace);

        #[cfg(feature = "perfetto-tracing")]
        let phase_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.queue_projection.workflow_v0_reviews",
        );
        let review_count = read
            .record_count(WorkflowBinaryV0Codec::review_file())
            .map_err(|error| Self::error("queue Review count", error))?;
        let mut reviews = Vec::new();
        for index in 0..review_count {
            let record = WorkflowBinaryV0Codec::decode_review(
                &read
                    .read_record(WorkflowBinaryV0Codec::review_file(), index)
                    .map_err(|error| Self::error("queue Review read", error))?,
            )
            .map_err(|error| Self::error("queue Review decode", error))?;
            if record.review_meta & REVIEW_TOMBSTONE == 0 {
                reviews.push(self.review_at(&read, index)?);
            }
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(phase_trace);

        #[cfg(feature = "perfetto-tracing")]
        let phase_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.queue_projection.workflow_v0_attestations",
        );
        let attestation_count = read
            .record_count(WorkflowBinaryV0Codec::attest_file())
            .map_err(|error| Self::error("queue Attestation count", error))?;
        let mut latest_attestations = BTreeMap::new();
        for index in 0..attestation_count {
            let record = WorkflowBinaryV0Codec::decode_attest(
                &read
                    .read_record(WorkflowBinaryV0Codec::attest_file(), index)
                    .map_err(|error| Self::error("queue Attestation read", error))?,
            )
            .map_err(|error| Self::error("queue Attestation decode", error))?;
            if record.attest_meta & ATTEST_TOMBSTONE == 0 {
                latest_attestations.insert(record.patchset_index, index);
            }
        }
        let attestations = latest_attestations
            .into_values()
            .map(|index| self.attestation_at(&read, index))
            .collect::<Result<Vec<_>, _>>()?;
        #[cfg(feature = "perfetto-tracing")]
        drop(phase_trace);

        #[cfg(feature = "perfetto-tracing")]
        let phase_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.queue_projection.workflow_v0_policies",
        );
        let policy_count = read
            .record_count(WorkflowBinaryV0Codec::policy_file())
            .map_err(|error| Self::error("queue Policy count", error))?;
        let mut latest_policies = BTreeMap::new();
        for index in 0..policy_count {
            let record = WorkflowBinaryV0Codec::decode_policy(
                &read
                    .read_record(WorkflowBinaryV0Codec::policy_file(), index)
                    .map_err(|error| Self::error("queue Policy read", error))?,
            )
            .map_err(|error| Self::error("queue Policy decode", error))?;
            if record.policy_meta & POLICY_TOMBSTONE == 0 {
                latest_policies.insert(record.patchset_index, index);
            }
        }
        let policy_decisions = latest_policies
            .into_values()
            .map(|index| self.policy_at(&read, index))
            .collect::<Result<Vec<_>, _>>()?;
        #[cfg(feature = "perfetto-tracing")]
        drop(phase_trace);

        Ok(json!({
            "tasks": tasks,
            "changes": changes,
            "patchsets": patchsets,
            "reviews": reviews,
            "attestations": attestations,
            "policy_decisions": policy_decisions,
        }))
    }
}

pub fn validate_server_workflow_v0<D>(db: &D) -> Result<(), String>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    validate_server_workflow_v0_with_patchset_codec(db, PatchsetCodecMode::Transitional)
}

pub fn validate_frozen_server_workflow_v0<D>(db: &D) -> Result<(), String>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    validate_server_workflow_v0_with_patchset_codec(db, PatchsetCodecMode::Frozen)
}

fn validate_server_workflow_v0_with_patchset_codec<D>(
    db: &D,
    patchset_codec: PatchsetCodecMode,
) -> Result<(), String>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    let read = BinaryDbReadTxn::new(db);
    let task_count = read
        .record_count(WorkflowBinaryV0Codec::task_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    for path in [
        "task_change_index.bin",
        "task_patchset_index.bin",
        "task_attest_index.bin",
        "task_review_index.bin",
        "task_policy_index.bin",
        "task_land_index.bin",
        "task_snapshot_index.bin",
        "task_waiver_index.bin",
    ] {
        let count = read
            .record_count(WorkflowBinaryV0Codec::chain_index_file(path))
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        if count != task_count {
            return Err(format!(
                "Binary DB v0 {path} count {count} does not align with task.bin count {task_count}"
            ));
        }
    }
    let change_count = read
        .record_count(WorkflowBinaryV0Codec::change_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    for path in [
        "change_patchset_index.bin",
        "change_land_index.bin",
        "change_snapshot_index.bin",
    ] {
        if read
            .record_count(WorkflowBinaryV0Codec::chain_index_file(path))
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?
            != change_count
        {
            return Err(format!(
                "Binary DB v0 {path} is not aligned with change.bin"
            ));
        }
    }
    let patchset_count = read
        .record_count(WorkflowBinaryV0Codec::patchset_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    for path in [
        "patchset_attest_index.bin",
        "patchset_review_index.bin",
        "patchset_policy_index.bin",
        "patchset_waiver_index.bin",
    ] {
        if read
            .record_count(WorkflowBinaryV0Codec::chain_index_file(path))
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?
            != patchset_count
        {
            return Err(format!(
                "Binary DB v0 {path} is not aligned with patchset.bin"
            ));
        }
    }
    let land_count = read
        .record_count(WorkflowBinaryV0Codec::land_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let line_count = read
        .record_count(ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let mut line_live = Vec::with_capacity(line_count as usize);
    for index in 0..line_count {
        let record = ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(
            &read
                .read_record(
                    ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
                    index,
                )
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        line_live.push(!record.is_tombstone());
    }
    let snapshot_count = read
        .record_count(ServerBinarySnapshotCodec::<2>::record_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let mut snapshot_live = Vec::with_capacity(snapshot_count as usize);
    for index in 0..snapshot_count {
        let record = ServerBinarySnapshotCodec::<2>::decode_record(
            &read
                .read_record(ServerBinarySnapshotCodec::<2>::record_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        snapshot_live.push(!record.is_tombstone());
    }
    let plan_revision_count = read
        .record_count(crate::foundation::server_plan_binary_db::remote_plan_revision_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let plan_item_count = read
        .record_count(BinaryFileId::new(
            "plan_item.bin",
            1,
            16,
            BinaryDbFileFamily::Plan,
        ))
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;

    let mut task_ranges = Vec::new();
    let mut tasks = Vec::with_capacity(task_count as usize);
    for index in 0..task_count {
        let record = WorkflowBinaryV0Codec::decode_task(
            &read
                .read_record(WorkflowBinaryV0Codec::task_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let payload = read
            .read_payload(
                WorkflowBinaryV0Codec::task_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        WorkflowBinaryV0Codec::decode_task_payload(&payload)
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        task_ranges.push((record.payload_offset, u32::from(record.payload_len), index));
        if record
            .origin_plan_revision_index_plus1
            .checked_sub(1)
            .is_some_and(|value| value >= plan_revision_count)
            || record
                .plan_item_index_plus1
                .checked_sub(1)
                .is_some_and(|value| value >= plan_item_count)
        {
            return Err(format!(
                "Binary DB v0 Task {index} has an out-of-range Plan binding"
            ));
        }
        tasks.push(record);
    }
    validate_payload_ranges("task_payload.bin", &mut task_ranges)?;

    let mut change_ranges = Vec::new();
    let mut changes = Vec::with_capacity(change_count as usize);
    let mut change_chain_items = Vec::new();
    for index in 0..change_count {
        let record = WorkflowBinaryV0Codec::decode_change(
            &read
                .read_record(WorkflowBinaryV0Codec::change_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        if record.task_index >= task_count {
            return Err(format!(
                "Binary DB v0 Change {index} has an invalid Task owner"
            ));
        }
        if record
            .base_line_index_plus1
            .checked_sub(1)
            .and_then(|value| line_live.get(value as usize))
            .copied()
            != Some(true)
            || record
                .fork_snapshot_index_plus1
                .checked_sub(1)
                .is_some_and(|value| snapshot_live.get(value as usize).copied() != Some(true))
        {
            return Err(format!(
                "Binary DB v0 Change {index} has an invalid Line/Snapshot relation"
            ));
        }
        if record.lifecycle() != CHANGE_LIFECYCLE_ARCHIVED && record.archived_at_s != 0 {
            return Err(format!(
                "Binary DB v0 Change {index} retains archive time outside archived lifecycle"
            ));
        }
        let payload = read
            .read_payload(
                WorkflowBinaryV0Codec::change_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        WorkflowBinaryV0Codec::decode_single_text_payload(&payload, "Change title")
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        change_ranges.push((record.payload_offset, u32::from(record.payload_len), index));
        change_chain_items.push((
            index,
            record.task_index,
            record.change_ordinal,
            record.previous_change_index_plus1,
        ));
        changes.push(record);
    }
    validate_payload_ranges("change_payload.bin", &mut change_ranges)?;

    let mut change_by_owner = BTreeMap::new();
    for (index, change) in changes.iter().enumerate() {
        if change_by_owner
            .insert((change.task_index, change.change_ordinal), index as u32)
            .is_some()
        {
            return Err("Duplicate Task Change ordinal".to_string());
        }
    }
    let mut task_patchset_ordinals = BTreeMap::<u32, BTreeSet<u8>>::new();
    let mut patchset_ranges = Vec::new();
    let mut patchsets = Vec::with_capacity(patchset_count as usize);
    let mut patchset_change_items = Vec::new();
    let mut patchset_task_items = Vec::new();
    for index in 0..patchset_count {
        let raw = read
            .read_record(WorkflowBinaryV0Codec::patchset_file(), index)
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let record = match patchset_codec {
            PatchsetCodecMode::Transitional => WorkflowBinaryV0Codec::decode_patchset(&raw),
            PatchsetCodecMode::Frozen => WorkflowBinaryV0Codec::decode_frozen_patchset(&raw)
                .map(BinaryDbServerWorkflowV0Store::<D>::logical_patchset_from_frozen),
        }
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let change_index = record
            .change_ordinal_plus1
            .checked_sub(1)
            .map(|ordinal| {
                change_by_owner
                    .get(&(record.task_index, ordinal))
                    .copied()
                    .ok_or_else(|| format!("Patchset {index} has an invalid explicit Change"))
            })
            .transpose()?;
        if record.task_index >= task_count
            || !task_patchset_ordinals
                .entry(record.task_index)
                .or_default()
                .insert(record.patch_ordinal)
            || snapshot_live
                .get(record.base_snapshot_index as usize)
                .copied()
                != Some(true)
            || snapshot_live
                .get(record.revision_snapshot_index as usize)
                .copied()
                != Some(true)
        {
            return Err(format!(
                "Binary DB v0 Patchset {index} has an invalid Change/Snapshot relation"
            ));
        }
        let summary = read
            .read_payload(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                record.summary_offset,
                u32::from(record.summary_len),
            )
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        WorkflowBinaryV0Codec::decode_single_text_payload(&summary, "Patchset summary")
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        patchset_ranges.push((record.summary_offset, u32::from(record.summary_len), index));
        if let Some(change_index) = change_index {
            patchset_change_items.push((
                index,
                change_index,
                record.patch_ordinal,
                record.previous_change_patchset_index_plus1,
            ));
        }
        patchset_task_items.push((
            index,
            record.task_index,
            record.previous_task_patchset_index_plus1,
        ));
        patchsets.push(record);
    }
    validate_payload_ranges("patchset_summary_payload.bin", &mut patchset_ranges)?;
    for (index, change) in changes.iter().enumerate() {
        let has_patchsets = patchsets.iter().any(|patchset| {
            patchset.task_index == change.task_index
                && patchset.change_ordinal_plus1 == change.change_ordinal + 1
        });
        if (change.change_meta & CHANGE_META_HAS_PATCHSETS != 0) != has_patchsets {
            return Err(format!(
                "Binary DB v0 Change {index} has_patchsets projection disagrees"
            ));
        }
    }

    let attest_count = read
        .record_count(WorkflowBinaryV0Codec::attest_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let mut attest_task_items = Vec::new();
    let mut attest_patch_items = Vec::new();
    for index in 0..attest_count {
        let record = WorkflowBinaryV0Codec::decode_attest(
            &read
                .read_record(WorkflowBinaryV0Codec::attest_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let patchset = patchsets
            .get(record.patchset_index as usize)
            .ok_or_else(|| {
                format!("Binary DB v0 Attestation {index} has an invalid Patchset owner")
            })?;
        if record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal_plus1 != patchset.change_ordinal_plus1
        {
            return Err(format!(
                "Binary DB v0 Attestation {index} ownership disagrees"
            ));
        }
        attest_task_items.push((
            index,
            patchset.task_index,
            record.attest_ordinal,
            record.previous_task_attest_index_plus1,
        ));
        attest_patch_items.push((
            index,
            record.patchset_index,
            record.previous_patchset_attest_index_plus1,
        ));
    }

    let actor_count = read
        .record_count(WorkflowBinaryV0Codec::actor_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let mut actor_ranges = Vec::new();
    let mut actors = Vec::with_capacity(actor_count as usize);
    let mut actor_identities = BTreeSet::new();
    for index in 0..actor_count {
        let record = WorkflowBinaryV0Codec::decode_actor(
            &read
                .read_record(WorkflowBinaryV0Codec::actor_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let bytes = read
            .read_payload(
                WorkflowBinaryV0Codec::actor_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let payload = WorkflowBinaryV0Codec::decode_actor_payload(&bytes)
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let expected_optional = (if payload.user_id.is_empty() {
            0
        } else {
            1 << 3
        }) | (if payload.email.is_empty() { 0 } else { 1 << 4 })
            | (if payload.memo.is_empty() { 0 } else { 1 << 5 });
        if record.actor_meta & 0b0011_1000 != expected_optional
            || record.actor_key_hash != fnv1a64(payload.user_name.as_bytes())
            || ((record.created_at_s == 0) != (record.last_seen_at_s == 0))
            || (record.created_at_s != 0 && record.created_at_s > record.last_seen_at_s)
            || (record.actor_meta & 0x80 == 0
                && !actor_identities.insert((
                    record.actor_meta & 0b111,
                    payload.user_name,
                    payload.user_id,
                    payload.email,
                    payload.memo,
                )))
        {
            return Err(format!(
                "Binary DB v0 Actor {index} identity authority is invalid"
            ));
        }
        let key = record.actor_key_hash.to_le_bytes();
        if !read
            .lookup_index(WorkflowBinaryV0Codec::actor_lookup_index(), &key)
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?
            .contains(&index)
        {
            return Err(format!(
                "Binary DB v0 Actor {index} is missing from actor_lookup.idx"
            ));
        }
        actor_ranges.push((record.payload_offset, u32::from(record.payload_len), index));
        actors.push(record);
    }
    validate_payload_ranges("actor_payload.bin", &mut actor_ranges)?;

    let review_count = read
        .record_count(WorkflowBinaryV0Codec::review_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let mut review_ranges = Vec::new();
    let mut review_patch_items = Vec::new();
    let mut review_task_items = Vec::new();
    for index in 0..review_count {
        let record = WorkflowBinaryV0Codec::decode_review(
            &read
                .read_record(WorkflowBinaryV0Codec::review_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let patchset = patchsets
            .get(record.patchset_index as usize)
            .ok_or_else(|| format!("Binary DB v0 Review {index} has an invalid Patchset owner"))?;
        if record
            .actor_index_plus1
            .checked_sub(1)
            .and_then(|value| actors.get(value as usize))
            .is_none_or(|actor| actor.actor_meta & 0x80 != 0)
            || record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal_plus1 != patchset.change_ordinal_plus1
        {
            return Err(format!("Binary DB v0 Review {index} ownership disagrees"));
        }
        let message = read
            .read_payload(
                WorkflowBinaryV0Codec::review_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        WorkflowBinaryV0Codec::decode_review_payload(&message)
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        review_ranges.push((record.payload_offset, u32::from(record.payload_len), index));
        review_patch_items.push((
            index,
            record.patchset_index,
            record.review_ordinal,
            record.previous_patchset_review_index_plus1,
        ));
        review_task_items.push((
            index,
            patchset.task_index,
            record.previous_task_review_index_plus1,
        ));
    }
    validate_payload_ranges("review_payload.bin", &mut review_ranges)?;

    let snapshot_link_count = read
        .record_count(WorkflowBinaryV0Codec::snapshot_link_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let mut snapshot_task_items = Vec::new();
    let mut snapshot_change_items = Vec::new();
    let mut linked_content = BTreeSet::new();
    for index in 0..snapshot_link_count {
        let record = WorkflowBinaryV0Codec::decode_snapshot_link(
            &read
                .read_record(WorkflowBinaryV0Codec::snapshot_link_file(), index)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if record.task_index >= task_count
            || snapshot_live
                .get(record.content_snapshot_index as usize)
                .copied()
                != Some(true)
            // Historical metadata can associate distinct owners with the same
            // content. Operation-specific capture/finish authority is checked
            // separately and never inferred by choosing one of these rows.
            || !linked_content.insert((record.task_index, record.change_index_plus1, record.content_snapshot_index))
        {
            return Err(format!(
                "Snapshot Link {index} has invalid or duplicate Task/content ownership"
            ));
        }
        let snapshot = ServerBinarySnapshotCodec::<2>::decode_record(
            &read
                .read_record(
                    ServerBinarySnapshotCodec::<2>::record_file(),
                    record.content_snapshot_index,
                )
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if record.created_at_s != snapshot.created_at_s {
            return Err("Snapshot Link immutable timestamp differs".to_string());
        }
        if let Some(change_index) = record.change_index_plus1.checked_sub(1) {
            let change = changes
                .get(change_index as usize)
                .ok_or("Snapshot Link Change is out of range")?;
            if change.task_index != record.task_index {
                return Err("Snapshot Link Change belongs to another Task".to_string());
            }
            snapshot_change_items.push((index, change_index));
        }
        snapshot_task_items.push((index, record.task_index, record.snapshot_ordinal));
    }

    let waiver_count = read
        .record_count(WorkflowBinaryV0Codec::waiver_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let mut waiver_ranges = Vec::new();
    let mut waiver_task_items = Vec::new();
    let mut waiver_patch_items = Vec::new();
    for index in 0..waiver_count {
        let record = WorkflowBinaryV0Codec::decode_waiver(
            &read
                .read_record(WorkflowBinaryV0Codec::waiver_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let patchset = patchsets
            .get(record.patchset_index as usize)
            .ok_or_else(|| format!("Binary DB v0 Waiver {index} has an invalid Patchset owner"))?;
        if record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal_plus1 != patchset.change_ordinal_plus1
        {
            return Err(format!("Binary DB v0 Waiver {index} authority is invalid"));
        }
        let reason = read
            .read_payload(
                WorkflowBinaryV0Codec::waiver_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        WorkflowBinaryV0Codec::decode_single_text_payload(&reason, "Waiver reason")
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        waiver_ranges.push((record.payload_offset, u32::from(record.payload_len), index));
        waiver_task_items.push((
            index,
            patchset.task_index,
            record.waiver_ordinal,
            record.previous_task_waiver_index_plus1,
        ));
        waiver_patch_items.push((
            index,
            record.patchset_index,
            record.previous_patchset_waiver_index_plus1,
        ));
    }
    validate_payload_ranges("waiver_payload.bin", &mut waiver_ranges)?;

    let policy_count = read
        .record_count(WorkflowBinaryV0Codec::policy_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let policy_check_count = read
        .record_count(WorkflowBinaryV0Codec::policy_check_file())
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
    let mut policy_patch_items = Vec::new();
    let mut policy_task_items = Vec::new();
    let mut expected_check_index = 0_u32;
    let mut latest_policy_by_patch = BTreeMap::<u32, V0PolicyRecord>::new();
    for index in 0..policy_count {
        let record = WorkflowBinaryV0Codec::decode_policy(
            &read
                .read_record(WorkflowBinaryV0Codec::policy_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let patchset = patchsets
            .get(record.patchset_index as usize)
            .ok_or_else(|| format!("Binary DB v0 Policy {index} has an invalid Patchset owner"))?;
        if record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal_plus1 != patchset.change_ordinal_plus1
        {
            return Err(format!("Binary DB v0 Policy {index} ownership disagrees"));
        }
        let first = record
            .first_check_index_plus1
            .checked_sub(1)
            .unwrap_or(expected_check_index);
        if first != expected_check_index {
            return Err(format!(
                "Binary DB v0 Policy {index} Check range is not contiguous"
            ));
        }
        for offset in 0..u32::from(record.check_count) {
            let check_index = first
                .checked_add(offset)
                .ok_or_else(|| "Binary DB v0 Policy Check index overflow".to_string())?;
            if check_index >= policy_check_count {
                return Err(format!(
                    "Binary DB v0 Policy {index} Check range is out of bounds"
                ));
            }
            WorkflowBinaryV0Codec::decode_policy_check(
                &read
                    .read_record(WorkflowBinaryV0Codec::policy_check_file(), check_index)
                    .map_err(|error| {
                        BinaryDbServerWorkflowV0Store::<D>::error("validation", error)
                    })?,
            )
            .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        }
        expected_check_index = first + u32::from(record.check_count);
        policy_patch_items.push((
            index,
            record.patchset_index,
            record.policy_ordinal,
            record.previous_patchset_policy_index_plus1,
        ));
        policy_task_items.push((
            index,
            patchset.task_index,
            record.previous_task_policy_index_plus1,
        ));
        if record.policy_meta & POLICY_TOMBSTONE == 0
            && latest_policy_by_patch
                .get(&record.patchset_index)
                .is_none_or(|current| record.policy_ordinal > current.policy_ordinal)
        {
            latest_policy_by_patch.insert(record.patchset_index, record);
        }
    }
    if expected_check_index != policy_check_count {
        return Err("Binary DB v0 policy_check.bin has orphan committed rows".to_string());
    }
    for (index, patchset) in patchsets.iter().enumerate() {
        if patchset.patchset_meta & PATCHSET_EVALUATION_PENDING == 0 {
            let latest = latest_policy_by_patch.get(&(index as u32)).ok_or_else(|| {
                format!("Binary DB v0 non-pending Patchset {index} has no Policy Decision")
            })?;
            if latest.policy_meta & POLICY_DECISION_MASK == 0 {
                return Err(format!(
                    "Binary DB v0 non-pending Patchset {index} has only a pending Policy Decision"
                ));
            }
        }
    }

    let mut land_change_items = Vec::new();
    let mut land_task_items = Vec::new();
    let mut succeeded_land_by_change = vec![false; change_count as usize];
    let mut succeeded_land_by_task = BTreeSet::new();
    let mut task_land_ordinals = BTreeMap::<u32, BTreeSet<u8>>::new();
    for index in 0..land_count {
        let record = WorkflowBinaryV0Codec::decode_land(
            &read
                .read_record(WorkflowBinaryV0Codec::land_file(), index)
                .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?,
        )
        .map_err(|error| BinaryDbServerWorkflowV0Store::<D>::error("validation", error))?;
        let patchset = patchsets
            .get(record.patchset_index as usize)
            .ok_or_else(|| format!("Binary DB v0 Land {index} has an invalid accepted Patchset"))?;
        if record.change_ordinal_plus1 != patchset.change_ordinal_plus1
            || patchset.task_index != record.task_index
            || !task_land_ordinals
                .entry(record.task_index)
                .or_default()
                .insert(record.land_ordinal)
            || record
                .pre_land_target_snapshot_index_plus1
                .checked_sub(1)
                .is_some_and(|value| snapshot_live.get(value as usize).copied() != Some(true))
            || record
                .landed_snapshot_index_plus1
                .checked_sub(1)
                .is_some_and(|value| snapshot_live.get(value as usize).copied() != Some(true))
        {
            return Err(format!(
                "Binary DB v0 Land {index} ownership/Snapshot relation disagrees"
            ));
        }
        if record
            .target_line_index_plus1
            .checked_sub(1)
            .and_then(|value| line_live.get(value as usize))
            .copied()
            != Some(true)
        {
            return Err(format!(
                "Binary DB v0 Land {index} has an invalid target Line"
            ));
        }
        if record.land_meta & LAND_TOMBSTONE == 0
            && record.land_meta & LAND_STATUS_MASK == LAND_STATUS_SUCCEEDED
        {
            if !succeeded_land_by_task.insert(record.task_index) {
                return Err("Task has multiple successful Remote receipts; historical disposition is required".to_string());
            }
            let task = tasks
                .get(record.task_index as usize)
                .ok_or("Land Task is out of range")?;
            if task.task_meta & TASK_META_COMPLETED == 0 {
                return Err("Successful Remote receipt has an incomplete Task".to_string());
            }
            let raw = read
                .read_payload(
                    WorkflowBinaryV0Codec::patchset_summary_file(),
                    patchset.summary_offset,
                    u32::from(patchset.summary_len),
                )
                .map_err(|e| e.to_string())?;
            let summary =
                WorkflowBinaryV0Codec::decode_single_text_payload(&raw, "accepted Patchset")
                    .map_err(|e| e.to_string())?;
            if !history_promotion::source_kind_for_summary(summary).1 {
                return Err(
                    "Provenance-only history cannot own a Remote successful receipt".to_string(),
                );
            }
            if let Some(ordinal) = record.change_ordinal_plus1.checked_sub(1) {
                let change_index = *change_by_owner
                    .get(&(record.task_index, ordinal))
                    .ok_or("Land explicit Change is missing")?;
                succeeded_land_by_change[change_index as usize] = true;
            }
        }
        if let Some(ordinal) = record.change_ordinal_plus1.checked_sub(1) {
            let change_index = *change_by_owner
                .get(&(record.task_index, ordinal))
                .ok_or("Land explicit Change is missing")?;
            land_change_items.push((
                index,
                change_index,
                record.land_ordinal,
                record.previous_change_land_index_plus1,
            ));
        }
        land_task_items.push((
            index,
            record.task_index,
            record.previous_task_land_index_plus1,
        ));
    }
    for (index, change) in changes.iter().enumerate() {
        if (change.lifecycle() == CHANGE_LIFECYCLE_LANDED) != succeeded_land_by_change[index] {
            return Err(format!(
                "Binary DB v0 Change {index} lifecycle disagrees with successful Land authority"
            ));
        }
    }

    validate_ordinal_chains(
        "Task Change",
        &read_ordinal_indexes(&read, "task_change_index.bin", task_count)?,
        &change_chain_items,
    )?;
    validate_ordinal_chains(
        "Change Patchset",
        &read_ordinal_indexes(&read, "change_patchset_index.bin", change_count)?,
        &patchset_change_items,
    )?;
    validate_inventory_chains(
        "Task Patchset",
        &read_inventory_indexes(&read, "task_patchset_index.bin", task_count)?,
        &patchset_task_items,
    )?;
    validate_ordinal_chains(
        "Task Attestation",
        &read_ordinal_indexes(&read, "task_attest_index.bin", task_count)?,
        &attest_task_items,
    )?;
    validate_inventory_chains(
        "Patchset Attestation",
        &read_inventory_indexes(&read, "patchset_attest_index.bin", patchset_count)?,
        &attest_patch_items,
    )?;
    validate_ordinal_chains(
        "Patchset Review",
        &read_ordinal_indexes(&read, "patchset_review_index.bin", patchset_count)?,
        &review_patch_items,
    )?;
    validate_inventory_chains(
        "Task Review",
        &read_inventory_indexes(&read, "task_review_index.bin", task_count)?,
        &review_task_items,
    )?;
    validate_snapshot_owner_indexes(
        &read_ordinal_indexes(&read, "task_snapshot_index.bin", task_count)?,
        &read_inventory_indexes(&read, "change_snapshot_index.bin", change_count)?,
        &snapshot_task_items,
        &snapshot_change_items,
    )?;
    validate_ordinal_chains(
        "Task Waiver",
        &read_ordinal_indexes(&read, "task_waiver_index.bin", task_count)?,
        &waiver_task_items,
    )?;
    validate_inventory_chains(
        "Patchset Waiver",
        &read_inventory_indexes(&read, "patchset_waiver_index.bin", patchset_count)?,
        &waiver_patch_items,
    )?;
    validate_ordinal_chains(
        "Patchset Policy",
        &read_ordinal_indexes(&read, "patchset_policy_index.bin", patchset_count)?,
        &policy_patch_items,
    )?;
    validate_inventory_chains(
        "Task Policy",
        &read_inventory_indexes(&read, "task_policy_index.bin", task_count)?,
        &policy_task_items,
    )?;
    validate_ordinal_chains(
        "Change Land",
        &read_ordinal_indexes(&read, "change_land_index.bin", change_count)?,
        &land_change_items,
    )?;
    validate_inventory_chains(
        "Task Land",
        &read_inventory_indexes(&read, "task_land_index.bin", task_count)?,
        &land_task_items,
    )?;
    Ok(())
}

fn read_ordinal_indexes<D>(
    read: &BinaryDbReadTxn<'_, D>,
    path: &'static str,
    count: u32,
) -> Result<Vec<V0OrdinalIndexRecord>, String>
where
    D: BinaryDb + ?Sized,
{
    (0..count)
        .map(|index| {
            let raw = read
                .read_record(WorkflowBinaryV0Codec::chain_index_file(path), index)
                .map_err(|error| format!("Binary DB v0 {path} read failed: {error}"))?;
            (if path == "task_snapshot_index.bin" {
                WorkflowBinaryV0Codec::decode_task_snapshot_index(&raw)
            } else {
                WorkflowBinaryV0Codec::decode_ordinal_index(&raw)
            })
            .map_err(|error| format!("Binary DB v0 {path} decode failed: {error}"))
        })
        .collect()
}

fn read_inventory_indexes<D>(
    read: &BinaryDbReadTxn<'_, D>,
    path: &'static str,
    count: u32,
) -> Result<Vec<V0InventoryIndexRecord>, String>
where
    D: BinaryDb + ?Sized,
{
    (0..count)
        .map(|index| {
            WorkflowBinaryV0Codec::decode_inventory_index(
                &read
                    .read_record(WorkflowBinaryV0Codec::chain_index_file(path), index)
                    .map_err(|error| format!("Binary DB v0 {path} read failed: {error}"))?,
            )
            .map_err(|error| format!("Binary DB v0 {path} decode failed: {error}"))
        })
        .collect()
}

fn validate_ordinal_chains(
    label: &str,
    indexes: &[V0OrdinalIndexRecord],
    items: &[(u32, u32, u8, u32)],
) -> Result<(), String> {
    let mut by_owner = BTreeMap::<u32, Vec<(u32, u8, u32)>>::new();
    for &(physical, owner, ordinal, previous_plus1) in items {
        if owner as usize >= indexes.len() {
            return Err(format!(
                "Binary DB v0 {label} item {physical} has invalid owner {owner}"
            ));
        }
        by_owner
            .entry(owner)
            .or_default()
            .push((physical, ordinal, previous_plus1));
    }
    for (owner, index) in indexes.iter().enumerate() {
        let rows = by_owner.entry(owner as u32).or_default();
        rows.sort_by_key(|(_, ordinal, _)| *ordinal);
        let mut previous = 0_u32;
        let mut ordinals = BTreeSet::new();
        for &(physical, ordinal, previous_plus1) in rows.iter() {
            if !ordinals.insert(ordinal) || previous_plus1 != previous {
                return Err(format!(
                    "Binary DB v0 {label} owner {owner} has duplicate ordinal or broken previous link"
                ));
            }
            previous = physical + 1;
        }
        let expected_next = rows
            .last()
            .map(|(_, ordinal, _)| ordinal.saturating_add(1))
            .unwrap_or(0);
        if index.latest_index_plus1 != previous
            || usize::from(index.count) != rows.len()
            || index.next_ordinal != expected_next
        {
            return Err(format!(
                "Binary DB v0 {label} owner {owner} index projection disagrees"
            ));
        }
    }
    Ok(())
}

fn validate_inventory_chains(
    label: &str,
    indexes: &[V0InventoryIndexRecord],
    items: &[(u32, u32, u32)],
) -> Result<(), String> {
    let mut by_owner = BTreeMap::<u32, Vec<(u32, u32)>>::new();
    for &(physical, owner, previous_plus1) in items {
        if owner as usize >= indexes.len() {
            return Err(format!(
                "Binary DB v0 {label} item {physical} has invalid owner {owner}"
            ));
        }
        by_owner
            .entry(owner)
            .or_default()
            .push((physical, previous_plus1));
    }
    for (owner, index) in indexes.iter().enumerate() {
        let rows = by_owner.entry(owner as u32).or_default();
        rows.sort_by_key(|(physical, _)| *physical);
        let mut previous = 0_u32;
        for &(physical, previous_plus1) in rows.iter() {
            if previous_plus1 != previous {
                return Err(format!(
                    "Binary DB v0 {label} owner {owner} has a broken physical inventory link"
                ));
            }
            previous = physical + 1;
        }
        if index.latest_index_plus1 != previous || usize::from(index.count) != rows.len() {
            return Err(format!(
                "Binary DB v0 {label} owner {owner} index projection disagrees"
            ));
        }
    }
    Ok(())
}

fn validate_snapshot_owner_indexes(
    tasks: &[V0OrdinalIndexRecord],
    changes: &[V0InventoryIndexRecord],
    task_links: &[(u32, u32, u8)],
    change_links: &[(u32, u32)],
) -> Result<(), String> {
    let mut by_task = BTreeMap::<u32, Vec<(u32, u8)>>::new();
    for &(index, task, ordinal) in task_links {
        by_task.entry(task).or_default().push((index, ordinal));
    }
    for (task, expected) in tasks.iter().enumerate() {
        let rows = by_task.entry(task as u32).or_default();
        rows.sort_by_key(|(index, _)| *index);
        if rows
            .iter()
            .enumerate()
            .any(|(n, (_, ordinal))| usize::from(*ordinal) != n)
            || rows.len() != usize::from(expected.count)
            || expected.latest_index_plus1 != rows.last().map_or(0, |(index, _)| index + 1)
            || expected.next_ordinal != u8::try_from(rows.len()).unwrap_or(u8::MAX)
        {
            return Err(format!(
                "Task {task} Snapshot owner index disagrees with allocated numeric links"
            ));
        }
    }
    let mut by_change = BTreeMap::<u32, Vec<u32>>::new();
    for &(index, change) in change_links {
        by_change.entry(change).or_default().push(index);
    }
    for (change, expected) in changes.iter().enumerate() {
        let rows = by_change.entry(change as u32).or_default();
        if rows.len() != usize::from(expected.count)
            || expected.latest_index_plus1 != rows.iter().max().map_or(0, |index| index + 1)
        {
            return Err(format!(
                "Change {change} Snapshot owner index disagrees with numeric links"
            ));
        }
    }
    Ok(())
}

fn validate_payload_ranges(label: &str, ranges: &mut [(u64, u32, u32)]) -> Result<(), String> {
    ranges.sort_by_key(|(offset, _, _)| *offset);
    let mut previous_end = 4_u64;
    for &(offset, len, owner) in ranges.iter() {
        let end = offset
            .checked_add(u64::from(len))
            .ok_or_else(|| format!("Binary DB v0 {label} range overflow for record {owner}"))?;
        if offset < previous_end {
            return Err(format!(
                "Binary DB v0 {label} ranges overlap at record {owner}"
            ));
        }
        previous_end = end;
    }
    Ok(())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x00000100000001b3)
    })
}
