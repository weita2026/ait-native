use super::*;

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

    pub fn persisted_name(self) -> &'static str {
        match self {
            Self::ZstdChunkedV1 => PACK_FORMAT_ZSTD_CHUNKED_V1,
        }
    }

    pub fn format_kind_name(self) -> &'static str {
        match self {
            Self::ZstdChunkedV1 => PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        }
    }

    pub fn index_entry_name(self) -> &'static str {
        match self {
            Self::ZstdChunkedV1 => ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME,
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

    pub fn index_entry_name(self) -> &'static str {
        match self {
            Self::ZstdChunkedTreeV1 => ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME,
        }
    }
}

pub trait ObjectPackBackend {
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

pub trait TreePackBackend {
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
}

#[derive(Debug)]
pub struct ZstdChunkedObjectPackBackend;

#[derive(Debug)]
pub struct ZstdChunkedTreePackBackend;

static ZSTD_CHUNKED_OBJECT_PACK_BACKEND: ZstdChunkedObjectPackBackend =
    ZstdChunkedObjectPackBackend;
static ZSTD_CHUNKED_TREE_PACK_BACKEND: ZstdChunkedTreePackBackend = ZstdChunkedTreePackBackend;

pub fn object_pack_backend(
    format_kind: PackFormatKind,
) -> Result<&'static dyn ObjectPackBackend, String> {
    match format_kind {
        PackFormatKind::ZstdChunkedV1 => Ok(&ZSTD_CHUNKED_OBJECT_PACK_BACKEND),
    }
}

pub fn object_pack_backend_from_persisted_format(
    persisted_pack_format: &str,
) -> Result<&'static dyn ObjectPackBackend, String> {
    object_pack_backend(PackFormatKind::from_persisted(persisted_pack_format)?)
}

pub fn tree_pack_backend(
    format_kind: TreePackFormatKind,
) -> Result<&'static dyn TreePackBackend, String> {
    match format_kind {
        TreePackFormatKind::ZstdChunkedTreeV1 => Ok(&ZSTD_CHUNKED_TREE_PACK_BACKEND),
    }
}

pub fn tree_pack_backend_from_persisted_format(
    persisted_pack_format: &str,
) -> Result<&'static dyn TreePackBackend, String> {
    tree_pack_backend(TreePackFormatKind::from_persisted(persisted_pack_format)?)
}

pub fn default_object_pack_relative_path(pack_id: &str) -> String {
    format!(".ait/objects/packs/{pack_id}{ZSTD_CHUNKED_PACK_SUFFIX}")
}

pub fn default_tree_pack_relative_path(pack_id: &str) -> String {
    format!(".ait/objects/tree-packs/{pack_id}{ZSTD_CHUNKED_PACK_SUFFIX}")
}
