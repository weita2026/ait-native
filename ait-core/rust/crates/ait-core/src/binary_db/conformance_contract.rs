use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};

pub const BINARY_DB_CONFORMANCE_CONTRACT_VERSION: &str = "ait.binary-db.substrate.v1";
pub const BINARY_DB_CONFORMANCE_CONTRACT_CHECKSUM: &str =
    "59b12ddf771158520e551bb9be3b96a7b9e7035df055cb044d1aa42ebe77a217";

pub const BINARY_DB_CONFORMANCE_ARCHITECTURE: &str = "Binary DB files begin with a little-endian u32 layout header. Reads select a supported codec from the persisted header; configured writer layout never reinterprets existing bytes. Fixed-record bodies remain aligned and assign dense zero-based indexes. Variable and fixed secondary indexes preserve exact key/value encodings, including index-plus-one normalization. Payload ranges are bounded by the persisted file. Mutation authority is transaction-owned, scoped to explicit file families, rejects unsafe paths before mutation, and is invalid after commit, abort, or drop. Fixed-record overwrite and multi-file record/payload/index transactions restore every before-image on abort or failed durability. Validation and failed I/O do not publish partial state. Once all touched data reaches its durable commit point, later lock cleanup is observable outcome metadata and never changes the operation into a retryable failure. Compact Plan layout-1 record and payload codecs are pinned by complete cross-repository golden bytes. General is an all-family recovery/admin composite and conflicts with every writer family. Server implementations may extend durability with persistent journals and server-only aggregate families without weakening these outcomes.";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryDbContractOutcome {
    Success,
    RetryableBusy,
    Corruption,
    LayoutMismatch,
    MissingData,
    InvalidDomainData,
    Io,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryDbContractMutation {
    None,
    CommitExactBytes,
    RestoreBeforeImage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryDbContractTransactionState {
    Preparing,
    Active,
    Committed,
    Aborted,
    Recovered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryDbContractFileFamily {
    Content,
    Snapshot,
    Plan,
    RemoteContent,
    RemotePlan,
    Workflow,
    RepositoryMetadata,
    RecoveryAdmin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BinaryDbBehaviorContract {
    pub id: &'static str,
    pub category: &'static str,
    pub expected_outcome: BinaryDbContractOutcome,
    pub expected_mutation: BinaryDbContractMutation,
    pub core_gate: &'static str,
    pub server_gate: &'static str,
}

pub const BINARY_DB_BEHAVIOR_CONTRACTS: &[BinaryDbBehaviorContract] = &[
    BinaryDbBehaviorContract {
        id: "missing_record_file_is_empty",
        category: "record_header",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "empty_record_file_is_corrupt",
        category: "record_header",
        expected_outcome: BinaryDbContractOutcome::Corruption,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "truncated_header_is_corrupt",
        category: "record_header",
        expected_outcome: BinaryDbContractOutcome::Corruption,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "header_only_record_file_has_zero_records",
        category: "fixed_record",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "misaligned_record_body_is_corrupt",
        category: "fixed_record",
        expected_outcome: BinaryDbContractOutcome::Corruption,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "append_assigns_dense_zero_based_index",
        category: "fixed_record",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "record_index_out_of_bounds_is_missing",
        category: "fixed_record",
        expected_outcome: BinaryDbContractOutcome::MissingData,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "payload_range_out_of_bounds_is_missing",
        category: "payload",
        expected_outcome: BinaryDbContractOutcome::MissingData,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "unsupported_persisted_layout_fails_closed",
        category: "persisted_layout",
        expected_outcome: BinaryDbContractOutcome::LayoutMismatch,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "record_header_layout_mismatch_fails_closed",
        category: "persisted_layout",
        expected_outcome: BinaryDbContractOutcome::LayoutMismatch,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_conformance_vectors_v2",
        server_gate: "server_binary_db_conformance_vectors_v2",
    },
    BinaryDbBehaviorContract {
        id: "validation_failure_precedes_mutation",
        category: "transaction",
        expected_outcome: BinaryDbContractOutcome::InvalidDomainData,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_transaction_conformance_v2",
        server_gate: "server_binary_db_transaction_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "injected_write_failure_restores_before_image",
        category: "transaction",
        expected_outcome: BinaryDbContractOutcome::Io,
        expected_mutation: BinaryDbContractMutation::RestoreBeforeImage,
        core_gate: "binary_db_transaction_conformance_v2",
        server_gate: "server_binary_db_transaction_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "injected_sync_failure_restores_before_image",
        category: "transaction",
        expected_outcome: BinaryDbContractOutcome::Io,
        expected_mutation: BinaryDbContractMutation::RestoreBeforeImage,
        core_gate: "binary_db_transaction_conformance_v2",
        server_gate: "server_binary_db_transaction_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "abort_restores_before_image",
        category: "transaction",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::RestoreBeforeImage,
        core_gate: "binary_db_transaction_conformance_v2",
        server_gate: "server_binary_db_transaction_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "commit_publishes_exact_bytes",
        category: "transaction",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_transaction_conformance_v2",
        server_gate: "server_binary_db_transaction_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "post_commit_lock_cleanup_failure_is_committed",
        category: "transaction",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_transaction_conformance_v2",
        server_gate: "server_binary_db_transaction_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "recovery_is_idempotent",
        category: "transaction",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::RestoreBeforeImage,
        core_gate: "binary_db_transaction_conformance_v2",
        server_gate: "server_binary_db_transaction_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "variable_index_exact_bytes_and_lookup",
        category: "secondary_index",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "fixed_index_plus_one_exact_bytes_and_lookup",
        category: "secondary_index",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "overwrite_commit_publishes_exact_bytes",
        category: "overwrite",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "overwrite_abort_restores_before_image",
        category: "overwrite",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::RestoreBeforeImage,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "overwrite_sync_failure_restores_before_image",
        category: "overwrite",
        expected_outcome: BinaryDbContractOutcome::Io,
        expected_mutation: BinaryDbContractMutation::RestoreBeforeImage,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "parent_path_is_rejected_before_mutation",
        category: "path_family_binding",
        expected_outcome: BinaryDbContractOutcome::InvalidDomainData,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "unauthorized_plan_family_is_rejected_before_mutation",
        category: "path_family_binding",
        expected_outcome: BinaryDbContractOutcome::InvalidDomainData,
        expected_mutation: BinaryDbContractMutation::None,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "authorized_plan_family_commits",
        category: "path_family_binding",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "multi_file_commit_publishes_all_exact_bytes",
        category: "multi_file_transaction",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "multi_file_abort_restores_all_before_images",
        category: "multi_file_transaction",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::RestoreBeforeImage,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "multi_file_sync_failure_restores_existing_files",
        category: "multi_file_transaction",
        expected_outcome: BinaryDbContractOutcome::Io,
        expected_mutation: BinaryDbContractMutation::RestoreBeforeImage,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
    BinaryDbBehaviorContract {
        id: "multi_file_post_commit_lock_cleanup_failure_is_committed",
        category: "multi_file_transaction",
        expected_outcome: BinaryDbContractOutcome::Success,
        expected_mutation: BinaryDbContractMutation::CommitExactBytes,
        core_gate: "binary_db_extended_conformance_v2",
        server_gate: "server_binary_db_extended_conformance_v2",
    },
];

pub const BINARY_DB_TRANSACTION_STATES: &[BinaryDbContractTransactionState] = &[
    BinaryDbContractTransactionState::Preparing,
    BinaryDbContractTransactionState::Active,
    BinaryDbContractTransactionState::Committed,
    BinaryDbContractTransactionState::Aborted,
    BinaryDbContractTransactionState::Recovered,
];

pub const BINARY_DB_FILE_FAMILY_VOCABULARY: &[BinaryDbContractFileFamily] = &[
    BinaryDbContractFileFamily::Content,
    BinaryDbContractFileFamily::Snapshot,
    BinaryDbContractFileFamily::Plan,
    BinaryDbContractFileFamily::RemoteContent,
    BinaryDbContractFileFamily::RemotePlan,
    BinaryDbContractFileFamily::Workflow,
    BinaryDbContractFileFamily::RepositoryMetadata,
    BinaryDbContractFileFamily::RecoveryAdmin,
];

fn binary_db_conformance_contract_payload() -> JsonValue {
    json!({
        "version": BINARY_DB_CONFORMANCE_CONTRACT_VERSION,
        "architecture": BINARY_DB_CONFORMANCE_ARCHITECTURE,
        "general_scope": {
            "purpose": "recovery_admin_all_family_composite",
            "conflicts_with_every_writer_family": true,
            "normal_domain_scope": false
        },
        "transaction_states": BINARY_DB_TRANSACTION_STATES,
        "file_families": BINARY_DB_FILE_FAMILY_VOCABULARY,
        "behaviors": BINARY_DB_BEHAVIOR_CONTRACTS,
        "observable_fixture_versions": {
            "substrate": "ait.binary-db.conformance-vectors.v2",
            "plan_golden": "ait.plan-binary-db.golden-bytes.v1",
            "parity_manifest": "ait.binary-db.cross-repo-parity-manifest.v1"
        },
        "intentional_server_extensions": [
            "persistent_rollback_journal",
            "server_only_aggregate_scopes",
            "workflow_layout_2_payload_envelopes"
        ],
        "shared_runtime_crate": false,
        "ait_external_required": false
    })
}

pub fn binary_db_conformance_contract_checksum() -> String {
    let encoded = serde_json::to_vec(&binary_db_conformance_contract_payload())
        .expect("static Binary DB conformance contract must serialize");
    format!("{:x}", Sha256::digest(encoded))
}

pub fn binary_db_conformance_contract_json() -> JsonValue {
    let mut payload = binary_db_conformance_contract_payload();
    payload
        .as_object_mut()
        .expect("contract payload is object")
        .insert(
            "checksum".to_string(),
            JsonValue::String(binary_db_conformance_contract_checksum()),
        );
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_db::BinaryDbCommandScope;
    use std::collections::BTreeSet;

    #[test]
    fn binary_db_cross_repository_contract_is_versioned_and_machine_readable() {
        let payload = binary_db_conformance_contract_json();
        assert_eq!(
            payload["version"],
            JsonValue::String(BINARY_DB_CONFORMANCE_CONTRACT_VERSION.to_string())
        );
        assert_eq!(
            payload["checksum"],
            JsonValue::String(binary_db_conformance_contract_checksum())
        );
        assert_eq!(
            binary_db_conformance_contract_checksum(),
            BINARY_DB_CONFORMANCE_CONTRACT_CHECKSUM
        );
        assert_eq!(payload["shared_runtime_crate"], JsonValue::Bool(false));
        assert_eq!(payload["ait_external_required"], JsonValue::Bool(false));
        assert!(BINARY_DB_BEHAVIOR_CONTRACTS.iter().all(|behavior| {
            !behavior.core_gate.is_empty() && !behavior.server_gate.is_empty()
        }));
    }

    #[test]
    fn binary_db_general_scope_is_the_all_family_recovery_admin_composite() {
        assert_eq!(
            BinaryDbCommandScope::General.lock_file_names(),
            BinaryDbCommandScope::all_write_lock_file_names()
        );
        let general = BinaryDbCommandScope::General
            .lock_file_names()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(general.contains("global.write.lock"));
        for scope in [
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbCommandScope::PlanSyncLocalPlan,
            BinaryDbCommandScope::PlanSyncRemote,
            BinaryDbCommandScope::RemoteSyncLocalImport,
            BinaryDbCommandScope::PlanImport,
            BinaryDbCommandScope::SnapshotWrite,
            BinaryDbCommandScope::ContentWrite,
            BinaryDbCommandScope::Gc,
        ] {
            assert!(BinaryDbCommandScope::General.conflicts_with(scope));
            assert!(scope.conflicts_with(BinaryDbCommandScope::General));
            assert!(scope
                .lock_file_names()
                .iter()
                .all(|name| general.contains(name)));
        }
    }
}
