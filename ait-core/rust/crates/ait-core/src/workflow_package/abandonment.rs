use super::*;
use crate::workflow_expected_identity::PackageWorkflowExpectedIdentity;
use serde::{Deserialize, Serialize};

/// Transient cancellation progress, not a persisted Worker Job outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageAbandonmentState {
    Abandoning,
    Abandoned,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PackageAbandonment {
    pub expected_identity: PackageWorkflowExpectedIdentity,
    pub state: PackageAbandonmentState,
}

impl PackageAbandonment {
    pub fn from_value(
        value: &Value,
        expected: &PackageWorkflowExpectedIdentity,
    ) -> Result<Self, String> {
        expected.validate()?;
        let result: Self = serde_json::from_value(package_response_data(value)?.clone())
            .map_err(|error| error.to_string())?;
        result.expected_identity.validate()?;
        if result.expected_identity != *expected {
            return Err("Abandonment response selected another package/Job".into());
        }
        Ok(result)
    }
}
