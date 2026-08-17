use super::normalization::{
    normalize_plan_backend_identity_payload_map,
    normalize_plan_diagnostics_compatibility_payload_map,
    normalize_plan_diagnostics_doctor_payload_map,
    normalize_plan_diagnostics_readiness_payload_map,
    normalize_plan_diagnostics_request_payload_map,
};
use crate::json_support::{JsonCodec, JsonMap, JsonValue};
use crate::shared_foundation::DiagnosticsProbe;

pub struct DiagnosticsJson<S> {
    store: S,
}

impl<S> DiagnosticsJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl DiagnosticsJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> DiagnosticsJson<S> {
    pub fn normalize_diagnostics_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan diagnostics request")?;
        normalize_plan_diagnostics_request_payload_map(payload)
    }

    pub fn normalize_backend_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan diagnostics backend identity")?;
        normalize_plan_backend_identity_payload_map(payload)
    }

    pub fn normalize_diagnostics_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan diagnostics compatibility")?;
        normalize_plan_diagnostics_compatibility_payload_map(payload)
    }

    pub fn normalize_diagnostics_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan diagnostics readiness")?;
        normalize_plan_diagnostics_readiness_payload_map(payload)
    }

    pub fn normalize_diagnostics_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan diagnostics doctor")?;
        normalize_plan_diagnostics_doctor_payload_map(payload)
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        JsonCodec::parse_object_with_error_prefix(
            payload_json,
            &format!("Invalid JSON for {label}"),
            &format!("Plan diagnostics payload field `{label}` must be an object."),
        )
        .map_err(String::from)
    }
}

#[derive(Default)]
pub struct DiagnosticsFoundation;

impl DiagnosticsProbe for DiagnosticsFoundation {
    fn normalize_diagnostics_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        DiagnosticsJson::stateless().normalize_diagnostics_request_payload_json(payload_json)
    }

    fn normalize_backend_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        DiagnosticsJson::stateless().normalize_backend_identity_payload_json(payload_json)
    }

    fn normalize_diagnostics_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        DiagnosticsJson::stateless().normalize_diagnostics_compatibility_payload_json(payload_json)
    }

    fn normalize_diagnostics_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        DiagnosticsJson::stateless().normalize_diagnostics_readiness_payload_json(payload_json)
    }

    fn normalize_diagnostics_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        DiagnosticsJson::stateless().normalize_diagnostics_doctor_payload_json(payload_json)
    }
}

pub fn normalize_plan_diagnostics_request_with_diagnostics_probe<P>(
    probe: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: DiagnosticsProbe + ?Sized,
{
    probe.normalize_diagnostics_request_payload_json(payload_json)
}

pub fn normalize_plan_backend_identity_with_diagnostics_probe<P>(
    probe: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: DiagnosticsProbe + ?Sized,
{
    probe.normalize_backend_identity_payload_json(payload_json)
}

pub fn normalize_plan_diagnostics_compatibility_with_diagnostics_probe<P>(
    probe: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: DiagnosticsProbe + ?Sized,
{
    probe.normalize_diagnostics_compatibility_payload_json(payload_json)
}

pub fn normalize_plan_diagnostics_readiness_with_diagnostics_probe<P>(
    probe: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: DiagnosticsProbe + ?Sized,
{
    probe.normalize_diagnostics_readiness_payload_json(payload_json)
}

pub fn normalize_plan_diagnostics_doctor_with_diagnostics_probe<P>(
    probe: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: DiagnosticsProbe + ?Sized,
{
    probe.normalize_diagnostics_doctor_payload_json(payload_json)
}
