//! The already-declared layout-1 local Tag authority. Names remain exact;
//! tombstoning and re-creation preserve physical Tag ordinals.
use super::*;
use crate::binary_db::{
    BinaryDbCommandScope, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId, BinaryIndexId,
    BinaryPayloadFileId,
};
use crate::content_binary_db::LocalContentBinaryDb;
use crate::line_binary_db::line_name_hash64;
use chrono::{DateTime, SecondsFormat, Utc};

pub struct BinaryDbTagStore {
    content: LocalContentBinaryDb<1>,
}
fn file() -> BinaryFileId {
    BinaryFileId::new("tag.bin", 1, 24)
}
fn payload() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new("tag_payload.bin", 1)
}
fn index() -> BinaryIndexId {
    BinaryIndexId::new_fixed("tag_name.idx", 1, 8, false)
}
fn decode(bytes: &[u8], data: &[u8]) -> TagStoreResult<(bool, String, String, u32, u64)> {
    if bytes.len() != 24
        || bytes[0] & !128 != 0
        || bytes[1] != 0
        || data.len() < 2
        || data.len() != usize::from(u16::from_le_bytes(bytes[2..4].try_into().unwrap()))
    {
        return Err("invalid native Tag record/payload".into());
    }
    let name_len = usize::from(u16::from_le_bytes(data[..2].try_into().unwrap()));
    if name_len == 0 || name_len + 2 > data.len() {
        return Err("invalid native Tag name length".into());
    }
    let name = std::str::from_utf8(&data[2..2 + name_len])
        .map_err(|_| "native Tag name must be UTF-8")?
        .to_string();
    let message = std::str::from_utf8(&data[2 + name_len..])
        .map_err(|_| "native Tag message must be UTF-8")?
        .to_string();
    if name.trim() != name || message.contains(['\r', '\n']) {
        return Err("noncanonical native Tag text".into());
    }
    Ok((
        bytes[0] & 128 != 0,
        name,
        message,
        u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
    ))
}
impl BinaryDbTagStore {
    pub fn new(content: LocalContentBinaryDb<1>) -> Self {
        Self { content }
    }
    /// Validate a historical physical FK, including retained tombstone slots.
    pub fn validate_tag_ordinal(&self, ordinal: u32) -> TagStoreResult<()> {
        self.row(ordinal).map(|_| ())
    }
    fn row(&self, ordinal: u32) -> TagStoreResult<(bool, TagRecord)> {
        let read = BinaryDbReadTxn::new(self.content.db());
        self.row_with_read(&read, ordinal)
    }
    fn row_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, crate::binary_db::LocalBinaryDbFs>,
        ordinal: u32,
    ) -> TagStoreResult<(bool, TagRecord)> {
        let bytes = read.read_record(file(), ordinal)?;
        let data = read.read_payload(
            payload(),
            u64::from_le_bytes(bytes[4..12].try_into().unwrap()),
            u16::from_le_bytes(bytes[2..4].try_into().unwrap()).into(),
        )?;
        let (dead, name, message, snapshot_index, seconds) = decode(&bytes, &data)?;
        let view = self
            .content
            .snapshots()
            .snapshot_view_at(read, snapshot_index)?;
        let created = DateTime::<Utc>::from_timestamp(
            i64::try_from(seconds).map_err(|_| "Tag timestamp overflow")?,
            0,
        )
        .ok_or("Tag timestamp out of range")?;
        Ok((
            dead,
            TagRecord {
                schema: 1,
                name,
                snapshot_id: view.snapshot_id,
                message,
                created_at: created.to_rfc3339_opts(SecondsFormat::Secs, true),
            },
        ))
    }
    pub fn tag_index_by_name(&self, name: &str) -> TagStoreResult<Option<u32>> {
        let name = require_non_empty(name, "tag name")?;
        Ok(self.tag_ordinals_by_name()?.get(&name).copied())
    }
    /// One frozen physical-name inventory, including retained tombstones.
    pub fn tag_ordinals_by_name(&self) -> TagStoreResult<std::collections::BTreeMap<String, u32>> {
        let read = BinaryDbReadTxn::new(self.content.db());
        let mut found = std::collections::BTreeMap::new();
        for ordinal in 0..read.record_count(file())? {
            let (_, row) = self.row_with_read(&read, ordinal)?;
            if found.insert(row.name, ordinal).is_some() {
                return Err("duplicate native Tag name".into());
            }
        }
        Ok(found)
    }
    pub fn ensure_imported_tag(&self, record: &TagRecord) -> TagStoreResult<u32> {
        if let Some(existing) = self.tag_by_name(&record.name)? {
            if existing.snapshot_id != record.snapshot_id {
                return Err(format!(
                    "Git import refuses to replace AIT tag {}: immutable binding differs",
                    record.name
                ));
            }
        } else {
            self.create_tag(record)?;
        }
        self.tag_index_by_name(&record.name)?
            .ok_or_else(|| "created native Tag index absent".into())
    }
}
impl TagStore for BinaryDbTagStore {
    fn create_tag(&self, record: &TagRecord) -> TagStoreResult<TagRecord> {
        let record = validate_record(record)?;
        let seconds = DateTime::parse_from_rfc3339(&record.created_at)
            .map_err(|_| "invalid Tag creation time")?
            .timestamp();
        let seconds = u64::try_from(seconds).map_err(|_| "negative Tag creation time")?;
        let read = BinaryDbReadTxn::new(self.content.db());
        let snapshot = self
            .content
            .snapshots()
            .get_snapshot_view(&read, &record.snapshot_id)?
            .ok_or("Tag Snapshot absent")?;
        drop(read);
        let name_len = u16::try_from(record.name.len()).map_err(|_| "Tag name too large")?;
        let mut data = name_len.to_le_bytes().to_vec();
        data.extend_from_slice(record.name.as_bytes());
        data.extend_from_slice(record.message.as_bytes());
        let data_len = u16::try_from(data.len()).map_err(|_| "Tag payload too large")?;
        let mut write = BinaryDbWriteTxn::begin(self.content.db(), BinaryDbCommandScope::General)?;
        let key = line_name_hash64(record.name.as_bytes()).to_le_bytes();
        let mut existing = None;
        for ordinal in 0..write.record_count(file())? {
            let bytes = write.read_record(file(), ordinal)?;
            let old_data = write.read_payload(
                payload(),
                u64::from_le_bytes(bytes[4..12].try_into().unwrap()),
                u16::from_le_bytes(bytes[2..4].try_into().unwrap()).into(),
            )?;
            let (dead, name, _, _, _) = decode(&bytes, &old_data)?;
            if name == record.name {
                if !dead {
                    return Err(format!("Tag {} already exists.", record.name));
                }
                if existing.replace(ordinal).is_some() {
                    return Err("duplicate native Tag name".into());
                }
            }
        }
        let range = write.append_payload(payload(), &data)?;
        let mut bytes = [0; 24];
        bytes[2..4].copy_from_slice(&data_len.to_le_bytes());
        bytes[4..12].copy_from_slice(&range.payload_offset.to_le_bytes());
        bytes[12..16].copy_from_slice(&snapshot.snapshot_index.to_le_bytes());
        bytes[16..24].copy_from_slice(&seconds.to_le_bytes());
        let count = write.record_count(file())?;
        let mut root = 1_u32.to_le_bytes().to_vec();
        for ordinal in 0..count {
            root.extend_from_slice(&write.read_record(file(), ordinal)?);
        }
        if let Some(ordinal) = existing {
            let at = 4 + ordinal as usize * 24;
            root[at..at + 24].copy_from_slice(&bytes);
        } else {
            root.extend_from_slice(&bytes);
            write.append_index_candidate(index(), &key, count)?;
        }
        write.commit_with_atomic_file_replacement(file(), &root, "native Tag owner publication")?;
        self.tag_by_name(&record.name)?
            .ok_or_else(|| "created native Tag absent".into())
    }
    fn list_tags(&self) -> TagStoreResult<Vec<TagRecord>> {
        let read = BinaryDbReadTxn::new(self.content.db());
        let mut records = Vec::new();
        for ordinal in 0..read.record_count(file())? {
            let (dead, row) = self.row_with_read(&read, ordinal)?;
            if !dead {
                records.push(row);
            }
        }
        records.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(records)
    }
    fn tag_by_name(&self, name: &str) -> TagStoreResult<Option<TagRecord>> {
        self.tag_index_by_name(name)?
            .map(|ordinal| self.row(ordinal))
            .transpose()
            .map(|row| row.and_then(|(dead, r)| (!dead).then_some(r)))
    }
    fn delete_tag(&self, name: &str) -> TagStoreResult<Option<TagRecord>> {
        let Some(ordinal) = self.tag_index_by_name(name)? else {
            return Ok(None);
        };
        let (dead, record) = self.row(ordinal)?;
        if dead {
            return Ok(None);
        };
        let mut write = BinaryDbWriteTxn::begin(self.content.db(), BinaryDbCommandScope::General)?;
        let mut root = 1_u32.to_le_bytes().to_vec();
        for i in 0..write.record_count(file())? {
            root.extend_from_slice(&write.read_record(file(), i)?);
        }
        let at = 4 + ordinal as usize * 24;
        if root[at] & 128 != 0 {
            return Ok(None);
        }
        root[at] |= 128;
        write.commit_with_atomic_file_replacement(
            file(),
            &root,
            "native Tag tombstone publication",
        )?;
        Ok(Some(record))
    }
}
