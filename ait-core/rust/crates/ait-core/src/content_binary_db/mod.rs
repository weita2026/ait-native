use std::path::{Path, PathBuf};

use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbFsyncPolicy, BinaryDbReadScope, BinaryDbReadTxn,
    BinaryDbStoreFsyncPolicy, BinaryDbWriteTxn, BinaryFileId, BinaryIndexId, BinaryPayloadFileId,
    StorePath, StoreResult,
};

pub mod adapters;
pub mod read;
pub mod schema;
pub mod write;

pub use adapters::{LocalContentBinaryDb, RemoteContentBinaryDb, RemoteFsContentBinaryDb};
pub use read::{
    BinaryBlobView, BinaryDbSnapshotReader, BinaryDbTreeReadCache, BinaryObjectPackMemberView,
    BinaryObjectPackView, BinarySnapshotView, BinaryTreeEntryView, BinaryTreePackView,
    BinaryTreeRootLocator, BinaryTreeRootResolver, BinaryTreeView, StaticBinaryTreeRootResolver,
};
pub use schema::{
    BinaryBlobCodec, BinaryBlobRecord, BinaryObjectPackCodec, BinaryObjectPackCompressionKind,
    BinaryObjectPackFormatKind, BinaryObjectPackMemberCodec, BinaryObjectPackMemberKind,
    BinaryObjectPackMemberRecord, BinaryObjectPackRecord, BinarySnapshotCodec, BinarySnapshotKind,
    BinarySnapshotPayload, BinarySnapshotRecord, BinaryTreeCodec, BinaryTreePackCodec,
    BinaryTreePackFormatKind, BinaryTreePackRecord, BinaryTreeRecord, BINARY_DB_CONTENT_LAYOUT_ID,
    BLOB_BIN, BLOB_ID_IDX, BLOB_RECORD_SIZE, MAX_SNAPSHOT_PARENT_COUNT, OBJECT_PACK_BIN,
    OBJECT_PACK_ID_IDX, OBJECT_PACK_MEMBER_BIN, OBJECT_PACK_MEMBER_RECORD_SIZE,
    OBJECT_PACK_RECORD_SIZE, SNAPSHOT_BIN, SNAPSHOT_ID_IDX, SNAPSHOT_PARENT_EXTENSION_VERSION,
    SNAPSHOT_PAYLOAD_BIN, SNAPSHOT_RECORD_SIZE, TREE_BIN, TREE_ID_IDX, TREE_PACK_BIN,
    TREE_PACK_ID_IDX, TREE_PACK_RECORD_SIZE, TREE_RECORD_SIZE,
};
#[cfg(test)]
pub(crate) use schema::{
    BLOB_RECORD_SIZE_USIZE, OBJECT_PACK_MEMBER_RECORD_SIZE_USIZE, OBJECT_PACK_RECORD_SIZE_USIZE,
    SNAPSHOT_RECORD_SIZE_USIZE, TREE_PACK_RECORD_SIZE_USIZE, TREE_RECORD_SIZE_USIZE,
};
pub use write::{
    BinaryDbContentWriteCoordinator, BinaryDbObjectPackMemberWriteInput,
    BinaryDbObjectPackWriteInput, BinaryDbSnapshotWriteInput, BinaryDbTreeEntryWriteInput,
    BinaryDbTreePackTreeWriteInput, BinaryDbTreePackWriteInput,
};

#[derive(Clone, Debug)]
pub struct BinaryDbBlobStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    db: B,
    repo_root: StorePath,
}

#[derive(Clone, Debug)]
pub struct BinaryDbSnapshotStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    db: B,
    repo_root: StorePath,
}

#[derive(Clone, Debug)]
pub struct BinaryDbObjectPackStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    db: B,
    repo_root: StorePath,
}

#[derive(Clone, Debug)]
pub struct BinaryDbTreePackStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    db: B,
    repo_root: StorePath,
}

#[derive(Clone, Debug)]
pub struct BinaryDbTreeStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    db: B,
    repo_root: StorePath,
}

macro_rules! impl_store_common {
    ($name:ident) => {
        impl<B, const WRITE_LAYOUT: u32> $name<B, WRITE_LAYOUT>
        where
            B: BinaryDb,
        {
            pub fn new(db: B, repo_root: impl Into<StorePath>) -> Self {
                Self {
                    db,
                    repo_root: repo_root.into(),
                }
            }

            pub fn db(&self) -> &B {
                &self.db
            }

            pub fn repo_root(&self) -> &StorePath {
                &self.repo_root
            }

            pub fn authority_root(&self) -> &StorePath {
                self.db.authority_root()
            }

            pub fn begin_read_txn(&self) -> BinaryDbReadTxn<'_, B> {
                BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Content)
            }

            pub fn begin_write_txn(
                &self,
                command_scope: BinaryDbCommandScope,
            ) -> StoreResult<BinaryDbWriteTxn<'_, B, BinaryDbStoreFsyncPolicy<'_, B>>> {
                BinaryDbWriteTxn::begin(&self.db, command_scope)
            }

            pub fn begin_write_txn_with_fsync_policy<F>(
                &self,
                command_scope: BinaryDbCommandScope,
                fsync_policy: F,
            ) -> StoreResult<BinaryDbWriteTxn<'_, B, F>>
            where
                F: BinaryDbFsyncPolicy,
            {
                BinaryDbWriteTxn::begin_with_fsync_policy(&self.db, command_scope, fsync_policy)
            }
        }
    };
}

impl_store_common!(BinaryDbBlobStore);
impl_store_common!(BinaryDbSnapshotStore);
impl_store_common!(BinaryDbObjectPackStore);
impl_store_common!(BinaryDbTreePackStore);
impl_store_common!(BinaryDbTreeStore);

impl<B, const WRITE_LAYOUT: u32> BinaryDbBlobStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn blob_file() -> BinaryFileId {
        BinaryBlobCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn blob_id_index() -> BinaryIndexId {
        BinaryBlobCodec::<WRITE_LAYOUT>::id_index()
    }

    pub fn object_pack_file() -> BinaryFileId {
        BinaryObjectPackCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn object_pack_member_file() -> BinaryFileId {
        BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::record_file()
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn snapshot_file() -> BinaryFileId {
        BinarySnapshotCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn snapshot_id_index() -> BinaryIndexId {
        BinarySnapshotCodec::<WRITE_LAYOUT>::id_index()
    }

    pub fn snapshot_payload_file() -> BinaryPayloadFileId {
        BinarySnapshotCodec::<WRITE_LAYOUT>::payload_file()
    }

    pub fn tree_pack_file() -> BinaryFileId {
        BinaryTreePackCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn tree_file() -> BinaryFileId {
        BinaryTreeCodec::<WRITE_LAYOUT>::record_file()
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbObjectPackStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn object_pack_file() -> BinaryFileId {
        BinaryObjectPackCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn object_pack_id_index() -> BinaryIndexId {
        BinaryObjectPackCodec::<WRITE_LAYOUT>::id_index()
    }

    pub fn object_pack_member_file() -> BinaryFileId {
        BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn blob_file() -> BinaryFileId {
        BinaryBlobCodec::<WRITE_LAYOUT>::record_file()
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbTreePackStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn tree_pack_file() -> BinaryFileId {
        BinaryTreePackCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn tree_pack_id_index() -> BinaryIndexId {
        BinaryTreePackCodec::<WRITE_LAYOUT>::id_index()
    }

    pub fn tree_file() -> BinaryFileId {
        BinaryTreeCodec::<WRITE_LAYOUT>::record_file()
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbTreeStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn tree_file() -> BinaryFileId {
        BinaryTreeCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn tree_id_index() -> BinaryIndexId {
        BinaryTreeCodec::<WRITE_LAYOUT>::id_index()
    }

    pub fn tree_pack_file() -> BinaryFileId {
        BinaryTreePackCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn blob_file() -> BinaryFileId {
        BinaryBlobCodec::<WRITE_LAYOUT>::record_file()
    }
}

pub fn blob_id_from_sha256(sha256: &[u8; 32]) -> String {
    format!("BLB-{}", hex_lower(&sha256[..10]))
}

pub fn tree_id_from_hash80(hash80: &[u8; 10]) -> String {
    format!("TRE-{}", hex_upper(hash80))
}

pub fn object_pack_id_from_hash48(hash48: u64) -> String {
    format!("PCK-{hash48:012X}")
}

pub fn tree_pack_id_from_hash48(hash48: u64) -> String {
    format!("TPK-{hash48:012X}")
}

pub fn snapshot_id_from_hash48(hash48: u64) -> String {
    format!("SNP-{hash48:012X}")
}

pub fn object_pack_hash48_from_id(pack_id: &str) -> StoreResult<u64> {
    prefixed_hash48_from_id(pack_id, "PCK-")
}

pub fn tree_pack_hash48_from_id(pack_id: &str) -> StoreResult<u64> {
    prefixed_hash48_from_id(pack_id, "TPK-")
}

pub fn snapshot_hash48_from_id(snapshot_id: &str) -> StoreResult<u64> {
    prefixed_hash48_from_id(snapshot_id, "SNP-")
}

pub fn blob_id_index_key(blob_id: &str) -> StoreResult<[u8; 10]> {
    prefixed_hex_key(blob_id, "BLB-")
}

pub fn tree_id_index_key(tree_id: &str) -> StoreResult<[u8; 10]> {
    prefixed_hex_key(tree_id, "TRE-")
}

pub fn object_pack_id_index_key(pack_id: &str) -> StoreResult<[u8; 8]> {
    Ok(object_pack_hash48_from_id(pack_id)?.to_le_bytes())
}

pub fn tree_pack_id_index_key(pack_id: &str) -> StoreResult<[u8; 8]> {
    Ok(tree_pack_hash48_from_id(pack_id)?.to_le_bytes())
}

pub fn snapshot_id_index_key(snapshot_id: &str) -> StoreResult<[u8; 8]> {
    Ok(snapshot_hash48_from_id(snapshot_id)?.to_le_bytes())
}

pub(crate) fn absolute_repo_path(
    repo_root: &StorePath,
    relative_path: &str,
) -> StoreResult<PathBuf> {
    let path = Path::new(relative_path);
    if path.is_absolute() {
        return Err(format!("content pack path must be repo-relative: {relative_path}").into());
    }
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::RootDir
        ) {
            return Err(format!("content pack path must not escape repo: {relative_path}").into());
        }
    }
    Ok(repo_root.as_path().join(path))
}

pub(crate) fn object_pack_relative_path(pack_id: &str, pack_format: &str) -> StoreResult<String> {
    match pack_format {
        crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1 => Ok(
            crate::pack_substrate::default_object_pack_relative_path(pack_id),
        ),
        other => Err(format!("unsupported object pack format for Binary DB path: {other}").into()),
    }
}

pub(crate) fn tree_pack_relative_path(pack_id: &str, pack_format: &str) -> StoreResult<String> {
    match pack_format {
        crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 => Ok(
            crate::pack_substrate::default_tree_pack_relative_path(pack_id),
        ),
        other => Err(format!("unsupported tree pack format for Binary DB path: {other}").into()),
    }
}

pub(crate) fn object_pack_format_name(
    kind: BinaryObjectPackFormatKind,
) -> StoreResult<&'static str> {
    match kind {
        BinaryObjectPackFormatKind::ZstdChunkedV1 => {
            Ok(crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1)
        }
        BinaryObjectPackFormatKind::Reserved(value) => {
            Err(format!("unsupported object pack format kind: {value}").into())
        }
    }
}

pub(crate) fn tree_pack_format_name(kind: BinaryTreePackFormatKind) -> StoreResult<&'static str> {
    match kind {
        BinaryTreePackFormatKind::ZstdChunkedTreeV1 => {
            Ok(crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
        }
        BinaryTreePackFormatKind::Reserved(value) => {
            Err(format!("unsupported tree pack format kind: {value}").into())
        }
    }
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub(crate) fn hex_upper(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02X}"));
    }
    out
}

fn prefixed_hash48_from_id(value: &str, prefix: &str) -> StoreResult<u64> {
    let key = prefixed_hex_key::<6>(value, prefix)?;
    Ok(key
        .iter()
        .fold(0_u64, |acc, byte| (acc << 8) | u64::from(*byte)))
}

fn prefixed_hex_key<const N: usize>(value: &str, prefix: &str) -> StoreResult<[u8; N]> {
    let value = value.trim();
    if value.len() < prefix.len() || !value[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return Err(format!("id `{value}` must start with {prefix}").into());
    };
    let hex = &value[prefix.len()..];
    if hex.len() != N * 2 {
        return Err(format!(
            "id `{value}` has {} hex chars, expected {}",
            hex.len(),
            N * 2
        )
        .into());
    }
    let mut out = [0_u8; N];
    for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_nibble(chunk[0]).ok_or_else(|| format!("invalid hex id `{value}`"))?;
        let lo = hex_nibble(chunk[1]).ok_or_else(|| format!("invalid hex id `{value}`"))?;
        out[index] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
