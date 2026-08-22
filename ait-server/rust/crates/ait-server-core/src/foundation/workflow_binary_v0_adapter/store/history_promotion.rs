use super::task_change::PayloadSyncBoundary;
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const HISTORY_PROMOTION_CONTRACT: &str = "history-promotion-prepare/v1";
pub(super) const HISTORY_PROMOTION_STAGED_CONTRACT: &str = "history-promotion-prepare/v2";
pub(super) const HISTORY_PROMOTION_SUMMARY_PREFIX: &str = "ait-history-promotion/v1 ";
pub(super) const HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX: &str = "ait-history-promotion-stage/v1 ";
pub(super) const LOCAL_LAND_RECEIPT_SUMMARY_PREFIX: &str = "ait-local-land-receipt/v1 ";
const MAX_HISTORY_PROMOTION_ENTRIES: usize = 64;
const MAX_HISTORY_SNAPSHOTS_PER_ENTRY: usize = 64;

#[derive(Clone, Debug)]
struct HistorySnapshotDag {
    snapshot_count: u32,
    snapshot_records: Vec<ServerBinarySnapshotRecord>,
    parent_edges_by_child: BTreeMap<u32, Vec<(u16, u32)>>,
}

pub(super) fn source_kind_for_summary(summary: &str) -> (&'static str, bool) {
    if summary.starts_with(LOCAL_LAND_RECEIPT_SUMMARY_PREFIX) {
        ("imported_local_land_receipt", false)
    } else if summary.starts_with(HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX) {
        ("history_promotion_stage", false)
    } else if summary.starts_with(HISTORY_PROMOTION_SUMMARY_PREFIX) {
        ("history_promotion_aggregate", true)
    } else {
        ("remote_patchset", true)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HistoryPromotionRequest {
    contract: String,
    idempotency_key: String,
    target_line: String,
    base_snapshot_id: String,
    revision_snapshot_id: String,
    author_mode: String,
    #[serde(default)]
    summary: Option<String>,
    entries: Vec<HistoryPromotionEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StagedHistoryPromotionRequest {
    contract: String,
    promotion_id: String,
    idempotency_key: String,
    target_line: String,
    base_snapshot_id: String,
    revision_snapshot_id: String,
    stage_ordinal: u64,
    stage_base_snapshot_id: String,
    stage_revision_snapshot_id: String,
    #[serde(default)]
    previous_stage_patchset_id: Option<String>,
    total_entry_count: u64,
    final_stage: bool,
    author_mode: String,
    #[serde(default)]
    summary: Option<String>,
    entries: Vec<HistoryPromotionEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HistoryPromotionEntry {
    local_task_id: String,
    local_change_id: String,
    local_change_ref: String,
    #[serde(default)]
    expected_remote_task_id: Option<String>,
    #[serde(default)]
    expected_remote_change_ref: Option<String>,
    task: HistoryPromotionTask,
    change: HistoryPromotionChange,
    pre_land_target_snapshot_id: String,
    landed_snapshot_id: String,
    landed_at_s: u64,
    snapshots: Vec<HistoryPromotionSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HistoryPromotionTask {
    title: String,
    intent: String,
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    origin_plan_revision_id: Option<String>,
    #[serde(default)]
    plan_item_ref: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HistoryPromotionChange {
    title: String,
    base_line: String,
    fork_snapshot_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HistoryPromotionSnapshot {
    snapshot_id: String,
    created_at_s: u64,
}

type HistoryPromotionPlanBinding = (u32, u32, u64);

#[derive(Clone, Debug, Deserialize)]
struct PersistedHistoryManifest {
    contract: String,
    target_line: String,
    base_snapshot_id: String,
    revision_snapshot_id: String,
    entries: Vec<PersistedHistoryReceipt>,
    aggregate: PersistedHistoryAggregate,
    #[serde(default)]
    promotion_id: Option<String>,
    #[serde(default)]
    stage_ordinal: Option<u64>,
    #[serde(default)]
    stage_base_snapshot_id: Option<String>,
    #[serde(default)]
    stage_revision_snapshot_id: Option<String>,
    #[serde(default)]
    previous_stage_patchset_index: Option<u32>,
    #[serde(default)]
    previous_stage_patchset_id: Option<String>,
    #[serde(default)]
    total_entry_count: Option<u64>,
    #[serde(default)]
    final_stage: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
struct PersistedHistoryReceipt {
    task_index: u32,
    task_id: String,
    change_index: u32,
    change_ref: String,
    receipt_patchset_index: u32,
    receipt_patchset_id: String,
    pre_land_target_snapshot_id: String,
    landed_snapshot_id: String,
    landed_at_s: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct PersistedHistoryAggregate {
    change_index: u32,
    change_ref: String,
    patchset_index: u32,
    patchset_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct PersistedLocalLandReceipt {
    contract: String,
    local_task_id: String,
    local_change_id: String,
    local_change_ref: String,
    task_id: String,
    change_ref: String,
    target_line: String,
    pre_land_target_snapshot_id: String,
    landed_snapshot_id: String,
    landed_at_s: u64,
    source_kind: String,
}

#[derive(Clone, Debug)]
struct ReusableHistoryReceipt {
    task_index: u32,
    task_id: String,
    change_index: u32,
    change_ref: String,
    patchset_index: u32,
    patchset_id: String,
}

#[derive(Clone, Copy)]
struct HistoryPromotionWriteContext<'a> {
    line_head_snapshot_id: &'a str,
    manifest_contract: &'a str,
    manifest_base_snapshot_id: &'a str,
    manifest_revision_snapshot_id: &'a str,
    summary_prefix: &'a str,
    patchset_base_snapshot_id: &'a str,
    patchset_revision_snapshot_id: &'a str,
    staged_request: Option<&'a StagedHistoryPromotionRequest>,
    force_select: bool,
}

fn nonempty(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{label} must be a non-empty string."))
    } else {
        Ok(value.to_string())
    }
}

fn request_sha256(payload: &JsonValue) -> Result<String, String> {
    let mut fingerprint_payload = payload.clone();
    if let Some(entries) = fingerprint_payload
        .get_mut("entries")
        .and_then(JsonValue::as_array_mut)
    {
        for entry in entries {
            if let Some(entry) = entry.as_object_mut() {
                entry.remove("expected_remote_task_id");
                entry.remove("expected_remote_change_ref");
            }
        }
    }
    let encoded = serde_json::to_vec(&fingerprint_payload)
        .map_err(|error| format!("History promotion request encoding failed: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn validate_expected_publication_identity(entry: &HistoryPromotionEntry) -> Result<(), String> {
    for (label, value) in [
        (
            "expected_remote_task_id",
            entry.expected_remote_task_id.as_deref(),
        ),
        (
            "expected_remote_change_ref",
            entry.expected_remote_change_ref.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            if value.is_empty() || value.trim() != value {
                return Err(format!(
                    "History promotion {label} must be a non-empty exact identity."
                ));
            }
        }
    }
    if entry.expected_remote_task_id.is_none() && entry.expected_remote_change_ref.is_some() {
        return Err(
            "History promotion expected_remote_change_ref requires expected_remote_task_id."
                .to_string(),
        );
    }
    Ok(())
}

fn validate_expected_publication_owner(
    entry: &HistoryPromotionEntry,
    task_id: &str,
    change_ref: &str,
) -> Result<(), String> {
    if entry
        .expected_remote_task_id
        .as_deref()
        .is_some_and(|expected| expected != task_id)
        || entry
            .expected_remote_change_ref
            .as_deref()
            .is_some_and(|expected| expected != change_ref)
    {
        return Err(format!(
            "HISTORY_PROMOTION_RECEIPT_CONFLICT: expected publication owner for {} does not match {task_id}/{change_ref}.",
            entry.local_change_ref
        ));
    }
    Ok(())
}

fn validate_replayed_publication_owners(
    request: &HistoryPromotionRequest,
    manifest: &JsonValue,
) -> Result<(), String> {
    let manifest_entries = manifest
        .get("entries")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "History promotion replay manifest has no entries.".to_string())?;
    if manifest_entries.len() != request.entries.len() {
        return Err(
            "History promotion replay manifest entry count differs from the request.".to_string(),
        );
    }
    for (entry, persisted) in request.entries.iter().zip(manifest_entries) {
        let persisted_local_task_id = persisted
            .get("local_task_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                "History promotion replay manifest entry has no local_task_id.".to_string()
            })?;
        let persisted_local_change_ref = persisted
            .get("local_change_ref")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                "History promotion replay manifest entry has no local_change_ref.".to_string()
            })?;
        if persisted_local_task_id != entry.local_task_id
            || persisted_local_change_ref != entry.local_change_ref
        {
            return Err(format!(
                "HISTORY_PROMOTION_RECEIPT_CONFLICT: replayed local identity does not match {}.",
                entry.local_change_ref
            ));
        }
        let task_id = persisted
            .get("task_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "History promotion replay manifest entry has no task_id.".to_string())?;
        let change_ref = persisted
            .get("change_ref")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                "History promotion replay manifest entry has no change_ref.".to_string()
            })?;
        validate_expected_publication_owner(entry, task_id, change_ref)?;
    }
    Ok(())
}

fn parse_summary(summary: &str, prefix: &str) -> Result<Option<JsonValue>, String> {
    let Some(encoded) = summary.strip_prefix(prefix) else {
        return Ok(None);
    };
    serde_json::from_str(encoded)
        .map(Some)
        .map_err(|error| format!("Persisted {prefix:?} summary is invalid JSON: {error}"))
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn history_manifest_for_patchset<A: ReadV0>(
        &self,
        read: &A,
        patchset_index: u32,
    ) -> Result<Option<JsonValue>, String> {
        let summary = self.history_patchset_summary(read, patchset_index)?;
        parse_summary(&summary, HISTORY_PROMOTION_SUMMARY_PREFIX)
    }

    fn history_patchset_summary<A: ReadV0>(
        &self,
        read: &A,
        patchset_index: u32,
    ) -> Result<String, String> {
        let record = self
            .read_patchset(read, patchset_index)
            .map_err(|error| Self::error("history promotion Patchset read", error))?;
        let raw = read
            .read_payload_v0(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                record.summary_offset,
                u32::from(record.summary_len),
            )
            .map_err(|error| Self::error("history promotion summary read", error))?;
        WorkflowBinaryV0Codec::decode_single_text_payload(&raw, "Patchset summary")
            .map(str::to_string)
            .map_err(|error| Self::error("history promotion summary decode", error))
    }

    fn staged_history_chain_for_final<A: ReadV0>(
        &self,
        read: &A,
        final_patchset_index: u32,
    ) -> Result<Vec<PersistedHistoryManifest>, String> {
        let mut stages_newest_first = Vec::new();
        let mut seen_patchsets = BTreeSet::new();
        let mut current_patchset_index = final_patchset_index;
        let mut expected_ordinal = None;
        let mut global_authority: Option<(String, String, String, String, u64)> = None;

        loop {
            if !seen_patchsets.insert(current_patchset_index) {
                return Err(
                    "HISTORY_PROMOTION_STAGE_CONFLICT: stage chain contains a cycle.".to_string(),
                );
            }
            let summary = self.history_patchset_summary(read, current_patchset_index)?;
            let is_final = stages_newest_first.is_empty();
            let manifest_value = if is_final {
                parse_summary(&summary, HISTORY_PROMOTION_SUMMARY_PREFIX)?.ok_or_else(|| {
                    "Selected history promotion Patchset has no aggregate manifest.".to_string()
                })?
            } else {
                parse_summary(&summary, HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX)?.ok_or_else(
                    || {
                        "HISTORY_PROMOTION_STAGE_CONFLICT: predecessor is not a promotion stage."
                            .to_string()
                    },
                )?
            };
            let manifest: PersistedHistoryManifest = serde_json::from_value(manifest_value)
                .map_err(|error| {
                    format!("Persisted history promotion stage is invalid: {error}")
                })?;
            let ordinal = manifest
                .stage_ordinal
                .ok_or_else(|| "Persisted history promotion stage has no ordinal.".to_string())?;
            let promotion_id = manifest
                .promotion_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "Persisted history promotion stage has no promotion identity.".to_string()
                })?;
            let total_entry_count = manifest.total_entry_count.ok_or_else(|| {
                "Persisted history promotion stage has no total entry count.".to_string()
            })?;
            let stage_base = manifest
                .stage_base_snapshot_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "Persisted history promotion stage has no base Snapshot.".to_string()
                })?;
            let _stage_revision = manifest
                .stage_revision_snapshot_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "Persisted history promotion stage has no revision Snapshot.".to_string()
                })?;
            if total_entry_count == 0
                || manifest.entries.is_empty()
                || manifest.entries.len() > MAX_HISTORY_PROMOTION_ENTRIES
            {
                return Err(
                    "Persisted history promotion stage has an invalid bounded entry inventory."
                        .to_string(),
                );
            }
            let stage_start = ordinal
                .checked_mul(MAX_HISTORY_PROMOTION_ENTRIES as u64)
                .ok_or_else(|| "Persisted history promotion stage ordinal overflow.".to_string())?;
            if stage_start >= total_entry_count {
                return Err(
                    "Persisted history promotion stage starts beyond its total entry count."
                        .to_string(),
                );
            }
            let expected_entry_count =
                (total_entry_count - stage_start).min(MAX_HISTORY_PROMOTION_ENTRIES as u64);
            let expected_final = stage_start + expected_entry_count == total_entry_count;
            if manifest.entries.len() as u64 != expected_entry_count
                || manifest.final_stage != Some(expected_final)
                || is_final != expected_final
                || (is_final && manifest.contract != "ait-history-promotion/v2")
                || (!is_final && manifest.contract != "ait-history-promotion-stage/v1")
            {
                return Err(
                    "Persisted history promotion stage geometry is inconsistent.".to_string(),
                );
            }
            if let Some(expected) = expected_ordinal {
                if ordinal != expected {
                    return Err(format!(
                        "HISTORY_PROMOTION_STAGE_CONFLICT: expected predecessor stage {expected}, got {ordinal}."
                    ));
                }
            }

            let patchset = self
                .read_patchset(read, current_patchset_index)
                .map_err(|error| Self::error("history promotion stage Patchset read", error))?;
            let change = self
                .read_change(read, patchset.change_index)
                .map_err(|error| Self::error("history promotion stage Change read", error))?;
            if manifest.aggregate.patchset_index != current_patchset_index
                || manifest.aggregate.change_index != patchset.change_index
                || manifest.aggregate.change_ref
                    != self.change_ref(change.task_index, change.change_ordinal)
                || manifest.aggregate.patchset_id
                    != self.patchset_id(change, patchset.patch_ordinal)
            {
                return Err(
                    "Persisted history promotion stage Patchset authority is inconsistent."
                        .to_string(),
                );
            }

            let authority = (
                promotion_id.to_string(),
                manifest.target_line.clone(),
                manifest.base_snapshot_id.clone(),
                manifest.revision_snapshot_id.clone(),
                total_entry_count,
            );
            if let Some(global) = &global_authority {
                if global != &authority {
                    return Err(
                        "HISTORY_PROMOTION_STAGE_CONFLICT: stage global authority diverges."
                            .to_string(),
                    );
                }
            } else {
                global_authority = Some(authority);
            }

            let previous_index = manifest.previous_stage_patchset_index;
            let previous_id = manifest.previous_stage_patchset_id.as_deref();
            if ordinal == 0 {
                if previous_index.is_some()
                    || previous_id.is_some()
                    || stage_base != manifest.base_snapshot_id
                {
                    return Err(
                        "History promotion stage zero has invalid predecessor or base authority."
                            .to_string(),
                    );
                }
                stages_newest_first.push(manifest);
                break;
            }
            let previous_index = previous_index.ok_or_else(|| {
                "History promotion continuation has no predecessor Patchset index.".to_string()
            })?;
            let previous_id = previous_id.ok_or_else(|| {
                "History promotion continuation has no predecessor Patchset identity.".to_string()
            })?;
            let previous_patchset = self
                .read_patchset(read, previous_index)
                .map_err(|error| Self::error("history promotion predecessor read", error))?;
            let previous_change = self
                .read_change(read, previous_patchset.change_index)
                .map_err(|error| Self::error("history promotion predecessor Change read", error))?;
            if self.patchset_id(previous_change, previous_patchset.patch_ordinal) != previous_id {
                return Err(
                    "HISTORY_PROMOTION_STAGE_CONFLICT: predecessor Patchset identity mismatches its index."
                        .to_string(),
                );
            }
            expected_ordinal = Some(ordinal - 1);
            stages_newest_first.push(manifest);
            current_patchset_index = previous_index;
        }

        stages_newest_first.reverse();
        let mut expected_snapshot = stages_newest_first
            .first()
            .map(|stage| stage.base_snapshot_id.clone())
            .ok_or_else(|| "History promotion stage chain is empty.".to_string())?;
        let mut seen_tasks = BTreeSet::new();
        let mut seen_changes = BTreeSet::new();
        let mut entry_count = 0_u64;
        for (expected_stage_ordinal, stage) in stages_newest_first.iter().enumerate() {
            if stage.stage_ordinal != Some(expected_stage_ordinal as u64)
                || stage.stage_base_snapshot_id.as_deref() != Some(expected_snapshot.as_str())
            {
                return Err("HISTORY_PROMOTION_STAGE_CONFLICT: stage chain has a gap.".to_string());
            }
            for receipt in &stage.entries {
                if receipt.pre_land_target_snapshot_id != expected_snapshot
                    || !seen_tasks.insert(receipt.task_index)
                    || !seen_changes.insert(receipt.change_index)
                {
                    return Err(
                        "History promotion stage receipts repeat an owner or break the Land chain."
                            .to_string(),
                    );
                }
                expected_snapshot = receipt.landed_snapshot_id.clone();
                entry_count = entry_count
                    .checked_add(1)
                    .ok_or_else(|| "History promotion entry count overflow.".to_string())?;
            }
            if stage.stage_revision_snapshot_id.as_deref() != Some(expected_snapshot.as_str()) {
                return Err(
                    "History promotion stage revision does not match its final receipt."
                        .to_string(),
                );
            }
        }
        let final_stage = stages_newest_first
            .last()
            .expect("non-empty history promotion stage chain");
        if expected_snapshot != final_stage.revision_snapshot_id
            || final_stage.total_entry_count != Some(entry_count)
        {
            return Err(
                "History promotion stage chain does not cover its global revision.".to_string(),
            );
        }
        Ok(stages_newest_first)
    }

    fn validate_landed_history_receipt_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        receipt: &PersistedHistoryReceipt,
        target_line_index: u32,
        expected_pre_land_index: u32,
    ) -> Result<u32, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let change = self
            .read_change(tx, receipt.change_index)
            .map_err(|error| Self::error("landed history receipt Change read", error))?;
        let task = self
            .read_task(tx, receipt.task_index)
            .map_err(|error| Self::error("landed history receipt Task read", error))?;
        let patchset = self
            .read_patchset(tx, receipt.receipt_patchset_index)
            .map_err(|error| Self::error("landed history receipt Patchset read", error))?;
        let pre_land_index = self.snapshot_index_in_history_write(
            tx,
            &receipt.pre_land_target_snapshot_id,
            "landed history receipt pre-Land",
        )?;
        let landed_index = self.snapshot_index_in_history_write(
            tx,
            &receipt.landed_snapshot_id,
            "landed history receipt revision",
        )?;
        let change_owner = WorkflowBinaryV0Codec::decode_ordinal_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("change_land_index.bin"),
                receipt.change_index,
            )
            .map_err(|error| Self::error("landed history receipt Change Land index", error))?,
        )
        .map_err(|error| Self::error("landed history receipt Change Land index", error))?;
        let land_index = change_owner
            .latest_index_plus1
            .checked_sub(1)
            .ok_or_else(|| {
                format!(
                    "History receipt Change {} is terminal without a Land.",
                    receipt.change_ref
                )
            })?;
        let land = WorkflowBinaryV0Codec::decode_land(
            &tx.read_record(WorkflowBinaryV0Codec::land_file(), land_index)
                .map_err(|error| Self::error("landed history receipt Land read", error))?,
        )
        .map_err(|error| Self::error("landed history receipt Land decode", error))?;
        let task_owner =
            self.task_inventory_in_write(tx, "task_land_index.bin", receipt.task_index)?;
        let expected_land_meta = LAND_STATUS_SUCCEEDED
            | LAND_HAS_PRE_TARGET
            | LAND_HAS_LANDED_SNAPSHOT
            | (LAND_MODE_DIRECT << 5);
        if receipt.task_id != self.task_id(receipt.task_index)
            || change.task_index != receipt.task_index
            || receipt.change_ref != self.change_ref(change.task_index, change.change_ordinal)
            || patchset.change_index != receipt.change_index
            || receipt.receipt_patchset_id != self.patchset_id(change, patchset.patch_ordinal)
            || patchset.base_snapshot_index != pre_land_index
            || patchset.revision_snapshot_index != landed_index
            || pre_land_index != expected_pre_land_index
            || change.lifecycle() != CHANGE_LIFECYCLE_LANDED
            || change.updated_at_s != receipt.landed_at_s
            || task.task_meta & TASK_META_COMPLETED == 0
            || task.closed_at_s != receipt.landed_at_s
            || task.updated_at_s != receipt.landed_at_s
            || land.land_meta != expected_land_meta
            || land.land_ordinal != 0
            || land.change_ordinal != change.change_ordinal
            || land.failure_kind != 0
            || land.change_index != receipt.change_index
            || land.patchset_index != receipt.receipt_patchset_index
            || land.previous_task_land_index_plus1 != 0
            || land.previous_change_land_index_plus1 != 0
            || land.pre_land_target_snapshot_index_plus1 != pre_land_index + 1
            || land.landed_snapshot_index_plus1 != landed_index + 1
            || land.submitted_at_s != receipt.landed_at_s
            || land.updated_at_s != receipt.landed_at_s
            || land.target_line_index_plus1 != target_line_index + 1
            || change_owner.latest_index_plus1 != land_index + 1
            || change_owner.count != 1
            || change_owner.next_ordinal != 1
            || task_owner.latest_index_plus1 != land_index + 1
            || task_owner.count != 1
        {
            return Err(format!(
                "HISTORY_PROMOTION_RECEIPT_CONFLICT: landed receipt {} does not match its exact imported Land authority.",
                receipt.change_ref
            ));
        }
        Ok(landed_index)
    }

    fn apply_history_receipt_batch_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        receipts: &[PersistedHistoryReceipt],
        target_line_index: u32,
        expected_base_snapshot_id: &str,
        expected_revision_snapshot_id: &str,
    ) -> Result<usize, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        if receipts.is_empty() || receipts.len() > MAX_HISTORY_PROMOTION_ENTRIES {
            return Err(format!(
                "History receipt writer stage requires 1..={MAX_HISTORY_PROMOTION_ENTRIES} entries."
            ));
        }
        let mut expected_pre_land_index = self.snapshot_index_in_history_write(
            tx,
            expected_base_snapshot_id,
            "history receipt stage base",
        )?;
        let expected_revision_index = self.snapshot_index_in_history_write(
            tx,
            expected_revision_snapshot_id,
            "history receipt stage revision",
        )?;
        let mut seen_tasks = BTreeSet::new();
        let mut seen_changes = BTreeSet::new();
        let land_file = WorkflowBinaryV0Codec::land_file();
        let first_land_index = tx
            .record_count(land_file.clone())
            .map_err(|error| Self::error("history receipt Land inventory", error))?;
        let mut land_records = Vec::with_capacity(receipts.len());
        let mut expected_land_indexes = Vec::with_capacity(receipts.len());
        let mut change_land_rows = Vec::with_capacity(receipts.len());
        let mut task_land_rows = Vec::with_capacity(receipts.len());
        let mut change_rows = Vec::with_capacity(receipts.len());
        let mut task_dependency_rows = Vec::with_capacity(receipts.len());
        let mut task_final_rows = Vec::with_capacity(receipts.len());

        for receipt in receipts {
            if !seen_tasks.insert(receipt.task_index)
                || !seen_changes.insert(receipt.change_index)
                || receipt.landed_at_s == 0
            {
                return Err(
                    "History promotion stage repeats an owner or has an invalid event time."
                        .to_string(),
                );
            }
            if receipt.task_id != self.task_id(receipt.task_index) {
                return Err(format!(
                    "History receipt Task identity {} does not match index {}.",
                    receipt.task_id, receipt.task_index
                ));
            }
            let mut change = self
                .read_change(tx, receipt.change_index)
                .map_err(|error| Self::error("history receipt Change read", error))?;
            if change.task_index != receipt.task_index
                || receipt.change_ref != self.change_ref(change.task_index, change.change_ordinal)
            {
                return Err(format!(
                    "History receipt Change {} does not match its persisted owner.",
                    receipt.change_ref
                ));
            }
            let receipt_patchset = self
                .read_patchset(tx, receipt.receipt_patchset_index)
                .map_err(|error| Self::error("history receipt Patchset read", error))?;
            if receipt_patchset.change_index != receipt.change_index
                || receipt.receipt_patchset_id
                    != self.patchset_id(change, receipt_patchset.patch_ordinal)
            {
                return Err(format!(
                    "History receipt Patchset {} does not match Change {}.",
                    receipt.receipt_patchset_id, receipt.change_ref
                ));
            }
            let receipt_summary =
                self.read_patchset_summary_in_write(tx, receipt.receipt_patchset_index)?;
            let receipt_value = parse_summary(&receipt_summary, LOCAL_LAND_RECEIPT_SUMMARY_PREFIX)?
                .ok_or_else(|| {
                    format!(
                        "Patchset {} is not an imported local Land receipt.",
                        receipt.receipt_patchset_id
                    )
                })?;
            let receipt_authority: PersistedLocalLandReceipt =
                serde_json::from_value(receipt_value)
                    .map_err(|error| format!("Persisted local Land receipt is invalid: {error}"))?;
            if receipt_authority.contract != "ait-local-land-receipt/v1"
                || receipt_authority.task_id != receipt.task_id
                || receipt_authority.change_ref != receipt.change_ref
                || receipt_authority.pre_land_target_snapshot_id
                    != receipt.pre_land_target_snapshot_id
                || receipt_authority.landed_snapshot_id != receipt.landed_snapshot_id
                || receipt_authority.landed_at_s != receipt.landed_at_s
                || receipt_authority.source_kind != "imported_local_land_receipt"
            {
                return Err(format!(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: receipt Patchset {} disagrees with its promotion manifest.",
                    receipt.receipt_patchset_id
                ));
            }
            let pre_land_index = self.snapshot_index_in_history_write(
                tx,
                &receipt.pre_land_target_snapshot_id,
                "history receipt pre-Land",
            )?;
            let landed_index = self.snapshot_index_in_history_write(
                tx,
                &receipt.landed_snapshot_id,
                "history receipt landed",
            )?;
            if pre_land_index != expected_pre_land_index
                || receipt_patchset.base_snapshot_index != pre_land_index
                || receipt_patchset.revision_snapshot_index != landed_index
            {
                return Err(format!(
                    "History receipt {} breaks the ordered Land chain.",
                    receipt.change_ref
                ));
            }
            if change.lifecycle() == CHANGE_LIFECYCLE_LANDED {
                expected_pre_land_index = self.validate_landed_history_receipt_in_write(
                    tx,
                    receipt,
                    target_line_index,
                    expected_pre_land_index,
                )?;
                continue;
            }
            if change.lifecycle() == CHANGE_LIFECYCLE_ARCHIVED
                || change.change_state & CHANGE_STATE_CANCELED != 0
            {
                return Err(format!(
                    "History receipt Change {} is terminal before its imported Land.",
                    receipt.change_ref
                ));
            }
            let mut task = self
                .read_task(tx, receipt.task_index)
                .map_err(|error| Self::error("history receipt Task read", error))?;
            if task.is_terminal() {
                return Err(format!(
                    "History receipt Task {} is terminal before its imported Land.",
                    receipt.task_id
                ));
            }
            let mut change_owner = WorkflowBinaryV0Codec::decode_ordinal_index(
                &tx.read_record(
                    WorkflowBinaryV0Codec::chain_index_file("change_land_index.bin"),
                    receipt.change_index,
                )
                .map_err(|error| Self::error("history receipt Change Land index", error))?,
            )
            .map_err(|error| Self::error("history receipt Change Land index", error))?;
            if change_owner.next_ordinal >= 64 {
                return Err(format!(
                    "History receipt Change {} has exhausted Land ordinals.",
                    receipt.change_ref
                ));
            }
            let mut task_owner =
                self.task_inventory_in_write(tx, "task_land_index.bin", receipt.task_index)?;
            let land = V0LandRecord {
                land_meta: LAND_STATUS_SUCCEEDED
                    | LAND_HAS_PRE_TARGET
                    | LAND_HAS_LANDED_SNAPSHOT
                    | (LAND_MODE_DIRECT << 5),
                land_ordinal: change_owner.next_ordinal,
                change_ordinal: change.change_ordinal,
                failure_kind: 0,
                change_index: receipt.change_index,
                patchset_index: receipt.receipt_patchset_index,
                previous_task_land_index_plus1: task_owner.latest_index_plus1,
                previous_change_land_index_plus1: change_owner.latest_index_plus1,
                pre_land_target_snapshot_index_plus1: pre_land_index + 1,
                landed_snapshot_index_plus1: landed_index + 1,
                submitted_at_s: receipt.landed_at_s,
                updated_at_s: receipt.landed_at_s,
                target_line_index_plus1: target_line_index + 1,
            };
            let receipt_offset = u32::try_from(land_records.len())
                .map_err(|_| "History receipt Land batch exceeds u32.".to_string())?;
            let land_index = first_land_index
                .checked_add(receipt_offset)
                .ok_or_else(|| "History receipt Land index exceeds u32.".to_string())?;
            expected_land_indexes.push(land_index);
            land_records.push(
                WorkflowBinaryV0Codec::encode_land(land)
                    .map_err(|error| Self::error("history receipt Land encode", error))?,
            );
            change_owner.latest_index_plus1 = land_index + 1;
            change_owner.count = change_owner
                .count
                .checked_add(1)
                .ok_or_else(|| "History receipt Change Land count exceeds u16.".to_string())?;
            change_owner.next_ordinal += 1;
            change_land_rows.push((
                receipt.change_index,
                WorkflowBinaryV0Codec::encode_ordinal_index(change_owner)
                    .map_err(|error| Self::error("history receipt Change Land index", error))?,
            ));
            task_owner.latest_index_plus1 = land_index + 1;
            task_owner.count = task_owner
                .count
                .checked_add(1)
                .ok_or_else(|| "History receipt Task Land count exceeds u16.".to_string())?;
            task_land_rows.push((
                receipt.task_index,
                WorkflowBinaryV0Codec::encode_inventory_index(task_owner)
                    .map_err(|error| Self::error("history receipt Task Land index", error))?,
            ));
            change.change_meta =
                (change.change_meta & CHANGE_META_HAS_PATCHSETS) | CHANGE_LIFECYCLE_LANDED;
            change.updated_at_s = receipt.landed_at_s;
            change_rows.push((
                receipt.change_index,
                WorkflowBinaryV0Codec::encode_change(change)
                    .map_err(|error| Self::error("history receipt Change closeout", error))?,
            ));
            let mut task_dependency = WorkflowBinaryV0Codec::encode_task(task)
                .map_err(|error| Self::error("history receipt Task dependency", error))?;
            task_dependency[52..60].copy_from_slice(&receipt.landed_at_s.to_le_bytes());
            task_dependency_rows.push((receipt.task_index, task_dependency));
            task.closed_at_s = receipt.landed_at_s;
            task.updated_at_s = receipt.landed_at_s;
            task.task_meta |= TASK_META_COMPLETED;
            task_final_rows.push((
                receipt.task_index,
                WorkflowBinaryV0Codec::encode_task(task)
                    .map_err(|error| Self::error("history receipt Task closeout", error))?,
            ));
            expected_pre_land_index = landed_index;
        }
        if expected_pre_land_index != expected_revision_index {
            return Err(
                "History promotion receipt stage does not end at its declared revision Snapshot."
                    .to_string(),
            );
        }
        if !land_records.is_empty() {
            let appended_land_indexes = tx
                .append_records(land_file, &land_records)
                .map_err(|error| Self::error("history receipt Land append", error))?;
            if appended_land_indexes != expected_land_indexes {
                return Err(
                    "History receipt Land batch did not preserve its predicted contiguous indexes."
                        .to_string(),
                );
            }
            tx.overwrite_records(
                WorkflowBinaryV0Codec::chain_index_file("change_land_index.bin"),
                &change_land_rows,
            )
            .map_err(|error| Self::error("history receipt Change Land indexes", error))?;
            tx.overwrite_records(
                WorkflowBinaryV0Codec::chain_index_file("task_land_index.bin"),
                &task_land_rows,
            )
            .map_err(|error| Self::error("history receipt Task Land indexes", error))?;
            tx.overwrite_records(WorkflowBinaryV0Codec::change_file(), &change_rows)
                .map_err(|error| Self::error("history receipt Change closeout", error))?;
            tx.overwrite_records(WorkflowBinaryV0Codec::task_file(), &task_dependency_rows)
                .map_err(|error| Self::error("history receipt Task dependencies", error))?;
            self.sync_file(tx, "task.bin")
                .map_err(|error| Self::error("history receipt Task dependency sync", error))?;
            tx.overwrite_records(WorkflowBinaryV0Codec::task_file(), &task_final_rows)
                .map_err(|error| Self::error("history receipt Task closeout", error))?;
        }
        Ok(land_records.len())
    }

    pub(super) fn apply_history_receipts_in_land_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        aggregate_patchset_index: u32,
        aggregate_change_index: u32,
        target_line: &str,
        target_line_index: u32,
        current_head_index: Option<u32>,
    ) -> Result<Option<JsonValue>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let Some(manifest_value) =
            self.history_manifest_for_patchset(tx, aggregate_patchset_index)?
        else {
            return Ok(None);
        };
        let manifest: PersistedHistoryManifest = serde_json::from_value(manifest_value.clone())
            .map_err(|error| format!("Persisted history promotion manifest is invalid: {error}"))?;
        if !matches!(
            manifest.contract.as_str(),
            "ait-history-promotion/v1" | "ait-history-promotion/v2"
        ) || manifest.target_line != target_line
            || manifest.aggregate.patchset_index != aggregate_patchset_index
            || manifest.aggregate.change_index != aggregate_change_index
        {
            return Err(
                "History promotion manifest does not match selected aggregate Land authority."
                    .to_string(),
            );
        }
        let aggregate_change = self
            .read_change(tx, aggregate_change_index)
            .map_err(|error| Self::error("history aggregate Change read", error))?;
        if manifest.aggregate.change_ref
            != self.change_ref(aggregate_change.task_index, aggregate_change.change_ordinal)
            || manifest.aggregate.patchset_id
                != self.patchset_id(
                    aggregate_change,
                    self.read_patchset(tx, aggregate_patchset_index)
                        .map_err(|error| Self::error("history aggregate Patchset read", error))?
                        .patch_ordinal,
                )
        {
            return Err(
                "History promotion aggregate identities do not match persisted records."
                    .to_string(),
            );
        }
        let base_snapshot_index = self.snapshot_index_in_history_write(
            tx,
            &manifest.base_snapshot_id,
            "history Land base",
        )?;
        if current_head_index != Some(base_snapshot_index) {
            return Err(format!(
                "STALE_HISTORY_PROMOTION_BASE: final Task Land expected {}, current index {:?}.",
                manifest.base_snapshot_id, current_head_index
            ));
        }

        if manifest.contract == "ait-history-promotion/v1" {
            self.apply_history_receipt_batch_in_write(
                tx,
                &manifest.entries,
                target_line_index,
                &manifest.base_snapshot_id,
                &manifest.revision_snapshot_id,
            )?;
            return Ok(Some(manifest_value));
        }

        let stages = self.staged_history_chain_for_final(tx, aggregate_patchset_index)?;
        let final_receipt = stages
            .last()
            .and_then(|stage| stage.entries.last())
            .ok_or_else(|| "Staged history promotion has no final receipt.".to_string())?;
        if final_receipt.change_index != aggregate_change_index
            || final_receipt.task_index != aggregate_change.task_index
            || final_receipt.landed_snapshot_id != manifest.revision_snapshot_id
        {
            return Err(
                "Staged history promotion final receipt does not own the aggregate Land."
                    .to_string(),
            );
        }
        let mut expected_pre_land_index = base_snapshot_index;
        for receipt in stages
            .iter()
            .flat_map(|stage| stage.entries.iter())
            .take_while(|receipt| receipt.change_index != final_receipt.change_index)
        {
            expected_pre_land_index = self.validate_landed_history_receipt_in_write(
                tx,
                receipt,
                target_line_index,
                expected_pre_land_index,
            )?;
        }
        let final_pre_land_index = self.snapshot_index_in_history_write(
            tx,
            &final_receipt.pre_land_target_snapshot_id,
            "final history receipt pre-Land",
        )?;
        if expected_pre_land_index != final_pre_land_index {
            return Err(
                "Staged history promotion receipt closeout has not reached the final receipt."
                    .to_string(),
            );
        }
        self.apply_history_receipt_batch_in_write(
            tx,
            std::slice::from_ref(final_receipt),
            target_line_index,
            &final_receipt.pre_land_target_snapshot_id,
            &final_receipt.landed_snapshot_id,
        )?;
        Ok(Some(manifest_value))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_staged_history_receipts_before_atomic_land(
        &self,
        change_id: &str,
        requested_ref: &str,
        target_line: &str,
        requested_patchset_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> Result<(), String> {
        let operation = "staged history promotion receipt closeout";
        let read = BinaryDbReadTxn::new(&self.db);
        let change_index = self.change_index_for_ref(&read, change_id)?;
        let change = self
            .read_change(&read, change_index)
            .map_err(|error| Self::error(operation, error))?;
        let resolved_ref = self.resolve_task_land_change_ref_in_read(&read, requested_ref)?;
        if resolved_ref != change_id {
            return Err(format!(
                "Atomic Task Land reference changed during staged closeout: expected {change_id}, resolved {resolved_ref}."
            ));
        }
        let selected_patchset_index = change
            .selected_patchset_index_plus1
            .checked_sub(1)
            .ok_or_else(|| format!("No selected Patchset found for Change {change_id}"))?;
        let selected_patchset = self
            .read_patchset(&read, selected_patchset_index)
            .map_err(|error| Self::error(operation, error))?;
        if selected_patchset.change_index != change_index {
            return Err(format!(
                "Selected Patchset does not belong to Change {change_id}."
            ));
        }
        let selected_patchset_id = self.patchset_id(change, selected_patchset.patch_ordinal);
        if requested_patchset_id.is_some_and(|requested| requested != selected_patchset_id) {
            return Err(format!(
                "Patchset {} is not the selected Patchset for Change {change_id}",
                requested_patchset_id.unwrap_or_default()
            ));
        }
        let Some(manifest_value) =
            self.history_manifest_for_patchset(&read, selected_patchset_index)?
        else {
            return Ok(());
        };
        let manifest: PersistedHistoryManifest = serde_json::from_value(manifest_value)
            .map_err(|error| format!("Persisted history promotion manifest is invalid: {error}"))?;
        if manifest.contract != "ait-history-promotion/v2" {
            return Ok(());
        }
        if change.lifecycle() == CHANGE_LIFECYCLE_LANDED {
            return Ok(());
        }
        if change.lifecycle() == CHANGE_LIFECYCLE_ARCHIVED {
            return Err(format!(
                "Change {change_id} is archived and cannot complete staged history promotion."
            ));
        }
        if change.change_meta & CHANGE_META_READY_TO_LAND == 0 {
            return Err(format!(
                "TASK_LAND_NOT_READY: Change {change_id} does not have an already-ready selected Patchset. Run `ait workflow ready {change_id} --apply` before Task Land."
            ));
        }
        if selected_patchset.patchset_meta & 0b11 != 0 {
            return Err(format!(
                "Selected Patchset for Change {change_id} is withdrawn or invalidated."
            ));
        }
        if manifest.target_line != target_line
            || manifest.aggregate.change_index != change_index
            || manifest.aggregate.patchset_index != selected_patchset_index
            || manifest.aggregate.change_ref != change_id
            || manifest.aggregate.patchset_id != selected_patchset_id
        {
            return Err(
                "Staged history promotion manifest does not match selected Land authority."
                    .to_string(),
            );
        }
        let stages = self.staged_history_chain_for_final(&read, selected_patchset_index)?;
        let final_receipt = stages
            .last()
            .and_then(|stage| stage.entries.last())
            .ok_or_else(|| "Staged history promotion has no final receipt.".to_string())?;
        if final_receipt.change_index != change_index
            || final_receipt.task_index != change.task_index
        {
            return Err(
                "Staged history promotion aggregate is not owned by its final receipt.".to_string(),
            );
        }
        let lines =
            ServerBinaryDbLineStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let snapshots =
            ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let (target_line_index, line) = lines
            .line_by_name(&read, target_line)
            .map_err(|error| Self::error(operation, error))?
            .ok_or_else(|| {
                format!("CANONICAL_LINE_MISSING: target line {target_line} does not exist")
            })?;
        let current_head = line
            .head_snapshot_index()
            .map(|index| {
                snapshots
                    .snapshot_id_at(&read, index)
                    .map_err(|error| Self::error(operation, error))
            })
            .transpose()?;
        if expected_head_snapshot_id.is_some()
            && expected_head_snapshot_id != current_head.as_deref()
        {
            return Err(format!(
                "STALE_TARGET_LINE: {}:{} expected {:?}, current {:?}",
                self.db.repo_name().as_str(),
                target_line,
                expected_head_snapshot_id,
                current_head.as_deref()
            ));
        }
        let patchset_base = snapshots
            .snapshot_id_at(&read, selected_patchset.base_snapshot_index)
            .map_err(|error| Self::error(operation, error))?;
        let patchset_revision = snapshots
            .snapshot_id_at(&read, selected_patchset.revision_snapshot_index)
            .map_err(|error| Self::error(operation, error))?;
        if current_head.as_deref() != Some(manifest.base_snapshot_id.as_str())
            || patchset_base != manifest.base_snapshot_id
            || patchset_revision != manifest.revision_snapshot_id
        {
            return Err(format!(
                "STALE_HISTORY_PROMOTION_BASE: final Task Land expected {}, current {:?}.",
                manifest.base_snapshot_id,
                current_head.as_deref()
            ));
        }

        let mut batches = Vec::with_capacity(stages.len());
        for stage in stages {
            let mut receipts = stage.entries;
            if receipts
                .last()
                .is_some_and(|receipt| receipt.change_index == change_index)
            {
                receipts.pop();
            }
            if receipts.is_empty() {
                continue;
            }
            if receipts
                .iter()
                .any(|receipt| receipt.change_index == change_index)
            {
                return Err(
                    "Staged history promotion repeats its aggregate owner before the final receipt."
                        .to_string(),
                );
            }
            let batch_base = receipts
                .first()
                .expect("non-empty staged receipt batch")
                .pre_land_target_snapshot_id
                .clone();
            let batch_revision = receipts
                .last()
                .expect("non-empty staged receipt batch")
                .landed_snapshot_id
                .clone();
            batches.push((batch_base, batch_revision, receipts));
        }
        drop(read);

        for (batch_base, batch_revision, receipts) in batches {
            let mut tx =
                BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerLand)
                    .map_err(|error| Self::error(operation, error))?;
            let current_change = self
                .read_change(&tx, change_index)
                .map_err(|error| Self::error(operation, error))?;
            if current_change.lifecycle() == CHANGE_LIFECYCLE_LANDED {
                return Ok(());
            }
            if current_change.lifecycle() == CHANGE_LIFECYCLE_ARCHIVED
                || current_change.selected_patchset_index_plus1 != selected_patchset_index + 1
                || current_change.change_meta & CHANGE_META_READY_TO_LAND == 0
            {
                return Err(
                    "Staged history promotion aggregate authority changed during receipt closeout."
                        .to_string(),
                );
            }
            let (_, current_line) = lines
                .line_by_name_in_write(&tx, target_line)
                .map_err(|error| Self::error(operation, error))?
                .ok_or_else(|| {
                    format!("CANONICAL_LINE_MISSING: target line {target_line} does not exist")
                })?;
            let current_line_head = current_line
                .head_snapshot_index()
                .map(|index| {
                    snapshots
                        .snapshot_id_at_in_write(&tx, index)
                        .map_err(|error| Self::error(operation, error))
                })
                .transpose()?;
            if current_line_head.as_deref() != Some(manifest.base_snapshot_id.as_str()) {
                return Err(format!(
                    "STALE_HISTORY_PROMOTION_BASE: final Task Land expected {}, current {:?}.",
                    manifest.base_snapshot_id,
                    current_line_head.as_deref()
                ));
            }
            self.apply_history_receipt_batch_in_write(
                &mut tx,
                &receipts,
                target_line_index,
                &batch_base,
                &batch_revision,
            )?;
            tx.commit().map_err(|error| Self::error(operation, error))?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn apply_staged_history_receipts_before_atomic_land_for_test(
        &self,
        change_id: &str,
        requested_ref: &str,
        target_line: &str,
        requested_patchset_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> Result<(), String> {
        self.apply_staged_history_receipts_before_atomic_land(
            change_id,
            requested_ref,
            target_line,
            requested_patchset_id,
            expected_head_snapshot_id,
        )
    }

    fn history_summary_bytes(&self, prefix: &str, payload: &JsonValue) -> Result<Vec<u8>, String> {
        let json = serde_json::to_string(payload)
            .map_err(|error| format!("History promotion summary encoding failed: {error}"))?;
        WorkflowBinaryV0Codec::encode_single_text_payload(
            &format!("{prefix}{json}"),
            "Patchset summary",
        )
        .map_err(|error| Self::error("history promotion summary", error))
    }

    fn read_patchset_summary_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        patchset_index: u32,
    ) -> Result<String, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let record = self
            .read_patchset(tx, patchset_index)
            .map_err(|error| Self::error("history promotion Patchset read", error))?;
        let raw = tx
            .read_payload(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                record.summary_offset,
                u32::from(record.summary_len),
            )
            .map_err(|error| Self::error("history promotion summary read", error))?;
        WorkflowBinaryV0Codec::decode_single_text_payload(&raw, "Patchset summary")
            .map_err(|error| Self::error("history promotion summary decode", error))
            .map(str::to_string)
    }

    pub(super) fn require_patchset_governance_authority<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        patchset_index: u32,
        patchset_id: &str,
        operation: &str,
    ) -> Result<(), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let summary = self.read_patchset_summary_in_write(tx, patchset_index)?;
        let (source_kind, governance_authority) = source_kind_for_summary(&summary);
        if governance_authority {
            Ok(())
        } else {
            Err(format!(
                "IMPORTED_LOCAL_LAND_RECEIPT: Patchset {patchset_id} is provenance-only ({source_kind}) and cannot perform {operation}; use the selected history-promotion aggregate Patchset."
            ))
        }
    }

    fn existing_history_manifest_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        idempotency_key: &str,
    ) -> Result<Option<JsonValue>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let count = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| Self::error("history promotion replay inventory", error))?;
        let mut matched = None;
        for patchset_index in 0..count {
            let summary = self.read_patchset_summary_in_write(tx, patchset_index)?;
            let Some(manifest) = parse_summary(&summary, HISTORY_PROMOTION_SUMMARY_PREFIX)? else {
                continue;
            };
            if manifest.get("idempotency_key").and_then(JsonValue::as_str) != Some(idempotency_key)
            {
                continue;
            }
            if matched.replace(manifest).is_some() {
                return Err(format!(
                    "History promotion idempotency key {idempotency_key:?} has multiple aggregate Patchsets."
                ));
            }
        }
        Ok(matched)
    }

    fn staged_manifest_from_summary(summary: &str) -> Result<Option<JsonValue>, String> {
        if let Some(manifest) = parse_summary(summary, HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX)? {
            return Ok(Some(manifest));
        }
        let Some(manifest) = parse_summary(summary, HISTORY_PROMOTION_SUMMARY_PREFIX)? else {
            return Ok(None);
        };
        Ok((manifest.get("contract").and_then(JsonValue::as_str)
            == Some("ait-history-promotion/v2"))
        .then_some(manifest))
    }

    fn existing_staged_manifest_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        idempotency_key: &str,
    ) -> Result<Option<JsonValue>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let count = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| Self::error("staged history replay inventory", error))?;
        let mut matched = None;
        for patchset_index in 0..count {
            let summary = self.read_patchset_summary_in_write(tx, patchset_index)?;
            let Some(manifest) = Self::staged_manifest_from_summary(&summary)? else {
                continue;
            };
            if manifest.get("idempotency_key").and_then(JsonValue::as_str) != Some(idempotency_key)
            {
                continue;
            }
            if matched.replace(manifest).is_some() {
                return Err(format!(
                    "History promotion stage idempotency key {idempotency_key:?} has multiple Patchsets."
                ));
            }
        }
        Ok(matched)
    }

    fn validate_staged_history_position_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        request: &StagedHistoryPromotionRequest,
    ) -> Result<(), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let patchset_count = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| Self::error("staged history position inventory", error))?;
        for patchset_index in 0..patchset_count {
            let summary = self.read_patchset_summary_in_write(tx, patchset_index)?;
            let Some(manifest) = Self::staged_manifest_from_summary(&summary)? else {
                continue;
            };
            if manifest.get("promotion_id").and_then(JsonValue::as_str)
                == Some(request.promotion_id.as_str())
                && manifest.get("stage_ordinal").and_then(JsonValue::as_u64)
                    == Some(request.stage_ordinal)
            {
                return Err(format!(
                    "HISTORY_PROMOTION_STAGE_CONFLICT: promotion {} already owns stage {} under another idempotency request.",
                    request.promotion_id, request.stage_ordinal
                ));
            }
        }

        if request.stage_ordinal == 0 {
            if request.previous_stage_patchset_id.is_some()
                || request.stage_base_snapshot_id != request.base_snapshot_id
            {
                return Err(
                    "History promotion stage zero must start at the global base without a predecessor."
                        .to_string(),
                );
            }
            return Ok(());
        }

        let previous_patchset_id =
            request
                .previous_stage_patchset_id
                .as_deref()
                .ok_or_else(|| {
                    "History promotion continuation requires a previous stage.".to_string()
                })?;
        let previous_index = self.patchset_index_for_id(tx, previous_patchset_id)?;
        let previous_summary = self.read_patchset_summary_in_write(tx, previous_index)?;
        let previous_value =
            parse_summary(&previous_summary, HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX)?.ok_or_else(
                || {
                    format!(
                "History promotion predecessor {previous_patchset_id} is not a non-final stage."
            )
                },
            )?;
        let previous: PersistedHistoryManifest = serde_json::from_value(previous_value)
            .map_err(|error| format!("Persisted history promotion stage is invalid: {error}"))?;
        let expected_previous_ordinal = request.stage_ordinal - 1;
        if previous.contract != "ait-history-promotion-stage/v1"
            || previous.promotion_id.as_deref() != Some(request.promotion_id.as_str())
            || previous.stage_ordinal != Some(expected_previous_ordinal)
            || previous.target_line != request.target_line
            || previous.base_snapshot_id != request.base_snapshot_id
            || previous.revision_snapshot_id != request.revision_snapshot_id
            || previous.total_entry_count != Some(request.total_entry_count)
            || previous.final_stage != Some(false)
            || previous.stage_revision_snapshot_id.as_deref()
                != Some(request.stage_base_snapshot_id.as_str())
            || previous.aggregate.patchset_index != previous_index
            || previous.aggregate.patchset_id != previous_patchset_id
        {
            return Err(format!(
                "HISTORY_PROMOTION_STAGE_CONFLICT: predecessor {previous_patchset_id} does not continue promotion {} stage {}.",
                request.promotion_id, request.stage_ordinal
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn reusable_history_receipt_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        entry: &HistoryPromotionEntry,
        plan_binding: HistoryPromotionPlanBinding,
        target_line: &str,
        target_line_index: u32,
        fork_snapshot_index: u32,
        pre_land_index: u32,
        landed_index: u32,
    ) -> Result<Option<ReusableHistoryReceipt>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let operation = "history promotion reusable receipt";
        let patchset_count = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| Self::error(operation, error))?;
        let mut matched = None;

        for patchset_index in 0..patchset_count {
            let summary = self.read_patchset_summary_in_write(tx, patchset_index)?;
            let Some(receipt_value) = parse_summary(&summary, LOCAL_LAND_RECEIPT_SUMMARY_PREFIX)?
            else {
                continue;
            };
            let receipt: PersistedLocalLandReceipt = serde_json::from_value(receipt_value)
                .map_err(|error| format!("Persisted local Land receipt is invalid: {error}"))?;
            let same_task = receipt.local_task_id == entry.local_task_id;
            let same_change = receipt.local_change_ref == entry.local_change_ref;
            if !same_task && !same_change {
                continue;
            }
            if !same_task
                || !same_change
                || receipt.local_change_id != entry.local_change_id
                || receipt.contract != "ait-local-land-receipt/v1"
                || receipt.source_kind != "imported_local_land_receipt"
            {
                return Err(format!(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: local identity {} collides with an existing receipt.",
                    entry.local_change_ref
                ));
            }
            if receipt.target_line != target_line
                || receipt.pre_land_target_snapshot_id != entry.pre_land_target_snapshot_id
                || receipt.landed_snapshot_id != entry.landed_snapshot_id
                || receipt.landed_at_s != entry.landed_at_s
            {
                return Err(format!(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: existing receipt for {} has different Land authority.",
                    entry.local_change_ref
                ));
            }

            let patchset = self
                .read_patchset(tx, patchset_index)
                .map_err(|error| Self::error(operation, error))?;
            if patchset.base_snapshot_index != pre_land_index
                || patchset.revision_snapshot_index != landed_index
                || patchset.created_at_s != entry.landed_at_s
                || patchset.ci_run_seq != 0
                || patchset.ci_selected_suite_count != 0
                || patchset.ci_suite_result_count != 0
            {
                return Err(format!(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: existing receipt for {} has divergent Patchset authority.",
                    entry.local_change_ref
                ));
            }
            let change = self
                .read_change(tx, patchset.change_index)
                .map_err(|error| Self::error(operation, error))?;
            let task = self
                .read_task(tx, change.task_index)
                .map_err(|error| Self::error(operation, error))?;
            let task_id = self.task_id(change.task_index);
            let change_ref = self.change_ref(change.task_index, change.change_ordinal);
            if receipt.task_id != task_id
                || receipt.change_ref != change_ref
                || patchset.change_ordinal != change.change_ordinal
                || task.remote_meta & 1 != 0
                || change.remote_meta & 1 != 0
                || task.is_terminal()
                || change.change_state & CHANGE_STATE_CANCELED != 0
                || matches!(
                    change.lifecycle(),
                    CHANGE_LIFECYCLE_LANDED | CHANGE_LIFECYCLE_ARCHIVED
                )
            {
                return Err(format!(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: existing receipt for {} no longer has reusable Task/Change ownership.",
                    entry.local_change_ref
                ));
            }
            validate_expected_publication_owner(entry, &task_id, &change_ref)?;
            let (revision_plus1, item_plus1, _) = plan_binding;
            if task.origin_plan_revision_index_plus1 != revision_plus1
                || task.plan_item_index_plus1 != item_plus1
                || (revision_plus1 != 0
                    && self.task_for_plan_binding_in_write(tx, revision_plus1, item_plus1)?
                        != Some(change.task_index))
            {
                return Err(format!(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: existing receipt for {} has a different Plan binding.",
                    entry.local_change_ref
                ));
            }

            let task_payload_raw = tx
                .read_payload(
                    WorkflowBinaryV0Codec::task_payload_file(),
                    task.payload_offset,
                    u32::from(task.payload_len),
                )
                .map_err(|error| Self::error(operation, error))?;
            let task_payload = WorkflowBinaryV0Codec::decode_task_payload(&task_payload_raw)
                .map_err(|error| Self::error(operation, error))?;
            let change_payload_raw = tx
                .read_payload(
                    WorkflowBinaryV0Codec::change_payload_file(),
                    change.payload_offset,
                    u32::from(change.payload_len),
                )
                .map_err(|error| Self::error(operation, error))?;
            let change_title = WorkflowBinaryV0Codec::decode_single_text_payload(
                &change_payload_raw,
                "Change title",
            )
            .map_err(|error| Self::error(operation, error))?;
            if task_payload.title != entry.task.title.trim()
                || task_payload.intent != entry.task.intent.trim()
                || change_title != entry.change.title.trim()
                || change.base_line_index_plus1 != target_line_index + 1
                || change.fork_snapshot_index_plus1 != fork_snapshot_index + 1
                || self
                    .latest_succeeded_land(tx, patchset.change_index)?
                    .is_some()
            {
                return Err(format!(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: existing receipt for {} has divergent Task, Change, or Land data.",
                    entry.local_change_ref
                ));
            }

            let reusable = ReusableHistoryReceipt {
                task_index: change.task_index,
                task_id,
                change_index: patchset.change_index,
                change_ref,
                patchset_index,
                patchset_id: self.patchset_id(change, patchset.patch_ordinal),
            };
            if matched.replace(reusable).is_some() {
                return Err(format!(
                    "History promotion local Change {} has multiple reusable receipts.",
                    entry.local_change_ref
                ));
            }
        }
        Ok(matched)
    }

    fn history_promotion_result(
        &self,
        manifest: &JsonValue,
        replayed: bool,
    ) -> Result<JsonValue, String> {
        let aggregate = manifest
            .get("aggregate")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| {
                "History promotion manifest is missing aggregate authority.".to_string()
            })?;
        let patchset_index = aggregate
            .get("patchset_index")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                "History promotion manifest has an invalid aggregate Patchset index.".to_string()
            })?;
        let read = BinaryDbReadTxn::new(&self.db);
        let patchset = self.patchset_at(&read, patchset_index)?;
        Ok(json!({
            "contract": HISTORY_PROMOTION_CONTRACT,
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "idempotency_key": manifest.get("idempotency_key").cloned().unwrap_or(JsonValue::Null),
            "replayed": replayed,
            "target_line": manifest.get("target_line").cloned().unwrap_or(JsonValue::Null),
            "base_snapshot_id": manifest.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
            "revision_snapshot_id": manifest.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
            "entries": manifest.get("entries").cloned().unwrap_or_else(|| json!([])),
            "aggregate": {
                "task_id": aggregate.get("task_id").cloned().unwrap_or(JsonValue::Null),
                "change_ref": aggregate.get("change_ref").cloned().unwrap_or(JsonValue::Null),
                "patchset_id": aggregate.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                "patchset": patchset,
            },
        }))
    }

    fn staged_history_promotion_result(
        &self,
        manifest: &JsonValue,
        replayed: bool,
    ) -> Result<JsonValue, String> {
        let aggregate = manifest
            .get("aggregate")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| "History promotion stage is missing Patchset authority.".to_string())?;
        let patchset_index = aggregate
            .get("patchset_index")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "History promotion stage has an invalid Patchset index.".to_string())?;
        let read = BinaryDbReadTxn::new(&self.db);
        let patchset = self.patchset_at(&read, patchset_index)?;
        let stage = json!({
            "task_id": aggregate.get("task_id").cloned().unwrap_or(JsonValue::Null),
            "change_ref": aggregate.get("change_ref").cloned().unwrap_or(JsonValue::Null),
            "patchset_id": aggregate.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
            "patchset": patchset,
        });
        let final_stage = manifest
            .get("final_stage")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        Ok(json!({
            "contract": HISTORY_PROMOTION_STAGED_CONTRACT,
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "promotion_id": manifest.get("promotion_id").cloned().unwrap_or(JsonValue::Null),
            "idempotency_key": manifest.get("idempotency_key").cloned().unwrap_or(JsonValue::Null),
            "replayed": replayed,
            "target_line": manifest.get("target_line").cloned().unwrap_or(JsonValue::Null),
            "base_snapshot_id": manifest.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
            "revision_snapshot_id": manifest.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
            "stage_ordinal": manifest.get("stage_ordinal").cloned().unwrap_or(JsonValue::Null),
            "stage_base_snapshot_id": manifest.get("stage_base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
            "stage_revision_snapshot_id": manifest.get("stage_revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
            "previous_stage_patchset_id": manifest.get("previous_stage_patchset_id").cloned().unwrap_or(JsonValue::Null),
            "total_entry_count": manifest.get("total_entry_count").cloned().unwrap_or(JsonValue::Null),
            "final_stage": final_stage,
            "entries": manifest.get("entries").cloned().unwrap_or_else(|| json!([])),
            "stage": stage,
            "aggregate": if final_stage { stage } else { JsonValue::Null },
        }))
    }

    fn snapshot_index_in_history_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        snapshot_id: &str,
        label: &str,
    ) -> Result<u32, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone())
            .snapshot_by_id_in_write(tx, snapshot_id)
            .map_err(|error| Self::error(label, error))?
            .map(|(index, _)| index)
            .ok_or_else(|| format!("Unknown {label} Snapshot {snapshot_id}."))
    }

    fn load_history_snapshot_dag<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
    ) -> Result<HistorySnapshotDag, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let snapshot_count = tx
            .record_count(
                ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            )
            .map_err(|error| Self::error("history Snapshot inventory", error))?;
        let edge_file =
            ServerBinarySnapshotParentEdgeCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file();
        let edge_count = tx
            .record_count(edge_file.clone())
            .map_err(|error| Self::error("history Snapshot parent-edge inventory", error))?;
        let mut parent_edges_by_child = BTreeMap::<u32, Vec<(u16, u32)>>::new();
        for edge_index in 0..edge_count {
            let edge = ServerBinarySnapshotParentEdgeCodec::<
                SERVER_CONTENT_BINARY_LAYOUT_ID,
            >::decode_record(
                &tx.read_record(edge_file.clone(), edge_index)
                    .map_err(|error| Self::error("history Snapshot parent-edge read", error))?,
            )
            .map_err(|error| Self::error("history Snapshot parent-edge decode", error))?;
            if edge.child_snapshot_index >= snapshot_count
                || edge.parent_snapshot_index >= snapshot_count
            {
                return Err(format!(
                    "History Snapshot parent edge {edge_index} references an out-of-range Snapshot."
                ));
            }
            parent_edges_by_child
                .entry(edge.child_snapshot_index)
                .or_default()
                .push((edge.parent_ordinal, edge.parent_snapshot_index));
        }
        for (child, rows) in &mut parent_edges_by_child {
            rows.sort_by_key(|(ordinal, _)| *ordinal);
            for (expected, (ordinal, _)) in rows.iter().enumerate() {
                if usize::from(*ordinal) != expected {
                    return Err(format!(
                        "History Snapshot {child} parent ordinals are not contiguous at {expected}."
                    ));
                }
            }
        }
        let snapshot_file =
            ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file();
        let mut snapshot_records = Vec::with_capacity(snapshot_count as usize);
        for snapshot_index in 0..snapshot_count {
            snapshot_records.push(
                ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(
                    &tx.read_record(snapshot_file.clone(), snapshot_index)
                        .map_err(|error| Self::error("history Snapshot read", error))?,
                )
                .map_err(|error| Self::error("history Snapshot decode", error))?,
            );
        }
        Ok(HistorySnapshotDag {
            snapshot_count,
            snapshot_records,
            parent_edges_by_child,
        })
    }

    fn history_snapshot_parents<F>(
        &self,
        _tx: &BinaryDbWriteTxn<'_, D, F>,
        dag: &HistorySnapshotDag,
        snapshot_index: u32,
    ) -> Result<Vec<u32>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        if snapshot_index >= dag.snapshot_count {
            return Err(format!(
                "History Snapshot index {snapshot_index} is outside the canonical inventory."
            ));
        }
        let record = dag
            .snapshot_records
            .get(snapshot_index as usize)
            .ok_or_else(|| {
                format!(
                    "History Snapshot index {snapshot_index} is missing from the loaded inventory."
                )
            })?;
        if record.is_tombstone() {
            return Err(format!(
                "History Snapshot index {snapshot_index} is tombstoned."
            ));
        }
        let parents = if record.has_parent_edges_authority() {
            dag.parent_edges_by_child
                .get(&snapshot_index)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|(_, parent)| parent)
                .collect::<Vec<_>>()
        } else {
            if dag.parent_edges_by_child.contains_key(&snapshot_index) {
                return Err(format!(
                    "History Snapshot {snapshot_index} has parent edges without parent-edge authority."
                ));
            }
            record
                .parent_snapshot_index_plus1
                .checked_sub(1)
                .into_iter()
                .collect::<Vec<_>>()
        };
        if parents.first().copied() != record.parent_snapshot_index_plus1.checked_sub(1) {
            return Err(format!(
                "History Snapshot {snapshot_index} first-parent cache disagrees with ordered edges."
            ));
        }
        if record.is_remote_head_history_boundary() && !parents.is_empty() {
            return Err(format!(
                "History boundary Snapshot {snapshot_index} unexpectedly has local parents."
            ));
        }
        if parents.iter().any(|parent| *parent >= dag.snapshot_count) {
            return Err(format!(
                "History Snapshot {snapshot_index} references an out-of-range parent."
            ));
        }
        Ok(parents)
    }

    fn history_snapshot_ancestor_closure<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        dag: &HistorySnapshotDag,
        snapshot_index: u32,
    ) -> Result<BTreeSet<u32>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let mut closure = BTreeSet::new();
        let mut pending = vec![snapshot_index];
        while let Some(cursor) = pending.pop() {
            if !closure.insert(cursor) {
                continue;
            }
            pending.extend(self.history_snapshot_parents(tx, dag, cursor)?);
        }
        Ok(closure)
    }

    fn validate_history_snapshot_range<'a, F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        dag: &HistorySnapshotDag,
        entry: &'a HistoryPromotionEntry,
        pre_land_index: u32,
        landed_index: u32,
        globally_linked: &mut BTreeSet<String>,
    ) -> Result<Vec<(u32, &'a HistoryPromotionSnapshot)>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        if entry.snapshots.is_empty() {
            return Err(format!(
                "History promotion entry {} has no Snapshot links.",
                entry.local_change_ref
            ));
        }
        if entry.snapshots.len() > MAX_HISTORY_SNAPSHOTS_PER_ENTRY {
            return Err(format!(
                "History promotion entry {} has {} Snapshots; the current bounded writer limit is {MAX_HISTORY_SNAPSHOTS_PER_ENTRY}.",
                entry.local_change_ref,
                entry.snapshots.len()
            ));
        }
        if entry.snapshots.last().map(|row| row.snapshot_id.as_str())
            != Some(entry.landed_snapshot_id.as_str())
        {
            return Err(format!(
                "History promotion entry {} Snapshot order must end at landed Snapshot {}.",
                entry.local_change_ref, entry.landed_snapshot_id
            ));
        }
        let mut resolved = Vec::with_capacity(entry.snapshots.len());
        let mut resolved_indexes = BTreeSet::new();
        for snapshot in &entry.snapshots {
            nonempty(&snapshot.snapshot_id, "history Snapshot id")?;
            if snapshot.snapshot_id == entry.pre_land_target_snapshot_id {
                return Err(format!(
                    "History promotion entry {} must not repeat its pre-Land boundary as a Snapshot link.",
                    entry.local_change_ref
                ));
            }
            if !globally_linked.insert(snapshot.snapshot_id.clone()) {
                return Err(format!(
                    "History promotion Snapshot {} is linked by more than one local Land entry.",
                    snapshot.snapshot_id
                ));
            }
            let snapshot_index =
                self.snapshot_index_in_history_write(tx, &snapshot.snapshot_id, "history linked")?;
            if !resolved_indexes.insert(snapshot_index) {
                return Err(format!(
                    "History promotion entry {} repeats canonical Snapshot index {snapshot_index}.",
                    entry.local_change_ref
                ));
            }
            resolved.push((snapshot_index, snapshot));
        }
        let landed_closure = self.history_snapshot_ancestor_closure(tx, dag, landed_index)?;
        if !landed_closure.contains(&pre_land_index) {
            return Err(format!(
                "History promotion landed Snapshot {} does not descend from pre-Land boundary {} for {}.",
                entry.landed_snapshot_id,
                entry.pre_land_target_snapshot_id,
                entry.local_change_ref
            ));
        }
        let pre_land_closure = self.history_snapshot_ancestor_closure(tx, dag, pre_land_index)?;
        let expected_indexes = landed_closure
            .difference(&pre_land_closure)
            .copied()
            .collect::<BTreeSet<_>>();
        if resolved_indexes != expected_indexes {
            return Err(format!(
                "History promotion entry {} must contain the complete Snapshot DAG difference for {} -> {}; expected {} canonical Snapshots, got {}.",
                entry.local_change_ref,
                entry.pre_land_target_snapshot_id,
                entry.landed_snapshot_id,
                expected_indexes.len(),
                resolved_indexes.len()
            ));
        }
        let mut admitted = pre_land_closure;
        for (snapshot_index, snapshot) in &resolved {
            for parent in self.history_snapshot_parents(tx, dag, *snapshot_index)? {
                if expected_indexes.contains(&parent) && !admitted.contains(&parent) {
                    return Err(format!(
                        "History promotion Snapshot {} is not in parent-before-child topological order for {}.",
                        snapshot.snapshot_id, entry.local_change_ref
                    ));
                }
            }
            admitted.insert(*snapshot_index);
        }
        Ok(resolved)
    }

    // The explicit persisted identities keep cross-file ownership auditable at
    // this one bounded write boundary.
    #[allow(clippy::too_many_arguments)]
    fn append_history_snapshot_links_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        task_index: u32,
        task_id: &str,
        change_index: u32,
        change_ref: &str,
        line_name: &str,
        author_mode: &str,
        snapshots: &[(u32, &HistoryPromotionSnapshot)],
    ) -> Result<(), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let operation = "history promotion Snapshot Link append";
        let mut task_owner = WorkflowBinaryV0Codec::decode_ordinal_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
                task_index,
            )
            .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        let mut change_owner = WorkflowBinaryV0Codec::decode_inventory_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("change_snapshot_index.bin"),
                change_index,
            )
            .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if usize::from(task_owner.next_ordinal) + snapshots.len() > MAX_HISTORY_SNAPSHOTS_PER_ENTRY
        {
            return Err(format!(
                "Task {task_id} exceeds the bounded history Snapshot Link limit."
            ));
        }

        let change_id = change_ref
            .rsplit_once('/')
            .map(|(_, child)| child)
            .unwrap_or(change_ref);
        let mut payloads = Vec::with_capacity(snapshots.len());
        for _ in snapshots {
            let payload =
                WorkflowBinaryV0Codec::encode_snapshot_link_payload(&V0SnapshotLinkPayload {
                    worktree_name: String::new(),
                    line_name: line_name.to_string(),
                    task_id: task_id.to_string(),
                    change_id: change_id.to_string(),
                    author_mode: author_mode.to_string(),
                    model_name: String::new(),
                })
                .map_err(|error| Self::error(operation, error))?;
            let range = tx
                .append_payload(
                    WorkflowBinaryV0Codec::snapshot_link_payload_file(),
                    &payload,
                )
                .map_err(|error| Self::error(operation, error))?;
            payloads.push(range);
        }
        // This helper is only used by the bounded history-promotion
        // transaction. Its commit fsyncs every touched payload and record file
        // before publishing the durable journal commit, so one commit boundary
        // safely replaces a per-entry payload fsync.

        for ((snapshot_index, snapshot), range) in snapshots.iter().zip(payloads) {
            let record = V0SnapshotLinkRecord {
                link_meta: SNAPSHOT_LINK_HAS_CHANGE
                    | SNAPSHOT_LINK_HAS_LINE_NAME
                    | SNAPSHOT_LINK_HAS_AUTHOR_OR_MODEL,
                snapshot_ordinal: task_owner.next_ordinal,
                payload_len: u16::try_from(range.payload_len)
                    .map_err(|_| "Snapshot Link payload exceeds u16".to_string())?,
                payload_offset: range.payload_offset,
                task_index,
                change_index_plus1: change_index + 1,
                content_snapshot_index: *snapshot_index,
                previous_task_snapshot_link_index_plus1: task_owner.latest_index_plus1,
                previous_change_snapshot_link_index_plus1: change_owner.latest_index_plus1,
                created_at_s: snapshot.created_at_s,
            };
            let link_index = tx
                .append_record(
                    WorkflowBinaryV0Codec::snapshot_link_file(),
                    &WorkflowBinaryV0Codec::encode_snapshot_link(record)
                        .map_err(|error| Self::error(operation, error))?,
                )
                .map_err(|error| Self::error(operation, error))?;
            task_owner.latest_index_plus1 = link_index + 1;
            task_owner.count = task_owner
                .count
                .checked_add(1)
                .ok_or_else(|| "Task Snapshot Link count exceeds u16".to_string())?;
            task_owner.next_ordinal = task_owner
                .next_ordinal
                .checked_add(1)
                .ok_or_else(|| "Task Snapshot Link ordinal overflow".to_string())?;
            change_owner.latest_index_plus1 = link_index + 1;
            change_owner.count = change_owner
                .count
                .checked_add(1)
                .ok_or_else(|| "Change Snapshot Link count exceeds u16".to_string())?;
        }
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
            task_index,
            &WorkflowBinaryV0Codec::encode_ordinal_index(task_owner)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("change_snapshot_index.bin"),
            change_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(change_owner)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        Ok(())
    }

    // Patchset authority fields stay explicit here because receipts and the
    // aggregate intentionally share this encoder with different semantics.
    #[allow(clippy::too_many_arguments)]
    fn append_history_patchset_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        change_index: u32,
        base_snapshot_index: u32,
        revision_snapshot_index: u32,
        summary_prefix: &str,
        summary_payload: &JsonValue,
        author_mode: &str,
        created_at_s: u64,
        force_select: bool,
    ) -> Result<(u32, String), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let operation = "history promotion Patchset append";
        if base_snapshot_index == revision_snapshot_index {
            return Err("History promotion Patchsets must not be empty.".to_string());
        }
        let mut change = self
            .read_change(tx, change_index)
            .map_err(|error| Self::error(operation, error))?;
        let mut owner = self.patchset_owner_index_in_write(tx, change_index)?;
        if owner.next_ordinal >= 64 {
            return Err(format!(
                "Change {} has exhausted its v0 Patchset ordinals.",
                self.change_ref(change.task_index, change.change_ordinal)
            ));
        }
        let mut task_inventory =
            self.task_inventory_in_write(tx, "task_patchset_index.bin", change.task_index)?;
        let summary = self.history_summary_bytes(summary_prefix, summary_payload)?;
        let range = tx
            .append_payload(WorkflowBinaryV0Codec::patchset_summary_file(), &summary)
            .map_err(|error| Self::error(operation, error))?;
        // History promotion batches receipt Patchsets in one rollback journal.
        // BinaryDbWriteTxn::commit fsyncs this payload together with every
        // referencing record before the journal commit becomes durable.
        let patch_ordinal = owner.next_ordinal;
        let record = V0PatchsetRecord {
            patchset_meta: Self::author_mode_bits(author_mode)? | PATCHSET_EVALUATION_PENDING,
            patch_ordinal,
            change_ordinal: change.change_ordinal,
            reserved0: 0,
            change_index,
            previous_task_patchset_index_plus1: task_inventory.latest_index_plus1,
            previous_change_patchset_index_plus1: owner.latest_index_plus1,
            base_snapshot_index,
            revision_snapshot_index,
            created_at_s,
            ci_completed_at_s: 0,
            ci_run_seq: 0,
            ci_selected_suite_count: 0,
            ci_suite_result_count: 0,
            ci_blocking_failure_count: 0,
            ci_status_bits: 0,
            summary_offset: range.payload_offset,
            summary_len: u16::try_from(range.payload_len)
                .map_err(|_| "Patchset summary exceeds u16".to_string())?,
            ci_worker_job_index_plus1: 0,
        };
        let patchset_index = tx
            .append_record(
                WorkflowBinaryV0Codec::patchset_file(),
                &self
                    .encode_new_patchset(record)
                    .map_err(|error| Self::error(operation, error))?,
            )
            .map_err(|error| Self::error(operation, error))?;
        self.append_patchset_index_rows(tx, patchset_index)
            .map_err(|error| Self::error(operation, error))?;
        owner.latest_index_plus1 = patchset_index + 1;
        owner.count = owner
            .count
            .checked_add(1)
            .ok_or_else(|| "Change Patchset count exceeds u16".to_string())?;
        owner.next_ordinal = patch_ordinal + 1;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("change_patchset_index.bin"),
            change_index,
            &WorkflowBinaryV0Codec::encode_ordinal_index(owner)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        task_inventory.latest_index_plus1 = patchset_index + 1;
        task_inventory.count = task_inventory
            .count
            .checked_add(1)
            .ok_or_else(|| "Task Patchset count exceeds u16".to_string())?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_patchset_index.bin"),
            change.task_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(task_inventory)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        change.change_meta = (change.change_meta & !CHANGE_META_LIFECYCLE_MASK)
            | CHANGE_LIFECYCLE_ACTIVE
            | CHANGE_META_HAS_PATCHSETS
            | CHANGE_META_REVIEW_PENDING
            | CHANGE_META_VALIDATION_PENDING;
        if change.selected_patchset_index_plus1 == 0 || force_select {
            change.selected_patchset_index_plus1 = patchset_index + 1;
        }
        change.updated_at_s = created_at_s;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::change_file(),
            change_index,
            &WorkflowBinaryV0Codec::encode_change(change)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        Ok((patchset_index, self.patchset_id(change, patch_ordinal)))
    }

    fn prepare_history_promotion_in_write<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        request: &HistoryPromotionRequest,
        plan_bindings: &[HistoryPromotionPlanBinding],
        fingerprint: &str,
        now: u64,
        context: HistoryPromotionWriteContext<'_>,
    ) -> Result<JsonValue, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        if plan_bindings.len() != request.entries.len() {
            return Err(
                "History promotion Plan-binding inventory does not match its entries.".to_string(),
            );
        }
        let line_store =
            ServerBinaryDbLineStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let (target_line_index, line) = line_store
            .line_by_name_in_write(tx, &request.target_line)
            .map_err(|error| Self::error("history promotion target Line", error))?
            .ok_or_else(|| {
                format!(
                    "Unknown history promotion target Line {}.",
                    request.target_line
                )
            })?;
        let line_head_snapshot_index = self.snapshot_index_in_history_write(
            tx,
            context.line_head_snapshot_id,
            "history promotion target-Line base",
        )?;
        if line.head_snapshot_index() != Some(line_head_snapshot_index) {
            let actual = line
                .head_snapshot_index()
                .map(|index| {
                    ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(
                        self.db.clone(),
                    )
                    .snapshot_id_at_in_write(tx, index)
                    .map_err(|error| Self::error("history promotion target Line head", error))
                })
                .transpose()?;
            return Err(format!(
                "STALE_HISTORY_PROMOTION_BASE: target Line {} expected {}, current {:?}.",
                request.target_line, context.line_head_snapshot_id, actual
            ));
        }
        let stage_final_snapshot_index = self.snapshot_index_in_history_write(
            tx,
            &request.revision_snapshot_id,
            "history promotion revision",
        )?;
        let history_dag = self.load_history_snapshot_dag(tx)?;
        if !self
            .history_snapshot_ancestor_closure(tx, &history_dag, stage_final_snapshot_index)?
            .contains(&line_head_snapshot_index)
        {
            return Err(format!(
                "History promotion revision {} does not descend from remote base {}.",
                request.revision_snapshot_id, context.line_head_snapshot_id
            ));
        }

        let mut expected_pre_land = request.base_snapshot_id.clone();
        let mut local_task_ids = BTreeSet::new();
        let mut local_change_refs = BTreeSet::new();
        let mut globally_linked = BTreeSet::new();
        let mut manifest_entries = Vec::with_capacity(request.entries.len());
        let mut final_change = None;
        let mut previous_landed_at_s = 0;

        for (entry, &(revision_plus1, item_plus1, linked_at_s)) in
            request.entries.iter().zip(plan_bindings)
        {
            nonempty(&entry.local_task_id, "local_task_id")?;
            nonempty(&entry.local_change_id, "local_change_id")?;
            nonempty(&entry.local_change_ref, "local_change_ref")?;
            let expected_local_change_ref =
                format!("{}/{}", entry.local_task_id, entry.local_change_id);
            if entry.local_change_ref != expected_local_change_ref {
                return Err(format!(
                    "History promotion local Change reference {} does not match exact source identity {}.",
                    entry.local_change_ref, expected_local_change_ref
                ));
            }
            if entry.landed_at_s == 0 || entry.landed_at_s < previous_landed_at_s {
                return Err(format!(
                    "History promotion local Land {} has a missing or regressing event time.",
                    entry.local_change_ref
                ));
            }
            if entry
                .snapshots
                .iter()
                .any(|snapshot| snapshot.created_at_s == 0)
            {
                return Err(format!(
                    "History promotion local Land {} has a Snapshot with a missing creation time.",
                    entry.local_change_ref
                ));
            }
            if !local_task_ids.insert(entry.local_task_id.clone()) {
                return Err(format!(
                    "History promotion repeats local Task {}.",
                    entry.local_task_id
                ));
            }
            if !local_change_refs.insert(entry.local_change_ref.clone()) {
                return Err(format!(
                    "History promotion repeats local Change {}.",
                    entry.local_change_ref
                ));
            }
            if entry.pre_land_target_snapshot_id != expected_pre_land {
                return Err(format!(
                    "History promotion chain gap before {}: expected {}, got {}.",
                    entry.local_change_ref, expected_pre_land, entry.pre_land_target_snapshot_id
                ));
            }
            if entry.landed_snapshot_id == entry.pre_land_target_snapshot_id {
                return Err(format!(
                    "History promotion local Land {} has an empty Snapshot boundary.",
                    entry.local_change_ref
                ));
            }
            if entry.change.base_line != request.target_line {
                return Err(format!(
                    "History promotion Change {} targets {}, not {}.",
                    entry.local_change_ref, entry.change.base_line, request.target_line
                ));
            }
            let fork_snapshot_index = self.snapshot_index_in_history_write(
                tx,
                &entry.change.fork_snapshot_id,
                "history Change fork",
            )?;
            let pre_land_index = self.snapshot_index_in_history_write(
                tx,
                &entry.pre_land_target_snapshot_id,
                "history pre-Land",
            )?;
            let landed_index = self.snapshot_index_in_history_write(
                tx,
                &entry.landed_snapshot_id,
                "history landed",
            )?;
            if !self
                .history_snapshot_ancestor_closure(tx, &history_dag, landed_index)?
                .contains(&fork_snapshot_index)
            {
                return Err(format!(
                    "History promotion Change {} landed Snapshot {} does not descend from its historical fork {}.",
                    entry.local_change_ref,
                    entry.landed_snapshot_id,
                    entry.change.fork_snapshot_id
                ));
            }
            let resolved_snapshots = self.validate_history_snapshot_range(
                tx,
                &history_dag,
                entry,
                pre_land_index,
                landed_index,
                &mut globally_linked,
            )?;

            let existing = self.reusable_history_receipt_in_write(
                tx,
                entry,
                (revision_plus1, item_plus1, linked_at_s),
                &request.target_line,
                target_line_index,
                fork_snapshot_index,
                pre_land_index,
                landed_index,
            )?;
            if existing.is_none() && entry.expected_remote_task_id.is_some() {
                return Err(format!(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: expected publication owner {} for {} has no exact reusable receipt.",
                    entry.expected_remote_task_id.as_deref().unwrap_or_default(),
                    entry.local_change_ref
                ));
            }
            let (
                task_index,
                task_id,
                change_index,
                change_ref,
                receipt_patchset_index,
                receipt_patchset_id,
            ) = if let Some(existing) = existing {
                (
                    existing.task_index,
                    existing.task_id,
                    existing.change_index,
                    existing.change_ref,
                    existing.patchset_index,
                    existing.patchset_id,
                )
            } else {
                let task_value = serde_json::to_value(&entry.task)
                    .map_err(|error| format!("History Task payload encoding failed: {error}"))?;
                let task_payload = task_value
                    .as_object()
                    .ok_or_else(|| "History Task payload must be an object.".to_string())?;
                if revision_plus1 != 0
                    && self
                        .task_for_plan_binding_in_write(tx, revision_plus1, item_plus1)?
                        .is_some()
                {
                    return Err(format!(
                        "History promotion Plan binding for local Task {} already owns a server Task without an exact reusable receipt.",
                        entry.local_task_id
                    ));
                }
                let (task_index, task_id) = self.append_task_in_write(
                    tx,
                    task_payload,
                    revision_plus1,
                    item_plus1,
                    linked_at_s,
                    now,
                    PayloadSyncBoundary::TransactionCommit,
                )?;
                let change_value = serde_json::to_value(&entry.change)
                    .map_err(|error| format!("History Change payload encoding failed: {error}"))?;
                let change_payload = change_value
                    .as_object()
                    .ok_or_else(|| "History Change payload must be an object.".to_string())?;
                let (change_index, change_ref) = self.append_change_with_fork_in_write(
                    tx,
                    change_payload,
                    task_index,
                    &task_id,
                    now,
                    Some(fork_snapshot_index + 1),
                    PayloadSyncBoundary::TransactionCommit,
                )?;
                self.append_history_snapshot_links_in_write(
                    tx,
                    task_index,
                    &task_id,
                    change_index,
                    &change_ref,
                    &request.target_line,
                    &request.author_mode,
                    &resolved_snapshots,
                )?;
                let receipt_payload = json!({
                    "contract": "ait-local-land-receipt/v1",
                    "local_task_id": entry.local_task_id,
                    "local_change_id": entry.local_change_id,
                    "local_change_ref": entry.local_change_ref,
                    "task_id": task_id,
                    "change_ref": change_ref,
                    "target_line": request.target_line,
                    "pre_land_target_snapshot_id": entry.pre_land_target_snapshot_id,
                    "landed_snapshot_id": entry.landed_snapshot_id,
                    "landed_at_s": entry.landed_at_s,
                    "source_kind": "imported_local_land_receipt",
                });
                let (receipt_patchset_index, receipt_patchset_id) = self
                    .append_history_patchset_in_write(
                        tx,
                        change_index,
                        pre_land_index,
                        landed_index,
                        LOCAL_LAND_RECEIPT_SUMMARY_PREFIX,
                        &receipt_payload,
                        &request.author_mode,
                        entry.landed_at_s,
                        false,
                    )?;
                (
                    task_index,
                    task_id,
                    change_index,
                    change_ref,
                    receipt_patchset_index,
                    receipt_patchset_id,
                )
            };
            manifest_entries.push(json!({
                "local_task_id": entry.local_task_id,
                "local_change_id": entry.local_change_id,
                "local_change_ref": entry.local_change_ref,
                "task_index": task_index,
                "task_id": task_id,
                "change_index": change_index,
                "change_ref": change_ref,
                "receipt_patchset_index": receipt_patchset_index,
                "receipt_patchset_id": receipt_patchset_id,
                "pre_land_target_snapshot_id": entry.pre_land_target_snapshot_id,
                "landed_snapshot_id": entry.landed_snapshot_id,
                "landed_at_s": entry.landed_at_s,
                "snapshot_link_count": resolved_snapshots.len(),
            }));
            final_change = Some((task_index, task_id, change_index, change_ref));
            expected_pre_land = entry.landed_snapshot_id.clone();
            previous_landed_at_s = entry.landed_at_s;
        }
        if expected_pre_land != request.revision_snapshot_id {
            return Err(format!(
                "History promotion chain ends at {}, not requested revision {}.",
                expected_pre_land, request.revision_snapshot_id
            ));
        }
        let (final_task_index, final_task_id, final_change_index, final_change_ref) =
            final_change.ok_or_else(|| "History promotion has no final Change.".to_string())?;
        let patchset_base_snapshot_index = self.snapshot_index_in_history_write(
            tx,
            context.patchset_base_snapshot_id,
            "history promotion Patchset base",
        )?;
        let patchset_revision_snapshot_index = self.snapshot_index_in_history_write(
            tx,
            context.patchset_revision_snapshot_id,
            "history promotion Patchset revision",
        )?;
        let aggregate_patchset_index = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| Self::error("history aggregate Patchset inventory", error))?;
        let aggregate_owner = self.patchset_owner_index_in_write(tx, final_change_index)?;
        let aggregate_patchset_id = format!(
            "{final_change_ref}/P-{:02}",
            aggregate_owner.next_ordinal + 1
        );
        let mut manifest = json!({
            "contract": context.manifest_contract,
            "idempotency_key": request.idempotency_key,
            "request_sha256": fingerprint,
            "target_line": request.target_line,
            "base_snapshot_id": context.manifest_base_snapshot_id,
            "revision_snapshot_id": context.manifest_revision_snapshot_id,
            "display_summary": request.summary,
            "entries": manifest_entries,
            "aggregate": {
                "task_index": final_task_index,
                "task_id": final_task_id,
                "change_index": final_change_index,
                "change_ref": final_change_ref,
                "patchset_index": aggregate_patchset_index,
                "patchset_id": aggregate_patchset_id,
            },
        });
        if let Some(staged) = context.staged_request {
            let previous_stage_patchset_index = staged
                .previous_stage_patchset_id
                .as_deref()
                .map(|patchset_id| self.patchset_index_for_id(tx, patchset_id))
                .transpose()?;
            let manifest_object = manifest
                .as_object_mut()
                .expect("history promotion manifest is an object");
            manifest_object.insert(
                "promotion_id".to_string(),
                JsonValue::String(staged.promotion_id.clone()),
            );
            manifest_object.insert("stage_ordinal".to_string(), json!(staged.stage_ordinal));
            manifest_object.insert(
                "stage_base_snapshot_id".to_string(),
                JsonValue::String(staged.stage_base_snapshot_id.clone()),
            );
            manifest_object.insert(
                "stage_revision_snapshot_id".to_string(),
                JsonValue::String(staged.stage_revision_snapshot_id.clone()),
            );
            manifest_object.insert(
                "previous_stage_patchset_index".to_string(),
                previous_stage_patchset_index
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null),
            );
            manifest_object.insert(
                "previous_stage_patchset_id".to_string(),
                staged
                    .previous_stage_patchset_id
                    .clone()
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            );
            manifest_object.insert(
                "total_entry_count".to_string(),
                json!(staged.total_entry_count),
            );
            manifest_object.insert("final_stage".to_string(), json!(staged.final_stage));
        }
        let (actual_patchset_index, actual_patchset_id) = self.append_history_patchset_in_write(
            tx,
            final_change_index,
            patchset_base_snapshot_index,
            patchset_revision_snapshot_index,
            context.summary_prefix,
            &manifest,
            &request.author_mode,
            now,
            context.force_select,
        )?;
        if actual_patchset_index != aggregate_patchset_index
            || actual_patchset_id != aggregate_patchset_id
        {
            return Err("History promotion aggregate Patchset identity drifted.".to_string());
        }
        Ok(manifest)
    }

    fn prepare_history_promotion_write_set<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
    ) -> Result<(), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let record_files = [
            WorkflowBinaryV0Codec::task_file(),
            WorkflowBinaryV0Codec::change_file(),
            WorkflowBinaryV0Codec::patchset_file(),
            WorkflowBinaryV0Codec::snapshot_link_file(),
            WorkflowBinaryV0Codec::chain_index_file("task_change_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("task_patchset_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("task_attest_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("task_review_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("task_policy_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("task_land_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("task_waiver_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("change_patchset_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("change_land_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("change_snapshot_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("patchset_attest_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("patchset_review_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("patchset_policy_index.bin"),
            WorkflowBinaryV0Codec::chain_index_file("patchset_waiver_index.bin"),
        ];
        let payload_files = [
            WorkflowBinaryV0Codec::task_payload_file(),
            WorkflowBinaryV0Codec::change_payload_file(),
            WorkflowBinaryV0Codec::snapshot_link_payload_file(),
            WorkflowBinaryV0Codec::patchset_summary_file(),
        ];
        tx.prepare_write_set(&record_files, &payload_files, &[])
            .map_err(|error| Self::error("history promotion write-set preparation", error))
    }
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn prepare_history_promotion_v1_from_payload(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        #[cfg(feature = "perfetto-tracing")]
        let _operation_trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.history_promotion.store");
        let operation = "ServerWorkflowTaskStore::prepare_history_promotion";
        let (request, fingerprint, now) = {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.history_promotion.request_decode_validate",
            );
            self.repo_scope(operation, repo_name)?;
            let request: HistoryPromotionRequest = serde_json::from_value(payload.clone())
                .map_err(|error| format!("Invalid history promotion request: {error}"))?;
            if request.contract != HISTORY_PROMOTION_CONTRACT {
                return Err(format!(
                    "History promotion contract must be {HISTORY_PROMOTION_CONTRACT:?}."
                ));
            }
            nonempty(
                &request.idempotency_key,
                "history promotion idempotency_key",
            )?;
            if request.idempotency_key.len() > 256 {
                return Err("History promotion idempotency_key exceeds 256 bytes.".to_string());
            }
            nonempty(&request.target_line, "history promotion target_line")?;
            nonempty(
                &request.base_snapshot_id,
                "history promotion base_snapshot_id",
            )?;
            nonempty(
                &request.revision_snapshot_id,
                "history promotion revision_snapshot_id",
            )?;
            Self::author_mode_bits(&request.author_mode)?;
            if request.entries.is_empty() || request.entries.len() > MAX_HISTORY_PROMOTION_ENTRIES {
                return Err(format!(
                    "History promotion requires 1..={MAX_HISTORY_PROMOTION_ENTRIES} entries."
                ));
            }
            for entry in &request.entries {
                validate_expected_publication_identity(entry)?;
            }
            let fingerprint = request_sha256(payload)?;
            let now = Self::now_s()?;
            (request, fingerprint, now)
        };
        let plan_bindings = {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.history_promotion.plan_binding_resolution",
            );
            request
                .entries
                .iter()
                .map(|entry| {
                    let task_value = serde_json::to_value(&entry.task).map_err(|error| {
                        format!("History Task payload encoding failed: {error}")
                    })?;
                    let task_payload = task_value
                        .as_object()
                        .ok_or_else(|| "History Task payload must be an object.".to_string())?;
                    self.resolve_plan_binding_for_create(task_payload)
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        #[cfg(feature = "perfetto-tracing")]
        let writer_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.history_promotion.writer_critical_section",
        );
        let mut tx = {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new_lane(
                "ait.server.history_promotion.writer_admission",
            );
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?
        };
        let existing = {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.history_promotion.idempotent_replay_lookup",
            );
            self.existing_history_manifest_in_write(&tx, &request.idempotency_key)?
        };
        if let Some(existing) = existing {
            if existing.get("request_sha256").and_then(JsonValue::as_str)
                != Some(fingerprint.as_str())
            {
                return Err(format!(
                    "HISTORY_PROMOTION_IDEMPOTENCY_CONFLICT: key {:?} belongs to another request.",
                    request.idempotency_key
                ));
            }
            validate_replayed_publication_owners(&request, &existing)?;
            drop(tx);
            #[cfg(feature = "perfetto-tracing")]
            drop(writer_trace);
            return {
                #[cfg(feature = "perfetto-tracing")]
                let _trace = crate::perfetto_trace::PerfettoRange::new(
                    "ait.server.history_promotion.response_projection",
                );
                self.history_promotion_result(&existing, true)
            };
        }
        {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.history_promotion.write_set_prepare",
            );
            self.prepare_history_promotion_write_set(&mut tx)?;
        }
        let manifest = {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.history_promotion.transaction_mutation",
            );
            self.prepare_history_promotion_in_write(
                &mut tx,
                &request,
                &plan_bindings,
                &fingerprint,
                now,
                HistoryPromotionWriteContext {
                    line_head_snapshot_id: &request.base_snapshot_id,
                    manifest_contract: "ait-history-promotion/v1",
                    manifest_base_snapshot_id: &request.base_snapshot_id,
                    manifest_revision_snapshot_id: &request.revision_snapshot_id,
                    summary_prefix: HISTORY_PROMOTION_SUMMARY_PREFIX,
                    patchset_base_snapshot_id: &request.base_snapshot_id,
                    patchset_revision_snapshot_id: &request.revision_snapshot_id,
                    staged_request: None,
                    force_select: true,
                },
            )?
        };
        {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.history_promotion.transaction_commit",
            );
            tx.commit().map_err(|error| Self::error(operation, error))?;
        }
        drop(tx);
        #[cfg(feature = "perfetto-tracing")]
        drop(writer_trace);
        {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.history_promotion.response_projection",
            );
            self.history_promotion_result(&manifest, false)
        }
    }

    fn prepare_staged_history_promotion_from_payload(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let operation = "ServerWorkflowTaskStore::prepare_staged_history_promotion";
        self.repo_scope(operation, repo_name)?;
        let staged: StagedHistoryPromotionRequest = serde_json::from_value(payload.clone())
            .map_err(|error| format!("Invalid staged history promotion request: {error}"))?;
        if staged.contract != HISTORY_PROMOTION_STAGED_CONTRACT {
            return Err(format!(
                "Staged history promotion contract must be {HISTORY_PROMOTION_STAGED_CONTRACT:?}."
            ));
        }
        for (value, label) in [
            (&staged.promotion_id, "history promotion promotion_id"),
            (&staged.idempotency_key, "history promotion idempotency_key"),
            (&staged.target_line, "history promotion target_line"),
            (
                &staged.base_snapshot_id,
                "history promotion base_snapshot_id",
            ),
            (
                &staged.revision_snapshot_id,
                "history promotion revision_snapshot_id",
            ),
            (
                &staged.stage_base_snapshot_id,
                "history promotion stage_base_snapshot_id",
            ),
            (
                &staged.stage_revision_snapshot_id,
                "history promotion stage_revision_snapshot_id",
            ),
        ] {
            nonempty(value, label)?;
        }
        if staged.promotion_id.len() > 256 || staged.idempotency_key.len() > 256 {
            return Err("History promotion identities must not exceed 256 bytes.".to_string());
        }
        Self::author_mode_bits(&staged.author_mode)?;
        if staged.base_snapshot_id == staged.revision_snapshot_id
            || staged.stage_base_snapshot_id == staged.stage_revision_snapshot_id
        {
            return Err("History promotion Snapshot boundaries must not be empty.".to_string());
        }
        if staged.total_entry_count == 0 {
            return Err("History promotion total_entry_count must be positive.".to_string());
        }
        let stage_start = staged
            .stage_ordinal
            .checked_mul(MAX_HISTORY_PROMOTION_ENTRIES as u64)
            .ok_or_else(|| "History promotion stage ordinal overflow.".to_string())?;
        if stage_start >= staged.total_entry_count {
            return Err("History promotion stage starts beyond total_entry_count.".to_string());
        }
        let remaining = staged.total_entry_count - stage_start;
        let expected_stage_len = remaining.min(MAX_HISTORY_PROMOTION_ENTRIES as u64);
        if staged.entries.len() as u64 != expected_stage_len {
            return Err(format!(
                "History promotion stage {} requires {expected_stage_len} entries, got {}.",
                staged.stage_ordinal,
                staged.entries.len()
            ));
        }
        let expected_final = expected_stage_len == remaining;
        if staged.final_stage != expected_final
            || (staged.final_stage
                && staged.stage_revision_snapshot_id != staged.revision_snapshot_id)
            || (!staged.final_stage
                && staged.stage_revision_snapshot_id == staged.revision_snapshot_id)
        {
            return Err(
                "History promotion final-stage declaration disagrees with its global boundary."
                    .to_string(),
            );
        }
        for entry in &staged.entries {
            validate_expected_publication_identity(entry)?;
        }

        let request = HistoryPromotionRequest {
            contract: HISTORY_PROMOTION_CONTRACT.to_string(),
            idempotency_key: staged.idempotency_key.clone(),
            target_line: staged.target_line.clone(),
            base_snapshot_id: staged.stage_base_snapshot_id.clone(),
            revision_snapshot_id: staged.stage_revision_snapshot_id.clone(),
            author_mode: staged.author_mode.clone(),
            summary: staged.summary.clone(),
            entries: staged.entries.clone(),
        };
        let plan_bindings = request
            .entries
            .iter()
            .map(|entry| {
                let task_value = serde_json::to_value(&entry.task)
                    .map_err(|error| format!("History Task payload encoding failed: {error}"))?;
                let task_payload = task_value
                    .as_object()
                    .ok_or_else(|| "History Task payload must be an object.".to_string())?;
                self.resolve_plan_binding_for_create(task_payload)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fingerprint = request_sha256(payload)?;
        let now = Self::now_s()?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        if let Some(existing) =
            self.existing_staged_manifest_in_write(&tx, &staged.idempotency_key)?
        {
            if existing.get("request_sha256").and_then(JsonValue::as_str)
                != Some(fingerprint.as_str())
            {
                return Err(format!(
                    "HISTORY_PROMOTION_IDEMPOTENCY_CONFLICT: key {:?} belongs to another staged request.",
                    staged.idempotency_key
                ));
            }
            validate_replayed_publication_owners(&request, &existing)?;
            drop(tx);
            return self.staged_history_promotion_result(&existing, true);
        }
        self.validate_staged_history_position_in_write(&tx, &staged)?;
        self.prepare_history_promotion_write_set(&mut tx)?;
        let final_stage = staged.final_stage;
        let manifest = self.prepare_history_promotion_in_write(
            &mut tx,
            &request,
            &plan_bindings,
            &fingerprint,
            now,
            HistoryPromotionWriteContext {
                line_head_snapshot_id: &staged.base_snapshot_id,
                manifest_contract: if final_stage {
                    "ait-history-promotion/v2"
                } else {
                    "ait-history-promotion-stage/v1"
                },
                manifest_base_snapshot_id: &staged.base_snapshot_id,
                manifest_revision_snapshot_id: &staged.revision_snapshot_id,
                summary_prefix: if final_stage {
                    HISTORY_PROMOTION_SUMMARY_PREFIX
                } else {
                    HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX
                },
                patchset_base_snapshot_id: if final_stage {
                    &staged.base_snapshot_id
                } else {
                    &staged.stage_base_snapshot_id
                },
                patchset_revision_snapshot_id: if final_stage {
                    &staged.revision_snapshot_id
                } else {
                    &staged.stage_revision_snapshot_id
                },
                staged_request: Some(&staged),
                force_select: final_stage,
            },
        )?;
        if final_stage {
            let aggregate_patchset_index = manifest
                .get("aggregate")
                .and_then(JsonValue::as_object)
                .and_then(|aggregate| aggregate.get("patchset_index"))
                .and_then(JsonValue::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    "History promotion final stage has no aggregate Patchset index.".to_string()
                })?;
            self.staged_history_chain_for_final(&tx, aggregate_patchset_index)?;
        }
        tx.commit().map_err(|error| Self::error(operation, error))?;
        drop(tx);
        self.staged_history_promotion_result(&manifest, false)
    }

    pub(super) fn prepare_history_promotion_from_payload(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        match payload.get("contract").and_then(JsonValue::as_str) {
            Some(HISTORY_PROMOTION_CONTRACT) => {
                self.prepare_history_promotion_v1_from_payload(repo_name, payload)
            }
            Some(HISTORY_PROMOTION_STAGED_CONTRACT) => {
                self.prepare_staged_history_promotion_from_payload(repo_name, payload)
            }
            other => Err(format!(
                "History promotion contract must be {HISTORY_PROMOTION_CONTRACT:?} or {HISTORY_PROMOTION_STAGED_CONTRACT:?}, got {other:?}."
            )),
        }
    }
}
