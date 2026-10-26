// Thin counterparts of the retained PyO3 JSON-request functions.
// Targets own parsing, domain validation and execution; no CLI transport lives here.
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

macro_rules! json_request_bindings {
    ($(($name:ident, $js_name:literal, $target:path)),* $(,)?) => {
        $(
            #[napi(js_name = $js_name)]
            pub fn $name(payload_json: String) -> Result<JsonValue> {
                $target(&payload_json).map_err(Error::from_reason)
            }
        )*
    };
}

json_request_bindings! {
    (plan_ports_normalize_plan_store_read_request, "planPortsNormalizePlanStoreReadRequest", ait_core::plan_ports_protocols::normalize_plan_store_read_request_payload_json),
    (validate_planning_session_join_payload, "validatePlanningSessionJoinPayload", ait_core::plan_http_contracts::validate_planning_session_join_payload_json),
    (plan_ports_normalize_plan_remote_transport, "planPortsNormalizePlanRemoteTransport", ait_core::plan_ports_protocols::normalize_plan_remote_transport_payload_json),
    (plan_ports_normalize_plan_remote_request, "planPortsNormalizePlanRemoteRequest", ait_core::plan_ports_protocols::normalize_plan_remote_request_payload_json),
    (plan_ports_normalize_artifact_resolver_request, "planPortsNormalizeArtifactResolverRequest", ait_core::plan_ports_protocols::normalize_artifact_resolver_request_payload_json),
    (plan_ports_normalize_artifact_publish_request, "planPortsNormalizeArtifactPublishRequest", ait_core::plan_ports_protocols::normalize_artifact_publish_request_payload_json),
    (plan_ports_normalize_linked_task_lookup_payload, "planPortsNormalizeLinkedTaskLookupPayload", ait_core::plan_ports_protocols::normalize_linked_task_lookup_payload_json),
    (plan_ports_normalize_plan_config_runtime_facts, "planPortsNormalizePlanConfigRuntimeFacts", ait_core::plan_ports_protocols::normalize_plan_config_runtime_facts_payload_json),
    (plan_ports_normalize_plan_connection_manager_stats, "planPortsNormalizePlanConnectionManagerStats", ait_core::plan_ports_protocols::normalize_plan_connection_manager_stats_payload_json),
    (plan_config_runtime_normalize_selection_request, "planConfigRuntimeNormalizeSelectionRequest", ait_core::config_runtime::normalize_plan_runtime_selection_request_payload_json),
    (plan_config_runtime_build_selection_facts, "planConfigRuntimeBuildSelectionFacts", ait_core::config_runtime::build_plan_runtime_selection_facts_json),
    (plan_config_runtime_normalize_selection_facts, "planConfigRuntimeNormalizeSelectionFacts", ait_core::config_runtime::normalize_plan_runtime_selection_facts_payload_json),
    (plan_config_runtime_normalize_compatibility, "planConfigRuntimeNormalizeCompatibility", ait_core::config_runtime::normalize_plan_runtime_compatibility_payload_json),
    (plan_config_runtime_normalize_readiness, "planConfigRuntimeNormalizeReadiness", ait_core::config_runtime::normalize_plan_runtime_readiness_payload_json),
    (plan_config_runtime_normalize_doctor, "planConfigRuntimeNormalizeDoctor", ait_core::config_runtime::normalize_plan_runtime_doctor_payload_json),
    (plan_diagnostics_normalize_request, "planDiagnosticsNormalizeRequest", ait_core::diagnostics::normalize_plan_diagnostics_request_payload_json),
    (plan_diagnostics_normalize_backend_identity, "planDiagnosticsNormalizeBackendIdentity", ait_core::diagnostics::normalize_plan_backend_identity_payload_json),
    (plan_diagnostics_normalize_compatibility, "planDiagnosticsNormalizeCompatibility", ait_core::diagnostics::normalize_plan_diagnostics_compatibility_payload_json),
    (plan_diagnostics_normalize_readiness, "planDiagnosticsNormalizeReadiness", ait_core::diagnostics::normalize_plan_diagnostics_readiness_payload_json),
    (plan_diagnostics_normalize_doctor, "planDiagnosticsNormalizeDoctor", ait_core::diagnostics::normalize_plan_diagnostics_doctor_payload_json),
    (plan_diagnostics_build_backend_identity_facts, "planDiagnosticsBuildBackendIdentityFacts", ait_core::diagnostics::build_plan_backend_identity_facts_json),
    (plan_diagnostics_build_storage_readiness_facts, "planDiagnosticsBuildStorageReadinessFacts", ait_core::diagnostics::build_plan_storage_readiness_facts_json),
    (plan_diagnostics_build_compatibility_status, "planDiagnosticsBuildCompatibilityStatus", ait_core::diagnostics::build_plan_diagnostics_compatibility_status_json),
    (plan_diagnostics_build_readiness_status, "planDiagnosticsBuildReadinessStatus", ait_core::diagnostics::build_plan_diagnostics_readiness_status_json),
    (plan_diagnostics_build_doctor_facts, "planDiagnosticsBuildDoctorFacts", ait_core::diagnostics::build_plan_diagnostics_doctor_facts_json),
    (plan_application_normalize_list_request, "planApplicationNormalizeListRequest", ait_core::plan_application::normalize_plan_list_service_request_payload_json),
    (plan_application_build_list, "planApplicationBuildList", ait_core::plan_application::build_plan_list_service_payload_json),
    (plan_application_normalize_show_request, "planApplicationNormalizeShowRequest", ait_core::plan_application::normalize_plan_show_service_request_payload_json),
    (plan_application_build_show, "planApplicationBuildShow", ait_core::plan_application::build_plan_show_service_payload_json),
    (plan_application_normalize_revisions_request, "planApplicationNormalizeRevisionsRequest", ait_core::plan_application::normalize_plan_revisions_service_request_payload_json),
    (plan_application_build_revisions, "planApplicationBuildRevisions", ait_core::plan_application::build_plan_revisions_service_payload_json),
    (plan_application_normalize_items_request, "planApplicationNormalizeItemsRequest", ait_core::plan_application::normalize_plan_items_service_request_payload_json),
    (plan_application_build_items, "planApplicationBuildItems", ait_core::plan_application::build_plan_items_service_payload_json),
    (plan_application_normalize_candidates_request, "planApplicationNormalizeCandidatesRequest", ait_core::plan_application::normalize_plan_candidates_service_request_payload_json),
    (plan_application_build_candidates, "planApplicationBuildCandidates", ait_core::plan_application::build_plan_candidates_service_payload_json),
    (plan_application_normalize_inspect_request, "planApplicationNormalizeInspectRequest", ait_core::plan_application::normalize_plan_inspect_service_request_payload_json),
    (plan_application_build_inspect, "planApplicationBuildInspect", ait_core::plan_application::build_plan_inspect_service_payload_json),
    (plan_application_normalize_sync_request, "planApplicationNormalizeSyncRequest", ait_core::plan_application::normalize_plan_sync_service_request_payload_json),
    (plan_application_build_sync, "planApplicationBuildSync", ait_core::plan_application::build_plan_sync_service_payload_json),
    (plan_command_normalize_list_request, "planCommandNormalizeListRequest", ait_core::plan_command::normalize_plan_list_command_request_payload_json),
    (plan_command_build_list, "planCommandBuildList", ait_core::plan_command::build_plan_list_command_payload_json),
    (plan_command_normalize_show_request, "planCommandNormalizeShowRequest", ait_core::plan_command::normalize_plan_show_command_request_payload_json),
    (plan_command_build_show, "planCommandBuildShow", ait_core::plan_command::build_plan_show_command_payload_json),
    (plan_command_normalize_revisions_request, "planCommandNormalizeRevisionsRequest", ait_core::plan_command::normalize_plan_revisions_command_request_payload_json),
    (plan_command_build_revisions, "planCommandBuildRevisions", ait_core::plan_command::build_plan_revisions_command_payload_json),
    (plan_command_normalize_items_request, "planCommandNormalizeItemsRequest", ait_core::plan_command::normalize_plan_items_command_request_payload_json),
    (plan_command_build_items, "planCommandBuildItems", ait_core::plan_command::build_plan_items_command_payload_json),
    (plan_command_normalize_candidates_request, "planCommandNormalizeCandidatesRequest", ait_core::plan_command::normalize_plan_candidates_command_request_payload_json),
    (plan_command_build_candidates, "planCommandBuildCandidates", ait_core::plan_command::build_plan_candidates_command_payload_json),
    (plan_command_normalize_inspect_request, "planCommandNormalizeInspectRequest", ait_core::plan_command::normalize_plan_inspect_command_request_payload_json),
    (plan_command_build_inspect, "planCommandBuildInspect", ait_core::plan_command::build_plan_inspect_command_payload_json),
    (plan_command_normalize_sync_request, "planCommandNormalizeSyncRequest", ait_core::plan_command::normalize_plan_sync_command_request_payload_json),
    (plan_command_build_sync, "planCommandBuildSync", ait_core::plan_command::build_plan_sync_command_payload_json),
    (plan_command_execute_list, "planCommandExecuteList", ait_core::plan_command_execution::execute_plan_list_command_request_json),
    (plan_command_execute_show, "planCommandExecuteShow", ait_core::plan_command_execution::execute_plan_show_command_request_json),
    (plan_command_execute_revisions, "planCommandExecuteRevisions", ait_core::plan_command_execution::execute_plan_revisions_command_request_json),
    (plan_command_execute_items, "planCommandExecuteItems", ait_core::plan_command_execution::execute_plan_items_command_request_json),
    (plan_command_execute_candidates, "planCommandExecuteCandidates", ait_core::plan_command_execution::execute_plan_candidates_command_request_json),
    (plan_command_execute_inspect, "planCommandExecuteInspect", ait_core::plan_command_execution::execute_plan_inspect_command_request_json),
    (plan_command_execute_sync, "planCommandExecuteSync", ait_core::plan_sync_execution::execute_plan_sync_command_request_json),
    (plan_provenance_normalize_revision_provenance, "planProvenanceNormalizeRevisionProvenance", ait_core::plan_provenance::normalize_plan_revision_provenance_payload_json),
    (plan_provenance_build_revision_provenance, "planProvenanceBuildRevisionProvenance", ait_core::plan_provenance::build_plan_revision_provenance_payload_json),
    (plan_time_identity_normalize_timestamp_request, "planTimeIdentityNormalizeTimestampRequest", ait_core::time_identity::normalize_plan_timestamp_request_payload_json),
    (plan_time_identity_normalize_timestamp, "planTimeIdentityNormalizeTimestamp", ait_core::time_identity::normalize_plan_timestamp_payload_json),
    (plan_time_identity_build_timestamp, "planTimeIdentityBuildTimestamp", ait_core::time_identity::build_plan_timestamp_payload_json),
    (plan_time_identity_normalize_sequence_identity_request, "planTimeIdentityNormalizeSequenceIdentityRequest", ait_core::time_identity::normalize_plan_sequence_identity_request_payload_json),
    (plan_time_identity_normalize_sequence_identity, "planTimeIdentityNormalizeSequenceIdentity", ait_core::time_identity::normalize_plan_sequence_identity_payload_json),
    (plan_time_identity_build_sequence_identity, "planTimeIdentityBuildSequenceIdentity", ait_core::time_identity::build_plan_sequence_identity_payload_json),
    (plan_time_identity_normalize_workflow_id_request, "planTimeIdentityNormalizeWorkflowIdRequest", ait_core::time_identity::normalize_plan_workflow_id_request_payload_json),
    (plan_time_identity_normalize_workflow_id, "planTimeIdentityNormalizeWorkflowId", ait_core::time_identity::normalize_plan_workflow_id_payload_json),
    (plan_time_identity_build_workflow_id, "planTimeIdentityBuildWorkflowId", ait_core::time_identity::build_plan_workflow_id_payload_json),
    (plan_time_identity_normalize_temporal_ordering, "planTimeIdentityNormalizeTemporalOrdering", ait_core::time_identity::normalize_plan_temporal_ordering_payload_json),
    (plan_time_identity_build_temporal_ordering, "planTimeIdentityBuildTemporalOrdering", ait_core::time_identity::build_plan_temporal_ordering_payload_json),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_timestamp_domain_api_and_invalid_request_errors_are_preserved() {
        let timestamp = plan_time_identity_build_timestamp("{}".into()).expect("native timestamp");
        assert!(timestamp["timestamp"].as_str().is_some());
        let malformed =
            plan_time_identity_build_timestamp("{".into()).expect_err("malformed request");
        assert!(!malformed.reason.is_empty());
        let argv = plan_time_identity_build_timestamp(r#"["status","--json"]"#.into())
            .expect_err("a domain request is not CLI argv");
        assert!(!argv.reason.is_empty());
    }
}
