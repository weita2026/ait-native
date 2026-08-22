use serde_json::{json, Map, Number, Value as JsonValue};
use sha2::{Digest, Sha256};
use similar::{capture_diff_slices, Algorithm, DiffOp};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

#[path = "pack_substrate/delta.rs"]
mod delta;
#[path = "pack_substrate/json.rs"]
mod json;
#[path = "pack_substrate/object.rs"]
mod object;
#[path = "pack_substrate/tree.rs"]
mod tree;
#[path = "pack_substrate/zstd_chunked.rs"]
mod zstd_chunked;

pub use delta::{
    apply_git_binary_delta, apply_pack_delta, build_git_binary_delta, build_git_binary_delta_member,
};
pub use object::{
    build_pack_members, build_storage_validation_summary, object_pack_backend,
    object_pack_backend_from_persisted_format, pack_has_entry, pack_has_entry_with_format,
    read_pack_entry, read_pack_entry_with_format, read_pack_index,
    read_pack_index_checksum_with_format, read_pack_index_with_format,
    read_zstd_object_pack_blob_from_bytes, summarize_pack_archives, write_pack_archive,
    write_pack_archive_with_format, write_rebuilt_zstd_pack_archive,
};
pub use tree::{
    build_tree_pack_members, read_tree_pack_index, read_tree_pack_index_checksum_with_format,
    read_tree_pack_index_with_format, read_tree_pack_index_without_ordinals,
    read_tree_pack_index_without_ordinals_with_format, read_tree_pack_tree,
    read_tree_pack_tree_by_entry_name_with_format, read_tree_pack_tree_by_ordinal,
    read_tree_pack_tree_by_ordinal_with_format, read_tree_pack_tree_with_format,
    summarize_tree_pack_archives, tree_pack_backend, tree_pack_backend_from_persisted_format,
    tree_pack_contains_blob_ids, tree_pack_contains_blob_ids_with_format, write_tree_pack_archive,
    write_tree_pack_archive_with_format,
};

use json::*;
use tree::*;
use zstd_chunked::*;

#[cfg(test)]
pub(crate) fn reset_test_zstd_file_read_counts() {
    reset_test_zstd_container_file_read_counts();
}

#[cfg(test)]
pub(crate) fn test_zstd_file_read_counts() -> (u64, u64, u64) {
    test_zstd_container_file_read_counts()
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackFormatKind {
    ZstdChunkedV1,
}

impl PackFormatKind {
    pub fn from_persisted(value: &str) -> Result<Self, String> {
        match value.trim() {
            PACK_FORMAT_ZSTD_CHUNKED_V1 | PACK_FORMAT_KIND_ZSTD_CHUNKED_V1 => {
                Ok(Self::ZstdChunkedV1)
            }
            "" => Err("Missing object pack format metadata.".to_string()),
            other => Err(format!("Unsupported object pack format: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TreePackFormatKind {
    ZstdChunkedTreeV1,
}

impl TreePackFormatKind {
    pub fn from_persisted(value: &str) -> Result<Self, String> {
        match value.trim() {
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 | TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1 => {
                Ok(Self::ZstdChunkedTreeV1)
            }
            "" => Err("Missing tree-pack format metadata.".to_string()),
            other => Err(format!("Unsupported tree-pack format: {other}")),
        }
    }

    pub fn persisted_name(self) -> &'static str {
        match self {
            Self::ZstdChunkedTreeV1 => TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        }
    }

    pub fn format_kind_name(self) -> &'static str {
        match self {
            Self::ZstdChunkedTreeV1 => TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        }
    }
}

pub(in crate::foundation) struct ObjectPackIndexJson<S> {
    _store: PhantomData<fn() -> S>,
}

impl<S> ObjectPackIndexJson<S> {
    pub(in crate::foundation) const fn new() -> Self {
        Self {
            _store: PhantomData,
        }
    }

    pub(in crate::foundation) fn entries_by_name(
        &self,
        pack_index: &JsonValue,
    ) -> Result<BTreeMap<String, PackIndexEntry>, String> {
        validate_current_pack_index_header(
            pack_index,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
            ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME,
            "pack",
        )?;
        pack_entries_by_name(pack_index)
    }

    pub(in crate::foundation::pack_substrate) fn zstd_chunked_index_json(
        &self,
        pack_index: &ZstdChunkedPackIndex,
    ) -> Result<JsonValue, String> {
        validate_zstd_chunked_index(
            pack_index,
            ZSTD_CHUNKED_INDEX_KIND_OBJECT,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
            None,
        )?;
        zstd_chunked_object_pack_index_json(pack_index)
    }
}

impl<S> Default for ObjectPackIndexJson<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectPackIndexJson<()> {
    pub(in crate::foundation) const fn stateless() -> Self {
        Self::new()
    }
}

pub(in crate::foundation) struct TreePackIndexJson<S> {
    _store: PhantomData<fn() -> S>,
}

impl<S> TreePackIndexJson<S> {
    pub(in crate::foundation) const fn new() -> Self {
        Self {
            _store: PhantomData,
        }
    }

    pub(in crate::foundation) fn entries_by_id(
        &self,
        pack_index: &JsonValue,
    ) -> Result<BTreeMap<String, TreePackIndexEntry>, String> {
        validate_current_pack_index_header(
            pack_index,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME,
            "tree pack",
        )?;
        tree_entries_by_id(pack_index)
    }

    pub(in crate::foundation::pack_substrate) fn zstd_chunked_index_json(
        &self,
        pack_index: &ZstdChunkedPackIndex,
    ) -> Result<JsonValue, String> {
        validate_zstd_chunked_index(
            pack_index,
            ZSTD_CHUNKED_INDEX_KIND_TREE,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            None,
        )?;
        zstd_chunked_tree_pack_index_json(pack_index)
    }
}

impl<S> Default for TreePackIndexJson<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl TreePackIndexJson<()> {
    pub(in crate::foundation) const fn stateless() -> Self {
        Self::new()
    }
}

pub trait ObjectPackBackend: Sync {
    fn format_kind(&self) -> PackFormatKind;

    fn write_pack_archive(
        &self,
        pack_path: &str,
        pack_id: &str,
        created_at: &str,
        members: &JsonValue,
    ) -> Result<JsonValue, String>;

    fn read_pack_index(&self, pack_path: &str) -> Result<JsonValue, String>;

    fn pack_has_entry(&self, pack_path: &str, entry_name: &str) -> Result<bool, String>;

    fn read_pack_entry(
        &self,
        pack_path: &str,
        entry_name: &str,
        resolve_base_blob_map: Option<&BTreeMap<String, Vec<u8>>>,
        max_chain_depth: usize,
    ) -> Result<Vec<u8>, String>;
}

pub trait TreePackBackend: Sync {
    fn format_kind(&self) -> TreePackFormatKind;

    fn write_tree_pack_archive(
        &self,
        pack_path: &str,
        pack_id: &str,
        created_at: &str,
        members: &JsonValue,
    ) -> Result<JsonValue, String>;

    fn read_tree_pack_index(&self, pack_path: &str) -> Result<JsonValue, String>;

    fn read_tree_pack_index_without_ordinals(&self, pack_path: &str) -> Result<JsonValue, String>;

    fn read_tree_pack_tree(&self, pack_path: &str, tree_id: &str) -> Result<JsonValue, String>;

    fn read_tree_pack_tree_by_ordinal(
        &self,
        pack_path: &str,
        entry_ordinal: usize,
    ) -> Result<JsonValue, String>;

    fn read_tree_pack_tree_by_entry_name(
        &self,
        pack_path: &str,
        tree_id: &str,
        entry_name: &str,
        entry_count: usize,
        checksum: &str,
    ) -> Result<JsonValue, String>;

    fn tree_pack_contains_blob_ids(
        &self,
        pack_path: &str,
        blob_ids: &JsonValue,
    ) -> Result<JsonValue, String>;
}

#[derive(Debug)]
pub struct ZstdChunkedObjectPackBackend;

#[derive(Debug)]
pub struct ZstdChunkedTreePackBackend;

static ZSTD_CHUNKED_OBJECT_PACK_BACKEND: ZstdChunkedObjectPackBackend =
    ZstdChunkedObjectPackBackend;
static ZSTD_CHUNKED_TREE_PACK_BACKEND: ZstdChunkedTreePackBackend = ZstdChunkedTreePackBackend;

#[derive(Clone, Debug)]
struct PackCandidate {
    entry_name: String,
    blob_id: String,
    data: Vec<u8>,
    path_hint: Option<String>,
    chain_depth: usize,
}

/// Logical blob input used when a caller must rebuild a zstd pack without
/// expanding every byte into a JSON number array first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPackRewriteBlob {
    pub entry_name: String,
    pub blob_id: String,
    pub data: Vec<u8>,
    pub path_hint: Option<String>,
}

#[derive(Clone, Debug)]
struct PackMember {
    entry_name: String,
    blob_id: String,
    data: Vec<u8>,
    logical_data: Vec<u8>,
    entry_type: String,
    base_blob_id: Option<String>,
    chain_depth: usize,
    delta_algorithm: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::foundation) struct PackIndexEntry {
    pub(in crate::foundation) entry_name: String,
    pub(in crate::foundation) blob_id: String,
    pub(in crate::foundation) entry_type: String,
    pub(in crate::foundation) byte_length: usize,
    pub(in crate::foundation) uncompressed_byte_length: usize,
    pub(in crate::foundation) base_blob_id: Option<String>,
    pub(in crate::foundation) chain_depth: usize,
    pub(in crate::foundation) checksum: String,
    pub(in crate::foundation) delta_algorithm: Option<String>,
}

#[derive(Clone, Debug)]
struct ZstdChunkedPackIndex {
    pack_format: String,
    pack_id: String,
    created_at: String,
    index_entry_name: String,
    chunks: Vec<ZstdChunkedChunkIndex>,
    members: Vec<ZstdChunkedMemberIndex>,
}

#[derive(Clone, Debug)]
struct ZstdChunkedChunkIndex {
    chunk_ordinal: usize,
    compressed_offset: u64,
    compressed_len: usize,
    raw_len: usize,
    checksum: String,
}

#[derive(Clone, Debug)]
struct ZstdChunkedMemberIndex {
    member_ordinal: usize,
    entry_name: String,
    content_id: String,
    entry_type: String,
    entry_count: Option<usize>,
    base_content_id: Option<String>,
    delta_algorithm: Option<String>,
    chain_depth: usize,
    chunk_ordinal: usize,
    in_chunk_offset: usize,
    stored_len: usize,
    logical_len: usize,
    checksum: String,
}

#[derive(Debug)]
pub struct PackEntryArchive {
    pack_path: String,
    pack_index: ZstdChunkedPackIndex,
    raw_chunk_cache: BTreeMap<usize, Vec<u8>>,
    entries_by_name: BTreeMap<String, PackIndexEntry>,
}

#[derive(Debug)]
pub struct TreePackEntryArchive {
    pack_path: String,
    pack_index: ZstdChunkedPackIndex,
    raw_chunk_cache: BTreeMap<usize, Vec<u8>>,
}

#[derive(Clone, Debug)]
struct TreePackMember {
    tree_id: String,
    entry_name: String,
    entry_count: usize,
    data: Vec<u8>,
    checksum: String,
}

#[derive(Clone, Debug)]
pub(in crate::foundation) struct TreePackIndexEntry {
    pub(in crate::foundation) tree_id: String,
    pub(in crate::foundation) entry_ordinal: usize,
    pub(in crate::foundation) entry_count: usize,
    pub(in crate::foundation) checksum: String,
}

pub fn tree_pack_manifest_path(pack_path: &str, entry_name: &str) -> String {
    format!("{pack_path}#{entry_name}")
}

#[cfg(test)]
mod tests;
