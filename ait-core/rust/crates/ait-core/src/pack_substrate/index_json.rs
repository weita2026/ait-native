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

    /// Build the legacy lookup view only after the native container reader has
    /// verified index SHA, structure and data ranges. This conversion retains
    /// the JSON contract's text normalization and duplicate-name semantics;
    /// it creates no proof/cache beyond the owning immutable archive reader.
    pub(in crate::pack_substrate) fn entries_by_name_from_validated_index(
        &self,
        pack_index: &ZstdChunkedPackIndex,
    ) -> Result<BTreeMap<String, PackIndexEntry>, String> {
        #[cfg(test)]
        crate::binary_db::read_profile::note("object_index_typed_map_builds", 0);
        let required = |value: &str, field: &str| {
            let value = value.trim();
            if value.is_empty() {
                Err(format!("missing required field: {field}"))
            } else {
                Ok(value.to_string())
            }
        };
        let optional = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let mut out = BTreeMap::new();
        for member in &pack_index.members {
            let entry_name = required(&member.entry_name, "entry_name")?;
            let parsed = PackIndexEntry {
                entry_name: entry_name.clone(),
                blob_id: required(&member.content_id, "blob_id")?,
                entry_type: required(&member.entry_type, "entry_type")?,
                byte_length: member.stored_len,
                uncompressed_byte_length: member.logical_len,
                base_blob_id: optional(member.base_content_id.as_deref()),
                chain_depth: member.chain_depth,
                checksum: required(&member.checksum, "checksum")?,
                delta_algorithm: optional(member.delta_algorithm.as_deref()),
            };
            if let Some(existing) = out.get(&entry_name) {
                if pack_index_entries_equivalent(existing, &parsed) {
                    continue;
                }
                return Err(format!(
                    "Invalid pack index: duplicate entry_name {entry_name}"
                ));
            }
            out.insert(entry_name, parsed);
        }
        Ok(out)
    }

    pub(in crate::pack_substrate) fn zstd_chunked_index_json(
        &self,
        pack_index: &ZstdChunkedPackIndex,
    ) -> Result<JsonValue, String> {
        #[cfg(test)]
        crate::binary_db::read_profile::note("object_index_json_materializations", 0);
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
