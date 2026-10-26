use super::codec::{
    decode_plan_record_for_layout, ServerPlanCodec, ServerPlanItemCodec, ServerPlanRevisionCodec,
};
use super::schema::{
    compact_plan_file_for, plan_file, plan_item_file, plan_item_payload_file, plan_payload_file,
    plan_revision_file, plan_revision_payload_file, CompactPlanFile, PlanRecord,
    PlanRevisionPayload, PlanRevisionRecord, REMOTE_PLAN_LAYOUT_ID,
};
use super::{binary_error, item_record_payload, u16_len};
use crate::foundation::remote_binary_db::{
    BinaryDbCommandScope, BinaryDbFsyncPolicy, BinaryDbIndexAppender, BinaryDbStoreFsyncPolicy,
    BinaryDbWriteTxn, BinaryFileId, BinaryPayloadFileId, PayloadRange, ServerRemoteBinaryDb,
};
use serde_json::Value as JsonValue;

type RawServerPlanWriteTxn<'a, D, F> = BinaryDbWriteTxn<'a, D, F>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerPlanBinaryDbWritePurpose {
    CreatePlan,
    RevisePlan,
    UpdatePlanStatus,
    TaskStartCreate,
    TaskStartRevise,
    TaskStartExisting,
}

impl ServerPlanBinaryDbWritePurpose {
    fn allows(self, operation: ServerPlanBinaryDbWriteOperation) -> bool {
        use ServerPlanBinaryDbWriteOperation as Op;
        use ServerPlanBinaryDbWritePurpose as Purpose;
        matches!(
            (self, operation),
            (
                Purpose::CreatePlan,
                Op::PlanRecord | Op::PlanItems | Op::PlanRevision
            ) | (
                Purpose::TaskStartCreate,
                Op::PlanRecord | Op::PlanItems | Op::PlanRevision
            ) | (
                Purpose::RevisePlan,
                Op::PlanItems | Op::PlanRevision | Op::PlanUpdate
            ) | (
                Purpose::TaskStartRevise,
                Op::PlanItems | Op::PlanRevision | Op::PlanUpdate
            ) | (Purpose::UpdatePlanStatus, Op::PlanUpdate)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::enum_variant_names)]
enum ServerPlanBinaryDbWriteOperation {
    PlanRecord,
    PlanUpdate,
    PlanRevision,
    PlanItems,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerPlanBinaryDbCommitPoint {
    PlanCreated {
        plan_index: u32,
        revision_index: u32,
    },
    PlanRevised {
        plan_index: u32,
        revision_index: u32,
    },
    PlanStatusUpdated {
        plan_index: u32,
    },
    TaskStarted {
        plan_index: u32,
        revision_index: u32,
        task_index: u32,
    },
}

impl ServerPlanBinaryDbCommitPoint {
    fn validates_purpose(self, purpose: ServerPlanBinaryDbWritePurpose) -> bool {
        matches!(
            (purpose, self),
            (
                ServerPlanBinaryDbWritePurpose::CreatePlan,
                ServerPlanBinaryDbCommitPoint::PlanCreated { .. }
            ) | (
                ServerPlanBinaryDbWritePurpose::RevisePlan,
                ServerPlanBinaryDbCommitPoint::PlanRevised { .. }
            ) | (
                ServerPlanBinaryDbWritePurpose::UpdatePlanStatus,
                ServerPlanBinaryDbCommitPoint::PlanStatusUpdated { .. }
            ) | (
                ServerPlanBinaryDbWritePurpose::TaskStartCreate
                    | ServerPlanBinaryDbWritePurpose::TaskStartRevise
                    | ServerPlanBinaryDbWritePurpose::TaskStartExisting,
                ServerPlanBinaryDbCommitPoint::TaskStarted { .. }
            )
        )
    }
}

pub(crate) struct ServerPlanBinaryDbWriteTxn<'a, D, F, const WRITE_LAYOUT: u32>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
    F: BinaryDbFsyncPolicy,
{
    purpose: ServerPlanBinaryDbWritePurpose,
    inner: RawServerPlanWriteTxn<'a, D, F>,
    commit_point: Option<ServerPlanBinaryDbCommitPoint>,
}

impl<'a, D, const WRITE_LAYOUT: u32>
    ServerPlanBinaryDbWriteTxn<'a, D, BinaryDbStoreFsyncPolicy<'a, D>, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub(super) fn begin(
        db: &'a D,
        purpose: ServerPlanBinaryDbWritePurpose,
    ) -> Result<Self, String> {
        ensure_supported_write_layout(WRITE_LAYOUT)?;
        // Actor liveness belongs to Workflow; keep its validation under the
        // existing Plan+Workflow writer boundary without widening Plan writes.
        let inner = BinaryDbWriteTxn::begin_serving(db, BinaryDbCommandScope::ServerTaskStart)
            .map_err(binary_error)?;
        Ok(Self {
            purpose,
            inner,
            commit_point: None,
        })
    }

    pub(crate) fn begin_task_start(
        db: &'a D,
        purpose: ServerPlanBinaryDbWritePurpose,
    ) -> Result<Self, String> {
        ensure_supported_write_layout(WRITE_LAYOUT)?;
        if !matches!(
            purpose,
            ServerPlanBinaryDbWritePurpose::TaskStartCreate
                | ServerPlanBinaryDbWritePurpose::TaskStartRevise
                | ServerPlanBinaryDbWritePurpose::TaskStartExisting
        ) {
            return Err(format!(
                "Binary DB Plan task-start transaction does not support purpose {purpose:?}"
            ));
        }
        let inner = BinaryDbWriteTxn::begin_serving(db, BinaryDbCommandScope::ServerTaskStart)
            .map_err(binary_error)?;
        Ok(Self {
            purpose,
            inner,
            commit_point: None,
        })
    }
}

impl<'a, D, F, const WRITE_LAYOUT: u32> ServerPlanBinaryDbWriteTxn<'a, D, F, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
    F: BinaryDbFsyncPolicy,
{
    pub(super) fn request_actor(
        &self,
        payload: &serde_json::Map<String, JsonValue>,
    ) -> Result<u32, String> {
        crate::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store::new_frozen(
            self.inner.db().clone(),
        )
        .request_actor_in_write(&self.inner, payload)
    }

    pub(super) fn record_count(&self, file: BinaryFileId) -> Result<u32, String> {
        self.inner.db().record_count(file).map_err(binary_error)
    }

    pub(super) fn matching_head(
        &self,
        plan_index: Option<u32>,
        title: &str,
        status: &str,
        revision_payload: &PlanRevisionPayload,
        items: &[JsonValue],
    ) -> Result<Option<u32>, String> {
        let candidates = match plan_index {
            Some(index) => index..index.checked_add(1).ok_or("Plan index overflow")?,
            None => 0..self.record_count(plan_file())?,
        };
        let expected_revision =
            ServerPlanRevisionCodec::<WRITE_LAYOUT>::encode_payload(revision_payload)?;
        let mut found = None;
        for index in candidates {
            let record = self.current_plan_record_locked(index)?;
            if super::plan_status_from_record(&record)? != status {
                continue;
            }
            let actual_title = self
                .inner
                .read_payload(
                    plan_payload_file(),
                    record.payload_offset,
                    u32::from(record.payload_len),
                )
                .map_err(binary_error)?;
            if actual_title != title.as_bytes() {
                continue;
            }
            let Some(head) = record.latest_revision_index_plus1.checked_sub(1) else {
                continue;
            };
            let revision = ServerPlanRevisionCodec::<WRITE_LAYOUT>::decode_record(
                &self
                    .inner
                    .read_record(plan_revision_file(), head)
                    .map_err(binary_error)?,
            )?;
            if revision.plan_index != index {
                return Err("Plan head references a foreign revision".into());
            }
            if usize::from(revision.item_count) != items.len() {
                continue;
            }
            let actual_payload = self
                .inner
                .read_payload(
                    plan_revision_payload_file(),
                    revision.payload_offset,
                    u32::from(revision.payload_len),
                )
                .map_err(binary_error)?;
            if actual_payload != expected_revision {
                continue;
            }
            let mut same = true;
            for (offset, item) in items.iter().enumerate() {
                let item_index = revision
                    .item_start_index
                    .checked_add(u32::try_from(offset).map_err(|_| "Plan item offset overflow")?)
                    .ok_or("Plan item index overflow")?;
                let actual = ServerPlanItemCodec::<WRITE_LAYOUT>::decode_record(
                    &self
                        .inner
                        .read_record(plan_item_file(), item_index)
                        .map_err(binary_error)?,
                )?;
                let (expected, payload) = item_record_payload(item)?;
                let actual_payload = self
                    .inner
                    .read_payload(
                        plan_item_payload_file(),
                        actual.payload_offset,
                        u32::from(actual.payload_len),
                    )
                    .map_err(binary_error)?;
                if actual.item_meta != expected.item_meta
                    || actual.reserved0 != expected.reserved0
                    || actual.line_number != expected.line_number
                    || actual_payload
                        != ServerPlanItemCodec::<WRITE_LAYOUT>::encode_payload(&payload)?
                {
                    same = false;
                    break;
                }
            }
            if same && found.replace(index).is_some() {
                return Err("Plan create matches multiple existing Plan authorities".into());
            }
        }
        Ok(found)
    }

    pub(super) fn require_unchanged_plan(
        &self,
        plan_index: u32,
        expected: &PlanRecord,
    ) -> Result<(), String> {
        self.ensure_commit_point_open()?;
        if !matches!(
            self.purpose,
            ServerPlanBinaryDbWritePurpose::RevisePlan
                | ServerPlanBinaryDbWritePurpose::UpdatePlanStatus
                | ServerPlanBinaryDbWritePurpose::TaskStartRevise
                | ServerPlanBinaryDbWritePurpose::TaskStartExisting
        ) {
            return Err(format!(
                "Binary DB plan write purpose {:?} cannot validate a mutable plan",
                self.purpose
            ));
        }
        let current = self.current_plan_record_locked(plan_index)?;
        if &current == expected {
            return Ok(());
        }
        Err(format!(
            "Plan PR-{plan_index} state advanced under the Binary DB write lock: expected revision {}, got {}",
            revision_ref(expected.latest_revision_index_plus1),
            revision_ref(current.latest_revision_index_plus1),
        ))
    }

    pub(super) fn append_plan(
        &mut self,
        mut record: PlanRecord,
        title_bytes: &[u8],
    ) -> Result<u32, String> {
        self.ensure_allowed(ServerPlanBinaryDbWriteOperation::PlanRecord)?;
        let range = self.append_payload(plan_payload_file(), title_bytes)?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16_len(range.payload_len as usize, "plan title")?;
        self.inner
            .append_record(
                plan_file(),
                &ServerPlanCodec::<WRITE_LAYOUT>::encode_record(&record)?,
            )
            .map_err(binary_error)
    }

    pub(super) fn overwrite_plan(
        &mut self,
        plan_index: u32,
        mut record: PlanRecord,
        title_bytes: &[u8],
    ) -> Result<(), String> {
        self.ensure_allowed(ServerPlanBinaryDbWriteOperation::PlanUpdate)?;
        let range = self.append_payload(plan_payload_file(), title_bytes)?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16_len(range.payload_len as usize, "plan title")?;
        self.inner
            .overwrite_record(
                plan_file(),
                plan_index,
                &ServerPlanCodec::<WRITE_LAYOUT>::encode_record(&record)?,
            )
            .map_err(binary_error)?;
        Ok(())
    }

    pub(super) fn append_plan_revision(
        &mut self,
        mut record: PlanRevisionRecord,
        payload: &PlanRevisionPayload,
    ) -> Result<u32, String> {
        self.ensure_allowed(ServerPlanBinaryDbWriteOperation::PlanRevision)?;
        let payload_bytes = ServerPlanRevisionCodec::<WRITE_LAYOUT>::encode_payload(payload)?;
        let range = self.append_payload(plan_revision_payload_file(), &payload_bytes)?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16_len(range.payload_len as usize, "plan revision payload")?;
        let revision_index = self
            .inner
            .append_record(
                plan_revision_file(),
                &ServerPlanRevisionCodec::<WRITE_LAYOUT>::encode_record(&record)?,
            )
            .map_err(binary_error)?;
        Ok(revision_index)
    }

    pub(super) fn append_items(&mut self, items: &[JsonValue]) -> Result<(), String> {
        self.ensure_allowed(ServerPlanBinaryDbWriteOperation::PlanItems)?;
        for item in items {
            let (mut record, payload) = item_record_payload(item)?;
            let payload_bytes = ServerPlanItemCodec::<WRITE_LAYOUT>::encode_payload(&payload)?;
            let range = self.append_payload(plan_item_payload_file(), &payload_bytes)?;
            record.payload_offset = range.payload_offset;
            record.payload_len = u16_len(range.payload_len as usize, "plan item payload")?;
            self.inner
                .append_record(
                    plan_item_file(),
                    &ServerPlanItemCodec::<WRITE_LAYOUT>::encode_record(&record)?,
                )
                .map_err(binary_error)?;
        }
        Ok(())
    }

    pub(crate) fn set_commit_point(
        &mut self,
        commit_point: ServerPlanBinaryDbCommitPoint,
    ) -> Result<(), String> {
        self.ensure_commit_point_open()?;
        if !commit_point.validates_purpose(self.purpose) {
            return Err(format!(
                "Binary DB plan write purpose {:?} cannot commit at {:?}",
                self.purpose, commit_point
            ));
        }
        self.commit_point = Some(commit_point);
        Ok(())
    }

    pub(crate) fn commit(mut self) -> Result<ServerPlanBinaryDbCommitPoint, String> {
        let commit_point = match self.commit_point {
            Some(commit_point) => commit_point,
            None => {
                self.inner.abort().map_err(binary_error)?;
                return Err(format!(
                    "Binary DB plan write purpose {:?} reached commit without a commit point",
                    self.purpose
                ));
            }
        };
        self.inner.commit().map_err(binary_error)?;
        Ok(commit_point)
    }

    pub(crate) fn workflow_write(
        &mut self,
    ) -> Result<&mut RawServerPlanWriteTxn<'a, D, F>, String> {
        self.ensure_commit_point_open()?;
        if !matches!(
            self.purpose,
            ServerPlanBinaryDbWritePurpose::TaskStartCreate
                | ServerPlanBinaryDbWritePurpose::TaskStartRevise
                | ServerPlanBinaryDbWritePurpose::TaskStartExisting
        ) {
            return Err(format!(
                "Binary DB plan write purpose {:?} cannot expose Workflow mutation access",
                self.purpose
            ));
        }
        Ok(&mut self.inner)
    }

    fn append_payload(
        &mut self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
    ) -> Result<PayloadRange, String> {
        self.ensure_commit_point_open()?;
        self.inner.append_payload(file, bytes).map_err(binary_error)
    }

    fn ensure_allowed(&self, operation: ServerPlanBinaryDbWriteOperation) -> Result<(), String> {
        self.ensure_commit_point_open()?;
        if self.purpose.allows(operation) {
            Ok(())
        } else {
            Err(format!(
                "Binary DB plan write purpose {:?} cannot perform {:?}",
                self.purpose, operation
            ))
        }
    }

    fn ensure_commit_point_open(&self) -> Result<(), String> {
        if let Some(commit_point) = self.commit_point {
            return Err(format!(
                "Binary DB plan write purpose {:?} already reached commit point {:?}",
                self.purpose, commit_point
            ));
        }
        Ok(())
    }

    fn current_plan_record_locked(&self, plan_index: u32) -> Result<PlanRecord, String> {
        let layout = self
            .inner
            .db()
            .layout_id(plan_file())
            .map_err(binary_error)?;
        let file = compact_plan_file_for(layout, CompactPlanFile::Plan)?;
        let raw = self
            .inner
            .read_record(file, plan_index)
            .map_err(binary_error)?;
        decode_plan_record_for_layout(layout, &raw)
    }
}

fn revision_ref(index_plus1: u32) -> String {
    index_plus1
        .checked_sub(1)
        .map(|index| format!("plan-revision:{index}"))
        .unwrap_or_else(|| "<none>".to_string())
}

fn ensure_supported_write_layout(layout: u32) -> Result<(), String> {
    if layout == REMOTE_PLAN_LAYOUT_ID {
        return Ok(());
    }
    Err(format!(
        "unsupported server Plan Binary DB write layout {layout}; supported layout is {REMOTE_PLAN_LAYOUT_ID}"
    ))
}
