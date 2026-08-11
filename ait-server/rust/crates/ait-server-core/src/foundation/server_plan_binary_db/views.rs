use super::*;

#[cfg_attr(test, allow(dead_code))]
impl<D, const WRITE_LAYOUT: u32> ServerPlanBinaryDbStore<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + Clone,
{
    #[cfg(test)]
    pub(super) fn plan_detail_json(&self, meta: &ServerPlanMeta) -> Result<JsonValue, String> {
        let read = self.read_txn();
        self.plan_detail_json_with_read(&read, meta)
    }

    pub(super) fn plan_detail_json_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        meta: &ServerPlanMeta,
    ) -> Result<JsonValue, String> {
        let (record, title) = self.current_plan_record_with_read(read, meta.plan_index)?;
        let head_revision_index = record.latest_revision_index_plus1.checked_sub(1);
        let head_revision_id = head_revision_index.map(server_revision_ref);
        let head_revision = if let Some(revision_index) = head_revision_index {
            self.compact_revision_json_with_read(read, revision_index, false)?
        } else {
            JsonValue::Null
        };
        Ok(json!({
            "plan_id": server_plan_ref(meta.plan_index),
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": meta.repo_id,
            "title": title,
            "status": meta.status,
            "head_revision_id": head_revision_id,
            "head_revision": head_revision,
            "created_by": meta.created_by,
            "created_at": meta.created_at,
            "updated_at": meta.updated_at,
        }))
    }

    pub(super) fn compact_revision_json_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        revision_index: u32,
        include_body: bool,
    ) -> Result<JsonValue, String> {
        let record = self.read_plan_revision_record_with_read(read, revision_index)?;
        let payload = self.read_plan_revision_payload_with_read(read, &record)?;
        let compact_items = self.revision_items_with_read(read, &record)?;
        let plan_revision_id = server_revision_ref(revision_index);
        let plan_id = server_plan_ref(record.plan_index);
        let created_at = timestamp_string(record.created_at_s)?;
        let parent_plan_revision_id = record
            .previous_revision_index_plus1
            .checked_sub(1)
            .map(server_revision_ref);
        let items_json = serde_json::to_string(&compact_items).map_err(|err| err.to_string())?;

        let packed_content = if include_body && !payload.artifact_blob_id.is_empty() {
            Some(self.resolve_packed_content_blob_with_read(read, &payload.artifact_blob_id)?)
        } else {
            None
        };
        let artifact_body = packed_content.as_ref().map(|content| content.body.clone());
        let blob = packed_content.as_ref().map(|content| {
            content.blob_json(
                &plan_revision_id,
                self.db.repo_name().as_str(),
                self.db.repo_id().as_str(),
                &created_at,
            )
        });
        if let Some(blob) = &blob {
            let canonical_blob_id = optional_text(blob, "blob_id").unwrap_or_default();
            if canonical_blob_id != payload.artifact_blob_id {
                return Err(format!(
                    "plan_revision.bin[{revision_index}] blob identity mismatch: compact={}, content={canonical_blob_id}",
                    payload.artifact_blob_id
                ));
            }
        }

        let row = JsonMap::from_iter([
            ("plan_revision_id".to_string(), json!(plan_revision_id)),
            ("plan_id".to_string(), json!(plan_id)),
            ("revision_number".to_string(), json!(record.revision_number)),
            (
                "parent_plan_revision_id".to_string(),
                json!(parent_plan_revision_id),
            ),
            ("title_snapshot".to_string(), json!(payload.title_snapshot)),
            (
                "summary".to_string(),
                clean_optional_text(Some(payload.summary.as_str()))
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "artifact_path".to_string(),
                clean_optional_text(Some(payload.artifact_path.as_str()))
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "artifact_selector".to_string(),
                clean_optional_text(Some(payload.artifact_selector.as_str()))
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "artifact_heading".to_string(),
                json!(payload.artifact_heading),
            ),
            (
                "artifact_blob_id".to_string(),
                clean_optional_text(Some(payload.artifact_blob_id.as_str()))
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            ("items_json".to_string(), json!(items_json)),
            ("plan_links_surface_hash".to_string(), JsonValue::Null),
            ("plan_links_changed_count_to_prev".to_string(), json!(0)),
            ("source_kind".to_string(), json!("plan_sync")),
            ("created_by".to_string(), JsonValue::Null),
            ("actor_type".to_string(), json!("repository")),
            ("created_at".to_string(), json!(created_at)),
        ]);
        let mut view = plan_revision_view(
            &row,
            blob.as_ref(),
            Some(Vec::new()),
            artifact_body,
            PlanRevisionViewOptions {
                include_artifact_body: include_body,
                preserve_items_json: true,
                include_blob_object: include_body,
            },
        )?;
        view.insert("items".to_string(), JsonValue::Array(compact_items));
        Ok(JsonValue::Object(view))
    }

    #[cfg(test)]
    pub(super) fn plan_list_json(&self, meta: &ServerPlanMeta) -> Result<JsonValue, String> {
        let read = self.read_txn();
        self.plan_list_json_with_read(&read, meta)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn plan_list_json_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        meta: &ServerPlanMeta,
    ) -> Result<JsonValue, String> {
        let (record, title) = self.current_plan_record_with_read(read, meta.plan_index)?;
        let head_revision_index = record.latest_revision_index_plus1.checked_sub(1);
        let head_revision_id = head_revision_index.map(server_revision_ref);
        let head_payload = head_revision_index
            .map(|revision_index| {
                let record = self.read_plan_revision_record_with_read(read, revision_index)?;
                if record.plan_index != meta.plan_index {
                    return Err(format!(
                        "plan_revision.bin[{revision_index}] belongs to plan {}, not plan {}",
                        record.plan_index, meta.plan_index
                    ));
                }
                let payload = self.read_plan_revision_payload_with_read(read, &record)?;
                Ok::<_, String>((record, payload))
            })
            .transpose()?;
        let head_revision_items = head_payload
            .as_ref()
            .map(|(record, _)| self.revision_items_with_read(read, record))
            .transpose()?
            .unwrap_or_default();
        let head_revision_number = head_payload
            .as_ref()
            .map(|(record, _)| i64::from(record.revision_number));
        let head_artifact_selector = head_payload
            .as_ref()
            .and_then(|(_, payload)| clean_optional_text(Some(payload.artifact_selector.as_str())));
        let head_artifact_path = head_payload
            .as_ref()
            .and_then(|(_, payload)| clean_optional_text(Some(payload.artifact_path.as_str())));
        let head_artifact_heading = head_payload
            .as_ref()
            .map(|(_, payload)| payload.artifact_heading.clone());
        let head_artifact_blob_id = head_payload
            .as_ref()
            .and_then(|(_, payload)| clean_optional_text(Some(payload.artifact_blob_id.as_str())));
        let head_revision_summary = head_payload
            .as_ref()
            .and_then(|(_, payload)| clean_optional_text(Some(payload.summary.as_str())));
        let head_revision_created_at = head_payload
            .as_ref()
            .map(|(record, _)| timestamp_string(record.created_at_s))
            .transpose()?;
        let head_revision_items_json = if head_payload.is_some() {
            Some(serde_json::to_string(&head_revision_items).map_err(|err| err.to_string())?)
        } else {
            None
        };
        Ok(json!({
            "plan_id": server_plan_ref(meta.plan_index),
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": meta.repo_id,
            "title": title,
            "status": meta.status,
            "head_revision_id": head_revision_id,
            "created_by": meta.created_by,
            "created_at": meta.created_at,
            "updated_at": meta.updated_at,
            "head_revision_number": head_revision_number,
            "head_artifact_selector": head_artifact_selector,
            "head_artifact_path": head_artifact_path,
            "head_artifact_heading": head_artifact_heading,
            "head_artifact_blob_id": head_artifact_blob_id,
            "head_revision_items_json": head_revision_items_json,
            "head_revision_summary": head_revision_summary,
            "head_revision_created_at": head_revision_created_at,
            "head_revision": {
                "plan_revision_id": head_revision_id,
                "revision_number": head_revision_number,
                "artifact_path": head_artifact_path,
                "artifact_selector": head_artifact_selector,
                "artifact_heading": head_artifact_heading,
                "artifact_blob_id": head_artifact_blob_id,
                "items": JsonValue::Array(head_revision_items),
                "summary": head_payload.as_ref().and_then(|(_, payload)| clean_optional_text(Some(payload.summary.as_str()))),
                "created_at": head_revision_created_at,
            },
        }))
    }
}
