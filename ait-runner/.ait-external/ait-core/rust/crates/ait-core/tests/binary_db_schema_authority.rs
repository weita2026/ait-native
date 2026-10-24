use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::PathBuf, sync::OnceLock};

const BINARY_DB_V0_SHA256: &str =
    "668ee2276e51d962223c18a418b6591aa8f813b95d5a526547d2f277d08e4817";
const PATCHSET_RECORD_LAYOUT: &str = r#"SERVER_PATCHSET_RECORD_SIZE = 65

ServerPatchsetRecord — patchset.bin:
u8  patchset_meta
u8  patch_ordinal
u8  change_ordinal
u8  reserved0
u32 change_index
u32 previous_task_patchset_index_plus1
u32 previous_change_patchset_index_plus1
u32 base_snapshot_index
u32 revision_snapshot_index
u64 created_at_s
u64 ci_completed_at_s
u32 ci_run_seq
u16 ci_selected_suite_count
u16 ci_suite_result_count
u16 ci_blocking_failure_count
u8  ci_status_bits
u64 summary_offset
u16 summary_len
u32 ci_worker_job_index_plus1"#;
const PATCHSET_CI_STATUS_LAYOUT: &str = r#"ServerPatchsetRecord.ci_status_bits:
bits 0..1 overall_status
bits 2..3 tests_status
bits 4..5 lint_status
bits 6..7 reserved = 0

Each two-bit status:
00 none
01 pass
10 fail
11 error"#;
const PATCHSET_SUMMARY_PAYLOAD_LAYOUT: &str = r#"PatchsetSummaryPayload — patchset_summary_payload.bin:
u8  summary_bytes[summary_len]"#;
const TASK_POLICY_INDEX_LAYOUT: &str = r#"TASK_POLICY_INDEX_RECORD_SIZE = 8

TaskPolicyIndexRecord — task_policy_index.bin:
u32 latest_policy_index_plus1
u16 policy_count
u16 reserved0"#;
const PATCHSET_POLICY_INDEX_LAYOUT: &str = r#"PATCHSET_POLICY_INDEX_RECORD_SIZE = 8

PatchsetPolicyIndexRecord — patchset_policy_index.bin:
u32 latest_policy_index_plus1
u16 policy_count
u8  next_policy_ordinal
u8  reserved0"#;
const TASK_PATCHSET_INDEX_LAYOUT: &str = r#"TASK_PATCHSET_INDEX_RECORD_SIZE = 8

TaskPatchsetIndexRecord — task_patchset_index.bin:
u32 latest_patchset_index_plus1
u16 patchset_count
u16 reserved0"#;
const CHANGE_PATCHSET_INDEX_LAYOUT: &str = r#"CHANGE_PATCHSET_INDEX_RECORD_SIZE = 8

ChangePatchsetIndexRecord — change_patchset_index.bin:
u32 latest_patchset_index_plus1
u16 patchset_count
u8  next_patch_ordinal
u8  reserved0"#;
const TASK_REVIEW_INDEX_LAYOUT: &str = r#"TASK_REVIEW_INDEX_RECORD_SIZE = 8

TaskReviewIndexRecord — task_review_index.bin:
u32 latest_review_index_plus1
u16 review_count
u16 reserved0"#;
const PATCHSET_REVIEW_INDEX_LAYOUT: &str = r#"PATCHSET_REVIEW_INDEX_RECORD_SIZE = 8

PatchsetReviewIndexRecord — patchset_review_index.bin:
u32 latest_review_index_plus1
u16 review_count
u8  next_review_ordinal
u8  reserved0"#;
const TASK_LAND_INDEX_LAYOUT: &str = r#"TASK_LAND_INDEX_RECORD_SIZE = 8

TaskLandIndexRecord — task_land_index.bin:
u32 latest_land_index_plus1
u16 land_count
u16 reserved0"#;
const CHANGE_LAND_INDEX_LAYOUT: &str = r#"CHANGE_LAND_INDEX_RECORD_SIZE = 8

ChangeLandIndexRecord — change_land_index.bin:
u32 latest_land_index_plus1
u16 land_count
u8  next_land_ordinal
u8  reserved0"#;
const REMOTE_CHANGE_RECORD_LAYOUT: &str = r#"REMOTE_CHANGE_RECORD_SIZE = 68

RemoteChangeRecord — change.bin:
u8  change_meta
u8  remote_meta
u16 payload_len
u8  change_ordinal
u8  change_state
u16 reserved1
u64 payload_offset
u32 task_index
u32 previous_change_index_plus1
u32 selected_patchset_index_plus1
u32 fork_snapshot_index_plus1
u64 created_at_s
u64 updated_at_s
u64 fetched_at_s
u32 base_line_index_plus1
u64 archived_at_s"#;
const SNAPSHOT_PARENT_EDGE_LAYOUT: &str = r#"SNAPSHOT_PARENT_EDGE_RECORD_SIZE = 12

SnapshotParentEdgeRecord — snapshot_parent_edge.bin:
u32 child_snapshot_index
u32 parent_snapshot_index
u16 parent_ordinal
u16 flags"#;
const TAG_RECORD_LAYOUT: &str = r#"TAG_RECORD_SIZE = 24

TagRecord — tag.bin:
u8  tag_meta
u8  reserved0
u16 payload_len
u64 payload_offset
u32 snapshot_index
u64 created_at_s"#;
const LOCAL_TASK_RECORD_LAYOUT: &str = r#"LOCAL_TASK_RECORD_SIZE = 64

LocalTaskRecord — task.bin:
u8  task_meta
u8  local_meta
u16 payload_len
u64 payload_offset
u32 origin_plan_revision_index_plus1
u32 plan_item_index_plus1
u32 published_remote_task_index
u64 created_at_s
u64 updated_at_s
u64 plan_linked_at_s
u64 published_at_s
u64 closed_at_s"#;
const REMOTE_TASK_RECORD_LAYOUT: &str = r#"REMOTE_TASK_RECORD_SIZE = 60

RemoteTaskRecord — task.bin:
u8  task_meta
u8  remote_meta
u16 payload_len
u64 payload_offset
u32 origin_plan_revision_index_plus1
u32 plan_item_index_plus1
u64 created_at_s
u64 updated_at_s
u64 plan_linked_at_s
u64 fetched_at_s
u64 closed_at_s"#;
const LOCAL_CHANGE_RECORD_LAYOUT: &str = r#"LOCAL_CHANGE_RECORD_SIZE = 68

LocalChangeRecord — change.bin:
u8  change_meta
u8  local_meta
u16 payload_len
u8  change_ordinal
u8  change_state
u16 reserved1
u64 payload_offset
u32 task_index
u32 previous_change_index_plus1
u32 fork_snapshot_index_plus1
u8  published_remote_change_ordinal_plus1
u8  reserved2
u16 reserved3
u64 created_at_s
u64 updated_at_s
u64 published_at_s
u32 base_line_index_plus1
u64 archived_at_s"#;
const LOCAL_LAND_RECORD_LAYOUT: &str = r#"LOCAL_LAND_RECORD_SIZE = 44

LocalLandRecord — land.bin:
u8  land_meta
u8  land_ordinal
u8  change_ordinal
u8  failure_kind
u32 change_index
u32 previous_task_land_index_plus1
u32 previous_change_land_index_plus1
u32 pre_land_target_snapshot_index_plus1
u32 landed_snapshot_index_plus1
u64 submitted_at_s
u64 updated_at_s
u32 target_line_index_plus1"#;
const SERVER_LAND_RECORD_LAYOUT: &str = r#"SERVER_LAND_RECORD_SIZE = 48

ServerLandRecord — land.bin:
u8  land_meta
u8  land_ordinal
u8  change_ordinal
u8  failure_kind
u32 change_index
u32 patchset_index
u32 previous_task_land_index_plus1
u32 previous_change_land_index_plus1
u32 pre_land_target_snapshot_index_plus1
u32 landed_snapshot_index_plus1
u64 submitted_at_s
u64 updated_at_s
u32 target_line_index_plus1"#;
const SNAPSHOT_HISTORY_RECORD_LAYOUT: &str = r#"CONTENT_SNAPSHOT_RECORD_SIZE = 88

ContentSnapshotRecord — snapshot.bin:
u8  snapshot_meta
u8  history_flags
u16 payload_len
u64 payload_offset
u64 snapshot_hash48
u32 parent_snapshot_index_plus1
u32 root_tree_pack_index_plus1
u32 root_entry_ordinal
u32 line_index_plus1
u8  manifest_hash[32]
u32 file_count
u64 total_bytes
u64 created_at_s"#;
const TREE_RECORD_LAYOUT: &str = r#"TREE_RECORD_SIZE = 20

TreeRecord — tree.bin:
u8  tree_meta
u8  reserved0
u32 pack_entry_ordinal
u32 entry_count
u8  tree_hash80[10]"#;
const TREE_ENTRY_RANGE_RECORD_LAYOUT: &str = r#"TREE_ENTRY_RANGE_RECORD_SIZE = 4

TreeEntryRangeRecord — tree_entry_range.bin:
u32 first_entry_index"#;
const ACTOR_RECORD_LAYOUT: &str = r#"ACTOR_RECORD_SIZE = 36

ActorRecord — actor.bin:
u8  actor_meta
u8  reserved0
u16 payload_len
u64 payload_offset
u64 actor_key_hash
u64 created_at_s
u64 last_seen_at_s"#;
const ACTOR_PAYLOAD_LAYOUT: &str = r#"ActorPayload — actor_payload.bin:
u8  user_name_len
u8  user_id_len
u8  email_len
u8  user_name_bytes[user_name_len]
u8  user_id_bytes[user_id_len]
u8  email_bytes[email_len]
u8  memo_bytes[payload_len - 3 - user_name_len - user_id_len - email_len]"#;
const PATCHSET_PUBLISH_STATE_LAYOUT: &str = r#"bits 5..6 publish_state_kind:
  00 published
  01 reserved
  10 superseded
  11 reserved"#;
const FIXED_SCHEMA_BIT_MARKERS: &[&str] = &[
    "bit 7 superseded; valid only with lifecycle 11",
    "ChangeRecord.change_state:\nbit 0 canceled; valid only with lifecycle 11\nbits 1..7 reserved = 0",
    "ContentSnapshotRecord.history_flags:\nbit 0 remote_head_history_boundary\nbits 1..7 reserved = 0",
    "bit 2 sparse_physical_ordinals",
];
const BIN_TO_BIN_SCHEMA_MARKERS: &[&str] = &[
    "bits 2..4 author_mode_kind:",
    PATCHSET_PUBLISH_STATE_LAYOUT,
    "bit 7 evaluation_pending",
    "bit 3 require_tests_pass",
    "bit 4 require_human_review",
    "bit 5 require_lint_pass",
    "bit 6 ci_backed",
    "bit 4 task_lane",
    "bit 5 code_review_summary",
    "bit 6 defer",
    "bits 5..6 mode_kind:",
    "detail_flags bit 0 = blocking suite",
    "detail_flags bit 1 = informational suite",
];
const GIT_RECORD_LAYOUT_MARKERS: &[&str] = &[
    "GIT_REPOSITORY_RECORD_SIZE = 28\n\nGitRepositoryRecord — git_repository.bin:",
    "GIT_GENERATION_RECORD_SIZE = 20\n\nGitGenerationRecord — git_generation.bin:",
    "GIT_IDENTITY_RECORD_SIZE = 24\n\nGitIdentityRecord — git_identity.bin:",
    "GIT_COMMIT_MAPPING_RECORD_SIZE = 96\n\nGitCommitMappingRecord — git_commit_mapping.bin:",
    "GIT_COMMIT_PARENT_RECORD_SIZE = 28\n\nGitCommitParentRecord — git_commit_parent.bin:",
    "GIT_FILE_MAPPING_RECORD_SIZE = 52\n\nGitFileMappingRecord — git_file_mapping.bin:",
    "GIT_REF_MAPPING_RECORD_SIZE = 80\n\nGitRefMappingRecord — git_ref_mapping.bin:",
    "GIT_TAG_MAPPING_RECORD_SIZE = 48\n\nGitTagMappingRecord — git_tag_mapping.bin:",
    "GIT_OPERATION_CHECKPOINT_RECORD_SIZE = 44\n\nGitOperationCheckpointRecord — git_operation_checkpoint.bin:",
];
const OPERATIONAL_REPOSITORY_RECORD_LAYOUT: &str = r#"OPERATIONAL_REPOSITORY_RECORD_SIZE = 33

OperationalRepositoryRecord — repository.bin:
u8  repository_meta
u8  lifecycle_kind
u8  namespace_ascii[2]
u8  policy_flags
u32 payload_len
u64 payload_offset
u64 created_at_s
u64 updated_at_s"#;
const OPERATIONAL_REPOSITORY_PAYLOAD_LAYOUT: &str = r#"OperationalRepositoryPayload — repository_payload.bin:
u16 repo_name_len
u8  repo_name_bytes[repo_name_len]"#;
const OPERATIONAL_NAMESPACE_INDEX_LAYOUT: &str = r#"OPERATIONAL_NAMESPACE_INDEX_RECORD_SIZE = 8

OperationalNamespaceIndexRecord — repository_namespace.idx:
u8  namespace_ascii[2]
u16 reserved0
u32 repository_index_plus1"#;
const OPERATIONAL_NAMESPACE_ENCODING: &str = r#"namespace_ascii[2]:
[0x00, 0x00] empty
[x,    0x00] one-byte namespace
[x,    y   ] two-byte namespace"#;
const OPERATIONAL_REPOSITORY_POLICY_LAYOUT: &str = r#"policy_flags:
bit 0 require_attestation
bit 1 require_tests
bit 2 require_lint
bit 3 require_security_scan
bit 4 require_license_scan
bit 5 require_ai_provenance
bit 6 require_code_review_summary
bit 7 docs_only_relaxed_checks"#;
const SERVER_WORKER_JOB_RECORD_LAYOUT: &str = r#"SERVER_WORKER_JOB_RECORD_SIZE = 52

ServerWorkerJobRecord — worker_job.bin:
u8  job_meta
u8  job_kind
u8  state_kind
u8  outcome_kind
u16 attempt_count
u16 max_attempts
u16 error_kind
u16 reserved0
u32 patchset_index_plus1
u32 snapshot_index_plus1
u64 available_at_s
u64 locked_at_s
u64 created_at_s
u64 updated_at_s"#;
const SERVER_WORKER_JOB_STATE_LAYOUT: &str = r#"state_kind:
1 queued
2 running
3 succeeded
4 failed

outcome_kind:
0 none
1 completed
2 skipped
3 attached
4 superseded
5 failed

error_kind:
0 none
1 retryable_execution
2 terminal_execution
3 lease_expired"#;
const SERVER_WORKER_READY_INDEX_LAYOUT: &str = r#"SERVER_WORKER_READY_INDEX_RECORD_SIZE = 12

ServerWorkerReadyIndexRecord — worker_ready.idx:
u64 available_at_s
u32 worker_job_index_plus1"#;
const SERVER_WORKER_STATE_INDEX_LAYOUT: &str = r#"SERVER_WORKER_STATE_INDEX_RECORD_SIZE = 8

ServerWorkerStateIndexRecord — worker_state.idx:
u8  state_kind
u8  reserved0
u16 reserved1
u32 worker_job_index_plus1"#;
const OPERATIONAL_FIXED_LAYOUT_MARKERS: &[&str] = &["OPERATIONAL_REPOSITORY_RECORD_SIZE = 33"];
const OPERATIONAL_PAYLOAD_FILE_MARKERS: &[&str] = &["repository_payload.bin"];
const OPERATIONAL_INDEX_FILE_MARKERS: &[&str] = &["repository_namespace.idx"];
const OPERATIONAL_EXCLUDED_LAYOUT_MARKERS: &[&str] = &[
    "OPERATIONAL_REPOSITORY_GROUP_RECORD_SIZE",
    "OPERATIONAL_REPOSITORY_GROUP_MEMBER_RECORD_SIZE",
    "OPERATIONAL_REPOSITORY_ROLE_BINDING_RECORD_SIZE",
    "OPERATIONAL_RELEASE_RECORD_SIZE",
    "OPERATIONAL_AUDIT_EVENT_RECORD_SIZE",
    "OPERATIONAL_AUTHORITY_MAP_RECORD_SIZE",
    "OPERATIONAL_AUTHORITY_NODE_RECORD_SIZE",
    "OPERATIONAL_AUTHORITY_MUTATION_RECORD_SIZE",
    "OPERATIONAL_REPOSITORY_RETIREMENT_RECORD_SIZE",
    "OPERATIONAL_STACK_RECORD_SIZE",
    "OPERATIONAL_STACK_CHANGE_RECORD_SIZE",
    "OPERATIONAL_SCOPED_KEY_INDEX_RECORD_SIZE",
    "OPERATIONAL_RELATION_INDEX_RECORD_SIZE",
    "OPERATIONAL_OWNER_ORDER_INDEX_RECORD_SIZE",
    "OPERATIONAL_REPOSITORY_TIME_INDEX_RECORD_SIZE",
    "OPERATIONAL_OWNER_TIME_INDEX_RECORD_SIZE",
    "OPERATIONAL_TEXT_TIME_INDEX_RECORD_SIZE",
    "OPERATIONAL_ID_INDEX_RECORD_SIZE",
    "OPERATIONAL_SESSION_RECORD_SIZE",
    "OPERATIONAL_SESSION_EVENT_RECORD_SIZE",
    "OPERATIONAL_SESSION_CHECKPOINT_RECORD_SIZE",
    "OPERATIONAL_PLANNING_SESSION_RECORD_SIZE",
    "OPERATIONAL_PLANNING_SESSION_EVENT_RECORD_SIZE",
    "OPERATIONAL_COMMUNITY_ACCOUNT_RECORD_SIZE",
    "OPERATIONAL_COMMUNITY_EXTERNAL_IDENTITY_RECORD_SIZE",
    "OPERATIONAL_COMMUNITY_PASSWORD_CREDENTIAL_RECORD_SIZE",
    "OPERATIONAL_COMMUNITY_WEB_SESSION_RECORD_SIZE",
    "OPERATIONAL_TEST_CASE_RECORD_SIZE",
    "OPERATIONAL_TEST_GROUP_RECORD_SIZE",
    "OPERATIONAL_TEST_GROUP_MEMBER_RECORD_SIZE",
    "OPERATIONAL_TASK_TEST_CASE_LINK_RECORD_SIZE",
    "OPERATIONAL_TASK_TEST_GROUP_RECORD_SIZE",
];
const OPERATIONAL_EXCLUDED_PAYLOAD_MARKERS: &[&str] = &[
    "repository_group_payload.bin",
    "repository_role_binding_payload.bin",
    "release_payload.bin",
    "audit_event_payload.bin",
    "authority_map_payload.bin",
    "authority_node_payload.bin",
    "authority_mutation_payload.bin",
    "repository_retirement_payload.bin",
    "repository_retirement_error_payload.bin",
    "stack_payload.bin",
    "stack_change_payload.bin",
];
const OPERATIONAL_EXCLUDED_INDEX_MARKERS: &[&str] = &[
    "repository_id.idx",
    "repository_name.idx",
    "repository_group_id.idx",
    "repository_group_slug.idx",
    "repository_role_binding_id.idx",
    "repository_role_binding_key.idx",
    "release_id.idx",
    "release_version.idx",
    "audit_event_id.idx",
    "audit_event_type_created.idx",
    "audit_event_entity_created.idx",
    "authority_map_id.idx",
    "authority_node_id.idx",
    "authority_mutation_id.idx",
    "repository_retirement_id.idx",
    "stack_id.idx",
    "stack_change_id.idx",
];

fn repository_docs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../docs")
}

type BinaryDbV0Read = Result<Vec<u8>, (ErrorKind, String)>;

fn binary_db_v0_bytes() -> &'static BinaryDbV0Read {
    static BYTES: OnceLock<BinaryDbV0Read> = OnceLock::new();
    BYTES.get_or_init(|| {
        let path = repository_docs_dir().join("binary_db_v0.md");
        std::fs::read(&path).map_err(|error| {
            (
                error.kind(),
                format!(
                    "failed to read protected authority {}: {error}",
                    path.display()
                ),
            )
        })
    })
}

macro_rules! binary_db_v0_bytes_or_skip {
    () => {{
        match binary_db_v0_bytes() {
            Ok(bytes) => bytes.as_slice(),
            Err((ErrorKind::NotFound, _)) => {
                eprintln!(
                    "skipping Binary DB authority-byte assertion: lineage-only docs/binary_db_v0.md is unavailable"
                );
                return;
            }
            Err((_, message)) => panic!("{message}"),
        }
    }};
}

#[test]
fn binary_db_v0_authority_is_byte_for_byte_pinned() {
    let actual = format!("{:x}", Sha256::digest(binary_db_v0_bytes_or_skip!()));
    assert_eq!(
        actual, BINARY_DB_V0_SHA256,
        "docs/binary_db_v0.md may change only through an explicit bounded repository-owner authorization followed by a complete digest repin"
    );
}

#[test]
fn binary_db_v0_patchset_schema_retains_ci_and_adds_the_worker_job_locator() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    assert!(
        authority.contains(PATCHSET_RECORD_LAYOUT),
        "the corrected 65-byte Patchset record must retain compact CI, summary, and Worker Job authority"
    );
    assert_eq!(
        authority
            .lines()
            .filter(|line| *line == "u64 ci_completed_at_s")
            .count(),
        1,
        "Patchset CI completion time must use the corrected v0 u64-second convention"
    );
    assert!(
        authority.contains(PATCHSET_CI_STATUS_LAYOUT),
        "the compact Patchset CI status-bit encoding must remain explicit"
    );
    assert!(
        authority.contains(PATCHSET_SUMMARY_PAYLOAD_LAYOUT),
        "Patchset summary must use only its narrowly typed payload"
    );
    assert!(
        authority.contains("The appended summary locator is exactly ten bytes."),
        "the Patchset summary locator size and padding boundary must remain explicit"
    );
    assert!(
        authority.contains("The final Worker Job\nlocator is exactly four bytes"),
        "the same-Repository Worker Job locator size and tail boundary must remain explicit"
    );
    assert!(
        authority.contains(
            "The first 51 bytes are the complete fixed Patchset/compact-CI region"
        ) && authority.contains("producing the complete 65-byte record"),
        "the widened Patchset prefix, compact-CI, summary, and Job-locator regions must total 65 bytes"
    );
    assert!(
        authority.contains(
            "places the\nunchanged four-byte locator at bytes 61 through 64 of the 65-byte record"
        ),
        "the corrected Patchset Job locator must occupy the final four bytes"
    );
    assert!(
        authority.contains("- generic `ci.bin` or `job.bin`;"),
        "generic CI and job files must remain excluded from Binary DB v0"
    );
    assert!(
        authority.contains(
            "generic `patchset_payload.bin`, `attest_payload.bin`, `policy_payload.bin`,"
        ),
        "generic workflow payloads must remain forbidden"
    );
}

#[test]
fn binary_db_v0_uses_the_owner_approved_u64_second_widths_without_layout_renaming() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for marker in [
        "LOCAL_TASK_RECORD_SIZE = 64",
        "REMOTE_TASK_RECORD_SIZE = 60",
        "LOCAL_CHANGE_RECORD_SIZE = 68",
        "REMOTE_CHANGE_RECORD_SIZE = 68",
        "WORKTREE_CURSOR_RECORD_SIZE = 28",
        "LINE_RECORD_SIZE = 40",
        "TAG_RECORD_SIZE = 24",
        "LOCAL_LAND_RECORD_SIZE = 44",
        "SERVER_LAND_RECORD_SIZE = 48",
        "CONTENT_SNAPSHOT_RECORD_SIZE = 88",
        "AUTHORITATIVE_SNAPSHOT_LINK_RECORD_SIZE = 40",
        "REMOTE_MIRROR_SNAPSHOT_LINK_RECORD_SIZE = 52",
        "TREE_PACK_RECORD_SIZE = 32",
        "OBJECT_PACK_RECORD_SIZE = 32",
        "BLOB_RECORD_SIZE = 64",
        "GIT_COMMIT_MAPPING_RECORD_SIZE = 96",
        "GIT_REF_MAPPING_RECORD_SIZE = 80",
        "GIT_TAG_MAPPING_RECORD_SIZE = 48",
        "GIT_OPERATION_CHECKPOINT_RECORD_SIZE = 44",
        "SERVER_PATCHSET_RECORD_SIZE = 65",
        "SERVER_ATTEST_RECORD_SIZE = 24",
        "ACTOR_RECORD_SIZE = 36",
        "SERVER_REVIEW_RECORD_SIZE = 40",
        "POLICY_DECISION_RECORD_SIZE = 32",
        "WAIVER_RECORD_SIZE = 44",
        "PLAN_RECORD_SIZE = 48",
        "PLAN_REVISION_RECORD_SIZE = 56",
        "OPERATIONAL_REPOSITORY_RECORD_SIZE = 33",
        "SERVER_WORKER_JOB_RECORD_SIZE = 52",
        "SERVER_WORKER_READY_INDEX_RECORD_SIZE = 12",
    ] {
        assert!(
            authority.contains(marker),
            "missing owner-approved u64-second fixed width: {marker}"
        );
    }
    let remaining_u32_times = authority
        .lines()
        .filter(|line| line.starts_with("u32 ") && line.ends_with("_at_s"))
        .collect::<Vec<_>>();
    assert!(
        remaining_u32_times.is_empty(),
        "active fixed time declarations must all be u64: {remaining_u32_times:?}"
    );
    assert!(authority.contains("i64 timestamp_s"));
    for invariant in [
        "retains the authority name v0 and persisted `layout_id = 1`",
        "only admitted predecessor selector is exact `u32-time-v0`",
        "guessing from file divisibility is insufficient",
        "zero-extends each little-endian `u32` second value",
        "An active file never mixes the predecessor and corrected\nrecord widths",
        "Storage accepts every value through `u64::MAX`",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved u64-second invariant: {invariant}"
        );
    }
}

#[test]
fn binary_db_v0_defines_the_global_registry_and_repository_local_jobs() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    assert!(
        authority.contains("## Server-Global Repository Registry Authority"),
        "the owner-approved global-registry boundary must remain explicit"
    );
    for marker in OPERATIONAL_FIXED_LAYOUT_MARKERS {
        assert!(
            authority.contains(marker),
            "missing owner-approved operational fixed layout: {marker}"
        );
    }
    for marker in OPERATIONAL_PAYLOAD_FILE_MARKERS {
        assert!(
            authority.contains(marker),
            "missing owner-approved operational typed payload: {marker}"
        );
    }
    for marker in OPERATIONAL_INDEX_FILE_MARKERS {
        assert!(
            authority.contains(marker),
            "missing owner-approved operational rebuildable index: {marker}"
        );
    }
    for marker in OPERATIONAL_EXCLUDED_LAYOUT_MARKERS {
        assert!(
            !authority.contains(marker),
            "explicitly excluded PostgreSQL domain gained an operational layout: {marker}"
        );
    }
    for marker in OPERATIONAL_EXCLUDED_PAYLOAD_MARKERS {
        assert!(
            !authority.contains(marker),
            "retired PostgreSQL domain retained an operational payload: {marker}"
        );
    }
    for marker in OPERATIONAL_EXCLUDED_INDEX_MARKERS {
        assert!(
            !authority.contains(marker),
            "retired PostgreSQL domain retained an operational index: {marker}"
        );
    }
    for layout in [
        OPERATIONAL_REPOSITORY_RECORD_LAYOUT,
        OPERATIONAL_REPOSITORY_PAYLOAD_LAYOUT,
        OPERATIONAL_NAMESPACE_INDEX_LAYOUT,
        OPERATIONAL_NAMESPACE_ENCODING,
        OPERATIONAL_REPOSITORY_POLICY_LAYOUT,
        SERVER_WORKER_JOB_RECORD_LAYOUT,
        SERVER_WORKER_JOB_STATE_LAYOUT,
        SERVER_WORKER_READY_INDEX_LAYOUT,
        SERVER_WORKER_STATE_INDEX_LAYOUT,
    ] {
        assert!(
            authority.contains(layout),
            "missing corrected fixed-width Repository schema: {layout}"
        );
    }
    for retired_repository_field in [
        "u64 repo_id_hash64",
        "u64 repo_name_hash64",
        "u64 created_at_us",
        "u64 updated_at_us",
        "u16 repo_id_len",
        "u8  repo_id_bytes[repo_id_len]",
        "u8  namespace_kind",
        "u16 default_line_len",
        "u16 id_namespace_prefix_len",
        "u8  default_line_bytes[default_line_len]",
        "u8  id_namespace_prefix_bytes[id_namespace_prefix_len]",
        "u8  policy_json_bytes[",
    ] {
        assert!(
            !authority.contains(retired_repository_field),
            "Repository schema retained a retired field: {retired_repository_field}"
        );
    }
    for retired_worker_job_layout in [
        "OPERATIONAL_WORKER_JOB_RECORD_SIZE = 104",
        "SERVER_WORKER_JOB_RECORD_SIZE = 100",
        "SERVER_WORKER_JOB_RECORD_SIZE = 92",
        "SERVER_WORKER_JOB_RECORD_SIZE = 88",
        "SERVER_WORKER_JOB_RECORD_SIZE = 60",
        "u64 available_at_us",
        "u64 locked_at_us",
        "u16 reserved0\nu32 repository_index\nu32 request_len",
        "OperationalWorkerJobRecord — worker_job.bin:",
        "u32 request_len\nu64 job_id\nu64 request_offset",
        "ServerWorkerJobRequestPayload — worker_job_request_payload.bin:",
        "ServerWorkerJobResultPayload — worker_job_result_payload.bin:",
        "ServerWorkerJobErrorPayload — worker_job_error_payload.bin:",
        "ServerWorkerJobLeaseOwnerPayload — worker_job_lease_owner_payload.bin:",
        "u64 result_offset",
        "u64 error_offset",
        "u64 lease_owner_offset",
        "ServerWorkerJobInputPayload — worker_job_input_payload.bin:",
        "u32 input_len",
        "u64 input_offset",
        "u8  lease_owner_hash[16]",
        "u32 subject_index_plus1",
        "u32 context_index_plus1",
        "u32 auxiliary_index_plus1",
        "u32 related_job_index_plus1",
        "OPERATIONAL_PUBLIC_SEQUENCE_RECORD_SIZE = 16",
        "OPERATIONAL_WORKER_JOB_ID_INDEX_RECORD_SIZE = 16",
        "OPERATIONAL_WORKER_READY_INDEX_RECORD_SIZE = 28",
        "OPERATIONAL_WORKER_REPOSITORY_STATE_INDEX_RECORD_SIZE = 20",
        "02-queue.lock",
    ] {
        assert!(
            !authority.contains(retired_worker_job_layout),
            "Repository-scoped Worker Job authority retained a global-owner field or layout: {retired_worker_job_layout}"
        );
    }
    for invariant in [
        "This file is Remote Binary input for that one Repository",
        "Neither Repository ID nor `repository_index` is duplicated in the\nfixed record.",
        "A Worker Job identity is meaningful only\n  as `(repository_index, worker_job_index)`",
        "The direct physical `worker_job_index` record ordinal is the\nsole Worker Job primary key",
        "Records are append-only:\nan assigned index is never renumbered, removed, reused, or transferred.",
        "v0 stores no separate\npublic or global Job ID.",
        "`ci_worker_job_index_plus1 = 0` means the Patchset has no selected\nWorker-Job-backed CI run",
        "No Repository ID\nor Repository index participates in that same-root relationship.",
        "`operational_public_sequence.bin`, every Worker Job fixed file, and every\nWorker Job index are forbidden in this global root.",
        "The two\nWorker Job indexes belong separately to every numeric server Repository\nauthority.",
        "merges them in memory by\n`(available_at_s, repository_index, worker_job_index)`",
        "PostgreSQL sequence state is not admitted input.",
        "Source `job_id` values and\ntheir holes are conversion-only ordering evidence, not Binary public history.",
        "The logical Repository default Line is always exact UTF-8 `main`.",
        "The Repository primary key is the record's physical `repository_index`; it is\nnot duplicated inside the record or payload.",
        "0 ait-core\n1 ait-server\n2 ait-python\n3 ait-node",
        "Repository names are non-empty\nimmutable UTF-8 display metadata and may repeat without limit.",
        "All server routing, configuration, and operational relationships identify a\nRepository by `repository_index`, never by Repository name.",
        "The configured server Repository-authority parent uses the canonical unsigned\nbase-10 `repository_index` as each Repository directory basename:",
        "Repository\nname and conversion-only PostgreSQL `repo_id` never appear in an authoritative\ndirectory name.",
        "It never reorders,\nremoves, or renumbers a `repository.bin` or `worker_job.bin` record;",
        "`default_line` is exact-verified as `main` and discarded.",
        "`id_namespace_prefix` maps by exact source bytes: empty becomes\n  `[0x00, 0x00]`, one valid ASCII byte `x` becomes `[x, 0x00]`, and two valid\n  ASCII bytes `x, y` become `[x, y]`.",
        "The logical namespace length is\nderived from the trailing zero and is never stored.",
        "It is converted to `policy_flags`\n  and no source JSON bytes are persisted.",
        "`ait_native`, exact tables `ait_native_content.repositories` and\n`ait_native_control.jobs`.",
        "`repository_groups`, `repository_group_memberships`, `role_bindings`,\n  `releases`, `events`, `authority_maps`, `authority_nodes`,\n  `authority_mutations`, `repository_retirements`, `stacks`, and\n  `stack_changes`, whose superseded operational layouts are retired from\n  active v0;",
        "`remote_task_inventory`, `remote_change_inventory`, and\n  `remote_patchset_inventory`, which are disposable read projections",
        "retired `sessions`, `session_events`, `session_checkpoints`,\n  `planning_sessions`, and `planning_session_events`;",
        "Community/ait-web `community_accounts`,\n  `community_external_identities`, `community_password_credentials`, and\n  `community_web_sessions`;",
        "Test inventory/coverage `test_case_inventory`, `test_groups`,\n  `test_group_memberships`, `test_group_memberships_legacy`,",
        "This\nschema amendment itself does not authorize creating, converting, or activating\nthose roots.",
        "`job_kind` is fixed authority; an API\n`job_type` string is synthesized from it and is never persisted as Job bytes.",
        "Active v0\ndefines no Worker Job payload file.",
        "No variable Worker Job request or input bytes exist.",
        "`job_kind = 1` is unassigned in active v0. `agent.turn.submit` requires an\nopaque turn request",
        "Full successful result objects belong to the domain records they\nmutate.",
        "Lease credentials are not Binary DB authority.",
        "The temporary Binary is a disposable runtime replica, not a layout-1 file\nfamily.",
        "A missing,\ncorrupt, stale, mismatched, or expired entry invalidates the lease",
        "Worker identity is diagnostic runtime\ndata and is not stored in either Worker Job authority or the lease token.",
        "`land.process` derives its Change and revision Snapshot from the resolved\nPatchset. The Job is committed before any Land exists",
        "`attached` and `superseded` preserve only why this Job performed no independent\nsuccessful work. They do not identify another Job.",
        "a Worker Job Land index, target-Line index, related-Job index, or second\n  Snapshot-reference field",
        "runner context, and materialized runtime payload are reconstructed runtime\ndata",
        "Operational timestamps are non-negative `u64` Unix seconds.",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved operational invariant: {invariant}"
        );
    }
}

#[test]
fn binary_db_v0_has_the_owner_approved_layout_one_amendments() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    assert!(
        authority.contains("u32 layout_id = 1     # little-endian"),
        "the owner-approved amendments retain persisted layout_id 1"
    );
    assert!(
        authority.contains(SNAPSHOT_PARENT_EDGE_LAYOUT),
        "the owner-approved fixed Snapshot DAG edge authority must remain exact"
    );
    assert!(
        authority.contains(TAG_RECORD_LAYOUT),
        "the owner-approved local AIT Tag record must remain exact"
    );
    for marker in GIT_RECORD_LAYOUT_MARKERS {
        assert!(
            authority.contains(marker),
            "missing owner-approved Git import/export schema marker: {marker}"
        );
    }
    assert!(
        !authority.contains("u64 recorded_at_unix_nanos"),
        "AIT-generated nanosecond recording time is not Git source authority"
    );
    assert!(
        authority.contains("i64 timestamp_s"),
        "the signed Git-source identity timestamp must remain exact"
    );
    assert!(
        authority.contains(
            "It must not be edited, replaced, renamed, or deleted without an explicit,\n\
bounded authorization from the repository owner."
        ),
        "the authority must remain locked unless the repository owner explicitly authorizes a bounded amendment"
    );
}

#[test]
fn binary_db_v0_bounds_review_projection_and_lockless_source_freezing() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for invariant in [
        "a present source\n`reviewer` is also admitted only when its exact UTF-8 bytes equal that sole\nrequested-group identity.",
        "conversion creates or reuses one existing `team` Actor from those bytes and\nwrites the existing Review `actor_index_plus1` reference once.",
        "A different\nreviewer, an empty identity, zero groups, or multiple groups remains\nfail-closed.",
        "before conversion without changing the existing manifest schema and without\nrequiring or creating source locks.",
        "complete current authority data-file inventory before copying, copies every\ncurrent authority data file without trusting a stale manifest file list",
        "both inventories must agree exactly by `relative_path`, `byte_size`, and\ncomplete SHA-256, and the copied inventory must equal them.",
        "isolated frozen generation without acquiring or creating source locks.",
        "No source lock file is required or created in either the selected source or\nthe frozen copy.",
        "Any drift fails closed and leaves no published\ntarget.",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved Review projection/lockless-source-freezing invariant: {invariant}"
        );
    }
    assert!(
        !authority.contains("acquires all declared source locks"),
        "the owner removed the source-lock prerequisite"
    );
}

#[test]
fn binary_db_v0_has_the_owner_approved_fixed_runtime_parity_schema() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for (layout, purpose) in [
        (LOCAL_TASK_RECORD_LAYOUT, "local Task close-time tail"),
        (REMOTE_TASK_RECORD_LAYOUT, "remote Task close-time tail"),
        (LOCAL_CHANGE_RECORD_LAYOUT, "local inline Change lifecycle"),
        (
            REMOTE_CHANGE_RECORD_LAYOUT,
            "remote inline Change lifecycle",
        ),
        (LOCAL_LAND_RECORD_LAYOUT, "local inline Land target Line"),
        (SERVER_LAND_RECORD_LAYOUT, "remote inline Land target Line"),
        (SNAPSHOT_HISTORY_RECORD_LAYOUT, "Snapshot history boundary"),
        (TREE_RECORD_LAYOUT, "Tree physical pack ordinal"),
        (
            TREE_ENTRY_RANGE_RECORD_LAYOUT,
            "normalized Tree entry range",
        ),
    ] {
        assert!(
            authority.contains(layout),
            "missing owner-approved fixed schema for {purpose}"
        );
    }
    for marker in FIXED_SCHEMA_BIT_MARKERS {
        assert!(
            authority.contains(marker),
            "missing owner-approved fixed-schema bit assignment: {marker}"
        );
    }
    assert!(
        authority.contains(
            "`change_lifecycle.bin` and `ChangeLifecycleRecord` are retired from active v0"
        ),
        "the former Change lifecycle side file must remain explicitly retired"
    );
    assert!(
        !authority.contains("ChangeLifecycleRecord — change_lifecycle.bin:"),
        "the retired Change lifecycle side file must not remain an active schema block"
    );
    assert!(
        authority.contains(
            "`land_target_line.bin` and\n`LandTargetLineRecord` are retired from active v0"
        ),
        "the former Land target-Line side file must remain explicitly retired"
    );
    assert!(
        !authority.contains("LandTargetLineRecord — land_target_line.bin:"),
        "the retired Land target-Line side file must not remain an active schema block"
    );
    assert!(
        authority.contains(
            "Archive and reopen each rewrite and durably commit\none complete 68-byte Change record"
        ),
        "Change lifecycle mutation must be a single-record commit"
    );
}

#[test]
fn binary_db_v0_separates_line_refs_from_snapshot_authoring_line() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for invariant in [
        "A Snapshot's `line_index_plus1` is its immutable authoring-Line\nidentity, not Line-head membership",
        "preserves\nthe exact referenced live fork Snapshot without comparing or rewriting its\nauthoring Line",
        "`landed_snapshot_index_plus1` references the exact live Snapshot installed as\nthat Line's new head",
        "Land never relabels it or compares those Line\nindexes for equality",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved Line/Snapshot reference invariant: {invariant}"
        );
    }
    for invalid_equality in [
        "Snapshot's `line_index_plus1` must equal `base_line_index_plus1`",
        "`line_index_plus1` equals `target_line_index_plus1`",
    ] {
        assert!(
            !authority.contains(invalid_equality),
            "Snapshot authoring Line must not be equated with a mutable Line ref: {invalid_equality}"
        );
    }
}

#[test]
fn binary_db_v0_has_the_owner_approved_bin_to_bin_conversion_schema() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    assert!(
        authority.contains(REMOTE_CHANGE_RECORD_LAYOUT),
        "Remote Change must retain its 44-byte prefix while naming the selected Patchset and appending the inline lifecycle tail"
    );
    for marker in BIN_TO_BIN_SCHEMA_MARKERS {
        assert!(
            authority.contains(marker),
            "missing owner-approved bin-to-bin fixed-bit marker: {marker}"
        );
    }
    for invariant in [
        "Source `current_patchset_number` resolves through the latest index;",
        "`explicit_unplanned` both encode as `TaskRecord.task_meta` bit 0 clear;",
        "A converter presented with a different reviewer, an empty identity,\nzero groups, or more than one requested group for a request fails closed",
        "Source `input_fingerprint` is cache metadata, not Policy authority",
        "`submitted_at_s = 0`, and `updated_at_s` equal to the preserved\nsource `landed_at`.",
        "Source `identity_source` is an annotation, not stored identity.",
        "Source\n`published_remote_name` is likewise not stored.",
        "the legacy format does not persist selected-Patchset history at\n`attested_at_s`.",
        "resolves the row through its owning Change's authoritative current\n`selected_patchset_number`",
        "converter-local state and creates no historical selection field, record, bin,\nor payload.",
        "same source pointer independently populates the existing\n`RemoteChangeRecord.selected_patchset_index_plus1` authority",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved bin-to-bin conversion invariant: {invariant}"
        );
    }
}

#[test]
fn binary_db_v0_has_the_owner_approved_bin_to_bin_followup_schema() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for (layout, purpose) in [
        (
            LOCAL_CHANGE_RECORD_LAYOUT,
            "local Change canceled-state byte",
        ),
        (
            REMOTE_CHANGE_RECORD_LAYOUT,
            "remote Change canceled-state byte",
        ),
        (ACTOR_RECORD_LAYOUT, "existing fixed Actor record"),
        (
            ACTOR_PAYLOAD_LAYOUT,
            "existing typed Actor identity payload",
        ),
    ] {
        assert!(
            authority.contains(layout),
            "missing owner-approved follow-up schema for {purpose}"
        );
    }
    assert_eq!(
        authority
            .lines()
            .filter(|line| *line == "u8  change_state")
            .count(),
        2,
        "change_state must replace the same reserved byte in both 52-byte Change records"
    );
    assert!(
        !authority.contains("  01 selected_for_landing"),
        "immutable Patchset metadata must not persist selected-for-landing state"
    );
    for invariant in [
        "Selection is\nChange state, not an immutable Patchset property.",
        "A legacy source Patchset state\n`selected_for_landing` is a non-authoritative projection and normalizes to\n`published`",
        "An admitted source Task status `abandoned` maps to\n`TaskRecord.task_meta` bit 7 `canceled`.",
        "A source\nChange status `abandoned` maps to archived lifecycle, canceled set",
        "An offline converter writes a separate target generation and may assign new\ndense physical target record indexes.",
        "Physical renumbering never changes `change_ordinal`, `patch_ordinal`,",
        "legacy salvage mode for a Patchset whose canonical `revision_snapshot_id` is\nabsent from the same locked source generation's content Snapshot authority.",
        "the converter emits no row for that Change or any of its\nPatchsets and dependents; it never chooses a fallback current or selected\nPatchset.",
        "This exception does not admit an absent or sentinel Patchset Snapshot index,",
        "The\nconversion report must enumerate the omitted Change and Patchset identities",
        "The newly admitted missing-time zero cases are limited to",
        "For bin-to-bin conversion, this existing fixed record and the existing\n`ActorPayload` are the complete Actor schema",
        "actor_key_hash = fnv1a64(user_name_bytes)\noffset_basis   = 0xcbf29ce484222325\nprime          = 0x00000100000001b3",
        "A non-empty identity supplied through `requested_groups` is structured team\nevidence and uses kind `team`.",
        "`ci_rollout_phase` is the numeric rollout phase of CI enforcement policy.",
        "CI rollout phase 0 blocks `rust_core` and keeps none visible as",
        "conversion writes `check_kind = 8`,\npreserves the source check status, writes `subject_ordinal = 0`, and writes\n`detail_flags = 0`.",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved bin-to-bin follow-up invariant: {invariant}"
        );
    }
}

#[test]
fn binary_db_v0_has_the_owner_approved_legacy_landed_normalization() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for invariant in [
        "a landed Change that has one or\nmore surviving Patchsets may supply `current_patchset_number = 0`",
        "source selected pointer is non-zero and resolves to the surviving Patchset\nowned by the Change with the greatest `patch_ordinal`",
        "The admitted legacy server format may retain more than one succeeded Land for\none Change and may append a blocked, failed, canceled, or updating attempt\nafter an earlier success.",
        "If any surviving Land succeeded, the Change lifecycle is\nnormalized to landed regardless of the duplicated source Change status.",
        "The\nsucceeded Land with the greatest `land_ordinal` supplies the public target Line\nand landed time; later non-succeeded attempts remain immutable history",
        "Legacy Change `landed_at` is a non-authoritative duplicated projection",
        "the Change's mutable selected-Patchset pointer and a historical\nLand's exact accepted Patchset are independent authorities after submission.",
        "and add no field, bit, bin, payload, fallback Patchset, or\ninferred timestamp.",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved legacy landed normalization: {invariant}"
        );
    }
}

#[test]
fn binary_db_v0_scopes_policy_ordinals_to_patchsets_without_widening() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    assert!(authority.contains(TASK_POLICY_INDEX_LAYOUT));
    assert!(authority.contains(PATCHSET_POLICY_INDEX_LAYOUT));
    for invariant in [
        "`policy_ordinal` is unique in `(patchset_index, policy_ordinal)`, not in the\nowning Task.",
        "A Task may therefore own more than 64\nPolicy Decisions across multiple Patchsets",
        "`TaskPolicyIndexRecord` retains the complete Task inventory only",
        "Review and Policy\nDecision allocate within their owning Patchset.",
        "`.../P-##/POL-##` identity maps the exact `POL-##` suffix to\nPatchset-scoped `K-##`.",
        "No old-to-new mapping bin or payload is added.",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved Patchset-scoped Policy invariant: {invariant}"
        );
    }
    assert!(
        !authority.contains(
            "TaskPolicyIndexRecord — task_policy_index.bin:\nu32 latest_policy_index_plus1\nu16 policy_count\nu8  next_policy_ordinal"
        ),
        "Task Policy index must not allocate Patchset-scoped ordinals"
    );
}

#[test]
fn binary_db_v0_has_the_owner_approved_converter_normalization_correction() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for layout in [
        TASK_PATCHSET_INDEX_LAYOUT,
        CHANGE_PATCHSET_INDEX_LAYOUT,
        TASK_REVIEW_INDEX_LAYOUT,
        PATCHSET_REVIEW_INDEX_LAYOUT,
        TASK_LAND_INDEX_LAYOUT,
        CHANGE_LAND_INDEX_LAYOUT,
    ] {
        assert!(authority.contains(layout), "missing corrected index layout");
    }
    for invariant in [
        "`patch_ordinal` is unique in `(change_index, patch_ordinal)`.",
        "`review_ordinal` is unique in `(patchset_index, review_ordinal)`.",
        "`land_ordinal` is unique in `(change_index, land_ordinal)`.",
        "Task Patchset, Review, Policy, and Land inventory\nlinks are rebuilt in emitted physical record order.",
        "Snapshot authority is a DAG forest, not a single-root tree. An authority with\nzero live Snapshots is valid.",
        "target `created_at_s` is the minimum valid non-zero source creation time.",
        "Legacy conversion accepts only the following exact\n`(catalog, blocking set, informational set, phase message)` tuples.",
        "They are not bounded by the owning\nPatchset's `ci_selected_suite_count`",
        "the Patchset field is compact evidence\nfor one CI run, while immutable Policy rows retain historical evaluations",
        "The second omission exception is the separately selected named legacy salvage\nfor `T-0934/C-01`.",
        "Header synthesis is never performed in the\nselected source root",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved converter normalization invariant: {invariant}"
        );
    }
}

#[test]
fn binary_db_v0_bounds_the_exact_legacy_stale_zero_diff_gate() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for patchset_id in [
        "RT-1982/C-01/P-03",
        "RT-1982/C-01/P-02",
        "RT-1982/C-01/P-01",
        "RT-1981/C-01/P-03",
    ] {
        assert!(
            authority.contains(patchset_id),
            "missing exact admitted stale-zero Patchset identity: {patchset_id}"
        );
    }
    for invariant in [
        "The only admitted supplied `diff_stats` disagreement is an explicitly selected\noffline legacy stale-zero gate",
        "`files_added = 0`, `files_changed = 0`, `files_deleted = 0`, and\n`files_modified = 0`",
        "exact comparison\nof their complete Trees must succeed and produce a non-empty difference.",
        "the target stores no diff-stat value.",
        "No\nwildcard, ordinal-only match, fallback Snapshot, reconstructed source value,\nnew field, bit, bin, index, payload, record-width change, or `layout_id` change\nis admitted.",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved stale-zero diff invariant: {invariant}"
        );
    }
}

#[test]
fn binary_db_v0_bounds_the_exact_legacy_task_canceled_spelling_gate() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    for invariant in [
        "`f287ac90f3c5c480da7da26ca6859057f9f72758e4212a2dce1cfaddf93d5355`",
        "For repository key `ait` only, an explicitly selected gate maps the legacy\nsource Task status spelling `canceled`",
        "The server orchestrator verifies both the complete manifest digest and the\nrepository key before forwarding the gate.",
        "Without the explicit gate, source `status = \"canceled\"` fails closed.",
        "The gate does not\nchange the conversion report and adds no field, bit, bin, index, payload,",
    ] {
        assert!(
            authority.contains(invariant),
            "missing owner-approved legacy Task canceled spelling invariant: {invariant}"
        );
    }
}

#[test]
fn local_change_publication_mapping_is_task_scoped_and_width_preserving() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    assert!(
        authority.contains(LOCAL_CHANGE_RECORD_LAYOUT),
        "the owner-approved Local Change publication slot must preserve the exact 44-byte prefix in the corrected 52-byte layout"
    );
    for invariant in [
        "`C-01` is stored as `1` and `C-64`\nas `64`; values `65..255` are invalid.",
        "No Local Change stores or requires a global Remote `change_index`.",
        "Remote ordinal may differ from the Local `change_ordinal`",
    ] {
        assert!(
            authority.contains(invariant),
            "missing Task-scoped Remote Change publication invariant: {invariant}"
        );
    }
    assert!(
        !authority.contains("u32 published_remote_change_index"),
        "Local Change publication must not persist a global Remote Change index"
    );
}

#[test]
fn task_close_time_is_one_task_record_field_and_never_a_side_file() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    assert_eq!(
        authority
            .lines()
            .filter(|line| *line == "u64 closed_at_s")
            .count(),
        2,
        "closed_at_s must appear once in each fixed Task record layout"
    );
    for forbidden_side_schema in [
        "TASK_CLOSE_RECORD_SIZE",
        "TaskCloseRecord",
        "task_close.bin",
    ] {
        assert!(
            !authority.contains(forbidden_side_schema),
            "Task close time must not define a side schema: {forbidden_side_schema}"
        );
    }
    for invariant in [
        "Zero means\neither that the effective Task state is not terminal or that an offline legacy\nconversion had no source close time for a terminal Task.",
        "Native writers never clear terminal status to reopen a Task.",
        "Native v0 writers must never create a terminal\nTask with zero close time.",
        "readers, validators, and recovery accept terminal plus zero as an unknown\nhistorical close time",
        "For a terminal Task whose legacy source\nformat demonstrably did not record close time, it writes `closed_at_s = 0`",
        "must not substitute creation time, update time, conversion time, or any other\ninferred timestamp.",
    ] {
        assert!(
            authority.contains(invariant),
            "missing legacy Task unknown-close-time invariant: {invariant}"
        );
    }
    assert!(
        !authority
            .contains("A terminal Task with zero close time\nis corruption and fails closed."),
        "legacy terminal Tasks without a recorded close time must not be rejected"
    );
}

#[test]
fn runtime_parity_fields_are_not_payload_schema() {
    let authority = std::str::from_utf8(binary_db_v0_bytes_or_skip!())
        .expect("docs/binary_db_v0.md must remain valid UTF-8");
    let payload_section = authority
        .split_once("## Typed Payload Files")
        .expect("typed payload section must exist")
        .1
        .split_once("## Rebuildable Indexes")
        .expect("rebuildable indexes must follow typed payloads")
        .0;
    for fixed_field in [
        "closed_at_s",
        "base_line_index_plus1",
        "archived_at_s",
        "target_line_index_plus1",
        "history_flags",
        "pack_entry_ordinal",
        "first_entry_index",
        "selected_patchset_index_plus1",
        "change_state",
        "evaluation_pending",
        "mode_kind",
        "detail_flags",
    ] {
        assert!(
            !payload_section.contains(fixed_field),
            "owner-approved fixed field must not appear in payload schema: {fixed_field}"
        );
    }
}

#[test]
fn superseded_binary_db_schema_documents_are_absent() {
    let docs_dir = repository_docs_dir();
    assert!(
        !docs_dir.join("binary_db.md").exists(),
        "docs/binary_db.md is forbidden; docs/binary_db_v0.md is the sole schema authority"
    );
    assert!(
        !docs_dir.join("binary_db_v1.md").exists(),
        "docs/binary_db_v1.md is superseded and forbidden; docs/binary_db_v0.md is the sole schema authority"
    );
}
