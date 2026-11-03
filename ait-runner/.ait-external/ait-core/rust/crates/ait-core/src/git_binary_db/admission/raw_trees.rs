//! Parsed raw Trees live only inside one held immutable admission context.
use super::*;
use std::sync::Arc;
type Entries = Arc<Vec<(u16, String, [u8; 20])>>;
type Key = (u32, [u8; 20]);
struct CachedTree {
    entries: Entries,
    size: usize,
    used: u64,
}
pub(super) struct RawTreeReadCache {
    values: BTreeMap<Key, CachedTree>,
    order: BTreeSet<(u64, Key)>,
    bytes: usize,
    budget: usize,
    clock: u64,
}
impl RawTreeReadCache {
    pub(super) fn new(budget: usize) -> Self {
        Self {
            values: BTreeMap::new(),
            order: BTreeSet::new(),
            bytes: 0,
            budget,
            clock: 0,
        }
    }
    pub(super) fn remember(
        &mut self,
        record: &GitObjectRecord,
        body: &[u8],
    ) -> Result<Entries, String> {
        #[cfg(test)]
        crate::binary_db::read_profile::note("raw_tree_parses", body.len() as u64);
        let entries = Arc::new(tree_entries(body)?);
        let size = entries
            .capacity()
            .saturating_mul(std::mem::size_of::<(u16, String, [u8; 20])>())
            .saturating_add(
                entries
                    .iter()
                    .map(|(_, name, _)| name.capacity())
                    .sum::<usize>(),
            )
            .saturating_add(256); // Arc plus both bounded map/set nodes.
        if size <= self.budget {
            let key = (record.content_blob_index, record.object_id);
            if let Some(old) = self.values.remove(&key) {
                self.bytes -= old.size;
                self.order.remove(&(old.used, key));
            }
            while self.bytes.saturating_add(size) > self.budget {
                let Some((used, key)) = self.order.pop_first() else {
                    break;
                };
                let old = self
                    .values
                    .remove(&key)
                    .ok_or("raw Tree LRU missing entry")?;
                debug_assert_eq!(used, old.used);
                self.bytes -= old.size;
                #[cfg(test)]
                crate::binary_db::read_profile::note("raw_tree_evictions", old.size as u64);
            }
            self.clock = self
                .clock
                .checked_add(1)
                .ok_or("raw Tree LRU clock overflow")?;
            self.values.insert(
                key,
                CachedTree {
                    entries: entries.clone(),
                    size,
                    used: self.clock,
                },
            );
            self.order.insert((self.clock, key));
            self.bytes += size;
        }
        Ok(entries)
    }
    pub(super) fn entries(
        &mut self,
        session: &mut GitObjectReadSession<'_>,
        repository: u32,
        oid: &[u8; 20],
    ) -> Result<Entries, String> {
        let record = session
            .record(repository, 1, oid)?
            .ok_or("Git tree absent")?;
        if record.kind != GitObjectKind::Tree {
            return Err("Git tree object type differs".into());
        }
        let key = (record.content_blob_index, record.object_id);
        if let Some(hit) = self.values.get_mut(&key) {
            self.order.remove(&(hit.used, key));
            self.clock = self
                .clock
                .checked_add(1)
                .ok_or("raw Tree LRU clock overflow")?;
            hit.used = self.clock;
            self.order.insert((hit.used, key));
            #[cfg(test)]
            crate::binary_db::read_profile::note("raw_tree_hits", 0);
            return Ok(hit.entries.clone());
        }
        let (record, body) = session.get(repository, 1, oid)?.ok_or("Git tree absent")?;
        self.remember(&record, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parsed_tree_lru_bounds_entries_and_does_not_admit_oversize_values() {
        let mut cache = RawTreeReadCache::new(1000);
        for i in 0..32u32 {
            let mut body = format!("100644 item-{i}\0").into_bytes();
            body.extend_from_slice(&[1; 20]);
            let record = GitObjectRecord {
                kind: GitObjectKind::Tree,
                repository_index: 0,
                content_blob_index: i,
                object_id: git_object_id(GitObjectKind::Tree, &body),
            };
            assert_eq!(cache.remember(&record, &body).unwrap().len(), 1);
            assert!(cache.bytes <= cache.budget);
            assert_eq!(cache.values.len(), cache.order.len());
        }
        assert!(cache.values.len() < 4);
        let mut body = Vec::new();
        for i in 0..32 {
            body.extend_from_slice(format!("100644 large-{i}\0").as_bytes());
            body.extend_from_slice(&[2; 20]);
        }
        let record = GitObjectRecord {
            kind: GitObjectKind::Tree,
            repository_index: 0,
            content_blob_index: 33,
            object_id: git_object_id(GitObjectKind::Tree, &body),
        };
        let before = cache.bytes;
        assert_eq!(cache.remember(&record, &body).unwrap().len(), 32);
        assert_eq!(before, cache.bytes);
        assert!(!cache
            .values
            .contains_key(&(record.content_blob_index, record.object_id)));
        assert!(cache.remember(&record, b"truncated").is_err());
    }
}
