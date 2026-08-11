mod codec;
mod helpers;
mod metadata;
mod packed_content;
mod read;
mod schema;
mod service;
mod store;
pub(crate) mod task_start;
mod views;
mod write;

#[cfg(test)]
mod tests;

use self::helpers::*;
use self::metadata::*;
use self::store::ServerPlanBinaryDbStore;

use self::schema::{
    compact_plan_file_for, plan_file, plan_item_file, plan_item_payload_file, plan_payload_file,
    plan_revision_file, plan_revision_payload_file, CompactPlanFile, PlanItemPayload,
    PlanItemRecord, PlanRecord, PlanRevisionPayload, PlanRevisionRecord, ITEM_HAS_REF_META,
    ITEM_STATE_DONE_META, ITEM_STATE_OPEN_META, ITEM_TASKABLE_HINT_META, PLAN_LAYOUT_ID,
    PLAN_STATE_ARCHIVED_META, PLAN_STATE_DRAFT_META, PLAN_STATE_MASK, PLAN_STATE_SUPERSEDED_META,
};
#[allow(unused_imports)]
pub(crate) use self::write::{
    ServerPlanBinaryDbCommitPoint, ServerPlanBinaryDbWritePurpose, ServerPlanBinaryDbWriteTxn,
};
use crate::foundation::pack_substrate::{
    read_pack_index_checksum_with_format, PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use crate::foundation::plan_revision::{
    normalize_plan_revision_artifact, normalized_plan_items, plan_revision_view,
    PlanRevisionViewOptions,
};
#[cfg(test)]
use crate::foundation::remote_binary_db::BinaryDbFileFamily;
use crate::foundation::remote_binary_db::{
    binary_db_runtime_error, BinaryDbError, BinaryDbErrorKind, BinaryDbFsyncPolicy,
    BinaryDbIndexAppender, BinaryDbReadScope, BinaryDbReadTxn, BinaryDbStoreFsyncPolicy,
    BinaryDbWriteTxn, BinaryFileId, ServerRemoteBinaryDb,
};
use crate::foundation::server_content_binary_db::ServerBinaryRepositoryContentStore;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type ServerPlanDefaultWriteTxn<'a, D, const WRITE_LAYOUT: u32> =
    ServerPlanBinaryDbWriteTxn<'a, D, BinaryDbStoreFsyncPolicy<'a, D>, WRITE_LAYOUT>;

pub const SERVER_PLAN_BINARY_DB_LAYOUT_V1: u32 = PLAN_LAYOUT_ID;

/// Exact fixed-record file identities used by offline, layout-preserving
/// recovery tooling. These expose the existing Plan v1 files without allowing
/// callers to construct undeclared Binary DB files.
pub fn server_plan_file_id() -> BinaryFileId {
    plan_file()
}

pub fn server_plan_revision_file_id() -> BinaryFileId {
    plan_revision_file()
}

pub fn server_plan_item_file_id() -> BinaryFileId {
    plan_item_file()
}

pub fn server_plan_payload_file_id() -> crate::foundation::remote_binary_db::BinaryPayloadFileId {
    plan_payload_file()
}

pub fn server_plan_revision_payload_file_id(
) -> crate::foundation::remote_binary_db::BinaryPayloadFileId {
    plan_revision_payload_file()
}

pub fn server_plan_item_payload_file_id() -> crate::foundation::remote_binary_db::BinaryPayloadFileId
{
    plan_item_payload_file()
}

pub type BinaryDbServerPlanServiceV1<D> =
    BinaryDbServerPlanService<D, SERVER_PLAN_BINARY_DB_LAYOUT_V1>;

const PLAN_ARTIFACT_CLOSURE_ISSUE_LIMIT: usize = 256;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanArtifactBlobClosureIssue {
    pub plan_id: String,
    pub plan_revision_id: String,
    pub artifact_path: String,
    pub artifact_blob_id: String,
    pub error: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanArtifactBlobClosureAudit {
    pub schema: String,
    pub status: String,
    pub plan_count: u64,
    pub revision_count: u64,
    pub referenced_revision_count: u64,
    pub referenced_blob_count: u64,
    pub healthy_blob_count: u64,
    pub unhealthy_blob_count: u64,
    pub unhealthy_revision_count: u64,
    pub issue_limit: usize,
    pub issues_truncated: bool,
    pub issues: Vec<PlanArtifactBlobClosureIssue>,
}

impl PlanArtifactBlobClosureAudit {
    pub fn is_complete(&self) -> bool {
        self.status == "complete" && self.unhealthy_blob_count == 0
    }

    pub fn failure_summary(&self) -> String {
        let blob_ids = self
            .issues
            .iter()
            .map(|issue| issue.artifact_blob_id.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(8)
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if self.issues_truncated {
            "; issue list truncated"
        } else {
            ""
        };
        let blob_detail = if blob_ids.is_empty() {
            String::new()
        } else {
            format!("; blobs: {blob_ids}")
        };
        format!(
            "Plan artifact closure is incomplete: {} unique blobs affect {} revisions{blob_detail}{suffix}",
            self.unhealthy_blob_count, self.unhealthy_revision_count
        )
    }
}

#[derive(Clone, Debug)]
pub struct BinaryDbServerPlanService<D, const WRITE_LAYOUT: u32 = SERVER_PLAN_BINARY_DB_LAYOUT_V1>
where
    D: ServerRemoteBinaryDb + Clone,
{
    store: ServerPlanBinaryDbStore<D, WRITE_LAYOUT>,
}

impl<D> BinaryDbServerPlanService<D, SERVER_PLAN_BINARY_DB_LAYOUT_V1>
where
    D: ServerRemoteBinaryDb + Clone,
{
    pub fn new(db: D) -> Self {
        Self {
            store: ServerPlanBinaryDbStore::new(db),
        }
    }
}

impl<D, const WRITE_LAYOUT: u32> BinaryDbServerPlanService<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + Clone,
{
    pub fn with_write_layout(db: D) -> Self {
        Self {
            store: ServerPlanBinaryDbStore::new(db),
        }
    }

    pub fn db(&self) -> &D {
        self.store.db()
    }

    #[cfg(test)]
    fn store(&self) -> &ServerPlanBinaryDbStore<D, WRITE_LAYOUT> {
        &self.store
    }
}
