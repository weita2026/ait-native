use super::*;

#[derive(Clone)]
pub struct ServerState {
    pub(super) service_endpoints: Vec<String>,
    pub(super) runtime_service: Arc<dyn ServerRuntimeService>,
    pub(super) workflow_service: Arc<dyn ServerWorkflowStore>,
    pub(super) repository_service: Arc<dyn NativeRepositoryService>,
    pub(super) operational_binary: Arc<OperationalBinaryRuntime>,
}
