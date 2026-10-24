use super::task_change::PayloadSyncBoundary;
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const HISTORY_PROMOTION_CONTRACT: &str = "history-promotion-prepare/v3";
pub(super) const HISTORY_PROMOTION_STAGED_CONTRACT: &str = "history-promotion-prepare/v3";
pub(super) const HISTORY_PROMOTION_SUMMARY_PREFIX: &str = "ait-history-promotion/v1 ";
pub(super) const HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX: &str = "ait-history-promotion-stage/v1 ";
pub(super) const LOCAL_LAND_RECEIPT_SUMMARY_PREFIX: &str = "ait-local-land-receipt/v1 ";
#[path = "history_promotion_correction.rs"]
mod correction;
#[path = "history_promotion_legacy.rs"]
mod legacy;
#[path = "history_promotion_package.rs"]
mod package;

const MAX_HISTORY_PROMOTION_ENTRIES: usize = 64;
const MAX_HISTORY_SNAPSHOTS_PER_ENTRY: usize = 64;

struct HistorySnapshotRange<'a> {
    owner_ref: &'a str,
    base_snapshot_id: &'a str,
    revision_snapshot_id: &'a str,
    snapshots: &'a [HistoryPromotionSnapshot],
}

#[derive(Clone, Debug)]
pub(super) struct HistorySnapshotDag {
    snapshot_count: u32,
    snapshot_records: BTreeMap<u32, ServerBinarySnapshotRecord>,
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct HistoryPromotionEntry {
    local_task_id: String,
    local_change_id: Option<String>,
    local_change_ref: Option<String>,
    #[serde(default)]
    expected_remote_task_id: Option<String>,
    #[serde(default)]
    expected_remote_change_ref: Option<String>,
    task: HistoryPromotionTask,
    change: Option<HistoryPromotionChange>,
    pre_land_target_snapshot_id: String,
    landed_snapshot_id: String,
    landed_at_s: u64,
    snapshots: Vec<HistoryPromotionSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct HistoryPromotionChange {
    title: String,
    base_line: String,
    fork_snapshot_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
    correction: Option<correction::HistoryCorrection>,
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
    change_index: Option<u32>,
    change_ref: Option<String>,
    receipt_patchset_index: u32,
    receipt_patchset_id: String,
    pre_land_target_snapshot_id: String,
    landed_snapshot_id: String,
    landed_at_s: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct PersistedHistoryAggregate {
    task_index: u32,
    task_id: String,
    change_index: Option<u32>,
    change_ref: Option<String>,
    patchset_index: u32,
    patchset_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct PersistedLocalLandReceipt {
    contract: String,
    local_task_id: String,
    local_change_id: Option<String>,
    local_change_ref: Option<String>,
    task_id: String,
    change_ref: Option<String>,
    target_line: String,
    pre_land_target_snapshot_id: String,
    landed_snapshot_id: String,
    landed_at_s: u64,
    source_kind: String,
}

#[derive(Clone, Debug)]
struct ReusableHistoryReceipt {
    patchset_index: u32,
    patchset_id: String,
    legacy: bool,
}

#[derive(Clone, Copy)]
struct HistoryPromotionWriteContext<'a> {
    actor_index_plus1: u32,
    line_head_snapshot_id: &'a str,
    manifest_contract: &'a str,
    manifest_base_snapshot_id: &'a str,
    manifest_revision_snapshot_id: &'a str,
    summary_prefix: &'a str,
    patchset_base_snapshot_id: &'a str,
    patchset_revision_snapshot_id: &'a str,
    staged_request: Option<&'a StagedHistoryPromotionRequest>,
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
    let mut fingerprint_payload = super::actors::domain_request(payload)?;
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

#[cfg(test)]
#[test]
fn actor_context_does_not_change_history_request_identity() {
    let domain =
        json!({"contract":HISTORY_PROMOTION_CONTRACT,"idempotency_key":"stable","entries":[]});
    let expected = request_sha256(&domain).unwrap();
    for actor in [0, 1] {
        let mut request = domain.clone();
        request["actor_key_index"] = json!(actor);
        assert_eq!(request_sha256(&request).unwrap(), expected);
        assert_eq!(super::actors::domain_request(&request).unwrap(), domain);
    }
}

fn validate_expected_publication_identity(entry: &HistoryPromotionEntry) -> Result<(), String> {
    nonempty(&entry.local_task_id, "local_task_id")?;
    match (
        &entry.local_change_id,
        &entry.local_change_ref,
        &entry.change,
    ) {
        (None, None, None) => {
            if entry.expected_remote_change_ref.is_some() {
                return Err("INCOMING_PUBLICATION_REFERENCE_CONFLICT: Task-only history retains a published Change reference; explicit disposition required".to_string());
            }
        }
        (Some(id), Some(reference), Some(change)) => {
            nonempty(&change.title, "explicit Change title")?;
            nonempty(&change.base_line, "explicit Change base Line")?;
            nonempty(&change.fork_snapshot_id, "explicit Change fork")?;
            let ordinal = id
                .strip_prefix("C-")
                .filter(|digits| digits.len() == 2)
                .and_then(|digits| digits.parse::<u8>().ok())
                .filter(|n| (1..=64).contains(n));
            if ordinal.is_none() || reference != &format!("{}/{}", entry.local_task_id, id) {
                return Err(
                    "History promotion Change does not match exact source identity".to_string(),
                );
            }
        }
        _ => return Err(
            "History Change, local_change_id and local_change_ref must all be present or all null"
                .to_string(),
        ),
    }
    for value in [
        entry.expected_remote_task_id.as_deref(),
        entry.expected_remote_change_ref.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if value.trim() != value || value.is_empty() {
            return Err(
                "Expected publication identities must be exact nonempty strings".to_string(),
            );
        }
    }
    if entry.expected_remote_task_id.is_none() && entry.expected_remote_change_ref.is_some() {
        return Err("expected_remote_change_ref requires expected_remote_task_id".to_string());
    }
    Ok(())
}

fn validate_expected_publication_owner(
    entry: &HistoryPromotionEntry,
    task_id: &str,
    change_ref: Option<&str>,
) -> Result<(), String> {
    if entry
        .expected_remote_task_id
        .as_deref()
        .is_some_and(|expected| expected != task_id)
        || entry
            .expected_remote_change_ref
            .as_deref()
            .is_some_and(|expected| Some(expected) != change_ref)
    {
        return Err(format!(
            "HISTORY_PROMOTION_RECEIPT_CONFLICT: expected publication owner for {} differs",
            entry.local_task_id
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
            .and_then(JsonValue::as_str);
        if persisted_local_task_id != entry.local_task_id
            || persisted_local_change_ref != entry.local_change_ref.as_deref()
        {
            return Err(format!(
                "HISTORY_PROMOTION_RECEIPT_CONFLICT: replayed local identity does not match {}.",
                entry.local_task_id
            ));
        }
        let task_id = persisted
            .get("task_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "History promotion replay manifest entry has no task_id.".to_string())?;
        let change_ref = persisted.get("change_ref").and_then(JsonValue::as_str);
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
    fn history_request_fingerprint<A: ReadV0>(
        &self,
        read: &A,
        payload: &JsonValue,
    ) -> Result<String, String> {
        let mut normalized = payload.clone();
        for pointer in [
            "/expected_identity/patchset_id",
            "/previous_stage_patchset_id",
        ] {
            if let Some(value) = normalized
                .pointer_mut(pointer)
                .filter(|value| !value.is_null())
            {
                let reference = value
                    .as_str()
                    .ok_or("History Patchset reference must be a string")?;
                let index = self.patchset_index_for_id(read, reference)?;
                let patchset = self
                    .read_patchset(read, index)
                    .map_err(|error| error.to_string())?;
                *value = json!(self.patchset_public_id(patchset));
            }
        }
        request_sha256(&normalized)
    }
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
                || (is_final && manifest.contract != "ait-history-promotion/v3")
                || (!is_final && manifest.contract != "ait-history-promotion-stage/v2")
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
            let owner = self.patchset_owner(read, patchset)?;
            if manifest.aggregate.patchset_index != current_patchset_index
                || manifest.aggregate.task_index != owner.task_index
                || manifest.aggregate.task_id != self.task_id(owner.task_index)
                || manifest.aggregate.change_index != owner.change.map(|(index, _)| index)
                || manifest.aggregate.change_ref
                    != owner
                        .change
                        .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal))
                || manifest.aggregate.patchset_id != self.patchset_public_id(patchset)
            {
                return Err("Persisted history promotion stage owner is inconsistent".to_string());
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
            self.patchset_owner(read, previous_patchset)?;
            if self.patchset_public_id(previous_patchset) != previous_id {
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
                    || receipt
                        .change_index
                        .is_some_and(|index| !seen_changes.insert(index))
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

    fn validate_history_receipt_batch<A: ReadV0>(
        &self,
        read: &A,
        receipts: &[PersistedHistoryReceipt],
        base: &str,
        revision: &str,
    ) -> Result<(), String> {
        if receipts.is_empty() || receipts.len() > MAX_HISTORY_PROMOTION_ENTRIES {
            return Err("History receipt stage exceeds its bounded inventory".to_string());
        }
        let mut expected = base.to_string();
        let mut seen = BTreeSet::new();
        for receipt in receipts {
            let patchset = self
                .read_patchset(read, receipt.receipt_patchset_index)
                .map_err(|e| e.to_string())?;
            let owner = self.patchset_owner(read, patchset)?;
            let change_ref = owner
                .change
                .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal));
            if !seen.insert(receipt.task_index)
                || receipt.landed_at_s == 0
                || receipt.task_index != owner.task_index
                || receipt.task_id != self.task_id(owner.task_index)
                || receipt.change_index != owner.change.map(|(index, _)| index)
                || receipt.change_ref != change_ref
                || receipt.receipt_patchset_id != self.patchset_public_id(patchset)
                || receipt.pre_land_target_snapshot_id != expected
                || self.content_snapshot_id(read, patchset.base_snapshot_index)? != expected
                || self.content_snapshot_id(read, patchset.revision_snapshot_index)?
                    != receipt.landed_snapshot_id
            {
                return Err("History receipt breaks its exact owner/candidate chain".to_string());
            }
            let summary = self.history_patchset_summary(read, receipt.receipt_patchset_index)?;
            let value = parse_summary(&summary, LOCAL_LAND_RECEIPT_SUMMARY_PREFIX)?
                .ok_or("History receipt Patchset is not provenance-only")?;
            let mut local: PersistedLocalLandReceipt =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            let legacy = local.contract == "ait-local-land-receipt/v1";
            if legacy {
                local = self
                    .legacy_local_receipt_view(
                        read,
                        receipt.receipt_patchset_index,
                        &receipt.task_id,
                    )?
                    .ok_or("Legacy source receipt view is missing")?;
            }
            if local.contract
                != if legacy {
                    "ait-local-land-receipt/v1"
                } else {
                    "ait-local-land-receipt/v2"
                }
                || local.task_id != receipt.task_id
                || local.change_ref != receipt.change_ref
                || local.pre_land_target_snapshot_id != expected
                || local.landed_snapshot_id != receipt.landed_snapshot_id
                || local.landed_at_s != receipt.landed_at_s
                || local.source_kind != "imported_local_land_receipt"
            {
                return Err(
                    "HISTORY_PROMOTION_RECEIPT_CONFLICT: typed local receipt disagrees".to_string(),
                );
            }
            expected = receipt.landed_snapshot_id.clone();
        }
        if expected != revision {
            return Err("History receipts do not end at the declared revision".to_string());
        }
        // These records are provenance. Validation creates no Remote Land or terminal owner state.
        Ok(())
    }

    pub(super) fn apply_history_receipts_in_land_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        patchset_index: u32,
        task_index: u32,
        target: &str,
        _target_index: u32,
        head: Option<u32>,
    ) -> Result<Option<JsonValue>, String> {
        self.apply_history_receipts_with_package_mode(
            tx,
            patchset_index,
            task_index,
            target,
            _target_index,
            head,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_history_receipts_with_package_mode<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        patchset_index: u32,
        task_index: u32,
        target: &str,
        _target_index: u32,
        head: Option<u32>,
        package: bool,
    ) -> Result<Option<JsonValue>, String> {
        let Some(value) = self.history_manifest_for_patchset(tx, patchset_index)? else {
            if self
                .latest_history_manifest_for_task(tx, task_index)?
                .is_some()
            {
                return Err("AGGREGATE_HISTORY_MANIFEST_REQUIRED: current candidate lost its typed history authority".to_string());
            }
            return Ok(None);
        };
        let manifest: PersistedHistoryManifest =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        let patchset = self
            .read_patchset(tx, patchset_index)
            .map_err(|e| e.to_string())?;
        let owner = self.patchset_owner(tx, patchset)?;
        if manifest.target_line != target
            || manifest.aggregate.task_index != task_index
            || manifest.aggregate.task_id != self.task_id(task_index)
            || patchset.task_index != task_index
            || manifest.aggregate.patchset_index != patchset_index
            || manifest.aggregate.patchset_id != self.patchset_public_id(patchset)
            || manifest.aggregate.change_index != owner.change.map(|(index, _)| index)
            || manifest.aggregate.change_ref
                != owner
                    .change
                    .map(|(_, change)| self.change_ref(task_index, change.change_ordinal))
            || self.content_snapshot_id(tx, patchset.base_snapshot_index)?
                != manifest.base_snapshot_id
            || self.content_snapshot_id(tx, patchset.revision_snapshot_index)?
                != manifest.revision_snapshot_id
            || head != Some(patchset.base_snapshot_index)
        {
            return Err(
                "History manifest does not match exact aggregate acceptance authority".to_string(),
            );
        }
        if manifest.contract == "ait-history-promotion-correction/v2" {
            self.apply_history_correction_receipts(tx, &value, &manifest, _target_index)?;
        } else if manifest.contract != "ait-history-promotion/v3" || manifest.correction.is_some() {
            return Err("Unsupported history manifest contract".to_string());
        } else if manifest.stage_ordinal.is_some() {
            for stage in self.staged_history_chain_for_final(tx, patchset_index)? {
                self.validate_history_receipt_batch(
                    tx,
                    &stage.entries,
                    stage
                        .stage_base_snapshot_id
                        .as_deref()
                        .ok_or("Stage base is missing")?,
                    stage
                        .stage_revision_snapshot_id
                        .as_deref()
                        .ok_or("Stage revision is missing")?,
                )?;
            }
        } else if package {
            // Original successful Local receipts retain their own chain. The
            // tested rebased chain was independently validated before staging.
            let first = manifest.entries.first().ok_or("Package history is empty")?;
            let last = manifest.entries.last().ok_or("Package history is empty")?;
            self.validate_history_receipt_batch(
                tx,
                &manifest.entries,
                &first.pre_land_target_snapshot_id,
                &last.landed_snapshot_id,
            )?;
        } else {
            self.validate_history_receipt_batch(
                tx,
                &manifest.entries,
                &manifest.base_snapshot_id,
                &manifest.revision_snapshot_id,
            )?;
        }
        Ok(Some(value))
    }

    #[cfg(test)]
    pub(crate) fn apply_staged_history_receipts_before_atomic_land_for_test(
        &self,
        reference: &str,
        requested_ref: &str,
        target: &str,
        requested_patchset: Option<&str>,
        expected_head: Option<&str>,
    ) -> Result<(), String> {
        let mut tx = BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerLand)
            .map_err(|e| e.to_string())?;
        let owner = self.workflow_owner(&tx, reference)?;
        let requested = self.workflow_owner(&tx, requested_ref)?;
        if owner.task_index != requested.task_index {
            return Err("Task differs".to_string());
        }
        let (index, patchset) = self
            .current_task_patchset(&tx, owner.task_index)?
            .ok_or("Current candidate is missing")?;
        self.require_current_patchset(&tx, index, patchset)?;
        if requested_patchset.is_some_and(|id| id != self.patchset_public_id(patchset)) {
            return Err("Current candidate differs".to_string());
        }
        let (target_index, line) = ServerBinaryDbLineStore::<D, 1>::new(self.db.clone())
            .line_by_name_in_write(&tx, target)
            .map_err(|e| e.to_string())?
            .ok_or("Target is missing")?;
        if expected_head.is_some_and(|id| {
            Some(id.to_string())
                != line
                    .head_snapshot_index()
                    .map(|i| self.content_snapshot_id(&tx, i))
                    .transpose()
                    .ok()
                    .flatten()
        }) {
            return Err("STALE_TARGET_LINE".to_string());
        }
        self.apply_history_receipts_in_land_write(
            &mut tx,
            index,
            owner.task_index,
            target,
            target_index,
            line.head_snapshot_index(),
        )?;
        Ok(())
    }

    fn read_patchset_summary_in_write<A: ReadV0>(
        &self,
        tx: &A,
        patchset_index: u32,
    ) -> Result<String, String> {
        let record = self
            .read_patchset(tx, patchset_index)
            .map_err(|error| Self::error("history promotion Patchset read", error))?;
        let raw = tx
            .read_payload_v0(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                record.summary_offset,
                u32::from(record.summary_len),
            )
            .map_err(|error| Self::error("history promotion summary read", error))?;
        WorkflowBinaryV0Codec::decode_single_text_payload(&raw, "Patchset summary")
            .map_err(|error| Self::error("history promotion summary decode", error))
            .map(str::to_string)
    }

    pub(super) fn require_patchset_governance_authority<A: ReadV0>(
        &self,
        tx: &A,
        patchset_index: u32,
        patchset_id: &str,
        operation: &str,
    ) -> Result<(), String> {
        let summary = self.read_patchset_summary_in_write(tx, patchset_index)?;
        let (source_kind, governance_authority) = source_kind_for_summary(&summary);
        if governance_authority {
            Ok(())
        } else {
            Err(format!(
                "IMPORTED_LOCAL_LAND_RECEIPT: Patchset {patchset_id} is provenance-only ({source_kind}) and cannot perform {operation}; use the current history-promotion aggregate Patchset."
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
            == Some("ait-history-promotion/v3"))
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
        let canonical_previous_id = self.patchset_public_id(
            self.read_patchset(tx, previous_index)
                .map_err(|error| error.to_string())?,
        );
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
        if previous.contract != "ait-history-promotion-stage/v2"
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
            || previous.aggregate.patchset_id != canonical_previous_id
        {
            return Err(format!(
                "HISTORY_PROMOTION_STAGE_CONFLICT: predecessor {previous_patchset_id} does not continue promotion {} stage {}.",
                request.promotion_id, request.stage_ordinal
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn reusable_history_receipt_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        entry: &HistoryPromotionEntry,
        plan_binding: HistoryPromotionPlanBinding,
        target: &str,
        target_index: u32,
        fork: Option<u32>,
        base: u32,
        revision: u32,
    ) -> Result<Option<ReusableHistoryReceipt>, String> {
        let mut matched = None;
        for index in 0..tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|e| e.to_string())?
        {
            let summary = self.history_patchset_summary(tx, index)?;
            let Some(value) = parse_summary(&summary, LOCAL_LAND_RECEIPT_SUMMARY_PREFIX)? else {
                continue;
            };
            let mut receipt: PersistedLocalLandReceipt =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            let same_task = receipt.local_task_id == entry.local_task_id;
            let same_change = receipt
                .local_change_ref
                .as_ref()
                .is_some_and(|r| Some(r) == entry.local_change_ref.as_ref());
            if !same_task && !same_change {
                continue;
            }
            let legacy = receipt.contract == "ait-local-land-receipt/v1";
            if legacy {
                let expected = entry.expected_remote_task_id.as_deref()
                    .ok_or("HISTORY_PROMOTION_RECEIPT_CONFLICT: legacy reuse requires an exact expected Remote Task")?;
                if entry.change.is_some()
                    || entry.local_change_id.is_some()
                    || entry.local_change_ref.is_some()
                    || entry.expected_remote_change_ref.is_some()
                {
                    return Err("HISTORY_PROMOTION_RECEIPT_CONFLICT: legacy Task-only reuse cannot claim a current Change".into());
                }
                receipt = self
                    .legacy_local_receipt_view(tx, index, expected)?
                    .ok_or("Legacy source receipt view is missing")?;
            }
            let patchset = self.read_patchset(tx, index).map_err(|e| e.to_string())?;
            let owner = self.patchset_owner(tx, patchset)?;
            let task = self
                .read_task(tx, owner.task_index)
                .map_err(|e| e.to_string())?;
            let task_id = self.task_id(owner.task_index);
            let change_ref = owner
                .change
                .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal));
            if !same_task
                || receipt.local_change_id != entry.local_change_id
                || receipt.local_change_ref != entry.local_change_ref
                || receipt.contract
                    != if legacy {
                        "ait-local-land-receipt/v1"
                    } else {
                        "ait-local-land-receipt/v2"
                    }
                || receipt.source_kind != "imported_local_land_receipt"
                || receipt.target_line != target
                || receipt.pre_land_target_snapshot_id != entry.pre_land_target_snapshot_id
                || receipt.landed_snapshot_id != entry.landed_snapshot_id
                || receipt.landed_at_s != entry.landed_at_s
                || receipt.task_id != task_id
                || receipt.change_ref != change_ref
                || patchset.base_snapshot_index != base
                || patchset.revision_snapshot_index != revision
                || patchset.created_at_s != entry.landed_at_s
                || patchset.ci_run_seq != 0
                || patchset.ci_selected_suite_count != 0
                || patchset.ci_suite_result_count != 0
                || task.origin_plan_revision_index_plus1 != plan_binding.0
                || task.plan_item_index_plus1 != plan_binding.1
            {
                return Err("HISTORY_PROMOTION_RECEIPT_CONFLICT: reusable source identity or receipt data differs".to_string());
            }
            self.require_active_owner(tx, owner)
                .map_err(|error| format!("HISTORY_PROMOTION_RECEIPT_CONFLICT: no longer has reusable Task/Change ownership: {error}"))?;
            validate_expected_publication_owner(entry, &task_id, change_ref.as_deref())?;
            let task_payload = WorkflowBinaryV0Codec::decode_task_payload(
                &tx.read_payload(
                    WorkflowBinaryV0Codec::task_payload_file(),
                    task.payload_offset,
                    u32::from(task.payload_len),
                )
                .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if task_payload.title != entry.task.title.trim()
                || task_payload.intent != entry.task.intent.trim()
            {
                return Err("HISTORY_PROMOTION_RECEIPT_CONFLICT: Task content differs".to_string());
            }
            match (owner.change, entry.change.as_ref()) {
                (None, None) => {}
                (Some((_, change)), Some(source)) => {
                    let bytes = tx
                        .read_payload(
                            WorkflowBinaryV0Codec::change_payload_file(),
                            change.payload_offset,
                            u32::from(change.payload_len),
                        )
                        .map_err(|e| e.to_string())?;
                    let title =
                        WorkflowBinaryV0Codec::decode_single_text_payload(&bytes, "Change title")
                            .map_err(|e| e.to_string())?;
                    if title != source.title.trim()
                        || change.base_line_index_plus1 != target_index + 1
                        || change.fork_snapshot_index_plus1 != fork.map_or(0, |i| i + 1)
                    {
                        return Err(
                            "HISTORY_PROMOTION_RECEIPT_CONFLICT: explicit Change differs"
                                .to_string(),
                        );
                    }
                }
                _ => {
                    return Err(
                        "HISTORY_PROMOTION_RECEIPT_CONFLICT: optional Change association differs"
                            .to_string(),
                    )
                }
            }
            if self
                .task_successful_receipt(tx, owner.task_index)?
                .is_some()
            {
                return Err("HISTORY_PROMOTION_RECEIPT_CONFLICT: source Task already has real Remote acceptance".to_string());
            }
            let reusable = ReusableHistoryReceipt {
                patchset_index: index,
                patchset_id: self.patchset_public_id(patchset),
                legacy,
            };
            if matched.replace(reusable).is_some() {
                return Err("Source Task has multiple local receipt Patchsets".to_string());
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

    pub(super) fn load_history_snapshot_dag<A: ReadV0>(
        &self,
        tx: &A,
    ) -> Result<HistorySnapshotDag, String> {
        let snapshot_count = tx
            .record_count_v0(ServerBinarySnapshotCodec::<2>::record_file())
            .map_err(|error| Self::error("history Snapshot inventory", error))?;
        let parent_edges_by_child = self.load_history_parent_edges(tx, snapshot_count)?;
        let snapshot_file = ServerBinarySnapshotCodec::<2>::record_file();
        let mut snapshot_records = BTreeMap::new();
        for snapshot_index in 0..snapshot_count {
            snapshot_records.insert(
                snapshot_index,
                ServerBinarySnapshotCodec::<2>::decode_record(
                    &tx.read_record_v0(snapshot_file.clone(), snapshot_index)
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

    fn load_history_parent_edges<A: ReadV0>(
        &self,
        tx: &A,
        snapshot_count: u32,
    ) -> Result<BTreeMap<u32, Vec<(u16, u32)>>, String> {
        let edge_file =
            ServerBinarySnapshotParentEdgeCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file();
        let edge_count = tx
            .record_count_v0(edge_file.clone())
            .map_err(|error| Self::error("history Snapshot parent-edge inventory", error))?;
        let mut parent_edges_by_child = BTreeMap::<u32, Vec<(u16, u32)>>::new();
        for edge_index in 0..edge_count {
            let edge = ServerBinarySnapshotParentEdgeCodec::<
                SERVER_CONTENT_BINARY_LAYOUT_ID,
            >::decode_record(
                &tx.read_record_v0(edge_file.clone(), edge_index)
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
        Ok(parent_edges_by_child)
    }

    /// Required legacy receipt conversion needs only its endpoint closure, not
    /// a second traversal of every Snapshot in the Repository.
    pub(super) fn load_history_snapshot_dag_for_roots<A: ReadV0>(
        &self,
        tx: &A,
        roots: &[u32],
    ) -> Result<HistorySnapshotDag, String> {
        let snapshot_count = tx
            .record_count_v0(ServerBinarySnapshotCodec::<2>::record_file())
            .map_err(|e| e.to_string())?;
        let mut dag = HistorySnapshotDag {
            snapshot_count,
            snapshot_records: BTreeMap::new(),
            parent_edges_by_child: BTreeMap::new(),
        };
        let mut edges_loaded = false;
        let mut pending = roots.to_vec();
        while let Some(index) = pending.pop() {
            if dag.snapshot_records.contains_key(&index) {
                continue;
            }
            if index >= snapshot_count {
                return Err("Required Snapshot is outside canonical inventory".into());
            }
            let record = ServerBinarySnapshotCodec::<2>::decode_record(
                &tx.read_record_v0(ServerBinarySnapshotCodec::<2>::record_file(), index)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if record.has_parent_edges_authority() && !edges_loaded {
                dag.parent_edges_by_child = self.load_history_parent_edges(tx, snapshot_count)?;
                edges_loaded = true;
            }
            dag.snapshot_records.insert(index, record);
            pending.extend(self.history_snapshot_parents(tx, &dag, index)?);
        }
        Ok(dag)
    }

    fn history_snapshot_parents<A: ReadV0>(
        &self,
        _tx: &A,
        dag: &HistorySnapshotDag,
        snapshot_index: u32,
    ) -> Result<Vec<u32>, String> {
        if snapshot_index >= dag.snapshot_count {
            return Err(format!(
                "History Snapshot index {snapshot_index} is outside the canonical inventory."
            ));
        }
        let record = dag.snapshot_records.get(&snapshot_index).ok_or_else(|| {
            format!("History Snapshot index {snapshot_index} is missing from the loaded inventory.")
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

    pub(super) fn history_snapshot_ancestor_closure<A: ReadV0>(
        &self,
        tx: &A,
        dag: &HistorySnapshotDag,
        snapshot_index: u32,
    ) -> Result<BTreeSet<u32>, String> {
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
        self.validate_history_snapshot_links_range(
            tx,
            dag,
            HistorySnapshotRange {
                owner_ref: entry
                    .local_change_ref
                    .as_deref()
                    .unwrap_or(&entry.local_task_id),
                base_snapshot_id: &entry.pre_land_target_snapshot_id,
                revision_snapshot_id: &entry.landed_snapshot_id,
                snapshots: &entry.snapshots,
            },
            pre_land_index,
            landed_index,
            globally_linked,
        )
    }

    fn validate_history_snapshot_links_range<'a, F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        dag: &HistorySnapshotDag,
        range: HistorySnapshotRange<'a>,
        pre_land_index: u32,
        landed_index: u32,
        globally_linked: &mut BTreeSet<String>,
    ) -> Result<Vec<(u32, &'a HistoryPromotionSnapshot)>, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        if range.snapshots.is_empty() {
            return Err(format!(
                "History promotion entry {} has no Snapshot links.",
                range.owner_ref
            ));
        }
        if range.snapshots.len() > MAX_HISTORY_SNAPSHOTS_PER_ENTRY {
            return Err(format!(
                "History promotion entry {} has {} Snapshots; the current bounded writer limit is {MAX_HISTORY_SNAPSHOTS_PER_ENTRY}.",
                range.owner_ref,
                range.snapshots.len()
            ));
        }
        if range.snapshots.last().map(|row| row.snapshot_id.as_str())
            != Some(range.revision_snapshot_id)
        {
            return Err(format!(
                "History promotion entry {} Snapshot order must end at landed Snapshot {}.",
                range.owner_ref, range.revision_snapshot_id
            ));
        }
        let mut resolved = Vec::with_capacity(range.snapshots.len());
        let mut resolved_indexes = BTreeSet::new();
        for snapshot in range.snapshots {
            nonempty(&snapshot.snapshot_id, "history Snapshot id")?;
            if snapshot.snapshot_id == range.base_snapshot_id {
                return Err(format!(
                    "History promotion entry {} must not repeat its pre-Land boundary as a Snapshot link.",
                    range.owner_ref
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
                    range.owner_ref
                ));
            }
            resolved.push((snapshot_index, snapshot));
        }
        let landed_closure = self.history_snapshot_ancestor_closure(tx, dag, landed_index)?;
        if !landed_closure.contains(&pre_land_index) {
            return Err(format!(
                "History promotion landed Snapshot {} does not descend from pre-Land boundary {} for {}.",
                range.revision_snapshot_id,
                range.base_snapshot_id,
                range.owner_ref
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
                range.owner_ref,
                range.base_snapshot_id,
                range.revision_snapshot_id,
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
                        snapshot.snapshot_id, range.owner_ref
                    ));
                }
            }
            admitted.insert(*snapshot_index);
        }
        Ok(resolved)
    }

    fn append_history_snapshot_links_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        owner: WorkflowOwner,
        snapshots: &[(u32, &HistoryPromotionSnapshot)],
        actor_index_plus1: u32,
    ) -> Result<(), String> {
        if snapshots.len() > MAX_HISTORY_SNAPSHOTS_PER_ENTRY {
            return Err("History Snapshot range exceeds its bound".to_string());
        }
        for (index, snapshot) in snapshots {
            let raw = tx
                .read_record(ServerBinarySnapshotCodec::<2>::record_file(), *index)
                .map_err(|e| e.to_string())?;
            let content =
                ServerBinarySnapshotCodec::<2>::decode_record(&raw).map_err(|e| e.to_string())?;
            if snapshot.created_at_s != content.created_at_s {
                return Err("History Snapshot time differs from immutable content".to_string());
            }
            self.append_numeric_snapshot_link(tx, owner, *index, actor_index_plus1)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_history_patchset_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        owner: WorkflowOwner,
        base: u32,
        revision: u32,
        prefix: &str,
        payload: &JsonValue,
        author_mode: &str,
        created_at_s: u64,
        actor_index_plus1: u32,
    ) -> Result<(u32, String), String> {
        let summary = format!(
            "{prefix}{}",
            serde_json::to_string(payload).map_err(|e| e.to_string())?
        );
        let index = self.append_candidate_in_write(
            tx,
            owner,
            base,
            revision,
            &summary,
            author_mode,
            created_at_s,
            actor_index_plus1,
        )?;
        let record = self.read_patchset(tx, index).map_err(|e| e.to_string())?;
        Ok((index, self.patchset_public_id(record)))
    }

    fn prepare_history_promotion_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        request: &HistoryPromotionRequest,
        bindings: &[HistoryPromotionPlanBinding],
        fingerprint: &str,
        now: u64,
        context: HistoryPromotionWriteContext<'_>,
    ) -> Result<JsonValue, String> {
        if bindings.len() != request.entries.len() {
            return Err("History Plan-binding inventory differs".to_string());
        }
        let (target_index, line) = ServerBinaryDbLineStore::<D, 1>::new(self.db.clone())
            .line_by_name_in_write(tx, &request.target_line)
            .map_err(|e| e.to_string())?
            .ok_or("History target Line is missing")?;
        let head = self.snapshot_index_in_history_write(
            tx,
            context.line_head_snapshot_id,
            "history target base",
        )?;
        if line.head_snapshot_index() != Some(head) {
            return Err("STALE_HISTORY_PROMOTION_BASE: target moved".to_string());
        }
        let dag = self.load_history_snapshot_dag(tx)?;
        let final_snapshot = self.snapshot_index_in_history_write(
            tx,
            &request.revision_snapshot_id,
            "history revision",
        )?;
        if !self
            .history_snapshot_ancestor_closure(tx, &dag, final_snapshot)?
            .contains(&head)
        {
            return Err("History revision does not descend from remote base".to_string());
        }
        let mut expected_base = request.base_snapshot_id.clone();
        let mut tasks = BTreeSet::new();
        let mut linked = BTreeSet::new();
        let mut entries = Vec::new();
        let mut final_owner = None;
        let mut legacy_receipts = Vec::new();
        let mut previous_time = 0;
        for (entry, &binding) in request.entries.iter().zip(bindings) {
            validate_expected_publication_identity(entry)?;
            if entry.landed_at_s == 0 || entry.landed_at_s < previous_time {
                return Err("History promotion has a missing or regressing event time".to_string());
            }
            if !tasks.insert(entry.local_task_id.clone())
                || entry.landed_at_s == 0
                || entry.landed_at_s < previous_time
                || entry.pre_land_target_snapshot_id != expected_base
                || entry.landed_snapshot_id == expected_base
            {
                return Err(
                    "History entries repeat a Task or break the ordered local receipt chain"
                        .to_string(),
                );
            }
            let base = self.snapshot_index_in_history_write(
                tx,
                &entry.pre_land_target_snapshot_id,
                "history receipt base",
            )?;
            let revision = self.snapshot_index_in_history_write(
                tx,
                &entry.landed_snapshot_id,
                "history receipt revision",
            )?;
            let fork = entry
                .change
                .as_ref()
                .map(|change| {
                    if change.base_line != request.target_line {
                        return Err("Explicit history Change targets another Line".to_string());
                    }
                    self.snapshot_index_in_history_write(
                        tx,
                        &change.fork_snapshot_id,
                        "history Change fork",
                    )
                })
                .transpose()?;
            if let Some(fork) = fork {
                if !self
                    .history_snapshot_ancestor_closure(tx, &dag, revision)?
                    .contains(&fork)
                {
                    return Err(
                        "History revision does not descend from explicit Change fork".to_string(),
                    );
                }
            }
            let resolved =
                self.validate_history_snapshot_range(tx, &dag, entry, base, revision, &mut linked)?;
            let reusable = self.reusable_history_receipt_in_write(
                tx,
                entry,
                binding,
                &request.target_line,
                target_index,
                fork,
                base,
                revision,
            )?;
            let (owner, receipt_index, receipt_id) = if let Some(reusable) = reusable {
                if reusable.legacy {
                    legacy_receipts.push(reusable.patchset_index);
                }
                let patchset = self
                    .read_patchset(tx, reusable.patchset_index)
                    .map_err(|e| e.to_string())?;
                let owner = self.patchset_owner(tx, patchset)?;
                for (snapshot_index, _) in &resolved {
                    let count = tx
                        .record_count(WorkflowBinaryV0Codec::snapshot_link_file())
                        .map_err(|e| e.to_string())?;
                    let mut found = false;
                    for link_index in 0..count {
                        let link = WorkflowBinaryV0Codec::decode_snapshot_link(
                            &tx.read_record(
                                WorkflowBinaryV0Codec::snapshot_link_file(),
                                link_index,
                            )
                            .map_err(|e| e.to_string())?,
                        )
                        .map_err(|e| e.to_string())?;
                        if link.content_snapshot_index == *snapshot_index {
                            if found
                                || link.link_meta & SNAPSHOT_LINK_TOMBSTONE != 0
                                || link.task_index != owner.task_index
                                || link.change_index_plus1
                                    != owner.change.map_or(0, |(index, _)| index + 1)
                            {
                                return Err("Reusable history Snapshot numeric ownership differs"
                                    .to_string());
                            }
                            found = true;
                        }
                    }
                    if !found {
                        return Err("Reusable history Snapshot lacks numeric ownership".to_string());
                    }
                }
                (owner, reusable.patchset_index, reusable.patchset_id)
            } else {
                if entry.expected_remote_task_id.is_some() {
                    return Err("HISTORY_PROMOTION_RECEIPT_CONFLICT: expected publication owner has no exact reusable receipt".to_string());
                }
                if binding.0 != 0
                    && self
                        .task_for_plan_binding_in_write(tx, binding.0, binding.1)?
                        .is_some()
                {
                    return Err(
                        "History Plan binding already owns a server Task without an exact reusable receipt"
                            .to_string(),
                    );
                }
                let task = serde_json::to_value(&entry.task).map_err(|e| e.to_string())?;
                let (task_index, task_id) = self.append_task_in_write(
                    tx,
                    task.as_object().ok_or("Invalid Task payload")?,
                    binding.0,
                    binding.1,
                    binding.2,
                    now,
                    PayloadSyncBoundary::TransactionCommit,
                    context.actor_index_plus1,
                )?;
                let change = entry
                    .change
                    .as_ref()
                    .map(|change| {
                        let value = serde_json::to_value(change).map_err(|e| e.to_string())?;
                        let (index, _) = self.append_change_with_fork_in_write(
                            tx,
                            value.as_object().ok_or("Invalid Change payload")?,
                            task_index,
                            &task_id,
                            now,
                            fork.map(|index| index + 1),
                            PayloadSyncBoundary::TransactionCommit,
                            context.actor_index_plus1,
                        )?;
                        Ok::<_, String>((
                            index,
                            self.read_change(tx, index).map_err(|e| e.to_string())?,
                        ))
                    })
                    .transpose()?;
                let owner = WorkflowOwner { task_index, change };
                self.append_history_snapshot_links_in_write(
                    tx,
                    owner,
                    &resolved,
                    context.actor_index_plus1,
                )?;
                let receipt = json!({"contract":"ait-local-land-receipt/v2", "local_task_id":entry.local_task_id,
                "local_change_id":entry.local_change_id, "local_change_ref":entry.local_change_ref,
                "task_id":task_id, "change_ref":owner.change.map(|(_, c)| self.change_ref(task_index,c.change_ordinal)),
                "target_line":request.target_line, "pre_land_target_snapshot_id":entry.pre_land_target_snapshot_id,
                "landed_snapshot_id":entry.landed_snapshot_id, "landed_at_s":entry.landed_at_s,
                "source_kind":"imported_local_land_receipt"});
                let (index, id) = self.append_history_patchset_in_write(
                    tx,
                    owner,
                    base,
                    revision,
                    LOCAL_LAND_RECEIPT_SUMMARY_PREFIX,
                    &receipt,
                    &request.author_mode,
                    entry.landed_at_s,
                    context.actor_index_plus1,
                )?;
                (owner, index, id)
            };
            entries.push(json!({"local_task_id":entry.local_task_id,"local_change_id":entry.local_change_id,
            "local_change_ref":entry.local_change_ref,"task_index":owner.task_index,"task_id":self.task_id(owner.task_index),
            "change_index":owner.change.map(|(index,_)| index),"change_ref":owner.change.map(|(_,c)|self.change_ref(owner.task_index,c.change_ordinal)),
            "receipt_patchset_index":receipt_index,"receipt_patchset_id":receipt_id,
            "pre_land_target_snapshot_id":entry.pre_land_target_snapshot_id,"landed_snapshot_id":entry.landed_snapshot_id,
            "landed_at_s":entry.landed_at_s,"snapshot_link_count":resolved.len()}));
            final_owner = Some(owner);
            expected_base = entry.landed_snapshot_id.clone();
            previous_time = entry.landed_at_s;
        }
        if expected_base != request.revision_snapshot_id {
            return Err("History chain ends at a different revision".to_string());
        }
        let owner = final_owner.ok_or("History has no final Task")?;
        if !legacy_receipts.is_empty() {
            if context.staged_request.is_some() {
                return Err(
                    "LEGACY_HISTORY_BATCH_MISMATCH: staged compatibility is unsupported".into(),
                );
            }
            self.require_legacy_history_batch_window(tx, request, &entries, &legacy_receipts)?;
        }
        let base = self.snapshot_index_in_history_write(
            tx,
            context.patchset_base_snapshot_id,
            "aggregate base",
        )?;
        let revision = self.snapshot_index_in_history_write(
            tx,
            context.patchset_revision_snapshot_id,
            "aggregate revision",
        )?;
        let index = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|e| e.to_string())?;
        let ordinal = self.next_task_patchset_ordinal(tx, owner.task_index)?;
        let id = self.patchset_public_id(V0PatchsetRecord {
            task_index: owner.task_index,
            change_ordinal_plus1: owner.change_ordinal_plus1(),
            patch_ordinal: ordinal,
            ..V0PatchsetRecord::default()
        });
        let mut manifest = json!({"contract":context.manifest_contract,"idempotency_key":request.idempotency_key,
        "request_sha256":fingerprint,"target_line":request.target_line,"base_snapshot_id":context.manifest_base_snapshot_id,
        "revision_snapshot_id":context.manifest_revision_snapshot_id,"display_summary":request.summary,"entries":entries,
        "aggregate":{"task_index":owner.task_index,"task_id":self.task_id(owner.task_index),
            "change_index":owner.change.map(|(index,_)|index),"change_ref":owner.change.map(|(_,c)|self.change_ref(owner.task_index,c.change_ordinal)),
            "patchset_index":index,"patchset_id":id}});
        if let Some(stage) = context.staged_request {
            let previous = stage
                .previous_stage_patchset_id
                .as_deref()
                .map(|id| self.patchset_index_for_id(tx, id))
                .transpose()?;
            let previous_id = previous
                .map(|index| {
                    self.read_patchset(tx, index)
                        .map(|patchset| self.patchset_public_id(patchset))
                        .map_err(|error| error.to_string())
                })
                .transpose()?;
            let object = manifest.as_object_mut().ok_or("Invalid history manifest")?;
            for (key, value) in [
                ("promotion_id", json!(stage.promotion_id)),
                ("stage_ordinal", json!(stage.stage_ordinal)),
                (
                    "stage_base_snapshot_id",
                    json!(stage.stage_base_snapshot_id),
                ),
                (
                    "stage_revision_snapshot_id",
                    json!(stage.stage_revision_snapshot_id),
                ),
                ("previous_stage_patchset_index", json!(previous)),
                ("previous_stage_patchset_id", json!(previous_id)),
                ("total_entry_count", json!(stage.total_entry_count)),
                ("final_stage", json!(stage.final_stage)),
            ] {
                object.insert(key.to_string(), value);
            }
        }
        let actual = self.append_history_patchset_in_write(
            tx,
            owner,
            base,
            revision,
            context.summary_prefix,
            &manifest,
            &request.author_mode,
            now,
            context.actor_index_plus1,
        )?;
        if actual != (index, id) {
            return Err("Aggregate Patchset identity drifted".to_string());
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
            let request: HistoryPromotionRequest =
                serde_json::from_value(super::actors::domain_request(payload)?)
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
            if self.task_prefix.starts_with('R') {
                crate::foundation::workflow_package::require_ordinary_remote_line(
                    &request.target_line,
                )?;
            }
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
        let actor_index_plus1 = self.request_actor_in_write(
            &tx,
            Self::required_object(payload, "history promotion payload")?,
        )?;
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
                    actor_index_plus1,
                    line_head_snapshot_id: &request.base_snapshot_id,
                    manifest_contract: "ait-history-promotion/v3",
                    manifest_base_snapshot_id: &request.base_snapshot_id,
                    manifest_revision_snapshot_id: &request.revision_snapshot_id,
                    summary_prefix: HISTORY_PROMOTION_SUMMARY_PREFIX,
                    patchset_base_snapshot_id: &request.base_snapshot_id,
                    patchset_revision_snapshot_id: &request.revision_snapshot_id,
                    staged_request: None,
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
        let staged: StagedHistoryPromotionRequest =
            serde_json::from_value(super::actors::domain_request(payload)?)
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
        let now = Self::now_s()?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let actor_index_plus1 = self.request_actor_in_write(
            &tx,
            Self::required_object(payload, "history promotion payload")?,
        )?;
        let fingerprint = self.history_request_fingerprint(&tx, payload)?;
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
                actor_index_plus1,
                line_head_snapshot_id: &staged.base_snapshot_id,
                manifest_contract: if final_stage {
                    "ait-history-promotion/v3"
                } else {
                    "ait-history-promotion-stage/v2"
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
        self.request_actor(Self::required_object(payload, "history promotion payload")?)?;
        match payload.get("contract").and_then(JsonValue::as_str) {
            Some(HISTORY_PROMOTION_CONTRACT) if payload.get("stage_ordinal").is_some() => {
                self.prepare_staged_history_promotion_from_payload(repo_name, payload)
            }
            Some(HISTORY_PROMOTION_CONTRACT) => {
                self.prepare_history_promotion_v1_from_payload(repo_name, payload)
            }
            Some(correction::CORRECTION_PREPARE_CONTRACT) => {
                self.prepare_history_correction_from_payload(repo_name, payload)
            }
            other => Err(format!(
                "Unsupported history promotion contract {other:?}; require v3 or correction v2"
            )),
        }
    }
}
