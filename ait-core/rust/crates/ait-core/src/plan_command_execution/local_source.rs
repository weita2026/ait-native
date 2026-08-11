use crate::binary_db::{BinaryDbReadTxn, LocalBinaryDbFs, LocalStateScope};
use crate::json_support::{json, JsonMap, JsonValue};

use super::data_source::{
    PlanCommandCandidateInputSource, PlanCommandCandidateInputs, PlanCommandPlanLister,
    PlanCommandPlanReader, PlanCommandRevisionLister, PlanCommandRevisionReader,
    PlanCommandTaskLister,
};
use super::local_shadow_ports::PlanCommandLocalShadowSource;
use super::{local_plan_publish_shadow_from_plan, optional_text, value_get};
use crate::plan_binary_db::{
    BinaryDbPlanStore, LocalPlanBinaryDb, PlanHeadScanFilter, PlanHeadView, PlanItemView,
    PlanRevisionSummaryView, PlanRevisionView, PlanSummaryView,
};

pub(super) struct LocalBinaryPlanCommandSource<const WRITE_LAYOUT: u32> {
    repo_name: String,
    store: LocalPlanBinaryDb<WRITE_LAYOUT>,
}
impl<const WRITE_LAYOUT: u32> LocalBinaryPlanCommandSource<WRITE_LAYOUT> {
    pub(super) fn from_db(repo_name: impl Into<String>, db: LocalBinaryDbFs) -> Self {
        let store = LocalPlanBinaryDb::from_db(db);
        Self {
            repo_name: repo_name.into(),
            store,
        }
    }

    fn read_plan_by_ref(&self, plan_ref: &str) -> Result<PlanHeadView, String> {
        let read = self.store.begin_read_txn();
        self.ensure_plan_metadata_ready(&read)?;
        let plan_index = self.resolve_plan_index(&read, plan_ref)?;
        self.store
            .get_plan(&read, plan_index, Some(self.repo_name.as_str()))
            .map_err(|err| err.to_string())
    }

    fn read_revision_by_ref(
        &self,
        plan_ref: &str,
        revision_ref: &str,
    ) -> Result<PlanRevisionView, String> {
        let read = self.store.begin_read_txn();
        self.ensure_plan_metadata_ready(&read)?;
        let plan_index = self.resolve_plan_index(&read, plan_ref)?;
        let revision_index = self.resolve_revision_index(&read, plan_index, revision_ref)?;
        self.store
            .get_plan_revision(&read, plan_index, revision_index)
            .map_err(|err| err.to_string())
    }

    fn ensure_plan_metadata_ready(
        &self,
        read: &crate::binary_db::BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    ) -> Result<(), String> {
        read.layout_id(BinaryDbPlanStore::<LocalBinaryDbFs, WRITE_LAYOUT>::plan_file())
            .map(|_| ())
            .map_err(|err| {
                format!("Plan Binary DB metadata file `plan.bin` is not readable: {err}")
            })
    }

    fn resolve_plan_index(
        &self,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
        plan_ref: &str,
    ) -> Result<u32, String> {
        match parse_plan_command_ref(plan_ref)? {
            PlanCommandRef::DenseIndex(plan_index) => Ok(plan_index),
            PlanCommandRef::ArtifactPath(path) => {
                let matches = self
                    .store
                    .scan_plan_heads(
                        read,
                        PlanHeadScanFilter {
                            repo_name: Some(self.repo_name.as_str()),
                            artifact_path: Some(path.as_str()),
                            contains_terms: &[],
                            active_only: false,
                        },
                    )
                    .map_err(|err| err.to_string())?;
                one_plan_index_match(matches, format!("artifact path `{path}`"))
            }
            PlanCommandRef::Title(title) => {
                let plans = self
                    .store
                    .scan_plan_heads(
                        read,
                        PlanHeadScanFilter {
                            repo_name: Some(self.repo_name.as_str()),
                            artifact_path: None,
                            contains_terms: &[],
                            active_only: false,
                        },
                    )
                    .map_err(|err| err.to_string())?;
                let mut matches = Vec::new();
                for plan in plans {
                    if plan.title_text().map_err(|err| err.to_string())? == title {
                        matches.push(plan);
                    }
                }
                one_plan_index_match(matches, format!("title `{title}`"))
            }
            PlanCommandRef::PublishedPlanIndex(published_plan_index) => {
                let plans = self
                    .store
                    .scan_plan_heads(
                        read,
                        PlanHeadScanFilter {
                            repo_name: Some(self.repo_name.as_str()),
                            artifact_path: None,
                            contains_terms: &[],
                            active_only: false,
                        },
                    )
                    .map_err(|err| err.to_string())?;
                let matches = plans
                    .into_iter()
                    .filter(|plan| plan.record.published_plan_index() == Some(published_plan_index))
                    .collect::<Vec<_>>();
                one_plan_index_match(
                    matches,
                    format!("published plan index `{published_plan_index}`"),
                )
            }
        }
    }

    fn resolve_revision_index(
        &self,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
        plan_index: u32,
        revision_ref: &str,
    ) -> Result<u32, String> {
        match parse_revision_command_ref(revision_ref)? {
            RevisionCommandRef::DenseIndex(revision_index) => Ok(revision_index),
            RevisionCommandRef::RevisionNumber(revision_number) => {
                let revisions = self
                    .store
                    .list_plan_revisions(read, plan_index)
                    .map_err(|err| err.to_string())?;
                let matches = revisions
                    .into_iter()
                    .filter(|revision| revision.record.revision_number == revision_number)
                    .collect::<Vec<_>>();
                one_revision_index_match(matches, format!("revision number `{revision_number}`"))
            }
            RevisionCommandRef::PublishedRevisionIndex(published_revision_index) => {
                let revisions = self
                    .store
                    .list_plan_revisions(read, plan_index)
                    .map_err(|err| err.to_string())?;
                let matches = revisions
                    .into_iter()
                    .filter(|revision| {
                        revision.record.published_revision_index() == Some(published_revision_index)
                    })
                    .collect::<Vec<_>>();
                one_revision_index_match(
                    matches,
                    format!("published revision index `{published_revision_index}`"),
                )
            }
        }
    }
}
impl<const WRITE_LAYOUT: u32> PlanCommandPlanLister for LocalBinaryPlanCommandSource<WRITE_LAYOUT> {
    fn list_plans(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String> {
        let read = self.store.begin_read_txn();
        self.ensure_plan_metadata_ready(&read)?;
        Ok(self
            .store
            .list_plans(&read, Some(repo_name), None)
            .map_err(|err| err.to_string())?
            .iter()
            .map(binary_plan_summary_json)
            .collect())
    }
}

impl<const WRITE_LAYOUT: u32> PlanCommandPlanReader for LocalBinaryPlanCommandSource<WRITE_LAYOUT> {
    fn get_plan(&mut self, plan_id: &str) -> Result<JsonValue, String> {
        self.read_plan_by_ref(plan_id)
            .and_then(|view| binary_plan_detail_json(&view))
    }
}

impl<const WRITE_LAYOUT: u32> PlanCommandRevisionLister
    for LocalBinaryPlanCommandSource<WRITE_LAYOUT>
{
    fn list_plan_revisions(&mut self, plan_id: &str) -> Result<Vec<JsonValue>, String> {
        let read = self.store.begin_read_txn();
        self.ensure_plan_metadata_ready(&read)?;
        let plan_index = self.resolve_plan_index(&read, plan_id)?;
        self.store
            .list_plan_revisions(&read, plan_index)
            .map_err(|err| err.to_string())?
            .iter()
            .map(binary_revision_json)
            .collect::<Result<Vec<_>, _>>()
    }
}

impl<const WRITE_LAYOUT: u32> PlanCommandRevisionReader
    for LocalBinaryPlanCommandSource<WRITE_LAYOUT>
{
    fn get_plan_revision(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        self.read_revision_by_ref(plan_id, plan_revision_id)
            .and_then(|view| binary_revision_json(&view))
    }
}

impl<const WRITE_LAYOUT: u32> PlanCommandTaskLister for LocalBinaryPlanCommandSource<WRITE_LAYOUT> {
    fn list_tasks(&mut self, _repo_name: &str) -> Result<Vec<JsonValue>, String> {
        Ok(Vec::new())
    }
}

impl<const WRITE_LAYOUT: u32> PlanCommandCandidateInputSource
    for LocalBinaryPlanCommandSource<WRITE_LAYOUT>
{
    fn candidate_inputs(
        &mut self,
        _repo_name: &str,
        contains_terms: &[String],
    ) -> Result<PlanCommandCandidateInputs, String> {
        let read = self.store.begin_read_txn();
        self.ensure_plan_metadata_ready(&read)?;
        let plans = self
            .store
            .scan_plan_heads(
                &read,
                PlanHeadScanFilter {
                    repo_name: Some(self.repo_name.as_str()),
                    artifact_path: None,
                    contains_terms,
                    active_only: true,
                },
            )
            .map_err(|err| err.to_string())?
            .iter()
            .map(binary_plan_detail_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PlanCommandCandidateInputs {
            plans,
            tasks: Vec::new(),
        })
    }
}

impl<const WRITE_LAYOUT: u32> PlanCommandLocalShadowSource
    for LocalBinaryPlanCommandSource<WRITE_LAYOUT>
{
    fn local_shadow_index(&mut self) -> Result<JsonMap<String, JsonValue>, String> {
        let read = self.store.begin_read_txn();
        self.ensure_plan_metadata_ready(&read)?;
        let rows = self
            .store
            .list_plans(&read, Some(self.repo_name.as_str()), None)
            .map_err(|err| err.to_string())?;
        let mut index = JsonMap::new();
        for row in rows {
            let plan = self
                .store
                .get_plan(&read, row.plan_index, Some(self.repo_name.as_str()))
                .map_err(|err| err.to_string())?;
            let plan = binary_plan_detail_json(&plan)?;
            let Some(shadow) = local_plan_publish_shadow_from_plan(&plan)? else {
                continue;
            };
            if let Some(local_plan_id) = optional_text(value_get(&plan, "plan_id"))? {
                index.insert(local_plan_id, shadow.clone());
            }
            if let Some(published_plan_id) = optional_text(value_get(&plan, "published_plan_id"))? {
                index.insert(published_plan_id, shadow.clone());
            }
        }
        Ok(index)
    }
}
enum PlanCommandRef {
    DenseIndex(u32),
    ArtifactPath(String),
    Title(String),
    PublishedPlanIndex(u32),
}

enum RevisionCommandRef {
    DenseIndex(u32),
    RevisionNumber(u16),
    PublishedRevisionIndex(u32),
}

pub(super) fn local_state_scope_from_text(value: &str) -> Result<LocalStateScope, String> {
    match value.trim() {
        "repository" => Ok(LocalStateScope::Repository),
        "line" => Ok(LocalStateScope::Line),
        "task" => Ok(LocalStateScope::Task),
        "remote_cache" => Ok(LocalStateScope::RemoteCache),
        other => Err(format!(
            "Unsupported Plan Binary DB current_line_state_scope `{other}`; expected repository, line, task, or remote_cache."
        )),
    }
}

fn parse_plan_command_ref(value: &str) -> Result<PlanCommandRef, String> {
    let value = value.trim();
    if value.starts_with("PR-") {
        return crate::plan_binary_db::parse_repository_plan_id(value)
            .map(PlanCommandRef::DenseIndex);
    }
    if let Some(raw) = value.strip_prefix("artifact:") {
        return nonempty_ref(raw, value, "artifact path").map(PlanCommandRef::ArtifactPath);
    }
    if looks_like_artifact_path(value) {
        return Ok(PlanCommandRef::ArtifactPath(value.to_string()));
    }
    if let Some(raw) = value.strip_prefix("title:") {
        return nonempty_ref(raw, value, "title").map(PlanCommandRef::Title);
    }
    if let Some(raw) = value.strip_prefix("published-plan:") {
        return parse_u32_ref(raw, value, "published plan").map(PlanCommandRef::PublishedPlanIndex);
    }
    if value.is_empty() {
        return Err("Plan Binary DB plan ref must not be empty.".to_string());
    }
    Err(format!(
        "Plan Binary DB plan ref `{value}` is not canonical; use `PR-<plan.bin ordinal>`, `artifact:<path>`, `title:<title>`, or `published-plan:<index>`."
    ))
}

fn parse_revision_command_ref(value: &str) -> Result<RevisionCommandRef, String> {
    let value = value.trim();
    if let Some(raw) = value
        .strip_prefix("plan-revision:")
        .or_else(|| value.strip_prefix("revision:"))
    {
        return parse_u32_ref(raw, value, "revision").map(RevisionCommandRef::DenseIndex);
    }
    if let Some(raw) = value.strip_prefix("revision-number:") {
        let revision_number = parse_u16_ref(raw, value, "revision number")?;
        if revision_number == 0 {
            return Err(format!(
                "Plan Binary DB revision ref `{value}` must use a positive revision number."
            ));
        }
        return Ok(RevisionCommandRef::RevisionNumber(revision_number));
    }
    if let Some(raw) = value.strip_prefix("published-revision:") {
        return parse_u32_ref(raw, value, "published revision")
            .map(RevisionCommandRef::PublishedRevisionIndex);
    }
    if let Ok(revision_index) = value.parse::<u32>() {
        return Ok(RevisionCommandRef::DenseIndex(revision_index));
    }
    if value.is_empty() {
        return Err("Plan Binary DB revision ref must not be empty.".to_string());
    }
    Err(format!(
        "Plan Binary DB revision ref `{value}` is not canonical; use `plan-revision:<index>`, `revision-number:<number>`, or `published-revision:<index>`."
    ))
}

fn parse_u32_ref(raw: &str, value: &str, label: &str) -> Result<u32, String> {
    raw.parse::<u32>()
        .map_err(|_| format!("Plan Binary DB {label} ref `{value}` must contain a u32 index."))
}

fn parse_u16_ref(raw: &str, value: &str, label: &str) -> Result<u16, String> {
    raw.parse::<u16>()
        .map_err(|_| format!("Plan Binary DB {label} ref `{value}` must contain a u16 number."))
}

fn nonempty_ref(raw: &str, value: &str, label: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(format!(
            "Plan Binary DB {label} ref `{value}` must not be empty."
        ));
    }
    Ok(raw.to_string())
}

fn looks_like_artifact_path(value: &str) -> bool {
    value.contains('/')
        || value.ends_with(".md")
        || value.ends_with(".markdown")
        || value.ends_with(".txt")
}

fn one_plan_index_match(matches: Vec<PlanHeadView>, label: String) -> Result<u32, String> {
    match matches.as_slice() {
        [plan] => Ok(plan.plan_index),
        [] => Err(format!("Plan Binary DB found no plan for {label}.")),
        _ => Err(format!(
            "Plan Binary DB found multiple plans for {label}; use its canonical `PR-<plan.bin ordinal>` identity."
        )),
    }
}

fn one_revision_index_match(matches: Vec<PlanRevisionView>, label: String) -> Result<u32, String> {
    match matches.as_slice() {
        [revision] => Ok(revision.revision_index),
        [] => Err(format!("Plan Binary DB found no revision for {label}.")),
        _ => Err(format!(
            "Plan Binary DB found multiple revisions for {label}; use a dense `plan-revision:<index>` ref."
        )),
    }
}

fn binary_plan_ref(plan_index: u32) -> String {
    crate::plan_binary_db::repository_plan_id(plan_index)
}

fn binary_revision_ref(revision_index: u32) -> String {
    format!("plan-revision:{revision_index}")
}

fn payload_plan_ref(index: u32, payload: &crate::plan_binary_db::PlanPayload) -> String {
    let _ = payload;
    binary_plan_ref(index)
}

fn payload_revision_ref(
    index: u32,
    payload: &crate::plan_binary_db::PlanRevisionPayload,
) -> String {
    let _ = payload;
    binary_revision_ref(index)
}

fn timestamp_string(value: u64) -> String {
    value.to_string()
}

fn optional_timestamp_value(value: u64) -> JsonValue {
    if value == 0 {
        JsonValue::Null
    } else {
        JsonValue::String(timestamp_string(value))
    }
}

fn optional_string_value(value: Option<String>) -> JsonValue {
    value.map(JsonValue::String).unwrap_or(JsonValue::Null)
}

fn optional_nonempty_string_value(value: String) -> JsonValue {
    if value.is_empty() {
        JsonValue::Null
    } else {
        JsonValue::String(value)
    }
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

fn binary_plan_summary_json(view: &PlanSummaryView) -> JsonValue {
    let head = view.head_revision.as_ref();
    let summary = head.and_then(|revision| revision.payload.summary_text().ok());
    let artifact_path = head.and_then(|revision| revision.payload.artifact_path_text().ok());
    let artifact_selector =
        head.and_then(|revision| revision.payload.artifact_selector_text().ok());
    let artifact_heading = head.and_then(|revision| revision.payload.artifact_heading_text().ok());
    let artifact_blob_id = head.and_then(|revision| revision.payload.artifact_blob_id_text().ok());
    json!({
        "plan_id": payload_plan_ref(view.plan_index, &view.payload),
        "repo_name": view.repo_name.clone().unwrap_or_default(),
        "title": view.payload.title_text().unwrap_or_default(),
        "status": view.record.status_name(),
        "head_revision_id": head
            .map(|revision| JsonValue::String(payload_revision_ref(revision.revision_index, &revision.payload)))
            .unwrap_or(JsonValue::Null),
        "publication_state": if view.record.is_published() {
            "published"
        } else {
            "local_draft"
        },
        "published_remote_name": JsonValue::Null,
        "published_plan_id": optional_plan_ref_value(view.record.published_plan_index_plus1),
        "published_head_revision_id": optional_revision_ref_value(
            view.record.published_latest_revision_index_plus1,
        ),
        "published_at": optional_timestamp_value(view.record.published_at_s),
        "created_by": JsonValue::Null,
        "created_at": timestamp_string(view.record.created_at_s),
        "updated_at": timestamp_string(view.record.updated_at_s),
        "head_revision_number": head.map(|revision| JsonValue::Number(i64::from(revision.record.revision_number).into())).unwrap_or(JsonValue::Null),
        "head_revision_summary": optional_string_value(summary),
        "head_artifact_path": optional_string_value(artifact_path),
        "head_artifact_selector": optional_string_value(artifact_selector),
        "head_artifact_heading": optional_string_value(artifact_heading),
        "head_artifact_blob_id": optional_string_value(artifact_blob_id),
        "head_revision_created_at": head.map(|revision| JsonValue::String(timestamp_string(revision.record.created_at_s))).unwrap_or(JsonValue::Null),
    })
}

fn binary_plan_detail_json(view: &PlanHeadView) -> Result<JsonValue, String> {
    Ok(json!({
        "plan_id": payload_plan_ref(view.plan_index, &view.payload),
        "repo_name": view.repo_name.clone().unwrap_or_default(),
        "title": view.title_text().map_err(|err| err.to_string())?,
        "status": view.status_name(),
        "head_revision_id": view
            .head_revision
            .as_ref()
            .map(|revision| JsonValue::String(payload_revision_ref(revision.revision_index, &revision.payload)))
            .unwrap_or(JsonValue::Null),
        "publication_state": view.publication_state_name(),
        "published_remote_name": JsonValue::Null,
        "published_plan_id": optional_plan_ref_value(view.record.published_plan_index_plus1),
        "published_head_revision_id": optional_revision_ref_value(
            view.record.published_latest_revision_index_plus1,
        ),
        "published_at": optional_timestamp_value(view.record.published_at_s),
        "created_by": JsonValue::Null,
        "created_at": timestamp_string(view.record.created_at_s),
        "updated_at": timestamp_string(view.record.updated_at_s),
        "head_revision": view
            .head_revision
            .as_ref()
            .map(binary_revision_json)
            .transpose()?
            .unwrap_or(JsonValue::Null),
    }))
}

fn binary_revision_summary_json(
    revision_index: u32,
    revision: &PlanRevisionSummaryView,
) -> Result<JsonValue, String> {
    Ok(json!({
        "plan_revision_id": payload_revision_ref(revision_index, &revision.payload),
        "plan_id": binary_plan_ref(revision.record.plan_index),
        "revision_number": revision.record.revision_number,
        "parent_plan_revision_id": revision
            .record
            .previous_revision_index()
            .map(binary_revision_ref)
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
        "title_snapshot": revision.payload.title_snapshot_text().map_err(|err| err.to_string())?,
        "summary": optional_nonempty_string_value(
            revision.payload.summary_text().map_err(|err| err.to_string())?,
        ),
        "artifact_path": revision.payload.artifact_path_text().map_err(|err| err.to_string())?,
        "artifact_selector": optional_nonempty_string_value(
            revision.payload.artifact_selector_text().map_err(|err| err.to_string())?,
        ),
        "artifact_heading": revision.payload.artifact_heading_text().map_err(|err| err.to_string())?,
        "artifact_blob_id": optional_nonempty_string_value(
            revision.payload.artifact_blob_id_text().map_err(|err| err.to_string())?,
        ),
        "items": [],
        "source_kind": "binary_db",
        "created_by": JsonValue::Null,
        "actor_type": "system",
        "publication_state": if revision.record.is_published() {
            "published"
        } else {
            "local_draft"
        },
        "published_plan_revision_id": optional_revision_ref_value(
            revision.record.published_revision_index_plus1,
        ),
        "published_at": optional_timestamp_value(revision.record.published_at_s),
        "created_at": timestamp_string(revision.record.created_at_s),
    }))
}

fn binary_revision_json(view: &PlanRevisionView) -> Result<JsonValue, String> {
    let summary = PlanRevisionSummaryView {
        revision_index: view.revision_index,
        record: view.record.clone(),
        payload: view.payload.clone(),
    };
    let mut revision = binary_revision_summary_json(view.revision_index, &summary)?;
    let payload = revision
        .as_object_mut()
        .ok_or("Binary DB revision payload must be a JSON object.")?;
    payload.insert(
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

fn binary_item_json(view: &PlanItemView) -> Result<JsonValue, String> {
    let plan_item_ref = view
        .payload
        .plan_item_ref_text()
        .map_err(|err| err.to_string())?;
    let text = view.payload.text().map_err(|err| err.to_string())?;
    let heading_path = JsonValue::Array(
        view.payload
            .heading_path
            .iter()
            .cloned()
            .map(JsonValue::String)
            .collect(),
    );
    Ok(json!({
        "plan_item_ref": if view.record.has_item_ref() {
            JsonValue::String(plan_item_ref)
        } else {
            JsonValue::Null
        },
        "text": optional_nonempty_string_value(text),
        "checkbox_state": view.record.checkbox_state_name(),
        "heading_path": heading_path,
        "line_number": view.record.line_number,
        "taskable_hint": view.record.is_taskable_hint(),
        "binary_item_id": format!("plan-item:{}", view.item_index),
    }))
}
