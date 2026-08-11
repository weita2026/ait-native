use super::schema::{
    PlanItemPayload, PlanItemRecord, PlanRecord, PlanRevisionPayload, PlanRevisionRecord,
    PLAN_ITEM_RECORD_SIZE, PLAN_LAYOUT_ID, PLAN_RECORD_SIZE, PLAN_REVISION_RECORD_SIZE,
};

pub(super) struct ServerPlanCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> ServerPlanCodec<LAYOUT> {
    pub(super) fn encode_record(record: &PlanRecord) -> Result<Vec<u8>, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        encode_plan_record(record)
    }

    pub(super) fn decode_record(raw: &[u8]) -> Result<PlanRecord, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        decode_plan_record(raw)
    }

    pub(super) fn decode_title_payload(raw: Vec<u8>) -> Result<String, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        decode_utf8(raw, "plan title")
    }
}

pub(super) struct ServerPlanRevisionCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> ServerPlanRevisionCodec<LAYOUT> {
    pub(super) fn encode_record(record: &PlanRevisionRecord) -> Result<Vec<u8>, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        encode_plan_revision_record(record)
    }

    pub(super) fn decode_record(raw: &[u8]) -> Result<PlanRevisionRecord, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        decode_plan_revision_record(raw)
    }

    pub(super) fn encode_payload(payload: &PlanRevisionPayload) -> Result<Vec<u8>, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        encode_revision_payload(payload)
    }

    pub(super) fn decode_payload(raw: &[u8]) -> Result<PlanRevisionPayload, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        decode_revision_payload(raw)
    }
}

pub(super) struct ServerPlanItemCodec<const LAYOUT: u32>;

#[allow(dead_code)]
impl<const LAYOUT: u32> ServerPlanItemCodec<LAYOUT> {
    pub(super) fn encode_record(record: &PlanItemRecord) -> Result<Vec<u8>, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        encode_plan_item_record(record)
    }

    pub(super) fn decode_record(raw: &[u8]) -> Result<PlanItemRecord, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        decode_plan_item_record(raw)
    }

    pub(super) fn encode_payload(payload: &PlanItemPayload) -> Result<Vec<u8>, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        encode_item_payload(payload)
    }

    pub(super) fn decode_payload(raw: &[u8]) -> Result<PlanItemPayload, String> {
        ensure_supported_codec_layout(LAYOUT)?;
        decode_item_payload(raw)
    }
}

pub(super) fn decode_plan_record_for_layout(layout: u32, raw: &[u8]) -> Result<PlanRecord, String> {
    match layout {
        PLAN_LAYOUT_ID => ServerPlanCodec::<PLAN_LAYOUT_ID>::decode_record(raw),
        _ => unsupported_codec_layout(layout),
    }
}

pub(super) fn decode_plan_revision_record_for_layout(
    layout: u32,
    raw: &[u8],
) -> Result<PlanRevisionRecord, String> {
    match layout {
        PLAN_LAYOUT_ID => ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_record(raw),
        _ => unsupported_codec_layout(layout),
    }
}

pub(super) fn decode_plan_item_record_for_layout(
    layout: u32,
    raw: &[u8],
) -> Result<PlanItemRecord, String> {
    match layout {
        PLAN_LAYOUT_ID => ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_record(raw),
        _ => unsupported_codec_layout(layout),
    }
}

pub(super) fn encode_plan_record(record: &PlanRecord) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(PLAN_RECORD_SIZE as usize);
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
    require_encoded_len(&out, PLAN_RECORD_SIZE as usize, "PlanRecord")?;
    Ok(out)
}

pub(super) fn decode_plan_record(raw: &[u8]) -> Result<PlanRecord, String> {
    require_raw_len(raw, PLAN_RECORD_SIZE as usize, "PlanRecord")?;
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

pub(super) fn encode_plan_revision_record(record: &PlanRevisionRecord) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(PLAN_REVISION_RECORD_SIZE as usize);
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
    require_encoded_len(
        &out,
        PLAN_REVISION_RECORD_SIZE as usize,
        "PlanRevisionRecord",
    )?;
    Ok(out)
}

pub(super) fn decode_plan_revision_record(raw: &[u8]) -> Result<PlanRevisionRecord, String> {
    require_raw_len(
        raw,
        PLAN_REVISION_RECORD_SIZE as usize,
        "PlanRevisionRecord",
    )?;
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

pub(super) fn encode_plan_item_record(record: &PlanItemRecord) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(PLAN_ITEM_RECORD_SIZE as usize);
    push_u8(&mut out, record.item_meta);
    push_u8(&mut out, record.reserved0);
    push_u16(&mut out, record.payload_len);
    push_u64(&mut out, record.payload_offset);
    push_u32(&mut out, record.line_number);
    require_encoded_len(&out, PLAN_ITEM_RECORD_SIZE as usize, "PlanItemRecord")?;
    Ok(out)
}

#[allow(dead_code)]
pub(super) fn decode_plan_item_record(raw: &[u8]) -> Result<PlanItemRecord, String> {
    require_raw_len(raw, PLAN_ITEM_RECORD_SIZE as usize, "PlanItemRecord")?;
    let mut cursor = Cursor::new(raw);
    Ok(PlanItemRecord {
        item_meta: cursor.take_u8("item_meta")?,
        reserved0: cursor.take_u8("reserved0")?,
        payload_len: cursor.take_u16("payload_len")?,
        payload_offset: cursor.take_u64("payload_offset")?,
        line_number: cursor.take_u32("line_number")?,
    })
}

pub(super) fn encode_revision_payload(payload: &PlanRevisionPayload) -> Result<Vec<u8>, String> {
    let title_len = u16_len(payload.title_snapshot.len(), "title_snapshot")?;
    let summary_len = u16_len(payload.summary.len(), "summary")?;
    let artifact_path_len = u16_len(payload.artifact_path.len(), "artifact_path")?;
    let artifact_selector_len = u16_len(payload.artifact_selector.len(), "artifact_selector")?;
    let artifact_heading_len = u16_len(payload.artifact_heading.len(), "artifact_heading")?;
    let _ = u16_len(payload.artifact_blob_id.len(), "artifact_blob_id")?;
    let mut out = Vec::new();
    push_u16(&mut out, title_len);
    push_u16(&mut out, summary_len);
    push_u16(&mut out, artifact_path_len);
    push_u16(&mut out, artifact_selector_len);
    push_u16(&mut out, artifact_heading_len);
    out.extend_from_slice(payload.title_snapshot.as_bytes());
    out.extend_from_slice(payload.summary.as_bytes());
    out.extend_from_slice(payload.artifact_path.as_bytes());
    out.extend_from_slice(payload.artifact_selector.as_bytes());
    out.extend_from_slice(payload.artifact_heading.as_bytes());
    out.extend_from_slice(payload.artifact_blob_id.as_bytes());
    let _ = u16_len(out.len(), "plan revision payload")?;
    Ok(out)
}

pub(super) fn decode_revision_payload(raw: &[u8]) -> Result<PlanRevisionPayload, String> {
    if raw.len() < 10 {
        return Err("PlanRevisionPayload bytes are truncated".to_string());
    }
    let mut cursor = Cursor::new(raw);
    let title_len = cursor.take_u16("title_snapshot_len")? as usize;
    let summary_len = cursor.take_u16("summary_len")? as usize;
    let artifact_path_len = cursor.take_u16("artifact_path_len")? as usize;
    let artifact_selector_len = cursor.take_u16("artifact_selector_len")? as usize;
    let artifact_heading_len = cursor.take_u16("artifact_heading_len")? as usize;
    let title_snapshot = decode_utf8(
        cursor.take_bytes(title_len, "title_snapshot_bytes")?,
        "title_snapshot",
    )?;
    let summary = decode_utf8(cursor.take_bytes(summary_len, "summary_bytes")?, "summary")?;
    let artifact_path = decode_utf8(
        cursor.take_bytes(artifact_path_len, "artifact_path_bytes")?,
        "artifact_path",
    )?;
    let artifact_selector = decode_utf8(
        cursor.take_bytes(artifact_selector_len, "artifact_selector_bytes")?,
        "artifact_selector",
    )?;
    let artifact_heading = decode_utf8(
        cursor.take_bytes(artifact_heading_len, "artifact_heading_bytes")?,
        "artifact_heading",
    )?;
    let artifact_blob_id = decode_utf8(cursor.take_remaining_bytes(), "artifact_blob_id")?;
    Ok(PlanRevisionPayload {
        title_snapshot,
        summary,
        artifact_path,
        artifact_selector,
        artifact_heading,
        artifact_blob_id,
    })
}

pub(super) fn encode_item_payload(payload: &PlanItemPayload) -> Result<Vec<u8>, String> {
    let ref_len = u16_len(payload.plan_item_ref.len(), "plan_item_ref")?;
    let text_len = u16_len(payload.text.len(), "plan item text")?;
    let heading_count = u16_len(payload.heading_path.len(), "heading_path")?;
    let mut out = Vec::new();
    push_u16(&mut out, ref_len);
    push_u16(&mut out, text_len);
    push_u16(&mut out, heading_count);
    out.extend_from_slice(payload.plan_item_ref.as_bytes());
    out.extend_from_slice(payload.text.as_bytes());
    for part in &payload.heading_path {
        push_u16(&mut out, u16_len(part.len(), "heading_path entry")?);
        out.extend_from_slice(part.as_bytes());
    }
    let _ = u16_len(out.len(), "plan item payload")?;
    Ok(out)
}

#[allow(dead_code)]
pub(super) fn decode_item_payload(raw: &[u8]) -> Result<PlanItemPayload, String> {
    if raw.len() < 6 {
        return Err("PlanItemPayload bytes are truncated".to_string());
    }
    let mut cursor = Cursor::new(raw);
    let ref_len = cursor.take_u16("plan_item_ref_len")? as usize;
    let text_len = cursor.take_u16("text_len")? as usize;
    let heading_count = cursor.take_u16("heading_path_count")? as usize;
    let plan_item_ref = decode_utf8(
        cursor.take_bytes(ref_len, "plan_item_ref_bytes")?,
        "plan_item_ref",
    )?;
    let text = decode_utf8(cursor.take_bytes(text_len, "text_bytes")?, "plan item text")?;
    let mut heading_path = Vec::with_capacity(heading_count);
    for _ in 0..heading_count {
        let len = cursor.take_u16("heading_path_entry_len")? as usize;
        heading_path.push(decode_utf8(
            cursor.take_bytes(len, "heading_path_entry")?,
            "heading_path entry",
        )?);
    }
    let trailing = cursor.take_remaining_bytes();
    if !trailing.is_empty() {
        return Err(format!(
            "PlanItemPayload contains {} trailing bytes",
            trailing.len()
        ));
    }
    Ok(PlanItemPayload {
        plan_item_ref,
        text,
        heading_path,
    })
}

pub(super) fn decode_utf8(bytes: Vec<u8>, label: &str) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|err| format!("{label} is not valid UTF-8: {err}"))
}

fn ensure_supported_codec_layout(layout: u32) -> Result<(), String> {
    if layout == PLAN_LAYOUT_ID {
        return Ok(());
    }
    unsupported_codec_layout(layout)
}

fn unsupported_codec_layout<T>(layout: u32) -> Result<T, String> {
    Err(format!(
        "unsupported server Plan Binary DB codec layout {layout}; supported layout is {PLAN_LAYOUT_ID}"
    ))
}

fn u16_len(len: usize, field: &str) -> Result<u16, String> {
    u16::try_from(len).map_err(|_| format!("{field} length exceeds u16::MAX: {len}"))
}

fn require_raw_len(raw: &[u8], expected: usize, name: &str) -> Result<(), String> {
    if raw.len() != expected {
        return Err(format!(
            "{name} has invalid fixed size: expected {expected}, got {}",
            raw.len()
        ));
    }
    Ok(())
}

fn require_encoded_len(raw: &[u8], expected: usize, name: &str) -> Result<(), String> {
    if raw.len() != expected {
        return Err(format!(
            "{name} encoding produced wrong length: expected {expected}, got {}",
            raw.len()
        ));
    }
    Ok(())
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

struct Cursor<'a> {
    raw: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(raw: &'a [u8]) -> Self {
        Self { raw, offset: 0 }
    }

    fn take_u8(&mut self, field: &str) -> Result<u8, String> {
        let bytes = self.take_array::<1>(field)?;
        Ok(bytes[0])
    }

    fn take_u16(&mut self, field: &str) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take_array::<2>(field)?))
    }

    fn take_u32(&mut self, field: &str) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take_array::<4>(field)?))
    }

    fn take_u64(&mut self, field: &str) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take_array::<8>(field)?))
    }

    fn take_bytes(&mut self, len: usize, field: &str) -> Result<Vec<u8>, String> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| format!("{field} cursor overflow"))?;
        if end > self.raw.len() {
            return Err(format!("{field} bytes are truncated"));
        }
        let out = self.raw[self.offset..end].to_vec();
        self.offset = end;
        Ok(out)
    }

    fn take_remaining_bytes(&mut self) -> Vec<u8> {
        let out = self.raw[self.offset..].to_vec();
        self.offset = self.raw.len();
        out
    }

    fn take_array<const N: usize>(&mut self, field: &str) -> Result<[u8; N], String> {
        let bytes = self.take_bytes(N, field)?;
        bytes
            .try_into()
            .map_err(|_| format!("{field} byte length mismatch"))
    }
}
