use super::*;
use crate::foundation::workflow_binary_v0::WorkflowBinaryV0Codec;

impl<D, const WRITE_LAYOUT: u32> BinaryDbServerPlanService<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + Clone,
{
    /// Validate fixed metadata and its existing references without reading packs.
    pub fn validate_metadata(&self) -> Result<(), String> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(
            self.db(),
            BinaryDbReadScope::PLAN
                .union(BinaryDbReadScope::WORKFLOW)
                .union(BinaryDbReadScope::CONTENT),
        );
        let plans = read.record_count(plan_file()).map_err(binary_error)?;
        let revisions = read
            .record_count(plan_revision_file())
            .map_err(binary_error)?;
        let items = read.record_count(plan_item_file()).map_err(binary_error)?;
        let actors = read
            .record_count(WorkflowBinaryV0Codec::actor_file())
            .map_err(binary_error)?;
        for index in 0..plans {
            let (record, _) = self.store.current_plan_record_with_read(&read, index)?;
            plan_status_from_record(&record)?;
            timestamp_string(record.created_at_s)?;
            timestamp_string(record.updated_at_s)?;
            if record.actor_index_plus1 > actors {
                return Err(format!("Plan PR-{index} references an unknown Actor"));
            }
            if let Some(head) = record.latest_revision_index_plus1.checked_sub(1) {
                if head >= revisions
                    || self
                        .store
                        .read_plan_revision_record_with_read(&read, head)?
                        .plan_index
                        != index
                {
                    return Err(format!(
                        "Plan PR-{index} references a missing or foreign head revision"
                    ));
                }
            }
        }
        let content = ServerBinaryRepositoryContentStore::new(self.db().clone());
        for index in 0..revisions {
            let record = self
                .store
                .read_plan_revision_record_with_read(&read, index)?;
            if record.plan_index >= plans || record.actor_index_plus1 > actors {
                return Err(format!(
                    "Plan revision {index} references an unknown Plan or Actor"
                ));
            }
            if let Some(previous) = record.previous_revision_index_plus1.checked_sub(1) {
                if previous >= revisions
                    || previous == index
                    || self
                        .store
                        .read_plan_revision_record_with_read(&read, previous)?
                        .plan_index
                        != record.plan_index
                {
                    return Err(format!(
                        "Plan revision {index} references an invalid predecessor"
                    ));
                }
            }
            if record
                .item_start_index
                .checked_add(u32::from(record.item_count))
                .is_none_or(|end| end > items)
            {
                return Err(format!(
                    "Plan revision {index} item range exceeds its existing authority"
                ));
            }
            let payload = self
                .store
                .read_plan_revision_payload_with_read(&read, &record)?;
            self.store.revision_items_with_read(&read, &record)?;
            timestamp_string(record.created_at_s)?;
            if !payload.artifact_blob_id.is_empty()
                && content
                    .blob_with_read(&read, &payload.artifact_blob_id)
                    .map_err(binary_error)?
                    .is_none()
            {
                return Err(format!(
                    "Plan revision {index} references a missing content Blob"
                ));
            }
        }
        Ok(())
    }
}
