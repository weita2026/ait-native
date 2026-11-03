use crate::binary_db::{BinaryDbError, BinaryFileId, BinaryIndexId, StoreResult};
use sha1::{Digest, Sha1};
use sha2::Sha256;

pub const GIT_OBJECT_RECORD_SIZE: u32 = 32;
pub const GIT_COMMIT_MAPPING_RECORD_SIZE: u32 = 100;
pub const GIT_DOCUMENT_VERSION_DESCRIPTOR_SIZE: usize = 48;
pub const GIT_REPOSITORY_RECORD_SIZE: u32 = 28;
pub const GIT_OBJECT_INDEX_RECORD_SIZE: u32 = 29;

pub fn git_object_file() -> BinaryFileId {
    BinaryFileId::new("git_object.bin", 1, GIT_OBJECT_RECORD_SIZE)
}

pub fn git_object_index() -> BinaryIndexId {
    BinaryIndexId::new_fixed("git_object.idx", 1, 25, true)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GitObjectKind {
    Commit = 0,
    Tree = 1,
    Blob = 2,
    Tag = 3,
}

impl GitObjectKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Tree => "tree",
            Self::Blob => "blob",
            Self::Tag => "tag",
        }
    }

    fn decode(value: u8) -> StoreResult<Self> {
        match value {
            0 => Ok(Self::Commit),
            1 => Ok(Self::Tree),
            2 => Ok(Self::Blob),
            3 => Ok(Self::Tag),
            _ => Err(BinaryDbError::corruption("unsupported Git object kind")),
        }
    }
}

/// Rejects unsupported formats and widths before any persistence or lock IO.
pub fn require_sha1_object_id(format: u8, bytes: &[u8]) -> StoreResult<[u8; 20]> {
    if format != 1 || bytes.len() != 20 {
        return Err(BinaryDbError::unsupported(
            "Git interop admits SHA-1 and exactly 20 raw OID bytes only",
        ));
    }
    Ok(bytes.try_into().expect("checked OID width"))
}

pub fn git_object_id(kind: GitObjectKind, body: &[u8]) -> [u8; 20] {
    let mut hash = Sha1::new();
    hash.update(format!("{} {}\0", kind.name(), body.len()).as_bytes());
    hash.update(body);
    hash.finalize().into()
}

pub(crate) fn body_sha256(body: &[u8]) -> [u8; 32] {
    Sha256::digest(body).into()
}

pub fn validate_git_artifact_path(path: &str) -> StoreResult<()> {
    if path.is_empty()
        || path.contains(['\\', '\0'])
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | "..") || part.len() > 255)
    {
        return Err(BinaryDbError::unsupported(
            "noncanonical Git artifact path or segment width",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitObjectRecord {
    pub kind: GitObjectKind,
    pub repository_index: u32,
    pub content_blob_index: u32,
    pub object_id: [u8; 20],
}

impl GitObjectRecord {
    pub fn encode(&self) -> [u8; 32] {
        let mut bytes = [0; 32];
        bytes[0] = self.kind as u8;
        bytes[1] = 1;
        bytes[4..8].copy_from_slice(&self.repository_index.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.content_blob_index.to_le_bytes());
        bytes[12..32].copy_from_slice(&self.object_id);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> StoreResult<Self> {
        require_width(bytes, 32, "GitObject")?;
        require_sha1_object_id(bytes[1], &bytes[12..32])?;
        if bytes[2..4] != [0, 0] {
            return Err(BinaryDbError::corruption(
                "GitObject reserved bits are nonzero",
            ));
        }
        Ok(Self {
            kind: GitObjectKind::decode(bytes[0])?,
            repository_index: u32_at(bytes, 4),
            content_blob_index: u32_at(bytes, 8),
            object_id: bytes[12..32].try_into().expect("checked record width"),
        })
    }

    pub fn index_key(&self) -> [u8; 25] {
        object_key(self.repository_index, &self.object_id)
    }
}

pub(crate) fn object_key(repository_index: u32, oid: &[u8; 20]) -> [u8; 25] {
    let mut key = [0; 25];
    key[..4].copy_from_slice(&repository_index.to_le_bytes());
    key[4] = 1;
    key[5..].copy_from_slice(oid);
    key
}

/// Exact source revision and mode, stored as an ordinary native Blob body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitDocumentVersionDescriptor {
    pub plan_revision_index: u32,
    pub git_mode: u16,
    pub raw_body_length: u64,
    pub raw_body_sha256: [u8; 32],
}

impl GitDocumentVersionDescriptor {
    pub fn encode(&self) -> StoreResult<[u8; 48]> {
        validate_mode(self.git_mode)?;
        let mut bytes = [0; 48];
        bytes[..4].copy_from_slice(&self.plan_revision_index.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.git_mode.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.raw_body_length.to_le_bytes());
        bytes[16..48].copy_from_slice(&self.raw_body_sha256);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> StoreResult<Self> {
        require_width(bytes, 48, "GitDocumentVersionDescriptor")?;
        let git_mode = u16::from_le_bytes(bytes[4..6].try_into().expect("checked width"));
        validate_mode(git_mode)?;
        if bytes[6..8] != [0, 0] {
            return Err(BinaryDbError::corruption(
                "descriptor reserved bits are nonzero",
            ));
        }
        Ok(Self {
            plan_revision_index: u32_at(bytes, 0),
            git_mode,
            raw_body_length: u64_at(bytes, 8),
            raw_body_sha256: bytes[16..48].try_into().expect("checked width"),
        })
    }

    pub fn verify_body(&self, body: &[u8]) -> StoreResult<()> {
        validate_mode(self.git_mode)?;
        if self.raw_body_length != body.len() as u64 || self.raw_body_sha256 != body_sha256(body) {
            return Err(BinaryDbError::corruption(
                "document body length/hash mismatch",
            ));
        }
        if self.git_mode == 0o120000 && body.contains(&0) {
            return Err(BinaryDbError::invalid_domain_data(
                "Git symlink target contains NUL",
            ));
        }
        Ok(())
    }
}

fn validate_mode(mode: u16) -> StoreResult<()> {
    if matches!(mode, 0o644 | 0o755 | 0o120000) {
        Ok(())
    } else {
        Err(BinaryDbError::unsupported("noncanonical Git document mode"))
    }
}

/// New object-backed records have no normalized parent/file/payload ranges.
/// Legacy 96-byte inputs require explicit offline conversion, not this codec.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCommitMappingRecord {
    pub mapping_meta: u8,
    pub generation_index: u32,
    pub snapshot_index: u32,
    pub author_identity_index: u32,
    pub committer_identity_index: u32,
    pub object_id: [u8; 20],
    pub tree_object_id: [u8; 20],
    pub lfs_pointer_count: u32,
    pub created_at_s: u64,
    pub document_set_tree_index: u32,
}

impl GitCommitMappingRecord {
    pub fn encode(&self) -> StoreResult<[u8; 100]> {
        if self.mapping_meta & 8 == 0 || self.mapping_meta & !15 != 0 {
            return Err(BinaryDbError::invalid_domain_data(
                "mapping must be object-backed",
            ));
        }
        let mut bytes = [0; 100];
        bytes[0] = self.mapping_meta;
        for (offset, value) in [
            (4, self.generation_index),
            (8, self.snapshot_index),
            (12, self.author_identity_index),
            (16, self.committer_identity_index),
            (84, self.lfs_pointer_count),
            (96, self.document_set_tree_index),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[44..64].copy_from_slice(&self.object_id);
        bytes[64..84].copy_from_slice(&self.tree_object_id);
        bytes[88..96].copy_from_slice(&self.created_at_s.to_le_bytes());
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> StoreResult<Self> {
        require_width(bytes, 100, "object-backed GitCommitMapping")?;
        if bytes[0] & 8 == 0
            || bytes[0] & !15 != 0
            || bytes[1..4] != [0; 3]
            || bytes[20..44] != [0; 24]
        {
            return Err(BinaryDbError::corruption(
                "invalid object-backed mapping flags/ranges",
            ));
        }
        Ok(Self {
            mapping_meta: bytes[0],
            generation_index: u32_at(bytes, 4),
            snapshot_index: u32_at(bytes, 8),
            author_identity_index: u32_at(bytes, 12),
            committer_identity_index: u32_at(bytes, 16),
            object_id: bytes[44..64].try_into().expect("checked width"),
            tree_object_id: bytes[64..84].try_into().expect("checked width"),
            lfs_pointer_count: u32_at(bytes, 84),
            created_at_s: u64_at(bytes, 88),
            document_set_tree_index: u32_at(bytes, 96),
        })
    }
}

pub(crate) fn require_width(bytes: &[u8], width: usize, name: &str) -> StoreResult<()> {
    if bytes.len() != width {
        return Err(BinaryDbError::layout_mismatch(format!(
            "{name} requires {width} bytes, found {}; legacy widths require explicit conversion",
            bytes.len()
        )));
    }
    Ok(())
}

pub(crate) fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked record width"),
    )
}

pub(crate) fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("checked record width"),
    )
}
