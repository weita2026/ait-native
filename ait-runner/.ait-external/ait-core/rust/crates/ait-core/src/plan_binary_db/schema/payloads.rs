use crate::binary_db::{BinaryDbError, StoreResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanPayload {
    pub title_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRevisionPayload {
    pub title_snapshot_bytes: Vec<u8>,
    pub summary_bytes: Vec<u8>,
    pub artifact_path_bytes: Vec<u8>,
    pub artifact_selector_bytes: Vec<u8>,
    pub artifact_heading_bytes: Vec<u8>,
    pub artifact_blob_id_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItemPayload {
    pub plan_item_ref_bytes: Vec<u8>,
    pub text_bytes: Vec<u8>,
    pub heading_path: Vec<String>,
}

impl PlanPayload {
    pub fn title_text(&self) -> StoreResult<String> {
        decode_utf8(&self.title_bytes, "plan title")
    }
}

impl PlanRevisionPayload {
    pub fn title_snapshot_text(&self) -> StoreResult<String> {
        decode_utf8(&self.title_snapshot_bytes, "plan revision title")
    }

    pub fn summary_text(&self) -> StoreResult<String> {
        decode_utf8(&self.summary_bytes, "plan revision summary")
    }

    pub fn artifact_path_text(&self) -> StoreResult<String> {
        decode_utf8(&self.artifact_path_bytes, "plan revision artifact_path")
    }

    pub fn artifact_selector_text(&self) -> StoreResult<String> {
        decode_utf8(
            &self.artifact_selector_bytes,
            "plan revision artifact_selector",
        )
    }

    pub fn artifact_heading_text(&self) -> StoreResult<String> {
        decode_utf8(
            &self.artifact_heading_bytes,
            "plan revision artifact_heading",
        )
    }

    pub fn artifact_blob_id_text(&self) -> StoreResult<String> {
        decode_utf8(
            &self.artifact_blob_id_bytes,
            "plan revision artifact_blob_id",
        )
    }
}

impl PlanItemPayload {
    pub fn plan_item_ref_text(&self) -> StoreResult<String> {
        decode_utf8(&self.plan_item_ref_bytes, "plan item ref")
    }

    pub fn text(&self) -> StoreResult<String> {
        decode_utf8(&self.text_bytes, "plan item text")
    }
}
fn decode_utf8(bytes: &[u8], label: &str) -> StoreResult<String> {
    String::from_utf8(bytes.to_vec())
        .map_err(|err| BinaryDbError::corruption(format!("{label} is not valid UTF-8: {err}")))
}
