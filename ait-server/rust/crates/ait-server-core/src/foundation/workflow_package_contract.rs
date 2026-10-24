use crate::foundation::server_content_binary_db::server_snapshot_hash48_from_id;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PACKAGE_CONTENT_TYPE: &str = "application/vnd.ait.workflow-package.v1+zstd";
pub const PACKAGE_MANIFEST_CONTRACT: &str = "workflow-package/v1";
pub const PACKAGE_FINISH_CONTRACT: &str = "task-land-package-guarded/v1";
pub const PACKAGE_EVIDENCE_CONTRACT: &str = "workflow-package-evidence/v1";
pub const PACKAGE_ABANDON_CONTRACT: &str = "workflow-package-abandon/v1";

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageExpectedIdentity {
    pub repository_index: u32,
    pub package_hash: String,
    pub worker_job_index: u32,
    pub base_snapshot_id: String,
    pub revision_snapshot_id: String,
}

impl PackageExpectedIdentity {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let identity: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid package expected_identity: {error}"))?;
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.hash_bytes()?;
        for snapshot in [&self.base_snapshot_id, &self.revision_snapshot_id] {
            server_snapshot_hash48_from_id(snapshot).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn hash_bytes(&self) -> Result<[u8; 32], String> {
        parse_package_hash(&self.package_hash)
    }

    pub fn idempotency_key(&self) -> String {
        format!(
            "workflow-package:{}:{}",
            self.package_hash, self.worker_job_index
        )
    }
}

pub fn parse_package_hash(text: &str) -> Result<[u8; 32], String> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("package_hash must be 64 lowercase SHA-256 hex characters".to_string());
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * index..2 * index + 2], 16)
            .map_err(|error| format!("Invalid package hash: {error}"))?;
    }
    Ok(digest)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFinishRequest {
    pub contract: String,
    pub idempotency_key: String,
    pub actor_key_index: u32,
    pub target_line: String,
    pub mode: String,
    pub expected_identity: PackageExpectedIdentity,
}

impl PackageFinishRequest {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let request: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid package finish request: {error}"))?;
        request.expected_identity.validate()?;
        if request.contract != PACKAGE_FINISH_CONTRACT
            || request.idempotency_key != request.expected_identity.idempotency_key()
            || request.target_line.trim().is_empty()
            || !matches!(request.mode.as_str(), "direct" | "merge" | "ff-only")
        {
            return Err(
                "Package finish contract, idempotency key, target or mode differs".to_string(),
            );
        }
        Ok(request)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PackageEvidenceOperation {
    Review,
    Attestation,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageAbandonRequest {
    pub contract: String,
    pub actor_key_index: u32,
    pub expected_identity: PackageExpectedIdentity,
    pub idempotency_key: String,
}
impl PackageAbandonRequest {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let request: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid package abandonment request: {error}"))?;
        request.expected_identity.validate()?;
        if request.contract != PACKAGE_ABANDON_CONTRACT
            || request.idempotency_key
                != format!(
                    "workflow-package-abandon:{}:{}",
                    request.expected_identity.package_hash,
                    request.expected_identity.worker_job_index
                )
        {
            return Err("Package abandonment contract or idempotency key differs".into());
        }
        Ok(request)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageEvidenceRequest {
    pub contract: String,
    pub operation: PackageEvidenceOperation,
    pub idempotency_key: String,
    pub actor_key_index: u32,
    pub expected_identity: PackageExpectedIdentity,
    pub payload: Map<String, Value>,
}

impl PackageEvidenceRequest {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let request: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid package evidence request: {error}"))?;
        request.expected_identity.validate()?;
        if request.contract != PACKAGE_EVIDENCE_CONTRACT
            || request.idempotency_key.trim().is_empty()
        {
            return Err(
                "Package evidence requires its contract and a stable nonempty idempotency key"
                    .to_string(),
            );
        }
        if request.payload.keys().any(|key| {
            key == "patchset_id"
                || key == "patchset_ci"
                || key.ends_with("_at")
                || key.ends_with("_at_s")
                || matches!(key.as_str(), "date" | "timestamp" | "timestamps")
        }) {
            return Err(
                "Package evidence cannot supply formal identity, CI result or event timestamps"
                    .to_string(),
            );
        }
        if request.operation == PackageEvidenceOperation::Review {
            const FIELDS: &[&str] = &["reviewer", "action", "reviewer_kind", "comment", "blocking"];
            if request
                .payload
                .keys()
                .any(|key| !FIELDS.contains(&key.as_str()))
            {
                return Err("Package review contains an undeclared field".to_string());
            }
            for field in ["reviewer", "action"] {
                if request
                    .payload
                    .get(field)
                    .and_then(Value::as_str)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(format!("Package review requires {field}"));
                }
            }
        }
        if request
            .payload
            .get("detail")
            .and_then(Value::as_object)
            .is_some_and(|detail| detail.contains_key("patchset_ci"))
        {
            return Err("Package attestation cannot embed patchset_ci".to_string());
        }
        Ok(request)
    }
}

#[cfg(test)]
#[path = "workflow_package_contract/tests.rs"]
mod tests;
