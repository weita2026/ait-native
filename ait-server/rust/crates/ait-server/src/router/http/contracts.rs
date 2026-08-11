use super::*;

#[derive(Debug, Serialize)]
struct HandshakeResponse {
    ready: bool,
    contract_version: String,
    package_version: String,
    authority_backend: &'static str,
    repository_identity: &'static str,
    ci_capabilities: JsonValue,
    operational_capabilities: JsonValue,
    supported_async_job_types: Vec<String>,
    endpoints: Vec<String>,
}

pub(super) async fn health(State(state): State<ServerState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "ready": true,
            "authority_backend": "binary_v0",
            "repository_identity": "repository_index",
            "operational_capabilities": state.operational_binary.capabilities()["operational_capabilities"].clone(),
        })),
    )
}

pub(super) async fn handshake(State(state): State<ServerState>) -> impl IntoResponse {
    let operational_capabilities =
        state.operational_binary.capabilities()["operational_capabilities"].clone();
    let mut ci_capabilities = RemoteSyncPlanJson::stateless().capabilities_payload();
    ci_capabilities["native_runner"] = json!({
        "contract": NATIVE_JOB_V3_CONTRACT,
        "result_contract": "ait.runner.native-result.v1",
        "queue_contract": "ait.server.worker-job.service.v1",
        "claim_filtering": true,
        "lease_heartbeat": true,
        "remote_snapshot_source": true,
        "repository_identity": "binary-repository-index.v0",
        "repository_entrypoint": "ci/run",
        "platform_entrypoints": {
            "darwin": NATIVE_JOB_REPOSITORY_CI_UNIX_PATH,
            "linux": NATIVE_JOB_REPOSITORY_CI_UNIX_PATH,
            "windows": NATIVE_JOB_REPOSITORY_CI_WINDOWS_PATH,
        },
        "command_execution": "direct_argv_without_command_string_concatenation",
        "docker_required": false,
    });
    (
        StatusCode::OK,
        Json(HandshakeResponse {
            ready: true,
            contract_version: agent_server_protocol_version().to_string(),
            package_version: env!("CARGO_PKG_VERSION").to_string(),
            authority_backend: "binary_v0",
            repository_identity: "repository_index",
            ci_capabilities,
            operational_capabilities,
            supported_async_job_types: WorkerJobKind::ALL
                .into_iter()
                .map(|kind| kind.as_str().to_string())
                .collect(),
            endpoints: state.service_endpoints,
        }),
    )
}
