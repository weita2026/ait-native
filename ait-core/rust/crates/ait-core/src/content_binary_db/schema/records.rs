#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryBlobRecord {
    pub blob_meta: u8,
    pub hash_kind: u8,
    pub reserved0: u16,
    pub size_bytes: u64,
    pub pack_member_index_plus1: u32,
    pub created_at_s: u64,
    pub pruned_at_s: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinarySnapshotRecord {
    pub snapshot_meta: u8,
    pub history_flags: u8,
    pub payload_len: u16,
    pub payload_offset: u64,
    pub snapshot_hash48: u64,
    pub parent_snapshot_index_plus1: u32,
    pub root_tree_pack_index_plus1: u32,
    pub root_entry_ordinal: u32,
    pub line_index_plus1: u32,
    pub manifest_hash: [u8; 32],
    pub file_count: u32,
    pub total_bytes: u64,
    pub created_at_s: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryObjectPackRecord {
    pub pack_meta: u8,
    pub pack_format_kind: u8,
    pub pack_hash_hi16: u16,
    pub pack_hash_lo32: u32,
    pub first_member_index: u32,
    pub member_count: u32,
    pub total_bytes: u64,
    pub created_at_s: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryObjectPackMemberRecord {
    pub member_meta: u8,
    pub delta_chain_depth: u8,
    pub reserved0: u16,
    pub pack_index: u32,
    pub blob_index: u32,
    pub base_blob_index_plus1: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryTreePackRecord {
    pub pack_meta: u8,
    pub pack_format_kind: u8,
    pub pack_hash_hi16: u16,
    pub pack_hash_lo32: u32,
    pub first_tree_index: u32,
    pub tree_count: u32,
    pub total_bytes: u64,
    pub created_at_s: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryTreeRecord {
    pub tree_meta: u8,
    pub reserved0: u8,
    pub pack_entry_ordinal: u32,
    pub entry_count: u32,
    pub tree_hash80: [u8; 10],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryObjectPackFormatKind {
    ZstdChunkedV1,
    Reserved(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinarySnapshotKind {
    Line,
    Stash,
    Reserved(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryTreePackFormatKind {
    ZstdChunkedTreeV1,
    Reserved(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryObjectPackMemberKind {
    Full,
    Delta,
    Reserved(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryObjectPackCompressionKind {
    None,
    Zstd,
    Reserved(u8),
}

impl BinaryBlobRecord {
    pub const META_HAS_PACK_MEMBER: u8 = 0b0000_0001;
    pub const META_PRUNED: u8 = 0b0000_0010;
    pub const META_TOMBSTONE: u8 = 0b1000_0000;

    pub fn is_pruned(&self) -> bool {
        self.blob_meta & Self::META_PRUNED != 0 || self.pruned_at_s != 0
    }

    pub fn is_tombstone(&self) -> bool {
        self.blob_meta & Self::META_TOMBSTONE != 0
    }

    pub fn pack_member_index(&self) -> Option<u32> {
        self.pack_member_index_plus1.checked_sub(1)
    }
}

impl BinarySnapshotRecord {
    pub const META_KIND_MASK: u8 = 0b0000_0011;
    pub const META_HAS_MESSAGE: u8 = 0b0000_0100;
    pub const META_HAS_LINE_NAME_PAYLOAD: u8 = 0b0000_1000;
    pub const META_HAS_ADDITIONAL_PARENTS: u8 = 0b0001_0000;
    pub const META_HAS_ROOT_LOCATOR: u8 = 0b0010_0000;
    pub const META_TOMBSTONE: u8 = 0b1000_0000;
    pub const FLAG_REMOTE_HEAD_HISTORY_BOUNDARY: u8 = 0b0000_0001;
    pub const KNOWN_FLAGS: u8 = Self::FLAG_REMOTE_HEAD_HISTORY_BOUNDARY;

    pub fn is_ready(&self) -> bool {
        self.has_root_locator()
    }

    pub fn is_tombstone(&self) -> bool {
        self.snapshot_meta & Self::META_TOMBSTONE != 0
    }

    pub fn kind(&self) -> BinarySnapshotKind {
        match self.snapshot_meta & Self::META_KIND_MASK {
            0 => BinarySnapshotKind::Line,
            1 => BinarySnapshotKind::Stash,
            other => BinarySnapshotKind::Reserved(other),
        }
    }

    pub fn snapshot_hash48(&self) -> u64 {
        self.snapshot_hash48
    }

    pub fn parent_snapshot_index(&self) -> Option<u32> {
        self.parent_snapshot_index_plus1.checked_sub(1)
    }

    pub fn has_additional_parents(&self) -> bool {
        self.snapshot_meta & Self::META_HAS_ADDITIONAL_PARENTS != 0
    }

    pub fn is_remote_head_history_boundary(&self) -> bool {
        self.history_flags & Self::FLAG_REMOTE_HEAD_HISTORY_BOUNDARY != 0
    }

    pub fn root_tree_pack_index(&self) -> Option<u32> {
        self.root_tree_pack_index_plus1.checked_sub(1)
    }

    pub fn line_index(&self) -> Option<u32> {
        self.line_index_plus1.checked_sub(1)
    }

    pub fn has_message(&self) -> bool {
        self.snapshot_meta & Self::META_HAS_MESSAGE != 0
    }

    pub fn has_line_name_payload(&self) -> bool {
        self.snapshot_meta & Self::META_HAS_LINE_NAME_PAYLOAD != 0
    }

    pub fn has_root_locator(&self) -> bool {
        self.snapshot_meta & Self::META_HAS_ROOT_LOCATOR != 0
    }
}

impl BinaryObjectPackRecord {
    pub const META_READY: u8 = 0b0000_0001;
    pub const META_CORRUPT: u8 = 0b0000_0010;
    pub const META_PRUNED: u8 = 0b0000_0100;
    pub const META_TOMBSTONE: u8 = 0b1000_0000;

    pub fn format_kind(&self) -> BinaryObjectPackFormatKind {
        match self.pack_format_kind {
            1 => BinaryObjectPackFormatKind::ZstdChunkedV1,
            other => BinaryObjectPackFormatKind::Reserved(other),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.pack_meta & Self::META_READY != 0
    }

    pub fn is_tombstone(&self) -> bool {
        self.pack_meta & Self::META_TOMBSTONE != 0
    }

    pub fn pack_hash48(&self) -> u64 {
        (u64::from(self.pack_hash_hi16) << 32) | u64::from(self.pack_hash_lo32)
    }
}

impl BinaryObjectPackMemberRecord {
    pub const META_TOMBSTONE: u8 = 0b1000_0000;

    pub fn member_kind(&self) -> BinaryObjectPackMemberKind {
        match self.member_meta & 0b0000_0011 {
            0 => BinaryObjectPackMemberKind::Full,
            1 => BinaryObjectPackMemberKind::Delta,
            other => BinaryObjectPackMemberKind::Reserved(other),
        }
    }

    pub fn compression_kind(&self) -> BinaryObjectPackCompressionKind {
        match (self.member_meta >> 2) & 0b0000_0011 {
            0 => BinaryObjectPackCompressionKind::None,
            2 => BinaryObjectPackCompressionKind::Zstd,
            other => BinaryObjectPackCompressionKind::Reserved(other),
        }
    }

    pub fn is_tombstone(&self) -> bool {
        self.member_meta & Self::META_TOMBSTONE != 0
    }

    pub fn base_blob_index(&self) -> Option<u32> {
        self.base_blob_index_plus1.checked_sub(1)
    }
}

impl BinaryTreePackRecord {
    pub const META_READY: u8 = 0b0000_0001;
    pub const META_CORRUPT: u8 = 0b0000_0010;
    pub const META_SPARSE_PHYSICAL_ORDINALS: u8 = 0b0000_0100;
    pub const META_TOMBSTONE: u8 = 0b1000_0000;

    pub fn format_kind(&self) -> BinaryTreePackFormatKind {
        match self.pack_format_kind {
            1 => BinaryTreePackFormatKind::ZstdChunkedTreeV1,
            other => BinaryTreePackFormatKind::Reserved(other),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.pack_meta & Self::META_READY != 0
    }

    pub fn is_tombstone(&self) -> bool {
        self.pack_meta & Self::META_TOMBSTONE != 0
    }

    pub fn has_sparse_physical_ordinals(&self) -> bool {
        self.pack_meta & Self::META_SPARSE_PHYSICAL_ORDINALS != 0
    }

    pub fn pack_hash48(&self) -> u64 {
        (u64::from(self.pack_hash_hi16) << 32) | u64::from(self.pack_hash_lo32)
    }
}

impl BinaryTreeRecord {
    pub const META_TOMBSTONE: u8 = 0b1000_0000;

    pub fn is_tombstone(&self) -> bool {
        self.tree_meta & Self::META_TOMBSTONE != 0
    }
}
