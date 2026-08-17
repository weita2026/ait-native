use crate::json_support::{json, JsonValue};

pub const ENVIRONMENT_CONTRACT_VERSION: &str = "ait.environment-contract/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentVariablePurpose {
    Configuration,
    Credential,
    Bootstrap,
    ProcessBoundary,
    Automation,
    Diagnostic,
}

impl EnvironmentVariablePurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Credential => "credential",
            Self::Bootstrap => "bootstrap",
            Self::ProcessBoundary => "process_boundary",
            Self::Automation => "automation",
            Self::Diagnostic => "diagnostic",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentVariableOwner {
    Core,
    Cli,
    Agent,
    Release,
}

impl EnvironmentVariableOwner {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "ait-core",
            Self::Cli => "ait-cli",
            Self::Agent => "ait-agent-worker",
            Self::Release => "ait-release",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvironmentVariableContract {
    pub name: &'static str,
    pub purpose: EnvironmentVariablePurpose,
    pub owner: EnvironmentVariableOwner,
    pub secret: bool,
    pub description: &'static str,
}

impl EnvironmentVariableContract {
    fn to_json(self) -> JsonValue {
        json!({
            "name": self.name,
            "purpose": self.purpose.as_str(),
            "owner": self.owner.as_str(),
            "secret": self.secret,
            "description": self.description,
        })
    }
}

macro_rules! define_environment_contract {
    ($(
        $constant:ident => ($purpose:ident, $owner:ident, $secret:literal, $description:literal)
    ),+ $(,)?) => {
        pub mod names {
            $(pub const $constant: &str = stringify!($constant);)+
        }

        pub const ENVIRONMENT_VARIABLES: &[EnvironmentVariableContract] = &[
            $(EnvironmentVariableContract {
                name: names::$constant,
                purpose: EnvironmentVariablePurpose::$purpose,
                owner: EnvironmentVariableOwner::$owner,
                secret: $secret,
                description: $description,
            },)+
        ];
    };
}

define_environment_contract!(
    AIT_AGENT_CONFIG_PATH => (Bootstrap, Agent, false, "Path to the typed Agent worker manifest."),
    AIT_DISCORD_APPLICATION_ID => (Credential, Agent, false, "Discord application identity used by the selected worker."),
    AIT_DISCORD_BOT_TOKEN => (Credential, Agent, true, "Discord bot credential for the selected worker."),
    AIT_DISCORD_PUBLIC_KEY => (Credential, Agent, false, "Discord verification public key for the selected worker."),
    AIT_EXTERNAL_CORE_REPO_ROOT => (Bootstrap, Core, false, "Explicit source root for the external-core diagnostic adapter."),
    AIT_LINE_CHANNEL_ACCESS_TOKEN => (Credential, Agent, true, "LINE channel access credential for the selected worker."),
    AIT_LINE_CHANNEL_SECRET => (Credential, Agent, true, "LINE channel verification secret for the selected worker."),
    AIT_NATIVE_ACTOR => (Configuration, Core, false, "Actor identity claim attached to local authorship and remote requests."),
    AIT_NATIVE_SERVER_DATA => (Bootstrap, Core, false, "Durable server data root used when resolving server authority."),
    AIT_OPENAI_API_KEY => (Credential, Agent, true, "OpenAI credential available to the selected Agent worker."),
    AIT_PERFETTO_TRACE => (Diagnostic, Core, false, "Output path for opt-in Perfetto trace capture."),
    AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS => (Configuration, Core, false, "Remote mutation response deadline override."),
    AIT_REMOTE_MUTATION_SETTLE_POLL_SECONDS => (Configuration, Core, false, "Remote mutation reconciliation polling interval."),
    AIT_REMOTE_MUTATION_SETTLE_WINDOW_SECONDS => (Configuration, Core, false, "Remote mutation reconciliation window."),
    AIT_REPO_ROOT => (Bootstrap, Cli, false, "Explicit AIT repository root for process discovery."),
    AIT_RUNTIME_DATA => (Bootstrap, Core, false, "Runtime data root shared by local diagnostics and server-aware operations."),
    AIT_SHARED_CARGO_TARGET_DIR => (Automation, Release, false, "Explicit shared Cargo target directory used by release smoke."),
    AIT_SLACK_APP_TOKEN => (Credential, Agent, true, "Slack app-level credential for the selected worker."),
    AIT_SLACK_SIGNING_SECRET => (Credential, Agent, true, "Slack request verification secret for the selected worker."),
    AIT_TELEGRAM_BOT_TOKEN => (Credential, Agent, true, "Telegram bot credential for the selected worker."),
    AIT_TELEGRAM_OPENAI_API_KEY => (Credential, Agent, true, "Telegram-scoped OpenAI credential for the selected worker."),
    AIT_TELEGRAM_WEBHOOK_SECRET => (Credential, Agent, true, "Telegram webhook verification secret for the selected worker."),
    AIT_WORKSPACE_LOCK_OWNER_TOKEN => (ProcessBoundary, Cli, true, "Opaque token propagated to nested commands sharing a workspace lock."),
);

pub const REMOVED_ENVIRONMENT_NAMES: &[&str] = &[
    "AIT_ACTOR",
    "AIT_ACTOR_TYPE",
    "AIT_AGENT_LOCAL_REPLY_ARGS_JSON",
    "AIT_AGENT_LOCAL_REPLY_PROGRAM",
    "AIT_AGENT_RUST_TRANSPORTS",
    "AIT_AGENT_WORKER_HOST_SIGNAL_CHILD",
    "AIT_AGENT_WORKER_HOST_SIGNAL_READY_MARKER",
    "AIT_ALLOW_RUST_BACKEND_EXPERIMENTS",
    "AIT_CLI_DELEGATE_BIN",
    "AIT_CLI_DELEGATE_ENV_LOG",
    "AIT_CLI_DELEGATE_EXIT_CODE",
    "AIT_CLI_DELEGATE_LOG",
    "AIT_CLI_DELEGATE_STDERR",
    "AIT_CLI_DELEGATE_STDOUT",
    "AIT_CORE_BACKEND",
    "AIT_DISCORD_INTERACTION_SIGNATURE",
    "AIT_DISCORD_INTERACTION_TIMESTAMP",
    "AIT_DISCORD_SIGNATURE",
    "AIT_DISCORD_SIGNATURE_TIMESTAMP",
    "AIT_ENABLE_SPRINT",
    "AIT_GIT_MIRROR_TEST_FAIL_AFTER_TRANSFER",
    "AIT_JSON_MODE",
    "AIT_MAIN_SEED_BENCH_ROOT",
    "AIT_MODEL",
    "AIT_NATIVE_ACTOR_TYPE",
    "AIT_NATIVE_REPOS",
    "AIT_NATIVE_ROLES",
    "AIT_NATIVE_WORKSPACE_ROOT",
    "AIT_NODE_ROOT",
    "AIT_NPM_TAG_TEST_LOG",
    "AIT_OBJECT_PACK_CHUNK_MIB",
    "AIT_PLAN_BACKEND",
    "AIT_PLAN_BLOB_DIFF_BACKEND",
    "AIT_PLAN_CONFIG_RUNTIME_BACKEND",
    "AIT_PLAN_CORE_BACKEND",
    "AIT_PLAN_DIAGNOSTICS_BACKEND",
    "AIT_PLAN_FILESYSTEM_BACKEND",
    "AIT_PLAN_HTTP_BACKEND",
    "AIT_PLAN_PACK_SUBSTRATE_BACKEND",
    "AIT_PLAN_PORTS_PROTOCOLS_BACKEND",
    "AIT_RAM",
    "AIT_RAM_AUTO_MOUNT",
    "AIT_RAM_CAPACITY_BYTES",
    "AIT_RAM_MIN_AVAILABLE_BYTES",
    "AIT_RAM_MOUNT_LOCK_PATH",
    "AIT_RAM_MOUNT_POINT",
    "AIT_RAM_VOLUME_NAME",
    "AIT_RELEASE_HOMEBREW_PYTHON",
    "AIT_RELEASE_MONOREPO_PUBLIC_LAYOUT_SELFTEST",
    "AIT_RELEASE_NATIVE_COMMAND_DIR",
    "AIT_RELEASE_NATIVE_MATRIX_DIR",
    "AIT_RELEASE_NATIVE_SMOKE",
    "AIT_REMOTE_AUTH_HEADER",
    "AIT_REMOTE_SYNC_PACK_PARALLELISM",
    "AIT_REPOS",
    "AIT_ROLES",
    "AIT_RUNTIME_RAM_ROOT",
    "AIT_RUST_EXT_MODULE",
    "AIT_SLACK_SIGNATURE",
    "AIT_SLACK_SIGNATURE_TIMESTAMP",
    "AIT_SPRINT",
    "AIT_TELEGRAM_GRAPH_TRIGGER_WORKER",
    "AIT_TEST_DISABLE_GLOBAL_HOST_RAM_ROOT_CLEANUP",
    "AIT_TEST_OUTSIDE_REPO_TMP",
    "AIT_TEST_RUST_WORKSPACE_ROOT",
    "AIT_TG1_TARGET_ROOT",
    "AIT_TREE_PACK_CHUNK_MIB",
    "AIT_WORKSPACE_LOCK_OWNER_PID",
    "AIT_WORKSPACE_LOCK_PATH",
    "AIT_WORKSPACE_LOCK_ROOT",
    "AIT_WORKSPACE_ROOT",
    "AIT_WORKTREE_LINE",
    "AIT_WORKTREE_NAME",
    "AIT_WORKTREE_PATH",
    "CODEX_BIN",
    "CODEX_MODEL",
    "CODEX_REASONING_EFFORT",
    "CODEX_SANDBOX",
    "X_SLACK_REQUEST_TIMESTAMP",
    "X_SLACK_SIGNATURE",
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
        assert_eq!(ENVIRONMENT_VARIABLES.len(), 23);
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
