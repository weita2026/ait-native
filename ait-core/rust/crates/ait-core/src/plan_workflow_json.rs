use crate::json_support::{json, JsonMap, JsonValue};

use crate::{
    change_json::ChangeJson, json_support::JsonCodec, plan_application, plan_command,
    plan_http_client::PlanHttpRequestSpec, plan_ports_protocols, plan_provenance,
    task_json::TaskJson,
};

pub struct PlanWorkflowJson<S> {
    store: S,
}

impl<S> PlanWorkflowJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl PlanWorkflowJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> PlanWorkflowJson<S> {
    pub fn normalize_plan_store_read_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan store read request")?;
        plan_ports_protocols::normalize_plan_store_read_request_payload_map(payload)
    }

    pub fn normalize_plan_remote_transport_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan remote transport")?;
        plan_ports_protocols::normalize_plan_remote_transport_payload_object(payload)
    }

    pub fn normalize_plan_remote_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan remote request")?;
        plan_ports_protocols::normalize_plan_remote_request_payload_map(payload)
    }

    pub fn normalize_artifact_resolver_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "artifact resolver request")?;
        plan_ports_protocols::normalize_artifact_resolver_request_payload_map(payload)
    }

    pub fn normalize_artifact_publish_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "artifact publish request")?;
        plan_ports_protocols::normalize_artifact_publish_request_payload_map(payload)
    }

    pub fn normalize_linked_task_lookup_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        TaskJson::stateless().normalize_linked_task_lookup_payload_json(payload_json)
    }

    pub fn normalize_plan_config_runtime_facts_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan config/runtime facts payload")?;
        plan_ports_protocols::normalize_plan_config_runtime_facts_payload_map(payload)
    }

    pub fn normalize_plan_connection_manager_stats_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan connection manager stats payload")?;
        plan_ports_protocols::normalize_plan_connection_manager_stats_payload_map(payload)
    }

    pub fn build_linked_task_lookup_payload(
        &self,
        task_links_by_item_rows: Option<&JsonValue>,
        tasks_by_plan_rows: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        TaskJson::stateless()
            .build_linked_task_lookup_payload(task_links_by_item_rows, tasks_by_plan_rows)
    }

    pub fn normalize_linked_change_lookup_payload(
        &self,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        ChangeJson::stateless().normalize_linked_change_lookup_payload(payload)
    }

    pub fn build_linked_change_lookup_payload(
        &self,
        change_links_by_task_rows: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        ChangeJson::stateless().build_linked_change_lookup_payload(change_links_by_task_rows)
    }

    pub fn build_task_tracking_title_payload(&self, task: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        TaskJson::stateless().build_task_tracking_title_payload(task)
    }

    pub fn build_task_tracking_metadata_payload(
        &self,
        task: &JsonValue,
        author_mode: &str,
        tracking_policy: &str,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        TaskJson::stateless().build_task_tracking_metadata_payload(
            task,
            author_mode,
            tracking_policy,
        )
    }

    pub fn normalize_plan_revision_provenance_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan revision provenance")?;
        plan_provenance::normalize_plan_revision_provenance_payload_map(payload)
    }

    pub fn build_plan_revision_provenance_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan revision provenance build request")?;
        plan_provenance::build_plan_revision_provenance_payload_map(payload)
    }

    pub fn normalize_plan_list_service_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan list service request")?;
        plan_application::normalize_plan_list_service_request_payload_map(payload)
    }

    pub fn build_plan_list_service_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan list service payload")?;
        plan_application::build_plan_list_service_payload_map(payload)
    }

    pub fn normalize_plan_show_service_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan show service request")?;
        plan_application::normalize_plan_show_service_request_payload_map(payload)
    }

    pub fn build_plan_show_service_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan show service payload")?;
        plan_application::build_plan_show_service_payload_map(payload)
    }

    pub fn normalize_plan_revisions_service_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan revisions service request")?;
        plan_application::normalize_plan_revisions_service_request_payload_map(payload)
    }

    pub fn build_plan_revisions_service_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan revisions service payload")?;
        plan_application::build_plan_revisions_service_payload_map(payload)
    }

    pub fn normalize_plan_items_service_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan items service request")?;
        plan_application::normalize_plan_items_service_request_payload_map(payload)
    }

    pub fn build_plan_items_service_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan items service payload")?;
        plan_application::build_plan_items_service_payload_map(payload)
    }

    pub fn normalize_plan_candidates_service_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan candidates service request")?;
        plan_application::normalize_plan_candidates_service_request_payload_map(payload)
    }

    pub fn build_plan_candidates_service_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan candidates service payload")?;
        plan_application::build_plan_candidates_service_payload_map(payload)
    }

    pub fn normalize_plan_inspect_service_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan inspect service request")?;
        plan_application::normalize_plan_inspect_service_request_payload_map(payload)
    }

    pub fn build_plan_inspect_service_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan inspect service payload")?;
        plan_application::build_plan_inspect_service_payload_map(payload)
    }

    pub fn normalize_plan_sync_service_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan sync service request")?;
        plan_application::normalize_plan_sync_service_request_payload_map(payload)
    }

    pub fn build_plan_sync_service_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan sync service payload")?;
        plan_application::build_plan_sync_service_payload_map(payload)
    }

    pub fn normalize_plan_list_command_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan list command request")?;
        plan_command::normalize_plan_list_command_request_payload_map(payload)
    }

    pub fn build_plan_list_command_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan list command payload")?;
        plan_command::build_plan_list_command_payload_map(payload)
    }

    pub fn normalize_plan_show_command_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan show command request")?;
        plan_command::normalize_plan_show_command_request_payload_map(payload)
    }

    pub fn build_plan_show_command_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan show command payload")?;
        plan_command::build_plan_show_command_payload_map(payload)
    }

    pub fn normalize_plan_revisions_command_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan revisions command request")?;
        plan_command::normalize_plan_revisions_command_request_payload_map(payload)
    }

    pub fn build_plan_revisions_command_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan revisions command payload")?;
        plan_command::build_plan_revisions_command_payload_map(payload)
    }

    pub fn normalize_plan_items_command_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan items command request")?;
        plan_command::normalize_plan_items_command_request_payload_map(payload)
    }

    pub fn build_plan_items_command_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan items command payload")?;
        plan_command::build_plan_items_command_payload_map(payload)
    }

    pub fn normalize_plan_candidates_command_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan candidates command request")?;
        plan_command::normalize_plan_candidates_command_request_payload_map(payload)
    }

    pub fn build_plan_candidates_command_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan candidates command payload")?;
        plan_command::build_plan_candidates_command_payload_map(payload)
    }

    pub fn normalize_plan_inspect_command_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan inspect command request")?;
        plan_command::normalize_plan_inspect_command_request_payload_map(payload)
    }

    pub fn build_plan_inspect_command_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan inspect command payload")?;
        plan_command::build_plan_inspect_command_payload_map(payload)
    }

    pub fn normalize_plan_sync_command_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan sync command request")?;
        plan_command::normalize_plan_sync_command_request_payload_map(payload)
    }

    pub fn build_plan_sync_command_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan sync command payload")?;
        plan_command::build_plan_sync_command_payload_map(payload)
    }

    pub fn task_workflow_http_request_spec_payload(&self, spec: &PlanHttpRequestSpec) -> JsonValue {
        let _ = &self.store;
        plan_http_request_spec_payload(spec)
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        JsonCodec::parse_object_with_error_prefix(
            payload_json,
            &format!("Failed to parse {label} JSON"),
            &format!("{label} payload must decode to an object."),
        )
        .map_err(String::from)
    }
}

fn plan_http_request_spec_payload(spec: &PlanHttpRequestSpec) -> JsonValue {
    json!({
        "method": spec.method.as_str(),
        "path": spec.path.as_str(),
        "url": spec.url.as_str(),
        "query_pairs": spec.query_pairs.iter().map(|(name, value)| {
            json!({
                "name": name,
                "value": value,
            })
        }).collect::<Vec<_>>(),
        "headers": &spec.headers,
        "body": spec.body.clone().unwrap_or(JsonValue::Null),
        "timeout_ms": spec.timeout_ms,
    })
}

#[cfg(test)]
mod tests;
