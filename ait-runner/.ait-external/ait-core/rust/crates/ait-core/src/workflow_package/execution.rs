use super::*;
use crate::workflow_expected_identity::PackageWorkflowExpectedIdentity;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageCiState {
    Queued,
    Running,
    Passed,
    Failed,
    Interrupted,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageAvailability {
    Available,
    Unavailable,
    Unknown,
}

pub fn package_response_data(value: &Value) -> Result<&Value, String> {
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err("Package response reports failure".into());
    }
    let data = value.get("data").unwrap_or(value);
    if !data.is_object() {
        return Err("Package response data must be an object".into());
    }
    Ok(data)
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PreparedPackageExecution {
    pub expected_identity: PackageWorkflowExpectedIdentity,
    pub ci_state: PackageCiState,
    pub reused: bool,
}

impl PreparedPackageExecution {
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let result: Self = serde_json::from_value(package_response_data(value)?.clone())
            .map_err(|error| error.to_string())?;
        result.expected_identity.validate()?;
        if result.ci_state == PackageCiState::Unknown {
            return Err("Prepare response cannot claim an unknown execution is complete".into());
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageExecution {
    pub expected_identity: PackageWorkflowExpectedIdentity,
    pub ci_state: PackageCiState,
    pub availability: PackageAvailability,
    pub recoverable: bool,
    #[serde(default)]
    pub author_mode: Option<String>,
}

impl PackageExecution {
    pub fn from_job_response(
        value: &Value,
        expected: &PackageWorkflowExpectedIdentity,
    ) -> Result<Option<Self>, String> {
        expected.validate()?;
        let data = package_response_data(value)?;
        let job = &data["job"];
        if job["repository_index"].as_u64() != Some(u64::from(expected.repository_index))
            || job["worker_job_index"].as_u64() != Some(u64::from(expected.worker_job_index))
        {
            return Err("Worker Job readback belongs to another Repository/execution".into());
        }
        match data.get("package_execution") {
            Some(Value::Null) => Ok(None),
            Some(value) => {
                let result: Self =
                    serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
                result.expected_identity.validate()?;
                if result.expected_identity != *expected {
                    return Err("Package execution readback changed the pinned identity".into());
                }
                if let Some(mode) = &result.author_mode {
                    if crate::policy::normalize_author_mode(mode)? != *mode {
                        return Err(
                            "Package execution author_mode must be an exact existing mode".into(),
                        );
                    }
                }
                if result.recoverable
                    && (result.availability != PackageAvailability::Available
                        || matches!(
                            result.ci_state,
                            PackageCiState::Unknown
                                | PackageCiState::Interrupted
                                | PackageCiState::Failed
                        ))
                {
                    return Err("Package recovery conflicts with unavailable source or untrusted/failed CI state".into());
                }
                Ok(Some(result))
            }
            None => Err("Worker Job readback is missing nullable package_execution".into()),
        }
    }

    pub fn ready_passed(&self) -> bool {
        self.ci_state == PackageCiState::Passed
            && self.availability == PackageAvailability::Available
            && self.recoverable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> PackageWorkflowExpectedIdentity {
        PackageWorkflowExpectedIdentity {
            repository_index: 0,
            package_hash: "ab".repeat(32),
            worker_job_index: 0,
            base_snapshot_id: "SNP-000000000001".into(),
            revision_snapshot_id: "SNP-000000000002".into(),
        }
    }
    #[test]
    fn completed_worker_with_failed_required_ci_is_not_ready_success() {
        let id = identity();
        let value = json!({"job":{"repository_index":0,"worker_job_index":0,"state":"succeeded","outcome_kind":1},
            "package_execution":{"expected_identity":id,"ci_state":"failed","availability":"available","recoverable":false}});
        let result = PackageExecution::from_job_response(&value, &id)
            .unwrap()
            .unwrap();
        assert_eq!(result.ci_state, PackageCiState::Failed);
        assert!(!result.ready_passed());
    }
    #[test]
    fn missing_runtime_and_foreign_job_never_inherit_a_previous_success() {
        let id = identity();
        let mut value = json!({"job":{"repository_index":0,"worker_job_index":0,"state":"succeeded"},"package_execution":null});
        assert!(PackageExecution::from_job_response(&value, &id)
            .unwrap()
            .is_none());
        value["package_execution"] = json!({"expected_identity":id,"ci_state":"unknown","availability":"unknown","recoverable":false});
        assert!(!PackageExecution::from_job_response(&value, &id)
            .unwrap()
            .unwrap()
            .ready_passed());
        value["package_execution"]["expected_identity"]["worker_job_index"] = json!(1);
        assert!(PackageExecution::from_job_response(&value, &id).is_err());
        value["package_execution"]["expected_identity"]["worker_job_index"] = json!(0);
        value["job"]["worker_job_index"] = json!(1);
        assert!(PackageExecution::from_job_response(&value, &id).is_err());
    }

    #[test]
    fn sealed_mode_is_nullable_but_must_be_an_exact_existing_mode() {
        let id = identity();
        let mut value = json!({"job":{"repository_index":0,"worker_job_index":0},
            "package_execution":{"expected_identity":id,"ci_state":"passed","availability":"available","recoverable":true}});
        assert_eq!(
            PackageExecution::from_job_response(&value, &id)
                .unwrap()
                .unwrap()
                .author_mode,
            None
        );
        for mode in [
            Value::Null,
            json!("human_only"),
            json!("human_with_ai_assist"),
            json!("ai_with_human_review"),
            json!("ai_only_experimental"),
        ] {
            value["package_execution"]["author_mode"] = mode.clone();
            assert_eq!(
                PackageExecution::from_job_response(&value, &id)
                    .unwrap()
                    .unwrap()
                    .author_mode
                    .as_deref(),
                mode.as_str()
            );
        }
        for invalid in [
            json!(""),
            json!("default"),
            json!(" human_only "),
            json!(true),
            json!(0),
        ] {
            value["package_execution"]["author_mode"] = invalid;
            assert!(PackageExecution::from_job_response(&value, &id).is_err());
        }
    }
}
