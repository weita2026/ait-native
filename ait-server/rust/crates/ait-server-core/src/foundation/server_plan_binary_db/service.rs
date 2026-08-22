use super::codec::{ServerPlanCodec, ServerPlanItemCodec, ServerPlanRevisionCodec};
use super::*;

#[cfg_attr(test, allow(dead_code))]
impl<D, const WRITE_LAYOUT: u32> BinaryDbServerPlanService<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub fn list_plans(
        &self,
        repo_name: &str,
        artifact_path: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.store.repo_scope("list_plans", repo_name)?;
        let read = self.store.read_txn();
        let artifact_path = clean_optional_text(artifact_path);
        let mut plans = match artifact_path {
            Some(ref artifact_path) => self
                .store
                .plan_metas_matching_artifact_path_with_read(&read, artifact_path)?,
            None => self.store.latest_plan_metas_with_read(&read)?,
        };
        plans.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.created_at.cmp(&left.created_at))
        });
        plans
            .into_iter()
            .map(|meta| self.store.plan_list_json_with_read(&read, &meta))
            .collect::<Result<Vec<_>, _>>()
            .map(JsonValue::Array)
    }

    pub fn create_plan(&self, repo_name: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store.repo_scope("create_plan", repo_name)?;
        let payload = payload
            .as_object()
            .ok_or_else(|| "plan create payload must be a JSON object.".to_string())?;
        if payload.contains_key("plan_id") {
            return Err(
                "Plan Binary DB create does not accept caller-supplied `plan_id`; identity is assigned as `PR-<index>`."
                    .to_string(),
            );
        }
        let title = required_text(payload, "title")?;
        let status = normalize_plan_status(optional_text(payload, "status").as_deref())?;
        let artifact = normalize_plan_revision_artifact(payload)?;
        let artifact_path = required_text(&artifact, "artifact_path")?;
        let artifact_selector = optional_text(&artifact, "artifact_selector");
        let artifact_heading = required_text(&artifact, "artifact_heading")?;
        let items_value = artifact
            .get("items")
            .cloned()
            .unwrap_or_else(|| JsonValue::Array(Vec::new()));
        let items = normalized_plan_items(Some(&items_value))?;
        let summary = optional_text(payload, "summary");
        let artifact_body = exact_optional_text(payload.get("artifact_body"))?;
        let packed_content = self.store.resolve_requested_packed_content(
            &artifact_path,
            artifact_body.as_deref(),
            payload.get("packed_artifact"),
        )?;
        let now = utc_now_string();
        let now_s = timestamp_s(&now)?;

        let plan_id = {
            let mut tx = self
                .store
                .begin_write(ServerPlanBinaryDbWritePurpose::CreatePlan)?;
            let plan_index = tx.record_count(plan_file())?;
            let revision_index = tx.record_count(plan_revision_file())?;
            let plan_id = server_plan_ref(plan_index);
            let plan_revision_id = server_revision_ref(revision_index);
            let blob = packed_content.as_ref().map(|content| {
                content.blob_json(
                    &plan_revision_id,
                    repo_name,
                    self.db().repo_id().as_str(),
                    &now,
                )
            });
            let item_start_index = tx.record_count(plan_item_file())?;
            tx.append_items(&items)?;
            let item_count = u16_len(items.len(), "Binary DB plan item count")?;
            let plan_record = PlanRecord {
                plan_meta: plan_meta_for_status(&status)?,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                latest_revision_index_plus1: revision_index_plus1(revision_index)?,
                published_plan_index_plus1: 0,
                published_latest_revision_index_plus1: 0,
                created_at_s: now_s,
                updated_at_s: now_s,
                published_at_s: 0,
            };
            let committed_plan_index = tx.append_plan(plan_record, title.as_bytes())?;
            if committed_plan_index != plan_index {
                return Err(format!(
                    "Binary DB plan expected plan index {plan_index}, wrote {committed_plan_index}"
                ));
            }
            let revision_record = PlanRevisionRecord {
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
            };
            let revision_payload = PlanRevisionPayload {
                title_snapshot: title.clone(),
                summary: summary.clone().unwrap_or_default(),
                artifact_path: artifact_path.clone(),
                artifact_selector: artifact_selector.clone().unwrap_or_default(),
                artifact_heading: artifact_heading.clone(),
                artifact_blob_id: blob
                    .as_ref()
                    .and_then(|blob| optional_text(blob, "blob_id"))
                    .unwrap_or_default(),
            };
            let committed_revision_index =
                tx.append_plan_revision(revision_record, &revision_payload)?;
            if committed_revision_index != revision_index {
                return Err(format!(
                    "Binary DB plan expected revision index {revision_index}, wrote {committed_revision_index}"
                ));
            }
            tx.set_commit_point(ServerPlanBinaryDbCommitPoint::PlanCreated {
                plan_index,
                revision_index,
            })?;
            tx.commit()?;
            plan_id
        };
        self.get_plan(&plan_id)
    }

    pub fn get_plan(&self, plan_id: &str) -> Result<JsonValue, String> {
        let read = self.store.read_txn();
        let meta = self.store.plan_meta_by_id_with_read(&read, plan_id)?;
        self.store.plan_detail_json_with_read(&read, &meta)
    }

    pub fn update_plan_status(
        &self,
        plan_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let payload = payload
            .as_object()
            .ok_or_else(|| "plan status payload must be a JSON object.".to_string())?;
        let status = normalize_plan_status(Some(required_text(payload, "status")?.as_str()))?;
        let now = utc_now_string();
        let now_s = timestamp_s(&now)?;
        let (current, record) = {
            let read = self.store.read_txn();
            let current = self.store.plan_meta_by_id_with_read(&read, plan_id)?;
            let (record, _) = self
                .store
                .current_plan_record_with_read(&read, current.plan_index)?;
            (current, record)
        };
        {
            let mut tx = self.store.begin_write_with_plan_cas(
                ServerPlanBinaryDbWritePurpose::UpdatePlanStatus,
                current.plan_index,
                &record,
            )?;
            let mut next = record;
            next.plan_meta = plan_meta_with_status(next.plan_meta, &status)?;
            next.updated_at_s = now_s;
            tx.overwrite_plan(current.plan_index, next, current.title.as_bytes())?;
            tx.set_commit_point(ServerPlanBinaryDbCommitPoint::PlanStatusUpdated {
                plan_index: current.plan_index,
            })?;
            tx.commit()?;
        }
        self.get_plan(plan_id)
    }

    pub fn list_plan_revisions(&self, plan_id: &str) -> Result<JsonValue, String> {
        let read = self.store.read_txn();
        let plan = self.store.plan_meta_by_id_with_read(&read, plan_id)?;
        let (record, _) = self
            .store
            .current_plan_record_with_read(&read, plan.plan_index)?;
        let mut out = Vec::new();
        let mut cursor = record.latest_revision_index_plus1;
        let revision_count = self.store.compact_record_count_with_read(
            &read,
            plan_revision_file(),
            CompactPlanFile::PlanRevision,
        )?;
        let mut walked = 0_u32;
        while cursor != 0 {
            if walked >= revision_count {
                return Err(format!(
                    "Binary DB plan {plan_id} revision chain exceeds plan_revision.bin record count"
                ));
            }
            let revision_index = cursor - 1;
            let revision_record = self
                .store
                .read_plan_revision_record_with_read(&read, revision_index)?;
            if revision_record.plan_index != plan.plan_index {
                return Err(format!(
                    "plan_revision.bin[{revision_index}] belongs to plan {}, not plan {}",
                    revision_record.plan_index, plan.plan_index
                ));
            }
            let revision =
                self.store
                    .compact_revision_json_with_read(&read, revision_index, false)?;
            cursor = revision_record.previous_revision_index_plus1;
            out.push(revision);
            walked += 1;
        }
        Ok(JsonValue::Array(out))
    }

    pub fn get_plan_revision(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        let read = self.store.read_txn_with_content();
        let plan = self.store.plan_meta_by_id_with_read(&read, plan_id)?;
        let revision_index = parse_server_revision_ref(plan_revision_id)?;
        let revision = self
            .store
            .read_plan_revision_record_with_read(&read, revision_index)?;
        if revision.plan_index != plan.plan_index {
            return Err(format!("Unknown plan revision: {plan_revision_id}"));
        }
        self.store
            .compact_revision_json_with_read(&read, revision_index, true)
    }

    pub(crate) fn task_binding_projection_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        revision_index: u32,
        item_index: u32,
    ) -> Result<(u32, String, Vec<String>), String> {
        let revision = self
            .store
            .read_plan_revision_record_with_read(read, revision_index)?;
        let _ = self
            .store
            .read_plan_meta_at_with_read(read, revision.plan_index)?;
        let item_offset = item_index
            .checked_sub(revision.item_start_index)
            .ok_or_else(|| {
                "Binary DB v0 Task Plan item precedes its bound revision range".to_string()
            })?;
        if item_offset >= u32::from(revision.item_count) {
            return Err(
                "Binary DB v0 Task Plan item is outside its bound revision range".to_string(),
            );
        }
        let item = self
            .store
            .read_plan_item_record_with_read(read, item_index)?;
        let payload = self.store.read_plan_item_payload_with_read(read, &item)?;
        let item_ref = payload.plan_item_ref.trim().to_string();
        if item_ref.is_empty() {
            return Err("Binary DB v0 Task Plan item has no plan_item_ref".to_string());
        }
        Ok((revision.plan_index, item_ref, payload.heading_path))
    }

    pub(crate) fn task_binding_projection_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        revision_index: u32,
        item_index: u32,
    ) -> Result<(u32, String, Vec<String>), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let revision = ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_record(
            &write
                .read_record(plan_revision_file(), revision_index)
                .map_err(binary_error)?,
        )?;
        let plan = ServerPlanCodec::<PLAN_LAYOUT_ID>::decode_record(
            &write
                .read_record(plan_file(), revision.plan_index)
                .map_err(binary_error)?,
        )?;
        let title = write
            .read_payload(
                plan_payload_file(),
                plan.payload_offset,
                u32::from(plan.payload_len),
            )
            .map_err(binary_error)?;
        let _ = ServerPlanCodec::<PLAN_LAYOUT_ID>::decode_title_payload(title)?;
        let _ = plan_status_from_record(&plan)?;
        let item_offset = item_index
            .checked_sub(revision.item_start_index)
            .ok_or_else(|| {
                "Binary DB v0 Task Plan item precedes its bound revision range".to_string()
            })?;
        if item_offset >= u32::from(revision.item_count) {
            return Err(
                "Binary DB v0 Task Plan item is outside its bound revision range".to_string(),
            );
        }
        let item = ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_record(
            &write
                .read_record(plan_item_file(), item_index)
                .map_err(binary_error)?,
        )?;
        let raw = write
            .read_payload(
                plan_item_payload_file(),
                item.payload_offset,
                u32::from(item.payload_len),
            )
            .map_err(binary_error)?;
        let payload = ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(&raw)?;
        let item_ref = payload.plan_item_ref.trim().to_string();
        if item_ref.is_empty() {
            return Err("Binary DB v0 Task Plan item has no plan_item_ref".to_string());
        }
        Ok((revision.plan_index, item_ref, payload.heading_path))
    }

    pub fn revise_plan(&self, plan_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let payload = payload
            .as_object()
            .ok_or_else(|| "plan revise payload must be a JSON object.".to_string())?;
        let artifact = normalize_plan_revision_artifact(payload)?;
        let artifact_path = required_text(&artifact, "artifact_path")?;
        let artifact_selector = optional_text(&artifact, "artifact_selector");
        let artifact_heading = required_text(&artifact, "artifact_heading")?;
        let items_value = artifact
            .get("items")
            .cloned()
            .unwrap_or_else(|| JsonValue::Array(Vec::new()));
        let items = normalized_plan_items(Some(&items_value))?;
        let title_override = optional_text(payload, "title");
        let summary = optional_text(payload, "summary");
        let artifact_body = exact_optional_text(payload.get("artifact_body"))?;
        let packed_content = self.store.resolve_requested_packed_content(
            &artifact_path,
            artifact_body.as_deref(),
            payload.get("packed_artifact"),
        )?;
        let expected_head_revision_id = optional_text(payload, "expected_head_revision_id");
        let now = utc_now_string();
        let now_s = timestamp_s(&now)?;
        let (
            current_plan,
            plan_record,
            current_head_revision_index,
            current_head_revision_id,
            previous_revision_number,
        ) = {
            let read = self.store.read_txn();
            let current_plan = self.store.plan_meta_by_id_with_read(&read, plan_id)?;
            let (plan_record, _) = self
                .store
                .current_plan_record_with_read(&read, current_plan.plan_index)?;
            let current_head_revision_index =
                plan_record.latest_revision_index_plus1.checked_sub(1);
            let current_head_revision_id = current_head_revision_index.map(server_revision_ref);
            let previous_revision_number = current_head_revision_index
                .map(|revision_index| {
                    let revision = self
                        .store
                        .read_plan_revision_record_with_read(&read, revision_index)?;
                    if revision.plan_index != current_plan.plan_index {
                        return Err(format!(
                            "plan_revision.bin[{revision_index}] belongs to plan {}, not plan {}",
                            revision.plan_index, current_plan.plan_index
                        ));
                    }
                    Ok(revision.revision_number)
                })
                .transpose()?
                .unwrap_or(0);
            (
                current_plan,
                plan_record,
                current_head_revision_index,
                current_head_revision_id,
                previous_revision_number,
            )
        };
        if let Some(expected) = expected_head_revision_id.as_deref() {
            if current_head_revision_id.as_deref() != Some(expected) {
                return Err(format!(
                    "Plan {plan_id} head advanced: expected {expected}, got {}",
                    current_head_revision_id.as_deref().unwrap_or("")
                ));
            }
        }
        let title = title_override
            .as_deref()
            .unwrap_or(current_plan.title.as_str())
            .to_string();
        let previous_revision_index_plus1 = current_head_revision_index
            .map(revision_index_plus1)
            .transpose()?
            .unwrap_or(0);
        let revision_number = previous_revision_number
            .checked_add(1)
            .ok_or_else(|| "Binary DB plan revision_number overflow".to_string())?;

        {
            let mut tx = self.store.begin_write_with_plan_cas(
                ServerPlanBinaryDbWritePurpose::RevisePlan,
                current_plan.plan_index,
                &plan_record,
            )?;
            let revision_index = tx.record_count(plan_revision_file())?;
            let plan_revision_id = server_revision_ref(revision_index);
            let item_start_index = tx.record_count(plan_item_file())?;
            tx.append_items(&items)?;
            let item_count = u16_len(items.len(), "Binary DB plan item count")?;
            let revision_record = PlanRevisionRecord {
                revision_meta: 0,
                reserved0: 0,
                payload_len: 0,
                revision_number,
                item_count,
                payload_offset: 0,
                plan_index: current_plan.plan_index,
                previous_revision_index_plus1,
                item_start_index,
                published_revision_index_plus1: 0,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                created_at_s: now_s,
                published_at_s: 0,
            };
            let blob = packed_content.as_ref().map(|content| {
                content.blob_json(
                    &plan_revision_id,
                    self.db().repo_name().as_str(),
                    current_plan.repo_id.as_str(),
                    &now,
                )
            });
            let revision_payload = PlanRevisionPayload {
                title_snapshot: title.clone(),
                summary: summary.clone().unwrap_or_default(),
                artifact_path: artifact_path.clone(),
                artifact_selector: artifact_selector.clone().unwrap_or_default(),
                artifact_heading: artifact_heading.clone(),
                artifact_blob_id: blob
                    .as_ref()
                    .and_then(|blob| optional_text(blob, "blob_id"))
                    .unwrap_or_default(),
            };
            let committed_revision_index =
                tx.append_plan_revision(revision_record, &revision_payload)?;
            if committed_revision_index != revision_index {
                return Err(format!(
                    "Binary DB plan expected revision index {revision_index}, wrote {committed_revision_index}"
                ));
            }
            let mut next_plan = plan_record;
            next_plan.latest_revision_index_plus1 = revision_index_plus1(revision_index)?;
            next_plan.updated_at_s = now_s;
            tx.overwrite_plan(current_plan.plan_index, next_plan, title.as_bytes())?;
            tx.set_commit_point(ServerPlanBinaryDbCommitPoint::PlanRevised {
                plan_index: current_plan.plan_index,
                revision_index,
            })?;
            tx.commit()?;
        }
        self.get_plan(plan_id)
    }

    pub fn put_plan_revision_artifacts(
        &self,
        _plan_id: &str,
        _plan_revision_id: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err(
            "supporting Plan artifact writes are not part of the compact layout-1 Plan schema; only the six schema-defined Plan files are permitted"
                .to_string(),
        )
    }
}

impl<D, const WRITE_LAYOUT: u32> BinaryDbServerPlanService<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + Clone,
{
    /// Returns every distinct packed artifact Blob referenced by the complete
    /// Plan revision inventory, including historical revisions that are no
    /// longer a Plan head. Offline bin-to-bin recovery uses this inventory to
    /// transfer Plan-owned content that is not necessarily reachable from a
    /// Snapshot Tree.
    pub fn referenced_artifact_blob_ids(&self) -> Result<Vec<String>, String> {
        let read = self.store.read_txn();
        let revision_count = self.store.compact_record_count_with_read(
            &read,
            plan_revision_file(),
            CompactPlanFile::PlanRevision,
        )?;
        let mut blob_ids = BTreeMap::<String, String>::new();
        for revision_index in 0..revision_count {
            let record = self
                .store
                .read_plan_revision_record_with_read(&read, revision_index)?;
            let payload = self
                .store
                .read_plan_revision_payload_with_read(&read, &record)?;
            let artifact_blob_id = payload.artifact_blob_id.trim();
            if !artifact_blob_id.is_empty() {
                blob_ids
                    .entry(artifact_blob_id.to_ascii_uppercase())
                    .or_insert_with(|| artifact_blob_id.to_string());
            }
        }
        Ok(blob_ids.into_values().collect())
    }

    pub fn audit_artifact_blob_closure(&self) -> Result<PlanArtifactBlobClosureAudit, String> {
        self.audit_artifact_blob_closure_matching(None)
    }

    fn audit_artifact_blob_closure_matching(
        &self,
        target_blob_id: Option<&str>,
    ) -> Result<PlanArtifactBlobClosureAudit, String> {
        let read = self.store.read_txn_with_content();
        let plan_count =
            self.store
                .compact_record_count_with_read(&read, plan_file(), CompactPlanFile::Plan)?;
        let revision_count = self.store.compact_record_count_with_read(
            &read,
            plan_revision_file(),
            CompactPlanFile::PlanRevision,
        )?;
        let mut referenced_revision_count = 0_u64;
        let mut unhealthy_revision_count = 0_u64;
        let mut blob_results = BTreeMap::<String, Result<(), String>>::new();
        let mut issues = Vec::new();
        let mut issues_truncated = false;

        for revision_index in 0..revision_count {
            let record = self
                .store
                .read_plan_revision_record_with_read(&read, revision_index)?;
            let payload = self
                .store
                .read_plan_revision_payload_with_read(&read, &record)?;
            let artifact_blob_id = payload.artifact_blob_id.trim();
            if artifact_blob_id.is_empty() {
                continue;
            }
            if target_blob_id.is_some_and(|target| !artifact_blob_id.eq_ignore_ascii_case(target)) {
                continue;
            }
            referenced_revision_count += 1;
            let normalized_blob_id = artifact_blob_id.to_ascii_uppercase();
            if !blob_results.contains_key(&normalized_blob_id) {
                let result = self
                    .store
                    .resolve_packed_content_blob_with_read(&read, artifact_blob_id)
                    .and_then(|resolved| {
                        if resolved.blob_id.eq_ignore_ascii_case(artifact_blob_id) {
                            Ok(())
                        } else {
                            Err(format!(
                                "Binary DB Plan artifact blob identity mismatch: revision={}, content={}",
                                artifact_blob_id, resolved.blob_id
                            ))
                        }
                    });
                blob_results.insert(normalized_blob_id.clone(), result);
            }
            if let Some(Err(error)) = blob_results.get(&normalized_blob_id) {
                unhealthy_revision_count += 1;
                if issues.len() < PLAN_ARTIFACT_CLOSURE_ISSUE_LIMIT {
                    issues.push(PlanArtifactBlobClosureIssue {
                        plan_id: server_plan_ref(record.plan_index),
                        plan_revision_id: server_revision_ref(revision_index),
                        artifact_path: payload.artifact_path,
                        artifact_blob_id: artifact_blob_id.to_string(),
                        error: bounded_integrity_error(error),
                    });
                } else {
                    issues_truncated = true;
                }
            }
        }

        let unhealthy_blob_count = blob_results
            .values()
            .filter(|result| result.is_err())
            .count() as u64;
        let referenced_blob_count = blob_results.len() as u64;
        Ok(PlanArtifactBlobClosureAudit {
            schema: "ait.server.plan_artifact_blob_closure.v1".to_string(),
            status: if unhealthy_blob_count == 0 {
                "complete".to_string()
            } else {
                "blocked".to_string()
            },
            plan_count: u64::from(plan_count),
            revision_count: u64::from(revision_count),
            referenced_revision_count,
            referenced_blob_count,
            healthy_blob_count: referenced_blob_count.saturating_sub(unhealthy_blob_count),
            unhealthy_blob_count,
            unhealthy_revision_count,
            issue_limit: PLAN_ARTIFACT_CLOSURE_ISSUE_LIMIT,
            issues_truncated,
            issues,
        })
    }
}

fn bounded_integrity_error(error: &str) -> String {
    const MAX_CHARS: usize = 512;
    let mut chars = error.chars();
    let bounded = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{bounded}...")
    } else {
        bounded
    }
}
