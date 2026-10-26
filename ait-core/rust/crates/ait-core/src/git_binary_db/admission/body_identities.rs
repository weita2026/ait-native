//! Complete Blob identity metadata proofs live only inside one immutable read.
//! Exact ID lookup proofs and physical-ordinal proofs are separate: an ordinal
//! observed in raw Git does not bypass the native ID index's alias selection.
use super::*;
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Key {
    Ordinal(u32),
    Id(String),
}
struct Entry {
    identity: BodyIdentity,
    used: u64,
    size: usize,
}
pub(super) struct BodyIdentityMemo {
    values: BTreeMap<Key, Entry>,
    order: BTreeSet<(u64, Key)>,
    bytes: usize,
    budget: usize,
    clock: u64,
}
impl BodyIdentityMemo {
    pub(super) fn new(budget: usize) -> Self {
        Self {
            values: BTreeMap::new(),
            order: BTreeSet::new(),
            bytes: 0,
            budget,
            clock: 0,
        }
    }
    fn get(&mut self, key: &Key) -> Option<BodyIdentity> {
        let entry = match self.values.get_mut(key) {
            Some(entry) => entry,
            None => {
                return None;
            }
        };
        self.order.remove(&(entry.used, key.clone()));
        self.clock = self
            .clock
            .checked_add(1)
            .expect("bounded identity memo clock");
        entry.used = self.clock;
        self.order.insert((entry.used, key.clone()));
        #[cfg(test)]
        crate::binary_db::read_profile::note("algo_identity_memo_hits", 0);
        Some(entry.identity.clone())
    }
    pub(super) fn ordinal(&mut self, index: u32) -> Option<BodyIdentity> {
        self.get(&Key::Ordinal(index))
    }
    fn id(&mut self, id: &str) -> Option<BodyIdentity> {
        self.get(&Key::Id(id.into()))
    }
    fn remember(&mut self, key: Key, identity: BodyIdentity) -> Result<(), String> {
        // Conservative accounting includes both keyed trees, allocator nodes
        // and the two owned ID strings. No bodies or persisted trusted proof.
        let size = 384
            + match &key {
                Key::Id(id) => id.capacity().saturating_mul(2),
                Key::Ordinal(_) => 0,
            };
        if size.saturating_add(2048) > self.budget {
            return Ok(());
        }
        if let Some(old) = self.values.remove(&key) {
            self.bytes -= old.size;
            self.order.remove(&(old.used, key.clone()));
        }
        while self.values.len() >= 65536
            || self.bytes.saturating_add(size).saturating_add(2048) > self.budget
        {
            let Some((used, old_key)) = self.order.pop_first() else {
                break;
            };
            let old = self
                .values
                .remove(&old_key)
                .ok_or("identity memo LRU missing entry")?;
            debug_assert_eq!(used, old.used);
            self.bytes -= old.size;

            #[cfg(test)]
            crate::binary_db::read_profile::note("algo_identity_memo_evictions", old.size as u64);
        }
        self.clock = self
            .clock
            .checked_add(1)
            .ok_or("identity memo clock overflow")?;
        self.order.insert((self.clock, key.clone()));
        self.values.insert(
            key,
            Entry {
                identity,
                used: self.clock,
                size,
            },
        );
        self.bytes += size;

        #[cfg(test)]
        crate::binary_db::read_profile::note_max(
            "algo_identity_memo_peak_bytes",
            (self.bytes + 2048) as u64,
        );
        Ok(())
    }
    pub(super) fn remember_ordinal(
        &mut self,
        index: u32,
        identity: BodyIdentity,
    ) -> Result<(), String> {
        self.remember(Key::Ordinal(index), identity)
    }
    fn identity_for_id(
        &mut self,
        content: &LocalContentBinaryDb<1>,
        session: &mut GitObjectReadSession<'_>,
        id: &str,
        verified: &mut BTreeSet<u32>,
    ) -> Result<BodyIdentity, String> {
        if let Some(identity) = self.id(id) {
            return Ok(identity);
        }
        let view = content
            .blobs()
            .get_blob_view(&session.content_read, id)?
            .ok_or_else(|| format!("Unknown Binary DB blob: {id}"))?;
        let identity = native_body_identity(content, session, view.blob_index, verified, self)?;
        // Preserve the generic Snapshot row's checked i64 length conversion.
        i64::try_from(identity.size)
            .map_err(|_| format!("blob size overflows i64: {}", view.blob_id))?;
        self.remember(Key::Id(id.into()), identity.clone())?;
        Ok(identity)
    }
}

pub(super) fn code_file_identities(
    content: &LocalContentBinaryDb<1>,
    session: &mut GitObjectReadSession<'_>,
    snapshot_id: &str,
    trees: &mut crate::content_binary_db::BinaryDbTreeReadCache,
    verified: &mut BTreeSet<u32>,
    memo: &mut BodyIdentityMemo,
) -> Result<BTreeMap<String, (u16, BodyIdentity)>, String> {
    if content.db().authority_root() != session.content_read.db().authority_root() {
        return Err("Blob identity read scope must share content authority".into());
    }
    // Retain the exact Snapshot-ID lookup used by the previous collector.
    let root = content
        .snapshots()
        .get_snapshot_view(&session.content_read, snapshot_id)?
        .and_then(|v| v.root_tree_id)
        .ok_or_else(|| format!("missing root Tree for Snapshot {snapshot_id}"))?;
    let mut output = BTreeMap::new();

    let mut pending = vec![(String::new(), root, BTreeSet::new())];
    while let Some((prefix, id, mut ancestors)) = pending.pop() {
        if !ancestors.insert(id.clone()) {
            return Err(format!(
                "cycle detected while reading Binary DB snapshot tree {id}"
            ));
        }
        for entry in
            content
                .trees()
                .required_tree_entries_with_cache(&session.content_read, &id, trees)?
        {
            #[cfg(test)]
            crate::binary_db::read_profile::note("algo_native_tree_edge_checks", 0);
            let path = if prefix.is_empty() {
                entry.entry_name
            } else {
                format!("{prefix}/{}", entry.entry_name)
            };
            if entry.entry_type == "tree" {
                pending.push((path, entry.target_id, ancestors.clone()));
                continue;
            }
            if path.to_ascii_lowercase().ends_with(".md") {
                return Err("Git code Snapshot contains Markdown".into());
            }
            let mode = mode(snapshot_mode_to_git_mode(
                entry.mode.as_deref().unwrap_or_default(),
            )?)?;
            let identity = memo.identity_for_id(content, session, &entry.target_id, verified)?;
            if output.insert(path, (mode, identity)).is_some() {
                return Err("duplicate Git code Snapshot path".into());
            }
        }
    }

    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_identity_memo_keeps_lookup_and_ordinal_proofs_separate() {
        let mut memo = BodyIdentityMemo::new(4096);
        let identity = BodyIdentity {
            size: 3,
            sha256: [7; 32],
        };
        memo.remember_ordinal(0, identity.clone()).unwrap();
        assert!(memo.id("BLB-FIXTURE").is_none());
        memo.remember(Key::Id("BLB-FIXTURE".into()), identity.clone())
            .unwrap();
        assert_eq!(memo.id("BLB-FIXTURE"), Some(identity.clone()));
        for i in 1..100 {
            memo.remember_ordinal(i, identity.clone()).unwrap();
            assert!(memo.bytes + 2048 <= 4096);
            assert_eq!(memo.order.len(), memo.values.len());
        }
        assert!(memo.ordinal(0).is_none());
        assert!(memo.id("BLB-FIXTURE").is_none());
        let mut zero = BodyIdentityMemo::new(0);
        zero.remember_ordinal(0, identity).unwrap();
        assert_eq!(zero.bytes, 0);
    }
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    use crate::binary_db::{AuthorityId, LocalBinaryDbFs, LocalStateScope};
    #[test]
    fn memo_reuse_and_eviction_keep_body_and_metadata_mutation_guards() {
        for metadata in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let native = root.join("native");
            let content = LocalContentBinaryDb::<1>::new(
                root.join("content"),
                native.as_path(),
                AuthorityId::new("memo-test"),
                LocalStateScope::Repository,
            );
            let git = LocalBinaryDbFs::new(
                root.join("interop"),
                native.as_path(),
                AuthorityId::new("memo-test"),
                LocalStateScope::Repository,
            )
            .with_declared_bin_paths(GIT_OBJECT_STORE_BIN_PATHS)
            .with_declared_index_paths(GIT_OBJECT_STORE_INDEX_PATHS);
            let objects = LocalGitObjectStore::open(git, content.clone()).unwrap();
            let repo = objects
                .ensure_repository(
                    GitRepositoryKind::ImportSource,
                    1,
                    [3; 12],
                    "memo-test-source",
                )
                .unwrap();
            let body = b"complete source bytes\r\n";
            let oid = git_object_id(GitObjectKind::Blob, body);
            objects
                .put(repo, GitObjectKind::Blob, 1, &oid, body)
                .unwrap();
            let record = objects.get_record(repo, 1, &oid).unwrap().unwrap();
            let mut bodies = crate::content_binary_db::BinaryBlobReadSession::for_git_admission();
            let mut session = objects.read_session(&mut bodies);
            session.verify_metadata_authority().unwrap();
            let mut verified = BTreeSet::new();
            let mut memo = BodyIdentityMemo::new(16384);
            let first = native_body_identity(
                &content,
                &mut session,
                record.content_blob_index,
                &mut verified,
                &mut memo,
            )
            .unwrap();
            assert_eq!(first.size, body.len() as u64);
            assert_eq!(first.sha256, Sha256::digest(body).as_slice());
            let foreign = LocalContentBinaryDb::<1>::new(
                root.join("foreign-content"),
                root.join("foreign-native"),
                AuthorityId::new("foreign"),
                LocalStateScope::Repository,
            );
            assert!(native_body_identity(
                &foreign,
                &mut session,
                record.content_blob_index,
                &mut verified,
                &mut memo
            )
            .unwrap_err()
            .contains("scope"));
            session.body_read.evict_readers_for_test();
            let paths = if metadata {
                vec![root.join("content/blob.bin")]
            } else {
                std::fs::read_dir(native.join(".ait/objects/packs"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .collect::<Vec<_>>()
            };
            for path in paths {
                let mut bytes = std::fs::read(&path).unwrap();
                bytes[4] ^= 0x80;
                std::fs::write(path, bytes).unwrap();
            }
            // The exact memo hit is allowed internally; it must never turn a
            // changed held authority into a successful published admission.
            assert_eq!(
                native_body_identity(
                    &content,
                    &mut session,
                    record.content_blob_index,
                    &mut verified,
                    &mut memo
                )
                .unwrap(),
                first
            );
            if metadata {
                assert!(session.verify_metadata_authority().is_err())
            } else {
                assert!(session.body_read.verify_admission_stamps().is_err())
            }
        }
    }
}
