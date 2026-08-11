#![allow(unused_imports)]

use super::*;
use ::postgres as pg;

#[path = "postgres/blob_content.rs"]
mod blob_content;
#[path = "postgres/helpers.rs"]
mod helpers;
#[path = "postgres/line_snapshot.rs"]
mod line_snapshot;
#[path = "postgres/pack_locator.rs"]
mod pack_locator;
#[path = "postgres/repository.rs"]
mod repository;
#[path = "postgres/retirement.rs"]
mod retirement;
#[path = "postgres/rows.rs"]
mod rows;
#[path = "postgres/service.rs"]
mod service;

pub(in crate::foundation::native_repositories) use blob_content::{
    blob_bytes_for_blob_id, native_blob_resolver_delta_chain_exceeded,
    require_blob_locator_for_repo, select_blob_by_id, select_blob_locator_for_repo,
};
use blob_content::{
    blob_bytes_for_blob_id_inner, inline_blob_content_bytes, require_blob_locator_pack_id,
};
use helpers::ensure_namespace_prefix_available;
pub(in crate::foundation::native_repositories) use helpers::runtime_storage_path;
use line_snapshot::{close_line_json, get_line_json, list_lines_json, snapshot_existence_json};
pub(in crate::foundation::native_repositories) use line_snapshot::{
    select_snapshot_row, snapshot_json_from_row, update_line_json, validate_existing_snapshot,
};
use pack_locator::pack_locator_for_id;
pub(in crate::foundation::native_repositories) use pack_locator::{
    tree_pack_locator_for_id, tree_pack_locator_for_tree_id, walk_tree_rows,
};
pub(in crate::foundation::native_repositories) use repository::select_repository_row;
use repository::{
    create_or_update_repository, create_or_update_repository_metadata, get_repository_json,
    get_repository_json_by_id, get_repository_metadata_json, get_repository_metadata_json_by_id,
    list_repositories_json, list_repository_metadata_json,
};
use retirement::retire_repository_json;
use rows::{
    blob_locator_row_from_db, blob_row_from_db, repository_row_from_db, snapshot_row_from_db,
};
pub use service::PostgresNativeRepositoryService;
pub(in crate::foundation::native_repositories) use service::{
    CONTENT_SCHEMA_SQL, REPOSITORY_METADATA_SCHEMA_SQL,
};
