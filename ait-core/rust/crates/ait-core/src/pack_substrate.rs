use crate::json_support::{json, JsonMap as Map, JsonNumber as Number, JsonValue};
use sha2::{Digest, Sha256};
use similar::{capture_diff_slices, Algorithm, DiffOp};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const PACK_DELTA_GIT_BINARY_V1: &str = "git-binary-v1";
/// Writer policy for newly-created object packs.
pub const DEFAULT_MAX_DELTA_CHAIN_DEPTH: usize = 4;
/// Absolute safety ceiling for verified object-pack delta reads.
///
/// Writers use `DEFAULT_MAX_DELTA_CHAIN_DEPTH`; readers allow a larger bounded
/// depth while retaining cycle, size, and checksum validation.
pub const MAX_DELTA_CHAIN_READ_DEPTH: usize = DEFAULT_MAX_DELTA_CHAIN_DEPTH * 16;
pub const MIN_DELTA_BLOB_BYTES: usize = 32;
pub const MAX_DELTA_BLOB_BYTES: usize = 131_072;
pub const MIN_DELTA_SAVINGS_BYTES: usize = 16;

pub const PACK_FORMAT_KIND_ZSTD_CHUNKED_V1: &str = "zstd_chunked_v1";
pub const PACK_FORMAT_ZSTD_CHUNKED_V1: &str = "ait-pack-v3-zstd-chunked";
pub const DEFAULT_OBJECT_PACK_WRITE_FORMAT: &str = PACK_FORMAT_ZSTD_CHUNKED_V1;

pub const TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1: &str = "zstd_chunked_tree_v1";
pub const TREE_PACK_FORMAT_ZSTD_CHUNKED_V1: &str = "ait-tree-pack-v2-zstd-chunked";
pub const DEFAULT_TREE_PACK_WRITE_FORMAT: &str = TREE_PACK_FORMAT_ZSTD_CHUNKED_V1;
pub const ZSTD_CHUNKED_PACK_SUFFIX: &str = ".zstpack";

/// Pack archives are content-addressed by `PCK-*` / `TPK-*` identity. Their
/// embedded index must therefore be byte-stable across repositories; lifecycle
/// timestamps belong in Binary DB metadata, outside the content identity.
pub const CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT: &str = "1970-01-01T00:00:00Z";

const ZSTD_CHUNKED_VERSION: u32 = 1;
const ZSTD_CHUNKED_LEVEL: i32 = 3;
const ZSTD_CHUNKED_DEFAULT_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const ZSTD_CHUNKED_TRAILER_MAGIC: &[u8; 8] = b"AITZSTP1";
const ZSTD_CHUNKED_INDEX_MAGIC: &[u8; 8] = b"AITZIDX1";
const ZSTD_CHUNKED_TRAILER_LEN: usize = 8 + 4 + 8 + 8 + 32;
const ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME: &str = "zstd-chunked-object-index";
const ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME: &str = "zstd-chunked-tree-index";
const ZSTD_CHUNKED_INDEX_KIND_OBJECT: u8 = 1;
const ZSTD_CHUNKED_INDEX_KIND_TREE: u8 = 2;
const ZSTD_CHUNKED_TREE_MEMBER_MAGIC: &[u8; 8] = b"AITTREE1";

/// Validates reuse of an immutable remote zstd pack written before pack-index
/// timestamps became byte-stable.
///
/// Every container and index field, plus every uncompressed stored member,
/// must match the current locally prepared pack after excluding only the
/// index `created_at`. The returned checksum is the checksum of the actual
/// remote binary index and must be used in the remote commit payload.
pub fn validate_content_addressed_zstd_pack_reuse(
    local_pack_bytes: &[u8],
    remote_pack_bytes: &[u8],
    expected_pack_id: &str,
    pack_format: &str,
) -> Result<String, String> {
    let (kind, canonical_format) = match pack_format.trim() {
        PACK_FORMAT_ZSTD_CHUNKED_V1 | PACK_FORMAT_KIND_ZSTD_CHUNKED_V1 => {
            (ZSTD_CHUNKED_INDEX_KIND_OBJECT, PACK_FORMAT_ZSTD_CHUNKED_V1)
        }
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 | TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1 => (
            ZSTD_CHUNKED_INDEX_KIND_TREE,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        ),
        other => {
            return Err(format!(
                "Unsupported content-addressed zstd pack reuse format: {other}"
            ))
        }
    };
    let local_index =
        read_zstd_chunked_container_index_from_bytes(local_pack_bytes, kind, canonical_format)
            .map_err(|error| format!("Local zstd pack {expected_pack_id} is invalid: {error}"))?;
    let remote_index =
        read_zstd_chunked_container_index_from_bytes(remote_pack_bytes, kind, canonical_format)
            .map_err(|error| format!("Remote zstd pack {expected_pack_id} is invalid: {error}"))?;
    for (label, index) in [("local", &local_index), ("remote", &remote_index)] {
        if index.pack_id != expected_pack_id {
            return Err(format!(
                "Content-addressed zstd pack reuse {label} identity mismatch: expected {expected_pack_id}, got {}",
                index.pack_id
            ));
        }
    }
    if local_index.created_at != CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT {
        return Err(format!(
            "Local zstd pack {expected_pack_id} is not canonical: index created_at is {:?}",
            local_index.created_at
        ));
    }
    chrono::DateTime::parse_from_rfc3339(&remote_index.created_at).map_err(|error| {
        format!(
            "Remote zstd pack {expected_pack_id} has invalid index created_at {:?}: {error}",
            remote_index.created_at
        )
    })?;

    let mut normalized_remote_index = remote_index.clone();
    normalized_remote_index.created_at = local_index.created_at.clone();
    if normalized_remote_index != local_index {
        return Err(format!(
            "Remote zstd pack {expected_pack_id} differs from the canonical local pack beyond index created_at"
        ));
    }
    for (local_member, remote_member) in local_index.members.iter().zip(remote_index.members.iter())
    {
        let local_stored = read_zstd_chunked_member_stored_bytes_from_bytes(
            local_pack_bytes,
            &local_index,
            local_member,
        )
        .map_err(|error| {
            format!(
                "Local zstd pack {expected_pack_id} member {} is invalid: {error}",
                local_member.entry_name
            )
        })?;
        let remote_stored = read_zstd_chunked_member_stored_bytes_from_bytes(
            remote_pack_bytes,
            &remote_index,
            remote_member,
        )
        .map_err(|error| {
            format!(
                "Remote zstd pack {expected_pack_id} member {} is invalid: {error}",
                remote_member.entry_name
            )
        })?;
        if local_stored != remote_stored {
            return Err(format!(
                "Remote zstd pack {expected_pack_id} member {} differs from canonical local stored bytes",
                local_member.entry_name
            ));
        }
    }
    let remote_index_bytes = encode_zstd_chunked_index(&remote_index, kind)?;
    Ok(sha256_hex(&remote_index_bytes))
}

mod format;
mod index_json;
mod object;
mod tree_pack;
mod types;
mod util;
mod zstd;

pub use format::*;
use index_json::*;
pub use object::*;
pub use tree_pack::*;
pub use types::*;
use util::*;
use zstd::*;

#[cfg(test)]
mod tests;
