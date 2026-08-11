use super::*;

pub fn zstd_only_object_pack_write_format() -> &'static str {
    PackFormatKind::ZstdChunkedV1.persisted_name()
}

pub fn zstd_only_tree_pack_write_format() -> &'static str {
    TreePackFormatKind::ZstdChunkedTreeV1.persisted_name()
}
