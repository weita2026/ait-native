use super::*;
use crate::config_runtime::RuntimeConfigFoundation;
use crate::diagnostics::DiagnosticsFoundation;
use crate::json_support::json;
use crate::plan_filesystem::{PlanArtifactResolverFoundation, PlanFilesystemError};
use crate::plan_provenance::PlanProvenanceFoundation;
use crate::time_identity::TimeIdentityFoundation;

fn assert_time_identity_provider<T: TimeIdentityProvider>() {}
fn assert_plan_provenance_codec<T: PlanProvenanceCodec>() {}
fn assert_config_provider<T: ConfigProvider>() {}
fn assert_diagnostics_probe<T: DiagnosticsProbe>() {}
fn assert_artifact_resolver<T: ArtifactResolver<Error = PlanFilesystemError>>() {}
fn assert_connection_manager<T: ConnectionManager<Error = String>>() {}
fn assert_storage_readiness_probe<T: StorageReadinessProbe<Error = String, Output = JsonValue>>() {}

#[derive(Default)]
struct SubstituteStoreManager;

impl ConnectionManager for SubstituteStoreManager {
    type Error = String;
    type LeaseMode = ();
    type LeaseReceipt = u64;
    type Stats = usize;

    fn inspect(&self) -> Self::Stats {
        0
    }

    fn acquire(&self, _mode: Self::LeaseMode) -> Result<Self::LeaseReceipt, Self::Error> {
        Ok(1)
    }

    fn release(&self, _lease_id: u64) -> Result<Self::Stats, Self::Error> {
        Ok(0)
    }

    fn close(&self) -> Result<Self::Stats, Self::Error> {
        Ok(0)
    }
}

impl StorageReadinessProbe for SubstituteStoreManager {
    type Error = String;
    type Output = JsonValue;

    fn inspect_storage_readiness(&self) -> Result<Self::Output, Self::Error> {
        Ok(json!({ "ready": true, "backend": "substitute" }))
    }
}

#[test]
fn foundation_types_and_store_ports_remain_backend_neutral() {
    assert_time_identity_provider::<TimeIdentityFoundation>();
    assert_plan_provenance_codec::<PlanProvenanceFoundation>();
    assert_config_provider::<RuntimeConfigFoundation>();
    assert_diagnostics_probe::<DiagnosticsFoundation>();
    assert_artifact_resolver::<PlanArtifactResolverFoundation>();
    assert_connection_manager::<SubstituteStoreManager>();
    assert_storage_readiness_probe::<SubstituteStoreManager>();

    let readiness = SubstituteStoreManager.inspect_storage_readiness().unwrap();
    assert_eq!(readiness["ready"], true);
}
