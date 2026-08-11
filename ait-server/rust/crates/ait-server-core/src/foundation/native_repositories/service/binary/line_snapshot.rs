use super::*;

#[path = "line_snapshot/bundle.rs"]
mod bundle;
#[path = "line_snapshot/helpers.rs"]
mod helpers;
#[path = "line_snapshot/line.rs"]
mod line;
#[path = "line_snapshot/snapshot.rs"]
mod snapshot;
#[path = "line_snapshot/zstd.rs"]
mod zstd;

pub(super) use helpers::now_timestamp_s;
use helpers::{
    decode_optional_sha256, json_u32, json_u64, manifest_hash_text, timestamp_s, timestamp_string,
};

#[derive(Default)]
pub(super) struct BinaryBlobReadSession {
    resolved_blobs: BTreeMap<String, Vec<u8>>,
    ready_pack_ids: BTreeSet<String>,
    pack_archives: BTreeMap<String, PackEntryArchive>,
}
