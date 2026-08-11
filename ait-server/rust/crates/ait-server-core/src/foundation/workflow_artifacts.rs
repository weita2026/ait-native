use serde_json::{json, Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{Cursor, Read};
use std::path::{Component, Path};
use zip::ZipArchive;

#[path = "workflow_artifacts/contract.rs"]
mod contract;
#[path = "workflow_artifacts/helpers.rs"]
mod helpers;
#[path = "workflow_artifacts/ids.rs"]
mod ids;
#[path = "workflow_artifacts/patchset.rs"]
mod patchset;
#[path = "workflow_artifacts/policy.rs"]
mod policy;
#[path = "workflow_artifacts/release.rs"]
mod release;
#[path = "workflow_artifacts/reviews.rs"]
mod reviews;
#[path = "workflow_artifacts/suites.rs"]
mod suites;

pub use contract::{
    workflow_artifacts_json, WORKFLOW_ARTIFACTS_CONTRACT, WORKFLOW_ARTIFACTS_REFERENCE_MODULE,
};
pub use ids::{attestation_id_for_patchset, land_submission_id_for_change};
pub use patchset::{dedupe_text_values, patchset_changed_paths, requires_code_review_summary};
pub use policy::{effective_policy_status, policy_status_view};
pub use release::{
    release_artifact_download_path, release_artifact_media_type, release_artifact_view,
    release_formula_payload, release_row, sanitize_release_artifact_path,
    validate_release_artifact_pack, ReleaseArtifactPackValidation, RELEASES_REFERENCE_MODULE,
    RELEASE_ARTIFACT_PACK_FORMAT_V1, RELEASE_ARTIFACT_PACK_MANIFEST_ENTRY,
};
pub use reviews::{
    is_structured_code_review_summary_text, review_decision_lane, review_summary_from_rows,
    CODE_REVIEW_SUMMARY_ACTION, TASK_REVIEW_APPROVE_ACTION, TASK_REVIEW_COMMENT_ACTION,
    TASK_REVIEW_DEFER_ACTION, TASK_REVIEW_REQUEST_CHANGES_ACTION, TEAM_REVIEW_APPROVE_ACTION,
    TEAM_REVIEW_COMMENT_ACTION, TEAM_REVIEW_REQUEST_CHANGES_ACTION,
};
pub use suites::{
    ci_rollout_patchset_suite_checks, ci_rollout_summary_message, coerce_suite_catalog_payload,
    patchset_rollout_suite_ids, suite_manifest_catalog_path,
};

use helpers::*;
use suites::CHECKED_IN_CI_CONTRACT_PATH;
