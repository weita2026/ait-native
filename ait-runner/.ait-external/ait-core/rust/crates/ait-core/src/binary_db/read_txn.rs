//! Binary DB read transaction lifecycle.

use super::*;
use std::cell::RefCell;

pub struct BinaryDbReadTxn<'a, B: BinaryDb + ?Sized> {
    db: &'a B,
    read_lock: StoreResult<BinaryDbReadLockSet>,
    cache: RefCell<BinaryDbReadCache>,
}

impl<'a, B> BinaryDbReadTxn<'a, B>
where
    B: BinaryDb + ?Sized,
{
    pub fn new(db: &'a B) -> Self {
        Self::new_for_scope(db, BinaryDbReadScope::All)
    }

    pub fn new_for_scope(db: &'a B, read_scope: BinaryDbReadScope) -> Self {
        Self {
            db,
            read_lock: db.acquire_read_lock_for_scope(read_scope),
            cache: RefCell::new(BinaryDbReadCache::default()),
        }
    }

    pub fn db(&self) -> &'a B {
        self.db
    }

    pub fn read_lock_paths(&self) -> StoreResult<&[PathBuf]> {
        Ok(self.read_guard()?.paths())
    }

    fn read_guard(&self) -> StoreResult<&BinaryDbReadLockSet> {
        self.read_lock.as_ref().map_err(Clone::clone)
    }

    pub fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.read_guard()?;
        if let Some(layout_id) = self.cache.borrow().layout_ids.get(&file).copied() {
            return Ok(layout_id);
        }
        let layout_id = self.db.layout_id(file.clone())?;
        self.cache.borrow_mut().layout_ids.insert(file, layout_id);
        Ok(layout_id)
    }

    pub fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.read_guard()?;
        self.db.record_count(file)
    }

    pub fn read_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
    ) -> StoreResult<BinaryRecordBytes> {
        self.read_guard()?;
        self.db
            .read_record_in_read_txn(file, record_index, &mut self.cache.borrow_mut())
    }

    pub fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.read_guard()?;
        self.db.read_payload(file, offset, len)
    }

    pub fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        self.read_guard()?;
        self.db
            .lookup_index_in_read_txn(index, key, &mut self.cache.borrow_mut())
    }

    #[cfg(test)]
    pub(crate) fn cached_parsed_index_count(&self) -> usize {
        self.cache.borrow().parsed_index_candidates.len()
    }
}
