use serde_json::{Value as JsonValue, json};

pub const ENVIRONMENT_CONTRACT_VERSION: &str = "ait.runner.environment-contract/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentVariableScope {
    ChildProcess,
    RuntimeCredential,
}

impl EnvironmentVariableScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChildProcess => "child_process",
            Self::RuntimeCredential => "runtime_credential",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvironmentVariableContract {
    pub name: &'static str,
    pub scope: EnvironmentVariableScope,
    pub secret: bool,
    pub family: bool,
    pub description: &'static str,
}

impl EnvironmentVariableContract {
    pub fn matches(self, candidate: &str) -> bool {
        if !self.family {
            return candidate == self.name;
        }
        let Some(external_name) = candidate
            .strip_prefix(names::EXTERNAL_REPO_ROOT_PREFIX)
            .and_then(|value| value.strip_suffix(names::EXTERNAL_REPO_ROOT_SUFFIX))
        else {
            return false;
        };
        !external_name.is_empty()
            && external_name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            && external_name.bytes().any(|byte| byte != b'_')
    }

    fn to_json(self) -> JsonValue {
        json!({
            "name": self.name,
            "scope": self.scope.as_str(),
            "secret": self.secret,
            "family": self.family,
            "description": self.description,
        })
    }
}

pub mod names {
    pub const EXTERNAL_REPO_ROOT_PATTERN: &str = "AIT_EXTERNAL_<NAME>_REPO_ROOT";
    pub const EXTERNAL_REPO_ROOT_PREFIX: &str = "AIT_EXTERNAL_";
    pub const EXTERNAL_REPO_ROOT_SUFFIX: &str = "_REPO_ROOT";
    pub const AIT_RUNNER_ATTEMPT_ROOT: &str = "AIT_RUNNER_ATTEMPT_ROOT";
    pub const AIT_RUNNER_WORKSPACE: &str = "AIT_RUNNER_WORKSPACE";
    pub const AIT_SERVER_TOKEN: &str = "AIT_SERVER_TOKEN";
}

pub const ENVIRONMENT_VARIABLES: &[EnvironmentVariableContract] = &[
    EnvironmentVariableContract {
        name: names::EXTERNAL_REPO_ROOT_PATTERN,
        scope: EnvironmentVariableScope::ChildProcess,
        secret: false,
        family: true,
        description: "Runner-created path to one locked external repository materialized for the child process.",
    },
    EnvironmentVariableContract {
        name: names::AIT_RUNNER_ATTEMPT_ROOT,
        scope: EnvironmentVariableScope::ChildProcess,
        secret: false,
        family: false,
        description: "Runner-owned root of the current execution attempt, injected into the child process.",
    },
    EnvironmentVariableContract {
        name: names::AIT_RUNNER_WORKSPACE,
        scope: EnvironmentVariableScope::ChildProcess,
        secret: false,
        family: false,
        description: "Materialized workspace for the current execution, injected into the child process.",
    },
    EnvironmentVariableContract {
        name: names::AIT_SERVER_TOKEN,
        scope: EnvironmentVariableScope::RuntimeCredential,
        secret: true,
        family: false,
        description: "Optional bearer credential used by the runner when calling ait-server.",
    },
];

pub const REMOVED_ENVIRONMENT_NAMES: &[&str] = &[
    "AIT_RELEASE_TARGET",
    "AIT_RUNNER_WORKER_ID",
    "AIT_SERVER_URL",
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
    fn registry_is_sorted_unique_and_matches_only_bounded_families() {
        let names = ENVIRONMENT_VARIABLES
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert_eq!(names.len(), names.iter().collect::<BTreeSet<_>>().len());

        let external = ENVIRONMENT_VARIABLES
            .iter()
            .copied()
            .find(|entry| entry.family)
            .expect("external root family");
        assert!(external.matches("AIT_EXTERNAL_CORE_REPO_ROOT"));
        assert!(external.matches("AIT_EXTERNAL_COMPANY_SDK_REPO_ROOT"));
        assert!(!external.matches("AIT_EXTERNAL__REPO_ROOT"));
        assert!(!external.matches("AIT_EXTERNAL_core_REPO_ROOT"));
        assert!(!external.matches("AIT_EXTERNAL_CORE_OTHER"));

        let mut removed = REMOVED_ENVIRONMENT_NAMES.to_vec();
        removed.sort_unstable();
        assert_eq!(REMOVED_ENVIRONMENT_NAMES, removed);
        assert!(REMOVED_ENVIRONMENT_NAMES.iter().all(|removed| {
            ENVIRONMENT_VARIABLES
                .iter()
                .all(|entry| !entry.matches(removed))
        }));
    }

    #[test]
    fn machine_readable_contract_is_deterministic_and_marks_only_the_token_secret() {
        let first = environment_contract_json();
        assert_eq!(first, environment_contract_json());
        assert_eq!(
            first["count"].as_u64(),
            Some(ENVIRONMENT_VARIABLES.len() as u64)
        );
        let secrets = ENVIRONMENT_VARIABLES
            .iter()
            .filter(|entry| entry.secret)
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(secrets, vec![names::AIT_SERVER_TOKEN]);
    }
}
