#![allow(unused_imports)]

use crate::foundation::db::ensure_postgres_schema_name;
use crate::foundation::pack_substrate::{read_tree_pack_tree, read_tree_pack_tree_by_ordinal};
use crate::foundation::server_context::{DEFAULT_CONTENT_SCHEMA, DEFAULT_CONTROL_SCHEMA};
use crate::foundation::workflow_artifacts::{
    attestation_id_for_patchset, effective_policy_status, review_summary_from_rows,
};
use chrono::Utc;
use postgres::{Client, NoTls, Row};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

#[path = "server_patchset_store/attestations.rs"]
mod attestations;
#[path = "server_patchset_store/changes.rs"]
mod changes;
#[path = "server_patchset_store/helpers.rs"]
mod helpers;
#[path = "server_patchset_store/patchsets.rs"]
mod patchsets;
#[path = "server_patchset_store/reviews.rs"]
mod reviews;
#[path = "server_patchset_store/rows.rs"]
mod rows;
#[path = "server_patchset_store/runtime.rs"]
mod runtime;
#[path = "server_patchset_store/store.rs"]
mod store;
#[cfg(test)]
#[path = "server_patchset_store/tests.rs"]
mod tests;

use helpers::{
    derive_patchset_id, ensure_change_mutable, insert_i64, insert_text, int_value,
    normalize_author_mode, optional_text, payload_object, repo_scoped_sequence_ref, required_text,
    row_bool, row_i64, row_text, schema_table, truthy, utc_now,
};
use rows::{attestation_row_json, change_row_json, patchset_row_json, review_row_json};
pub use runtime::{server_patchset_store_json, SERVER_PATCHSET_STORE_CONTRACT};
use runtime::{PatchsetStoreRuntime, FAKE_POSTGRES_PREFIX, REQUIRED_APPROVALS};
use store::PostgresPatchsetStore;
