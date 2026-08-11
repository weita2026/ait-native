use super::*;

#[derive(Clone, Debug)]
pub(crate) struct PackCandidate {
    pub(crate) entry_name: String,
    pub(crate) blob_id: String,
    pub(crate) data: Vec<u8>,
    pub(crate) path_hint: Option<String>,
    pub(crate) chain_depth: usize,
}

#[derive(Clone, Debug)]
pub(in crate::pack_substrate) struct PackMember {
    pub(in crate::pack_substrate) entry_name: String,
    pub(in crate::pack_substrate) blob_id: String,
    pub(in crate::pack_substrate) data: Vec<u8>,
    pub(in crate::pack_substrate) logical_data: Vec<u8>,
    pub(in crate::pack_substrate) entry_type: String,
    pub(in crate::pack_substrate) base_blob_id: Option<String>,
    pub(in crate::pack_substrate) chain_depth: usize,
    pub(in crate::pack_substrate) delta_algorithm: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::pack_substrate) struct PackIndexEntry {
    pub(in crate::pack_substrate) entry_name: String,
    pub(in crate::pack_substrate) blob_id: String,
    pub(in crate::pack_substrate) entry_type: String,
    pub(in crate::pack_substrate) byte_length: usize,
    pub(in crate::pack_substrate) uncompressed_byte_length: usize,
    pub(in crate::pack_substrate) base_blob_id: Option<String>,
    pub(in crate::pack_substrate) chain_depth: usize,
    pub(in crate::pack_substrate) checksum: String,
    pub(in crate::pack_substrate) delta_algorithm: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::pack_substrate) struct ZstdChunkedPackIndex {
    pub(in crate::pack_substrate) pack_format: String,
    pub(in crate::pack_substrate) pack_id: String,
    pub(in crate::pack_substrate) created_at: String,
    pub(in crate::pack_substrate) index_entry_name: String,
    pub(in crate::pack_substrate) chunks: Vec<ZstdChunkedChunkIndex>,
    pub(in crate::pack_substrate) members: Vec<ZstdChunkedMemberIndex>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::pack_substrate) struct ZstdChunkedChunkIndex {
    pub(in crate::pack_substrate) chunk_ordinal: usize,
    pub(in crate::pack_substrate) compressed_offset: u64,
    pub(in crate::pack_substrate) compressed_len: usize,
    pub(in crate::pack_substrate) raw_len: usize,
    pub(in crate::pack_substrate) checksum: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::pack_substrate) struct ZstdChunkedMemberIndex {
    pub(in crate::pack_substrate) member_ordinal: usize,
    pub(in crate::pack_substrate) entry_name: String,
    pub(in crate::pack_substrate) content_id: String,
    pub(in crate::pack_substrate) entry_type: String,
    pub(in crate::pack_substrate) entry_count: Option<usize>,
    pub(in crate::pack_substrate) base_content_id: Option<String>,
    pub(in crate::pack_substrate) delta_algorithm: Option<String>,
    pub(in crate::pack_substrate) chain_depth: usize,
    pub(in crate::pack_substrate) chunk_ordinal: usize,
    pub(in crate::pack_substrate) in_chunk_offset: usize,
    pub(in crate::pack_substrate) stored_len: usize,
    pub(in crate::pack_substrate) logical_len: usize,
    pub(in crate::pack_substrate) checksum: String,
}

#[derive(Debug)]
pub struct PackEntryArchive {
    pub(in crate::pack_substrate) pack_path: String,
    pub(in crate::pack_substrate) pack_index: ZstdChunkedPackIndex,
    pub(in crate::pack_substrate) raw_chunk_cache: BTreeMap<usize, Vec<u8>>,
    pub(in crate::pack_substrate) entries_by_name: BTreeMap<String, PackIndexEntry>,
}

#[derive(Debug)]
pub struct TreePackEntryArchive {
    pub(in crate::pack_substrate) pack_path: String,
    pub(in crate::pack_substrate) pack_index: ZstdChunkedPackIndex,
    pub(in crate::pack_substrate) raw_chunk_cache: BTreeMap<usize, Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPackWriteMember {
    pub entry_name: String,
    pub blob_id: String,
    pub data: Vec<u8>,
    pub logical_data: Option<Vec<u8>>,
    pub entry_type: String,
    pub base_blob_id: Option<String>,
    pub chain_depth: usize,
    pub delta_algorithm: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::pack_substrate) struct TreePackMember {
    pub(in crate::pack_substrate) tree_id: String,
    pub(in crate::pack_substrate) entry_name: String,
    pub(in crate::pack_substrate) entry_count: usize,
    pub(in crate::pack_substrate) data: Vec<u8>,
    pub(in crate::pack_substrate) checksum: String,
}

#[derive(Clone, Debug)]
pub(in crate::pack_substrate) struct TreePackIndexEntry {
    pub(in crate::pack_substrate) tree_id: String,
    pub(in crate::pack_substrate) entry_ordinal: usize,
    pub(in crate::pack_substrate) entry_count: usize,
    pub(in crate::pack_substrate) byte_length: usize,
    pub(in crate::pack_substrate) checksum: String,
}

#[derive(Clone, Debug)]
pub(in crate::pack_substrate) struct ZstdChunkedMemberInput {
    pub(in crate::pack_substrate) member_ordinal: usize,
    pub(in crate::pack_substrate) entry_name: String,
    pub(in crate::pack_substrate) content_id: String,
    pub(in crate::pack_substrate) entry_type: String,
    pub(in crate::pack_substrate) entry_count: Option<usize>,
    pub(in crate::pack_substrate) base_content_id: Option<String>,
    pub(in crate::pack_substrate) delta_algorithm: Option<String>,
    pub(in crate::pack_substrate) chain_depth: usize,
    pub(in crate::pack_substrate) data: Vec<u8>,
    pub(in crate::pack_substrate) logical_len: usize,
    pub(in crate::pack_substrate) checksum: String,
}
