use std::collections::BTreeMap;
use std::path::{Component, Path};

use ait_core::server_operational::{
    NATIVE_JOB_REPOSITORY_CI_ARGV0, NATIVE_JOB_V3_CONTRACT, RepositoryIndex,
};
use serde::{Deserialize, Serialize};

use crate::RunnerError;

pub const NATIVE_JOB_CONTRACT: &str = NATIVE_JOB_V3_CONTRACT;
pub(crate) const LEGACY_NATIVE_JOB_CONTRACT: &str = "ait.runner.native-job.v1";
const LEGACY_NATIVE_JOB_ARGV0: &str = "./ci/run.sh";
pub const NATIVE_RESULT_CONTRACT: &str = "ait.runner.native-result.v1";
pub const DELIVERY_CONTRACT: &str = "ait.runner.delivery.v1";
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_TERMINAL_RESULT_BYTES: usize = 64 * 1024;
pub const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1000;
pub const DEFAULT_TIMEOUT_MS: u64 = 15 * 60 * 1000;
pub const STREAM_TAIL_BYTES: usize = 8 * 1024;

const MAX_ARGUMENTS: usize = 256;
const MAX_ARGUMENT_BYTES: usize = 16 * 1024;
const MAX_ARGUMENT_TOTAL_BYTES: usize = 128 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 256;
const MAX_ENVIRONMENT_TOTAL_BYTES: usize = 256 * 1024;
const MAX_LABEL_BYTES: usize = 256;
const MAX_EXTERNAL_REPOSITORY_ROUTES: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeJobRequest {
    pub contract: String,
    #[serde(default)]
    pub label: Option<String>,
    pub source: SourceSpec,
    pub command: CommandSpec,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub suite_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceSpec {
    LocalDirectory {
        path: String,
    },
    RemoteSnapshot {
        repository_index: RepositoryIndex,
        repository_name: String,
        snapshot_id: String,
        #[serde(default)]
        external_repository_indexes: BTreeMap<String, RepositoryIndex>,
    },
    #[doc(hidden)]
    LegacyRemoteSnapshot {
        repo_name: String,
        #[serde(default)]
        repo_id: Option<String>,
        snapshot_id: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyNativeJobRequest {
    contract: String,
    #[serde(default)]
    label: Option<String>,
    source: LegacySourceSpec,
    command: CommandSpec,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default)]
    suite_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum LegacySourceSpec {
    LocalDirectory {
        path: String,
    },
    RemoteSnapshot {
        repo_name: String,
        #[serde(default)]
        repo_id: Option<String>,
        snapshot_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub argv: Vec<String>,
    #[serde(default = "default_working_directory")]
    pub working_directory: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NativeResult {
    pub contract: &'static str,
    pub status: TerminalStatus,
    pub tests_status: TestStatus,
    pub suite_result_count: usize,
    pub suite_results: Vec<SuiteResult>,
    pub cleanup: CleanupEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatus {
    Succeeded,
    CommandFailed,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    Pass,
    Fail,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SuiteResult {
    pub suite_id: String,
    pub status: TestStatus,
    pub blocking: bool,
    pub mode: &'static str,
    pub plane: &'static str,
    pub runner_kind: &'static str,
    pub duration_seconds: f64,
    pub summary: String,
    pub execution: ExecutionEvidence,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutionEvidence {
    pub contract: &'static str,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub materialization: MaterializationEvidence,
    pub stdout: StreamEvidence,
    pub stderr: StreamEvidence,
    pub cleanup: CleanupEvidence,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MaterializationEvidence {
    pub source_kind: &'static str,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StreamEvidence {
    pub byte_count: u64,
    pub sha256: String,
    pub tail_base64: String,
    pub tail_byte_count: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CleanupEvidence {
    pub attempt_root_removed: bool,
    pub remaining_owned_paths: u64,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_working_directory() -> String {
    ".".to_string()
}

impl NativeJobRequest {
    pub fn parse_bounded(bytes: &[u8]) -> Result<Self, RunnerError> {
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(RunnerError::InvalidRequest(format!(
                "request is {} bytes; maximum is {MAX_REQUEST_BYTES}",
                bytes.len()
            )));
        }
        let request: Self = serde_json::from_slice(bytes).map_err(|error| {
            RunnerError::InvalidRequest(format!("request is not valid typed JSON: {error}"))
        })?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), RunnerError> {
        if self.contract != NATIVE_JOB_CONTRACT {
            return Err(RunnerError::InvalidRequest(format!(
                "contract must be `{NATIVE_JOB_CONTRACT}`, got `{}`",
                self.contract
            )));
        }
        self.source.validate_v3()?;
        self.validate_common(NATIVE_JOB_REPOSITORY_CI_ARGV0)
    }

    pub(crate) fn parse_legacy_bounded(bytes: &[u8]) -> Result<Self, RunnerError> {
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(RunnerError::InvalidRequest(format!(
                "legacy request is {} bytes; maximum is {MAX_REQUEST_BYTES}",
                bytes.len()
            )));
        }
        let legacy: LegacyNativeJobRequest = serde_json::from_slice(bytes).map_err(|error| {
            RunnerError::InvalidRequest(format!("legacy request is not valid typed JSON: {error}"))
        })?;
        if legacy.contract != LEGACY_NATIVE_JOB_CONTRACT {
            return Err(RunnerError::InvalidRequest(format!(
                "legacy contract must be `{LEGACY_NATIVE_JOB_CONTRACT}`, got `{}`",
                legacy.contract
            )));
        }
        let source = match legacy.source {
            LegacySourceSpec::LocalDirectory { path } => SourceSpec::LocalDirectory { path },
            LegacySourceSpec::RemoteSnapshot {
                repo_name,
                repo_id,
                snapshot_id,
            } => SourceSpec::LegacyRemoteSnapshot {
                repo_name,
                repo_id,
                snapshot_id,
            },
        };
        let request = Self {
            contract: LEGACY_NATIVE_JOB_CONTRACT.to_string(),
            label: legacy.label,
            source,
            command: legacy.command,
            timeout_ms: legacy.timeout_ms,
            suite_id: legacy.suite_id,
        };
        request.validate_execution()?;
        Ok(request)
    }

    pub(crate) fn validate_execution(&self) -> Result<(), RunnerError> {
        match self.contract.as_str() {
            NATIVE_JOB_CONTRACT => {
                self.source.validate_v3()?;
                self.validate_common(NATIVE_JOB_REPOSITORY_CI_ARGV0)
            }
            LEGACY_NATIVE_JOB_CONTRACT => {
                self.source.validate_legacy()?;
                self.validate_common(LEGACY_NATIVE_JOB_ARGV0)
            }
            contract => Err(RunnerError::InvalidRequest(format!(
                "execution contract must be `{NATIVE_JOB_CONTRACT}` or negotiated legacy `{LEGACY_NATIVE_JOB_CONTRACT}`, got `{contract}`"
            ))),
        }
    }

    fn validate_common(&self, expected_argv0: &str) -> Result<(), RunnerError> {
        validate_optional_label(self.label.as_deref(), "label")?;
        validate_optional_label(self.suite_id.as_deref(), "suite_id")?;
        validate_confined_relative_path(
            &self.command.working_directory,
            "command.working_directory",
        )?;
        if self.command.argv.is_empty() {
            return Err(RunnerError::InvalidRequest(format!(
                "command.argv must contain `{expected_argv0}`"
            )));
        }
        if self.command.argv[0] != expected_argv0 {
            return Err(RunnerError::InvalidRequest(format!(
                "command.argv[0] must be the logical `{expected_argv0}` selector"
            )));
        }
        if self.command.argv.len() > MAX_ARGUMENTS {
            return Err(RunnerError::InvalidRequest(format!(
                "command.argv contains {} entries; maximum is {MAX_ARGUMENTS}",
                self.command.argv.len()
            )));
        }
        let mut argument_bytes = 0usize;
        for argument in &self.command.argv {
            if argument.contains('\0') {
                return Err(RunnerError::InvalidRequest(
                    "command.argv must not contain NUL bytes".to_string(),
                ));
            }
            if argument.len() > MAX_ARGUMENT_BYTES {
                return Err(RunnerError::InvalidRequest(format!(
                    "one command argument exceeds {MAX_ARGUMENT_BYTES} bytes"
                )));
            }
            argument_bytes = argument_bytes.saturating_add(argument.len());
        }
        if argument_bytes > MAX_ARGUMENT_TOTAL_BYTES {
            return Err(RunnerError::InvalidRequest(format!(
                "command arguments total {argument_bytes} bytes; maximum is {MAX_ARGUMENT_TOTAL_BYTES}"
            )));
        }
        if self.command.environment.len() > MAX_ENVIRONMENT_ENTRIES {
            return Err(RunnerError::InvalidRequest(format!(
                "command.environment contains {} entries; maximum is {MAX_ENVIRONMENT_ENTRIES}",
                self.command.environment.len()
            )));
        }
        let mut environment_bytes = 0usize;
        for (key, value) in &self.command.environment {
            validate_environment_key(key)?;
            if value.contains('\0') {
                return Err(RunnerError::InvalidRequest(format!(
                    "command.environment value for `{key}` contains a NUL byte"
                )));
            }
            environment_bytes =
                environment_bytes.saturating_add(key.len().saturating_add(value.len()));
        }
        if environment_bytes > MAX_ENVIRONMENT_TOTAL_BYTES {
            return Err(RunnerError::InvalidRequest(format!(
                "command.environment totals {environment_bytes} bytes; maximum is {MAX_ENVIRONMENT_TOTAL_BYTES}"
            )));
        }
        if !(1..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(RunnerError::InvalidRequest(format!(
                "timeout_ms must be between 1 and {MAX_TIMEOUT_MS}"
            )));
        }
        Ok(())
    }
}

impl SourceSpec {
    pub fn local_directory(path: impl Into<String>) -> Self {
        Self::LocalDirectory { path: path.into() }
    }

    pub fn source_kind(&self) -> &'static str {
        match self {
            Self::LocalDirectory { .. } => "local_directory",
            Self::RemoteSnapshot { .. } | Self::LegacyRemoteSnapshot { .. } => "remote_snapshot",
        }
    }

    fn validate_v3(&self) -> Result<(), RunnerError> {
        match self {
            Self::LocalDirectory { path } => validate_confined_relative_path(path, "source.path"),
            Self::RemoteSnapshot {
                repository_name,
                snapshot_id,
                external_repository_indexes,
                ..
            } => {
                validate_remote_identity(repository_name, "source.repository_name")?;
                validate_remote_identity(snapshot_id, "source.snapshot_id")?;
                if external_repository_indexes.len() > MAX_EXTERNAL_REPOSITORY_ROUTES {
                    return Err(RunnerError::InvalidRequest(format!(
                        "source.external_repository_indexes contains {} entries; maximum is {MAX_EXTERNAL_REPOSITORY_ROUTES}",
                        external_repository_indexes.len()
                    )));
                }
                for name in external_repository_indexes.keys() {
                    validate_remote_identity(name, "source.external_repository_indexes key")?;
                }
                Ok(())
            }
            Self::LegacyRemoteSnapshot { .. } => Err(RunnerError::InvalidRequest(
                "native-job.v3 forbids legacy repo_name/repo_id routing".to_string(),
            )),
        }
    }

    fn validate_legacy(&self) -> Result<(), RunnerError> {
        match self {
            Self::LocalDirectory { path } => validate_confined_relative_path(path, "source.path"),
            Self::LegacyRemoteSnapshot {
                repo_name,
                repo_id,
                snapshot_id,
            } => {
                validate_remote_identity(repo_name, "source.repo_name")?;
                if let Some(repo_id) = repo_id {
                    validate_remote_identity(repo_id, "source.repo_id")?;
                }
                validate_remote_identity(snapshot_id, "source.snapshot_id")
            }
            Self::RemoteSnapshot { .. } => Err(RunnerError::InvalidRequest(
                "legacy native-job.v1 cannot carry Binary Repository index routing".to_string(),
            )),
        }
    }
}

impl NativeResult {
    pub fn encoded_len(&self) -> Result<usize, RunnerError> {
        serde_json::to_vec(self)
            .map(|bytes| bytes.len())
            .map_err(|error| {
                RunnerError::Process(format!("could not encode terminal result: {error}"))
            })
    }

    pub fn validate_bound(&self) -> Result<(), RunnerError> {
        let encoded_len = self.encoded_len()?;
        if encoded_len > MAX_TERMINAL_RESULT_BYTES {
            return Err(RunnerError::Process(format!(
                "terminal result is {encoded_len} bytes; maximum is {MAX_TERMINAL_RESULT_BYTES}"
            )));
        }
        Ok(())
    }
}

pub(crate) fn validate_confined_relative_path(raw: &str, field: &str) -> Result<(), RunnerError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(RunnerError::InvalidRequest(format!(
            "{field} must be a non-empty relative path without NUL bytes"
        )));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(RunnerError::InvalidRequest(format!(
            "{field} must be relative"
        )));
    }
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(RunnerError::InvalidRequest(format!(
                "{field} must stay within its configured root"
            )));
        }
    }
    Ok(())
}

fn validate_optional_label(value: Option<&str>, field: &str) -> Result<(), RunnerError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.trim().is_empty() || value.len() > MAX_LABEL_BYTES || value.contains('\0') {
        return Err(RunnerError::InvalidRequest(format!(
            "{field} must be non-empty, contain no NUL byte, and be at most {MAX_LABEL_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_remote_identity(value: &str, field: &str) -> Result<(), RunnerError> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > MAX_LABEL_BYTES
        || value.contains(['\0', '\r', '\n'])
    {
        return Err(RunnerError::InvalidRequest(format!(
            "{field} must be non-empty, have no surrounding whitespace or control newlines, and be at most {MAX_LABEL_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_environment_key(key: &str) -> Result<(), RunnerError> {
    if key.is_empty()
        || key.contains(['=', '\0'])
        || !key
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
        || key.as_bytes()[0].is_ascii_digit()
    {
        return Err(RunnerError::InvalidRequest(format!(
            "command.environment key `{key}` is not a portable environment variable name"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> NativeJobRequest {
        NativeJobRequest {
            contract: NATIVE_JOB_CONTRACT.to_string(),
            label: None,
            source: SourceSpec::local_directory("."),
            command: CommandSpec {
                argv: vec![
                    NATIVE_JOB_REPOSITORY_CI_ARGV0.to_string(),
                    "patchset".to_string(),
                ],
                working_directory: ".".to_string(),
                environment: BTreeMap::new(),
            },
            timeout_ms: 1_000,
            suite_id: Some("patchset".to_string()),
        }
    }

    #[test]
    fn accepts_exact_contract_and_fixed_entrypoint() {
        request().validate().expect("request should be valid");
    }

    #[test]
    fn rejects_parent_path_and_shell_entrypoint() {
        let mut parent = request();
        parent.source = SourceSpec::local_directory("../source");
        assert!(parent.validate().is_err());

        let mut shell = request();
        shell.command.argv = vec!["sh".to_string(), "-c".to_string(), "true".to_string()];
        assert!(shell.validate().is_err());

        for direct_entrypoint in ["./ci/run.sh", "./ci/run.ps1", "ci/run"] {
            let mut direct = request();
            direct.command.argv = vec![direct_entrypoint.to_string()];
            assert!(direct.validate().is_err(), "accepted {direct_entrypoint}");
        }
    }

    #[test]
    fn rejects_unknown_fields_and_oversized_input() {
        let unknown = br#"{
            "contract":"ait.runner.native-job.v3",
            "source":{"kind":"local_directory","path":"."},
            "command":{"argv":["./ci/run"]},
            "unexpected":true
        }"#;
        assert!(NativeJobRequest::parse_bounded(unknown).is_err());
        assert!(NativeJobRequest::parse_bounded(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());

        let mut previous = request();
        previous.contract = "ait.runner.native-job.v2".to_string();
        assert!(previous.validate().is_err());
    }

    #[test]
    fn accepts_remote_snapshot_without_a_local_path() {
        let mut remote = request();
        remote.source = SourceSpec::RemoteSnapshot {
            repository_index: RepositoryIndex::new(0),
            repository_name: "ait-core".to_string(),
            snapshot_id: "SNP-ABC".to_string(),
            external_repository_indexes: BTreeMap::from([(
                "ait-server".to_string(),
                RepositoryIndex::new(1),
            )]),
        };
        remote.validate().expect("remote source");

        let encoded = serde_json::to_value(remote).expect("encode");
        assert_eq!(encoded["source"]["kind"], "remote_snapshot");
        assert_eq!(encoded["source"]["repository_index"], 0);
        assert!(encoded["source"].get("repo_id").is_none());
        assert!(encoded["source"].get("repo_name").is_none());
        assert!(encoded["source"].get("path").is_none());
    }

    #[test]
    fn v3_rejects_legacy_identity_while_private_compatibility_parser_admits_v1() {
        let legacy = br#"{
            "contract":"ait.runner.native-job.v1",
            "source":{
                "kind":"remote_snapshot",
                "repo_name":"ait-core",
                "repo_id":"R-0",
                "snapshot_id":"SNP-ABC"
            },
            "command":{"argv":["./ci/run.sh"]}
        }"#;
        assert!(NativeJobRequest::parse_bounded(legacy).is_err());
        let parsed = NativeJobRequest::parse_legacy_bounded(legacy).expect("legacy transition");
        assert!(matches!(
            parsed.source,
            SourceSpec::LegacyRemoteSnapshot { .. }
        ));

        let mut v3 = request();
        v3.source = SourceSpec::LegacyRemoteSnapshot {
            repo_name: "ait-core".to_string(),
            repo_id: None,
            snapshot_id: "SNP-ABC".to_string(),
        };
        assert!(v3.validate().is_err());
    }
}
