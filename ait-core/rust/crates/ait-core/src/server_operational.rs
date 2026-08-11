use crate::json_support::JsonValue;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;

pub const SERVER_OPERATIONAL_CAPABILITY_CONTRACT: &str = "ait.server.operational-capabilities.v1";
pub const BINARY_REPOSITORY_IDENTITY: &str = "binary-repository-index.v0";
pub const BINARY_WORKER_JOB_IDENTITY: &str = "binary-worker-job-key.v0";
pub const NATIVE_JOB_V3_CONTRACT: &str = "ait.runner.native-job.v3";
pub const NATIVE_JOB_V2_CONTRACT: &str = "ait.runner.native-job.v2";
pub const NATIVE_JOB_REPOSITORY_CI_ARGV0: &str = "./ci/run";
pub const NATIVE_JOB_REPOSITORY_CI_UNIX_PATH: &str = "ci/run.sh";
pub const NATIVE_JOB_REPOSITORY_CI_WINDOWS_PATH: &str = "ci/run.ps1";
pub const REPOSITORY_INDEX_CONFIG_KEY: &str = "repository_index";

const MAX_LEASE_TOKEN_BYTES: usize = 128;

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct RepositoryIndex(u32);

impl RepositoryIndex {
    pub const AIT_CORE: Self = Self(0);
    pub const AIT_SERVER: Self = Self(1);
    pub const AIT_PYTHON: Self = Self(2);
    pub const AIT_NODE: Self = Self(3);

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub fn parse_config_value(value: &JsonValue) -> Result<Self, String> {
        let number = value.as_u64().ok_or_else(|| {
            format!("{REPOSITORY_INDEX_CONFIG_KEY} must be an unsigned JSON integer")
        })?;
        let number = u32::try_from(number).map_err(|_| {
            format!("{REPOSITORY_INDEX_CONFIG_KEY} must fit an unsigned 32-bit integer")
        })?;
        Ok(Self(number))
    }
}

impl Display for RepositoryIndex {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl FromStr for RepositoryIndex {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_canonical_u32(value, "repository_index").map(Self)
    }
}

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct WorkerJobIndex(u32);

impl WorkerJobIndex {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Display for WorkerJobIndex {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl FromStr for WorkerJobIndex {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_canonical_u32(value, "worker_job_index").map(Self)
    }
}

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct WorkerJobKey {
    pub repository_index: RepositoryIndex,
    pub worker_job_index: WorkerJobIndex,
}

impl WorkerJobKey {
    pub const fn new(repository_index: RepositoryIndex, worker_job_index: WorkerJobIndex) -> Self {
        Self {
            repository_index,
            worker_job_index,
        }
    }
}

impl Display for WorkerJobKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}/{}",
            self.repository_index, self.worker_job_index
        )
    }
}

impl FromStr for WorkerJobKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (repository_index, worker_job_index) = value.split_once('/').ok_or_else(|| {
            "WorkerJobKey must use canonical `<repository_index>/<worker_job_index>` form"
                .to_string()
        })?;
        if worker_job_index.contains('/') {
            return Err("WorkerJobKey must contain exactly one `/` separator".to_string());
        }
        Ok(Self::new(
            repository_index.parse()?,
            worker_job_index.parse()?,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerRepositoryAuthorityConfig {
    pub repository_index: RepositoryIndex,
}

impl ServerRepositoryAuthorityConfig {
    pub const fn new(repository_index: RepositoryIndex) -> Self {
        Self { repository_index }
    }

    pub fn from_config_object(
        object: &serde_json::Map<String, JsonValue>,
    ) -> Result<Option<Self>, String> {
        object
            .get(REPOSITORY_INDEX_CONFIG_KEY)
            .map(RepositoryIndex::parse_config_value)
            .transpose()
            .map(|repository_index| repository_index.map(Self::new))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerLeaseProof {
    pub repository_index: RepositoryIndex,
    pub worker_job_index: WorkerJobIndex,
    pub attempt_count: u16,
    pub lease_token: String,
}

impl WorkerLeaseProof {
    pub fn new(
        key: WorkerJobKey,
        attempt_count: u16,
        lease_token: impl Into<String>,
    ) -> Result<Self, String> {
        if attempt_count == 0 {
            return Err("attempt_count must be non-zero for a claimed Worker Job".to_string());
        }
        let lease_token = lease_token.into();
        validate_lease_token(&lease_token)?;
        Ok(Self {
            repository_index: key.repository_index,
            worker_job_index: key.worker_job_index,
            attempt_count,
            lease_token,
        })
    }

    pub const fn key(&self) -> WorkerJobKey {
        WorkerJobKey::new(self.repository_index, self.worker_job_index)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.attempt_count == 0 {
            return Err("attempt_count must be non-zero for a claimed Worker Job".to_string());
        }
        validate_lease_token(&self.lease_token)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServerOperationalCapabilities {
    pub binary_repository_index: bool,
    pub binary_worker_job_key: bool,
    pub native_job_v3: bool,
    pub native_job_v2: bool,
}

impl ServerOperationalCapabilities {
    pub fn from_server_payload(payload: Option<&JsonValue>) -> Self {
        let Some(capabilities) = payload
            .and_then(|value| value.get("operational_capabilities"))
            .and_then(JsonValue::as_object)
        else {
            return Self::default();
        };
        if capabilities.get("contract").and_then(JsonValue::as_str)
            != Some(SERVER_OPERATIONAL_CAPABILITY_CONTRACT)
        {
            return Self::default();
        }
        let repository_identity = capabilities
            .get("repository_identity")
            .and_then(JsonValue::as_str);
        let worker_job_identity = capabilities
            .get("worker_job_identity")
            .and_then(JsonValue::as_str);
        let runner_contracts = capabilities
            .get("runner_contracts")
            .and_then(JsonValue::as_array);
        let advertises = |expected| {
            runner_contracts.is_some_and(|contracts| {
                contracts
                    .iter()
                    .any(|contract| contract.as_str() == Some(expected))
            })
        };
        Self {
            binary_repository_index: repository_identity == Some(BINARY_REPOSITORY_IDENTITY),
            binary_worker_job_key: worker_job_identity == Some(BINARY_WORKER_JOB_IDENTITY),
            native_job_v3: advertises(NATIVE_JOB_V3_CONTRACT),
            native_job_v2: advertises(NATIVE_JOB_V2_CONTRACT),
        }
    }

    pub fn require_binary_runtime(&self) -> Result<(), String> {
        let mut missing = Vec::new();
        if !self.binary_repository_index {
            missing.push(BINARY_REPOSITORY_IDENTITY);
        }
        if !self.binary_worker_job_key {
            missing.push(BINARY_WORKER_JOB_IDENTITY);
        }
        if !self.native_job_v3 {
            missing.push(NATIVE_JOB_V3_CONTRACT);
        }
        if missing.is_empty() {
            return Ok(());
        }
        Err(format!(
            "Remote server does not advertise the PostgreSQL-free Binary runtime contract: missing {}",
            missing.join(", ")
        ))
    }
}

pub fn repository_authority_path(repository_index: RepositoryIndex) -> String {
    format!("/v1/native/repository-authorities/{repository_index}")
}

pub fn repository_worker_jobs_path(repository_index: RepositoryIndex) -> String {
    format!(
        "{}/worker-jobs",
        repository_authority_path(repository_index)
    )
}

pub fn repository_retirement_path(repository_index: RepositoryIndex) -> String {
    format!("{}/retirement", repository_authority_path(repository_index))
}

pub fn repository_retirement_abort_path(repository_index: RepositoryIndex) -> String {
    format!("{}/abort", repository_retirement_path(repository_index))
}

pub fn repository_retirement_purge_path(repository_index: RepositoryIndex) -> String {
    format!("{}/purge", repository_retirement_path(repository_index))
}

pub const fn repository_restores_path() -> &'static str {
    "/v1/native/repository-restores"
}

pub fn worker_job_path(key: WorkerJobKey) -> String {
    format!(
        "{}/{}",
        repository_worker_jobs_path(key.repository_index),
        key.worker_job_index
    )
}

pub fn worker_job_operation_path(key: WorkerJobKey, operation: &str) -> Result<String, String> {
    match operation {
        "claim" | "heartbeat" | "complete" | "fail" => {
            Ok(format!("{}:{operation}", worker_job_path(key)))
        }
        _ => Err(format!("Unknown Worker Job operation: {operation}")),
    }
}

pub const fn claim_next_worker_job_path() -> &'static str {
    "/v1/native/worker-jobs:claim"
}

fn parse_canonical_u32(value: &str, field: &str) -> Result<u32, String> {
    if value.is_empty()
        || value.bytes().any(|byte| !byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(format!(
            "{field} must be canonical unsigned base-10 without leading zeroes"
        ));
    }
    value
        .parse::<u32>()
        .map_err(|_| format!("{field} must fit an unsigned 32-bit integer"))
}

fn validate_lease_token(token: &str) -> Result<(), String> {
    if token.is_empty()
        || token.len() > MAX_LEASE_TOKEN_BYTES
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\"' | b'\\'))
    {
        return Err(format!(
            "lease_token must be 1..={MAX_LEASE_TOKEN_BYTES} visible ASCII bytes without quote or backslash"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_support::json;

    #[test]
    fn fixed_repository_indexes_match_binary_v0() {
        assert_eq!(RepositoryIndex::AIT_CORE.get(), 0);
        assert_eq!(RepositoryIndex::AIT_SERVER.get(), 1);
        assert_eq!(RepositoryIndex::AIT_PYTHON.get(), 2);
        assert_eq!(RepositoryIndex::AIT_NODE.get(), 3);
    }

    #[test]
    fn numeric_identities_use_canonical_decimal_and_numeric_json() {
        let key = WorkerJobKey::new(RepositoryIndex::new(12), WorkerJobIndex::new(34));
        assert_eq!(key.to_string(), "12/34");
        assert_eq!("12/34".parse::<WorkerJobKey>(), Ok(key));
        assert!("01/34".parse::<WorkerJobKey>().is_err());
        assert!("12/034".parse::<WorkerJobKey>().is_err());
        assert!("12/34/56".parse::<WorkerJobKey>().is_err());
        assert_eq!(
            serde_json::to_value(key).expect("encode WorkerJobKey"),
            json!({"repository_index": 12, "worker_job_index": 34})
        );
    }

    #[test]
    fn repository_config_accepts_only_unsigned_numeric_index() {
        let config = json!({"repository_index": 0});
        assert_eq!(
            ServerRepositoryAuthorityConfig::from_config_object(
                config.as_object().expect("config object")
            ),
            Ok(Some(ServerRepositoryAuthorityConfig::new(
                RepositoryIndex::AIT_CORE
            )))
        );
        for invalid in [
            json!({"repository_index": -1}),
            json!({"repository_index": "0"}),
            json!({"repository_index": 4_294_967_296_u64}),
        ] {
            assert!(ServerRepositoryAuthorityConfig::from_config_object(
                invalid.as_object().expect("invalid config object")
            )
            .is_err());
        }
    }

    #[test]
    fn routes_are_numeric_and_never_embed_legacy_ids() {
        let key = WorkerJobKey::new(RepositoryIndex::new(7), WorkerJobIndex::new(11));
        assert_eq!(
            repository_authority_path(key.repository_index),
            "/v1/native/repository-authorities/7"
        );
        assert_eq!(
            worker_job_path(key),
            "/v1/native/repository-authorities/7/worker-jobs/11"
        );
        assert_eq!(
            worker_job_operation_path(key, "heartbeat"),
            Ok("/v1/native/repository-authorities/7/worker-jobs/11:heartbeat".to_string())
        );
        assert!(worker_job_operation_path(key, "delete").is_err());
    }

    #[test]
    fn lease_proof_is_attempt_bound_and_contains_only_pair_identity() {
        let proof = WorkerLeaseProof::new(
            WorkerJobKey::new(RepositoryIndex::new(3), WorkerJobIndex::new(9)),
            2,
            "opaque-token",
        )
        .expect("valid lease proof");
        assert_eq!(proof.key().to_string(), "3/9");
        assert!(proof.validate().is_ok());
        assert!(WorkerLeaseProof::new(
            WorkerJobKey::new(RepositoryIndex::new(3), WorkerJobIndex::new(9)),
            0,
            "opaque-token",
        )
        .is_err());
        let encoded = serde_json::to_value(proof).expect("encode lease proof");
        assert!(encoded.get("repo_id").is_none());
        assert!(encoded.get("job_id").is_none());
    }

    #[test]
    fn capability_negotiation_fails_closed() {
        let payload = json!({
            "operational_capabilities": {
                "contract": SERVER_OPERATIONAL_CAPABILITY_CONTRACT,
                "repository_identity": BINARY_REPOSITORY_IDENTITY,
                "worker_job_identity": BINARY_WORKER_JOB_IDENTITY,
                "runner_contracts": [NATIVE_JOB_V3_CONTRACT],
            }
        });
        let capabilities = ServerOperationalCapabilities::from_server_payload(Some(&payload));
        assert_eq!(
            capabilities,
            ServerOperationalCapabilities {
                binary_repository_index: true,
                binary_worker_job_key: true,
                native_job_v3: true,
                native_job_v2: false,
            }
        );
        capabilities
            .require_binary_runtime()
            .expect("complete Binary runtime capability");

        let wrong_contract = json!({
            "operational_capabilities": {
                "contract": "future",
                "repository_identity": BINARY_REPOSITORY_IDENTITY,
                "worker_job_identity": BINARY_WORKER_JOB_IDENTITY,
                "runner_contracts": [NATIVE_JOB_V2_CONTRACT],
            }
        });
        assert!(
            ServerOperationalCapabilities::from_server_payload(Some(&wrong_contract))
                .require_binary_runtime()
                .is_err()
        );

        let previous_only = json!({
            "operational_capabilities": {
                "contract": SERVER_OPERATIONAL_CAPABILITY_CONTRACT,
                "repository_identity": BINARY_REPOSITORY_IDENTITY,
                "worker_job_identity": BINARY_WORKER_JOB_IDENTITY,
                "runner_contracts": [NATIVE_JOB_V2_CONTRACT],
            }
        });
        let previous = ServerOperationalCapabilities::from_server_payload(Some(&previous_only));
        assert!(previous.native_job_v2);
        assert!(!previous.native_job_v3);
        assert!(previous.require_binary_runtime().is_err());

        assert_eq!(NATIVE_JOB_REPOSITORY_CI_ARGV0, "./ci/run");
        assert_eq!(NATIVE_JOB_REPOSITORY_CI_UNIX_PATH, "ci/run.sh");
        assert_eq!(NATIVE_JOB_REPOSITORY_CI_WINDOWS_PATH, "ci/run.ps1");
    }
}
