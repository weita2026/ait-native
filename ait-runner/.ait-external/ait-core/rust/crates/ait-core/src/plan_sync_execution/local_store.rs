use super::local_ports::{
    PlanSyncLocalArtifactWriter, PlanSyncLocalInventoryStore, PlanSyncLocalLifecycleStore,
    PlanSyncLocalPlanCreate, PlanSyncLocalPlanRevision, PlanSyncLocalPlanStore,
    PlanSyncLocalPublicationStore, PlanSyncLocalRevisionArtifact, PlanSyncLocalRevisionStore,
};
use super::shared::{plan_lineage_identity_matches, LocalPlanId};
use crate::binary_db::LocalBinaryDbFs;
use crate::json_support::{json, JsonCodec, JsonMap, JsonNumber as Number, JsonValue};
use crate::plan_binary_db::read::views::{
    PlanHeadView, PlanItemView, PlanRevisionSummaryView, PlanRevisionView, PlanSummaryView,
};
use crate::plan_binary_db::{
    BinaryDbPlanStore, LocalPlanBinaryDb, PlanItemPayload, PlanItemRecord, PlanPayload, PlanRecord,
    PlanRevisionPayload, PlanRevisionRecord,
};
use chrono::{DateTime, Utc};

const PLAN_STATE_DRAFT_META: u8 = 0;
const PLAN_STATE_ARCHIVED_META: u8 = 1;
const PLAN_STATE_SUPERSEDED_META: u8 = 2;
const PLAN_PUBLISHED_META: u8 = 0b0000_0100;
const REVISION_PUBLISHED_META: u8 = 0b0000_0001;
const ITEM_STATE_OPEN_META: u8 = 1;
const ITEM_STATE_DONE_META: u8 = 2;
const ITEM_HAS_REF_META: u8 = 0b0000_0100;
const ITEM_TASKABLE_HINT_META: u8 = 0b0000_1000;

pub(super) struct BinaryDbPlanSyncLocalStore<const WRITE_LAYOUT: u32> {
    repo_name: String,
    plans: LocalPlanBinaryDb<WRITE_LAYOUT>,
}

impl<const WRITE_LAYOUT: u32> BinaryDbPlanSyncLocalStore<WRITE_LAYOUT> {
    pub(super) fn from_db(repo_name: impl Into<String>, db: LocalBinaryDbFs) -> Self {
        Self {
            repo_name: repo_name.into(),
            plans: LocalPlanBinaryDb::from_db(db),
        }
    }

    fn read_txn(&self) -> crate::binary_db::BinaryDbReadTxn<'_, LocalBinaryDbFs> {
        self.plans.begin_read_txn()
    }

    fn parse_plan_index(&self, plan_id: &str) -> Result<u32, String> {
        parse_binary_plan_index(plan_id)
    }

    fn parse_plan_index_for(&self, plan_id: &str, operation: &str) -> Result<u32, String> {
        self.parse_plan_index(plan_id)
            .map_err(|error| format!("{error} Operation: {operation}."))
    }

    fn parse_revision_index(&self, revision_id: &str) -> Result<u32, String> {
        parse_binary_revision_index(revision_id)
    }

    fn read_plan_head(&self, plan_index: u32) -> Result<PlanHeadView, String> {
        let read = self.read_txn();
        self.plans
            .get_plan(&read, plan_index, Some(self.repo_name.as_str()))
            .map_err(|err| err.to_string())
    }
}

impl<const WRITE_LAYOUT: u32> PlanSyncLocalInventoryStore
    for BinaryDbPlanSyncLocalStore<WRITE_LAYOUT>
{
    fn list_plan_summaries(&self) -> Result<Vec<JsonValue>, String> {
        let read = self.read_txn();
        self.plans
            .list_plans(&read, Some(self.repo_name.as_str()), None)
            .map_err(|err| err.to_string())?
            .iter()
            .map(binary_plan_summary_json)
            .collect()
    }

    fn list_plan_inventory_details(&self) -> Result<Option<Vec<JsonValue>>, String> {
        let read = self.read_txn();
        let plans = self
            .plans
            .list_plans(&read, Some(self.repo_name.as_str()), None)
            .map_err(|err| err.to_string())?;
        plans
            .into_iter()
            .filter(|summary| !matches!(summary.record.status_name(), "archived" | "superseded"))
            .map(|summary| {
                self.plans
                    .get_plan(&read, summary.plan_index, Some(self.repo_name.as_str()))
                    .map_err(|err| err.to_string())
                    .and_then(|view| binary_plan_detail_json(&view))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

impl<const WRITE_LAYOUT: u32> PlanSyncLocalPlanStore for BinaryDbPlanSyncLocalStore<WRITE_LAYOUT> {
    fn get_plan(&self, plan_id: &str) -> Result<JsonValue, String> {
        let plan_index = self.parse_plan_index_for(plan_id, "get local plan")?;
        let read = self.read_txn();
        let view = self
            .plans
            .get_plan(&read, plan_index, Some(self.repo_name.as_str()))
            .map_err(|err| err.to_string())?;
        binary_plan_detail_json(&view)
    }
}

impl<const WRITE_LAYOUT: u32> PlanSyncLocalRevisionStore
    for BinaryDbPlanSyncLocalStore<WRITE_LAYOUT>
{
    fn list_plan_revisions(&self, plan_id: &str) -> Result<Vec<JsonValue>, String> {
        let plan_index = self.parse_plan_index_for(plan_id, "list local plan revisions")?;
        let read = self.read_txn();
        self.plans
            .list_plan_revisions(&read, plan_index)
            .map_err(|err| err.to_string())?
            .iter()
            .map(binary_revision_json)
            .collect()
    }

    fn get_plan_revision_artifact(
        &self,
        plan_revision_id: &str,
    ) -> Result<Option<PlanSyncLocalRevisionArtifact>, String> {
        let revision_index = self.parse_revision_index(plan_revision_id)?;
        let read = self.read_txn();
        let (record, payload) = self
            .plans
            .read_current_plan_revision(&read, revision_index)
            .map_err(|err| err.to_string())?;
        if record.is_tombstone() {
            return Ok(None);
        }
        Ok(Some(PlanSyncLocalRevisionArtifact {
            artifact_path: payload
                .artifact_path_text()
                .map_err(|err| err.to_string())?,
            artifact_blob_id: optional_nonempty(
                payload
                    .artifact_blob_id_text()
                    .map_err(|err| err.to_string())?,
            ),
            remote_published: record.is_published() && record.published_revision_index().is_some(),
        }))
    }
}

impl<const WRITE_LAYOUT: u32> PlanSyncLocalPublicationStore
    for BinaryDbPlanSyncLocalStore<WRITE_LAYOUT>
{
    fn remote_adoption_allocates_fresh_local_plan_identity(&self) -> bool {
        true
    }

    fn remote_adoption_preserves_local_plan_identity(&self) -> bool {
        true
    }

    fn mark_plan_published(
        &self,
        plan_id: &str,
        _remote_name: Option<&str>,
        published_plan_id: &str,
        published_head_revision_id: Option<&str>,
        revision_mappings: &[(String, String)],
        published_at: &str,
    ) -> Result<JsonValue, String> {
        let plan_index = self.parse_plan_index_for(plan_id, "mark local plan published")?;
        let published_plan_index =
            self.parse_plan_index_for(published_plan_id, "record published remote plan ordinal")?;
        let published_head_revision_index = published_head_revision_id
            .map(|value| self.parse_revision_index(value))
            .transpose()?;
        let published_at_s = timestamp_s(published_at)?;
        let (current, title, revision_updates) = {
            let read = self.read_txn();
            let current = self
                .plans
                .get_plan(&read, plan_index, Some(self.repo_name.as_str()))
                .map_err(|err| err.to_string())?;
            let title = current.title_text().map_err(|err| err.to_string())?;
            let mut revision_updates = Vec::new();
            for (local_revision_id, remote_revision_id) in revision_mappings {
                let local_revision_index = self.parse_revision_index(local_revision_id)?;
                let remote_revision_index = self.parse_revision_index(remote_revision_id)?;
                let (record, _payload) = self
                    .plans
                    .read_current_plan_revision(&read, local_revision_index)
                    .map_err(|err| err.to_string())?;
                if record.plan_index != plan_index {
                    return Err(format!(
                        "Binary DB plan sync revision {local_revision_id} belongs to {}, not {plan_id}.",
                        crate::plan_binary_db::repository_plan_id(record.plan_index)
                    ));
                }
                let mut revision = record;
                revision.revision_meta |= REVISION_PUBLISHED_META;
                revision.published_revision_index_plus1 =
                    remote_revision_index.checked_add(1).ok_or_else(|| {
                        format!(
                            "Binary DB plan sync published revision index overflow: {remote_revision_id}"
                        )
                    })?;
                if revision.published_at_s == 0 {
                    revision.published_at_s = published_at_s;
                }
                revision_updates.push((local_revision_index, revision));
            }
            (current, title, revision_updates)
        };

        let mut plan_record = current.record.clone();
        plan_record.plan_meta |= PLAN_PUBLISHED_META;
        plan_record.published_plan_index_plus1 =
            published_plan_index.checked_add(1).ok_or_else(|| {
                format!("Binary DB plan sync published plan index overflow: {published_plan_id}")
            })?;
        plan_record.published_latest_revision_index_plus1 = published_head_revision_index
            .map(|index| {
                index.checked_add(1).ok_or_else(|| {
                    format!(
                        "Binary DB plan sync published head revision index overflow: {}",
                        published_head_revision_id.unwrap_or_default()
                    )
                })
            })
            .transpose()?
            .unwrap_or(0);
        if plan_record.published_at_s == 0 {
            plan_record.published_at_s = published_at_s;
        }
        plan_record.updated_at_s = published_at_s;

        let mut tx = self
            .plans
            .begin_local_publish_receipt_txn()
            .map_err(|err| err.to_string())?;
        tx.require_unchanged_plan(plan_index, &current.record)
            .map_err(|err| err.to_string())?;
        for (revision_index, revision) in &revision_updates {
            tx.overwrite_plan_revision(*revision_index, revision)
                .map_err(|err| err.to_string())?;
        }
        tx.overwrite_plan_commit(
            plan_index,
            plan_record,
            &plan_payload_preserving_id(&current.payload, title.as_str()),
        )
        .map_err(|err| err.to_string())?;
        tx.commit().map_err(|err| err.to_string())?;

        let read = self.read_txn();
        let view = self
            .plans
            .get_plan(&read, plan_index, Some(self.repo_name.as_str()))
            .map_err(|err| err.to_string())?;
        binary_plan_detail_json(&view)
    }
}

impl<const WRITE_LAYOUT: u32> PlanSyncLocalLifecycleStore
    for BinaryDbPlanSyncLocalStore<WRITE_LAYOUT>
{
    fn close_plan(
        &self,
        plan_id: &str,
        status: &str,
        closed_at: &str,
    ) -> Result<JsonValue, String> {
        if !matches!(status, "archived" | "superseded") {
            return Err(format!(
                "Unsupported historical Binary DB plan sync status: {status}"
            ));
        }
        let plan_index = self.parse_plan_index_for(plan_id, "close local plan")?;
        let closed_at_s = timestamp_s(closed_at)?;
        let current = self.read_plan_head(plan_index)?;
        let title = current.title_text().map_err(|err| err.to_string())?;
        if current.record.status_name() == status {
            return binary_plan_detail_json(&current);
        }
        let mut plan_record = current.record.clone();
        plan_record.plan_meta =
            (plan_record.plan_meta & !0b0000_0011_u8) | plan_state_meta(status)?;
        plan_record.updated_at_s = closed_at_s;

        let mut tx = self
            .plans
            .begin_local_prune_txn()
            .map_err(|err| err.to_string())?;
        tx.require_unchanged_plan(plan_index, &current.record)
            .map_err(|err| err.to_string())?;
        tx.overwrite_plan_commit(
            plan_index,
            plan_record,
            &plan_payload_preserving_id(&current.payload, title.as_str()),
        )
        .map_err(|err| err.to_string())?;
        tx.commit().map_err(|err| err.to_string())?;

        let view = self.read_plan_head(plan_index)?;
        binary_plan_detail_json(&view)
    }

    fn rekey_plan(
        &self,
        plan_id: &str,
        new_plan_id: &str,
        _rekeyed_at: &str,
    ) -> Result<JsonValue, String> {
        let plan_index = self.parse_plan_index_for(plan_id, "rekey local plan source")?;
        let new_plan_index = self.parse_plan_index_for(new_plan_id, "rekey local plan target")?;
        if plan_index != new_plan_index {
            return Err(format!(
                "Binary DB plan sync cannot rekey canonical dense plan identity from {plan_id} to {new_plan_id} without a migration alias index."
            ));
        }
        let current = self.read_plan_head(plan_index)?;
        binary_plan_detail_json(&current)
    }
}

impl<const WRITE_LAYOUT: u32> PlanSyncLocalArtifactWriter
    for BinaryDbPlanSyncLocalStore<WRITE_LAYOUT>
{
    fn create_plan(&self, request: &PlanSyncLocalPlanCreate<'_>) -> Result<JsonValue, String> {
        // Binary DB allocates dense local identities and stores repository and
        // actor provenance at the owning repository boundary. Retain these
        // values in the writer port for alternate stores and adoption tests.
        let _port_metadata = (
            request.plan_id,
            request.plan_revision_id,
            request.repo_name,
            request.source_kind,
            request.created_by,
            request.actor_type,
        );
        let created_at_s = timestamp_s(request.now)?;
        let item_values = parse_items_json(request.items_json)?;
        let mut tx = self
            .plans
            .begin_local_upsert_txn()
            .map_err(|err| err.to_string())?;
        let plan_index = tx
            .record_count(BinaryDbPlanStore::<LocalBinaryDbFs, WRITE_LAYOUT>::plan_file())
            .map_err(|err| err.to_string())?;
        let revision_index = tx
            .record_count(BinaryDbPlanStore::<LocalBinaryDbFs, WRITE_LAYOUT>::plan_revision_file())
            .map_err(|err| err.to_string())?;
        let item_start_index = tx
            .record_count(BinaryDbPlanStore::<LocalBinaryDbFs, WRITE_LAYOUT>::plan_item_file())
            .map_err(|err| err.to_string())?;
        for item in &item_values {
            let (record, payload) = binary_item_record_payload(item)?;
            tx.append_plan_item(record, &payload)
                .map_err(|err| err.to_string())?;
        }
        let revision_number = 1;
        let item_count = u16::try_from(item_values.len()).map_err(|_| {
            format!(
                "Binary DB plan sync item count exceeds u16::MAX: {}",
                item_values.len()
            )
        })?;
        let plan_record = PlanRecord {
            plan_meta: plan_meta(request.status, request.publication_state)?,
            reserved0: 0,
            payload_len: 0,
            payload_offset: 0,
            latest_revision_index_plus1: revision_index
                .checked_add(1)
                .ok_or_else(|| "Binary DB plan revision index overflow.".to_string())?,
            published_plan_index_plus1: 0,
            published_latest_revision_index_plus1: 0,
            created_at_s,
            updated_at_s: created_at_s,
            published_at_s: 0,
        };
        tx.append_plan(
            plan_record,
            &PlanPayload {
                title_bytes: request.title.as_bytes().to_vec(),
            },
        )
        .map_err(|err| err.to_string())?;
        let revision_record = PlanRevisionRecord {
            revision_meta: revision_meta(request.publication_state),
            reserved0: 0,
            payload_len: 0,
            revision_number,
            item_count,
            payload_offset: 0,
            plan_index,
            previous_revision_index_plus1: 0,
            item_start_index,
            published_revision_index_plus1: 0,
            root_tree_pack_index_plus1: request
                .artifact_root
                .map(|locator| locator.root_tree_pack_index_plus1)
                .unwrap_or(0),
            root_entry_ordinal: request
                .artifact_root
                .map(|locator| locator.root_entry_ordinal)
                .unwrap_or(0),
            created_at_s,
            published_at_s: 0,
        };
        let revision_payload = PlanRevisionPayload {
            title_snapshot_bytes: request.title.as_bytes().to_vec(),
            summary_bytes: request.summary.unwrap_or("").as_bytes().to_vec(),
            artifact_path_bytes: request.artifact_path.as_bytes().to_vec(),
            artifact_selector_bytes: request.artifact_selector.unwrap_or("").as_bytes().to_vec(),
            artifact_heading_bytes: request.artifact_heading.as_bytes().to_vec(),
            artifact_blob_id_bytes: request.artifact_blob_id.unwrap_or("").as_bytes().to_vec(),
        };
        tx.bind_revision_content_root(&revision_record, &revision_payload)
            .map_err(|err| err.to_string())?;
        let (committed_revision_index, _) = tx
            .append_plan_revision_commit(revision_record, &revision_payload)
            .map_err(|err| err.to_string())?;
        if committed_revision_index != revision_index {
            return Err(format!(
                "Binary DB plan sync expected revision index {revision_index}, wrote {committed_revision_index}."
            ));
        }
        tx.commit().map_err(|err| err.to_string())?;

        let read = self.read_txn();
        let view = self
            .plans
            .get_plan(&read, plan_index, Some(self.repo_name.as_str()))
            .map_err(|err| err.to_string())?;
        binary_plan_detail_json(&view)
    }

    fn revise_plan(&self, request: &PlanSyncLocalPlanRevision<'_>) -> Result<JsonValue, String> {
        // Actor/source provenance remains part of the writer port even though
        // the Binary DB revision record derives it from repository authority.
        let _port_metadata = (request.source_kind, request.created_by, request.actor_type);
        let plan_index =
            self.parse_plan_index_for(request.plan_id, "append local plan revision")?;
        let created_at_s = timestamp_s(request.now)?;
        let item_values = parse_items_json(request.items_json)?;
        let current = self.read_plan_head(plan_index)?;
        if current.record.is_tombstone() {
            return Err(format!("Unknown local plan: {}", request.plan_id));
        }
        if let Some(head_revision) = current.head_revision.as_ref() {
            let current_path = head_revision
                .payload
                .artifact_path_text()
                .map_err(|err| err.to_string())?;
            let current_selector = optional_nonempty(
                head_revision
                    .payload
                    .artifact_selector_text()
                    .map_err(|err| err.to_string())?,
            );
            if !plan_lineage_identity_matches(
                &current_path,
                current_selector.as_deref(),
                request.artifact_path,
                request.artifact_selector,
            ) {
                let local_plan_id = LocalPlanId::from_raw(request.plan_id)?;
                return Err(format!(
                    "Local Plan {} tracks {}, but the requested revision belongs to {}; refusing a cross-lineage revision.",
                    local_plan_id.reference(),
                    super::shared::plan_artifact_identity_label(
                        &current_path,
                        current_selector.as_deref()
                    ),
                    super::shared::plan_artifact_identity_label(
                        request.artifact_path,
                        request.artifact_selector
                    )
                ));
            }
        }
        let current_title = current.title_text().map_err(|err| err.to_string())?;
        let title = request.title.unwrap_or(current_title.as_str());
        let previous_revision_index_plus1 = current.record.latest_revision_index_plus1;
        let previous_revision_number = current
            .head_revision
            .as_ref()
            .map(|revision| revision.record.revision_number)
            .unwrap_or(0);
        let revision_number = previous_revision_number
            .checked_add(1)
            .ok_or_else(|| "Binary DB plan sync revision_number overflow.".to_string())?;

        let mut tx = self
            .plans
            .begin_local_upsert_txn()
            .map_err(|err| err.to_string())?;
        tx.require_unchanged_plan(plan_index, &current.record)
            .map_err(|err| err.to_string())?;
        let revision_index = tx
            .record_count(BinaryDbPlanStore::<LocalBinaryDbFs, WRITE_LAYOUT>::plan_revision_file())
            .map_err(|err| err.to_string())?;
        let item_start_index = tx
            .record_count(BinaryDbPlanStore::<LocalBinaryDbFs, WRITE_LAYOUT>::plan_item_file())
            .map_err(|err| err.to_string())?;
        if let Ok(request_revision_index) = self.parse_revision_index(request.plan_revision_id) {
            if request_revision_index != revision_index {
                return Err(format!(
                    "Binary DB plan sync expected next revision plan-revision:{revision_index}, got {}.",
                    request.plan_revision_id
                ));
            }
        }
        for item in &item_values {
            let (record, payload) = binary_item_record_payload(item)?;
            tx.append_plan_item(record, &payload)
                .map_err(|err| err.to_string())?;
        }
        let item_count = u16::try_from(item_values.len()).map_err(|_| {
            format!(
                "Binary DB plan sync item count exceeds u16::MAX: {}",
                item_values.len()
            )
        })?;
        let revision_record = PlanRevisionRecord {
            revision_meta: 0,
            reserved0: 0,
            payload_len: 0,
            revision_number,
            item_count,
            payload_offset: 0,
            plan_index,
            previous_revision_index_plus1,
            item_start_index,
            published_revision_index_plus1: 0,
            root_tree_pack_index_plus1: request
                .artifact_root
                .map(|locator| locator.root_tree_pack_index_plus1)
                .unwrap_or(0),
            root_entry_ordinal: request
                .artifact_root
                .map(|locator| locator.root_entry_ordinal)
                .unwrap_or(0),
            created_at_s,
            published_at_s: 0,
        };
        let revision_payload = PlanRevisionPayload {
            title_snapshot_bytes: title.as_bytes().to_vec(),
            summary_bytes: request.summary.unwrap_or("").as_bytes().to_vec(),
            artifact_path_bytes: request.artifact_path.as_bytes().to_vec(),
            artifact_selector_bytes: request.artifact_selector.unwrap_or("").as_bytes().to_vec(),
            artifact_heading_bytes: request.artifact_heading.as_bytes().to_vec(),
            artifact_blob_id_bytes: request.artifact_blob_id.unwrap_or("").as_bytes().to_vec(),
        };
        tx.bind_revision_content_root(&revision_record, &revision_payload)
            .map_err(|err| err.to_string())?;
        let (committed_revision_index, _) = tx
            .append_plan_revision_commit(revision_record, &revision_payload)
            .map_err(|err| err.to_string())?;
        if committed_revision_index != revision_index {
            return Err(format!(
                "Binary DB plan sync expected revision index {revision_index}, wrote {committed_revision_index}."
            ));
        }
        let mut plan_record = current.record.clone();
        plan_record.latest_revision_index_plus1 = revision_index
            .checked_add(1)
            .ok_or_else(|| "Binary DB plan sync latest revision index overflow.".to_string())?;
        plan_record.updated_at_s = created_at_s;
        tx.overwrite_plan_commit(
            plan_index,
            plan_record,
            &plan_payload_preserving_id(&current.payload, title),
        )
        .map_err(|err| err.to_string())?;
        tx.commit().map_err(|err| err.to_string())?;

        let read = self.read_txn();
        let view = self
            .plans
            .get_plan(&read, plan_index, Some(self.repo_name.as_str()))
            .map_err(|err| err.to_string())?;
        binary_plan_detail_json(&view)
    }
}

fn parse_binary_plan_index(value: &str) -> Result<u32, String> {
    crate::plan_binary_db::parse_repository_plan_id(value).map_err(|_| {
        format!("Binary DB plan sync expected canonical PR-<plan.bin ordinal>, got `{value}`.")
    })
}

fn parse_binary_revision_index(value: &str) -> Result<u32, String> {
    let raw = value.trim();
    let raw = raw
        .strip_prefix("plan-revision:")
        .or_else(|| raw.strip_prefix("revision:"))
        .unwrap_or(raw);
    raw.parse::<u32>().map_err(|_| {
        format!("Binary DB plan sync expected dense plan-revision:<index>, got `{value}`.")
    })
}

fn binary_plan_ref(index: u32) -> String {
    crate::plan_binary_db::repository_plan_id(index)
}

fn binary_revision_ref(index: u32) -> String {
    format!("plan-revision:{index}")
}

fn plan_payload_preserving_id(_current: &PlanPayload, title: &str) -> PlanPayload {
    PlanPayload {
        title_bytes: title.as_bytes().to_vec(),
    }
}

fn payload_plan_ref(index: u32, payload: &PlanPayload) -> Result<String, String> {
    let _ = payload;
    Ok(binary_plan_ref(index))
}

fn payload_revision_ref(index: u32, payload: &PlanRevisionPayload) -> Result<String, String> {
    let _ = payload;
    Ok(binary_revision_ref(index))
}

fn timestamp_s(raw: &str) -> Result<u64, String> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|err| format!("Binary DB plan sync timestamp `{raw}` is invalid: {err}"))?
        .with_timezone(&Utc);
    u64::try_from(parsed.timestamp())
        .map_err(|_| format!("Binary DB plan sync timestamp `{raw}` is before the Unix epoch."))
}

fn timestamp_string(seconds: u64) -> Result<String, String> {
    let seconds = i64::try_from(seconds)
        .map_err(|_| "Binary DB plan sync timestamp exceeds the RFC 3339 range.".to_string())?;
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
        .ok_or_else(|| "Binary DB plan sync timestamp exceeds the RFC 3339 range.".to_string())
}

fn optional_timestamp_value(seconds: u64) -> Result<JsonValue, String> {
    if seconds == 0 {
        Ok(JsonValue::Null)
    } else {
        Ok(JsonValue::String(timestamp_string(seconds)?))
    }
}

fn optional_nonempty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn optional_nonempty_string_value(value: String) -> JsonValue {
    optional_nonempty(value)
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}

fn optional_revision_ref_value(index_plus1: u32) -> JsonValue {
    index_plus1
        .checked_sub(1)
        .map(binary_revision_ref)
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}

fn optional_plan_ref_value(index_plus1: u32) -> JsonValue {
    index_plus1
        .checked_sub(1)
        .map(binary_plan_ref)
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}

fn plan_meta(status: &str, publication_state: &str) -> Result<u8, String> {
    let mut meta = plan_state_meta(status)?;
    if publication_state == "published" {
        meta |= PLAN_PUBLISHED_META;
    }
    Ok(meta)
}

fn plan_state_meta(status: &str) -> Result<u8, String> {
    Ok(match status {
        "draft" => PLAN_STATE_DRAFT_META,
        "archived" => PLAN_STATE_ARCHIVED_META,
        "superseded" => PLAN_STATE_SUPERSEDED_META,
        other => {
            return Err(format!(
                "Unsupported Binary DB plan sync status `{other}`; expected draft, archived, or superseded."
            ))
        }
    })
}

fn revision_meta(publication_state: &str) -> u8 {
    if publication_state == "published" {
        REVISION_PUBLISHED_META
    } else {
        0
    }
}

fn parse_items_json(items_json: &str) -> Result<Vec<JsonValue>, String> {
    let value = JsonCodec::parse_value(items_json, "Binary DB plan sync items")
        .map_err(|err| err.to_string())?;
    value
        .as_array()
        .cloned()
        .ok_or_else(|| "Binary DB plan sync items_json must be a JSON array.".to_string())
}

fn binary_item_record_payload(
    item: &JsonValue,
) -> Result<(PlanItemRecord, PlanItemPayload), String> {
    let object = item
        .as_object()
        .ok_or_else(|| "Binary DB plan sync item must be a JSON object.".to_string())?;
    let plan_item_ref = object
        .get("plan_item_ref")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let text = object.get("text").and_then(JsonValue::as_str).unwrap_or("");
    let checkbox_state = object
        .get("checkbox_state")
        .and_then(JsonValue::as_str)
        .unwrap_or("none");
    let heading_path = object
        .get("heading_path")
        .and_then(JsonValue::as_array)
        .map(|parts| {
            parts
                .iter()
                .map(|part| {
                    part.as_str().map(ToString::to_string).ok_or_else(|| {
                        "Binary DB plan sync heading_path entries must be strings.".to_string()
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let line_number = object
        .get("line_number")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let mut item_meta = match checkbox_state {
        "none" => 0,
        "open" => ITEM_STATE_OPEN_META,
        "done" => ITEM_STATE_DONE_META,
        other => {
            return Err(format!(
                "Unsupported Binary DB plan sync checkbox_state `{other}`."
            ))
        }
    };
    if !plan_item_ref.trim().is_empty() {
        item_meta |= ITEM_HAS_REF_META | ITEM_TASKABLE_HINT_META;
    }
    Ok((
        PlanItemRecord {
            item_meta,
            reserved0: 0,
            payload_len: 0,
            payload_offset: 0,
            line_number: u32::try_from(line_number).map_err(|_| {
                format!("Binary DB plan sync line_number is outside u32: {line_number}")
            })?,
        },
        PlanItemPayload {
            plan_item_ref_bytes: plan_item_ref.as_bytes().to_vec(),
            text_bytes: text.as_bytes().to_vec(),
            heading_path,
        },
    ))
}

fn binary_plan_summary_json(view: &PlanSummaryView) -> Result<JsonValue, String> {
    let head = view.head_revision.as_ref();
    let summary = head
        .map(|revision| {
            revision
                .payload
                .summary_text()
                .map_err(|err| err.to_string())
        })
        .transpose()?;
    let artifact_path = head
        .map(|revision| {
            revision
                .payload
                .artifact_path_text()
                .map_err(|err| err.to_string())
        })
        .transpose()?;
    let artifact_selector = head
        .map(|revision| {
            revision
                .payload
                .artifact_selector_text()
                .map_err(|err| err.to_string())
        })
        .transpose()?;
    let artifact_heading = head
        .map(|revision| {
            revision
                .payload
                .artifact_heading_text()
                .map_err(|err| err.to_string())
        })
        .transpose()?;
    let artifact_blob_id = head
        .map(|revision| {
            revision
                .payload
                .artifact_blob_id_text()
                .map_err(|err| err.to_string())
        })
        .transpose()?;
    Ok(json!({
        "plan_id": payload_plan_ref(view.plan_index, &view.payload)?,
        "repo_name": view.repo_name.clone().unwrap_or_default(),
        "title": view.payload.title_text().map_err(|err| err.to_string())?,
        "status": view.record.status_name(),
        "head_revision_id": head
            .map(|revision| payload_revision_ref(revision.revision_index, &revision.payload))
            .transpose()?
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
        "publication_state": if view.record.is_published() { "published" } else { "local_draft" },
        "published_remote_name": JsonValue::Null,
        "published_plan_id": optional_plan_ref_value(view.record.published_plan_index_plus1),
        "published_head_revision_id": optional_revision_ref_value(view.record.published_latest_revision_index_plus1),
        "published_at": optional_timestamp_value(view.record.published_at_s)?,
        "created_by": JsonValue::Null,
        "created_at": timestamp_string(view.record.created_at_s)?,
        "updated_at": timestamp_string(view.record.updated_at_s)?,
        "head_revision_number": head.map(|revision| JsonValue::Number(Number::from(i64::from(revision.record.revision_number)))).unwrap_or(JsonValue::Null),
        "head_revision_summary": summary.map(JsonValue::String).unwrap_or(JsonValue::Null),
        "head_artifact_path": artifact_path.map(JsonValue::String).unwrap_or(JsonValue::Null),
        "head_artifact_selector": artifact_selector.map(JsonValue::String).unwrap_or(JsonValue::Null),
        "head_artifact_heading": artifact_heading.map(JsonValue::String).unwrap_or(JsonValue::Null),
        "head_artifact_blob_id": artifact_blob_id.map(JsonValue::String).unwrap_or(JsonValue::Null),
        "head_revision_created_at": head
            .map(|revision| timestamp_string(revision.record.created_at_s).map(JsonValue::String))
            .transpose()?
            .unwrap_or(JsonValue::Null),
    }))
}

fn binary_plan_detail_json(view: &PlanHeadView) -> Result<JsonValue, String> {
    Ok(json!({
        "plan_id": payload_plan_ref(view.plan_index, &view.payload)?,
        "repo_name": view.repo_name.clone().unwrap_or_default(),
        "title": view.payload.title_text().map_err(|err| err.to_string())?,
        "status": view.record.status_name(),
        "head_revision_id": view
            .head_revision
            .as_ref()
            .map(|revision| payload_revision_ref(revision.revision_index, &revision.payload))
            .transpose()?
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
        "publication_state": if view.record.is_published() { "published" } else { "local_draft" },
        "published_remote_name": JsonValue::Null,
        "published_plan_id": optional_plan_ref_value(view.record.published_plan_index_plus1),
        "published_head_revision_id": optional_revision_ref_value(view.record.published_latest_revision_index_plus1),
        "published_at": optional_timestamp_value(view.record.published_at_s)?,
        "created_by": JsonValue::Null,
        "created_at": timestamp_string(view.record.created_at_s)?,
        "updated_at": timestamp_string(view.record.updated_at_s)?,
        "head_revision": view
            .head_revision
            .as_ref()
            .map(binary_revision_json)
            .transpose()?
            .unwrap_or(JsonValue::Null),
    }))
}

fn binary_revision_json(view: &PlanRevisionView) -> Result<JsonValue, String> {
    let summary = PlanRevisionSummaryView {
        revision_index: view.revision_index,
        record: view.record.clone(),
        payload: view.payload.clone(),
    };
    let mut revision = binary_revision_summary_json(view.revision_index, &summary)?;
    let object = revision
        .as_object_mut()
        .ok_or_else(|| "Binary DB revision JSON must be an object.".to_string())?;
    object.insert(
        "items".to_string(),
        JsonValue::Array(
            view.items
                .iter()
                .map(binary_item_json)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    Ok(revision)
}

fn binary_revision_summary_json(
    revision_index: u32,
    revision: &PlanRevisionSummaryView,
) -> Result<JsonValue, String> {
    Ok(json!({
        "plan_revision_id": payload_revision_ref(revision_index, &revision.payload)?,
        "plan_id": binary_plan_ref(revision.record.plan_index),
        "revision_number": revision.record.revision_number,
        "parent_plan_revision_id": revision
            .record
            .previous_revision_index()
            .map(binary_revision_ref)
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
        "title_snapshot": revision.payload.title_snapshot_text().map_err(|err| err.to_string())?,
        "summary": optional_nonempty_string_value(revision.payload.summary_text().map_err(|err| err.to_string())?),
        "artifact_path": revision.payload.artifact_path_text().map_err(|err| err.to_string())?,
        "artifact_selector": optional_nonempty_string_value(revision.payload.artifact_selector_text().map_err(|err| err.to_string())?),
        "artifact_heading": revision.payload.artifact_heading_text().map_err(|err| err.to_string())?,
        "artifact_blob_id": optional_nonempty_string_value(revision.payload.artifact_blob_id_text().map_err(|err| err.to_string())?),
        "items": [],
        "source_kind": "binary_db",
        "created_by": JsonValue::Null,
        "actor_type": "system",
        "publication_state": if revision.record.is_published() { "published" } else { "local_draft" },
        "published_plan_revision_id": optional_revision_ref_value(revision.record.published_revision_index_plus1),
        "published_at": optional_timestamp_value(revision.record.published_at_s)?,
        "created_at": timestamp_string(revision.record.created_at_s)?,
    }))
}

fn binary_item_json(view: &PlanItemView) -> Result<JsonValue, String> {
    let heading_path = JsonValue::Array(
        view.payload
            .heading_path
            .iter()
            .cloned()
            .map(JsonValue::String)
            .collect(),
    );
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "plan_item_ref".to_string(),
            JsonValue::String(
                view.payload
                    .plan_item_ref_text()
                    .map_err(|err| err.to_string())?,
            ),
        ),
        (
            "text".to_string(),
            JsonValue::String(view.payload.text().map_err(|err| err.to_string())?),
        ),
        (
            "checkbox_state".to_string(),
            JsonValue::String(view.record.checkbox_state_name().to_string()),
        ),
        ("heading_path".to_string(), heading_path),
        (
            "line_number".to_string(),
            JsonValue::Number(Number::from(i64::from(view.record.line_number))),
        ),
    ])))
}
