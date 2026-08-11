use crate::binary_db::{BinaryDb, BinaryDbReadTxn, StoreResult};

use crate::plan_binary_db::BinaryDbPlanStore;

use super::filters::{
    plan_head_matches_contains_terms, sort_plan_heads, sort_plan_summaries, PlanHeadScanFilter,
};
use super::views::{
    PlanHeadView, PlanItemView, PlanRevisionSummaryView, PlanRevisionView, PlanSummaryView,
};

impl<B, const WRITE_LAYOUT: u32> BinaryDbPlanStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn list_plans<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        repo_name: Option<&str>,
        artifact_path: Option<&str>,
    ) -> StoreResult<Vec<PlanSummaryView>> {
        let count = read.record_count(Self::plan_file())?;
        let mut plans = Vec::new();
        for plan_index in 0..count {
            let summary = self.read_plan_summary_view(read, plan_index, repo_name)?;
            if summary.record.is_tombstone() {
                continue;
            }
            if let Some(path) = artifact_path {
                let head_artifact_path = summary
                    .head_revision
                    .as_ref()
                    .map(|revision| revision.payload.artifact_path_text())
                    .transpose()?;
                if head_artifact_path.as_deref() != Some(path) {
                    continue;
                }
            }
            plans.push(summary);
        }
        sort_plan_summaries(&mut plans);
        Ok(plans)
    }

    pub fn get_plan<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        plan_index: u32,
        repo_name: Option<&str>,
    ) -> StoreResult<PlanHeadView> {
        self.read_plan_head_view(read, plan_index, repo_name)
    }

    pub fn list_plan_revisions<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        plan_index: u32,
    ) -> StoreResult<Vec<PlanRevisionView>> {
        let (plan_record, _) = self.read_current_plan(read, plan_index)?;
        let revision_count = read.record_count(Self::plan_revision_file())?;
        let mut revisions = Vec::new();
        let mut next_index_plus1 = plan_record.latest_revision_index_plus1;
        let mut walked = 0_u32;
        while next_index_plus1 != 0 {
            if walked >= revision_count {
                return Err(format!(
                    "plan {plan_index} revision chain exceeds plan_revision.bin record count"
                )
                .into());
            }
            let revision_index = next_index_plus1 - 1;
            let revision = self.get_plan_revision(read, plan_index, revision_index)?;
            next_index_plus1 = revision.record.previous_revision_index_plus1;
            revisions.push(revision);
            walked += 1;
        }
        Ok(revisions)
    }

    pub fn get_plan_revision<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        plan_index: u32,
        revision_index: u32,
    ) -> StoreResult<PlanRevisionView> {
        let revision = self.read_plan_revision_view(read, revision_index)?;
        if revision.record.plan_index != plan_index {
            return Err(format!(
                "plan_revision.bin[{revision_index}] belongs to plan {}, not plan {plan_index}",
                revision.record.plan_index
            )
            .into());
        }
        Ok(revision)
    }

    pub fn scan_plan_heads<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        filter: PlanHeadScanFilter<'_>,
    ) -> StoreResult<Vec<PlanHeadView>> {
        let count = read.record_count(Self::plan_file())?;
        let mut plans = Vec::new();
        for plan_index in 0..count {
            let view = self.read_plan_head_view(read, plan_index, filter.repo_name)?;
            if view.record.is_tombstone() {
                continue;
            }
            if filter.active_only && !view.record.is_active() {
                continue;
            }
            if let Some(path) = filter.artifact_path {
                let head_artifact_path = view.head_artifact_path()?;
                if head_artifact_path.as_deref() != Some(path) {
                    continue;
                }
            }
            if !filter.contains_terms.is_empty()
                && !plan_head_matches_contains_terms(&view, filter.contains_terms)?
            {
                continue;
            }
            plans.push(view);
        }
        sort_plan_heads(&mut plans);
        Ok(plans)
    }

    fn read_plan_summary_view<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        plan_index: u32,
        repo_name: Option<&str>,
    ) -> StoreResult<PlanSummaryView> {
        let (record, payload) = self.read_current_plan(read, plan_index)?;
        let head_revision = match record.latest_revision_index() {
            Some(index) => Some(self.read_plan_revision_summary_view(read, index)?),
            None => None,
        };
        Ok(PlanSummaryView {
            plan_index,
            repo_name: repo_name.map(str::to_string),
            record,
            payload,
            head_revision,
        })
    }

    fn read_plan_head_view<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        plan_index: u32,
        repo_name: Option<&str>,
    ) -> StoreResult<PlanHeadView> {
        let (record, payload) = self.read_current_plan(read, plan_index)?;
        let head_revision = match record.latest_revision_index() {
            Some(index) => Some(self.get_plan_revision(read, plan_index, index)?),
            None => None,
        };
        Ok(PlanHeadView {
            plan_index,
            repo_name: repo_name.map(str::to_string),
            record,
            payload,
            head_revision,
        })
    }

    fn read_plan_revision_summary_view<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        revision_index: u32,
    ) -> StoreResult<PlanRevisionSummaryView> {
        let (record, payload) = self.read_current_plan_revision(read, revision_index)?;
        Ok(PlanRevisionSummaryView {
            revision_index,
            record,
            payload,
        })
    }

    fn read_plan_revision_view<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        revision_index: u32,
    ) -> StoreResult<PlanRevisionView> {
        let (record, payload) = self.read_current_plan_revision(read, revision_index)?;
        let mut items = Vec::new();
        for offset in 0..u32::from(record.item_count) {
            let item_index = record
                .item_start_index
                .checked_add(offset)
                .ok_or_else(|| "plan item index overflow".to_string())?;
            let (record, payload) = self.read_plan_item(read, item_index)?;
            items.push(PlanItemView {
                item_index,
                record,
                payload,
            });
        }
        Ok(PlanRevisionView {
            revision_index,
            record,
            payload,
            items,
        })
    }
}
