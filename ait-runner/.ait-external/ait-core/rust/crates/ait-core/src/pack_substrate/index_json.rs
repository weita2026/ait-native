use super::*;
use std::collections::BTreeMap;
use std::marker::PhantomData;

pub(in crate::pack_substrate) struct ObjectPackIndexJson<S> {
    _store: PhantomData<fn() -> S>,
}

impl<S> ObjectPackIndexJson<S> {
    pub(in crate::pack_substrate) const fn new() -> Self {
        Self {
            _store: PhantomData,
        }
    }

    pub(in crate::pack_substrate) fn entries_by_name(
        &self,
        pack_index: &JsonValue,
    ) -> Result<BTreeMap<String, PackIndexEntry>, String> {
        pack_entries_by_name(pack_index)
    }

    pub(in crate::pack_substrate) fn zstd_chunked_index_json(
        &self,
        pack_index: &ZstdChunkedPackIndex,
    ) -> Result<JsonValue, String> {
        validate_zstd_chunked_index(
            pack_index,
            ZSTD_CHUNKED_INDEX_KIND_OBJECT,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
            None,
        )?;
        zstd_chunked_object_pack_index_json(pack_index)
    }
}

impl<S> Default for ObjectPackIndexJson<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectPackIndexJson<()> {
    pub(in crate::pack_substrate) const fn stateless() -> Self {
        Self::new()
    }
}

pub(in crate::pack_substrate) struct TreePackIndexJson<S> {
    _store: PhantomData<fn() -> S>,
}

impl<S> TreePackIndexJson<S> {
    pub(in crate::pack_substrate) const fn new() -> Self {
        Self {
            _store: PhantomData,
        }
    }

    pub(in crate::pack_substrate) fn zstd_chunked_index_json(
        &self,
        pack_index: &ZstdChunkedPackIndex,
    ) -> Result<JsonValue, String> {
        validate_zstd_chunked_index(
            pack_index,
            ZSTD_CHUNKED_INDEX_KIND_TREE,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            None,
        )?;
        zstd_chunked_tree_pack_index_json(pack_index)
    }
}

impl<S> Default for TreePackIndexJson<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl TreePackIndexJson<()> {
    pub(in crate::pack_substrate) const fn stateless() -> Self {
        Self::new()
    }
}
