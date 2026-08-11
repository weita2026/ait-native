use crate::binary_db::{
    BinaryDbError, BinaryFileId, BinaryIndexId, BinaryPayloadFileId, StoreResult,
};

pub const BINARY_DB_CONTENT_LAYOUT_ID: u32 = 1;

use super::files::{
    BLOB_BIN, BLOB_ID_IDX, BLOB_RECORD_SIZE, BLOB_RECORD_SIZE_USIZE, OBJECT_PACK_BIN,
    OBJECT_PACK_ID_IDX, OBJECT_PACK_MEMBER_BIN, OBJECT_PACK_MEMBER_RECORD_SIZE,
    OBJECT_PACK_MEMBER_RECORD_SIZE_USIZE, OBJECT_PACK_RECORD_SIZE, OBJECT_PACK_RECORD_SIZE_USIZE,
    SNAPSHOT_BIN, SNAPSHOT_ID_IDX, SNAPSHOT_PAYLOAD_BIN, SNAPSHOT_RECORD_SIZE,
    SNAPSHOT_RECORD_SIZE_USIZE, TREE_BIN, TREE_ID_IDX, TREE_PACK_BIN, TREE_PACK_ID_IDX,
    TREE_PACK_RECORD_SIZE, TREE_PACK_RECORD_SIZE_USIZE, TREE_RECORD_SIZE, TREE_RECORD_SIZE_USIZE,
};
use super::payloads::{
    BinarySnapshotPayload, MAX_SNAPSHOT_PARENT_COUNT, SNAPSHOT_PARENT_EXTENSION_VERSION,
};
use super::records::{
    BinaryBlobRecord, BinaryObjectPackMemberRecord, BinaryObjectPackRecord, BinarySnapshotRecord,
    BinaryTreePackRecord, BinaryTreeRecord,
};

pub struct BinaryBlobCodec<const LAYOUT: u32>;
pub struct BinarySnapshotCodec<const LAYOUT: u32>;
pub struct BinaryObjectPackCodec<const LAYOUT: u32>;
pub struct BinaryObjectPackMemberCodec<const LAYOUT: u32>;
pub struct BinaryTreePackCodec<const LAYOUT: u32>;
pub struct BinaryTreeCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> BinaryBlobCodec<LAYOUT> {
    pub const RECORD_SIZE: u32 = BLOB_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(BLOB_BIN, LAYOUT, BLOB_RECORD_SIZE)
    }

    pub fn id_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(BLOB_ID_IDX, LAYOUT, 10, true)
    }

    pub fn encode_record(record: &BinaryBlobRecord) -> StoreResult<Vec<u8>> {
        require_supported_content_layout::<LAYOUT>()?;
        let mut out = Vec::with_capacity(BLOB_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.blob_meta);
        push_u8(&mut out, record.hash_kind);
        push_u16(&mut out, record.reserved0);
        push_u64(&mut out, record.size_bytes);
        push_u32(&mut out, record.pack_member_index_plus1);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.pruned_at_s);
        out.extend_from_slice(&record.sha256);
        require_encoded_len(&out, BLOB_RECORD_SIZE_USIZE, "BinaryBlobRecord")?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<BinaryBlobRecord> {
        require_supported_content_layout::<LAYOUT>()?;
        require_raw_len(raw, BLOB_RECORD_SIZE_USIZE, "BinaryBlobRecord")?;
        let mut cursor = Cursor::new(raw);
        let blob_meta = cursor.take_u8("blob_meta")?;
        let hash_kind = cursor.take_u8("hash_kind")?;
        let reserved0 = cursor.take_u16("reserved0")?;
        let size_bytes = cursor.take_u64("size_bytes")?;
        let pack_member_index_plus1 = cursor.take_u32("pack_member_index_plus1")?;
        let created_at_s = cursor.take_u64("created_at_s")?;
        let pruned_at_s = cursor.take_u64("pruned_at_s")?;
        let sha256 = cursor.take_array_32("sha256")?;
        Ok(BinaryBlobRecord {
            blob_meta,
            hash_kind,
            reserved0,
            size_bytes,
            pack_member_index_plus1,
            created_at_s,
            pruned_at_s,
            sha256,
        })
    }
}

impl<const LAYOUT: u32> BinarySnapshotCodec<LAYOUT> {
    pub const RECORD_SIZE: u32 = SNAPSHOT_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(SNAPSHOT_BIN, LAYOUT, SNAPSHOT_RECORD_SIZE)
    }

    pub fn id_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(SNAPSHOT_ID_IDX, LAYOUT, 8, true)
    }

    pub fn payload_file() -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(SNAPSHOT_PAYLOAD_BIN, LAYOUT)
    }

    pub fn encode_record(record: &BinarySnapshotRecord) -> StoreResult<Vec<u8>> {
        require_supported_snapshot_layout::<LAYOUT>()?;
        validate_snapshot_record(record)?;
        let mut out = Vec::with_capacity(SNAPSHOT_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.snapshot_meta);
        push_u8(&mut out, record.history_flags);
        push_u16(&mut out, record.payload_len);
        push_u64(&mut out, record.payload_offset);
        push_u64(&mut out, record.snapshot_hash48);
        push_u32(&mut out, record.parent_snapshot_index_plus1);
        push_u32(&mut out, record.root_tree_pack_index_plus1);
        push_u32(&mut out, record.root_entry_ordinal);
        push_u32(&mut out, record.line_index_plus1);
        out.extend_from_slice(&record.manifest_hash);
        push_u32(&mut out, record.file_count);
        push_u64(&mut out, record.total_bytes);
        push_u64(&mut out, record.created_at_s);
        require_encoded_len(&out, SNAPSHOT_RECORD_SIZE_USIZE, "BinarySnapshotRecord")?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<BinarySnapshotRecord> {
        require_supported_snapshot_layout::<LAYOUT>()?;
        require_raw_len(raw, SNAPSHOT_RECORD_SIZE_USIZE, "BinarySnapshotRecord")?;
        let mut cursor = Cursor::new(raw);
        let record = BinarySnapshotRecord {
            snapshot_meta: cursor.take_u8("snapshot_meta")?,
            history_flags: cursor.take_u8("history_flags")?,
            payload_len: cursor.take_u16("payload_len")?,
            payload_offset: cursor.take_u64("payload_offset")?,
            snapshot_hash48: cursor.take_u64("snapshot_hash48")?,
            parent_snapshot_index_plus1: cursor.take_u32("parent_snapshot_index_plus1")?,
            root_tree_pack_index_plus1: cursor.take_u32("root_tree_pack_index_plus1")?,
            root_entry_ordinal: cursor.take_u32("root_entry_ordinal")?,
            line_index_plus1: cursor.take_u32("line_index_plus1")?,
            manifest_hash: cursor.take_array_32("manifest_hash")?,
            file_count: cursor.take_u32("file_count")?,
            total_bytes: cursor.take_u64("total_bytes")?,
            created_at_s: cursor.take_u64("created_at_s")?,
        };
        validate_snapshot_record(&record)?;
        Ok(record)
    }

    pub fn encode_payload(payload: &BinarySnapshotPayload) -> StoreResult<Vec<u8>> {
        require_supported_snapshot_layout::<LAYOUT>()?;
        let line_name = payload.line_name.as_bytes();
        let message = payload.message.as_deref().unwrap_or("").as_bytes();
        let additional_parent_count = payload.additional_parent_snapshot_indices.len();
        if additional_parent_count >= MAX_SNAPSHOT_PARENT_COUNT {
            return Err(format!(
                "snapshot parent count exceeds {MAX_SNAPSHOT_PARENT_COUNT}: {}",
                additional_parent_count + 1
            )
            .into());
        }
        let message_len = u16::try_from(message.len())
            .map_err(|_| "snapshot message exceeds u16::MAX bytes".to_string())?;
        let extension_len = if additional_parent_count == 0 {
            0
        } else {
            3 + additional_parent_count * 4
        };
        let mut out = Vec::with_capacity(extension_len + 2 + line_name.len() + message.len());
        if additional_parent_count > 0 {
            push_u8(&mut out, SNAPSHOT_PARENT_EXTENSION_VERSION);
            push_u16(
                &mut out,
                u16::try_from(additional_parent_count)
                    .map_err(|_| "snapshot additional parent count exceeds u16::MAX")?,
            );
            for parent_index in &payload.additional_parent_snapshot_indices {
                push_u32(
                    &mut out,
                    parent_index
                        .checked_add(1)
                        .ok_or_else(|| "snapshot parent index plus-one overflow".to_string())?,
                );
            }
        }
        push_u16(&mut out, message_len);
        out.extend_from_slice(message);
        out.extend_from_slice(line_name);
        if out.len() > usize::from(u16::MAX) {
            return Err(format!("snapshot payload exceeds u16::MAX bytes: {}", out.len()).into());
        }
        Ok(out)
    }

    pub fn decode_payload(
        raw: &[u8],
        has_line_name_payload: bool,
        has_additional_parents: bool,
    ) -> StoreResult<BinarySnapshotPayload> {
        require_supported_snapshot_layout::<LAYOUT>()?;
        let mut cursor = Cursor::new(raw);
        let additional_parent_snapshot_indices = if has_additional_parents {
            let version = cursor.take_u8("parent_extension_version")?;
            if version != SNAPSHOT_PARENT_EXTENSION_VERSION {
                return Err(format!(
                    "unsupported snapshot parent extension version: {version}; supported version is {SNAPSHOT_PARENT_EXTENSION_VERSION}"
                )
                .into());
            }
            let additional_parent_count = usize::from(cursor.take_u16("additional_parent_count")?);
            if additional_parent_count == 0 {
                return Err(
                    "snapshot additional-parent flag requires at least one additional parent"
                        .into(),
                );
            }
            if additional_parent_count >= MAX_SNAPSHOT_PARENT_COUNT {
                return Err(format!(
                    "snapshot parent count exceeds {MAX_SNAPSHOT_PARENT_COUNT}: {}",
                    additional_parent_count + 1
                )
                .into());
            }
            let required_bytes = additional_parent_count
                .checked_mul(4)
                .and_then(|value| value.checked_add(2))
                .ok_or_else(|| "snapshot parent extension length overflow".to_string())?;
            if raw.len().saturating_sub(cursor.offset) < required_bytes {
                return Err("snapshot parent extension is truncated".into());
            }
            let mut indices = Vec::with_capacity(additional_parent_count);
            for _ in 0..additional_parent_count {
                let index = cursor
                    .take_u32("additional_parent_snapshot_index_plus1")?
                    .checked_sub(1)
                    .ok_or_else(|| {
                        "snapshot additional parent contains a zero plus-one index".to_string()
                    })?;
                indices.push(index);
            }
            indices
        } else {
            Vec::new()
        };
        let message_len = usize::from(cursor.take_u16("message_len")?);
        let message_bytes = cursor.take(message_len, "message")?;
        let line_name_bytes = cursor.take(raw.len() - cursor.offset, "line_name")?;
        if !has_line_name_payload && !line_name_bytes.is_empty() {
            return Err("BinarySnapshotPayload has line-name bytes without metadata flag".into());
        }
        let line_name = String::from_utf8(line_name_bytes.to_vec())
            .map_err(|err| format!("snapshot line_name is not UTF-8: {err}"))?;
        let message = if message_bytes.is_empty() {
            None
        } else {
            Some(
                String::from_utf8(message_bytes.to_vec())
                    .map_err(|err| format!("snapshot message is not UTF-8: {err}"))?,
            )
        };
        Ok(BinarySnapshotPayload {
            line_name,
            message,
            additional_parent_snapshot_indices,
        })
    }
}

fn require_supported_content_layout<const LAYOUT: u32>() -> StoreResult<()> {
    if LAYOUT == BINARY_DB_CONTENT_LAYOUT_ID {
        Ok(())
    } else {
        Err(BinaryDbError::layout_mismatch(format!(
            "unsupported Binary DB content layout: {LAYOUT}; supported layout is {BINARY_DB_CONTENT_LAYOUT_ID}"
        )))
    }
}

fn require_supported_snapshot_layout<const LAYOUT: u32>() -> StoreResult<()> {
    if LAYOUT == BINARY_DB_CONTENT_LAYOUT_ID {
        Ok(())
    } else {
        Err(BinaryDbError::layout_mismatch(format!(
            "unsupported Binary DB snapshot layout: {LAYOUT}; supported layout is {BINARY_DB_CONTENT_LAYOUT_ID}"
        )))
    }
}

fn validate_snapshot_record(record: &BinarySnapshotRecord) -> StoreResult<()> {
    const RESERVED_META_BITS: u8 = 0b0100_0000;
    if record.snapshot_meta & RESERVED_META_BITS != 0 {
        return Err("BinarySnapshotRecord has reserved snapshot_meta bits set".into());
    }
    if record.history_flags & !BinarySnapshotRecord::KNOWN_FLAGS != 0 {
        return Err("BinarySnapshotRecord has unknown history flags".into());
    }
    if record.snapshot_hash48 >> 48 != 0 {
        return Err("BinarySnapshotRecord snapshot_hash48 high 16 bits must be zero".into());
    }
    if record.payload_len > 0 && record.payload_offset < 4 {
        return Err("BinarySnapshotRecord payload_offset must follow the layout header".into());
    }
    if record.has_additional_parents() && record.parent_snapshot_index().is_none() {
        return Err(
            "BinarySnapshotRecord additional-parent flag requires an ordinal-zero parent".into(),
        );
    }
    if record.has_additional_parents() && record.payload_len < 9 {
        return Err("BinarySnapshotRecord additional-parent payload is too short".into());
    }
    if record.is_remote_head_history_boundary()
        && (record.parent_snapshot_index().is_some() || record.has_additional_parents())
    {
        return Err(
            "BinarySnapshotRecord remote-head history boundary cannot have local parents".into(),
        );
    }
    if record.has_root_locator() != (record.root_tree_pack_index_plus1 != 0) {
        return Err(
            "BinarySnapshotRecord root locator flag and pack index must be set together".into(),
        );
    }
    Ok(())
}

impl<const LAYOUT: u32> BinaryObjectPackCodec<LAYOUT> {
    pub const RECORD_SIZE: u32 = OBJECT_PACK_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(OBJECT_PACK_BIN, LAYOUT, OBJECT_PACK_RECORD_SIZE)
    }

    pub fn id_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(OBJECT_PACK_ID_IDX, LAYOUT, 8, true)
    }

    pub fn encode_record(record: &BinaryObjectPackRecord) -> StoreResult<Vec<u8>> {
        require_supported_content_layout::<LAYOUT>()?;
        let mut out = Vec::with_capacity(OBJECT_PACK_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.pack_meta);
        push_u8(&mut out, record.pack_format_kind);
        push_u16(&mut out, record.pack_hash_hi16);
        push_u32(&mut out, record.pack_hash_lo32);
        push_u32(&mut out, record.first_member_index);
        push_u32(&mut out, record.member_count);
        push_u64(&mut out, record.total_bytes);
        push_u64(&mut out, record.created_at_s);
        require_encoded_len(
            &out,
            OBJECT_PACK_RECORD_SIZE_USIZE,
            "BinaryObjectPackRecord",
        )?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<BinaryObjectPackRecord> {
        require_supported_content_layout::<LAYOUT>()?;
        require_raw_len(raw, OBJECT_PACK_RECORD_SIZE_USIZE, "BinaryObjectPackRecord")?;
        let mut cursor = Cursor::new(raw);
        Ok(BinaryObjectPackRecord {
            pack_meta: cursor.take_u8("pack_meta")?,
            pack_format_kind: cursor.take_u8("pack_format_kind")?,
            pack_hash_hi16: cursor.take_u16("pack_hash_hi16")?,
            pack_hash_lo32: cursor.take_u32("pack_hash_lo32")?,
            first_member_index: cursor.take_u32("first_member_index")?,
            member_count: cursor.take_u32("member_count")?,
            total_bytes: cursor.take_u64("total_bytes")?,
            created_at_s: cursor.take_u64("created_at_s")?,
        })
    }
}

impl<const LAYOUT: u32> BinaryObjectPackMemberCodec<LAYOUT> {
    pub const RECORD_SIZE: u32 = OBJECT_PACK_MEMBER_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(
            OBJECT_PACK_MEMBER_BIN,
            LAYOUT,
            OBJECT_PACK_MEMBER_RECORD_SIZE,
        )
    }

    pub fn encode_record(record: &BinaryObjectPackMemberRecord) -> StoreResult<Vec<u8>> {
        require_supported_content_layout::<LAYOUT>()?;
        let mut out = Vec::with_capacity(OBJECT_PACK_MEMBER_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.member_meta);
        push_u8(&mut out, record.delta_chain_depth);
        push_u16(&mut out, record.reserved0);
        push_u32(&mut out, record.pack_index);
        push_u32(&mut out, record.blob_index);
        push_u32(&mut out, record.base_blob_index_plus1);
        require_encoded_len(
            &out,
            OBJECT_PACK_MEMBER_RECORD_SIZE_USIZE,
            "BinaryObjectPackMemberRecord",
        )?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<BinaryObjectPackMemberRecord> {
        require_supported_content_layout::<LAYOUT>()?;
        require_raw_len(
            raw,
            OBJECT_PACK_MEMBER_RECORD_SIZE_USIZE,
            "BinaryObjectPackMemberRecord",
        )?;
        let mut cursor = Cursor::new(raw);
        Ok(BinaryObjectPackMemberRecord {
            member_meta: cursor.take_u8("member_meta")?,
            delta_chain_depth: cursor.take_u8("delta_chain_depth")?,
            reserved0: cursor.take_u16("reserved0")?,
            pack_index: cursor.take_u32("pack_index")?,
            blob_index: cursor.take_u32("blob_index")?,
            base_blob_index_plus1: cursor.take_u32("base_blob_index_plus1")?,
        })
    }
}

impl<const LAYOUT: u32> BinaryTreePackCodec<LAYOUT> {
    pub const RECORD_SIZE: u32 = TREE_PACK_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(TREE_PACK_BIN, LAYOUT, TREE_PACK_RECORD_SIZE)
    }

    pub fn id_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(TREE_PACK_ID_IDX, LAYOUT, 8, true)
    }

    pub fn encode_record(record: &BinaryTreePackRecord) -> StoreResult<Vec<u8>> {
        require_supported_content_layout::<LAYOUT>()?;
        let mut out = Vec::with_capacity(TREE_PACK_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.pack_meta);
        push_u8(&mut out, record.pack_format_kind);
        push_u16(&mut out, record.pack_hash_hi16);
        push_u32(&mut out, record.pack_hash_lo32);
        push_u32(&mut out, record.first_tree_index);
        push_u32(&mut out, record.tree_count);
        push_u64(&mut out, record.total_bytes);
        push_u64(&mut out, record.created_at_s);
        require_encoded_len(&out, TREE_PACK_RECORD_SIZE_USIZE, "BinaryTreePackRecord")?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<BinaryTreePackRecord> {
        require_supported_content_layout::<LAYOUT>()?;
        require_raw_len(raw, TREE_PACK_RECORD_SIZE_USIZE, "BinaryTreePackRecord")?;
        let mut cursor = Cursor::new(raw);
        Ok(BinaryTreePackRecord {
            pack_meta: cursor.take_u8("pack_meta")?,
            pack_format_kind: cursor.take_u8("pack_format_kind")?,
            pack_hash_hi16: cursor.take_u16("pack_hash_hi16")?,
            pack_hash_lo32: cursor.take_u32("pack_hash_lo32")?,
            first_tree_index: cursor.take_u32("first_tree_index")?,
            tree_count: cursor.take_u32("tree_count")?,
            total_bytes: cursor.take_u64("total_bytes")?,
            created_at_s: cursor.take_u64("created_at_s")?,
        })
    }
}

impl<const LAYOUT: u32> BinaryTreeCodec<LAYOUT> {
    pub const RECORD_SIZE: u32 = TREE_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(TREE_BIN, LAYOUT, TREE_RECORD_SIZE)
    }

    pub fn id_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(TREE_ID_IDX, LAYOUT, 10, true)
    }

    pub fn encode_record(record: &BinaryTreeRecord) -> StoreResult<Vec<u8>> {
        require_supported_content_layout::<LAYOUT>()?;
        let mut out = Vec::with_capacity(TREE_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.tree_meta);
        push_u8(&mut out, record.reserved0);
        push_u32(&mut out, record.pack_entry_ordinal);
        push_u32(&mut out, record.entry_count);
        out.extend_from_slice(&record.tree_hash80);
        require_encoded_len(&out, TREE_RECORD_SIZE_USIZE, "BinaryTreeRecord")?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<BinaryTreeRecord> {
        require_supported_content_layout::<LAYOUT>()?;
        require_raw_len(raw, TREE_RECORD_SIZE_USIZE, "BinaryTreeRecord")?;
        let mut cursor = Cursor::new(raw);
        Ok(BinaryTreeRecord {
            tree_meta: cursor.take_u8("tree_meta")?,
            reserved0: cursor.take_u8("reserved0")?,
            pack_entry_ordinal: cursor.take_u32("pack_entry_ordinal")?,
            entry_count: cursor.take_u32("entry_count")?,
            tree_hash80: cursor.take_array_10("tree_hash80")?,
        })
    }
}

fn push_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn require_encoded_len(out: &[u8], expected: usize, label: &str) -> StoreResult<()> {
    if out.len() != expected {
        return Err(format!(
            "{label} encoded to {} bytes, expected {expected}",
            out.len()
        )
        .into());
    }
    Ok(())
}

fn require_raw_len(raw: &[u8], expected: usize, label: &str) -> StoreResult<()> {
    if raw.len() != expected {
        return Err(format!("{label} is {} bytes, expected {expected}", raw.len()).into());
    }
    Ok(())
}

struct Cursor<'a> {
    raw: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(raw: &'a [u8]) -> Self {
        Self { raw, offset: 0 }
    }

    fn take_u8(&mut self, label: &str) -> StoreResult<u8> {
        let bytes = self.take(1, label)?;
        Ok(bytes[0])
    }

    fn take_u16(&mut self, label: &str) -> StoreResult<u16> {
        let bytes = self.take(2, label)?;
        let mut buf = [0_u8; 2];
        buf.copy_from_slice(bytes);
        Ok(u16::from_le_bytes(buf))
    }

    fn take_u32(&mut self, label: &str) -> StoreResult<u32> {
        let bytes = self.take(4, label)?;
        let mut buf = [0_u8; 4];
        buf.copy_from_slice(bytes);
        Ok(u32::from_le_bytes(buf))
    }

    fn take_u64(&mut self, label: &str) -> StoreResult<u64> {
        let bytes = self.take(8, label)?;
        let mut buf = [0_u8; 8];
        buf.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(buf))
    }

    fn take_array_10(&mut self, label: &str) -> StoreResult<[u8; 10]> {
        let bytes = self.take(10, label)?;
        let mut buf = [0_u8; 10];
        buf.copy_from_slice(bytes);
        Ok(buf)
    }

    fn take_array_32(&mut self, label: &str) -> StoreResult<[u8; 32]> {
        let bytes = self.take(32, label)?;
        let mut buf = [0_u8; 32];
        buf.copy_from_slice(bytes);
        Ok(buf)
    }

    fn take(&mut self, len: usize, label: &str) -> StoreResult<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| format!("{label} range overflow"))?;
        if end > self.raw.len() {
            return Err(format!("{label} is truncated").into());
        }
        let bytes = &self.raw[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}
