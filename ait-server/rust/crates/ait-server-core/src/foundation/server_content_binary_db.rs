use crate::foundation::remote_binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbErrorKind, BinaryDbFileFamily,
    BinaryDbFsyncPolicy, BinaryDbIndexAppender, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId,
    BinaryIndexId, BinaryPayloadFileId, ServerRemoteBinaryDb, StoreResult,
};

pub const SERVER_CONTENT_BINARY_LAYOUT_ID: u32 = 1;
pub const SERVER_LINE_RECORD_SIZE: u32 = 40;
pub const SERVER_SNAPSHOT_RECORD_SIZE: u32 = 88;
pub const SERVER_SNAPSHOT_PARENT_EDGE_RECORD_SIZE: u32 = 12;
pub const SERVER_FIXED_INDEX_RECORD_SIZE: u32 = 12;
pub const SERVER_LINE_BIN: &str = "line.bin";
pub const SERVER_LINE_NAME_PAYLOAD_BIN: &str = "line_name_payload.bin";
pub const SERVER_LINE_NAME_IDX: &str = "line_name.idx";
pub const SERVER_SNAPSHOT_BIN: &str = "snapshot.bin";
pub const SERVER_SNAPSHOT_PAYLOAD_BIN: &str = "snapshot_payload.bin";
pub const SERVER_SNAPSHOT_ID_IDX: &str = "snapshot_id.idx";
pub const SERVER_SNAPSHOT_PARENT_EDGE_BIN: &str = "snapshot_parent_edge.bin";

#[path = "server_content_binary_db/line.rs"]
mod line;
#[path = "server_content_binary_db/repository.rs"]
mod repository;
#[path = "server_content_binary_db/snapshot.rs"]
mod snapshot;
#[path = "server_content_binary_db/validation.rs"]
mod validation;

pub use line::*;
pub use repository::*;
pub use snapshot::*;
pub use validation::{
    server_line_name_hash64, server_snapshot_hash48_from_id, server_snapshot_id_from_hash48,
    server_snapshot_id_index_key,
};

use validation::{
    decode_line_record_for_layout, decode_snapshot_payload_for_layout,
    decode_snapshot_record_for_layout, find_line, find_line_in_write, find_snapshot_in_write,
    line_payload_file_for_layout, line_record_file_for_layout, normalize_line_name,
    persisted_content_layout, require_layout, require_len, set_payload_flags,
    snapshot_index_for_layout, snapshot_payload_file_for_layout, snapshot_record_file_for_layout,
    validate_line_record, validate_optional_record_link, validate_snapshot_line_name,
    validate_snapshot_line_name_from_persisted_layout, validate_snapshot_link,
    validate_snapshot_record,
};

#[cfg(test)]
mod tests;
