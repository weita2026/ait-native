use std::ops::Deref;

use chrono::{DateTime, SecondsFormat, Utc};

use crate::binary_db::LocalBinaryDbFs;
use crate::json_support::{JsonMap, JsonValue};
use crate::plan_store::{
    PlanHeadSummary, PlanItemRecord as StorePlanItemRecord, PlanPublishLinkage, PlanReadStore,
    PlanRecord as StorePlanRecord, PlanRevisionRecord as StorePlanRevisionRecord, PlanStoreError,
    PlanStoreResult,
};

use crate::plan_binary_db::{
    parse_repository_plan_id, repository_plan_id, LocalPlanBinaryDb, PlanHeadView, PlanItemView,
    PlanRevisionView, PlanSummaryView,
};

/// Repository-scoped Binary DB implementation of the neutral Plan read port.
///
/// Repository identity is contextual metadata; dense Plan records intentionally
/// do not duplicate it in every on-disk record.
pub struct LocalRepositoryPlanStore<const WRITE_LAYOUT: u32> {
    repo_name: String,
    plans: LocalPlanBinaryDb<WRITE_LAYOUT>,
}

impl<const WRITE_LAYOUT: u32> LocalRepositoryPlanStore<WRITE_LAYOUT> {
    pub fn from_db(repo_name: impl Into<String>, db: LocalBinaryDbFs) -> Self {
        Self {
            repo_name: repo_name.into(),
            plans: LocalPlanBinaryDb::from_db(db),
        }
    }

    pub fn repo_name(&self) -> &str {
        &self.repo_name
    }

    pub fn plans(&self) -> &LocalPlanBinaryDb<WRITE_LAYOUT> {
        &self.plans
    }

    fn read_plan(&self, plan_index: u32) -> PlanStoreResult<PlanHeadView> {
        let read = self.plans.begin_read_txn();
        self.plans
            .get_plan(&read, plan_index, Some(&self.repo_name))
            .map_err(storage_error)
    }

    fn read_revision(&self, revision_index: u32) -> PlanStoreResult<PlanRevisionView> {
        let read = self.plans.begin_read_txn();
        let (record, _) = self
            .plans
            .read_current_plan_revision(&read, revision_index)
            .map_err(storage_error)?;
        if record.is_tombstone() {
            return Err(PlanStoreError::NotFound(format!(
                "Unknown local plan revision: {}",
                revision_ref(revision_index)
            )));
        }
        self.plans
            .get_plan_revision(&read, record.plan_index, revision_index)
            .map_err(storage_error)
    }
}

impl<const WRITE_LAYOUT: u32> Deref for LocalRepositoryPlanStore<WRITE_LAYOUT> {
    type Target = LocalPlanBinaryDb<WRITE_LAYOUT>;

    fn deref(&self) -> &Self::Target {
        &self.plans
    }
}

impl<const WRITE_LAYOUT: u32> PlanReadStore for LocalRepositoryPlanStore<WRITE_LAYOUT> {
    fn list_plans(&self) -> PlanStoreResult<Vec<StorePlanRecord>> {
        let read = self.plans.begin_read_txn();
        self.plans
            .list_plans(&read, Some(&self.repo_name), None)
            .map_err(storage_error)?
            .iter()
            .map(plan_summary_record)
            .collect()
    }

    fn get_plan(&self, plan_id: &str) -> PlanStoreResult<StorePlanRecord> {
        let plan_index = parse_plan_index(plan_id)?;
        let view = self.read_plan(plan_index)?;
        if view.record.is_tombstone() {
            return Err(PlanStoreError::NotFound(format!(
                "Unknown local plan: {plan_id}"
            )));
        }
        plan_detail_record(&view)
    }

    fn list_plan_revisions(&self, plan_id: &str) -> PlanStoreResult<Vec<StorePlanRevisionRecord>> {
        let plan_index = parse_plan_index(plan_id)?;
        let read = self.plans.begin_read_txn();
        self.plans
            .list_plan_revisions(&read, plan_index)
            .map_err(storage_error)?
            .iter()
            .map(plan_revision_record)
            .collect()
    }

    fn get_plan_revision(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> PlanStoreResult<StorePlanRevisionRecord> {
        let plan_index = parse_plan_index(plan_id)?;
        let revision_index = parse_revision_index(plan_revision_id)?;
        let read = self.plans.begin_read_txn();
        let revision = self
            .plans
            .get_plan_revision(&read, plan_index, revision_index)
            .map_err(storage_error)?;
        plan_revision_record(&revision)
    }

    fn get_plan_revision_by_id(
        &self,
        plan_revision_id: &str,
    ) -> PlanStoreResult<StorePlanRevisionRecord> {
        let revision_index = parse_revision_index(plan_revision_id)?;
        plan_revision_record(&self.read_revision(revision_index)?)
    }

    fn resolve_plan_publish_linkage(
        &self,
        plan_id: Option<&str>,
        plan_revision_id: Option<&str>,
    ) -> PlanStoreResult<PlanPublishLinkage> {
        if plan_id.is_none() && plan_revision_id.is_none() {
            return Err(PlanStoreError::Invalid(
                "Plan linkage requires plan_id or plan_revision_id.".to_string(),
            ));
        }
        let requested_plan_index = plan_id.map(parse_plan_index).transpose()?;
        let requested_revision = plan_revision_id
            .map(parse_revision_index)
            .transpose()?
            .map(|index| self.read_revision(index).map(|view| (index, view)))
            .transpose()?;
        let revision_plan_index = requested_revision
            .as_ref()
            .map(|(_, revision)| revision.record.plan_index);
        if let (Some(plan_index), Some(revision_plan_index)) =
            (requested_plan_index, revision_plan_index)
        {
            if plan_index != revision_plan_index {
                return Err(PlanStoreError::Invalid(format!(
                    "Plan revision {} belongs to {}, not {}.",
                    plan_revision_id.unwrap_or_default(),
                    repository_plan_id(revision_plan_index),
                    repository_plan_id(plan_index)
                )));
            }
        }
        let plan_index = requested_plan_index
            .or(revision_plan_index)
            .ok_or_else(|| PlanStoreError::Invalid("Plan linkage is incomplete.".to_string()))?;
        let plan = self.read_plan(plan_index)?;
        let selected_revision_index = requested_revision
            .as_ref()
            .map(|(index, _)| *index)
            .or_else(|| plan.record.latest_revision_index());
        let selected_revision = match requested_revision {
            Some((_, revision)) => Some(revision),
            None => selected_revision_index
                .map(|index| self.read_revision(index))
                .transpose()?,
        };
        Ok(PlanPublishLinkage {
            plan_id: repository_plan_id(plan_index),
            published_plan_id: plan.record.published_plan_index().map(repository_plan_id),
            plan_revision_id: selected_revision_index.map(revision_ref),
            published_plan_revision_id: selected_revision
                .and_then(|revision| revision.record.published_revision_index())
                .map(revision_ref),
        })
    }
}

fn plan_summary_record(view: &PlanSummaryView) -> PlanStoreResult<StorePlanRecord> {
    let head = view.head_revision.as_ref();
    Ok(StorePlanRecord {
        plan_id: repository_plan_id(view.plan_index),
        repo_name: view.repo_name.clone().unwrap_or_default(),
        title: view.payload.title_text().map_err(storage_error)?,
        status: view.record.status_name().to_string(),
        head_revision_id: head.map(|revision| revision_ref(revision.revision_index)),
        publication_state: publication_state(view.record.is_published()),
        published_remote_name: None,
        published_plan_id: view.record.published_plan_index().map(repository_plan_id),
        published_head_revision_id: view
            .record
            .published_latest_revision_index()
            .map(revision_ref),
        published_at: optional_timestamp(view.record.published_at_s)?,
        created_by: None,
        created_at: timestamp(view.record.created_at_s)?,
        updated_at: timestamp(view.record.updated_at_s)?,
        head_revision: None,
        head_summary: Some(PlanHeadSummary {
            head_revision_number: head.map(|value| i64::from(value.record.revision_number)),
            head_revision_summary: head
                .map(|value| value.payload.summary_text().map_err(storage_error))
                .transpose()?
                .and_then(nonempty),
            head_artifact_path: head
                .map(|value| value.payload.artifact_path_text().map_err(storage_error))
                .transpose()?,
            head_artifact_selector: head
                .map(|value| {
                    value
                        .payload
                        .artifact_selector_text()
                        .map_err(storage_error)
                })
                .transpose()?
                .and_then(nonempty),
            head_artifact_heading: head
                .map(|value| value.payload.artifact_heading_text().map_err(storage_error))
                .transpose()?,
            head_artifact_blob_id: head
                .map(|value| value.payload.artifact_blob_id_text().map_err(storage_error))
                .transpose()?
                .and_then(nonempty),
            head_revision_created_at: head
                .map(|value| timestamp(value.record.created_at_s))
                .transpose()?,
        }),
    })
}

fn plan_detail_record(view: &PlanHeadView) -> PlanStoreResult<StorePlanRecord> {
    Ok(StorePlanRecord {
        plan_id: repository_plan_id(view.plan_index),
        repo_name: view.repo_name.clone().unwrap_or_default(),
        title: view.payload.title_text().map_err(storage_error)?,
        status: view.record.status_name().to_string(),
        head_revision_id: view
            .head_revision
            .as_ref()
            .map(|revision| revision_ref(revision.revision_index)),
        publication_state: publication_state(view.record.is_published()),
        published_remote_name: None,
        published_plan_id: view.record.published_plan_index().map(repository_plan_id),
        published_head_revision_id: view
            .record
            .published_latest_revision_index()
            .map(revision_ref),
        published_at: optional_timestamp(view.record.published_at_s)?,
        created_by: None,
        created_at: timestamp(view.record.created_at_s)?,
        updated_at: timestamp(view.record.updated_at_s)?,
        head_revision: view
            .head_revision
            .as_ref()
            .map(plan_revision_record)
            .transpose()?,
        head_summary: None,
    })
}

fn plan_revision_record(view: &PlanRevisionView) -> PlanStoreResult<StorePlanRevisionRecord> {
    let items = view
        .items
        .iter()
        .map(plan_item_record)
        .collect::<PlanStoreResult<Vec<_>>>()?;
    Ok(StorePlanRevisionRecord {
        plan_revision_id: revision_ref(view.revision_index),
        plan_id: repository_plan_id(view.record.plan_index),
        revision_number: i64::from(view.record.revision_number),
        parent_plan_revision_id: view.record.previous_revision_index().map(revision_ref),
        title_snapshot: view.payload.title_snapshot_text().map_err(storage_error)?,
        summary: nonempty(view.payload.summary_text().map_err(storage_error)?),
        artifact_path: view.payload.artifact_path_text().map_err(storage_error)?,
        artifact_selector: nonempty(
            view.payload
                .artifact_selector_text()
                .map_err(storage_error)?,
        ),
        artifact_heading: view
            .payload
            .artifact_heading_text()
            .map_err(storage_error)?,
        artifact_blob_id: nonempty(
            view.payload
                .artifact_blob_id_text()
                .map_err(storage_error)?,
        ),
        items,
        source_kind: "binary_db".to_string(),
        created_by: None,
        actor_type: "system".to_string(),
        publication_state: publication_state(view.record.is_published()),
        published_plan_revision_id: view.record.published_revision_index().map(revision_ref),
        published_at: optional_timestamp(view.record.published_at_s)?,
        created_at: timestamp(view.record.created_at_s)?,
    })
}

fn plan_item_record(view: &PlanItemView) -> PlanStoreResult<StorePlanItemRecord> {
    let plan_item_ref = nonempty(view.payload.plan_item_ref_text().map_err(storage_error)?);
    let text = nonempty(view.payload.text().map_err(storage_error)?);
    let checkbox_state = view.record.checkbox_state_name().to_string();
    let heading_path = view.payload.heading_path.clone();
    let line_number = i64::from(view.record.line_number);
    let payload = JsonMap::from_iter([
        (
            "plan_item_ref".to_string(),
            plan_item_ref
                .as_ref()
                .map(|value| JsonValue::String(value.clone()))
                .unwrap_or(JsonValue::Null),
        ),
        (
            "text".to_string(),
            text.as_ref()
                .map(|value| JsonValue::String(value.clone()))
                .unwrap_or(JsonValue::Null),
        ),
        (
            "checkbox_state".to_string(),
            JsonValue::String(checkbox_state.clone()),
        ),
        (
            "heading_path".to_string(),
            JsonValue::Array(
                heading_path
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        ),
        ("line_number".to_string(), JsonValue::from(line_number)),
    ]);
    Ok(StorePlanItemRecord {
        plan_item_ref,
        text,
        checkbox_state: Some(checkbox_state),
        heading_path,
        line_number: Some(line_number),
        payload,
    })
}

fn parse_plan_index(value: &str) -> PlanStoreResult<u32> {
    parse_repository_plan_id(value).map_err(PlanStoreError::Invalid)
}

fn parse_revision_index(value: &str) -> PlanStoreResult<u32> {
    let raw = value
        .trim()
        .strip_prefix("plan-revision:")
        .or_else(|| value.trim().strip_prefix("revision:"))
        .unwrap_or(value.trim());
    raw.parse::<u32>().map_err(|_| {
        PlanStoreError::Invalid(format!(
            "Plan revision identity `{value}` must be plan-revision:<u32 ordinal>."
        ))
    })
}

fn revision_ref(index: u32) -> String {
    format!("plan-revision:{index}")
}

fn publication_state(published: bool) -> String {
    if published {
        "published".to_string()
    } else {
        "local_draft".to_string()
    }
}

fn timestamp(seconds: u64) -> PlanStoreResult<String> {
    let seconds = i64::try_from(seconds).map_err(|_| {
        PlanStoreError::Storage("Binary DB Plan timestamp exceeds the RFC 3339 range".to_string())
    })?;
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, false))
        .ok_or_else(|| {
            PlanStoreError::Storage(
                "Binary DB Plan timestamp exceeds the RFC 3339 range".to_string(),
            )
        })
}

fn optional_timestamp(seconds: u64) -> PlanStoreResult<Option<String>> {
    (seconds != 0).then(|| timestamp(seconds)).transpose()
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn storage_error(error: impl ToString) -> PlanStoreError {
    PlanStoreError::Storage(error.to_string())
}
