//! The owner-approved per-file layout-2 Snapshot Link codecs.
use super::*;
pub const SNAPSHOT_LINK_LAYOUT_ID: u32 = 2;
pub const AUTHORITATIVE_SNAPSHOT_LINK_RECORD_SIZE: u32 = 22;
pub const REMOTE_MIRROR_SNAPSHOT_LINK_RECORD_SIZE: u32 = 34;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotLinkRecord {
    pub meta: u8,
    pub ordinal: u8,
    pub task: u32,
    pub change_plus1: u32,
    pub snapshot: u32,
    pub created_at_s: u64,
}
impl SnapshotLinkRecord {
    pub fn encode(&self) -> StoreResult<Vec<u8>> {
        if self.meta & 0x78 != 0 || (self.meta & 1 != 0) != (self.change_plus1 != 0) {
            return Err(
                "Snapshot Link has reserved metadata or an inconsistent Change presence flag"
                    .into(),
            );
        }
        let mut out = vec![self.meta, self.ordinal];
        for value in [self.task, self.change_plus1, self.snapshot] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&self.created_at_s.to_le_bytes());
        Ok(out)
    }
    pub fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(
            raw,
            AUTHORITATIVE_SNAPSHOT_LINK_RECORD_SIZE,
            "Snapshot Link",
        )?;
        let field = |at| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
        let row = Self {
            meta: raw[0],
            ordinal: raw[1],
            task: field(2),
            change_plus1: field(6),
            snapshot: field(10),
            created_at_s: u64::from_le_bytes(raw[14..22].try_into().unwrap()),
        };
        row.encode()?;
        Ok(row)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteMirrorSnapshotLinkRecord {
    pub link: SnapshotLinkRecord,
    pub remote_meta: u8,
    pub fetched_at_s: u64,
}
impl RemoteMirrorSnapshotLinkRecord {
    pub fn encode(&self) -> StoreResult<Vec<u8>> {
        if self.remote_meta & !0x0f != 0 {
            return Err("Snapshot Link mirror has reserved remote metadata".into());
        }
        let link = self.link.encode()?;
        let mut out = vec![link[0], self.remote_meta, 0, 0, link[1], 0];
        out.extend_from_slice(&link[2..]);
        out.extend_from_slice(&self.fetched_at_s.to_le_bytes());
        Ok(out)
    }
    pub fn decode(raw: &[u8]) -> StoreResult<Self> {
        require_raw_size(
            raw,
            REMOTE_MIRROR_SNAPSHOT_LINK_RECORD_SIZE,
            "Remote Mirror Snapshot Link",
        )?;
        if raw[2..4] != [0, 0] || raw[5] != 0 {
            return Err("Snapshot Link mirror has reserved bytes".into());
        }
        let mut link = vec![raw[0], raw[4]];
        link.extend_from_slice(&raw[6..26]);
        let row = Self {
            link: SnapshotLinkRecord::decode(&link)?,
            remote_meta: raw[1],
            fetched_at_s: u64::from_le_bytes(raw[26..34].try_into().unwrap()),
        };
        row.encode()?;
        Ok(row)
    }
}
