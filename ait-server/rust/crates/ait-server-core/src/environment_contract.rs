use serde_json::{json, Value as JsonValue};

pub const ENVIRONMENT_CONTRACT_VERSION: &str = "ait.server.environment-contract/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentVariablePurpose {
    Bootstrap,
    Configuration,
    Diagnostic,
    HostBoundary,
}

impl EnvironmentVariablePurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::Configuration => "configuration",
            Self::Diagnostic => "diagnostic",
            Self::HostBoundary => "host_boundary",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvironmentVariableContract {
    pub name: &'static str,
    pub purpose: EnvironmentVariablePurpose,
    pub secret: bool,
    pub description: &'static str,
}

impl EnvironmentVariableContract {
    fn to_json(self) -> JsonValue {
        json!({
            "name": self.name,
            "purpose": self.purpose.as_str(),
            "secret": self.secret,
            "description": self.description,
        })
    }
}

macro_rules! define_environment_contract {
    ($(
        $constant:ident => ($purpose:ident, $secret:literal, $description:literal)
    ),+ $(,)?) => {
        pub mod names {
            $(pub const $constant: &str = stringify!($constant);)+
        }

        pub const ENVIRONMENT_VARIABLES: &[EnvironmentVariableContract] = &[
            $(EnvironmentVariableContract {
                name: names::$constant,
                purpose: EnvironmentVariablePurpose::$purpose,
                secret: $secret,
                description: $description,
            },)+
        ];
    };
}

define_environment_contract!(
    AIT_NATIVE_SERVER_CI_RAM_MIN_AVAILABLE_BYTES => (Configuration, false, "Optional free-byte floor applied when admitting server CI onto the validated RAM root."),
    AIT_NATIVE_SERVER_CI_RAM_RECLAIM_TARGET_BYTES => (Configuration, false, "Optional post-reclamation free-byte target for the validated server CI RAM root."),
    AIT_NATIVE_SERVER_CI_RAM_ROOT => (HostBoundary, false, "Absolute memory-backed host root used for isolated server CI work."),
    AIT_NATIVE_SERVER_DATA => (Bootstrap, false, "Durable server authority root used when --data is not supplied."),
    AIT_OBJECT_PACK_CHUNK_MIB => (Configuration, false, "Positive object-pack write chunk size in mebibytes."),
    AIT_PERFETTO_TRACE => (Diagnostic, false, "Output path enabling opt-in Perfetto trace capture in a tracing-enabled build."),
    AIT_TREE_PACK_CHUNK_MIB => (Configuration, false, "Positive Tree-pack write chunk size in mebibytes."),
);

pub const REMOVED_ENVIRONMENT_NAMES: &[&str] = &[
    "AITSERVER_LISTEN",
    "AIT_CI_RAM_MIN_AVAILABLE_BYTES",
    "AIT_CI_RAM_RECLAIM_TARGET_BYTES",
    "AIT_CI_RAM_ROOT",
    "AIT_LAND_MAIN_SEED_TMPDIR",
    "AIT_MAIN_SEED_ROOT",
    "AIT_NATIVE_QUEUE_MODE",
    "AIT_NATIVE_SERVER_BINARY_ACTIVATION",
    "AIT_NATIVE_SERVER_BINARY_REGISTRY",
    "AIT_NATIVE_SERVER_CARGO_BIN",
    "AIT_NATIVE_SERVER_CI_NICE",
    "AIT_NATIVE_SERVER_CI_STARTUP_ADMISSION",
    "AIT_NATIVE_SERVER_CI_TMP_ROOT",
    "AIT_NATIVE_SERVER_FAST_DATA_ROOT",
    "AIT_NATIVE_SERVER_FRESH_BOOTSTRAP",
    "AIT_NATIVE_SERVER_MAIN_SEED_ROOT",
    "AIT_NATIVE_SERVER_RAM_SHARD_ROOT",
    "AIT_NATIVE_SERVER_RUNTIME_LEASE_REPLICA",
    "AIT_NATIVE_SERVER_WORKER_LEASE_SECONDS",
    "AIT_NATIVE_SERVER_WORKER_RETRY_SECONDS",
    "AIT_PATCHSET_CI_TMPDIR",
    "AIT_PERFETTO_TRACE_MAX_EVENTS",
    "AIT_RAM_SHARD_ROOT",
    "AIT_REPO_CI_TMPDIR",
    "AIT_RUNTIME_DATA",
    "AIT_RUNTIME_ROOT",
    "AIT_SERVER_CI_NICE",
    "AIT_SERVER_FULL_TEST_JOB_CPU_TOKENS",
    "AIT_SERVER_PRESSURE_METRICS_CACHE_TTL_SECONDS",
    "AIT_SERVER_SCHEDULER_POSTURE",
    "AIT_SERVER_STARTUP_PROBE_ONLY",
    "AIT_SERVER_V0_QUEUE_PERF_AUTHORITY",
    "AIT_SERVER_V0_QUEUE_PERF_NAMESPACE",
    "AIT_SERVER_V0_QUEUE_PERF_REPO_ID",
    "AIT_SERVER_V0_QUEUE_PERF_REPO_NAME",
    "AIT_WORKER_QUEUE_TEST_POSTGRES_DSN",
];

pub fn environment_contract_json() -> JsonValue {
    json!({
        "contract": ENVIRONMENT_CONTRACT_VERSION,
        "count": ENVIRONMENT_VARIABLES.len(),
        "variables": ENVIRONMENT_VARIABLES
            .iter()
            .copied()
            .map(EnvironmentVariableContract::to_json)
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn registry_is_sorted_unique_and_excludes_removed_names() {
        let names = ENVIRONMENT_VARIABLES
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert_eq!(names.len(), names.iter().collect::<BTreeSet<_>>().len());
        for removed in REMOVED_ENVIRONMENT_NAMES {
            assert!(
                !names.contains(removed),
                "removed name remains registered: {removed}"
            );
        }
        let mut removed = REMOVED_ENVIRONMENT_NAMES.to_vec();
        removed.sort_unstable();
        assert_eq!(REMOVED_ENVIRONMENT_NAMES, removed);
        assert_eq!(removed.len(), removed.iter().collect::<BTreeSet<_>>().len());
    }

    #[test]
    fn machine_readable_contract_is_complete_and_deterministic() {
        let first = environment_contract_json();
        let second = environment_contract_json();
        assert_eq!(first, second);
        assert_eq!(
            first["count"].as_u64(),
            Some(ENVIRONMENT_VARIABLES.len() as u64)
        );
        assert_eq!(
            first["variables"].as_array().map(Vec::len),
            Some(ENVIRONMENT_VARIABLES.len())
        );
    }
}
