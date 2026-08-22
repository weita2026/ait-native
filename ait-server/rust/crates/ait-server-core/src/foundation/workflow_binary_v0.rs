use crate::foundation::remote_binary_db::{
    BinaryDbError, BinaryDbFileFamily, BinaryFileId, BinaryIndexId, BinaryPayloadFileId,
    StoreResult,
};

pub const WORKFLOW_V0_LAYOUT_ID: u32 = 1;

pub const TASK_RECORD_SIZE: u32 = 60;
pub const CHANGE_RECORD_SIZE: u32 = 68;
pub const PATCHSET_RECORD_SIZE: u32 = 65;
pub const ATTEST_RECORD_SIZE: u32 = 24;
pub const ACTOR_RECORD_SIZE: u32 = 36;
pub const REVIEW_RECORD_SIZE: u32 = 40;
pub const POLICY_RECORD_SIZE: u32 = 32;
pub const POLICY_CHECK_RECORD_SIZE: u32 = 8;
pub const LAND_RECORD_SIZE: u32 = 48;
pub const SNAPSHOT_LINK_RECORD_SIZE: u32 = 40;
pub const WAIVER_RECORD_SIZE: u32 = 44;
pub const CHAIN_INDEX_RECORD_SIZE: u32 = 8;

pub const TASK_META_PLANNED: u8 = 1 << 0;
pub const TASK_META_SNAPSHOTTED: u8 = 1 << 1;
pub const TASK_META_REVIEW_PENDING: u8 = 1 << 2;
pub const TASK_META_VALIDATION_PENDING: u8 = 1 << 3;
pub const TASK_META_BLOCKED: u8 = 1 << 4;
pub const TASK_META_READY_TO_LAND: u8 = 1 << 5;
pub const TASK_META_COMPLETED: u8 = 1 << 6;
pub const TASK_META_CANCELED: u8 = 1 << 7;
pub const REMOTE_META_KNOWN_MASK: u8 = 0b0000_1111;

pub const CHANGE_META_LIFECYCLE_MASK: u8 = 0b0000_0011;
pub const CHANGE_META_HAS_PATCHSETS: u8 = 1 << 2;
pub const CHANGE_META_REVIEW_PENDING: u8 = 1 << 3;
pub const CHANGE_META_VALIDATION_PENDING: u8 = 1 << 4;
pub const CHANGE_META_READY_TO_LAND: u8 = 1 << 5;
pub const CHANGE_META_BLOCKED: u8 = 1 << 6;
pub const CHANGE_META_SUPERSEDED: u8 = 1 << 7;
pub const CHANGE_STATE_CANCELED: u8 = 1 << 0;

pub const CHANGE_LIFECYCLE_DRAFT: u8 = 0;
pub const CHANGE_LIFECYCLE_ACTIVE: u8 = 1;
pub const CHANGE_LIFECYCLE_LANDED: u8 = 2;
pub const CHANGE_LIFECYCLE_ARCHIVED: u8 = 3;

pub const PATCHSET_AUTHOR_MODE_MASK: u8 = 0b0001_1100;
pub const PATCHSET_PUBLISH_STATE_MASK: u8 = 0b0110_0000;
pub const PATCHSET_EVALUATION_PENDING: u8 = 1 << 7;

pub const CI_STATUS_NONE: u8 = 0;
pub const CI_STATUS_PASS: u8 = 1;
pub const CI_STATUS_FAIL: u8 = 2;
pub const CI_STATUS_ERROR: u8 = 3;
pub const CI_STATUS_OVERALL_SHIFT: u8 = 0;
pub const CI_STATUS_TESTS_SHIFT: u8 = 2;
pub const CI_STATUS_LINT_SHIFT: u8 = 4;
pub const CI_STATUS_RESERVED_MASK: u8 = 0b1100_0000;

pub const ATTEST_VERIFICATION_MASK: u8 = 0b0000_0011;
pub const ATTEST_REVOKED: u8 = 1 << 2;
pub const ATTEST_REQUIRE_TESTS_PASS: u8 = 1 << 3;
pub const ATTEST_REQUIRE_HUMAN_REVIEW: u8 = 1 << 4;
pub const ATTEST_REQUIRE_LINT_PASS: u8 = 1 << 5;
pub const ATTEST_CI_BACKED: u8 = 1 << 6;
pub const ATTEST_TOMBSTONE: u8 = 1 << 7;

pub const REVIEW_ACTION_MASK: u8 = 0b0000_0111;
pub const REVIEW_BLOCKING: u8 = 1 << 3;
pub const REVIEW_TASK_LANE: u8 = 1 << 4;
pub const REVIEW_CODE_REVIEW_SUMMARY: u8 = 1 << 5;
pub const REVIEW_DEFER: u8 = 1 << 6;
pub const REVIEW_TOMBSTONE: u8 = 1 << 7;

pub const POLICY_DECISION_MASK: u8 = 0b0000_0111;
pub const POLICY_TOMBSTONE: u8 = 1 << 7;

pub const LAND_STATUS_MASK: u8 = 0b0000_0111;
pub const LAND_HAS_PRE_TARGET: u8 = 1 << 3;
pub const LAND_HAS_LANDED_SNAPSHOT: u8 = 1 << 4;
pub const LAND_MODE_MASK: u8 = 0b0110_0000;
pub const LAND_TOMBSTONE: u8 = 1 << 7;

pub const LAND_STATUS_QUEUED: u8 = 0;
pub const LAND_STATUS_RUNNING: u8 = 1;
pub const LAND_STATUS_SUCCEEDED: u8 = 2;
pub const LAND_STATUS_BLOCKED: u8 = 3;
pub const LAND_STATUS_FAILED: u8 = 4;
pub const LAND_STATUS_CANCELED: u8 = 5;
pub const LAND_STATUS_UPDATING: u8 = 6;

pub const LAND_MODE_DIRECT: u8 = 0;
pub const LAND_MODE_MERGE: u8 = 1;
pub const LAND_MODE_FF_ONLY: u8 = 2;

pub const SNAPSHOT_LINK_HAS_CHANGE: u8 = 1 << 0;
pub const SNAPSHOT_LINK_HAS_SESSION: u8 = 1 << 1;
pub const SNAPSHOT_LINK_HAS_CHECKPOINT: u8 = 1 << 2;
pub const SNAPSHOT_LINK_HAS_WORKTREE_NAME: u8 = 1 << 3;
pub const SNAPSHOT_LINK_HAS_LINE_NAME: u8 = 1 << 4;
pub const SNAPSHOT_LINK_HAS_AUTHOR_OR_MODEL: u8 = 1 << 5;
pub const SNAPSHOT_LINK_TOMBSTONE: u8 = 1 << 7;
pub const SNAPSHOT_LINK_KNOWN_MASK: u8 = 0b1011_1111;

pub const WAIVER_REVOKED: u8 = 1 << 0;
pub const WAIVER_TOMBSTONE: u8 = 1 << 7;

pub fn workflow_record_file(path: &'static str, record_size: u32) -> BinaryFileId {
    BinaryFileId::new(
        path,
        WORKFLOW_V0_LAYOUT_ID,
        record_size,
        BinaryDbFileFamily::Workflow,
    )
}

pub fn workflow_payload_file(path: &'static str) -> BinaryPayloadFileId {
    BinaryPayloadFileId::new(path, WORKFLOW_V0_LAYOUT_ID, BinaryDbFileFamily::Workflow)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0TaskRecord {
    pub task_meta: u8,
    pub remote_meta: u8,
    pub payload_len: u16,
    pub payload_offset: u64,
    pub origin_plan_revision_index_plus1: u32,
    pub plan_item_index_plus1: u32,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    pub plan_linked_at_s: u64,
    pub fetched_at_s: u64,
    pub closed_at_s: u64,
}

impl V0TaskRecord {
    pub fn is_terminal(self) -> bool {
        self.task_meta & (TASK_META_COMPLETED | TASK_META_CANCELED) != 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0ChangeRecord {
    pub change_meta: u8,
    pub remote_meta: u8,
    pub payload_len: u16,
    pub change_ordinal: u8,
    pub change_state: u8,
    pub reserved1: u16,
    pub payload_offset: u64,
    pub task_index: u32,
    pub previous_change_index_plus1: u32,
    pub selected_patchset_index_plus1: u32,
    pub fork_snapshot_index_plus1: u32,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    pub fetched_at_s: u64,
    pub base_line_index_plus1: u32,
    pub archived_at_s: u64,
}

impl V0ChangeRecord {
    pub fn lifecycle(self) -> u8 {
        self.change_meta & CHANGE_META_LIFECYCLE_MASK
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// Logical Patchset record using the active Binary DB v0 field order.
pub struct V0PatchsetRecord {
    pub patchset_meta: u8,
    pub patch_ordinal: u8,
    pub change_ordinal: u8,
    pub reserved0: u8,
    pub change_index: u32,
    pub previous_task_patchset_index_plus1: u32,
    pub previous_change_patchset_index_plus1: u32,
    pub base_snapshot_index: u32,
    pub revision_snapshot_index: u32,
    pub created_at_s: u64,
    pub ci_completed_at_s: u64,
    pub ci_run_seq: u32,
    pub ci_selected_suite_count: u16,
    pub ci_suite_result_count: u16,
    pub ci_blocking_failure_count: u16,
    pub ci_status_bits: u8,
    pub summary_offset: u64,
    pub summary_len: u16,
    pub ci_worker_job_index_plus1: u32,
}

impl V0PatchsetRecord {
    pub fn ci_status(self, shift: u8) -> u8 {
        (self.ci_status_bits >> shift) & 0b11
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// Exact Patchset record frozen by the current Binary DB v0 authority.
///
/// This named form is retained for call sites that require explicit frozen
/// authority semantics; its bytes are identical to [`V0PatchsetRecord`].
pub struct V0FrozenPatchsetRecord {
    pub patchset_meta: u8,
    pub patch_ordinal: u8,
    pub change_ordinal: u8,
    pub reserved0: u8,
    pub change_index: u32,
    pub previous_task_patchset_index_plus1: u32,
    pub previous_change_patchset_index_plus1: u32,
    pub base_snapshot_index: u32,
    pub revision_snapshot_index: u32,
    pub created_at_s: u64,
    pub ci_completed_at_s: u64,
    pub ci_run_seq: u32,
    pub ci_selected_suite_count: u16,
    pub ci_suite_result_count: u16,
    pub ci_blocking_failure_count: u16,
    pub ci_status_bits: u8,
    pub summary_offset: u64,
    pub summary_len: u16,
    pub ci_worker_job_index_plus1: u32,
}

impl V0FrozenPatchsetRecord {
    pub fn ci_status(self, shift: u8) -> u8 {
        (self.ci_status_bits >> shift) & 0b11
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0AttestRecord {
    pub attest_meta: u8,
    pub attest_ordinal: u8,
    pub patch_ordinal: u8,
    pub change_ordinal: u8,
    pub patchset_index: u32,
    pub previous_task_attest_index_plus1: u32,
    pub previous_patchset_attest_index_plus1: u32,
    pub created_at_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0ActorRecord {
    pub actor_meta: u8,
    pub reserved0: u8,
    pub payload_len: u16,
    pub payload_offset: u64,
    pub actor_key_hash: u64,
    pub created_at_s: u64,
    pub last_seen_at_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0ReviewRecord {
    pub review_meta: u8,
    pub review_ordinal: u8,
    pub patch_ordinal: u8,
    pub change_ordinal: u8,
    pub actor_index_plus1: u32,
    pub patchset_index: u32,
    pub previous_task_review_index_plus1: u32,
    pub previous_patchset_review_index_plus1: u32,
    pub payload_offset: u64,
    pub payload_len: u16,
    pub reserved0: u16,
    pub created_at_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0PolicyRecord {
    pub policy_meta: u8,
    pub policy_ordinal: u8,
    pub patch_ordinal: u8,
    pub change_ordinal: u8,
    pub patchset_index: u32,
    pub previous_task_policy_index_plus1: u32,
    pub previous_patchset_policy_index_plus1: u32,
    pub first_check_index_plus1: u32,
    pub check_count: u16,
    pub reserved0: u16,
    pub created_at_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0PolicyCheckRecord {
    pub check_kind: u8,
    pub check_status: u8,
    pub subject_ordinal: u16,
    pub detail_flags: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0LandRecord {
    pub land_meta: u8,
    pub land_ordinal: u8,
    pub change_ordinal: u8,
    pub failure_kind: u8,
    pub change_index: u32,
    pub patchset_index: u32,
    pub previous_task_land_index_plus1: u32,
    pub previous_change_land_index_plus1: u32,
    pub pre_land_target_snapshot_index_plus1: u32,
    pub landed_snapshot_index_plus1: u32,
    pub submitted_at_s: u64,
    pub updated_at_s: u64,
    pub target_line_index_plus1: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0SnapshotLinkRecord {
    pub link_meta: u8,
    pub snapshot_ordinal: u8,
    pub payload_len: u16,
    pub payload_offset: u64,
    pub task_index: u32,
    pub change_index_plus1: u32,
    pub content_snapshot_index: u32,
    pub previous_task_snapshot_link_index_plus1: u32,
    pub previous_change_snapshot_link_index_plus1: u32,
    pub created_at_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0WaiverRecord {
    pub waiver_meta: u8,
    pub waiver_ordinal: u8,
    pub patch_ordinal: u8,
    pub change_ordinal: u8,
    pub patchset_index: u32,
    pub previous_task_waiver_index_plus1: u32,
    pub previous_patchset_waiver_index_plus1: u32,
    pub payload_offset: u64,
    pub payload_len: u16,
    pub rule_code: u16,
    pub created_at_s: u64,
    pub expires_at_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0InventoryIndexRecord {
    pub latest_index_plus1: u32,
    pub count: u16,
    pub reserved0: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct V0OrdinalIndexRecord {
    pub latest_index_plus1: u32,
    pub count: u16,
    pub next_ordinal: u8,
    pub reserved0: u8,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct V0TaskPayload {
    pub title: String,
    pub intent: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct V0ActorPayload {
    pub user_name: String,
    pub user_id: String,
    pub email: String,
    pub memo: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct V0SnapshotLinkPayload {
    pub worktree_name: String,
    pub line_name: String,
    pub task_id: String,
    pub change_id: String,
    pub author_mode: String,
    pub model_name: String,
}

pub struct WorkflowBinaryV0Codec;

impl WorkflowBinaryV0Codec {
    pub fn task_file() -> BinaryFileId {
        workflow_record_file("task.bin", TASK_RECORD_SIZE)
    }

    pub fn task_payload_file() -> BinaryPayloadFileId {
        workflow_payload_file("task_payload.bin")
    }

    pub fn change_file() -> BinaryFileId {
        workflow_record_file("change.bin", CHANGE_RECORD_SIZE)
    }

    pub fn change_payload_file() -> BinaryPayloadFileId {
        workflow_payload_file("change_payload.bin")
    }

    pub fn patchset_file() -> BinaryFileId {
        workflow_record_file("patchset.bin", PATCHSET_RECORD_SIZE)
    }

    pub fn patchset_summary_file() -> BinaryPayloadFileId {
        workflow_payload_file("patchset_summary_payload.bin")
    }

    pub fn attest_file() -> BinaryFileId {
        workflow_record_file("attest.bin", ATTEST_RECORD_SIZE)
    }

    pub fn actor_file() -> BinaryFileId {
        workflow_record_file("actor.bin", ACTOR_RECORD_SIZE)
    }

    pub fn actor_payload_file() -> BinaryPayloadFileId {
        workflow_payload_file("actor_payload.bin")
    }

    pub fn review_file() -> BinaryFileId {
        workflow_record_file("review.bin", REVIEW_RECORD_SIZE)
    }

    pub fn review_payload_file() -> BinaryPayloadFileId {
        workflow_payload_file("review_payload.bin")
    }

    pub fn policy_file() -> BinaryFileId {
        workflow_record_file("policy.bin", POLICY_RECORD_SIZE)
    }

    pub fn policy_check_file() -> BinaryFileId {
        workflow_record_file("policy_check.bin", POLICY_CHECK_RECORD_SIZE)
    }

    pub fn land_file() -> BinaryFileId {
        workflow_record_file("land.bin", LAND_RECORD_SIZE)
    }

    pub fn snapshot_link_file() -> BinaryFileId {
        workflow_record_file("snapshot_link.bin", SNAPSHOT_LINK_RECORD_SIZE)
    }

    pub fn snapshot_link_payload_file() -> BinaryPayloadFileId {
        workflow_payload_file("snapshot_link_payload.bin")
    }

    pub fn waiver_file() -> BinaryFileId {
        workflow_record_file("waiver.bin", WAIVER_RECORD_SIZE)
    }

    pub fn waiver_payload_file() -> BinaryPayloadFileId {
        workflow_payload_file("waiver_payload.bin")
    }

    pub fn chain_index_file(path: &'static str) -> BinaryFileId {
        workflow_record_file(path, CHAIN_INDEX_RECORD_SIZE)
    }

    pub fn actor_lookup_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(
            "actor_lookup.idx",
            WORKFLOW_V0_LAYOUT_ID,
            8,
            true,
            BinaryDbFileFamily::Workflow,
        )
    }

    pub fn encode_task(record: V0TaskRecord) -> StoreResult<Vec<u8>> {
        validate_task(record)?;
        let mut out = Vec::with_capacity(TASK_RECORD_SIZE as usize);
        out.push(record.task_meta);
        out.push(record.remote_meta);
        push_u16(&mut out, record.payload_len);
        push_u64(&mut out, record.payload_offset);
        push_u32(&mut out, record.origin_plan_revision_index_plus1);
        push_u32(&mut out, record.plan_item_index_plus1);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.updated_at_s);
        push_u64(&mut out, record.plan_linked_at_s);
        push_u64(&mut out, record.fetched_at_s);
        push_u64(&mut out, record.closed_at_s);
        finish_encode(out, TASK_RECORD_SIZE, "RemoteTaskRecord")
    }

    pub fn decode_task(raw: &[u8]) -> StoreResult<V0TaskRecord> {
        let mut input = Cursor::new(raw, TASK_RECORD_SIZE, "RemoteTaskRecord")?;
        let record = V0TaskRecord {
            task_meta: input.u8()?,
            remote_meta: input.u8()?,
            payload_len: input.u16()?,
            payload_offset: input.u64()?,
            origin_plan_revision_index_plus1: input.u32()?,
            plan_item_index_plus1: input.u32()?,
            created_at_s: input.u64()?,
            updated_at_s: input.u64()?,
            plan_linked_at_s: input.u64()?,
            fetched_at_s: input.u64()?,
            closed_at_s: input.u64()?,
        };
        input.finish()?;
        validate_task(record)?;
        Ok(record)
    }

    pub fn encode_change(record: V0ChangeRecord) -> StoreResult<Vec<u8>> {
        validate_change(record)?;
        let mut out = Vec::with_capacity(CHANGE_RECORD_SIZE as usize);
        out.push(record.change_meta);
        out.push(record.remote_meta);
        push_u16(&mut out, record.payload_len);
        out.push(record.change_ordinal);
        out.push(record.change_state);
        push_u16(&mut out, record.reserved1);
        push_u64(&mut out, record.payload_offset);
        push_u32(&mut out, record.task_index);
        push_u32(&mut out, record.previous_change_index_plus1);
        push_u32(&mut out, record.selected_patchset_index_plus1);
        push_u32(&mut out, record.fork_snapshot_index_plus1);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.updated_at_s);
        push_u64(&mut out, record.fetched_at_s);
        push_u32(&mut out, record.base_line_index_plus1);
        push_u64(&mut out, record.archived_at_s);
        finish_encode(out, CHANGE_RECORD_SIZE, "RemoteChangeRecord")
    }

    pub fn decode_change(raw: &[u8]) -> StoreResult<V0ChangeRecord> {
        let mut input = Cursor::new(raw, CHANGE_RECORD_SIZE, "RemoteChangeRecord")?;
        let record = V0ChangeRecord {
            change_meta: input.u8()?,
            remote_meta: input.u8()?,
            payload_len: input.u16()?,
            change_ordinal: input.u8()?,
            change_state: input.u8()?,
            reserved1: input.u16()?,
            payload_offset: input.u64()?,
            task_index: input.u32()?,
            previous_change_index_plus1: input.u32()?,
            selected_patchset_index_plus1: input.u32()?,
            fork_snapshot_index_plus1: input.u32()?,
            created_at_s: input.u64()?,
            updated_at_s: input.u64()?,
            fetched_at_s: input.u64()?,
            base_line_index_plus1: input.u32()?,
            archived_at_s: input.u64()?,
        };
        input.finish()?;
        validate_change(record)?;
        Ok(record)
    }

    pub fn encode_patchset(record: V0PatchsetRecord) -> StoreResult<Vec<u8>> {
        validate_patchset(record)?;
        let mut out = Vec::with_capacity(PATCHSET_RECORD_SIZE as usize);
        out.push(record.patchset_meta);
        out.push(record.patch_ordinal);
        out.push(record.change_ordinal);
        out.push(record.reserved0);
        push_u32(&mut out, record.change_index);
        push_u32(&mut out, record.previous_task_patchset_index_plus1);
        push_u32(&mut out, record.previous_change_patchset_index_plus1);
        push_u32(&mut out, record.base_snapshot_index);
        push_u32(&mut out, record.revision_snapshot_index);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.ci_completed_at_s);
        push_u32(&mut out, record.ci_run_seq);
        push_u16(&mut out, record.ci_selected_suite_count);
        push_u16(&mut out, record.ci_suite_result_count);
        push_u16(&mut out, record.ci_blocking_failure_count);
        out.push(record.ci_status_bits);
        push_u64(&mut out, record.summary_offset);
        push_u16(&mut out, record.summary_len);
        push_u32(&mut out, record.ci_worker_job_index_plus1);
        finish_encode(out, PATCHSET_RECORD_SIZE, "ServerPatchsetRecord")
    }

    pub fn decode_patchset(raw: &[u8]) -> StoreResult<V0PatchsetRecord> {
        let mut input = Cursor::new(raw, PATCHSET_RECORD_SIZE, "ServerPatchsetRecord")?;
        let record = V0PatchsetRecord {
            patchset_meta: input.u8()?,
            patch_ordinal: input.u8()?,
            change_ordinal: input.u8()?,
            reserved0: input.u8()?,
            change_index: input.u32()?,
            previous_task_patchset_index_plus1: input.u32()?,
            previous_change_patchset_index_plus1: input.u32()?,
            base_snapshot_index: input.u32()?,
            revision_snapshot_index: input.u32()?,
            created_at_s: input.u64()?,
            ci_completed_at_s: input.u64()?,
            ci_run_seq: input.u32()?,
            ci_selected_suite_count: input.u16()?,
            ci_suite_result_count: input.u16()?,
            ci_blocking_failure_count: input.u16()?,
            ci_status_bits: input.u8()?,
            summary_offset: input.u64()?,
            summary_len: input.u16()?,
            ci_worker_job_index_plus1: input.u32()?,
        };
        input.finish()?;
        validate_patchset(record)?;
        Ok(record)
    }

    pub fn encode_frozen_patchset(record: V0FrozenPatchsetRecord) -> StoreResult<Vec<u8>> {
        validate_frozen_patchset(record)?;
        let mut out = Vec::with_capacity(PATCHSET_RECORD_SIZE as usize);
        out.push(record.patchset_meta);
        out.push(record.patch_ordinal);
        out.push(record.change_ordinal);
        out.push(record.reserved0);
        push_u32(&mut out, record.change_index);
        push_u32(&mut out, record.previous_task_patchset_index_plus1);
        push_u32(&mut out, record.previous_change_patchset_index_plus1);
        push_u32(&mut out, record.base_snapshot_index);
        push_u32(&mut out, record.revision_snapshot_index);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.ci_completed_at_s);
        push_u32(&mut out, record.ci_run_seq);
        push_u16(&mut out, record.ci_selected_suite_count);
        push_u16(&mut out, record.ci_suite_result_count);
        push_u16(&mut out, record.ci_blocking_failure_count);
        out.push(record.ci_status_bits);
        push_u64(&mut out, record.summary_offset);
        push_u16(&mut out, record.summary_len);
        push_u32(&mut out, record.ci_worker_job_index_plus1);
        finish_encode(out, PATCHSET_RECORD_SIZE, "FrozenServerPatchsetRecord")
    }

    pub fn decode_frozen_patchset(raw: &[u8]) -> StoreResult<V0FrozenPatchsetRecord> {
        let mut input = Cursor::new(raw, PATCHSET_RECORD_SIZE, "FrozenServerPatchsetRecord")?;
        let record = V0FrozenPatchsetRecord {
            patchset_meta: input.u8()?,
            patch_ordinal: input.u8()?,
            change_ordinal: input.u8()?,
            reserved0: input.u8()?,
            change_index: input.u32()?,
            previous_task_patchset_index_plus1: input.u32()?,
            previous_change_patchset_index_plus1: input.u32()?,
            base_snapshot_index: input.u32()?,
            revision_snapshot_index: input.u32()?,
            created_at_s: input.u64()?,
            ci_completed_at_s: input.u64()?,
            ci_run_seq: input.u32()?,
            ci_selected_suite_count: input.u16()?,
            ci_suite_result_count: input.u16()?,
            ci_blocking_failure_count: input.u16()?,
            ci_status_bits: input.u8()?,
            summary_offset: input.u64()?,
            summary_len: input.u16()?,
            ci_worker_job_index_plus1: input.u32()?,
        };
        input.finish()?;
        validate_frozen_patchset(record)?;
        Ok(record)
    }

    pub fn encode_attest(record: V0AttestRecord) -> StoreResult<Vec<u8>> {
        validate_ordinal(record.attest_ordinal, "Attestation")?;
        validate_ordinal(record.patch_ordinal, "Attestation Patchset")?;
        validate_ordinal(record.change_ordinal, "Attestation Change")?;
        let mut out = Vec::with_capacity(ATTEST_RECORD_SIZE as usize);
        out.push(record.attest_meta);
        out.push(record.attest_ordinal);
        out.push(record.patch_ordinal);
        out.push(record.change_ordinal);
        push_u32(&mut out, record.patchset_index);
        push_u32(&mut out, record.previous_task_attest_index_plus1);
        push_u32(&mut out, record.previous_patchset_attest_index_plus1);
        push_u64(&mut out, record.created_at_s);
        finish_encode(out, ATTEST_RECORD_SIZE, "ServerAttestationRecord")
    }

    pub fn decode_attest(raw: &[u8]) -> StoreResult<V0AttestRecord> {
        let mut input = Cursor::new(raw, ATTEST_RECORD_SIZE, "ServerAttestationRecord")?;
        let record = V0AttestRecord {
            attest_meta: input.u8()?,
            attest_ordinal: input.u8()?,
            patch_ordinal: input.u8()?,
            change_ordinal: input.u8()?,
            patchset_index: input.u32()?,
            previous_task_attest_index_plus1: input.u32()?,
            previous_patchset_attest_index_plus1: input.u32()?,
            created_at_s: input.u64()?,
        };
        input.finish()?;
        validate_ordinal(record.attest_ordinal, "Attestation")?;
        validate_ordinal(record.patch_ordinal, "Attestation Patchset")?;
        validate_ordinal(record.change_ordinal, "Attestation Change")?;
        Ok(record)
    }

    pub fn encode_actor(record: V0ActorRecord) -> StoreResult<Vec<u8>> {
        if record.reserved0 != 0
            || record.payload_len < 3
            || record.actor_meta & (1 << 6) != 0
            || record.actor_meta & 0b111 > 5
        {
            return Err(invalid("Actor reserved byte or payload length is invalid"));
        }
        let mut out = Vec::with_capacity(ACTOR_RECORD_SIZE as usize);
        out.push(record.actor_meta);
        out.push(record.reserved0);
        push_u16(&mut out, record.payload_len);
        push_u64(&mut out, record.payload_offset);
        push_u64(&mut out, record.actor_key_hash);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.last_seen_at_s);
        finish_encode(out, ACTOR_RECORD_SIZE, "ActorRecord")
    }

    pub fn decode_actor(raw: &[u8]) -> StoreResult<V0ActorRecord> {
        let mut input = Cursor::new(raw, ACTOR_RECORD_SIZE, "ActorRecord")?;
        let record = V0ActorRecord {
            actor_meta: input.u8()?,
            reserved0: input.u8()?,
            payload_len: input.u16()?,
            payload_offset: input.u64()?,
            actor_key_hash: input.u64()?,
            created_at_s: input.u64()?,
            last_seen_at_s: input.u64()?,
        };
        input.finish()?;
        if record.reserved0 != 0
            || record.payload_len < 3
            || record.actor_meta & (1 << 6) != 0
            || record.actor_meta & 0b111 > 5
        {
            return Err(corrupt("Actor reserved byte or payload length is invalid"));
        }
        Ok(record)
    }

    pub fn encode_review(record: V0ReviewRecord) -> StoreResult<Vec<u8>> {
        validate_review(record)?;
        let mut out = Vec::with_capacity(REVIEW_RECORD_SIZE as usize);
        out.push(record.review_meta);
        out.push(record.review_ordinal);
        out.push(record.patch_ordinal);
        out.push(record.change_ordinal);
        push_u32(&mut out, record.actor_index_plus1);
        push_u32(&mut out, record.patchset_index);
        push_u32(&mut out, record.previous_task_review_index_plus1);
        push_u32(&mut out, record.previous_patchset_review_index_plus1);
        push_u64(&mut out, record.payload_offset);
        push_u16(&mut out, record.payload_len);
        push_u16(&mut out, record.reserved0);
        push_u64(&mut out, record.created_at_s);
        finish_encode(out, REVIEW_RECORD_SIZE, "ServerReviewRecord")
    }

    pub fn decode_review(raw: &[u8]) -> StoreResult<V0ReviewRecord> {
        let mut input = Cursor::new(raw, REVIEW_RECORD_SIZE, "ServerReviewRecord")?;
        let record = V0ReviewRecord {
            review_meta: input.u8()?,
            review_ordinal: input.u8()?,
            patch_ordinal: input.u8()?,
            change_ordinal: input.u8()?,
            actor_index_plus1: input.u32()?,
            patchset_index: input.u32()?,
            previous_task_review_index_plus1: input.u32()?,
            previous_patchset_review_index_plus1: input.u32()?,
            payload_offset: input.u64()?,
            payload_len: input.u16()?,
            reserved0: input.u16()?,
            created_at_s: input.u64()?,
        };
        input.finish()?;
        validate_review(record)?;
        Ok(record)
    }

    pub fn encode_policy(record: V0PolicyRecord) -> StoreResult<Vec<u8>> {
        validate_policy(record)?;
        let mut out = Vec::with_capacity(POLICY_RECORD_SIZE as usize);
        out.push(record.policy_meta);
        out.push(record.policy_ordinal);
        out.push(record.patch_ordinal);
        out.push(record.change_ordinal);
        push_u32(&mut out, record.patchset_index);
        push_u32(&mut out, record.previous_task_policy_index_plus1);
        push_u32(&mut out, record.previous_patchset_policy_index_plus1);
        push_u32(&mut out, record.first_check_index_plus1);
        push_u16(&mut out, record.check_count);
        push_u16(&mut out, record.reserved0);
        push_u64(&mut out, record.created_at_s);
        finish_encode(out, POLICY_RECORD_SIZE, "PolicyDecisionRecord")
    }

    pub fn decode_policy(raw: &[u8]) -> StoreResult<V0PolicyRecord> {
        let mut input = Cursor::new(raw, POLICY_RECORD_SIZE, "PolicyDecisionRecord")?;
        let record = V0PolicyRecord {
            policy_meta: input.u8()?,
            policy_ordinal: input.u8()?,
            patch_ordinal: input.u8()?,
            change_ordinal: input.u8()?,
            patchset_index: input.u32()?,
            previous_task_policy_index_plus1: input.u32()?,
            previous_patchset_policy_index_plus1: input.u32()?,
            first_check_index_plus1: input.u32()?,
            check_count: input.u16()?,
            reserved0: input.u16()?,
            created_at_s: input.u64()?,
        };
        input.finish()?;
        validate_policy(record)?;
        Ok(record)
    }

    pub fn encode_policy_check(record: V0PolicyCheckRecord) -> StoreResult<Vec<u8>> {
        validate_policy_check(record)?;
        let mut out = Vec::with_capacity(POLICY_CHECK_RECORD_SIZE as usize);
        out.push(record.check_kind);
        out.push(record.check_status);
        push_u16(&mut out, record.subject_ordinal);
        push_u32(&mut out, record.detail_flags);
        finish_encode(out, POLICY_CHECK_RECORD_SIZE, "PolicyCheckRecord")
    }

    pub fn decode_policy_check(raw: &[u8]) -> StoreResult<V0PolicyCheckRecord> {
        let mut input = Cursor::new(raw, POLICY_CHECK_RECORD_SIZE, "PolicyCheckRecord")?;
        let record = V0PolicyCheckRecord {
            check_kind: input.u8()?,
            check_status: input.u8()?,
            subject_ordinal: input.u16()?,
            detail_flags: input.u32()?,
        };
        input.finish()?;
        validate_policy_check(record)?;
        Ok(record)
    }

    pub fn encode_land(record: V0LandRecord) -> StoreResult<Vec<u8>> {
        validate_land(record)?;
        let mut out = Vec::with_capacity(LAND_RECORD_SIZE as usize);
        out.push(record.land_meta);
        out.push(record.land_ordinal);
        out.push(record.change_ordinal);
        out.push(record.failure_kind);
        push_u32(&mut out, record.change_index);
        push_u32(&mut out, record.patchset_index);
        push_u32(&mut out, record.previous_task_land_index_plus1);
        push_u32(&mut out, record.previous_change_land_index_plus1);
        push_u32(&mut out, record.pre_land_target_snapshot_index_plus1);
        push_u32(&mut out, record.landed_snapshot_index_plus1);
        push_u64(&mut out, record.submitted_at_s);
        push_u64(&mut out, record.updated_at_s);
        push_u32(&mut out, record.target_line_index_plus1);
        finish_encode(out, LAND_RECORD_SIZE, "ServerLandRecord")
    }

    pub fn decode_land(raw: &[u8]) -> StoreResult<V0LandRecord> {
        let mut input = Cursor::new(raw, LAND_RECORD_SIZE, "ServerLandRecord")?;
        let record = V0LandRecord {
            land_meta: input.u8()?,
            land_ordinal: input.u8()?,
            change_ordinal: input.u8()?,
            failure_kind: input.u8()?,
            change_index: input.u32()?,
            patchset_index: input.u32()?,
            previous_task_land_index_plus1: input.u32()?,
            previous_change_land_index_plus1: input.u32()?,
            pre_land_target_snapshot_index_plus1: input.u32()?,
            landed_snapshot_index_plus1: input.u32()?,
            submitted_at_s: input.u64()?,
            updated_at_s: input.u64()?,
            target_line_index_plus1: input.u32()?,
        };
        input.finish()?;
        validate_land(record)?;
        Ok(record)
    }

    pub fn encode_snapshot_link(record: V0SnapshotLinkRecord) -> StoreResult<Vec<u8>> {
        validate_snapshot_link(record)?;
        let mut out = Vec::with_capacity(SNAPSHOT_LINK_RECORD_SIZE as usize);
        out.push(record.link_meta);
        out.push(record.snapshot_ordinal);
        push_u16(&mut out, record.payload_len);
        push_u64(&mut out, record.payload_offset);
        push_u32(&mut out, record.task_index);
        push_u32(&mut out, record.change_index_plus1);
        push_u32(&mut out, record.content_snapshot_index);
        push_u32(&mut out, record.previous_task_snapshot_link_index_plus1);
        push_u32(&mut out, record.previous_change_snapshot_link_index_plus1);
        push_u64(&mut out, record.created_at_s);
        finish_encode(
            out,
            SNAPSHOT_LINK_RECORD_SIZE,
            "ServerTaskSnapshotLinkRecord",
        )
    }

    pub fn decode_snapshot_link(raw: &[u8]) -> StoreResult<V0SnapshotLinkRecord> {
        let mut input = Cursor::new(
            raw,
            SNAPSHOT_LINK_RECORD_SIZE,
            "ServerTaskSnapshotLinkRecord",
        )?;
        let record = V0SnapshotLinkRecord {
            link_meta: input.u8()?,
            snapshot_ordinal: input.u8()?,
            payload_len: input.u16()?,
            payload_offset: input.u64()?,
            task_index: input.u32()?,
            change_index_plus1: input.u32()?,
            content_snapshot_index: input.u32()?,
            previous_task_snapshot_link_index_plus1: input.u32()?,
            previous_change_snapshot_link_index_plus1: input.u32()?,
            created_at_s: input.u64()?,
        };
        input.finish()?;
        validate_snapshot_link(record)?;
        Ok(record)
    }

    pub fn encode_waiver(record: V0WaiverRecord) -> StoreResult<Vec<u8>> {
        validate_waiver(record)?;
        let mut out = Vec::with_capacity(WAIVER_RECORD_SIZE as usize);
        out.push(record.waiver_meta);
        out.push(record.waiver_ordinal);
        out.push(record.patch_ordinal);
        out.push(record.change_ordinal);
        push_u32(&mut out, record.patchset_index);
        push_u32(&mut out, record.previous_task_waiver_index_plus1);
        push_u32(&mut out, record.previous_patchset_waiver_index_plus1);
        push_u64(&mut out, record.payload_offset);
        push_u16(&mut out, record.payload_len);
        push_u16(&mut out, record.rule_code);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.expires_at_s);
        finish_encode(out, WAIVER_RECORD_SIZE, "WaiverRecord")
    }

    pub fn decode_waiver(raw: &[u8]) -> StoreResult<V0WaiverRecord> {
        let mut input = Cursor::new(raw, WAIVER_RECORD_SIZE, "WaiverRecord")?;
        let record = V0WaiverRecord {
            waiver_meta: input.u8()?,
            waiver_ordinal: input.u8()?,
            patch_ordinal: input.u8()?,
            change_ordinal: input.u8()?,
            patchset_index: input.u32()?,
            previous_task_waiver_index_plus1: input.u32()?,
            previous_patchset_waiver_index_plus1: input.u32()?,
            payload_offset: input.u64()?,
            payload_len: input.u16()?,
            rule_code: input.u16()?,
            created_at_s: input.u64()?,
            expires_at_s: input.u64()?,
        };
        input.finish()?;
        validate_waiver(record)?;
        Ok(record)
    }

    pub fn encode_inventory_index(record: V0InventoryIndexRecord) -> StoreResult<Vec<u8>> {
        if record.reserved0 != 0 {
            return Err(invalid(
                "workflow inventory index reserved value is non-zero",
            ));
        }
        let mut out = Vec::with_capacity(CHAIN_INDEX_RECORD_SIZE as usize);
        push_u32(&mut out, record.latest_index_plus1);
        push_u16(&mut out, record.count);
        push_u16(&mut out, record.reserved0);
        finish_encode(out, CHAIN_INDEX_RECORD_SIZE, "WorkflowInventoryIndexRecord")
    }

    pub fn decode_inventory_index(raw: &[u8]) -> StoreResult<V0InventoryIndexRecord> {
        let mut input = Cursor::new(raw, CHAIN_INDEX_RECORD_SIZE, "WorkflowInventoryIndexRecord")?;
        let record = V0InventoryIndexRecord {
            latest_index_plus1: input.u32()?,
            count: input.u16()?,
            reserved0: input.u16()?,
        };
        input.finish()?;
        if record.reserved0 != 0 {
            return Err(corrupt(
                "workflow inventory index reserved value is non-zero",
            ));
        }
        Ok(record)
    }

    pub fn encode_ordinal_index(record: V0OrdinalIndexRecord) -> StoreResult<Vec<u8>> {
        if record.reserved0 != 0 || record.next_ordinal > 64 {
            return Err(invalid(
                "workflow ordinal index reserved/ordinal value is invalid",
            ));
        }
        let mut out = Vec::with_capacity(CHAIN_INDEX_RECORD_SIZE as usize);
        push_u32(&mut out, record.latest_index_plus1);
        push_u16(&mut out, record.count);
        out.push(record.next_ordinal);
        out.push(record.reserved0);
        finish_encode(out, CHAIN_INDEX_RECORD_SIZE, "WorkflowOrdinalIndexRecord")
    }

    pub fn decode_ordinal_index(raw: &[u8]) -> StoreResult<V0OrdinalIndexRecord> {
        let mut input = Cursor::new(raw, CHAIN_INDEX_RECORD_SIZE, "WorkflowOrdinalIndexRecord")?;
        let record = V0OrdinalIndexRecord {
            latest_index_plus1: input.u32()?,
            count: input.u16()?,
            next_ordinal: input.u8()?,
            reserved0: input.u8()?,
        };
        input.finish()?;
        if record.reserved0 != 0 || record.next_ordinal > 64 {
            return Err(corrupt(
                "workflow ordinal index reserved/ordinal value is invalid",
            ));
        }
        Ok(record)
    }

    pub fn encode_task_payload(payload: &V0TaskPayload) -> StoreResult<Vec<u8>> {
        if payload.title.is_empty() || payload.intent.is_empty() {
            return Err(invalid("Task title and intent are required"));
        }
        let title_len = u16::try_from(payload.title.len())
            .map_err(|_| invalid("Task title exceeds u16::MAX"))?;
        let total = 2_usize
            .checked_add(payload.title.len())
            .and_then(|value| value.checked_add(payload.intent.len()))
            .ok_or_else(|| invalid("Task payload length overflow"))?;
        if total > usize::from(u16::MAX) {
            return Err(invalid("Task payload exceeds u16::MAX"));
        }
        let mut out = Vec::with_capacity(total);
        push_u16(&mut out, title_len);
        out.extend_from_slice(payload.title.as_bytes());
        out.extend_from_slice(payload.intent.as_bytes());
        Ok(out)
    }

    pub fn decode_task_payload(raw: &[u8]) -> StoreResult<V0TaskPayload> {
        if raw.len() < 2 {
            return Err(corrupt("Task payload is truncated"));
        }
        let title_len = usize::from(u16::from_le_bytes(raw[0..2].try_into().unwrap()));
        let title_end = 2_usize
            .checked_add(title_len)
            .ok_or_else(|| corrupt("Task title length overflow"))?;
        if title_len == 0 || title_end >= raw.len() {
            return Err(corrupt("Task title or intent is empty/truncated"));
        }
        Ok(V0TaskPayload {
            title: utf8(&raw[2..title_end], "Task title")?.to_string(),
            intent: utf8(&raw[title_end..], "Task intent")?.to_string(),
        })
    }

    pub fn encode_single_text_payload(value: &str, label: &str) -> StoreResult<Vec<u8>> {
        if value.is_empty() {
            return Err(invalid(format!("{label} must not be empty")));
        }
        if value.len() > usize::from(u16::MAX) {
            return Err(invalid(format!("{label} exceeds u16::MAX")));
        }
        Ok(value.as_bytes().to_vec())
    }

    pub fn decode_single_text_payload<'a>(raw: &'a [u8], label: &str) -> StoreResult<&'a str> {
        if raw.is_empty() {
            return Err(corrupt(format!("{label} is empty")));
        }
        utf8(raw, label)
    }

    pub fn encode_review_payload(value: &str) -> StoreResult<Vec<u8>> {
        if value.len() > usize::from(u16::MAX) {
            return Err(invalid("Review message exceeds u16::MAX"));
        }
        Ok(value.as_bytes().to_vec())
    }

    pub fn decode_review_payload(raw: &[u8]) -> StoreResult<&str> {
        utf8(raw, "Review message")
    }

    pub fn encode_actor_payload(payload: &V0ActorPayload) -> StoreResult<Vec<u8>> {
        if payload.user_name.is_empty() {
            return Err(invalid("Actor user_name is required"));
        }
        let lengths = [
            payload.user_name.len(),
            payload.user_id.len(),
            payload.email.len(),
        ];
        if lengths.iter().any(|len| *len > usize::from(u8::MAX)) {
            return Err(invalid("Actor identity component exceeds u8::MAX"));
        }
        let total = 3_usize
            .checked_add(lengths.iter().sum::<usize>())
            .and_then(|value| value.checked_add(payload.memo.len()))
            .ok_or_else(|| invalid("Actor payload length overflow"))?;
        if total > usize::from(u16::MAX) {
            return Err(invalid("Actor payload exceeds u16::MAX"));
        }
        let mut out = Vec::with_capacity(total);
        out.extend(lengths.map(|len| len as u8));
        out.extend_from_slice(payload.user_name.as_bytes());
        out.extend_from_slice(payload.user_id.as_bytes());
        out.extend_from_slice(payload.email.as_bytes());
        out.extend_from_slice(payload.memo.as_bytes());
        Ok(out)
    }

    pub fn decode_actor_payload(raw: &[u8]) -> StoreResult<V0ActorPayload> {
        if raw.len() < 3 {
            return Err(corrupt("Actor payload is truncated"));
        }
        let name_len = usize::from(raw[0]);
        let user_id_len = usize::from(raw[1]);
        let email_len = usize::from(raw[2]);
        let name_end = 3_usize
            .checked_add(name_len)
            .ok_or_else(|| corrupt("Actor user_name length overflow"))?;
        let user_id_end = name_end
            .checked_add(user_id_len)
            .ok_or_else(|| corrupt("Actor user_id length overflow"))?;
        let email_end = user_id_end
            .checked_add(email_len)
            .ok_or_else(|| corrupt("Actor email length overflow"))?;
        if name_len == 0 || email_end > raw.len() {
            return Err(corrupt("Actor identity components are empty/truncated"));
        }
        Ok(V0ActorPayload {
            user_name: utf8(&raw[3..name_end], "Actor user_name")?.to_string(),
            user_id: utf8(&raw[name_end..user_id_end], "Actor user_id")?.to_string(),
            email: utf8(&raw[user_id_end..email_end], "Actor email")?.to_string(),
            memo: utf8(&raw[email_end..], "Actor memo")?.to_string(),
        })
    }

    pub fn encode_snapshot_link_payload(payload: &V0SnapshotLinkPayload) -> StoreResult<Vec<u8>> {
        let lengths = [
            payload.worktree_name.len(),
            payload.line_name.len(),
            payload.task_id.len(),
            payload.change_id.len(),
            payload.author_mode.len(),
        ];
        let encoded_lengths = lengths
            .map(|len| u16::try_from(len).map_err(|_| invalid("Snapshot Link field exceeds u16")))
            .into_iter()
            .collect::<StoreResult<Vec<_>>>()?;
        let total = 10_usize
            .checked_add(lengths.iter().sum::<usize>())
            .and_then(|value| value.checked_add(payload.model_name.len()))
            .ok_or_else(|| invalid("Snapshot Link payload length overflow"))?;
        if total > usize::from(u16::MAX) {
            return Err(invalid("Snapshot Link payload exceeds u16::MAX"));
        }
        let mut out = Vec::with_capacity(total);
        for len in encoded_lengths {
            push_u16(&mut out, len);
        }
        out.extend_from_slice(payload.worktree_name.as_bytes());
        out.extend_from_slice(payload.line_name.as_bytes());
        out.extend_from_slice(payload.task_id.as_bytes());
        out.extend_from_slice(payload.change_id.as_bytes());
        out.extend_from_slice(payload.author_mode.as_bytes());
        out.extend_from_slice(payload.model_name.as_bytes());
        Ok(out)
    }

    pub fn decode_snapshot_link_payload(raw: &[u8]) -> StoreResult<V0SnapshotLinkPayload> {
        if raw.len() < 10 {
            return Err(corrupt("Snapshot Link payload is truncated"));
        }
        let mut lengths = [0_usize; 5];
        for (index, length) in lengths.iter_mut().enumerate() {
            let start = index * 2;
            *length = usize::from(u16::from_le_bytes(
                raw[start..start + 2].try_into().unwrap(),
            ));
        }
        let mut offset = 10_usize;
        let mut take = |len: usize, label: &str| -> StoreResult<String> {
            let end = offset
                .checked_add(len)
                .ok_or_else(|| corrupt(format!("{label} length overflow")))?;
            let bytes = raw
                .get(offset..end)
                .ok_or_else(|| corrupt(format!("{label} is truncated")))?;
            offset = end;
            Ok(utf8(bytes, label)?.to_string())
        };
        let worktree_name = take(lengths[0], "Snapshot Link worktree_name")?;
        let line_name = take(lengths[1], "Snapshot Link line_name")?;
        let task_id = take(lengths[2], "Snapshot Link task_id")?;
        let change_id = take(lengths[3], "Snapshot Link change_id")?;
        let author_mode = take(lengths[4], "Snapshot Link author_mode")?;
        let model_name = utf8(
            raw.get(offset..)
                .ok_or_else(|| corrupt("Snapshot Link model_name is truncated"))?,
            "Snapshot Link model_name",
        )?
        .to_string();
        Ok(V0SnapshotLinkPayload {
            worktree_name,
            line_name,
            task_id,
            change_id,
            author_mode,
            model_name,
        })
    }
}

fn validate_task(record: V0TaskRecord) -> StoreResult<()> {
    if record.remote_meta & !REMOTE_META_KNOWN_MASK != 0 {
        return Err(invalid("Remote Task has reserved metadata bits"));
    }
    if record.payload_len < 3 {
        return Err(invalid("Remote Task payload is invalid"));
    }
    if record.task_meta & TASK_META_COMPLETED != 0 && record.task_meta & TASK_META_CANCELED != 0 {
        return Err(invalid("Task cannot be both completed and canceled"));
    }
    if record.closed_at_s != 0 && !record.is_terminal() {
        return Err(invalid("non-terminal Task retains closed_at_s"));
    }
    if (record.origin_plan_revision_index_plus1 == 0) != (record.plan_item_index_plus1 == 0) {
        return Err(invalid("Task Plan revision/item binding is incomplete"));
    }
    if record.origin_plan_revision_index_plus1 == 0 && record.plan_linked_at_s != 0 {
        return Err(invalid("unbound Task retains plan_linked_at_s"));
    }
    Ok(())
}

fn validate_change(record: V0ChangeRecord) -> StoreResult<()> {
    if record.remote_meta & !REMOTE_META_KNOWN_MASK != 0
        || record.reserved1 != 0
        || record.change_state & !1 != 0
    {
        return Err(invalid("Remote Change has reserved metadata"));
    }
    validate_ordinal(record.change_ordinal, "Change")?;
    if record.payload_len == 0 {
        return Err(invalid("Change title payload is empty"));
    }
    if record.base_line_index_plus1 == 0 {
        return Err(invalid("Change has no base Line"));
    }
    let lifecycle = record.lifecycle();
    if lifecycle != CHANGE_LIFECYCLE_ARCHIVED && record.archived_at_s != 0 {
        return Err(invalid("non-archived Change retains archived_at_s"));
    }
    if record.change_meta & CHANGE_META_SUPERSEDED != 0 && lifecycle != CHANGE_LIFECYCLE_ARCHIVED {
        return Err(invalid("superseded Change is not archived"));
    }
    if record.change_state & CHANGE_STATE_CANCELED != 0 && lifecycle != CHANGE_LIFECYCLE_ARCHIVED {
        return Err(invalid("canceled Change is not archived"));
    }
    Ok(())
}

fn validate_patchset(record: V0PatchsetRecord) -> StoreResult<()> {
    if record.reserved0 != 0 || record.summary_len == 0 {
        return Err(invalid("Patchset reserved byte or summary is invalid"));
    }
    validate_ordinal(record.patch_ordinal, "Patchset")?;
    validate_ordinal(record.change_ordinal, "Patchset Change")?;
    if (record.patchset_meta >> 2) & 0b111 == 7
        || matches!(
            (record.patchset_meta & PATCHSET_PUBLISH_STATE_MASK) >> 5,
            1 | 3
        )
    {
        return Err(invalid("Patchset metadata encoding is reserved"));
    }
    if record.ci_status_bits & CI_STATUS_RESERVED_MASK != 0
        || record.ci_suite_result_count > record.ci_selected_suite_count
        || record.ci_blocking_failure_count > record.ci_suite_result_count
    {
        return Err(invalid("Patchset CI compact counts/status are invalid"));
    }
    let overall = record.ci_status(CI_STATUS_OVERALL_SHIFT);
    let tests = record.ci_status(CI_STATUS_TESTS_SHIFT);
    let lint = record.ci_status(CI_STATUS_LINT_SHIFT);
    if record.ci_completed_at_s == 0 {
        if overall != 0
            || tests != 0
            || lint != 0
            || record.ci_selected_suite_count != 0
            || record.ci_suite_result_count != 0
            || record.ci_blocking_failure_count != 0
        {
            return Err(invalid("incomplete Patchset CI retains completed evidence"));
        }
    } else if record.ci_run_seq == 0 || overall == CI_STATUS_NONE {
        return Err(invalid("completed Patchset CI lacks run/status authority"));
    }
    if overall == CI_STATUS_NONE && (tests != CI_STATUS_NONE || lint != CI_STATUS_NONE) {
        return Err(invalid(
            "Patchset CI component status lacks overall evidence",
        ));
    }
    Ok(())
}

fn validate_frozen_patchset(record: V0FrozenPatchsetRecord) -> StoreResult<()> {
    validate_patchset(V0PatchsetRecord {
        patchset_meta: record.patchset_meta,
        patch_ordinal: record.patch_ordinal,
        change_ordinal: record.change_ordinal,
        reserved0: record.reserved0,
        change_index: record.change_index,
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
    })
}

fn validate_review(record: V0ReviewRecord) -> StoreResult<()> {
    validate_ordinal(record.review_ordinal, "Review")?;
    validate_ordinal(record.patch_ordinal, "Review Patchset")?;
    validate_ordinal(record.change_ordinal, "Review Change")?;
    if record.reserved0 != 0 || record.actor_index_plus1 == 0 {
        return Err(invalid(
            "Review reserved/payload/Actor authority is invalid",
        ));
    }
    let action = record.review_meta & REVIEW_ACTION_MASK;
    if action > 4 {
        return Err(invalid("Review action is reserved"));
    }
    let modifiers =
        record.review_meta & (REVIEW_TASK_LANE | REVIEW_CODE_REVIEW_SUMMARY | REVIEW_DEFER);
    let modifiers_valid = match action {
        0 => modifiers == 0,
        1 => {
            matches!(
                modifiers,
                0 | REVIEW_TASK_LANE | REVIEW_CODE_REVIEW_SUMMARY | REVIEW_DEFER
            ) || modifiers == (REVIEW_TASK_LANE | REVIEW_DEFER)
        }
        2 | 3 => matches!(modifiers, 0 | REVIEW_TASK_LANE),
        4 => modifiers == 0,
        _ => false,
    };
    if !modifiers_valid {
        return Err(invalid("Review action/modifier combination is invalid"));
    }
    Ok(())
}

fn validate_policy(record: V0PolicyRecord) -> StoreResult<()> {
    validate_ordinal(record.policy_ordinal, "Policy")?;
    validate_ordinal(record.patch_ordinal, "Policy Patchset")?;
    validate_ordinal(record.change_ordinal, "Policy Change")?;
    if record.reserved0 != 0
        || record.policy_meta & !(POLICY_DECISION_MASK | POLICY_TOMBSTONE) != 0
        || record.policy_meta & POLICY_DECISION_MASK > 4
    {
        return Err(invalid("Policy metadata has reserved bits"));
    }
    if (record.check_count == 0) != (record.first_check_index_plus1 == 0) {
        return Err(invalid("Policy check range presence is inconsistent"));
    }
    Ok(())
}

fn validate_policy_check(record: V0PolicyCheckRecord) -> StoreResult<()> {
    if record.check_kind > 9 || record.check_status > 7 {
        return Err(invalid("Policy check kind/status is reserved"));
    }
    match record.check_kind {
        0..=7 if record.subject_ordinal != 0 || record.detail_flags != 0 => {
            Err(invalid("fixed Policy check retains suite/phase detail"))
        }
        8 if record.detail_flags != 0 => Err(invalid("rollout-phase check has detail flags")),
        9 if record.subject_ordinal == 0 || !matches!(record.detail_flags, 1 | 2) => Err(invalid(
            "suite Policy check has invalid ordinal/detail flags",
        )),
        _ => Ok(()),
    }
}

fn validate_land(record: V0LandRecord) -> StoreResult<()> {
    validate_ordinal(record.land_ordinal, "Land")?;
    validate_ordinal(record.change_ordinal, "Land Change")?;
    if record.target_line_index_plus1 == 0 {
        return Err(invalid("Land target Line is required"));
    }
    let status = record.land_meta & LAND_STATUS_MASK;
    let mode = (record.land_meta & LAND_MODE_MASK) >> 5;
    if status > LAND_STATUS_UPDATING || mode > LAND_MODE_FF_ONLY || record.failure_kind > 7 {
        return Err(invalid("Land status/mode/failure is reserved"));
    }
    let has_pre = record.land_meta & LAND_HAS_PRE_TARGET != 0;
    let has_landed = record.land_meta & LAND_HAS_LANDED_SNAPSHOT != 0;
    if has_pre != (record.pre_land_target_snapshot_index_plus1 != 0)
        || has_landed != (record.landed_snapshot_index_plus1 != 0)
    {
        return Err(invalid("Land Snapshot flags and references disagree"));
    }
    if status == LAND_STATUS_SUCCEEDED && !has_landed {
        return Err(invalid("succeeded Land lacks landed Snapshot"));
    }
    if status != LAND_STATUS_SUCCEEDED && has_landed {
        return Err(invalid("non-succeeded Land retains landed Snapshot"));
    }
    if status != LAND_STATUS_BLOCKED && record.failure_kind != 0 && status != LAND_STATUS_FAILED {
        return Err(invalid("Land failure kind disagrees with status"));
    }
    Ok(())
}

fn validate_snapshot_link(record: V0SnapshotLinkRecord) -> StoreResult<()> {
    if record.link_meta & !SNAPSHOT_LINK_KNOWN_MASK != 0 || record.payload_len < 10 {
        return Err(invalid(
            "Snapshot Link reserved metadata or payload length is invalid",
        ));
    }
    if (record.link_meta & SNAPSHOT_LINK_HAS_CHANGE != 0) != (record.change_index_plus1 != 0) {
        return Err(invalid("Snapshot Link Change flag and reference disagree"));
    }
    Ok(())
}

fn validate_waiver(record: V0WaiverRecord) -> StoreResult<()> {
    if record.waiver_meta & !(WAIVER_REVOKED | WAIVER_TOMBSTONE) != 0 || record.payload_len == 0 {
        return Err(invalid("Waiver metadata or reason payload is invalid"));
    }
    validate_ordinal(record.waiver_ordinal, "Waiver")?;
    validate_ordinal(record.patch_ordinal, "Waiver Patchset")?;
    validate_ordinal(record.change_ordinal, "Waiver Change")?;
    Ok(())
}

fn validate_ordinal(value: u8, label: &str) -> StoreResult<()> {
    if value < 64 {
        Ok(())
    } else {
        Err(invalid(format!("{label} ordinal exceeds 63")))
    }
}

fn invalid(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::invalid_domain_data(message.into())
}

fn corrupt(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::corruption(message.into())
}

fn utf8<'a>(bytes: &'a [u8], label: &str) -> StoreResult<&'a str> {
    std::str::from_utf8(bytes).map_err(|_| corrupt(format!("{label} is not UTF-8")))
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn finish_encode(out: Vec<u8>, size: u32, label: &str) -> StoreResult<Vec<u8>> {
    if out.len() == size as usize {
        Ok(out)
    } else {
        Err(corrupt(format!(
            "{label} encoded {} bytes instead of {size}",
            out.len()
        )))
    }
}

struct Cursor<'a> {
    raw: &'a [u8],
    offset: usize,
    label: &'static str,
}

impl<'a> Cursor<'a> {
    fn new(raw: &'a [u8], size: u32, label: &'static str) -> StoreResult<Self> {
        if raw.len() != size as usize {
            return Err(corrupt(format!(
                "{label} requires {size} bytes, got {}",
                raw.len()
            )));
        }
        Ok(Self {
            raw,
            offset: 0,
            label,
        })
    }

    fn take<const N: usize>(&mut self) -> StoreResult<[u8; N]> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| corrupt(format!("{} cursor overflow", self.label)))?;
        let bytes = self
            .raw
            .get(self.offset..end)
            .ok_or_else(|| corrupt(format!("{} is truncated", self.label)))?;
        self.offset = end;
        Ok(bytes.try_into().unwrap())
    }

    fn u8(&mut self) -> StoreResult<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> StoreResult<u16> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    fn u32(&mut self) -> StoreResult<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> StoreResult<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn finish(self) -> StoreResult<()> {
        if self.offset == self.raw.len() {
            Ok(())
        } else {
            Err(corrupt(format!("{} has trailing bytes", self.label)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_record_widths_round_trip_without_padding() {
        let task = V0TaskRecord {
            payload_len: 3,
            created_at_s: u64::MAX,
            updated_at_s: u64::MAX,
            fetched_at_s: u64::MAX,
            ..V0TaskRecord::default()
        };
        let task_raw = WorkflowBinaryV0Codec::encode_task(task).unwrap();
        assert_eq!(task_raw.len(), TASK_RECORD_SIZE as usize);
        assert_eq!(WorkflowBinaryV0Codec::decode_task(&task_raw).unwrap(), task);

        let change = V0ChangeRecord {
            payload_len: 1,
            created_at_s: u64::MAX,
            updated_at_s: u64::MAX,
            base_line_index_plus1: 1,
            ..V0ChangeRecord::default()
        };
        let change_raw = WorkflowBinaryV0Codec::encode_change(change).unwrap();
        assert_eq!(change_raw.len(), CHANGE_RECORD_SIZE as usize);
        assert_eq!(
            WorkflowBinaryV0Codec::decode_change(&change_raw).unwrap(),
            change
        );

        let patchset = V0PatchsetRecord {
            summary_len: 1,
            created_at_s: u64::MAX,
            ..V0PatchsetRecord::default()
        };
        let patchset_raw = WorkflowBinaryV0Codec::encode_patchset(patchset).unwrap();
        assert_eq!(patchset_raw.len(), PATCHSET_RECORD_SIZE as usize);
        assert_eq!(
            WorkflowBinaryV0Codec::decode_patchset(&patchset_raw).unwrap(),
            patchset
        );
    }

    #[test]
    fn every_server_v0_record_has_exact_u64_second_offsets() {
        let task = V0TaskRecord {
            task_meta: 1,
            remote_meta: 2,
            payload_len: 3,
            payload_offset: 4,
            origin_plan_revision_index_plus1: 5,
            plan_item_index_plus1: 6,
            created_at_s: 7,
            updated_at_s: 8,
            plan_linked_at_s: 9,
            fetched_at_s: 10,
            closed_at_s: 0,
        };
        let task_raw = WorkflowBinaryV0Codec::encode_task(task).unwrap();
        assert_eq!(task_raw.len(), TASK_RECORD_SIZE as usize);
        assert_eq!(
            &task_raw[..20],
            &[1, 2, 3, 0, 4, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0]
        );
        for (offset, value) in [(20, 7_u64), (28, 8), (36, 9), (44, 10), (52, 0)] {
            assert_eq!(&task_raw[offset..offset + 8], &value.to_le_bytes());
        }
        assert_eq!(WorkflowBinaryV0Codec::decode_task(&task_raw).unwrap(), task);

        let change = V0ChangeRecord {
            change_meta: CHANGE_LIFECYCLE_ACTIVE,
            remote_meta: 2,
            payload_len: 3,
            change_ordinal: 4,
            change_state: 0,
            reserved1: 0,
            payload_offset: 5,
            task_index: 6,
            previous_change_index_plus1: 7,
            selected_patchset_index_plus1: 8,
            fork_snapshot_index_plus1: 9,
            created_at_s: 10,
            updated_at_s: 11,
            fetched_at_s: 12,
            base_line_index_plus1: 13,
            archived_at_s: 0,
        };
        let change_raw = WorkflowBinaryV0Codec::encode_change(change).unwrap();
        assert_eq!(change_raw.len(), CHANGE_RECORD_SIZE as usize);
        assert_eq!(
            &change_raw[..32],
            &[
                1, 2, 3, 0, 4, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, 8, 0, 0, 0,
                9, 0, 0, 0
            ]
        );
        for (offset, value) in [(32, 10_u64), (40, 11), (48, 12), (60, 0)] {
            assert_eq!(&change_raw[offset..offset + 8], &value.to_le_bytes());
        }
        assert_eq!(&change_raw[56..60], &13_u32.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_change(&change_raw).unwrap(),
            change
        );

        let patchset = V0PatchsetRecord {
            patchset_meta: 0b1000_0100,
            patch_ordinal: 2,
            change_ordinal: 3,
            reserved0: 0,
            change_index: 4,
            previous_task_patchset_index_plus1: 5,
            previous_change_patchset_index_plus1: 6,
            base_snapshot_index: 7,
            revision_snapshot_index: 8,
            created_at_s: 9,
            ci_completed_at_s: 0,
            ci_run_seq: 0,
            ci_selected_suite_count: 0,
            ci_suite_result_count: 0,
            ci_blocking_failure_count: 0,
            ci_status_bits: 0,
            summary_offset: 10,
            summary_len: 11,
            ci_worker_job_index_plus1: 12,
        };
        let patchset_raw = WorkflowBinaryV0Codec::encode_patchset(patchset).unwrap();
        assert_eq!(patchset_raw.len(), PATCHSET_RECORD_SIZE as usize);
        assert_eq!(
            &patchset_raw[..24],
            &[132, 2, 3, 0, 4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, 8, 0, 0, 0,]
        );
        assert_eq!(&patchset_raw[24..32], &9_u64.to_le_bytes());
        assert_eq!(&patchset_raw[32..40], &0_u64.to_le_bytes());
        assert_eq!(&patchset_raw[51..59], &10_u64.to_le_bytes());
        assert_eq!(&patchset_raw[59..61], &11_u16.to_le_bytes());
        assert_eq!(&patchset_raw[61..65], &12_u32.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_patchset(&patchset_raw).unwrap(),
            patchset
        );

        let attest = V0AttestRecord {
            attest_meta: 10,
            attest_ordinal: 1,
            patch_ordinal: 2,
            change_ordinal: 3,
            patchset_index: 4,
            previous_task_attest_index_plus1: 5,
            previous_patchset_attest_index_plus1: 6,
            created_at_s: 7,
        };
        let attest_raw = WorkflowBinaryV0Codec::encode_attest(attest).unwrap();
        assert_eq!(attest_raw.len(), ATTEST_RECORD_SIZE as usize);
        assert_eq!(
            &attest_raw[..16],
            &[10, 1, 2, 3, 4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0]
        );
        assert_eq!(&attest_raw[16..24], &7_u64.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_attest(&attest_raw).unwrap(),
            attest
        );

        let actor = V0ActorRecord {
            actor_meta: 5,
            reserved0: 0,
            payload_len: 3,
            payload_offset: 4,
            actor_key_hash: 5,
            created_at_s: 6,
            last_seen_at_s: 7,
        };
        let actor_raw = WorkflowBinaryV0Codec::encode_actor(actor).unwrap();
        assert_eq!(actor_raw.len(), ACTOR_RECORD_SIZE as usize);
        assert_eq!(
            &actor_raw[..20],
            &[5, 0, 3, 0, 4, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(&actor_raw[20..28], &6_u64.to_le_bytes());
        assert_eq!(&actor_raw[28..36], &7_u64.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_actor(&actor_raw).unwrap(),
            actor
        );

        let review = V0ReviewRecord {
            review_meta: 2,
            review_ordinal: 1,
            patch_ordinal: 2,
            change_ordinal: 3,
            actor_index_plus1: 4,
            patchset_index: 5,
            previous_task_review_index_plus1: 6,
            previous_patchset_review_index_plus1: 7,
            payload_offset: 8,
            payload_len: 0,
            reserved0: 0,
            created_at_s: 9,
        };
        let review_raw = WorkflowBinaryV0Codec::encode_review(review).unwrap();
        assert_eq!(review_raw.len(), REVIEW_RECORD_SIZE as usize);
        assert_eq!(&review_raw[32..40], &9_u64.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_review(&review_raw).unwrap(),
            review
        );

        let policy = V0PolicyRecord {
            policy_meta: 1,
            policy_ordinal: 1,
            patch_ordinal: 2,
            change_ordinal: 3,
            patchset_index: 4,
            previous_task_policy_index_plus1: 5,
            previous_patchset_policy_index_plus1: 6,
            first_check_index_plus1: 7,
            check_count: 8,
            reserved0: 0,
            created_at_s: 9,
        };
        let policy_raw = WorkflowBinaryV0Codec::encode_policy(policy).unwrap();
        assert_eq!(policy_raw.len(), POLICY_RECORD_SIZE as usize);
        assert_eq!(&policy_raw[24..32], &9_u64.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_policy(&policy_raw).unwrap(),
            policy
        );

        let check = V0PolicyCheckRecord {
            check_kind: 9,
            check_status: 3,
            subject_ordinal: 4,
            detail_flags: 1,
        };
        let check_raw = WorkflowBinaryV0Codec::encode_policy_check(check).unwrap();
        assert_eq!(check_raw, [9, 3, 4, 0, 1, 0, 0, 0]);
        assert_eq!(
            WorkflowBinaryV0Codec::decode_policy_check(&check_raw).unwrap(),
            check
        );

        let land = V0LandRecord {
            land_meta: LAND_STATUS_SUCCEEDED
                | LAND_HAS_PRE_TARGET
                | LAND_HAS_LANDED_SNAPSHOT
                | (LAND_MODE_MERGE << 5),
            land_ordinal: 1,
            change_ordinal: 2,
            failure_kind: 0,
            change_index: 3,
            patchset_index: 4,
            previous_task_land_index_plus1: 5,
            previous_change_land_index_plus1: 6,
            pre_land_target_snapshot_index_plus1: 7,
            landed_snapshot_index_plus1: 8,
            submitted_at_s: 9,
            updated_at_s: 10,
            target_line_index_plus1: 11,
        };
        let land_raw = WorkflowBinaryV0Codec::encode_land(land).unwrap();
        assert_eq!(land_raw.len(), LAND_RECORD_SIZE as usize);
        assert_eq!(&land_raw[28..36], &9_u64.to_le_bytes());
        assert_eq!(&land_raw[36..44], &10_u64.to_le_bytes());
        assert_eq!(&land_raw[44..48], &11_u32.to_le_bytes());
        assert_eq!(WorkflowBinaryV0Codec::decode_land(&land_raw).unwrap(), land);

        let link = V0SnapshotLinkRecord {
            link_meta: SNAPSHOT_LINK_HAS_CHANGE,
            snapshot_ordinal: u8::MAX,
            payload_len: 10,
            payload_offset: 2,
            task_index: 3,
            change_index_plus1: 4,
            content_snapshot_index: 5,
            previous_task_snapshot_link_index_plus1: 6,
            previous_change_snapshot_link_index_plus1: 7,
            created_at_s: 8,
        };
        let link_raw = WorkflowBinaryV0Codec::encode_snapshot_link(link).unwrap();
        assert_eq!(link_raw.len(), SNAPSHOT_LINK_RECORD_SIZE as usize);
        assert_eq!(&link_raw[32..40], &8_u64.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_snapshot_link(&link_raw).unwrap(),
            link
        );

        let waiver = V0WaiverRecord {
            waiver_meta: WAIVER_REVOKED,
            waiver_ordinal: 1,
            patch_ordinal: 2,
            change_ordinal: 3,
            patchset_index: 4,
            previous_task_waiver_index_plus1: 5,
            previous_patchset_waiver_index_plus1: 6,
            payload_offset: 7,
            payload_len: 1,
            rule_code: 8,
            created_at_s: 9,
            expires_at_s: 10,
        };
        let waiver_raw = WorkflowBinaryV0Codec::encode_waiver(waiver).unwrap();
        assert_eq!(waiver_raw.len(), WAIVER_RECORD_SIZE as usize);
        assert_eq!(&waiver_raw[28..36], &9_u64.to_le_bytes());
        assert_eq!(&waiver_raw[36..44], &10_u64.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_waiver(&waiver_raw).unwrap(),
            waiver
        );
    }

    #[test]
    fn patchset_uses_u64_ci_time_and_final_worker_job_locator() {
        let record = V0FrozenPatchsetRecord {
            patchset_meta: 0,
            patch_ordinal: 0,
            change_ordinal: 0,
            reserved0: 0,
            change_index: 1,
            previous_task_patchset_index_plus1: 2,
            previous_change_patchset_index_plus1: 3,
            base_snapshot_index: 4,
            revision_snapshot_index: 5,
            created_at_s: 6,
            ci_completed_at_s: 0x0102_0304,
            ci_run_seq: 7,
            ci_selected_suite_count: 1,
            ci_suite_result_count: 1,
            ci_blocking_failure_count: 0,
            ci_status_bits: CI_STATUS_PASS
                | (CI_STATUS_PASS << CI_STATUS_TESTS_SHIFT)
                | (CI_STATUS_PASS << CI_STATUS_LINT_SHIFT),
            summary_offset: 8,
            summary_len: 9,
            ci_worker_job_index_plus1: 0x0a0b_0c0d,
        };

        let raw = WorkflowBinaryV0Codec::encode_frozen_patchset(record).unwrap();

        assert_eq!(raw.len(), PATCHSET_RECORD_SIZE as usize);
        assert_eq!(&raw[32..40], &0x0102_0304_u64.to_le_bytes());
        assert_eq!(&raw[51..59], &8_u64.to_le_bytes());
        assert_eq!(&raw[59..61], &9_u16.to_le_bytes());
        assert_eq!(&raw[61..65], &[13, 12, 11, 10]);
        assert_eq!(
            WorkflowBinaryV0Codec::decode_frozen_patchset(&raw).unwrap(),
            record
        );
    }

    #[test]
    fn logical_patchset_codec_matches_the_active_frozen_offsets() {
        let record = V0PatchsetRecord {
            summary_offset: 8,
            summary_len: 9,
            ..V0PatchsetRecord::default()
        };

        let raw = WorkflowBinaryV0Codec::encode_patchset(record).unwrap();

        assert_eq!(raw.len(), PATCHSET_RECORD_SIZE as usize);
        assert_eq!(&raw[24..32], &0_u64.to_le_bytes());
        assert_eq!(&raw[32..40], &0_u64.to_le_bytes());
        assert_eq!(&raw[51..59], &8_u64.to_le_bytes());
        assert_eq!(&raw[59..61], &9_u16.to_le_bytes());
        assert_eq!(&raw[61..65], &0_u32.to_le_bytes());
        assert_eq!(
            WorkflowBinaryV0Codec::decode_patchset(&raw).unwrap(),
            record
        );
    }

    #[test]
    fn typed_payloads_are_exact_and_reject_empty_authority() {
        let task = V0TaskPayload {
            title: "title".to_string(),
            intent: "intent".to_string(),
        };
        let raw = WorkflowBinaryV0Codec::encode_task_payload(&task).unwrap();
        assert_eq!(
            WorkflowBinaryV0Codec::decode_task_payload(&raw).unwrap(),
            task
        );

        let actor = V0ActorPayload {
            user_name: "Ada".to_string(),
            user_id: String::new(),
            email: String::new(),
            memo: String::new(),
        };
        let raw = WorkflowBinaryV0Codec::encode_actor_payload(&actor).unwrap();
        assert_eq!(
            WorkflowBinaryV0Codec::decode_actor_payload(&raw).unwrap(),
            actor
        );
        let link = V0SnapshotLinkPayload {
            worktree_name: "worktree".to_string(),
            line_name: "main".to_string(),
            task_id: "T-0001".to_string(),
            change_id: "C-01".to_string(),
            author_mode: "human_only".to_string(),
            model_name: String::new(),
        };
        let raw = WorkflowBinaryV0Codec::encode_snapshot_link_payload(&link).unwrap();
        assert_eq!(
            WorkflowBinaryV0Codec::decode_snapshot_link_payload(&raw).unwrap(),
            link
        );
        assert_eq!(
            WorkflowBinaryV0Codec::encode_review_payload("").unwrap(),
            Vec::<u8>::new()
        );
        assert_eq!(
            WorkflowBinaryV0Codec::decode_review_payload(&[]).unwrap(),
            ""
        );
        assert!(WorkflowBinaryV0Codec::encode_single_text_payload("", "Review").is_err());
    }

    #[test]
    fn every_server_v0_record_rejects_malformed_or_reserved_authority() {
        assert!(WorkflowBinaryV0Codec::encode_change(V0ChangeRecord {
            payload_len: 1,
            base_line_index_plus1: 0,
            ..V0ChangeRecord::default()
        })
        .is_err());
        assert!(WorkflowBinaryV0Codec::encode_change(V0ChangeRecord {
            payload_len: 1,
            base_line_index_plus1: 1,
            archived_at_s: 1,
            ..V0ChangeRecord::default()
        })
        .is_err());
        assert!(WorkflowBinaryV0Codec::encode_attest(V0AttestRecord {
            attest_ordinal: 64,
            ..V0AttestRecord::default()
        })
        .is_err());
        assert!(WorkflowBinaryV0Codec::encode_actor(V0ActorRecord {
            actor_meta: 1 << 6,
            payload_len: 3,
            ..V0ActorRecord::default()
        })
        .is_err());
        assert!(WorkflowBinaryV0Codec::encode_review(V0ReviewRecord {
            actor_index_plus1: 0,
            ..V0ReviewRecord::default()
        })
        .is_err());
        assert!(WorkflowBinaryV0Codec::encode_policy(V0PolicyRecord {
            policy_meta: 1 << 6,
            ..V0PolicyRecord::default()
        })
        .is_err());
        assert!(
            WorkflowBinaryV0Codec::encode_policy_check(V0PolicyCheckRecord {
                check_kind: 9,
                check_status: 3,
                subject_ordinal: 0,
                detail_flags: 1,
            })
            .is_err()
        );
        assert!(WorkflowBinaryV0Codec::encode_land(V0LandRecord {
            land_meta: 0b111,
            ..V0LandRecord::default()
        })
        .is_err());
        assert!(WorkflowBinaryV0Codec::encode_land(V0LandRecord::default()).is_err());
        assert!(
            WorkflowBinaryV0Codec::encode_snapshot_link(V0SnapshotLinkRecord {
                link_meta: 1 << 6,
                payload_len: 10,
                ..V0SnapshotLinkRecord::default()
            })
            .is_err()
        );
        assert!(
            WorkflowBinaryV0Codec::encode_snapshot_link(V0SnapshotLinkRecord {
                link_meta: SNAPSHOT_LINK_HAS_CHANGE,
                payload_len: 10,
                change_index_plus1: 0,
                ..V0SnapshotLinkRecord::default()
            })
            .is_err()
        );
        assert!(WorkflowBinaryV0Codec::encode_waiver(V0WaiverRecord {
            waiver_meta: 1 << 1,
            payload_len: 1,
            ..V0WaiverRecord::default()
        })
        .is_err());
        assert!(WorkflowBinaryV0Codec::decode_snapshot_link_payload(&[0; 9]).is_err());
        assert!(WorkflowBinaryV0Codec::decode_actor_payload(&[0, 0, 0]).is_err());
    }

    #[test]
    fn reserved_bits_and_relationship_presence_fail_closed() {
        let mut task = V0TaskRecord {
            payload_len: 3,
            ..V0TaskRecord::default()
        };
        task.remote_meta = 1 << 4;
        assert!(WorkflowBinaryV0Codec::encode_task(task).is_err());

        let mut patchset = V0PatchsetRecord {
            summary_len: 1,
            ..V0PatchsetRecord::default()
        };
        patchset.ci_status_bits = CI_STATUS_PASS;
        assert!(WorkflowBinaryV0Codec::encode_patchset(patchset).is_err());

        let policy = V0PolicyRecord {
            check_count: 1,
            ..V0PolicyRecord::default()
        };
        assert!(WorkflowBinaryV0Codec::encode_policy(policy).is_err());
    }

    #[test]
    fn task_inventory_and_owner_ordinal_indexes_keep_distinct_tail_semantics() {
        let inventory = V0InventoryIndexRecord {
            latest_index_plus1: 9,
            count: 3,
            reserved0: 0,
        };
        let inventory_raw = WorkflowBinaryV0Codec::encode_inventory_index(inventory).unwrap();
        assert_eq!(inventory_raw, [9, 0, 0, 0, 3, 0, 0, 0]);
        assert_eq!(
            WorkflowBinaryV0Codec::decode_inventory_index(&inventory_raw).unwrap(),
            inventory
        );

        let owner = V0OrdinalIndexRecord {
            latest_index_plus1: 9,
            count: 3,
            next_ordinal: 7,
            reserved0: 0,
        };
        let owner_raw = WorkflowBinaryV0Codec::encode_ordinal_index(owner).unwrap();
        assert_eq!(owner_raw, [9, 0, 0, 0, 3, 0, 7, 0]);
        assert_eq!(
            WorkflowBinaryV0Codec::decode_ordinal_index(&owner_raw).unwrap(),
            owner
        );

        assert!(WorkflowBinaryV0Codec::decode_inventory_index(&owner_raw).is_err());
        let mut invalid_owner = owner_raw;
        invalid_owner[6] = 65;
        assert!(WorkflowBinaryV0Codec::decode_ordinal_index(&invalid_owner).is_err());
    }

    #[test]
    fn review_action_modifiers_follow_the_exact_v0_mapping() {
        let base = V0ReviewRecord {
            payload_len: 1,
            actor_index_plus1: 1,
            ..V0ReviewRecord::default()
        };
        for review_meta in [
            0,
            1,
            1 | REVIEW_TASK_LANE,
            1 | REVIEW_CODE_REVIEW_SUMMARY,
            1 | REVIEW_DEFER,
            1 | REVIEW_TASK_LANE | REVIEW_DEFER,
            2,
            2 | REVIEW_TASK_LANE,
            3,
            3 | REVIEW_TASK_LANE,
            4,
        ] {
            assert!(WorkflowBinaryV0Codec::encode_review(V0ReviewRecord {
                review_meta,
                ..base
            })
            .is_ok());
        }
        for review_meta in [
            REVIEW_TASK_LANE,
            1 | REVIEW_TASK_LANE | REVIEW_CODE_REVIEW_SUMMARY,
            2 | REVIEW_DEFER,
            3 | REVIEW_CODE_REVIEW_SUMMARY,
            4 | REVIEW_TASK_LANE,
        ] {
            assert!(WorkflowBinaryV0Codec::encode_review(V0ReviewRecord {
                review_meta,
                ..base
            })
            .is_err());
        }
    }
}
