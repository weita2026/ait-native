//! A transient read view of admitted source-era provenance. Historical Change
//! text is checked for consistency but is never resolved in current authority.
use super::*;

fn quoted_change(reference: &str, task: &str) -> bool {
    let Some((owner, child)) = reference.rsplit_once('/') else {
        return false;
    };
    owner == task
        && child
            .strip_prefix("C-")
            .filter(|n| n.len() == 2)
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=64).contains(&n))
}

fn quoted_patchset(reference: &str, task: &str, ordinal: u8) -> bool {
    reference.rsplit_once("/P-").is_some_and(|(owner, child)| {
        quoted_change(owner, task) && child == format!("{:02}", ordinal + 1)
    })
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn legacy_local_receipt_view<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
        expected_task: &str,
    ) -> Result<Option<PersistedLocalLandReceipt>, String> {
        self.legacy_local_receipt_view_with_links(read, index, expected_task, None)
    }

    pub(super) fn legacy_local_receipt_view_with_links<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
        expected_task: &str,
        selected_links: Option<&BTreeMap<u32, V0SnapshotLinkRecord>>,
    ) -> Result<Option<PersistedLocalLandReceipt>, String> {
        let summary = self.history_patchset_summary(read, index)?;
        let Some(value) = parse_summary(&summary, LOCAL_LAND_RECEIPT_SUMMARY_PREFIX)? else {
            return Ok(None);
        };
        let mut receipt: PersistedLocalLandReceipt =
            serde_json::from_value(value).map_err(|e| e.to_string())?;
        if receipt.contract == "ait-local-land-receipt/v2" {
            return Ok(None);
        }
        if receipt.contract != "ait-local-land-receipt/v1" {
            return Err("Unsupported legacy source receipt contract".into());
        }
        let patchset = self.read_patchset(read, index).map_err(|e| e.to_string())?;
        let owner = self.patchset_owner(read, patchset)?;
        let task = self
            .read_task(read, owner.task_index)
            .map_err(|e| e.to_string())?;
        let task_id = self.task_id(owner.task_index);
        if expected_task != task_id
            || receipt.task_id != task_id
            || owner.change.is_some()
            || patchset.patchset_meta & 0b11 != 0
            || task.remote_meta & 1 != 0
            || receipt.source_kind != "imported_local_land_receipt"
            || receipt.local_task_id.is_empty()
            || receipt.local_task_id.trim() != receipt.local_task_id
            || receipt
                .change_ref
                .as_deref()
                .is_none_or(|r| !quoted_change(r, &task_id))
            || receipt
                .local_change_ref
                .as_deref()
                .is_none_or(|r| !quoted_change(r, &receipt.local_task_id))
            || receipt.local_change_id.as_deref().is_none_or(|id| {
                receipt.local_change_ref.as_deref()
                    != Some(format!("{}/{id}", receipt.local_task_id).as_str())
            })
            || receipt.landed_at_s == 0
            || patchset.created_at_s != receipt.landed_at_s
            || self.content_snapshot_id(read, patchset.base_snapshot_index)?
                != receipt.pre_land_target_snapshot_id
            || self.content_snapshot_id(read, patchset.revision_snapshot_index)?
                != receipt.landed_snapshot_id
            || patchset.ci_run_seq != 0
            || patchset.ci_selected_suite_count != 0
            || patchset.ci_suite_result_count != 0
        {
            return Err("HISTORY_PROMOTION_RECEIPT_CONFLICT: legacy receipt numeric identity or provenance differs".into());
        }
        let dag = self.load_history_snapshot_dag_for_roots(
            read,
            &[
                patchset.base_snapshot_index,
                patchset.revision_snapshot_index,
            ],
        )?;
        let revision =
            self.history_snapshot_ancestor_closure(read, &dag, patchset.revision_snapshot_index)?;
        let base =
            self.history_snapshot_ancestor_closure(read, &dag, patchset.base_snapshot_index)?;
        if !revision.contains(&patchset.base_snapshot_index) {
            return Err("Legacy source receipt has invalid ancestry".into());
        }
        let wanted = revision.difference(&base).copied().collect::<BTreeSet<_>>();
        let mut found = BTreeSet::new();
        let links = if let Some(links) = selected_links {
            links.values().copied().collect::<Vec<_>>()
        } else {
            (0..read
                .record_count_v0(WorkflowBinaryV0Codec::snapshot_link_file())
                .map_err(|e| e.to_string())?)
                .map(|i| {
                    WorkflowBinaryV0Codec::decode_snapshot_link(
                        &read
                            .read_record_v0(WorkflowBinaryV0Codec::snapshot_link_file(), i)
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        for link in links {
            if !wanted.contains(&link.content_snapshot_index) {
                continue;
            }
            if link.link_meta & SNAPSHOT_LINK_TOMBSTONE != 0
                || link.task_index != owner.task_index
                || link.change_index_plus1 != 0
                || !found.insert(link.content_snapshot_index)
                || link.created_at_s
                    != dag.snapshot_records[&link.content_snapshot_index].created_at_s
            {
                return Err(
                    "Legacy source receipt has foreign, duplicate or inconsistent numeric Links"
                        .into(),
                );
            }
        }
        if wanted.is_empty() || wanted != found {
            return Err("Legacy source receipt lacks complete numeric Links".into());
        }
        // Only this ephemeral current view clears source-era associations. The
        // original summary, its contract tag, and all bytes remain unchanged.
        receipt.change_ref = None;
        receipt.local_change_id = None;
        receipt.local_change_ref = None;
        Ok(Some(receipt))
    }

    pub(super) fn require_legacy_history_batch_window<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        request: &HistoryPromotionRequest,
        entries: &[JsonValue],
        receipts: &[u32],
    ) -> Result<(), String> {
        let fail = || {
            "LEGACY_HISTORY_BATCH_MISMATCH: current candidates or complete source batch differ"
                .to_string()
        };
        if receipts.len() != entries.len() || entries.is_empty() {
            return Err(fail());
        }
        let final_task = entries
            .last()
            .and_then(|e| e["task_index"].as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(fail)?;
        let (current_index, current) = self
            .current_task_patchset(tx, final_task)?
            .ok_or_else(fail)?;
        self.require_current_patchset(tx, current_index, current)?;
        let value = self
            .history_manifest_for_patchset(tx, current_index)?
            .ok_or_else(fail)?;
        let old_entries = value["entries"].as_array().ok_or_else(fail)?;
        if value["contract"] != "ait-history-promotion/v1"
            || !value["stage_ordinal"].is_null()
            || !value["promotion_id"].is_null()
            || !value["correction"].is_null()
            || value["target_line"] != request.target_line
            || value["base_snapshot_id"] != request.base_snapshot_id
            || value["revision_snapshot_id"] != request.revision_snapshot_id
            || old_entries.len() != entries.len()
            || value["aggregate"]["task_index"].as_u64() != Some(u64::from(final_task))
            || value["aggregate"]["task_id"] != self.task_id(final_task)
            || value["aggregate"]["change_index"].as_u64().is_none()
            || old_entries.last().is_none_or(|entry| {
                entry["change_index"] != value["aggregate"]["change_index"]
                    || entry["change_ref"] != value["aggregate"]["change_ref"]
            })
            || !value["aggregate"]["change_ref"]
                .as_str()
                .is_some_and(|r| quoted_change(r, &self.task_id(final_task)))
            || value["aggregate"]["patchset_index"].as_u64() != Some(u64::from(current_index))
            || !value["aggregate"]["patchset_id"]
                .as_str()
                .is_some_and(|id| {
                    quoted_patchset(id, &self.task_id(final_task), current.patch_ordinal)
                })
            || current.change_ordinal_plus1 != 0
            || self.content_snapshot_id(tx, current.base_snapshot_index)?
                != request.base_snapshot_id
            || self.content_snapshot_id(tx, current.revision_snapshot_index)?
                != request.revision_snapshot_id
        {
            return Err(fail());
        }
        for (position, ((old, new), &receipt)) in
            old_entries.iter().zip(entries).zip(receipts).enumerate()
        {
            let task = new["task_index"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(fail)?;
            if old["receipt_patchset_index"].as_u64() != Some(u64::from(receipt))
                || old["change_index"]
                    .as_u64()
                    .and_then(|i| u32::try_from(i).ok())
                    .is_none()
                || new["receipt_patchset_index"] != old["receipt_patchset_index"]
                || old["task_index"] != new["task_index"]
                || old["task_id"] != new["task_id"]
                || old["local_task_id"] != request.entries[position].local_task_id
                || request.entries[position].expected_remote_task_id.as_deref()
                    != Some(self.task_id(task).as_str())
            {
                return Err(fail());
            }
            for key in [
                "pre_land_target_snapshot_id",
                "landed_snapshot_id",
                "landed_at_s",
                "snapshot_link_count",
            ] {
                if old[key] != new[key] {
                    return Err(fail());
                }
            }
            let p = self.read_patchset(tx, receipt).map_err(|e| e.to_string())?;
            let raw_summary = self.history_patchset_summary(tx, receipt)?;
            let raw =
                parse_summary(&raw_summary, LOCAL_LAND_RECEIPT_SUMMARY_PREFIX)?.ok_or_else(fail)?;
            for key in [
                "task_id",
                "change_ref",
                "local_task_id",
                "local_change_id",
                "local_change_ref",
                "pre_land_target_snapshot_id",
                "landed_snapshot_id",
                "landed_at_s",
            ] {
                if old[key] != raw[key] {
                    return Err(fail());
                }
            }
            if !old["receipt_patchset_id"]
                .as_str()
                .is_some_and(|id| quoted_patchset(id, &self.task_id(task), p.patch_ordinal))
            {
                return Err(fail());
            }
            let (latest, _) = self.current_task_patchset(tx, task)?.ok_or_else(fail)?;
            if latest
                != if position + 1 == entries.len() {
                    current_index
                } else {
                    receipt
                }
            {
                return Err(fail());
            }
        }
        Ok(())
    }
}
