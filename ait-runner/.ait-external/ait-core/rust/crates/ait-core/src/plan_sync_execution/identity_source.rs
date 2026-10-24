use super::identity_ports::PlanSyncWorkflowIdentitySource;
use crate::json_support::{JsonMap, JsonValue};
use crate::shared_foundation::TimeIdentityProvider;
use crate::time_identity::TimeIdentityFoundation;

pub(super) struct TimeIdentityPlanSyncWorkflowIdentitySource<P = TimeIdentityFoundation> {
    provider: P,
}

impl TimeIdentityPlanSyncWorkflowIdentitySource<TimeIdentityFoundation> {
    pub(super) fn default_source() -> Self {
        Self {
            provider: TimeIdentityFoundation,
        }
    }
}

impl<P> PlanSyncWorkflowIdentitySource for TimeIdentityPlanSyncWorkflowIdentitySource<P>
where
    P: TimeIdentityProvider,
{
    fn workflow_id(&self, family: &str, namespace_prefix: Option<&str>) -> Result<String, String> {
        let payload = self.provider.build_workflow_id_payload_json(
            &JsonValue::Object(JsonMap::from_iter([
                ("family".to_string(), JsonValue::String(family.to_string())),
                (
                    "namespace_prefix".to_string(),
                    namespace_prefix
                        .map(|value| JsonValue::String(value.to_string()))
                        .unwrap_or(JsonValue::Null),
                ),
            ]))
            .to_string(),
        )?;
        required_string_field(&payload, "generated_id")
    }

    fn timestamp(&self) -> Result<String, String> {
        let payload = self
            .provider
            .build_timestamp_payload_json(&JsonValue::Object(JsonMap::new()).to_string())?;
        required_string_field(&payload, "timestamp")
    }
}

fn required_string_field(payload: &JsonValue, name: &str) -> Result<String, String> {
    payload
        .get(name)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .ok_or_else(|| format!("Plan sync workflow identity payload is missing {name}."))
}
