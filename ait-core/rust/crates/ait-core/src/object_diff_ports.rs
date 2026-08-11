use crate::json_support::JsonValue;
use std::collections::BTreeMap;

pub trait ObjectReader {
    fn read_object_json(&self, object_id: &str) -> Result<Option<JsonValue>, String>;
}

impl<T: ObjectReader + ?Sized> ObjectReader for &T {
    fn read_object_json(&self, object_id: &str) -> Result<Option<JsonValue>, String> {
        (**self).read_object_json(object_id)
    }
}

pub trait BlobReader {
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Option<Vec<u8>>, String>;

    fn read_blob_bytes_batch(
        &self,
        blob_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let mut payload = BTreeMap::new();
        for blob_id in blob_ids {
            if let Some(bytes) = self.read_blob_bytes(blob_id)? {
                payload.insert(blob_id.clone(), bytes);
            }
        }
        Ok(payload)
    }
}

impl<T: BlobReader + ?Sized> BlobReader for &T {
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Option<Vec<u8>>, String> {
        (**self).read_blob_bytes(blob_id)
    }

    fn read_blob_bytes_batch(
        &self,
        blob_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, String> {
        (**self).read_blob_bytes_batch(blob_ids)
    }
}

pub trait SnapshotReader {
    fn read_snapshot_manifest(&self, snapshot_id: &str) -> Result<JsonValue, String>;

    fn read_snapshot_payload(&self, _snapshot_id: &str) -> Result<Option<JsonValue>, String> {
        Ok(None)
    }

    fn read_snapshot_root_tree_payload(
        &self,
        _snapshot_id: &str,
    ) -> Result<Option<JsonValue>, String> {
        Ok(None)
    }

    fn read_tree_payload(&self, _tree_id: &str) -> Result<Option<JsonValue>, String> {
        Ok(None)
    }
}

impl<T: SnapshotReader + ?Sized> SnapshotReader for &T {
    fn read_snapshot_manifest(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        (**self).read_snapshot_manifest(snapshot_id)
    }

    fn read_snapshot_payload(&self, snapshot_id: &str) -> Result<Option<JsonValue>, String> {
        (**self).read_snapshot_payload(snapshot_id)
    }

    fn read_snapshot_root_tree_payload(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<JsonValue>, String> {
        (**self).read_snapshot_root_tree_payload(snapshot_id)
    }

    fn read_tree_payload(&self, tree_id: &str) -> Result<Option<JsonValue>, String> {
        (**self).read_tree_payload(tree_id)
    }
}
