use super::*;
use crate::file_io::{FileIoError, FileIoResult, FileIoStore};
use crate::json_support::json;
use crate::shared_foundation::DiagnosticsProbe;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

fn assert_diagnostics_probe<T: DiagnosticsProbe>() {}

struct SubstituteDiagnosticsProbe;

#[derive(Default)]
struct FakeDiagnosticsFileIoStore {
    reads: RefCell<Vec<PathBuf>>,
}

impl FileIoStore for FakeDiagnosticsFileIoStore {
    fn home_dir(&self) -> Option<PathBuf> {
        None
    }

    fn path_exists(&self, _path: &Path) -> bool {
        true
    }

    fn read_bytes(&self, path: &Path) -> FileIoResult<Vec<u8>> {
        self.reads.borrow_mut().push(path.to_path_buf());
        Ok(b"not a zip".to_vec())
    }

    fn read_to_string(&self, _path: &Path) -> FileIoResult<String> {
        Err(FileIoError::other("unexpected string read"))
    }

    fn write_string(&self, _path: &Path, _text: &str) -> FileIoResult<()> {
        Err(FileIoError::other("unexpected write"))
    }

    fn write_string_atomically(
        &self,
        _path: &Path,
        _text: &str,
        _publish_label: &str,
    ) -> FileIoResult<()> {
        Err(FileIoError::other("unexpected atomic write"))
    }
}

impl SubstituteDiagnosticsProbe {
    fn payload(operation: &str, payload_json: &str) -> JsonValue {
        json!({
            "probe": "substitute",
            "operation": operation,
            "payload_json": payload_json,
        })
    }
}

impl DiagnosticsProbe for SubstituteDiagnosticsProbe {
    fn normalize_diagnostics_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_diagnostics_request_payload_json",
            payload_json,
        ))
    }

    fn normalize_backend_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_backend_identity_payload_json",
            payload_json,
        ))
    }

    fn normalize_wheel_status_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_wheel_status_payload_json",
            payload_json,
        ))
    }

    fn normalize_diagnostics_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_diagnostics_compatibility_payload_json",
            payload_json,
        ))
    }

    fn normalize_diagnostics_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_diagnostics_readiness_payload_json",
            payload_json,
        ))
    }

    fn normalize_diagnostics_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_diagnostics_doctor_payload_json",
            payload_json,
        ))
    }
}

#[test]
fn plan_diagnostics_foundation_implements_probe_trait() {
    assert_diagnostics_probe::<DiagnosticsFoundation>();
}

#[test]
fn diagnostics_bound_helpers_accept_substitute_probe() {
    let probe = SubstituteDiagnosticsProbe;
    let request = json!({ "sample": true }).to_string();

    let cases = [
        (
            "normalize_diagnostics_request_payload_json",
            normalize_plan_diagnostics_request_with_diagnostics_probe(&probe, &request)
                .expect("diagnostics request"),
        ),
        (
            "normalize_backend_identity_payload_json",
            normalize_plan_backend_identity_with_diagnostics_probe(&probe, &request)
                .expect("backend identity"),
        ),
        (
            "normalize_wheel_status_payload_json",
            normalize_plan_wheel_status_with_diagnostics_probe(&probe, &request)
                .expect("wheel status"),
        ),
        (
            "normalize_diagnostics_compatibility_payload_json",
            normalize_plan_diagnostics_compatibility_with_diagnostics_probe(&probe, &request)
                .expect("diagnostics compatibility"),
        ),
        (
            "normalize_diagnostics_readiness_payload_json",
            normalize_plan_diagnostics_readiness_with_diagnostics_probe(&probe, &request)
                .expect("diagnostics readiness"),
        ),
        (
            "normalize_diagnostics_doctor_payload_json",
            normalize_plan_diagnostics_doctor_with_diagnostics_probe(&probe, &request)
                .expect("diagnostics doctor"),
        ),
    ];

    for (operation, payload) in cases {
        assert_eq!(
            payload["probe"],
            JsonValue::String("substitute".to_string())
        );
        assert_eq!(
            payload["operation"],
            JsonValue::String(operation.to_string())
        );
        assert_eq!(payload["payload_json"], JsonValue::String(request.clone()));
    }
}

#[test]
fn plan_diagnostics_foundation_delegates_backend_identity_normalizer() {
    let foundation = DiagnosticsFoundation;
    let payload = r#"{"selected_backend":"python","selected_backend_ready":false,"rust_authority_ready":false,"compatibility":"plan","extension_loaded":true,"extension_module":null,"extension_path":null,"extension_task_contract_version":null,"extension_plan_contract_version":null,"expected_plan_contract_version":"1.0.0","extension_package_version":null,"package_version":null,"required_exports":[],"surface_commands":["ait plan list"],"issues":[],"env":{},"exports":{},"missing_exports":[]}"#;
    assert_eq!(
        foundation
            .normalize_backend_identity_payload_json(payload)
            .unwrap(),
        normalize_plan_backend_identity_payload_json(payload).unwrap()
    );
}

#[test]
fn wheel_status_reads_wheel_through_file_io_store() {
    let store = FakeDiagnosticsFileIoStore::default();
    let payload = build_wheel_status_payload_with_file_io_store(
        &store,
        Some("/tmp/example.whl"),
        false,
        false,
    )
    .expect("wheel status");

    assert_eq!(
        store.reads.borrow().as_slice(),
        &[PathBuf::from("/tmp/example.whl")]
    );
    assert!(payload["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .any(|issue| issue
            .as_str()
            .unwrap_or_default()
            .contains("Could not read wheel /tmp/example.whl")));
}
