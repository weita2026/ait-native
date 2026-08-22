use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbFsyncPolicy, BinaryDbReadTxn, BinaryDbWriteTxn,
    BinaryFileId, BinaryPayloadFileId, StoreResult,
};
use crate::change_store::ChangeStore;
use crate::content_binary_db::{
    snapshot_hash48_from_id, snapshot_id_from_hash48, BinarySnapshotCodec,
};
use crate::json_support::{json, JsonValue};
use crate::line_binary_db::BinaryLineCodec;
use crate::plan_binary_db::{
    parse_repository_plan_id, repository_plan_id, PlanItemCodec, PlanRevisionCodec,
};
use crate::plan_store::{PlanStoreError, PlanStoreResult};
use crate::task_store::TaskStore;
use crate::task_workflow_store_traits::{
    TaskWorkflowChangeCloser, TaskWorkflowChangeCreator, TaskWorkflowChangeLander,
    TaskWorkflowChangeLister, TaskWorkflowChangePublisher, TaskWorkflowChangeReader,
    TaskWorkflowTaskCloser, TaskWorkflowTaskCreator, TaskWorkflowTaskLister,
    TaskWorkflowTaskPublisher, TaskWorkflowTaskReader,
};
use crate::workflow_primitives::{
    generate_namespaced_sequence_id, workflow_origin_namespace_prefix,
};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use std::collections::BTreeMap;

pub const BINARY_DB_WORKFLOW_LAYOUT_ID: u32 = 1;

pub const LOCAL_TASK_RECORD_SIZE: u32 = 64;
pub const LOCAL_CHANGE_RECORD_SIZE: u32 = 68;
pub const TASK_CHANGE_INDEX_RECORD_SIZE: u32 = 8;
pub const LOCAL_LAND_RECORD_SIZE: u32 = 44;
pub const TASK_LAND_INDEX_RECORD_SIZE: u32 = 8;
pub const CHANGE_LAND_INDEX_RECORD_SIZE: u32 = 8;

pub const TASK_RECORD_BIN: &str = "task.bin";
pub const TASK_PAYLOAD_BIN: &str = "task_payload.bin";
pub const TASK_CHANGE_INDEX_BIN: &str = "task_change_index.bin";
pub const TASK_LAND_INDEX_BIN: &str = "task_land_index.bin";
pub const CHANGE_RECORD_BIN: &str = "change.bin";
pub const CHANGE_PAYLOAD_BIN: &str = "change_payload.bin";
pub const CHANGE_LAND_INDEX_BIN: &str = "change_land_index.bin";
pub const LAND_RECORD_BIN: &str = "land.bin";

const MAX_CHANGE_ORDINAL: u8 = 63;
const MAX_LAND_ORDINAL: u8 = 63;
const LOCAL_TASK_META_KNOWN: u8 = 0b0000_0111;
const CHANGE_META_LIFECYCLE_MASK: u8 = 0b0000_0011;
const CHANGE_STATE_KNOWN: u8 = 0b0000_0001;
const LOCAL_CHANGE_META_KNOWN: u8 = 0b0000_0001;
const LAND_META_STATUS_MASK: u8 = 0b0000_0111;
const LAND_META_HAS_PRE_LAND: u8 = 0b0000_1000;
const LAND_META_HAS_LANDED: u8 = 0b0001_0000;
const LAND_META_MODE_MASK: u8 = 0b0110_0000;
const LAND_META_TOMBSTONE: u8 = 0b1000_0000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalTaskRecord {
    pub task_meta: u8,
    pub local_meta: u8,
    pub payload_len: u16,
    pub payload_offset: u64,
    pub origin_plan_revision_index_plus1: u32,
    pub plan_item_index_plus1: u32,
    pub published_remote_task_index: u32,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    pub plan_linked_at_s: u64,
    pub published_at_s: u64,
    pub closed_at_s: u64,
}

impl LocalTaskRecord {
    pub(crate) fn encode(&self) -> StoreResult<Vec<u8>> {
        validate_local_task_record(self)?;
        let mut out = Vec::with_capacity(LOCAL_TASK_RECORD_SIZE as usize);
        out.push(self.task_meta);
        out.push(self.local_meta);
        out.extend_from_slice(&self.payload_len.to_le_bytes());
        out.extend_from_slice(&self.payload_offset.to_le_bytes());
        out.extend_from_slice(&self.origin_plan_revision_index_plus1.to_le_bytes());
        out.extend_from_slice(&self.plan_item_index_plus1.to_le_bytes());
        out.extend_from_slice(&self.published_remote_task_index.to_le_bytes());
        out.extend_from_slice(&self.created_at_s.to_le_bytes());
        out.extend_from_slice(&self.updated_at_s.to_le_bytes());
        out.extend_from_slice(&self.plan_linked_at_s.to_le_bytes());
        out.extend_from_slice(&self.published_at_s.to_le_bytes());
        out.extend_from_slice(&self.closed_at_s.to_le_bytes());
        require_encoded_size(&out, LOCAL_TASK_RECORD_SIZE, "LocalTaskRecord")?;
        Ok(out)
    }

    pub(crate) fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(raw, LOCAL_TASK_RECORD_SIZE, "LocalTaskRecord")?;
        let record = Self {
            task_meta: raw[0],
            local_meta: raw[1],
            payload_len: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
            payload_offset: u64::from_le_bytes(raw[4..12].try_into().unwrap()),
            origin_plan_revision_index_plus1: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            plan_item_index_plus1: u32::from_le_bytes(raw[16..20].try_into().unwrap()),
            published_remote_task_index: u32::from_le_bytes(raw[20..24].try_into().unwrap()),
            created_at_s: u64::from_le_bytes(raw[24..32].try_into().unwrap()),
            updated_at_s: u64::from_le_bytes(raw[32..40].try_into().unwrap()),
            plan_linked_at_s: u64::from_le_bytes(raw[40..48].try_into().unwrap()),
            published_at_s: u64::from_le_bytes(raw[48..56].try_into().unwrap()),
            closed_at_s: u64::from_le_bytes(raw[56..64].try_into().unwrap()),
        };
        validate_local_task_record(&record)?;
        Ok(record)
    }

    fn is_published(&self) -> bool {
        self.local_meta & 1 != 0
    }

    fn is_terminal(&self) -> bool {
        self.task_meta & (0b0100_0000 | 0b1000_0000) != 0 || self.local_meta & 0b10 != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalChangeRecord {
    pub change_meta: u8,
    pub local_meta: u8,
    pub payload_len: u16,
    pub change_ordinal: u8,
    pub change_state: u8,
    pub reserved1: u16,
    pub payload_offset: u64,
    pub task_index: u32,
    pub previous_change_index_plus1: u32,
    pub fork_snapshot_index_plus1: u32,
    pub published_remote_change_ordinal_plus1: u8,
    pub reserved2: u8,
    pub reserved3: u16,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    pub published_at_s: u64,
    pub base_line_index_plus1: u32,
    pub archived_at_s: u64,
}

impl LocalChangeRecord {
    pub(crate) fn encode(&self) -> StoreResult<Vec<u8>> {
        validate_local_change_record(self)?;
        let mut out = Vec::with_capacity(LOCAL_CHANGE_RECORD_SIZE as usize);
        out.push(self.change_meta);
        out.push(self.local_meta);
        out.extend_from_slice(&self.payload_len.to_le_bytes());
        out.push(self.change_ordinal);
        out.push(self.change_state);
        out.extend_from_slice(&self.reserved1.to_le_bytes());
        out.extend_from_slice(&self.payload_offset.to_le_bytes());
        out.extend_from_slice(&self.task_index.to_le_bytes());
        out.extend_from_slice(&self.previous_change_index_plus1.to_le_bytes());
        out.extend_from_slice(&self.fork_snapshot_index_plus1.to_le_bytes());
        out.push(self.published_remote_change_ordinal_plus1);
        out.push(self.reserved2);
        out.extend_from_slice(&self.reserved3.to_le_bytes());
        out.extend_from_slice(&self.created_at_s.to_le_bytes());
        out.extend_from_slice(&self.updated_at_s.to_le_bytes());
        out.extend_from_slice(&self.published_at_s.to_le_bytes());
        out.extend_from_slice(&self.base_line_index_plus1.to_le_bytes());
        out.extend_from_slice(&self.archived_at_s.to_le_bytes());
        require_encoded_size(&out, LOCAL_CHANGE_RECORD_SIZE, "LocalChangeRecord")?;
        Ok(out)
    }

    pub(crate) fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(raw, LOCAL_CHANGE_RECORD_SIZE, "LocalChangeRecord")?;
        let record = Self {
            change_meta: raw[0],
            local_meta: raw[1],
            payload_len: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
            change_ordinal: raw[4],
            change_state: raw[5],
            reserved1: u16::from_le_bytes(raw[6..8].try_into().unwrap()),
            payload_offset: u64::from_le_bytes(raw[8..16].try_into().unwrap()),
            task_index: u32::from_le_bytes(raw[16..20].try_into().unwrap()),
            previous_change_index_plus1: u32::from_le_bytes(raw[20..24].try_into().unwrap()),
            fork_snapshot_index_plus1: u32::from_le_bytes(raw[24..28].try_into().unwrap()),
            published_remote_change_ordinal_plus1: raw[28],
            reserved2: raw[29],
            reserved3: u16::from_le_bytes(raw[30..32].try_into().unwrap()),
            created_at_s: u64::from_le_bytes(raw[32..40].try_into().unwrap()),
            updated_at_s: u64::from_le_bytes(raw[40..48].try_into().unwrap()),
            published_at_s: u64::from_le_bytes(raw[48..56].try_into().unwrap()),
            base_line_index_plus1: u32::from_le_bytes(raw[56..60].try_into().unwrap()),
            archived_at_s: u64::from_le_bytes(raw[60..68].try_into().unwrap()),
        };
        validate_local_change_record(&record)?;
        Ok(record)
    }

    fn is_published(&self) -> bool {
        self.local_meta & 1 != 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TaskChangeIndexRecord {
    pub latest_change_index_plus1: u32,
    pub change_count: u16,
    pub next_change_ordinal: u8,
    pub reserved0: u8,
}

impl TaskChangeIndexRecord {
    pub(crate) fn encode(self) -> Vec<u8> {
        let mut out = self.latest_change_index_plus1.to_le_bytes().to_vec();
        out.extend_from_slice(&self.change_count.to_le_bytes());
        out.push(self.next_change_ordinal);
        out.push(self.reserved0);
        out
    }

    pub(crate) fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(raw, TASK_CHANGE_INDEX_RECORD_SIZE, "TaskChangeIndexRecord")?;
        let record = Self {
            latest_change_index_plus1: u32::from_le_bytes(raw[0..4].try_into().unwrap()),
            change_count: u16::from_le_bytes(raw[4..6].try_into().unwrap()),
            next_change_ordinal: raw[6],
            reserved0: raw[7],
        };
        if record.reserved0 != 0 || record.next_change_ordinal > 64 {
            return Err("TaskChangeIndexRecord has reserved or invalid ordinal data".into());
        }
        Ok(record)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TaskLandIndexRecord {
    pub latest_land_index_plus1: u32,
    pub land_count: u16,
    pub reserved0: u16,
}

impl TaskLandIndexRecord {
    pub(crate) fn encode(self) -> Vec<u8> {
        let mut out = self.latest_land_index_plus1.to_le_bytes().to_vec();
        out.extend_from_slice(&self.land_count.to_le_bytes());
        out.extend_from_slice(&self.reserved0.to_le_bytes());
        out
    }

    pub(crate) fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(raw, TASK_LAND_INDEX_RECORD_SIZE, "TaskLandIndexRecord")?;
        let record = Self {
            latest_land_index_plus1: u32::from_le_bytes(raw[0..4].try_into().unwrap()),
            land_count: u16::from_le_bytes(raw[4..6].try_into().unwrap()),
            reserved0: u16::from_le_bytes(raw[6..8].try_into().unwrap()),
        };
        if record.reserved0 != 0 {
            return Err("TaskLandIndexRecord.reserved0 must be zero".into());
        }
        Ok(record)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ChangeLandIndexRecord {
    pub latest_land_index_plus1: u32,
    pub land_count: u16,
    pub next_land_ordinal: u8,
    pub reserved0: u8,
}

impl ChangeLandIndexRecord {
    pub(crate) fn encode(self) -> Vec<u8> {
        let mut out = self.latest_land_index_plus1.to_le_bytes().to_vec();
        out.extend_from_slice(&self.land_count.to_le_bytes());
        out.push(self.next_land_ordinal);
        out.push(self.reserved0);
        out
    }

    pub(crate) fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(raw, CHANGE_LAND_INDEX_RECORD_SIZE, "ChangeLandIndexRecord")?;
        let record = Self {
            latest_land_index_plus1: u32::from_le_bytes(raw[0..4].try_into().unwrap()),
            land_count: u16::from_le_bytes(raw[4..6].try_into().unwrap()),
            next_land_ordinal: raw[6],
            reserved0: raw[7],
        };
        if record.reserved0 != 0 || record.next_land_ordinal > 64 {
            return Err("ChangeLandIndexRecord has reserved or invalid ordinal data".into());
        }
        Ok(record)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LocalLandRecord {
    pub land_meta: u8,
    pub land_ordinal: u8,
    pub change_ordinal: u8,
    pub failure_kind: u8,
    pub change_index: u32,
    pub previous_task_land_index_plus1: u32,
    pub previous_change_land_index_plus1: u32,
    pub pre_land_target_snapshot_index_plus1: u32,
    pub landed_snapshot_index_plus1: u32,
    pub submitted_at_s: u64,
    pub updated_at_s: u64,
    pub target_line_index_plus1: u32,
}

impl LocalLandRecord {
    pub(crate) fn encode(self) -> StoreResult<Vec<u8>> {
        validate_local_land_record(&self)?;
        let mut out = Vec::with_capacity(LOCAL_LAND_RECORD_SIZE as usize);
        out.push(self.land_meta);
        out.push(self.land_ordinal);
        out.push(self.change_ordinal);
        out.push(self.failure_kind);
        out.extend_from_slice(&self.change_index.to_le_bytes());
        out.extend_from_slice(&self.previous_task_land_index_plus1.to_le_bytes());
        out.extend_from_slice(&self.previous_change_land_index_plus1.to_le_bytes());
        out.extend_from_slice(&self.pre_land_target_snapshot_index_plus1.to_le_bytes());
        out.extend_from_slice(&self.landed_snapshot_index_plus1.to_le_bytes());
        out.extend_from_slice(&self.submitted_at_s.to_le_bytes());
        out.extend_from_slice(&self.updated_at_s.to_le_bytes());
        out.extend_from_slice(&self.target_line_index_plus1.to_le_bytes());
        require_encoded_size(&out, LOCAL_LAND_RECORD_SIZE, "LocalLandRecord")?;
        Ok(out)
    }

    pub(crate) fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(raw, LOCAL_LAND_RECORD_SIZE, "LocalLandRecord")?;
        let record = Self {
            land_meta: raw[0],
            land_ordinal: raw[1],
            change_ordinal: raw[2],
            failure_kind: raw[3],
            change_index: u32::from_le_bytes(raw[4..8].try_into().unwrap()),
            previous_task_land_index_plus1: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
            previous_change_land_index_plus1: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            pre_land_target_snapshot_index_plus1: u32::from_le_bytes(
                raw[16..20].try_into().unwrap(),
            ),
            landed_snapshot_index_plus1: u32::from_le_bytes(raw[20..24].try_into().unwrap()),
            submitted_at_s: u64::from_le_bytes(raw[24..32].try_into().unwrap()),
            updated_at_s: u64::from_le_bytes(raw[32..40].try_into().unwrap()),
            target_line_index_plus1: u32::from_le_bytes(raw[40..44].try_into().unwrap()),
        };
        validate_local_land_record(&record)?;
        Ok(record)
    }

    fn status_kind(self) -> u8 {
        self.land_meta & LAND_META_STATUS_MASK
    }

    fn is_succeeded(self) -> bool {
        self.status_kind() == 2 && self.land_meta & LAND_META_TOMBSTONE == 0
    }
}

#[derive(Clone, Debug, Default)]
struct WorkflowRows {
    tasks: BTreeMap<String, JsonValue>,
    changes: BTreeMap<String, JsonValue>,
    task_record_indexes: BTreeMap<String, u32>,
    change_record_indexes: BTreeMap<String, u32>,
}

struct LocalTaskCreateInput<'a> {
    title: String,
    intent: String,
    planned: bool,
    revision_plus1: u32,
    item_plus1: u32,
    plan_linked_at_s: u64,
    status: &'a str,
}

#[derive(Clone, Debug)]
pub struct BinaryDbWorkflowStore<B: BinaryDb, const WRITE_LAYOUT: u32> {
    db: B,
    repo_name: String,
    id_namespace_prefix: String,
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> BinaryDbWorkflowStore<B, WRITE_LAYOUT> {
    pub fn new(db: B, repo_name: impl Into<String>) -> Self {
        Self::new_with_namespace(db, repo_name, "")
    }

    pub fn new_with_namespace(
        db: B,
        repo_name: impl Into<String>,
        id_namespace_prefix: impl Into<String>,
    ) -> Self {
        Self {
            db,
            repo_name: repo_name.into(),
            id_namespace_prefix: id_namespace_prefix.into().trim().to_ascii_uppercase(),
        }
    }

    pub fn db(&self) -> &B {
        &self.db
    }

    pub fn task_record_file() -> BinaryFileId {
        fixed_file::<WRITE_LAYOUT>(TASK_RECORD_BIN, LOCAL_TASK_RECORD_SIZE)
    }

    pub fn task_payload_file() -> BinaryPayloadFileId {
        payload_file::<WRITE_LAYOUT>(TASK_PAYLOAD_BIN)
    }

    pub fn task_change_index_file() -> BinaryFileId {
        fixed_file::<WRITE_LAYOUT>(TASK_CHANGE_INDEX_BIN, TASK_CHANGE_INDEX_RECORD_SIZE)
    }

    pub fn task_land_index_file() -> BinaryFileId {
        fixed_file::<WRITE_LAYOUT>(TASK_LAND_INDEX_BIN, TASK_LAND_INDEX_RECORD_SIZE)
    }

    pub fn change_record_file() -> BinaryFileId {
        fixed_file::<WRITE_LAYOUT>(CHANGE_RECORD_BIN, LOCAL_CHANGE_RECORD_SIZE)
    }

    pub fn change_payload_file() -> BinaryPayloadFileId {
        payload_file::<WRITE_LAYOUT>(CHANGE_PAYLOAD_BIN)
    }

    pub fn change_land_index_file() -> BinaryFileId {
        fixed_file::<WRITE_LAYOUT>(CHANGE_LAND_INDEX_BIN, CHANGE_LAND_INDEX_RECORD_SIZE)
    }

    pub fn land_record_file() -> BinaryFileId {
        fixed_file::<WRITE_LAYOUT>(LAND_RECORD_BIN, LOCAL_LAND_RECORD_SIZE)
    }

    fn local_namespace(&self) -> PlanStoreResult<String> {
        workflow_origin_namespace_prefix("L", Some(&self.id_namespace_prefix))
            .map_err(PlanStoreError::Invalid)
    }

    fn remote_namespace(&self) -> PlanStoreResult<String> {
        workflow_origin_namespace_prefix("R", Some(&self.id_namespace_prefix))
            .map_err(PlanStoreError::Invalid)
    }

    fn local_task_id(&self, task_index: u32) -> StoreResult<String> {
        let sequence = task_index
            .checked_add(1)
            .ok_or_else(|| "Task sequence overflow".to_string())?;
        generate_namespaced_sequence_id(
            "T",
            i64::from(sequence),
            Some(
                &workflow_origin_namespace_prefix("L", Some(&self.id_namespace_prefix))
                    .map_err(|error| format!("invalid local namespace: {error}"))?,
            ),
            4,
        )
        .map_err(Into::into)
    }

    fn remote_task_id(&self, task_index: u32) -> StoreResult<String> {
        let sequence = task_index
            .checked_add(1)
            .ok_or_else(|| "Remote Task sequence overflow".to_string())?;
        generate_namespaced_sequence_id(
            "T",
            i64::from(sequence),
            Some(
                &workflow_origin_namespace_prefix("R", Some(&self.id_namespace_prefix))
                    .map_err(|error| format!("invalid remote namespace: {error}"))?,
            ),
            4,
        )
        .map_err(Into::into)
    }

    fn read_rows(&self) -> PlanStoreResult<WorkflowRows> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.read_rows_with_access(&read).map_err(storage_error)
    }

    pub(crate) fn validate_detached_authority(&self) -> PlanStoreResult<(usize, usize)> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let rows = self
            .read_rows_with_access(&UnlockedWorkflowReadAccess(&self.db))
            .map_err(storage_error)?;
        Ok((rows.tasks.len(), rows.changes.len()))
    }

    fn read_rows_with_access<A: WorkflowReadAccess>(
        &self,
        access: &A,
    ) -> StoreResult<WorkflowRows> {
        let task_count = access.record_count(Self::task_record_file())?;
        require_aligned_count(
            access,
            Self::task_change_index_file(),
            task_count,
            TASK_CHANGE_INDEX_BIN,
        )?;
        require_aligned_count(
            access,
            Self::task_land_index_file(),
            task_count,
            TASK_LAND_INDEX_BIN,
        )?;
        let change_count = access.record_count(Self::change_record_file())?;
        require_aligned_count(
            access,
            Self::change_land_index_file(),
            change_count,
            CHANGE_LAND_INDEX_BIN,
        )?;
        let land_count = access.record_count(Self::land_record_file())?;

        let mut rows = WorkflowRows::default();
        for task_index in 0..task_count {
            let record = LocalTaskRecord::decode(
                &access.read_record(Self::task_record_file(), task_index)?,
            )?;
            let row = self.project_task(access, task_index, &record)?;
            let task_id = required_row_text(&row, "task_id", "Task")?;
            rows.task_record_indexes.insert(task_id.clone(), task_index);
            if rows.tasks.insert(task_id.clone(), row).is_some() {
                return Err(format!("Binary DB Task identity is duplicated: {task_id}").into());
            }
        }

        let mut change_records = Vec::with_capacity(change_count as usize);
        for change_index in 0..change_count {
            let record = LocalChangeRecord::decode(
                &access.read_record(Self::change_record_file(), change_index)?,
            )?;
            if record.task_index >= task_count {
                return Err(format!(
                    "Change {change_index} references missing Task {}",
                    record.task_index
                )
                .into());
            }
            let row = self.project_change(access, change_index, &record)?;
            let change_ref = required_row_text(&row, "change_ref", "Change")?;
            rows.change_record_indexes
                .insert(change_ref.clone(), change_index);
            if rows.changes.insert(change_ref.clone(), row).is_some() {
                return Err(
                    format!("Binary DB Change reference is duplicated: {change_ref}").into(),
                );
            }
            change_records.push(record);
        }
        self.validate_owner_indexes(access, task_count, &change_records, land_count)?;
        Ok(rows)
    }

    fn project_task<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task_index: u32,
        record: &LocalTaskRecord,
    ) -> StoreResult<JsonValue> {
        let payload = access.read_payload(
            Self::task_payload_file(),
            record.payload_offset,
            u32::from(record.payload_len),
        )?;
        let (title, intent) = decode_task_payload(&payload)?;
        let task_id = self.local_task_id(task_index)?;
        let (plan_id, revision_id, item_ref) = self.project_task_plan_binding(access, record)?;
        let status = if record.task_meta & 0b1000_0000 != 0 {
            "canceled"
        } else if record.local_meta & 0b10 != 0 {
            "abandoned"
        } else if record.task_meta & 0b0100_0000 != 0 {
            "completed"
        } else {
            "active"
        };
        let published_task_id = if record.is_published() {
            Some(self.remote_task_id(record.published_remote_task_index)?)
        } else {
            None
        };
        Ok(json!({
            "task_id": task_id,
            "task_seq": task_index + 1,
            "repo_name": self.repo_name,
            "title": title,
            "intent": intent,
            "status": status,
            "publication_state": if record.is_published() { "published" } else { "local_draft" },
            "identity_source": "local",
            "planning_state": if record.task_meta & 1 != 0 { "planned" } else { "unplanned" },
            "plan_id": option_string(plan_id),
            "origin_plan_revision_id": option_string(revision_id),
            "plan_item_ref": option_string(item_ref),
            "plan_linked_at": timestamp_json(record.plan_linked_at_s)?,
            "published_remote_name": if record.is_published() { json!("origin") } else { JsonValue::Null },
            "published_task_id": option_string(published_task_id),
            "published_at": timestamp_json(record.published_at_s)?,
            "closed_at": timestamp_json(record.closed_at_s)?,
            "created_at": timestamp_required(record.created_at_s, "Task.created_at_s")?,
            "updated_at": timestamp_required(record.updated_at_s, "Task.updated_at_s")?,
        }))
    }

    fn project_task_plan_binding<A: WorkflowReadAccess>(
        &self,
        access: &A,
        record: &LocalTaskRecord,
    ) -> StoreResult<(Option<String>, Option<String>, Option<String>)> {
        let Some(revision_index) = record.origin_plan_revision_index_plus1.checked_sub(1) else {
            if record.plan_item_index_plus1 != 0 || record.plan_linked_at_s != 0 {
                return Err("Task has Plan item/time without Plan revision authority".into());
            }
            return Ok((None, None, None));
        };
        let revision = PlanRevisionCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_record(
            &access.read_record(
                PlanRevisionCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file(),
                revision_index,
            )?,
        )?;
        let plan_id = repository_plan_id(revision.plan_index);
        let item_ref = match record.plan_item_index_plus1.checked_sub(1) {
            None => None,
            Some(item_index) => {
                let item_end = revision
                    .item_start_index
                    .checked_add(u32::from(revision.item_count))
                    .ok_or_else(|| "Plan Item range overflow".to_string())?;
                if item_index < revision.item_start_index || item_index >= item_end {
                    return Err(format!(
                        "Task Plan Item {item_index} is outside revision {revision_index} range"
                    )
                    .into());
                }
                let item = PlanItemCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_record(
                    &access.read_record(
                        PlanItemCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file(),
                        item_index,
                    )?,
                )?;
                let payload = PlanItemCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_payload(
                    &access.read_payload(
                        PlanItemCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::payload_file(),
                        item.payload_offset,
                        u32::from(item.payload_len),
                    )?,
                )?;
                let value = std::str::from_utf8(&payload.plan_item_ref_bytes)
                    .map_err(|_| "Task Plan Item ref is not UTF-8")?
                    .to_string();
                if value.is_empty() {
                    return Err("Task references a Plan Item without an item ref".into());
                }
                Some(value)
            }
        };
        Ok((
            Some(plan_id),
            Some(format!("plan-revision:{revision_index}")),
            item_ref,
        ))
    }

    fn project_change<A: WorkflowReadAccess>(
        &self,
        access: &A,
        change_index: u32,
        record: &LocalChangeRecord,
    ) -> StoreResult<JsonValue> {
        let title_bytes = access.read_payload(
            Self::change_payload_file(),
            record.payload_offset,
            u32::from(record.payload_len),
        )?;
        let title = std::str::from_utf8(&title_bytes)
            .map_err(|_| format!("Change {change_index} title is not UTF-8"))?
            .to_string();
        if title.trim().is_empty() {
            return Err(format!("Change {change_index} title must not be empty").into());
        }
        let base_line_index = record
            .base_line_index_plus1
            .checked_sub(1)
            .ok_or_else(|| format!("Change {change_index} has no base Line"))?;
        let base_line = line_name_at(access, base_line_index)?;
        let task_id = self.local_task_id(record.task_index)?;
        let change_id = render_change_id(record.change_ordinal);
        let change_ref = format!("{task_id}/{change_id}");
        let fork_snapshot_id = record
            .fork_snapshot_index_plus1
            .checked_sub(1)
            .map(|index| snapshot_id_at(access, index))
            .transpose()?;
        let successful_land = self.latest_successful_land(access, change_index)?;
        let (target_line, pre_land_snapshot, landed_snapshot, landed_at) =
            if let Some(land) = successful_land {
                (
                    Some(line_name_at(
                        access,
                        land.target_line_index_plus1
                            .checked_sub(1)
                            .expect("validated target Line"),
                    )?),
                    land.pre_land_target_snapshot_index_plus1
                        .checked_sub(1)
                        .map(|index| snapshot_id_at(access, index))
                        .transpose()?,
                    land.landed_snapshot_index_plus1
                        .checked_sub(1)
                        .map(|index| snapshot_id_at(access, index))
                        .transpose()?,
                    timestamp_json(land.updated_at_s)?,
                )
            } else {
                (None, None, None, JsonValue::Null)
            };
        let lifecycle_kind = record.change_meta & CHANGE_META_LIFECYCLE_MASK;
        let status = match lifecycle_kind {
            0 => "draft",
            1 if record.change_meta & 0b0000_1000 != 0 => "review",
            1 => "active",
            2 => "landed",
            3 if record.change_state & 1 != 0 => "canceled",
            3 if record.change_meta & 0b1000_0000 != 0 => "superseded",
            3 => "archived",
            _ => unreachable!(),
        };
        if status == "landed" && landed_snapshot.is_none() {
            return Err(
                format!("landed Change {change_index} has no successful LocalLandRecord").into(),
            );
        }
        let published_change_id = if record.is_published() {
            let remote_task = self.remote_task_id(
                self.read_task_record(access, record.task_index)?
                    .published_remote_task_index,
            )?;
            Some(format!(
                "{remote_task}/{}",
                render_change_id(
                    record
                        .published_remote_change_ordinal_plus1
                        .checked_sub(1)
                        .ok_or_else(|| "published Change has no Remote ordinal".to_string())?,
                )
            ))
        } else {
            None
        };
        Ok(json!({
            "change_id": change_id,
            "change_ref": change_ref,
            "change_seq": u32::from(record.change_ordinal) + 1,
            "task_id": task_id,
            "repo_name": self.repo_name,
            "title": title,
            "base_line": base_line,
            "status": status,
            "publication_state": if record.is_published() { "published" } else { "local_draft" },
            "identity_source": "local",
            "fork_snapshot_id": option_string(fork_snapshot_id),
            "forked_from_line": line_name_at(access, base_line_index)?,
            "target_line": option_string(target_line),
            "landed_snapshot_id": option_string(landed_snapshot),
            "pre_land_target_snapshot_id": option_string(pre_land_snapshot),
            "landed_at": landed_at,
            "archived_at": timestamp_json(record.archived_at_s)?,
            "published_remote_name": if record.is_published() { json!("origin") } else { JsonValue::Null },
            "published_change_id": option_string(published_change_id),
            "published_at": timestamp_json(record.published_at_s)?,
            "created_at": timestamp_required(record.created_at_s, "Change.created_at_s")?,
            "updated_at": timestamp_required(record.updated_at_s, "Change.updated_at_s")?,
        }))
    }

    fn latest_successful_land<A: WorkflowReadAccess>(
        &self,
        access: &A,
        change_index: u32,
    ) -> StoreResult<Option<LocalLandRecord>> {
        let head = ChangeLandIndexRecord::decode(
            &access.read_record(Self::change_land_index_file(), change_index)?,
        )?;
        let mut cursor = head.latest_land_index_plus1;
        let mut visited = 0_u16;
        let mut best: Option<LocalLandRecord> = None;
        while let Some(land_index) = cursor.checked_sub(1) {
            let land = LocalLandRecord::decode(
                &access.read_record(Self::land_record_file(), land_index)?,
            )?;
            if land.change_index != change_index {
                return Err(format!(
                    "Change {change_index} Land chain contains Land {land_index} owned by {}",
                    land.change_index
                )
                .into());
            }
            if land.is_succeeded()
                && best
                    .as_ref()
                    .map(|current| land.land_ordinal > current.land_ordinal)
                    .unwrap_or(true)
            {
                best = Some(land);
            }
            cursor = land.previous_change_land_index_plus1;
            visited = visited
                .checked_add(1)
                .ok_or_else(|| "Change Land chain count overflow".to_string())?;
            if visited > head.land_count {
                return Err(format!("Change {change_index} Land chain contains a cycle").into());
            }
        }
        if visited != head.land_count {
            return Err(format!(
                "Change {change_index} Land index count {} disagrees with chain {visited}",
                head.land_count
            )
            .into());
        }
        Ok(best)
    }

    fn validate_owner_indexes<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task_count: u32,
        changes: &[LocalChangeRecord],
        land_count: u32,
    ) -> StoreResult<()> {
        let mut changes_by_task = vec![Vec::<u32>::new(); task_count as usize];
        for (index, change) in changes.iter().enumerate() {
            changes_by_task[change.task_index as usize].push(index as u32);
        }
        for (task_index, owned) in changes_by_task.iter().enumerate() {
            let head = TaskChangeIndexRecord::decode(
                &access.read_record(Self::task_change_index_file(), task_index as u32)?,
            )?;
            if usize::from(head.change_count) != owned.len() {
                return Err(format!(
                    "Task {task_index} Change count {} disagrees with {} records",
                    head.change_count,
                    owned.len()
                )
                .into());
            }
            let mut cursor = head.latest_change_index_plus1;
            for expected in owned.iter().rev() {
                let actual = cursor
                    .checked_sub(1)
                    .ok_or_else(|| format!("Task {task_index} Change chain ends early"))?;
                if actual != *expected {
                    return Err(format!(
                        "Task {task_index} Change chain expected {expected}, got {actual}"
                    )
                    .into());
                }
                cursor = changes[actual as usize].previous_change_index_plus1;
            }
            if cursor != 0 || usize::from(head.next_change_ordinal) != owned.len() {
                return Err(format!("Task {task_index} Change index is inconsistent").into());
            }
        }
        for land_index in 0..land_count {
            let land = LocalLandRecord::decode(
                &access.read_record(Self::land_record_file(), land_index)?,
            )?;
            let change = changes.get(land.change_index as usize).ok_or_else(|| {
                format!(
                    "Land {land_index} references missing Change {}",
                    land.change_index
                )
            })?;
            if land.change_ordinal != change.change_ordinal {
                return Err(
                    format!("Land {land_index} Change ordinal disagrees with owner").into(),
                );
            }
        }
        Ok(())
    }

    fn read_task_record<A: WorkflowReadAccess>(
        &self,
        access: &A,
        task_index: u32,
    ) -> StoreResult<LocalTaskRecord> {
        LocalTaskRecord::decode(&access.read_record(Self::task_record_file(), task_index)?)
    }

    fn task_index_for_id(&self, rows: &WorkflowRows, task_id: &str) -> PlanStoreResult<u32> {
        rows.task_record_indexes
            .get(task_id)
            .copied()
            .ok_or_else(|| unknown_row("task", task_id))
    }

    fn change_index_for_id(&self, rows: &WorkflowRows, change_id: &str) -> PlanStoreResult<u32> {
        let key = resolve_change_key(&rows.changes, change_id)?;
        rows.change_record_indexes
            .get(&key)
            .copied()
            .ok_or_else(|| unknown_row("change", change_id))
    }

    fn resolve_plan_binding<A: WorkflowReadAccess>(
        &self,
        access: &A,
        plan_id: Option<&str>,
        revision_id: Option<&str>,
        item_ref: Option<&str>,
    ) -> PlanStoreResult<(u32, u32)> {
        let Some(plan_id) = nonempty(plan_id) else {
            if nonempty(revision_id).is_some() || nonempty(item_ref).is_some() {
                return Err(PlanStoreError::Invalid(
                    "origin_plan_revision_id and plan_item_ref require plan_id".to_string(),
                ));
            }
            return Ok((0, 0));
        };
        let plan_index = parse_repository_plan_id(plan_id).map_err(PlanStoreError::Invalid)?;
        let revision_text = nonempty(revision_id).ok_or_else(|| {
            PlanStoreError::Invalid("planned Task requires origin_plan_revision_id".to_string())
        })?;
        let revision_index = parse_revision_id(revision_text)?;
        let revision = PlanRevisionCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_record(
            &access
                .read_record(
                    PlanRevisionCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file(),
                    revision_index,
                )
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        if revision.plan_index != plan_index {
            return Err(PlanStoreError::Invalid(format!(
                "{revision_text} belongs to PR-{}, not {plan_id}",
                revision.plan_index
            )));
        }
        let item_plus1 = match nonempty(item_ref) {
            None => 0,
            Some(expected) => {
                let mut found = None;
                for item_index in revision.item_start_index
                    ..revision
                        .item_start_index
                        .checked_add(u32::from(revision.item_count))
                        .ok_or_else(|| {
                            PlanStoreError::Storage("Plan Item range overflow".to_string())
                        })?
                {
                    let item = PlanItemCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_record(
                        &access
                            .read_record(
                                PlanItemCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file(),
                                item_index,
                            )
                            .map_err(storage_error)?,
                    )
                    .map_err(storage_error)?;
                    let payload = PlanItemCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_payload(
                        &access
                            .read_payload(
                                PlanItemCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::payload_file(),
                                item.payload_offset,
                                u32::from(item.payload_len),
                            )
                            .map_err(storage_error)?,
                    )
                    .map_err(storage_error)?;
                    if payload.plan_item_ref_bytes == expected.as_bytes()
                        && found.replace(item_index).is_some()
                    {
                        return Err(PlanStoreError::Invalid(format!(
                            "Plan Item ref {expected} is ambiguous in {revision_text}"
                        )));
                    }
                }
                found
                    .ok_or_else(|| {
                        PlanStoreError::Invalid(format!(
                            "Plan Item ref {expected} is absent from {revision_text}"
                        ))
                    })?
                    .checked_add(1)
                    .ok_or_else(|| {
                        PlanStoreError::Storage("Plan Item index overflow".to_string())
                    })?
            }
        };
        Ok((
            revision_index.checked_add(1).ok_or_else(|| {
                PlanStoreError::Storage("Plan revision index overflow".to_string())
            })?,
            item_plus1,
        ))
    }

    fn create_task_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        input: LocalTaskCreateInput<'_>,
    ) -> PlanStoreResult<(u32, LocalTaskRecord)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let payload = encode_task_payload(&input.title, &input.intent).map_err(storage_error)?;
        let range = write
            .append_payload(Self::task_payload_file(), &payload)
            .map_err(storage_error)?;
        let now = now_s()?;
        let mut task_meta = u8::from(input.planned);
        let mut local_meta = 0_u8;
        let closed_at_s = match input.status {
            "active" => 0,
            "completed" => {
                task_meta |= 0b0100_0000;
                now
            }
            "canceled" => {
                task_meta |= 0b1000_0000;
                now
            }
            "abandoned" => {
                local_meta |= 0b10;
                now
            }
            other => {
                return Err(PlanStoreError::Invalid(format!(
                    "Unsupported Task status: {other}"
                )))
            }
        };
        let record = LocalTaskRecord {
            task_meta,
            local_meta,
            payload_len: u16::try_from(range.payload_len)
                .map_err(|_| PlanStoreError::Invalid("Task payload exceeds u16".to_string()))?,
            payload_offset: range.payload_offset,
            origin_plan_revision_index_plus1: input.revision_plus1,
            plan_item_index_plus1: input.item_plus1,
            published_remote_task_index: 0,
            created_at_s: now,
            updated_at_s: now,
            plan_linked_at_s: input.plan_linked_at_s,
            published_at_s: 0,
            closed_at_s,
        };
        let task_index = write
            .append_record(
                Self::task_change_index_file(),
                &TaskChangeIndexRecord::default().encode(),
            )
            .map_err(storage_error)?;
        let land_owner_index = write
            .append_record(
                Self::task_land_index_file(),
                &TaskLandIndexRecord::default().encode(),
            )
            .map_err(storage_error)?;
        if land_owner_index != task_index {
            return Err(PlanStoreError::Storage(
                "Task owner-index alignment was lost".to_string(),
            ));
        }
        let actual = write
            .append_record(
                Self::task_record_file(),
                &record.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        if actual != task_index {
            return Err(PlanStoreError::Storage(
                "Task record alignment was lost".to_string(),
            ));
        }
        Ok((actual, record))
    }

    fn create_change_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        task_index: u32,
        title: &str,
        base_line: &str,
        fork_snapshot_id: Option<&str>,
        status: &str,
    ) -> PlanStoreResult<(u32, LocalChangeRecord)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let base_line_index = line_index_by_name(write, base_line)
            .map_err(storage_error)?
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown line: {base_line}")))?;
        let fork_snapshot_index_plus1 = match nonempty(fork_snapshot_id) {
            None => 0,
            Some(snapshot_id) => snapshot_index_by_id(write, snapshot_id)
                .map_err(storage_error)?
                .ok_or_else(|| {
                    PlanStoreError::NotFound(format!("Unknown Snapshot: {snapshot_id}"))
                })?
                .checked_add(1)
                .ok_or_else(|| PlanStoreError::Storage("Snapshot index overflow".to_string()))?,
        };
        let mut owner = TaskChangeIndexRecord::decode(
            &write
                .read_record(Self::task_change_index_file(), task_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        if owner.next_change_ordinal > MAX_CHANGE_ORDINAL {
            return Err(PlanStoreError::Invalid(format!(
                "Task {} already owns the maximum 64 Changes",
                self.local_task_id(task_index).map_err(storage_error)?
            )));
        }
        let change_ordinal = owner.next_change_ordinal;
        let title = required_text(title, "title")?;
        let range = write
            .append_payload(Self::change_payload_file(), title.as_bytes())
            .map_err(storage_error)?;
        let now = now_s()?;
        let (change_meta, change_state, archived_at_s) = change_status_bits(status, now)?;
        let record = LocalChangeRecord {
            change_meta,
            local_meta: 0,
            payload_len: u16::try_from(range.payload_len)
                .map_err(|_| PlanStoreError::Invalid("Change title exceeds u16".to_string()))?,
            change_ordinal,
            change_state,
            reserved1: 0,
            payload_offset: range.payload_offset,
            task_index,
            previous_change_index_plus1: owner.latest_change_index_plus1,
            fork_snapshot_index_plus1,
            published_remote_change_ordinal_plus1: 0,
            reserved2: 0,
            reserved3: 0,
            created_at_s: now,
            updated_at_s: now,
            published_at_s: 0,
            base_line_index_plus1: base_line_index
                .checked_add(1)
                .ok_or_else(|| PlanStoreError::Storage("Line index overflow".to_string()))?,
            archived_at_s,
        };
        let change_index = write
            .append_record(
                Self::change_land_index_file(),
                &ChangeLandIndexRecord::default().encode(),
            )
            .map_err(storage_error)?;
        let actual = write
            .append_record(
                Self::change_record_file(),
                &record.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        if actual != change_index {
            return Err(PlanStoreError::Storage(
                "Change record alignment was lost".to_string(),
            ));
        }
        owner.latest_change_index_plus1 = change_index
            .checked_add(1)
            .ok_or_else(|| PlanStoreError::Storage("Change index overflow".to_string()))?;
        owner.change_count = owner
            .change_count
            .checked_add(1)
            .ok_or_else(|| PlanStoreError::Storage("Change count overflow".to_string()))?;
        owner.next_change_ordinal = owner
            .next_change_ordinal
            .checked_add(1)
            .ok_or_else(|| PlanStoreError::Storage("Change ordinal overflow".to_string()))?;
        write
            .overwrite_record(Self::task_change_index_file(), task_index, &owner.encode())
            .map_err(storage_error)?;
        Ok((change_index, record))
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskStore for BinaryDbWorkflowStore<B, WRITE_LAYOUT> {
    fn list_tasks(&self) -> PlanStoreResult<Vec<JsonValue>> {
        Ok(self.read_rows()?.tasks.into_values().collect())
    }

    fn list_completed_tasks_with_landed_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
        let rows = self.read_rows()?;
        let landed = rows
            .changes
            .values()
            .filter(|change| string_value(change, "status").as_deref() == Some("landed"))
            .filter_map(|change| string_value(change, "task_id"))
            .collect::<Vec<_>>();
        Ok(rows
            .tasks
            .into_values()
            .filter(|task| string_value(task, "status").as_deref() == Some("completed"))
            .filter(|task| {
                string_value(task, "task_id")
                    .map(|id| landed.contains(&id))
                    .unwrap_or(false)
            })
            .collect())
    }

    fn get_task(&self, task_id: &str) -> PlanStoreResult<JsonValue> {
        let task_id = required_text(task_id, "task_id")?;
        self.read_rows()?
            .tasks
            .get(&task_id)
            .cloned()
            .ok_or_else(|| unknown_row("task", &task_id))
    }

    fn allocate_task_identity(
        &self,
        repo_name: &str,
        namespace_prefix: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        require_matching_repo(&self.repo_name, repo_name)?;
        self.require_namespace(namespace_prefix)?;
        let task_index = self
            .db
            .record_count(Self::task_record_file())
            .map_err(storage_error)?;
        let task_id = self.local_task_id(task_index).map_err(storage_error)?;
        Ok(json!({
            "task_id": task_id,
            "task_seq": task_index + 1,
            "namespace_prefix": self.local_namespace()?,
            "identity_source": "local",
        }))
    }

    fn sequence_floor(&self, repo_name: &str, family: &str) -> PlanStoreResult<i64> {
        require_matching_repo(&self.repo_name, repo_name)?;
        match family.trim().to_ascii_uppercase().as_str() {
            "T" | "TASK" => Ok(i64::from(
                self.db
                    .record_count(Self::task_record_file())
                    .map_err(storage_error)?,
            )),
            "C" | "CHANGE" => Ok(i64::from(
                self.db
                    .record_count(Self::change_record_file())
                    .map_err(storage_error)?,
            )),
            other => Err(PlanStoreError::Invalid(format!(
                "Unsupported workflow sequence family: {other}"
            ))),
        }
    }

    fn create_task(
        &self,
        repo_name: &str,
        title: &str,
        intent: &str,
        namespace_prefix: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        require_matching_repo(&self.repo_name, repo_name)?;
        self.require_namespace(namespace_prefix)?;
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let (revision_plus1, item_plus1) =
            self.resolve_plan_binding(&write, plan_id, origin_plan_revision_id, plan_item_ref)?;
        let plan_linked_at_s = if revision_plus1 == 0 { 0 } else { now_s()? };
        let (task_index, _) = self.create_task_record(
            &mut write,
            LocalTaskCreateInput {
                title: required_text(title, "title")?,
                intent: required_text(intent, "intent")?,
                planned: plan_id.is_some(),
                revision_plus1,
                item_plus1,
                plan_linked_at_s,
                status: "active",
            },
        )?;
        write.commit().map_err(storage_error)?;
        TaskStore::get_task(
            self,
            &self.local_task_id(task_index).map_err(storage_error)?,
        )
    }

    fn create_task_explicit(
        &self,
        task_id: &str,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_seq: Option<i64>,
        identity_source: Option<&str>,
        planning_state: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
        plan_linked_at: Option<&str>,
        status: Option<&str>,
        publication_state: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        require_matching_repo(&self.repo_name, repo_name)?;
        require_exact_optional(identity_source, "local", "identity_source")?;
        require_exact_optional(publication_state, "local_draft", "publication_state")?;
        let task_index = self
            .db
            .record_count(Self::task_record_file())
            .map_err(storage_error)?;
        let expected_id = self.local_task_id(task_index).map_err(storage_error)?;
        if required_text(task_id, "task_id")? != expected_id {
            return Err(PlanStoreError::Invalid(format!(
                "Task identity must be derived from task.bin ordinal: expected {expected_id}"
            )));
        }
        if let Some(sequence) = task_seq {
            if sequence != i64::from(task_index + 1) {
                return Err(PlanStoreError::Invalid(format!(
                    "Task sequence must be {}, got {sequence}",
                    task_index + 1
                )));
            }
        }
        let planned = match planning_state.unwrap_or(if plan_id.is_some() {
            "planned"
        } else {
            "unplanned"
        }) {
            "planned" => true,
            "unplanned" | "explicit_unplanned" => false,
            other => {
                return Err(PlanStoreError::Invalid(format!(
                    "Unsupported planning_state: {other}"
                )))
            }
        };
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let (revision_plus1, item_plus1) =
            self.resolve_plan_binding(&write, plan_id, origin_plan_revision_id, plan_item_ref)?;
        let linked_at = match nonempty(plan_linked_at) {
            Some(value) => parse_time_s(value, "plan_linked_at")?,
            None if revision_plus1 == 0 => 0,
            None => now_s()?,
        };
        self.create_task_record(
            &mut write,
            LocalTaskCreateInput {
                title: required_text(title, "title")?,
                intent: required_text(intent, "intent")?,
                planned,
                revision_plus1,
                item_plus1,
                plan_linked_at_s: linked_at,
                status: status.unwrap_or("active"),
            },
        )?;
        write.commit().map_err(storage_error)?;
        TaskStore::get_task(self, &expected_id)
    }

    fn close_task(&self, task_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let rows = self.read_rows_with_access(&write).map_err(storage_error)?;
        let task_index = self.task_index_for_id(&rows, task_id)?;
        let mut record = self
            .read_task_record(&write, task_index)
            .map_err(storage_error)?;
        let now = now_s()?;
        record.task_meta &= !(0b0100_0000 | 0b1000_0000);
        record.local_meta &= !0b10;
        match status {
            "completed" => record.task_meta |= 0b0100_0000,
            "canceled" => record.task_meta |= 0b1000_0000,
            "abandoned" => record.local_meta |= 0b10,
            other => {
                return Err(PlanStoreError::Invalid(format!(
                    "Unsupported closed Task status: {other}"
                )))
            }
        }
        record.closed_at_s = now;
        record.updated_at_s = now;
        write
            .overwrite_record(
                Self::task_record_file(),
                task_index,
                &record.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        write.commit().map_err(storage_error)?;
        TaskStore::get_task(self, task_id)
    }

    fn mark_task_published(
        &self,
        task_id: &str,
        remote_name: Option<&str>,
        published_task_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        require_origin_remote(remote_name)?;
        let remote_id = nonempty(published_task_id)
            .ok_or_else(|| PlanStoreError::Invalid("published_task_id is required".to_string()))?;
        let remote_index = self.parse_remote_task_id(remote_id)?;
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let rows = self.read_rows_with_access(&write).map_err(storage_error)?;
        let task_index = self.task_index_for_id(&rows, task_id)?;
        let mut record = self
            .read_task_record(&write, task_index)
            .map_err(storage_error)?;
        if record.is_published() && record.published_remote_task_index != remote_index {
            return Err(PlanStoreError::Concurrency(format!(
                "Task {task_id} is already published to {}",
                self.remote_task_id(record.published_remote_task_index)
                    .map_err(storage_error)?
            )));
        }
        let now = now_s()?;
        record.local_meta |= 1;
        record.published_remote_task_index = remote_index;
        record.published_at_s = record.published_at_s.max(now);
        record.updated_at_s = now;
        write
            .overwrite_record(
                Self::task_record_file(),
                task_index,
                &record.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        write.commit().map_err(storage_error)?;
        TaskStore::get_task(self, task_id)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> BinaryDbWorkflowStore<B, WRITE_LAYOUT> {
    fn require_namespace(&self, requested: Option<&str>) -> PlanStoreResult<()> {
        let requested = requested.unwrap_or("").trim().to_ascii_uppercase();
        if requested == self.id_namespace_prefix {
            Ok(())
        } else {
            Err(PlanStoreError::Invalid(format!(
                "Configured ID namespace is {:?}, not {:?}",
                self.id_namespace_prefix, requested
            )))
        }
    }

    fn parse_remote_task_id(&self, value: &str) -> PlanStoreResult<u32> {
        let token = format!("{}T-", self.remote_namespace()?);
        let raw = value
            .strip_prefix(&token)
            .ok_or_else(|| PlanStoreError::Invalid(format!("Invalid Remote Task ID: {value}")))?;
        let sequence = raw
            .parse::<u32>()
            .map_err(|_| PlanStoreError::Invalid(format!("Invalid Remote Task ID: {value}")))?;
        if sequence == 0 || self.remote_task_id(sequence - 1).map_err(storage_error)? != value {
            return Err(PlanStoreError::Invalid(format!(
                "Remote Task ID is not canonical: {value}"
            )));
        }
        Ok(sequence - 1)
    }

    fn parse_published_change_id(&self, value: &str) -> PlanStoreResult<(u32, u8)> {
        let (task_id, change_id) = value.split_once('/').ok_or_else(|| {
            PlanStoreError::Invalid(format!(
                "Published Change identity must be <Remote Task>/C-##: {value}"
            ))
        })?;
        let task_index = self.parse_remote_task_id(task_id)?;
        let ordinal = parse_change_id(change_id)?;
        Ok((task_index, ordinal))
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> ChangeStore for BinaryDbWorkflowStore<B, WRITE_LAYOUT> {
    fn list_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
        Ok(self.read_rows()?.changes.into_values().collect())
    }

    fn get_change(&self, change_id: &str) -> PlanStoreResult<JsonValue> {
        let rows = self.read_rows()?;
        let key = resolve_change_key(&rows.changes, &required_text(change_id, "change_id")?)?;
        rows.changes
            .get(&key)
            .cloned()
            .ok_or_else(|| unknown_row("change", change_id))
    }

    fn allocate_change_identity(
        &self,
        repo_name: &str,
        namespace_prefix: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        require_matching_repo(&self.repo_name, repo_name)?;
        self.require_namespace(namespace_prefix)?;
        Err(PlanStoreError::Invalid(
            "Task context is required to allocate C-01 through C-64".to_string(),
        ))
    }

    fn create_change(
        &self,
        task_id: &str,
        repo_name: &str,
        title: &str,
        base_line: &str,
        namespace_prefix: Option<&str>,
        fork_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        require_matching_repo(&self.repo_name, repo_name)?;
        self.require_namespace(namespace_prefix)?;
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let rows = self.read_rows_with_access(&write).map_err(storage_error)?;
        let task_index = self.task_index_for_id(&rows, task_id)?;
        let (change_index, record) = self.create_change_record(
            &mut write,
            task_index,
            title,
            &required_text(base_line, "base_line")?,
            fork_snapshot_id,
            "draft",
        )?;
        write.commit().map_err(storage_error)?;
        ChangeStore::get_change(
            self,
            &format!("{task_id}/{}", render_change_id(record.change_ordinal)),
        )
        .map_err(|error| {
            PlanStoreError::Storage(format!(
                "created Change {change_index} could not be read: {error}"
            ))
        })
    }

    fn create_change_explicit(
        &self,
        change_id: &str,
        task_id: &str,
        repo_name: &str,
        title: &str,
        base_line: &str,
        change_seq: Option<i64>,
        identity_source: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
        status: Option<&str>,
        publication_state: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        require_matching_repo(&self.repo_name, repo_name)?;
        require_exact_optional(identity_source, "local", "identity_source")?;
        require_exact_optional(publication_state, "local_draft", "publication_state")?;
        if let Some(forked) = nonempty(forked_from_line) {
            if forked != base_line {
                return Err(PlanStoreError::Invalid(
                    "forked_from_line must equal the fixed base Line".to_string(),
                ));
            }
        }
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let rows = self.read_rows_with_access(&write).map_err(storage_error)?;
        let task_index = self.task_index_for_id(&rows, task_id)?;
        let owner = TaskChangeIndexRecord::decode(
            &write
                .read_record(Self::task_change_index_file(), task_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        let expected_id = render_change_id(owner.next_change_ordinal);
        if required_text(change_id, "change_id")? != expected_id {
            return Err(PlanStoreError::Invalid(format!(
                "Change identity must be derived from Task ordinal: expected {expected_id}"
            )));
        }
        if let Some(sequence) = change_seq {
            if sequence != i64::from(owner.next_change_ordinal) + 1 {
                return Err(PlanStoreError::Invalid(format!(
                    "Change sequence must be {}, got {sequence}",
                    owner.next_change_ordinal + 1
                )));
            }
        }
        let (_, record) = self.create_change_record(
            &mut write,
            task_index,
            title,
            &required_text(base_line, "base_line")?,
            fork_snapshot_id,
            status.unwrap_or("draft"),
        )?;
        write.commit().map_err(storage_error)?;
        ChangeStore::get_change(
            self,
            &format!("{task_id}/{}", render_change_id(record.change_ordinal)),
        )
    }

    fn close_change(&self, change_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
        if !matches!(status, "archived" | "superseded" | "canceled") {
            return Err(PlanStoreError::Invalid(format!(
                "Unsupported closed Change status: {status}"
            )));
        }
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let rows = self.read_rows_with_access(&write).map_err(storage_error)?;
        let change_index = self.change_index_for_id(&rows, change_id)?;
        let mut record = LocalChangeRecord::decode(
            &write
                .read_record(Self::change_record_file(), change_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        if record.change_meta & CHANGE_META_LIFECYCLE_MASK == 2 {
            return Err(PlanStoreError::Invalid(format!(
                "Landed Change {change_id} cannot be archived"
            )));
        }
        let now = now_s()?;
        record.change_meta = (record.change_meta & !CHANGE_META_LIFECYCLE_MASK) | 3;
        record.change_meta &= !0b1000_0000;
        record.change_state &= !1;
        match status {
            "superseded" => record.change_meta |= 0b1000_0000,
            "canceled" => record.change_state |= 1,
            _ => {}
        }
        record.updated_at_s = now;
        record.archived_at_s = now;
        write
            .overwrite_record(
                Self::change_record_file(),
                change_index,
                &record.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        write.commit().map_err(storage_error)?;
        ChangeStore::get_change(self, change_id)
    }

    fn land_change(
        &self,
        change_id: &str,
        target_line: &str,
        landed_snapshot_id: &str,
        pre_land_target_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        let target_line = required_text(target_line, "target_line")?;
        let landed_snapshot_id = required_text(landed_snapshot_id, "landed_snapshot_id")?;
        let existing = ChangeStore::get_change(self, change_id)?;
        if string_value(&existing, "status").as_deref() == Some("landed") {
            if string_value(&existing, "target_line").as_deref() == Some(target_line.as_str())
                && string_value(&existing, "landed_snapshot_id").as_deref()
                    == Some(landed_snapshot_id.as_str())
            {
                return Ok(existing);
            }
            return Err(PlanStoreError::Invalid(format!(
                "Change {change_id} is already landed at a different Snapshot"
            )));
        }
        if string_value(&existing, "publication_state").as_deref() == Some("published") {
            return Err(PlanStoreError::Invalid(format!(
                "Published Change {change_id} cannot be landed through local authority"
            )));
        }
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let rows = self.read_rows_with_access(&write).map_err(storage_error)?;
        let change_index = self.change_index_for_id(&rows, change_id)?;
        let mut change = LocalChangeRecord::decode(
            &write
                .read_record(Self::change_record_file(), change_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        let target_line_index = line_index_by_name(&write, &target_line)
            .map_err(storage_error)?
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown line: {target_line}")))?;
        let landed_snapshot_index = snapshot_index_by_id(&write, &landed_snapshot_id)
            .map_err(storage_error)?
            .ok_or_else(|| {
                PlanStoreError::NotFound(format!("Unknown Snapshot: {landed_snapshot_id}"))
            })?;
        let pre_land_plus1 = match nonempty(pre_land_target_snapshot_id) {
            None => 0,
            Some(id) => snapshot_index_by_id(&write, id)
                .map_err(storage_error)?
                .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown Snapshot: {id}")))?
                .checked_add(1)
                .ok_or_else(|| PlanStoreError::Storage("Snapshot index overflow".to_string()))?,
        };
        let mut change_owner = ChangeLandIndexRecord::decode(
            &write
                .read_record(Self::change_land_index_file(), change_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        if change_owner.next_land_ordinal > MAX_LAND_ORDINAL {
            return Err(PlanStoreError::Invalid(format!(
                "Change {change_id} already owns the maximum 64 Land attempts"
            )));
        }
        let mut task_owner = TaskLandIndexRecord::decode(
            &write
                .read_record(Self::task_land_index_file(), change.task_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        let now = now_s()?;
        let land = LocalLandRecord {
            land_meta: 2
                | LAND_META_HAS_LANDED
                | if pre_land_plus1 != 0 {
                    LAND_META_HAS_PRE_LAND
                } else {
                    0
                },
            land_ordinal: change_owner.next_land_ordinal,
            change_ordinal: change.change_ordinal,
            failure_kind: 0,
            change_index,
            previous_task_land_index_plus1: task_owner.latest_land_index_plus1,
            previous_change_land_index_plus1: change_owner.latest_land_index_plus1,
            pre_land_target_snapshot_index_plus1: pre_land_plus1,
            landed_snapshot_index_plus1: landed_snapshot_index
                .checked_add(1)
                .ok_or_else(|| PlanStoreError::Storage("Snapshot index overflow".to_string()))?,
            submitted_at_s: now,
            updated_at_s: now,
            target_line_index_plus1: target_line_index
                .checked_add(1)
                .ok_or_else(|| PlanStoreError::Storage("Line index overflow".to_string()))?,
        };
        let land_index = write
            .append_record(
                Self::land_record_file(),
                &land.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        let land_plus1 = land_index
            .checked_add(1)
            .ok_or_else(|| PlanStoreError::Storage("Land index overflow".to_string()))?;
        change_owner.latest_land_index_plus1 = land_plus1;
        change_owner.land_count = change_owner
            .land_count
            .checked_add(1)
            .ok_or_else(|| PlanStoreError::Storage("Land count overflow".to_string()))?;
        change_owner.next_land_ordinal = change_owner
            .next_land_ordinal
            .checked_add(1)
            .ok_or_else(|| PlanStoreError::Storage("Land ordinal overflow".to_string()))?;
        task_owner.latest_land_index_plus1 = land_plus1;
        task_owner.land_count = task_owner
            .land_count
            .checked_add(1)
            .ok_or_else(|| PlanStoreError::Storage("Task Land count overflow".to_string()))?;
        write
            .overwrite_record(
                Self::change_land_index_file(),
                change_index,
                &change_owner.encode(),
            )
            .map_err(storage_error)?;
        write
            .overwrite_record(
                Self::task_land_index_file(),
                change.task_index,
                &task_owner.encode(),
            )
            .map_err(storage_error)?;
        change.change_meta = (change.change_meta & !CHANGE_META_LIFECYCLE_MASK) | 2;
        change.change_state = 0;
        change.updated_at_s = now;
        write
            .overwrite_record(
                Self::change_record_file(),
                change_index,
                &change.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        write.commit().map_err(storage_error)?;
        ChangeStore::get_change(self, change_id)
    }

    fn mark_change_published(
        &self,
        change_id: &str,
        remote_name: Option<&str>,
        published_change_id: Option<&str>,
        allow_landed: bool,
    ) -> PlanStoreResult<JsonValue> {
        require_origin_remote(remote_name)?;
        let published = nonempty(published_change_id).ok_or_else(|| {
            PlanStoreError::Invalid("published_change_id is required".to_string())
        })?;
        let (remote_task_index, remote_change_ordinal) =
            self.parse_published_change_id(published)?;
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let rows = self.read_rows_with_access(&write).map_err(storage_error)?;
        let change_index = self.change_index_for_id(&rows, change_id)?;
        let mut change = LocalChangeRecord::decode(
            &write
                .read_record(Self::change_record_file(), change_index)
                .map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        if change.change_meta & CHANGE_META_LIFECYCLE_MASK == 2 && !allow_landed {
            return Err(PlanStoreError::Invalid(format!(
                "Landed Change {change_id} requires completed-local publication authority"
            )));
        }
        let task = self
            .read_task_record(&write, change.task_index)
            .map_err(storage_error)?;
        if !task.is_published() || task.published_remote_task_index != remote_task_index {
            return Err(PlanStoreError::Invalid(format!(
                "Published Change {published} does not belong to the Local Task's exact Remote Task"
            )));
        }
        let ordinal_plus1 = remote_change_ordinal
            .checked_add(1)
            .ok_or_else(|| PlanStoreError::Storage("Remote Change ordinal overflow".to_string()))?;
        if change.is_published() && change.published_remote_change_ordinal_plus1 != ordinal_plus1 {
            return Err(PlanStoreError::Concurrency(format!(
                "Change {change_id} already has a different Remote mapping"
            )));
        }
        let now = now_s()?;
        change.local_meta |= 1;
        change.published_remote_change_ordinal_plus1 = ordinal_plus1;
        change.published_at_s = change.published_at_s.max(now);
        change.updated_at_s = now;
        write
            .overwrite_record(
                Self::change_record_file(),
                change_index,
                &change.encode().map_err(storage_error)?,
            )
            .map_err(storage_error)?;
        write.commit().map_err(storage_error)?;
        ChangeStore::get_change(self, change_id)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowTaskLister
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn list_tasks(&self) -> PlanStoreResult<Vec<JsonValue>> {
        TaskStore::list_tasks(self)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowTaskReader
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn get_task(&self, task_id: &str) -> PlanStoreResult<JsonValue> {
        TaskStore::get_task(self, task_id)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowTaskCreator
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn create_task(
        &self,
        repo_name: &str,
        title: &str,
        intent: &str,
        namespace_prefix: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        TaskStore::create_task(
            self,
            repo_name,
            title,
            intent,
            namespace_prefix,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        )
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowTaskCloser
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn close_task(&self, task_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
        TaskStore::close_task(self, task_id, status)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowTaskPublisher
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn mark_task_published(
        &self,
        task_id: &str,
        remote_name: Option<&str>,
        published_task_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        TaskStore::mark_task_published(self, task_id, remote_name, published_task_id)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowChangeLister
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn list_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
        ChangeStore::list_changes(self)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowChangeReader
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn get_change(&self, change_id: &str) -> PlanStoreResult<JsonValue> {
        ChangeStore::get_change(self, change_id)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowChangeCreator
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn create_change(
        &self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        namespace_prefix: Option<&str>,
        fork_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        ChangeStore::create_change(
            self,
            task_id,
            repo_name,
            title,
            base_line,
            namespace_prefix,
            fork_snapshot_id,
        )
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowChangeCloser
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn close_change(&self, change_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
        ChangeStore::close_change(self, change_id, status)
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowChangeLander
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn land_change(
        &self,
        change_id: &str,
        target_line: &str,
        landed_snapshot_id: &str,
        pre_land_target_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        ChangeStore::land_change(
            self,
            change_id,
            target_line,
            landed_snapshot_id,
            pre_land_target_snapshot_id,
        )
    }
}

impl<B: BinaryDb, const WRITE_LAYOUT: u32> TaskWorkflowChangePublisher
    for BinaryDbWorkflowStore<B, WRITE_LAYOUT>
{
    fn mark_change_published(
        &self,
        change_id: &str,
        remote_name: Option<&str>,
        published_change_id: Option<&str>,
        allow_landed: bool,
    ) -> PlanStoreResult<JsonValue> {
        ChangeStore::mark_change_published(
            self,
            change_id,
            remote_name,
            published_change_id,
            allow_landed,
        )
    }
}

trait WorkflowReadAccess {
    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32>;
    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<Vec<u8>>;
    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>>;
}

struct UnlockedWorkflowReadAccess<'a, B>(&'a B);

impl<B: BinaryDb> WorkflowReadAccess for UnlockedWorkflowReadAccess<'_, B> {
    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.0.record_count(file)
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<Vec<u8>> {
        self.0.read_record(file, record_index)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.0.read_payload(file, offset, len)
    }
}

impl<B: BinaryDb> WorkflowReadAccess for BinaryDbReadTxn<'_, B> {
    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        BinaryDbReadTxn::record_count(self, file)
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<Vec<u8>> {
        BinaryDbReadTxn::read_record(self, file, record_index)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        BinaryDbReadTxn::read_payload(self, file, offset, len)
    }
}

impl<B, F> WorkflowReadAccess for BinaryDbWriteTxn<'_, B, F>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        BinaryDbWriteTxn::record_count(self, file)
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<Vec<u8>> {
        BinaryDbWriteTxn::read_record(self, file, record_index)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        BinaryDbWriteTxn::read_payload(self, file, offset, len)
    }
}

fn require_aligned_count<A: WorkflowReadAccess>(
    access: &A,
    file: BinaryFileId,
    expected: u32,
    name: &str,
) -> StoreResult<()> {
    let actual = access.record_count(file)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{name} has {actual} records, expected {expected}").into())
    }
}

fn fixed_file<const LAYOUT: u32>(path: &'static str, record_size: u32) -> BinaryFileId {
    BinaryFileId::new(path, LAYOUT, record_size)
}

fn payload_file<const LAYOUT: u32>(path: &'static str) -> BinaryPayloadFileId {
    BinaryPayloadFileId::new(path, LAYOUT)
}

fn validate_local_task_record(record: &LocalTaskRecord) -> StoreResult<()> {
    if record.local_meta & !LOCAL_TASK_META_KNOWN != 0 {
        return Err("LocalTaskRecord has reserved metadata bits".into());
    }
    if record.payload_len < 2 || record.payload_offset < 4 {
        return Err("LocalTaskRecord has an invalid payload locator".into());
    }
    let published = record.local_meta & 1 != 0;
    if published != (record.published_at_s != 0) {
        return Err("LocalTaskRecord publication fields disagree".into());
    }
    let has_revision = record.origin_plan_revision_index_plus1 != 0;
    if !has_revision && (record.plan_item_index_plus1 != 0 || record.plan_linked_at_s != 0) {
        return Err("LocalTaskRecord Plan binding fields disagree".into());
    }
    if record.is_terminal() {
        // Zero is the bounded legacy unknown-close-time representation.
    } else if record.closed_at_s != 0 {
        return Err("non-terminal LocalTaskRecord has closed_at_s".into());
    }
    if record.created_at_s == 0 || record.updated_at_s < record.created_at_s {
        return Err("LocalTaskRecord has invalid event times".into());
    }
    Ok(())
}

fn validate_local_change_record(record: &LocalChangeRecord) -> StoreResult<()> {
    if record.local_meta & !LOCAL_CHANGE_META_KNOWN != 0
        || record.change_state & !CHANGE_STATE_KNOWN != 0
        || record.reserved1 != 0
        || record.reserved2 != 0
        || record.reserved3 != 0
        || record.change_ordinal > MAX_CHANGE_ORDINAL
    {
        return Err("LocalChangeRecord has reserved or invalid fields".into());
    }
    if record.payload_len == 0 || record.payload_offset < 4 {
        return Err("LocalChangeRecord has an invalid title locator".into());
    }
    if record.base_line_index_plus1 == 0 {
        return Err("LocalChangeRecord has no base Line".into());
    }
    let archived = record.change_meta & CHANGE_META_LIFECYCLE_MASK == 3;
    if !archived && record.archived_at_s != 0 {
        return Err("non-archived LocalChangeRecord has archived_at_s".into());
    }
    if record.change_state != 0 && !archived {
        return Err("LocalChangeRecord canceled state requires archived lifecycle".into());
    }
    if record.change_meta & 0b1000_0000 != 0 && !archived {
        return Err("LocalChangeRecord superseded bit requires archived lifecycle".into());
    }
    let published = record.local_meta & 1 != 0;
    if published
        != (record.published_remote_change_ordinal_plus1 != 0 && record.published_at_s != 0)
        || record.published_remote_change_ordinal_plus1 > 64
    {
        return Err("LocalChangeRecord publication fields disagree".into());
    }
    if record.created_at_s == 0 || record.updated_at_s < record.created_at_s {
        return Err("LocalChangeRecord has invalid event times".into());
    }
    Ok(())
}

fn validate_local_land_record(record: &LocalLandRecord) -> StoreResult<()> {
    let status = record.land_meta & LAND_META_STATUS_MASK;
    let mode = (record.land_meta & LAND_META_MODE_MASK) >> 5;
    if status == 7
        || mode == 3
        || record.land_ordinal > MAX_LAND_ORDINAL
        || record.change_ordinal > MAX_CHANGE_ORDINAL
        || record.failure_kind > 7
        || record.target_line_index_plus1 == 0
    {
        return Err("LocalLandRecord has reserved or invalid fields".into());
    }
    let has_pre = record.land_meta & LAND_META_HAS_PRE_LAND != 0;
    let has_landed = record.land_meta & LAND_META_HAS_LANDED != 0;
    if has_pre != (record.pre_land_target_snapshot_index_plus1 != 0)
        || has_landed != (record.landed_snapshot_index_plus1 != 0)
        || (status == 2) != has_landed
    {
        return Err("LocalLandRecord Snapshot presence bits disagree".into());
    }
    if matches!(status, 3 | 4) != (record.failure_kind != 0) {
        return Err("LocalLandRecord failure kind disagrees with status".into());
    }
    if record.submitted_at_s == 0 {
        let admitted_legacy = status == 2
            && record.land_ordinal == 0
            && record.previous_task_land_index_plus1 == 0
            && record.previous_change_land_index_plus1 == 0;
        if !admitted_legacy {
            return Err("LocalLandRecord has zero submitted_at_s outside legacy rule".into());
        }
    } else if record.updated_at_s < record.submitted_at_s {
        return Err("LocalLandRecord has invalid event times".into());
    }
    Ok(())
}

fn encode_task_payload(title: &str, intent: &str) -> StoreResult<Vec<u8>> {
    let title = title.trim();
    let intent = intent.trim();
    if title.is_empty() || intent.is_empty() {
        return Err("Task title and intent must not be empty".into());
    }
    let title_len = u16::try_from(title.len()).map_err(|_| "Task title exceeds u16::MAX bytes")?;
    let mut out = Vec::with_capacity(2 + title.len() + intent.len());
    out.extend_from_slice(&title_len.to_le_bytes());
    out.extend_from_slice(title.as_bytes());
    out.extend_from_slice(intent.as_bytes());
    if out.len() > usize::from(u16::MAX) {
        return Err("Task payload exceeds u16::MAX bytes".into());
    }
    Ok(out)
}

fn decode_task_payload(raw: &[u8]) -> StoreResult<(String, String)> {
    if raw.len() < 3 {
        return Err("Task payload is truncated".into());
    }
    let title_len = usize::from(u16::from_le_bytes(raw[0..2].try_into().unwrap()));
    if title_len == 0 || 2 + title_len >= raw.len() {
        return Err("Task payload title/intent boundary is invalid".into());
    }
    let title = std::str::from_utf8(&raw[2..2 + title_len])
        .map_err(|_| "Task title is not UTF-8")?
        .to_string();
    let intent = std::str::from_utf8(&raw[2 + title_len..])
        .map_err(|_| "Task intent is not UTF-8")?
        .to_string();
    if title.trim().is_empty() || intent.trim().is_empty() {
        return Err("Task title and intent must not be empty".into());
    }
    Ok((title, intent))
}

fn line_index_by_name<A: WorkflowReadAccess>(
    access: &A,
    expected: &str,
) -> StoreResult<Option<u32>> {
    let count =
        access.record_count(BinaryLineCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file())?;
    let mut found = None;
    for index in 0..count {
        let record =
            BinaryLineCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_record(&access.read_record(
                BinaryLineCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file(),
                index,
            )?)?;
        let name = access.read_payload(
            BinaryLineCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::payload_file(),
            record.line_name_offset,
            u32::from(record.line_name_len),
        )?;
        if name == expected.as_bytes() && !record.is_tombstone() && found.replace(index).is_some() {
            return Err(format!("Line name {expected:?} is ambiguous").into());
        }
    }
    Ok(found)
}

fn line_name_at<A: WorkflowReadAccess>(access: &A, index: u32) -> StoreResult<String> {
    let record =
        BinaryLineCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_record(&access.read_record(
            BinaryLineCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file(),
            index,
        )?)?;
    let raw = access.read_payload(
        BinaryLineCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::payload_file(),
        record.line_name_offset,
        u32::from(record.line_name_len),
    )?;
    Ok(std::str::from_utf8(&raw)
        .map_err(|_| "Line name is not UTF-8")?
        .to_string())
}

fn snapshot_index_by_id<A: WorkflowReadAccess>(
    access: &A,
    snapshot_id: &str,
) -> StoreResult<Option<u32>> {
    let expected = snapshot_hash48_from_id(snapshot_id)?;
    let count =
        access.record_count(BinarySnapshotCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file())?;
    let mut found = None;
    for index in 0..count {
        let record = BinarySnapshotCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_record(
            &access.read_record(
                BinarySnapshotCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file(),
                index,
            )?,
        )?;
        if record.snapshot_hash48 == expected
            && !record.is_tombstone()
            && found.replace(index).is_some()
        {
            return Err(format!("Snapshot ID {snapshot_id} is ambiguous").into());
        }
    }
    Ok(found)
}

fn snapshot_id_at<A: WorkflowReadAccess>(access: &A, index: u32) -> StoreResult<String> {
    let record =
        BinarySnapshotCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::decode_record(&access.read_record(
            BinarySnapshotCodec::<BINARY_DB_WORKFLOW_LAYOUT_ID>::record_file(),
            index,
        )?)?;
    if record.is_tombstone() {
        return Err(format!("Snapshot {index} is tombstoned").into());
    }
    Ok(snapshot_id_from_hash48(record.snapshot_hash48))
}

fn change_status_bits(status: &str, now: u64) -> PlanStoreResult<(u8, u8, u64)> {
    match status {
        "draft" => Ok((0, 0, 0)),
        "active" => Ok((1, 0, 0)),
        "review" => Ok((1 | 0b0000_1000, 0, 0)),
        "archived" => Ok((3, 0, now)),
        "superseded" => Ok((3 | 0b1000_0000, 0, now)),
        "canceled" => Ok((3, 1, now)),
        "landed" => Err(PlanStoreError::Invalid(
            "Change creation cannot invent a successful Land record".to_string(),
        )),
        other => Err(PlanStoreError::Invalid(format!(
            "Unsupported Change status: {other}"
        ))),
    }
}

fn render_change_id(ordinal: u8) -> String {
    format!("C-{:02}", u16::from(ordinal) + 1)
}

fn parse_change_id(value: &str) -> PlanStoreResult<u8> {
    let raw = value
        .strip_prefix("C-")
        .ok_or_else(|| PlanStoreError::Invalid(format!("Invalid Change ID: {value}")))?;
    if raw.len() != 2 {
        return Err(PlanStoreError::Invalid(format!(
            "Change ID must use C-01 through C-64: {value}"
        )));
    }
    let sequence = raw
        .parse::<u8>()
        .map_err(|_| PlanStoreError::Invalid(format!("Invalid Change ID: {value}")))?;
    if !(1..=64).contains(&sequence) {
        return Err(PlanStoreError::Invalid(format!(
            "Change ID must use C-01 through C-64: {value}"
        )));
    }
    Ok(sequence - 1)
}

fn parse_revision_id(value: &str) -> PlanStoreResult<u32> {
    let raw = value.strip_prefix("plan-revision:").ok_or_else(|| {
        PlanStoreError::Invalid(format!("Invalid Plan revision identity: {value}"))
    })?;
    let index = raw
        .parse::<u32>()
        .map_err(|_| PlanStoreError::Invalid(format!("Invalid Plan revision identity: {value}")))?;
    if format!("plan-revision:{index}") != value {
        return Err(PlanStoreError::Invalid(format!(
            "Plan revision identity is not canonical: {value}"
        )));
    }
    Ok(index)
}

fn resolve_change_key(
    rows: &BTreeMap<String, JsonValue>,
    requested: &str,
) -> PlanStoreResult<String> {
    if requested.contains('/') {
        return rows
            .contains_key(requested)
            .then(|| requested.to_string())
            .ok_or_else(|| unknown_row("change", requested));
    }
    let mut matches = rows
        .iter()
        .filter(|(_, row)| string_value(row, "change_id").as_deref() == Some(requested))
        .map(|(key, _)| key.clone());
    let Some(first) = matches.next() else {
        return Err(unknown_row("change", requested));
    };
    if matches.next().is_some() {
        return Err(PlanStoreError::Invalid(format!(
            "Task context is required to resolve ambiguous Change ID {requested}"
        )));
    }
    Ok(first)
}

fn timestamp_required(value: u64, field: &str) -> StoreResult<String> {
    if value == 0 {
        return Err(format!("{field} must not be zero").into());
    }
    let value = i64::try_from(value)
        .map_err(|_| format!("{field} is outside the RFC 3339 timestamp range"))?;
    Utc.timestamp_opt(value, 0)
        .single()
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
        .ok_or_else(|| format!("{field} is outside the timestamp range").into())
}

fn timestamp_json(value: u64) -> StoreResult<JsonValue> {
    if value == 0 {
        Ok(JsonValue::Null)
    } else {
        Ok(JsonValue::String(timestamp_required(value, "timestamp")?))
    }
}

fn parse_time_s(value: &str, field: &str) -> PlanStoreResult<u64> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| PlanStoreError::Invalid(format!("{field} is invalid: {error}")))?;
    u64::try_from(parsed.timestamp())
        .map_err(|_| PlanStoreError::Invalid(format!("{field} is before the Unix epoch")))
}

fn now_s() -> PlanStoreResult<u64> {
    u64::try_from(Utc::now().timestamp())
        .map_err(|_| PlanStoreError::Storage("current time is before the Unix epoch".to_string()))
}

fn option_string(value: Option<String>) -> JsonValue {
    value.map(JsonValue::String).unwrap_or(JsonValue::Null)
}

fn string_value(value: &JsonValue, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn required_row_text(row: &JsonValue, field: &str, kind: &str) -> StoreResult<String> {
    string_value(row, field)
        .ok_or_else(|| format!("Binary DB {kind} row is missing required text {field}").into())
}

fn required_text(value: &str, field: &str) -> PlanStoreResult<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(PlanStoreError::Invalid(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value.to_string())
    }
}

fn require_matching_repo(expected: &str, actual: &str) -> PlanStoreResult<()> {
    if expected == actual.trim() {
        Ok(())
    } else {
        Err(PlanStoreError::Invalid(format!(
            "Repository mismatch: expected {expected}, got {actual}"
        )))
    }
}

fn require_exact_optional(value: Option<&str>, expected: &str, field: &str) -> PlanStoreResult<()> {
    if nonempty(value).unwrap_or(expected) == expected {
        Ok(())
    } else {
        Err(PlanStoreError::Invalid(format!(
            "{field} is not representable in Binary DB v0"
        )))
    }
}

fn require_origin_remote(value: Option<&str>) -> PlanStoreResult<()> {
    match nonempty(value) {
        None | Some("origin") => Ok(()),
        Some(other) => Err(PlanStoreError::Invalid(format!(
            "published_remote_name {other:?} is not representable; v0 accepts only contextual origin"
        ))),
    }
}

fn require_raw_size(raw: &[u8], expected: u32, kind: &str) -> StoreResult<()> {
    if raw.len() == expected as usize {
        Ok(())
    } else {
        Err(format!("{kind} requires {expected} bytes, got {}", raw.len()).into())
    }
}

fn require_encoded_size(raw: &[u8], expected: u32, kind: &str) -> StoreResult<()> {
    require_raw_size(raw, expected, kind)
}

fn require_supported_layout<const WRITE_LAYOUT: u32>() -> PlanStoreResult<()> {
    if WRITE_LAYOUT == BINARY_DB_WORKFLOW_LAYOUT_ID {
        Ok(())
    } else {
        Err(PlanStoreError::Storage(format!(
            "unsupported Binary DB workflow layout: {WRITE_LAYOUT}"
        )))
    }
}

fn storage_error(error: impl ToString) -> PlanStoreError {
    PlanStoreError::Storage(error.to_string())
}

fn unknown_row(kind: &str, id: &str) -> PlanStoreError {
    PlanStoreError::NotFound(format!("Unknown {kind}: {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_db::{
        AuthorityId, LocalBinaryDbFs, LocalStateScope, StorePath, REPOSITORY_BINARY_DB_BIN_PATHS,
        REPOSITORY_BINARY_DB_INDEX_PATHS,
    };
    use crate::content_binary_db::{BinarySnapshotCodec, BinarySnapshotRecord};
    use crate::line_binary_db::BinaryDbLineStore;
    use tempfile::TempDir;

    fn fixture(
        temp: &TempDir,
    ) -> BinaryDbWorkflowStore<LocalBinaryDbFs, BINARY_DB_WORKFLOW_LAYOUT_ID> {
        let db = LocalBinaryDbFs::new(
            StorePath::from(temp.path().join("binary-db")),
            StorePath::from(temp.path()),
            AuthorityId::new("local:fixture"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
        BinaryDbWorkflowStore::new_with_namespace(db, "fixture", "C")
    }

    fn seed_line_and_snapshots(
        store: &BinaryDbWorkflowStore<LocalBinaryDbFs, 1>,
    ) -> (String, String) {
        let line = BinaryDbLineStore::<_, 1>::new(store.db().clone());
        line.append_line_for_bootstrap(
            "main",
            "active",
            Some("2026-07-24T00:00:00Z"),
            Some("2026-07-24T00:00:00Z"),
            None,
            None,
        )
        .unwrap();
        let first = "SNP-000000000001".to_string();
        let second = "SNP-000000000002".to_string();
        let mut write =
            BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
        for (index, id) in [first.clone(), second.clone()].into_iter().enumerate() {
            let record = BinarySnapshotRecord {
                snapshot_meta: 0b0010_0000,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: snapshot_hash48_from_id(&id).unwrap(),
                parent_snapshot_index_plus1: u32::try_from(index).unwrap(),
                root_tree_pack_index_plus1: 1,
                root_entry_ordinal: 0,
                line_index_plus1: 1,
                manifest_hash: [index as u8; 32],
                file_count: 0,
                total_bytes: 0,
                created_at_s: 1,
            };
            write
                .append_record(
                    BinarySnapshotCodec::<1>::record_file(),
                    &BinarySnapshotCodec::<1>::encode_record(&record).unwrap(),
                )
                .unwrap();
        }
        write.commit().unwrap();
        (first, second)
    }

    #[test]
    fn exact_v0_record_widths_and_narrow_payloads_round_trip() {
        let task = LocalTaskRecord {
            task_meta: 0,
            local_meta: 0,
            payload_len: 4,
            payload_offset: 4,
            origin_plan_revision_index_plus1: 0,
            plan_item_index_plus1: 0,
            published_remote_task_index: 0,
            created_at_s: 1,
            updated_at_s: 1,
            plan_linked_at_s: 0,
            published_at_s: 0,
            closed_at_s: 0,
        };
        assert_eq!(task.encode().unwrap().len(), 64);
        let change = LocalChangeRecord {
            change_meta: 0,
            local_meta: 0,
            payload_len: 1,
            change_ordinal: 0,
            change_state: 0,
            reserved1: 0,
            payload_offset: 4,
            task_index: 0,
            previous_change_index_plus1: 0,
            fork_snapshot_index_plus1: 0,
            published_remote_change_ordinal_plus1: 0,
            reserved2: 0,
            reserved3: 0,
            created_at_s: 1,
            updated_at_s: 1,
            published_at_s: 0,
            base_line_index_plus1: 1,
            archived_at_s: 0,
        };
        assert_eq!(change.encode().unwrap().len(), 68);
        let land = LocalLandRecord {
            land_meta: 2 | LAND_META_HAS_LANDED,
            land_ordinal: 0,
            change_ordinal: 0,
            failure_kind: 0,
            change_index: 0,
            previous_task_land_index_plus1: 0,
            previous_change_land_index_plus1: 0,
            pre_land_target_snapshot_index_plus1: 0,
            landed_snapshot_index_plus1: 1,
            submitted_at_s: 1,
            updated_at_s: 1,
            target_line_index_plus1: 1,
        };
        let encoded_land = land.encode().unwrap();
        assert_eq!(encoded_land.len(), 44);
        assert_eq!(&encoded_land[40..44], &1_u32.to_le_bytes());
        let mut missing_target = land;
        missing_target.target_line_index_plus1 = 0;
        assert!(missing_target.encode().is_err());
        assert_eq!(
            decode_task_payload(&encode_task_payload("Task", "Intent").unwrap()).unwrap(),
            ("Task".to_string(), "Intent".to_string())
        );
    }

    #[test]
    fn u64_second_storage_round_trips_beyond_u32_and_fails_closed_at_projection() {
        for seconds in [u64::from(u32::MAX) + 1, u64::MAX] {
            let task = LocalTaskRecord {
                task_meta: 0,
                local_meta: 0,
                payload_len: 4,
                payload_offset: 4,
                origin_plan_revision_index_plus1: 0,
                plan_item_index_plus1: 0,
                published_remote_task_index: 0,
                created_at_s: seconds,
                updated_at_s: seconds,
                plan_linked_at_s: 0,
                published_at_s: 0,
                closed_at_s: 0,
            };
            let task_bytes = task.encode().unwrap();
            assert_eq!(LocalTaskRecord::decode(&task_bytes).unwrap(), task);

            let change = LocalChangeRecord {
                change_meta: 0,
                local_meta: 0,
                payload_len: 1,
                change_ordinal: 0,
                change_state: 0,
                reserved1: 0,
                payload_offset: 4,
                task_index: 0,
                previous_change_index_plus1: 0,
                fork_snapshot_index_plus1: 0,
                published_remote_change_ordinal_plus1: 0,
                reserved2: 0,
                reserved3: 0,
                created_at_s: seconds,
                updated_at_s: seconds,
                published_at_s: 0,
                base_line_index_plus1: 1,
                archived_at_s: 0,
            };
            let change_bytes = change.encode().unwrap();
            assert_eq!(LocalChangeRecord::decode(&change_bytes).unwrap(), change);

            let land = LocalLandRecord {
                land_meta: 0,
                land_ordinal: 0,
                change_ordinal: 0,
                failure_kind: 0,
                change_index: 0,
                previous_task_land_index_plus1: 0,
                previous_change_land_index_plus1: 0,
                pre_land_target_snapshot_index_plus1: 0,
                landed_snapshot_index_plus1: 0,
                submitted_at_s: seconds,
                updated_at_s: seconds,
                target_line_index_plus1: 1,
            };
            let land_bytes = land.encode().unwrap();
            assert_eq!(LocalLandRecord::decode(&land_bytes).unwrap(), land);
        }

        assert!(timestamp_required(u64::MAX, "test timestamp").is_err());
    }

    #[test]
    fn local_task_change_land_runtime_inlines_required_land_target_line() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        let (base, landed) = seed_line_and_snapshots(&store);
        let task = TaskStore::create_task(
            &store,
            "fixture",
            "Task",
            "Intent",
            Some("C"),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(task["task_id"], json!("LCT-0001"));
        let change = ChangeStore::create_change(
            &store,
            "LCT-0001",
            "fixture",
            "Change",
            "main",
            Some("C"),
            Some(&base),
        )
        .unwrap();
        assert_eq!(change["change_ref"], json!("LCT-0001/C-01"));
        let landed_change =
            ChangeStore::land_change(&store, "LCT-0001/C-01", "main", &landed, Some(&base))
                .unwrap();
        assert_eq!(landed_change["status"], json!("landed"));
        TaskStore::close_task(&store, "LCT-0001", "completed").unwrap();
        assert_eq!(
            TaskStore::list_completed_tasks_with_landed_changes(&store)
                .unwrap()
                .len(),
            1
        );
        let root = temp.path().join("binary-db");
        for (name, size, count) in [
            (TASK_RECORD_BIN, 64_u64, 1_u64),
            (TASK_CHANGE_INDEX_BIN, 8, 1),
            (TASK_LAND_INDEX_BIN, 8, 1),
            (CHANGE_RECORD_BIN, 68, 1),
            (CHANGE_LAND_INDEX_BIN, 8, 1),
            (LAND_RECORD_BIN, 44, 1),
        ] {
            assert_eq!(
                std::fs::metadata(root.join(name)).unwrap().len(),
                4 + size * count
            );
        }
        assert!(!root.join("land_target_line.bin").exists());
        let task_payload = std::fs::read(root.join(TASK_PAYLOAD_BIN)).unwrap();
        assert!(!task_payload
            .windows(b"task_id".len())
            .any(|window| window == b"task_id"));
        assert!(!task_payload
            .windows(b"status".len())
            .any(|window| window == b"status"));
    }

    #[test]
    fn task_cancellation_uses_fixed_bit_and_close_time() {
        let temp = TempDir::new().unwrap();
        let store = fixture(&temp);
        TaskStore::create_task(
            &store,
            "fixture",
            "Task",
            "Intent",
            Some("C"),
            None,
            None,
            None,
        )
        .unwrap();
        let canceled = TaskStore::close_task(&store, "LCT-0001", "canceled").unwrap();
        assert_eq!(canceled["status"], json!("canceled"));
        assert!(canceled["closed_at"].is_string());
        let raw = store
            .db()
            .read_record(
                BinaryDbWorkflowStore::<LocalBinaryDbFs, 1>::task_record_file(),
                0,
            )
            .unwrap();
        let record = LocalTaskRecord::decode(&raw).unwrap();
        assert_ne!(record.task_meta & 0b1000_0000, 0);
        assert_ne!(record.closed_at_s, 0);
    }
}
