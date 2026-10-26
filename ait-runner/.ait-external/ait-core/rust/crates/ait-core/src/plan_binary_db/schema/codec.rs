use crate::binary_db::{BinaryFileId, BinaryPayloadFileId, StoreResult};

use super::files::{
    PLAN_BIN, PLAN_ITEM_BIN, PLAN_ITEM_PAYLOAD_BIN, PLAN_ITEM_RECORD_SIZE,
    PLAN_ITEM_RECORD_SIZE_USIZE, PLAN_LAYOUT_ID, PLAN_PAYLOAD_BIN, PLAN_RECORD_SIZE,
    PLAN_RECORD_SIZE_USIZE, PLAN_REVISION_BIN, PLAN_REVISION_PAYLOAD_BIN,
    PLAN_REVISION_RECORD_SIZE, PLAN_REVISION_RECORD_SIZE_USIZE,
};
use super::payloads::{PlanItemPayload, PlanPayload, PlanRevisionPayload};
use super::records::{PlanItemRecord, PlanRecord, PlanRevisionRecord};

pub struct PlanCodec<const LAYOUT: u32>;
pub struct PlanRevisionCodec<const LAYOUT: u32>;
pub struct PlanItemCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> PlanCodec<LAYOUT> {
    pub const LAYOUT_ID: u32 = LAYOUT;
    pub const RECORD_SIZE: u32 = PLAN_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(PLAN_BIN, LAYOUT, PLAN_RECORD_SIZE)
    }

    pub fn payload_file() -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(PLAN_PAYLOAD_BIN, LAYOUT)
    }

    pub fn encode_record(record: &PlanRecord) -> StoreResult<Vec<u8>> {
        ensure_supported_codec_layout(LAYOUT)?;
        let mut out = Vec::with_capacity(PLAN_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.plan_meta);
        push_u8(&mut out, record.reserved0);
        push_u16(&mut out, record.payload_len);
        push_u64(&mut out, record.payload_offset);
        push_u32(&mut out, record.latest_revision_index_plus1);
        push_u32(&mut out, record.published_plan_index_plus1);
        push_u32(&mut out, record.published_latest_revision_index_plus1);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.updated_at_s);
        push_u64(&mut out, record.published_at_s);
        require_encoded_len(&out, PLAN_RECORD_SIZE_USIZE, "PlanRecord")?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<PlanRecord> {
        ensure_supported_codec_layout(LAYOUT)?;
        require_raw_len(raw, PLAN_RECORD_SIZE_USIZE, "PlanRecord")?;
        let mut cursor = Cursor::new(raw);
        Ok(PlanRecord {
            plan_meta: cursor.take_u8("plan_meta")?,
            reserved0: cursor.take_u8("reserved0")?,
            payload_len: cursor.take_u16("payload_len")?,
            payload_offset: cursor.take_u64("payload_offset")?,
            latest_revision_index_plus1: cursor.take_u32("latest_revision_index_plus1")?,
            published_plan_index_plus1: cursor.take_u32("published_plan_index_plus1")?,
            published_latest_revision_index_plus1: cursor
                .take_u32("published_latest_revision_index_plus1")?,
            created_at_s: cursor.take_u64("created_at_s")?,
            updated_at_s: cursor.take_u64("updated_at_s")?,
            published_at_s: cursor.take_u64("published_at_s")?,
        })
    }

    pub fn encode_payload(payload: &PlanPayload) -> StoreResult<Vec<u8>> {
        ensure_supported_codec_layout(LAYOUT)?;
        require_u16_len(payload.title_bytes.len(), "plan title")?;
        Ok(payload.title_bytes.clone())
    }

    pub fn decode_payload(raw: &[u8]) -> StoreResult<PlanPayload> {
        ensure_supported_codec_layout(LAYOUT)?;
        Ok(PlanPayload {
            title_bytes: raw.to_vec(),
        })
    }
}

impl<const LAYOUT: u32> PlanRevisionCodec<LAYOUT> {
    pub const LAYOUT_ID: u32 = LAYOUT;
    pub const RECORD_SIZE: u32 = PLAN_REVISION_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(PLAN_REVISION_BIN, LAYOUT, PLAN_REVISION_RECORD_SIZE)
    }

    pub fn payload_file() -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(PLAN_REVISION_PAYLOAD_BIN, LAYOUT)
    }

    pub fn encode_record(record: &PlanRevisionRecord) -> StoreResult<Vec<u8>> {
        ensure_supported_codec_layout(LAYOUT)?;
        let mut out = Vec::with_capacity(PLAN_REVISION_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.revision_meta);
        push_u8(&mut out, record.reserved0);
        push_u16(&mut out, record.payload_len);
        push_u16(&mut out, record.revision_number);
        push_u16(&mut out, record.item_count);
        push_u64(&mut out, record.payload_offset);
        push_u32(&mut out, record.plan_index);
        push_u32(&mut out, record.previous_revision_index_plus1);
        push_u32(&mut out, record.item_start_index);
        push_u32(&mut out, record.published_revision_index_plus1);
        push_u32(&mut out, record.root_tree_pack_index_plus1);
        push_u32(&mut out, record.root_entry_ordinal);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.published_at_s);
        require_encoded_len(&out, PLAN_REVISION_RECORD_SIZE_USIZE, "PlanRevisionRecord")?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<PlanRevisionRecord> {
        ensure_supported_codec_layout(LAYOUT)?;
        require_raw_len(raw, PLAN_REVISION_RECORD_SIZE_USIZE, "PlanRevisionRecord")?;
        let mut cursor = Cursor::new(raw);
        Ok(PlanRevisionRecord {
            revision_meta: cursor.take_u8("revision_meta")?,
            reserved0: cursor.take_u8("reserved0")?,
            payload_len: cursor.take_u16("payload_len")?,
            revision_number: cursor.take_u16("revision_number")?,
            item_count: cursor.take_u16("item_count")?,
            payload_offset: cursor.take_u64("payload_offset")?,
            plan_index: cursor.take_u32("plan_index")?,
            previous_revision_index_plus1: cursor.take_u32("previous_revision_index_plus1")?,
            item_start_index: cursor.take_u32("item_start_index")?,
            published_revision_index_plus1: cursor.take_u32("published_revision_index_plus1")?,
            root_tree_pack_index_plus1: cursor.take_u32("root_tree_pack_index_plus1")?,
            root_entry_ordinal: cursor.take_u32("root_entry_ordinal")?,
            created_at_s: cursor.take_u64("created_at_s")?,
            published_at_s: cursor.take_u64("published_at_s")?,
        })
    }

    pub fn encode_payload(payload: &PlanRevisionPayload) -> StoreResult<Vec<u8>> {
        ensure_supported_codec_layout(LAYOUT)?;
        let title_len = require_u16_len(payload.title_snapshot_bytes.len(), "title_snapshot")?;
        let summary_len = require_u16_len(payload.summary_bytes.len(), "summary")?;
        let artifact_path_len =
            require_u16_len(payload.artifact_path_bytes.len(), "artifact_path")?;
        let artifact_selector_len =
            require_u16_len(payload.artifact_selector_bytes.len(), "artifact_selector")?;
        let artifact_heading_len =
            require_u16_len(payload.artifact_heading_bytes.len(), "artifact_heading")?;
        let _artifact_blob_id_len =
            require_u16_len(payload.artifact_blob_id_bytes.len(), "artifact_blob_id")?;

        let mut out = Vec::new();
        push_u16(&mut out, title_len);
        push_u16(&mut out, summary_len);
        push_u16(&mut out, artifact_path_len);
        push_u16(&mut out, artifact_selector_len);
        push_u16(&mut out, artifact_heading_len);
        out.extend_from_slice(&payload.title_snapshot_bytes);
        out.extend_from_slice(&payload.summary_bytes);
        out.extend_from_slice(&payload.artifact_path_bytes);
        out.extend_from_slice(&payload.artifact_selector_bytes);
        out.extend_from_slice(&payload.artifact_heading_bytes);
        out.extend_from_slice(&payload.artifact_blob_id_bytes);
        require_u16_len(out.len(), "plan revision payload")?;
        Ok(out)
    }

    pub fn decode_payload(raw: &[u8]) -> StoreResult<PlanRevisionPayload> {
        ensure_supported_codec_layout(LAYOUT)?;
        if raw.len() < 10 {
            return Err("PlanRevisionPayload bytes are truncated".into());
        }
        let mut cursor = Cursor::new(raw);
        let title_len = usize::from(cursor.take_u16("title_snapshot_len")?);
        let summary_len = usize::from(cursor.take_u16("summary_len")?);
        let artifact_path_len = usize::from(cursor.take_u16("artifact_path_len")?);
        let artifact_selector_len = usize::from(cursor.take_u16("artifact_selector_len")?);
        let artifact_heading_len = usize::from(cursor.take_u16("artifact_heading_len")?);
        let title_snapshot_bytes = cursor.take_bytes(title_len, "title_snapshot_bytes")?;
        let summary_bytes = cursor.take_bytes(summary_len, "summary_bytes")?;
        let artifact_path_bytes = cursor.take_bytes(artifact_path_len, "artifact_path_bytes")?;
        let artifact_selector_bytes =
            cursor.take_bytes(artifact_selector_len, "artifact_selector_bytes")?;
        let artifact_heading_bytes =
            cursor.take_bytes(artifact_heading_len, "artifact_heading_bytes")?;
        let artifact_blob_id_bytes = cursor.take_remaining_bytes();
        Ok(PlanRevisionPayload {
            title_snapshot_bytes,
            summary_bytes,
            artifact_path_bytes,
            artifact_selector_bytes,
            artifact_heading_bytes,
            artifact_blob_id_bytes,
        })
    }
}

impl<const LAYOUT: u32> PlanItemCodec<LAYOUT> {
    pub const LAYOUT_ID: u32 = LAYOUT;
    pub const RECORD_SIZE: u32 = PLAN_ITEM_RECORD_SIZE;

    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(PLAN_ITEM_BIN, LAYOUT, PLAN_ITEM_RECORD_SIZE)
    }

    pub fn payload_file() -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(PLAN_ITEM_PAYLOAD_BIN, LAYOUT)
    }

    pub fn encode_record(record: &PlanItemRecord) -> StoreResult<Vec<u8>> {
        ensure_supported_codec_layout(LAYOUT)?;
        let mut out = Vec::with_capacity(PLAN_ITEM_RECORD_SIZE_USIZE);
        push_u8(&mut out, record.item_meta);
        push_u8(&mut out, record.reserved0);
        push_u16(&mut out, record.payload_len);
        push_u64(&mut out, record.payload_offset);
        push_u32(&mut out, record.line_number);
        require_encoded_len(&out, PLAN_ITEM_RECORD_SIZE_USIZE, "PlanItemRecord")?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<PlanItemRecord> {
        ensure_supported_codec_layout(LAYOUT)?;
        require_raw_len(raw, PLAN_ITEM_RECORD_SIZE_USIZE, "PlanItemRecord")?;
        let mut cursor = Cursor::new(raw);
        Ok(PlanItemRecord {
            item_meta: cursor.take_u8("item_meta")?,
            reserved0: cursor.take_u8("reserved0")?,
            payload_len: cursor.take_u16("payload_len")?,
            payload_offset: cursor.take_u64("payload_offset")?,
            line_number: cursor.take_u32("line_number")?,
        })
    }

    pub fn encode_payload(payload: &PlanItemPayload) -> StoreResult<Vec<u8>> {
        ensure_supported_codec_layout(LAYOUT)?;
        let ref_len = require_u16_len(payload.plan_item_ref_bytes.len(), "plan_item_ref")?;
        let text_len = require_u16_len(payload.text_bytes.len(), "plan item text")?;
        let heading_count = require_u16_len(payload.heading_path.len(), "heading_path")?;
        let mut out = Vec::new();
        push_u16(&mut out, ref_len);
        push_u16(&mut out, text_len);
        push_u16(&mut out, heading_count);
        out.extend_from_slice(&payload.plan_item_ref_bytes);
        out.extend_from_slice(&payload.text_bytes);
        for part in &payload.heading_path {
            push_u16(&mut out, require_u16_len(part.len(), "heading_path entry")?);
            out.extend_from_slice(part.as_bytes());
        }
        require_u16_len(out.len(), "plan item payload")?;
        Ok(out)
    }

    pub fn decode_payload(raw: &[u8]) -> StoreResult<PlanItemPayload> {
        ensure_supported_codec_layout(LAYOUT)?;
        if raw.len() < 6 {
            return Err("PlanItemPayload bytes are truncated".into());
        }
        let mut cursor = Cursor::new(raw);
        let plan_item_ref_len = usize::from(cursor.take_u16("plan_item_ref_len")?);
        let text_len = usize::from(cursor.take_u16("text_len")?);
        let heading_count = usize::from(cursor.take_u16("heading_path_count")?);
        let plan_item_ref_bytes = cursor.take_bytes(plan_item_ref_len, "plan_item_ref_bytes")?;
        let text_bytes = cursor.take_bytes(text_len, "text_bytes")?;
        let mut heading_path = Vec::with_capacity(heading_count);
        for _ in 0..heading_count {
            let len = usize::from(cursor.take_u16("heading_path_entry_len")?);
            heading_path.push(decode_payload_utf8(
                cursor.take_bytes(len, "heading_path_entry")?,
                "heading_path entry",
            )?);
        }
        let trailing = cursor.take_remaining_bytes();
        if !trailing.is_empty() {
            return Err(
                format!("PlanItemPayload contains {} trailing bytes", trailing.len()).into(),
            );
        }
        Ok(PlanItemPayload {
            plan_item_ref_bytes,
            text_bytes,
            heading_path,
        })
    }
}

fn ensure_supported_codec_layout(layout: u32) -> StoreResult<()> {
    if layout == PLAN_LAYOUT_ID {
        return Ok(());
    }
    Err(format!(
        "unsupported Plan Binary DB codec layout {layout}; supported layout is {PLAN_LAYOUT_ID}"
    )
    .into())
}

fn require_raw_len(raw: &[u8], expected: usize, name: &str) -> StoreResult<()> {
    if raw.len() != expected {
        return Err(format!(
            "{name} has invalid fixed size: expected {expected}, got {}",
            raw.len()
        )
        .into());
    }
    Ok(())
}

fn require_encoded_len(raw: &[u8], expected: usize, name: &str) -> StoreResult<()> {
    if raw.len() != expected {
        return Err(format!(
            "{name} encoding produced wrong length: expected {expected}, got {}",
            raw.len()
        )
        .into());
    }
    Ok(())
}

fn require_u16_len(len: usize, field: &str) -> StoreResult<u16> {
    Ok(u16::try_from(len).map_err(|_| format!("{field} length exceeds u16::MAX: {len}"))?)
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

fn decode_payload_utf8(bytes: Vec<u8>, label: &str) -> StoreResult<String> {
    Ok(String::from_utf8(bytes).map_err(|err| format!("{label} is not valid UTF-8: {err}"))?)
}

struct Cursor<'a> {
    raw: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(raw: &'a [u8]) -> Self {
        Self { raw, offset: 0 }
    }

    fn take_u8(&mut self, field: &str) -> StoreResult<u8> {
        let bytes = self.take_array::<1>(field)?;
        Ok(bytes[0])
    }

    fn take_u16(&mut self, field: &str) -> StoreResult<u16> {
        Ok(u16::from_le_bytes(self.take_array::<2>(field)?))
    }

    fn take_u32(&mut self, field: &str) -> StoreResult<u32> {
        Ok(u32::from_le_bytes(self.take_array::<4>(field)?))
    }

    fn take_u64(&mut self, field: &str) -> StoreResult<u64> {
        Ok(u64::from_le_bytes(self.take_array::<8>(field)?))
    }

    fn take_array<const N: usize>(&mut self, field: &str) -> StoreResult<[u8; N]> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| format!("{field} range overflow"))?;
        if end > self.raw.len() {
            return Err(format!("{field} bytes truncated").into());
        }
        let mut bytes = [0_u8; N];
        bytes.copy_from_slice(&self.raw[self.offset..end]);
        self.offset = end;
        Ok(bytes)
    }

    fn take_bytes(&mut self, len: usize, field: &str) -> StoreResult<Vec<u8>> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| format!("{field} range overflow"))?;
        if end > self.raw.len() {
            return Err(format!("{field} bytes truncated").into());
        }
        let bytes = self.raw[self.offset..end].to_vec();
        self.offset = end;
        Ok(bytes)
    }

    fn take_remaining_bytes(&mut self) -> Vec<u8> {
        let bytes = self.raw[self.offset..].to_vec();
        self.offset = self.raw.len();
        bytes
    }
}
