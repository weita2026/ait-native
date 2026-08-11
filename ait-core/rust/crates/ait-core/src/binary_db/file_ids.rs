//! Binary DB file, payload, index, and range identities.

use super::*;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BinaryFileId {
    relative_path: StorePath,
    record_size: u32,
    layout_id: u32,
}

impl BinaryFileId {
    pub fn new(path: impl Into<StorePath>, layout_id: u32, record_size: u32) -> Self {
        Self {
            relative_path: path.into(),
            record_size,
            layout_id,
        }
    }

    pub fn relative_path(&self) -> &StorePath {
        &self.relative_path
    }

    pub fn record_size(&self) -> u32 {
        self.record_size
    }

    pub fn layout_id(&self) -> u32 {
        self.layout_id
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BinaryPayloadFileId {
    relative_path: StorePath,
    layout_id: u32,
}

impl BinaryPayloadFileId {
    pub fn new(path: impl Into<StorePath>, layout_id: u32) -> Self {
        Self {
            relative_path: path.into(),
            layout_id,
        }
    }

    pub fn relative_path(&self) -> &StorePath {
        &self.relative_path
    }

    pub fn layout_id(&self) -> u32 {
        self.layout_id
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BinaryIndexId {
    relative_path: StorePath,
    layout_id: u32,
    fixed_key_size: Option<u32>,
    stores_record_index_plus_one: bool,
}

impl BinaryIndexId {
    pub fn new(path: impl Into<StorePath>, layout_id: u32) -> Self {
        Self {
            relative_path: path.into(),
            layout_id,
            fixed_key_size: None,
            stores_record_index_plus_one: false,
        }
    }

    pub fn new_fixed(
        path: impl Into<StorePath>,
        layout_id: u32,
        key_size: u32,
        stores_record_index_plus_one: bool,
    ) -> Self {
        Self {
            relative_path: path.into(),
            layout_id,
            fixed_key_size: Some(key_size),
            stores_record_index_plus_one,
        }
    }

    pub fn relative_path(&self) -> &StorePath {
        &self.relative_path
    }

    pub fn layout_id(&self) -> u32 {
        self.layout_id
    }

    pub fn fixed_key_size(&self) -> Option<u32> {
        self.fixed_key_size
    }

    pub fn stores_record_index_plus_one(&self) -> bool {
        self.stores_record_index_plus_one
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadRange {
    pub payload_offset: u64,
    pub payload_len: u32,
}
