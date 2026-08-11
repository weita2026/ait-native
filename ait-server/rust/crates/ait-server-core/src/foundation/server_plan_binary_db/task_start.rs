use super::*;
use crate::foundation::remote_binary_db::{BinaryDbIndexAppender, BinaryDbStoreFsyncPolicy};

#[derive(Clone, Debug)]
struct PreparedRevision {
    title: String,
    status: String,
    artifact_path: String,
    artifact_selector: Option<String>,
    artifact_heading: String,
    items: Vec<JsonValue>,
    summary: Option<String>,
    artifact_blob_id: String,
    item_offset: u32,
}

#[derive(Clone, Debug)]
enum PreparedTaskStartPlanKind {
    Create {
        revision: PreparedRevision,
    },
    Revise {
        plan_index: u32,
        plan_record: PlanRecord,
        previous_revision_number: u16,
        revision: PreparedRevision,
    },
    Existing {
        plan_index: u32,
        plan_record: PlanRecord,
        revision_index: u32,
        item_index: u32,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedTaskStartPlan {
    kind: PreparedTaskStartPlanKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskStartPlanBinding {
    pub(crate) action: &'static str,
    pub(crate) plan_index: u32,
    pub(crate) revision_index: u32,
    pub(crate) item_index: u32,
    pub(crate) plan_id: String,
    pub(crate) plan_revision_id: String,
}

impl PreparedTaskStartPlan {
    pub(crate) fn write_purpose(&self) -> ServerPlanBinaryDbWritePurpose {
        match self.kind {
            PreparedTaskStartPlanKind::Create { .. } => {
                ServerPlanBinaryDbWritePurpose::TaskStartCreate
            }
            PreparedTaskStartPlanKind::Revise { .. } => {
                ServerPlanBinaryDbWritePurpose::TaskStartRevise
            }
            PreparedTaskStartPlanKind::Existing { .. } => {
                ServerPlanBinaryDbWritePurpose::TaskStartExisting
            }
        }
    }
}

impl<D, const WRITE_LAYOUT: u32> BinaryDbServerPlanService<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub(crate) fn prepare_task_start_plan(
        &self,
        repo_name: &str,
        operation: &JsonValue,
        plan_item_ref: &str,
    ) -> Result<PreparedTaskStartPlan, String> {
        self.store
            .repo_scope("prepare_task_start_plan", repo_name)?;
        let operation = operation
            .as_object()
            .ok_or_else(|| "task-start plan operation must be a JSON object.".to_string())?;
        let action = required_text(operation, "action")?;
        match action.as_str() {
            "create" => {
                let payload = required_object_field(operation, "payload")?;
                if payload.contains_key("plan_id") {
                    return Err(
                        "Task-start Plan create does not accept caller-supplied plan_id."
                        .to_string(),
                    );
                }
                let revision =
                    self.prepare_task_start_revision(payload, plan_item_ref, None, None)?;
                if let Some(existing) =
                    self.exact_existing_task_start_create(&revision, plan_item_ref)?
                {
                    return Ok(existing);
                }
                Ok(PreparedTaskStartPlan {
                    kind: PreparedTaskStartPlanKind::Create { revision },
                })
            }
            "revise" => {
                let plan_id = required_text(operation, "plan_id")?;
                let payload = required_object_field(operation, "payload")?;
                let expected_head_revision_id =
                    optional_text(operation, "expected_head_revision_id").or_else(|| {
                        optional_text(payload, "expected_head_revision_id")
                    });
                let (
                    current_plan,
                    plan_record,
                    previous_revision_number,
                    current_head_revision_index,
                    current_head_revision_id,
                ) = {
                    let read = self.store.read_txn();
                    let current_plan = self.store.plan_meta_by_id_with_read(&read, &plan_id)?;
                    let (plan_record, _) = self
                        .store
                        .current_plan_record_with_read(&read, current_plan.plan_index)?;
                    let current_head_revision_index = plan_record
                        .latest_revision_index_plus1
                        .checked_sub(1)
                        .ok_or_else(|| format!("Plan {plan_id} has no head revision."))?;
                    let current_head_revision_id = server_revision_ref(current_head_revision_index);
                    let previous_revision = self
                        .store
                        .read_plan_revision_record_with_read(&read, current_head_revision_index)?;
                    if previous_revision.plan_index != current_plan.plan_index {
                        return Err(format!(
                            "plan_revision.bin[{current_head_revision_index}] belongs to plan {}, not plan {}",
                            previous_revision.plan_index, current_plan.plan_index
                        ));
                    }
                    (
                        current_plan,
                        plan_record,
                        previous_revision.revision_number,
                        current_head_revision_index,
                        current_head_revision_id,
                    )
                };
                let current_status = plan_status_from_record(&plan_record)?;
                let revision = self.prepare_task_start_revision(
                    payload,
                    plan_item_ref,
                    Some(current_plan.title.as_str()),
                    Some(current_status.as_str()),
                )?;
                if expected_head_revision_id.as_deref()
                    != Some(current_head_revision_id.as_str())
                {
                    if self.prepared_revision_matches_current(
                        current_plan.plan_index,
                        &plan_record,
                        current_head_revision_index,
                        &revision,
                    )? {
                        let item_index = self.current_open_item_index(
                            current_head_revision_index,
                            plan_item_ref,
                        )?;
                        return Ok(PreparedTaskStartPlan {
                            kind: PreparedTaskStartPlanKind::Existing {
                                plan_index: current_plan.plan_index,
                                plan_record,
                                revision_index: current_head_revision_index,
                                item_index,
                            },
                        });
                    }
                    return Err(format!(
                        "Task-start Plan {plan_id} expected_head_revision_id must equal current head {current_head_revision_id}."
                    ));
                }
                Ok(PreparedTaskStartPlan {
                    kind: PreparedTaskStartPlanKind::Revise {
                        plan_index: current_plan.plan_index,
                        plan_record,
                        previous_revision_number,
                        revision,
                    },
                })
            }
            "existing" => {
                let plan_id = required_text(operation, "plan_id")?;
                let plan_revision_id = required_text(operation, "plan_revision_id")?;
                let revision_index = parse_server_revision_ref(&plan_revision_id)?;
                let (plan_index, plan_record, item_index) = {
                    let read = self.store.read_txn();
                    let current_plan = self.store.plan_meta_by_id_with_read(&read, &plan_id)?;
                    let (plan_record, _) = self
                        .store
                        .current_plan_record_with_read(&read, current_plan.plan_index)?;
                    if plan_record.latest_revision_index_plus1.checked_sub(1)
                        != Some(revision_index)
                    {
                        return Err(format!(
                            "Task-start Plan {plan_id} head advanced: requested {plan_revision_id}, current head {}.",
                            plan_record
                                .latest_revision_index_plus1
                                .checked_sub(1)
                                .map(server_revision_ref)
                                .unwrap_or_else(|| "<none>".to_string())
                        ));
                    }
                    let revision = self
                        .store
                        .read_plan_revision_record_with_read(&read, revision_index)?;
                    if revision.plan_index != current_plan.plan_index {
                        return Err(format!(
                            "Plan revision {plan_revision_id} does not belong to {plan_id}."
                        ));
                    }
                    let revision_json = self
                        .store
                        .compact_revision_json_with_read(&read, revision_index, false)?;
                    let items = revision_json
                        .get("items")
                        .and_then(JsonValue::as_array)
                        .ok_or_else(|| {
                            format!("Plan revision {plan_revision_id} has no item authority.")
                        })?;
                    let item_offset = open_plan_item_offset(items, plan_item_ref)?;
                    let item_index = revision
                        .item_start_index
                        .checked_add(item_offset)
                        .ok_or_else(|| "Task-start Plan item index overflow.".to_string())?;
                    (current_plan.plan_index, plan_record, item_index)
                };
                Ok(PreparedTaskStartPlan {
                    kind: PreparedTaskStartPlanKind::Existing {
                        plan_index,
                        plan_record,
                        revision_index,
                        item_index,
                    },
                })
            }
            other => Err(format!(
                "Unsupported task-start Plan action {other:?}; expected create, revise, or existing."
            )),
        }
    }

    fn prepare_task_start_revision(
        &self,
        payload: &JsonMap<String, JsonValue>,
        plan_item_ref: &str,
        fallback_title: Option<&str>,
        fallback_status: Option<&str>,
    ) -> Result<PreparedRevision, String> {
        let artifact = normalize_plan_revision_artifact(payload)?;
        let artifact_path = required_text(&artifact, "artifact_path")?;
        let artifact_selector = optional_text(&artifact, "artifact_selector");
        let artifact_heading = required_text(&artifact, "artifact_heading")?;
        let items_value = artifact
            .get("items")
            .cloned()
            .unwrap_or_else(|| JsonValue::Array(Vec::new()));
        let items = normalized_plan_items(Some(&items_value))?;
        let item_offset = open_plan_item_offset(&items, plan_item_ref)?;
        let title = optional_text(payload, "title")
            .or_else(|| fallback_title.map(str::to_string))
            .ok_or_else(|| "Task-start Plan create requires title.".to_string())?;
        let requested_status = optional_text(payload, "status");
        let status = normalize_plan_status(requested_status.as_deref().or(fallback_status))?;
        let summary = optional_text(payload, "summary");
        let artifact_body = exact_optional_text(payload.get("artifact_body"))?;
        let packed_content = self.store.resolve_requested_packed_content(
            &artifact_path,
            artifact_body.as_deref(),
            payload.get("packed_artifact"),
        )?;
        Ok(PreparedRevision {
            title,
            status,
            artifact_path,
            artifact_selector,
            artifact_heading,
            items,
            summary,
            artifact_blob_id: packed_content
                .as_ref()
                .map(|content| content.blob_id.clone())
                .unwrap_or_default(),
            item_offset,
        })
    }

    fn exact_existing_task_start_create(
        &self,
        requested: &PreparedRevision,
        plan_item_ref: &str,
    ) -> Result<Option<PreparedTaskStartPlan>, String> {
        let read = self.store.read_txn();
        let mut matched = None;
        for meta in self.store.latest_plan_metas_with_read(&read)? {
            let (plan_record, _) = self
                .store
                .current_plan_record_with_read(&read, meta.plan_index)?;
            let Some(revision_index) = plan_record.latest_revision_index_plus1.checked_sub(1)
            else {
                continue;
            };
            let revision_record = self
                .store
                .read_plan_revision_record_with_read(&read, revision_index)?;
            let persisted = self
                .store
                .read_plan_revision_payload_with_read(&read, &revision_record)?;
            if persisted.artifact_path != requested.artifact_path
                || persisted.artifact_selector
                    != requested.artifact_selector.clone().unwrap_or_default()
            {
                continue;
            }
            let items = self
                .store
                .revision_items_with_read(&read, &revision_record)?;
            if !items.iter().any(|item| {
                item.get("plan_item_ref").and_then(JsonValue::as_str) == Some(plan_item_ref)
            }) {
                continue;
            }
            if !self.prepared_revision_matches_with_read(
                &read,
                meta.plan_index,
                &plan_record,
                revision_index,
                requested,
            )? {
                return Err(format!(
                    "Task-start Plan create conflicts with the current Plan for artifact {:?} and item {:?}.",
                    requested.artifact_path, plan_item_ref
                ));
            }
            let item_offset = open_plan_item_offset(&items, plan_item_ref)?;
            let item_index = revision_record
                .item_start_index
                .checked_add(item_offset)
                .ok_or_else(|| "Task-start Plan item index overflow.".to_string())?;
            if matched.is_some() {
                return Err(format!(
                    "Task-start Plan create is ambiguous: multiple exact current Plans contain artifact {:?} and item {:?}.",
                    requested.artifact_path, plan_item_ref
                ));
            }
            matched = Some(PreparedTaskStartPlan {
                kind: PreparedTaskStartPlanKind::Existing {
                    plan_index: meta.plan_index,
                    plan_record,
                    revision_index,
                    item_index,
                },
            });
        }
        Ok(matched)
    }

    fn current_open_item_index(
        &self,
        revision_index: u32,
        plan_item_ref: &str,
    ) -> Result<u32, String> {
        let read = self.store.read_txn();
        let revision = self
            .store
            .read_plan_revision_record_with_read(&read, revision_index)?;
        let items = self.store.revision_items_with_read(&read, &revision)?;
        revision
            .item_start_index
            .checked_add(open_plan_item_offset(&items, plan_item_ref)?)
            .ok_or_else(|| "Task-start Plan item index overflow.".to_string())
    }

    fn prepared_revision_matches_current(
        &self,
        plan_index: u32,
        plan_record: &PlanRecord,
        revision_index: u32,
        requested: &PreparedRevision,
    ) -> Result<bool, String> {
        let read = self.store.read_txn();
        self.prepared_revision_matches_with_read(
            &read,
            plan_index,
            plan_record,
            revision_index,
            requested,
        )
    }

    fn prepared_revision_matches_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        plan_index: u32,
        plan_record: &PlanRecord,
        revision_index: u32,
        requested: &PreparedRevision,
    ) -> Result<bool, String> {
        let (_, title) = self.store.current_plan_record_with_read(read, plan_index)?;
        let revision_record = self
            .store
            .read_plan_revision_record_with_read(read, revision_index)?;
        if revision_record.plan_index != plan_index {
            return Err(format!(
                "plan_revision.bin[{revision_index}] belongs to plan {}, not plan {plan_index}",
                revision_record.plan_index
            ));
        }
        let persisted = self
            .store
            .read_plan_revision_payload_with_read(read, &revision_record)?;
        let items = self
            .store
            .revision_items_with_read(read, &revision_record)?;
        Ok(title == requested.title
            && plan_record.plan_meta & PLAN_STATE_MASK == plan_meta_for_status(&requested.status)?
            && persisted.title_snapshot == requested.title
            && persisted.summary == requested.summary.clone().unwrap_or_default()
            && persisted.artifact_path == requested.artifact_path
            && persisted.artifact_selector
                == requested.artifact_selector.clone().unwrap_or_default()
            && persisted.artifact_heading == requested.artifact_heading
            && persisted.artifact_blob_id == requested.artifact_blob_id
            && items == requested.items)
    }

    pub(crate) fn begin_task_start_write(
        &self,
        prepared: &PreparedTaskStartPlan,
    ) -> Result<
        ServerPlanBinaryDbWriteTxn<'_, D, BinaryDbStoreFsyncPolicy<'_, D>, WRITE_LAYOUT>,
        String,
    > {
        ServerPlanBinaryDbWriteTxn::begin_task_start(self.db(), prepared.write_purpose())
    }

    pub(crate) fn apply_task_start_plan(
        &self,
        tx: &mut ServerPlanBinaryDbWriteTxn<'_, D, BinaryDbStoreFsyncPolicy<'_, D>, WRITE_LAYOUT>,
        prepared: &PreparedTaskStartPlan,
        now_s: u64,
    ) -> Result<TaskStartPlanBinding, String> {
        match &prepared.kind {
            PreparedTaskStartPlanKind::Create { revision } => {
                let plan_index = tx.record_count(plan_file())?;
                let revision_index = tx.record_count(plan_revision_file())?;
                let item_start_index = tx.record_count(plan_item_file())?;
                tx.append_items(&revision.items)?;
                let item_count = u16_len(revision.items.len(), "Binary DB plan item count")?;
                let committed_plan_index = tx.append_plan(
                    PlanRecord {
                        plan_meta: plan_meta_for_status(&revision.status)?,
                        reserved0: 0,
                        payload_len: 0,
                        payload_offset: 0,
                        latest_revision_index_plus1: revision_index_plus1(revision_index)?,
                        published_plan_index_plus1: 0,
                        published_latest_revision_index_plus1: 0,
                        created_at_s: now_s,
                        updated_at_s: now_s,
                        published_at_s: 0,
                    },
                    revision.title.as_bytes(),
                )?;
                if committed_plan_index != plan_index {
                    return Err(format!(
                        "Binary DB task-start expected Plan index {plan_index}, wrote {committed_plan_index}."
                    ));
                }
                let committed_revision_index = tx.append_plan_revision(
                    PlanRevisionRecord {
                        revision_meta: 0,
                        reserved0: 0,
                        payload_len: 0,
                        revision_number: 1,
                        item_count,
                        payload_offset: 0,
                        plan_index,
                        previous_revision_index_plus1: 0,
                        item_start_index,
                        published_revision_index_plus1: 0,
                        root_tree_pack_index_plus1: 0,
                        root_entry_ordinal: 0,
                        created_at_s: now_s,
                        published_at_s: 0,
                    },
                    &revision_payload(revision),
                )?;
                if committed_revision_index != revision_index {
                    return Err(format!(
                        "Binary DB task-start expected Plan revision index {revision_index}, wrote {committed_revision_index}."
                    ));
                }
                task_start_plan_binding(
                    "created",
                    plan_index,
                    revision_index,
                    item_start_index,
                    revision.item_offset,
                )
            }
            PreparedTaskStartPlanKind::Revise {
                plan_index,
                plan_record,
                previous_revision_number,
                revision,
            } => {
                tx.require_unchanged_plan(*plan_index, plan_record)?;
                let revision_index = tx.record_count(plan_revision_file())?;
                let item_start_index = tx.record_count(plan_item_file())?;
                tx.append_items(&revision.items)?;
                let item_count = u16_len(revision.items.len(), "Binary DB plan item count")?;
                let revision_number = previous_revision_number
                    .checked_add(1)
                    .ok_or_else(|| "Binary DB plan revision_number overflow.".to_string())?;
                let committed_revision_index = tx.append_plan_revision(
                    PlanRevisionRecord {
                        revision_meta: 0,
                        reserved0: 0,
                        payload_len: 0,
                        revision_number,
                        item_count,
                        payload_offset: 0,
                        plan_index: *plan_index,
                        previous_revision_index_plus1: plan_record.latest_revision_index_plus1,
                        item_start_index,
                        published_revision_index_plus1: 0,
                        root_tree_pack_index_plus1: 0,
                        root_entry_ordinal: 0,
                        created_at_s: now_s,
                        published_at_s: 0,
                    },
                    &revision_payload(revision),
                )?;
                if committed_revision_index != revision_index {
                    return Err(format!(
                        "Binary DB task-start expected Plan revision index {revision_index}, wrote {committed_revision_index}."
                    ));
                }
                let mut next_plan = plan_record.clone();
                next_plan.latest_revision_index_plus1 = revision_index_plus1(revision_index)?;
                next_plan.plan_meta = plan_meta_with_status(next_plan.plan_meta, &revision.status)?;
                next_plan.updated_at_s = now_s;
                tx.overwrite_plan(*plan_index, next_plan, revision.title.as_bytes())?;
                task_start_plan_binding(
                    "revised",
                    *plan_index,
                    revision_index,
                    item_start_index,
                    revision.item_offset,
                )
            }
            PreparedTaskStartPlanKind::Existing {
                plan_index,
                plan_record,
                revision_index,
                item_index,
            } => {
                tx.require_unchanged_plan(*plan_index, plan_record)?;
                Ok(TaskStartPlanBinding {
                    action: "existing",
                    plan_index: *plan_index,
                    revision_index: *revision_index,
                    item_index: *item_index,
                    plan_id: server_plan_ref(*plan_index),
                    plan_revision_id: server_revision_ref(*revision_index),
                })
            }
        }
    }
}

fn required_object_field<'a>(
    object: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    object
        .get(field)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Task-start Plan {field} must be a JSON object."))
}

fn open_plan_item_offset(items: &[JsonValue], plan_item_ref: &str) -> Result<u32, String> {
    let matching = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.get("plan_item_ref").and_then(JsonValue::as_str) == Some(plan_item_ref)
        })
        .collect::<Vec<_>>();
    let (offset, item) = match matching.as_slice() {
        [(offset, item)] => (*offset, *item),
        [] => {
            return Err(format!(
                "Task-start Plan does not contain item {plan_item_ref:?}."
            ))
        }
        _ => {
            return Err(format!(
                "Task-start Plan contains duplicate item ref {plan_item_ref:?}."
            ))
        }
    };
    if item.get("checkbox_state").and_then(JsonValue::as_str) != Some("open") {
        return Err(format!(
            "Task-start Plan item {plan_item_ref:?} is not open."
        ));
    }
    u32::try_from(offset).map_err(|_| "Task-start Plan item offset exceeds u32.".to_string())
}

fn revision_payload(revision: &PreparedRevision) -> PlanRevisionPayload {
    PlanRevisionPayload {
        title_snapshot: revision.title.clone(),
        summary: revision.summary.clone().unwrap_or_default(),
        artifact_path: revision.artifact_path.clone(),
        artifact_selector: revision.artifact_selector.clone().unwrap_or_default(),
        artifact_heading: revision.artifact_heading.clone(),
        artifact_blob_id: revision.artifact_blob_id.clone(),
    }
}

fn task_start_plan_binding(
    action: &'static str,
    plan_index: u32,
    revision_index: u32,
    item_start_index: u32,
    item_offset: u32,
) -> Result<TaskStartPlanBinding, String> {
    let item_index = item_start_index
        .checked_add(item_offset)
        .ok_or_else(|| "Task-start Plan item index overflow.".to_string())?;
    Ok(TaskStartPlanBinding {
        action,
        plan_index,
        revision_index,
        item_index,
        plan_id: server_plan_ref(plan_index),
        plan_revision_id: server_revision_ref(revision_index),
    })
}
