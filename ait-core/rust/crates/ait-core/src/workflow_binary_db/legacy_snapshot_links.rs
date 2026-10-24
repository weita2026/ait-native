//! Layout-1 decoding is confined to validated conversion admission.
use super::*;
#[derive(Clone, Debug)]
pub(crate) struct LegacySnapshotLink {
    pub(crate) meta: u8,
    pub(crate) ordinal: u8,
    pub(crate) payload_len: u16,
    pub(crate) payload_offset: u64,
    pub(crate) task: u32,
    pub(crate) change_plus1: u32,
    pub(crate) snapshot: u32,
    pub(crate) previous_task: u32,
    pub(crate) previous_change: u32,
    pub(crate) created_at_s: u64,
}

impl LegacySnapshotLink {
    pub(crate) fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(raw, 40, "Legacy Snapshot Link")?;
        if raw[0] & 0x40 != 0 {
            return Err("Snapshot LegacySnapshotLink has reserved metadata".into());
        }
        let u32_at = |offset| u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
        let value = Self {
            meta: raw[0],
            ordinal: raw[1],
            payload_len: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
            payload_offset: u64::from_le_bytes(raw[4..12].try_into().unwrap()),
            task: u32_at(12),
            change_plus1: u32_at(16),
            snapshot: u32_at(20),
            previous_task: u32_at(24),
            previous_change: u32_at(28),
            created_at_s: u64::from_le_bytes(raw[32..40].try_into().unwrap()),
        };
        if (value.meta & 1 != 0) != (value.change_plus1 != 0) {
            return Err(
                "Snapshot LegacySnapshotLink Change flag disagrees with its pointer".into(),
            );
        }
        Ok(value)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LegacySnapshotLinkPayload {
    pub(crate) worktree_name: String,
    pub(crate) line_name: String,
    pub(crate) task_id: String,
    pub(crate) change_id: String,
    pub(crate) author_mode: String,
    pub(crate) model_name: String,
}
impl LegacySnapshotLinkPayload {
    pub(crate) fn decode(raw: &[u8]) -> StoreResult<Self> {
        if raw.len() < 10 {
            return Err("Snapshot Link payload is truncated".into());
        }
        let mut offset = 10;
        let mut fields = Vec::new();
        for index in 0..5 {
            let size = usize::from(u16::from_le_bytes(
                raw[index * 2..index * 2 + 2].try_into().unwrap(),
            ));
            let end = offset + size;
            let bytes = raw
                .get(offset..end)
                .ok_or("Snapshot Link payload field is truncated")?;
            fields.push(
                std::str::from_utf8(bytes)
                    .map_err(|_| "Snapshot Link payload is not UTF-8")?
                    .to_string(),
            );
            offset = end;
        }
        fields.push(
            std::str::from_utf8(&raw[offset..])
                .map_err(|_| "Snapshot Link model is not UTF-8")?
                .to_string(),
        );
        Ok(Self {
            worktree_name: fields[0].clone(),
            line_name: fields[1].clone(),
            task_id: fields[2].clone(),
            change_id: fields[3].clone(),
            author_mode: fields[4].clone(),
            model_name: fields[5].clone(),
        })
    }
}
