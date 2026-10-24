use std::collections::{BTreeMap, BTreeSet};

use crate::json_support::{json, JsonMap as Map, JsonValue};
use sha2::{Digest, Sha256};

use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbErrorKind, BinaryDbFsyncPolicy,
    BinaryDbIndexAppender, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId, BinaryIndexId,
    BinaryPayloadFileId, PayloadRange, StorePath, StoreResult,
};
use crate::content_store::{
    BlobRecord, BlobStore, ContentStoreResult, EnsureBlobInput, ObjectPackLocator,
    ObjectPackMemberRecord, ObjectPackRecord, ObjectPackStore, RecordObjectPackInput,
    RecordTreeInput, RecordTreePackInput, RepoPath, TreeEntryRecord, TreePackRecord, TreePackStore,
    TreeRecord, TreeStore,
};
use crate::line_binary_db::binary_line_name_at;
use crate::local_snapshot::{
    LocalSnapshotTreeReadStore, SnapshotFileRow, SnapshotPathBlobRow, SnapshotPathDelta,
    SnapshotTreeRootLocator,
};
use crate::object_diff_ports::{BlobReader, SnapshotReader};
use crate::pack_substrate::TreePackEntryArchive;
use crate::snapshot_json::SnapshotJson;
use crate::snapshot_store::{
    compatibility_parent_projections, validate_snapshot_parent_set, SnapshotParentLink,
    SnapshotParentLinkPage, SnapshotRecord, SnapshotStore, SnapshotStoreResult,
};

use super::filters::{
    BinaryTreeRootLocator, BinaryTreeRootReadResolver, BinaryTreeRootResolver,
    StaticBinaryTreeRootResolver,
};
use super::views::{
    BinaryBlobView, BinaryObjectPackMemberView, BinaryObjectPackView, BinarySnapshotView,
    BinaryTreeEntryView, BinaryTreePackView, BinaryTreeView,
};
use crate::content_binary_db::{
    absolute_repo_path, blob_id_from_sha256, blob_id_index_key, hex_lower, object_pack_format_name,
    object_pack_id_from_hash48, object_pack_id_index_key, object_pack_relative_path,
    snapshot_id_from_hash48, snapshot_id_index_key, tree_id_from_hash80, tree_id_index_key,
    tree_pack_format_name, tree_pack_id_from_hash48, tree_pack_id_index_key,
    tree_pack_relative_path, BinaryBlobCodec, BinaryBlobRecord, BinaryDbBlobStore,
    BinaryDbObjectPackStore, BinaryDbSnapshotStore, BinaryDbTreePackStore, BinaryDbTreeStore,
    BinaryObjectPackCodec, BinaryObjectPackMemberCodec, BinaryObjectPackMemberKind,
    BinaryObjectPackMemberRecord, BinaryObjectPackRecord, BinarySnapshotCodec, BinarySnapshotKind,
    BinarySnapshotPayload, BinarySnapshotRecord, BinaryTreeCodec, BinaryTreePackCodec,
    BinaryTreePackRecord, BinaryTreeRecord, BLOB_BIN, BLOB_ID_IDX, BLOB_RECORD_SIZE,
    MAX_SNAPSHOT_PARENT_COUNT, OBJECT_PACK_BIN, OBJECT_PACK_ID_IDX, OBJECT_PACK_MEMBER_BIN,
    OBJECT_PACK_MEMBER_RECORD_SIZE, OBJECT_PACK_RECORD_SIZE, SNAPSHOT_BIN, SNAPSHOT_ID_IDX,
    SNAPSHOT_PAYLOAD_BIN, SNAPSHOT_RECORD_SIZE, TREE_BIN, TREE_ID_IDX, TREE_PACK_BIN,
    TREE_PACK_ID_IDX, TREE_PACK_RECORD_SIZE, TREE_RECORD_SIZE,
};

const CONTENT_BINARY_LAYOUT_ID: u32 = 1;

fn require_supported_content_layout(layout: u32, label: &str) -> StoreResult<()> {
    if layout == CONTENT_BINARY_LAYOUT_ID {
        Ok(())
    } else {
        Err(BinaryDbError::layout_mismatch(format!(
            "unsupported persisted Binary DB {label} layout: {layout}; supported layout is {CONTENT_BINARY_LAYOUT_ID}"
        )))
    }
}

fn persisted_content_layout<B: BinaryDb>(
    read: &BinaryDbReadTxn<'_, B>,
    path: &str,
    record_size: u32,
    label: &str,
) -> StoreResult<Option<u32>> {
    let layout = match read.layout_id(BinaryFileId::new(
        path,
        CONTENT_BINARY_LAYOUT_ID,
        record_size,
    )) {
        Ok(layout) => layout,
        Err(error) if error.kind() == BinaryDbErrorKind::MissingData => return Ok(None),
        Err(error) => return Err(error),
    };
    require_supported_content_layout(layout, label)?;
    Ok(Some(layout))
}

fn required_content_layout<B: BinaryDb>(
    read: &BinaryDbReadTxn<'_, B>,
    path: &str,
    record_size: u32,
    label: &str,
) -> StoreResult<u32> {
    persisted_content_layout(read, path, record_size, label)?.ok_or_else(|| {
        BinaryDbError::missing_data(format!("Binary DB {label} file {path} is missing"))
    })
}

fn content_record_file(
    path: &str,
    record_size: u32,
    layout: u32,
    label: &str,
) -> StoreResult<BinaryFileId> {
    require_supported_content_layout(layout, label)?;
    Ok(BinaryFileId::new(path, layout, record_size))
}

fn content_index(
    path: &str,
    layout: u32,
    label: &str,
    fixed: Option<(u32, bool)>,
) -> StoreResult<BinaryIndexId> {
    require_supported_content_layout(layout, label)?;
    Ok(match fixed {
        Some((key_size, plus_one)) => BinaryIndexId::new_fixed(path, layout, key_size, plus_one),
        None => BinaryIndexId::new(path, layout),
    })
}

fn content_payload(path: &str, layout: u32, label: &str) -> StoreResult<BinaryPayloadFileId> {
    require_supported_content_layout(layout, label)?;
    Ok(BinaryPayloadFileId::new(path, layout))
}

macro_rules! content_record_decoder {
    ($name:ident, $codec:ident, $record:ty, $label:literal) => {
        fn $name(layout: u32, raw: &[u8]) -> StoreResult<$record> {
            match layout {
                CONTENT_BINARY_LAYOUT_ID => $codec::<CONTENT_BINARY_LAYOUT_ID>::decode_record(raw),
                _ => {
                    require_supported_content_layout(layout, $label)?;
                    unreachable!("supported content layout must have a record decoder")
                }
            }
        }
    };
}

content_record_decoder!(
    decode_blob_record,
    BinaryBlobCodec,
    BinaryBlobRecord,
    "blob"
);
content_record_decoder!(
    decode_snapshot_record,
    BinarySnapshotCodec,
    BinarySnapshotRecord,
    "snapshot"
);
content_record_decoder!(
    decode_object_pack_record,
    BinaryObjectPackCodec,
    BinaryObjectPackRecord,
    "object pack"
);
content_record_decoder!(
    decode_object_pack_member_record,
    BinaryObjectPackMemberCodec,
    BinaryObjectPackMemberRecord,
    "object pack member"
);
content_record_decoder!(
    decode_tree_pack_record,
    BinaryTreePackCodec,
    BinaryTreePackRecord,
    "tree pack"
);
content_record_decoder!(
    decode_tree_record,
    BinaryTreeCodec,
    BinaryTreeRecord,
    "tree"
);
mod blob_surface;
mod index_lookup;
mod local_snapshot_adapter;
mod pack_surface;
mod snapshot_reader;
mod snapshot_surface;
mod tree_surface;

use self::blob_surface::*;
use self::index_lookup::*;
use self::local_snapshot_adapter::*;
use self::pack_surface::*;
pub use self::snapshot_reader::*;
use self::snapshot_surface::*;
pub use self::tree_surface::BinaryDbTreeReadCache;
use self::tree_surface::*;
