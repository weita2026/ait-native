use super::records::{require_width, u32_at, u64_at};
use super::*;
use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId,
    BinaryIndexId, BinaryPayloadFileId, LocalBinaryDbFs, StoreResult,
};
use std::collections::BTreeMap;

fn file(name: &str, size: u32) -> BinaryFileId {
    BinaryFileId::new(name, 1, size)
}
fn index(name: &str, size: u32) -> BinaryIndexId {
    BinaryIndexId::new_fixed(name, 1, size, true)
}
type HistoryIndexRows = Vec<(&'static str, u32, Vec<(Vec<u8>, u32)>)>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitGenerationRecord {
    pub export: bool,
    pub source_repository_index: u32,
    pub target_repository_index_plus1: u32,
    pub hash: [u8; 8],
}
impl GitGenerationRecord {
    fn encode(&self) -> [u8; 20] {
        let mut b = [0; 20];
        b[0] = u8::from(self.export);
        b[4..8].copy_from_slice(&self.source_repository_index.to_le_bytes());
        b[8..12].copy_from_slice(&self.target_repository_index_plus1.to_le_bytes());
        b[12..20].copy_from_slice(&self.hash);
        b
    }
    pub fn decode(b: &[u8]) -> StoreResult<Self> {
        require_width(b, 20, "GitGeneration")?;
        if b[0] > 1 || b[1..4] != [0; 3] {
            return Err(BinaryDbError::corruption("invalid Git generation flags"));
        }
        Ok(Self {
            export: b[0] == 1,
            source_repository_index: u32_at(b, 4),
            target_repository_index_plus1: u32_at(b, 8),
            hash: b[12..20].try_into().unwrap(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRefMappingRecord {
    pub meta: u8,
    pub object_kind: GitObjectKind,
    pub generation_index: u32,
    pub snapshot_index_plus1: u32,
    pub target_index_plus1: u32,
    pub symbolic_target_index_plus1: u32,
    pub object_id: [u8; 20],
    pub expected_previous: [u8; 20],
    pub created_at_s: u64,
}
impl GitRefMappingRecord {
    fn encode(&self, name_len: u32, offset: u64) -> StoreResult<[u8; 80]> {
        if self.meta & !63 != 0
            || self.meta & 3 == 3
            || (self.meta & 4 != 0) != (self.snapshot_index_plus1 != 0)
            || (self.meta & 8 != 0) != (self.target_index_plus1 != 0)
            || (self.meta & 16 == 0 && self.expected_previous != [0; 20])
            || (self.meta & 3 == 2) != (self.symbolic_target_index_plus1 != 0)
        {
            return Err(BinaryDbError::invalid_domain_data(
                "invalid Git ref mapping metadata",
            ));
        }
        let mut b = [0; 80];
        b[0] = self.meta;
        b[1] = self.object_kind as u8;
        for (o, v) in [
            (4, self.generation_index),
            (8, self.snapshot_index_plus1),
            (12, self.target_index_plus1),
            (16, self.symbolic_target_index_plus1),
            (20, name_len),
        ] {
            b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[24..32].copy_from_slice(&offset.to_le_bytes());
        b[32..52].copy_from_slice(&self.object_id);
        b[52..72].copy_from_slice(&self.expected_previous);
        b[72..80].copy_from_slice(&self.created_at_s.to_le_bytes());
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> StoreResult<Self> {
        require_width(b, 80, "GitRefMapping")?;
        if b[2..4] != [0; 2] {
            return Err(BinaryDbError::corruption("Git ref reserved bits"));
        }
        let kind = match b[1] {
            0 => GitObjectKind::Commit,
            1 => GitObjectKind::Tree,
            2 => GitObjectKind::Blob,
            3 => GitObjectKind::Tag,
            _ => return Err(BinaryDbError::corruption("Git ref object kind")),
        };
        let r = Self {
            meta: b[0],
            object_kind: kind,
            generation_index: u32_at(b, 4),
            snapshot_index_plus1: u32_at(b, 8),
            target_index_plus1: u32_at(b, 12),
            symbolic_target_index_plus1: u32_at(b, 16),
            object_id: b[32..52].try_into().unwrap(),
            expected_previous: b[52..72].try_into().unwrap(),
            created_at_s: u64_at(b, 72),
        };
        r.encode(u32_at(b, 20), u64_at(b, 24))?;
        Ok(r)
    }
}

#[derive(Clone, Debug)]
pub struct GitOperationCheckpointRecord {
    pub status: u8,
    pub generation_index: u32,
    pub operation_hash: [u8; 8],
    pub plan_hash: [u8; 12],
    pub next_commit: u32,
    pub next_ref: u32,
    pub updated_at_s: u64,
}
impl GitOperationCheckpointRecord {
    fn encode(&self) -> StoreResult<[u8; 44]> {
        if self.status > 1 {
            return Err(BinaryDbError::invalid_domain_data("checkpoint state"));
        }
        let mut b = [0; 44];
        b[0] = self.status;
        b[4..8].copy_from_slice(&self.generation_index.to_le_bytes());
        b[8..16].copy_from_slice(&self.operation_hash);
        b[16..28].copy_from_slice(&self.plan_hash);
        b[28..32].copy_from_slice(&self.next_commit.to_le_bytes());
        b[32..36].copy_from_slice(&self.next_ref.to_le_bytes());
        b[36..44].copy_from_slice(&self.updated_at_s.to_le_bytes());
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> StoreResult<Self> {
        require_width(b, 44, "GitCheckpoint")?;
        if b[0] > 1 || b[1..4] != [0; 3] {
            return Err(BinaryDbError::corruption("checkpoint flags"));
        }
        Ok(Self {
            status: b[0],
            generation_index: u32_at(b, 4),
            operation_hash: b[8..16].try_into().unwrap(),
            plan_hash: b[16..28].try_into().unwrap(),
            next_commit: u32_at(b, 28),
            next_ref: u32_at(b, 32),
            updated_at_s: u64_at(b, 36),
        })
    }
}

pub struct GitHistoryStore {
    db: LocalBinaryDbFs,
    commits: BTreeMap<(u32, [u8; 20]), (u32, GitCommitMappingRecord)>,
}
impl GitHistoryStore {
    pub fn open(db: LocalBinaryDbFs) -> StoreResult<Self> {
        let read = BinaryDbReadTxn::new(&db);
        let mut commits = BTreeMap::new();
        for ordinal in 0..read.record_count(file("git_commit_mapping.bin", 100))? {
            let r = GitCommitMappingRecord::decode(
                &read.read_record(file("git_commit_mapping.bin", 100), ordinal)?,
            )?;
            if commits
                .insert((r.generation_index, r.object_id), (ordinal, r))
                .is_some()
            {
                return Err(BinaryDbError::corruption("duplicate Git commit mapping"));
            }
        }
        drop(read);
        let store = Self { db, commits };
        store.validate_indexes()?;
        Ok(store)
    }
    fn index_rows(&self) -> StoreResult<HistoryIndexRows> {
        let read = BinaryDbReadTxn::new(&self.db);
        for ordinal in 0..read.record_count(file("git_identity.bin", 24))? {
            self.identity_raw_with_read(&read, ordinal)?;
        }
        let generations = self.generations()?;
        let mut generation_rows = Vec::new();
        for (ordinal, row) in generations.iter().enumerate() {
            let source =
                read.read_record(file("git_repository.bin", 28), row.source_repository_index)?;
            if source[0] != if row.export { 2 } else { 0 }
                || (row.target_repository_index_plus1 != 0) != row.export
            {
                return Err(BinaryDbError::corruption(
                    "Git generation repository binding",
                ));
            }
            if row.export
                && read.read_record(
                    file("git_repository.bin", 28),
                    row.target_repository_index_plus1 - 1,
                )?[0]
                    != 1
            {
                return Err(BinaryDbError::corruption(
                    "Git generation target repository kind",
                ));
            }
            generation_rows.push((row.hash.to_vec(), ordinal as u32));
        }
        let mut commit_rows = Vec::new();
        let mut snapshot_rows = Vec::new();
        for (ordinal, row) in self.commits.values() {
            if row.generation_index as usize >= generations.len() {
                return Err(BinaryDbError::corruption("Git mapping generation FK"));
            }
            self.identity_raw_with_read(&read, row.author_identity_index)?;
            self.identity_raw_with_read(&read, row.committer_identity_index)?;
            let mut key = row.generation_index.to_le_bytes().to_vec();
            key.extend_from_slice(&row.object_id);
            commit_rows.push((key, *ordinal));
            snapshot_rows.push((row.snapshot_index.to_le_bytes().to_vec(), *ordinal));
        }
        let refs = self.refs()?;
        let mut ref_rows = Vec::new();
        for (ordinal, name, row) in &refs {
            let generation = generations
                .get(row.generation_index as usize)
                .ok_or("Git ref generation FK")?;
            if row.meta & 3 == 2 {
                let (_, target_name, target) = refs
                    .get((row.symbolic_target_index_plus1 - 1) as usize)
                    .ok_or("Git symbolic ref FK")?;
                if name != "HEAD"
                    || !target_name.starts_with("refs/heads/")
                    || target.generation_index != row.generation_index
                    || target.object_id != row.object_id
                    || target.meta & 3 != 0
                {
                    return Err(BinaryDbError::corruption("Git symbolic HEAD binding"));
                }
            }
            ref_rows.push((ref_key(generation, name), *ordinal));
        }
        let mut operation_rows = Vec::new();
        let mut operations = std::collections::BTreeSet::new();
        for ordinal in 0..read.record_count(file("git_operation_checkpoint.bin", 44))? {
            let row = GitOperationCheckpointRecord::decode(
                &read.read_record(file("git_operation_checkpoint.bin", 44), ordinal)?,
            )?;
            if row.generation_index as usize >= generations.len()
                || !operations.insert(row.operation_hash)
            {
                return Err(BinaryDbError::corruption(
                    "Git checkpoint generation/operation",
                ));
            }
            operation_rows.push((row.operation_hash.to_vec(), ordinal));
        }
        Ok(vec![
            ("git_generation_id.idx", 8, generation_rows),
            ("git_commit_object.idx", 24, commit_rows),
            ("git_commit_snapshot.idx", 4, snapshot_rows),
            ("git_ref_lookup.idx", 12, ref_rows),
            ("git_operation_id.idx", 8, operation_rows),
        ])
    }
    fn validate_indexes(&self) -> StoreResult<()> {
        let read = BinaryDbReadTxn::new(&self.db);
        for (name, width, rows) in self.index_rows()? {
            let length = self
                .db
                .metadata_len(&self.db.authority_root().as_path().join(name))?;
            if length != Some(4 + rows.len() as u64 * u64::from(width + 4))
                && !(rows.is_empty() && length.is_none())
            {
                return Err(BinaryDbError::corruption(format!(
                    "{name} requires explicit index recovery"
                )));
            }
            let mut expected = BTreeMap::<Vec<u8>, Vec<u32>>::new();
            for (key, ordinal) in rows {
                expected.entry(key).or_default().push(ordinal);
            }
            for (key, mut ordinals) in expected {
                let mut actual = read.lookup_index(index(name, width), &key)?;
                actual.sort_unstable();
                ordinals.sort_unstable();
                if actual != ordinals {
                    return Err(BinaryDbError::corruption(format!(
                        "{name} requires explicit index recovery"
                    )));
                }
            }
        }
        Ok(())
    }
    /// Recover only acceleration files. Fixed owners, payloads and physical
    /// ordinals remain byte-for-byte unchanged; interrupted rebuilds retry.
    pub fn rebuild_indexes(db: &LocalBinaryDbFs) -> StoreResult<()> {
        let read = BinaryDbReadTxn::new(db);
        let mut commits = BTreeMap::new();
        for ordinal in 0..read.record_count(file("git_commit_mapping.bin", 100))? {
            let row = GitCommitMappingRecord::decode(
                &read.read_record(file("git_commit_mapping.bin", 100), ordinal)?,
            )?;
            if commits
                .insert((row.generation_index, row.object_id), (ordinal, row))
                .is_some()
            {
                return Err(BinaryDbError::corruption(
                    "duplicate Git mapping during recovery",
                ));
            }
        }
        drop(read);
        let store = Self {
            db: db.clone(),
            commits,
        };
        for (name, width, mut rows) in store.index_rows()? {
            rows.sort_by_key(|(_, ordinal)| *ordinal);
            let mut bytes = 1u32.to_le_bytes().to_vec();
            for (key, ordinal) in rows {
                bytes.extend_from_slice(&key);
                bytes.extend_from_slice(
                    &ordinal
                        .checked_add(1)
                        .ok_or("Git index ordinal overflow")?
                        .to_le_bytes(),
                );
            }
            BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?
                .commit_with_atomic_file_replacement(
                    file(name, width + 4),
                    &bytes,
                    "git-history-index-recovery",
                )?;
        }
        Ok(())
    }
    pub fn identity_raw(&self, ordinal: u32) -> StoreResult<Vec<u8>> {
        let read = BinaryDbReadTxn::new(&self.db);
        self.identity_raw_with_read(&read, ordinal)
    }
    pub(crate) fn identity_raw_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
        ordinal: u32,
    ) -> StoreResult<Vec<u8>> {
        if read.db().authority_root() != self.db.authority_root() {
            return Err(BinaryDbError::invalid_domain_data(
                "Git identity read scope authority differs",
            ));
        }
        let record = read.read_record(file("git_identity.bin", 24), ordinal)?;
        if record[0] != 1 || record[1] != 0 {
            return Err(BinaryDbError::corruption("Git identity flags"));
        }
        let payload = read.read_payload(
            BinaryPayloadFileId::new("git_identity_payload.bin", 1),
            u64_at(&record, 8),
            u32_at(&record, 4),
        )?;
        if payload.len() < 8 {
            return Err(BinaryDbError::corruption("Git identity payload header"));
        }
        let raw_end = 8usize
            .checked_add(u32_at(&payload, 0) as usize)
            .ok_or("Git identity length overflow")?;
        let name_end = raw_end
            .checked_add(u32_at(&payload, 4) as usize)
            .ok_or("Git identity name overflow")?;
        if name_end > payload.len() {
            return Err(BinaryDbError::corruption("Git identity payload range"));
        }
        let raw = &payload[8..raw_end];
        let (timestamp, minutes, name, email) = identity_parts(raw)?;
        if record[2..4] != minutes.to_le_bytes()
            || record[16..24] != timestamp.to_le_bytes()
            || payload[raw_end..name_end] != *name
            || payload[name_end..] != *email
        {
            return Err(BinaryDbError::corruption(
                "Git identity typed/raw binding differs",
            ));
        }
        Ok(raw.to_vec())
    }
    pub fn ensure_generation(&self, r: GitGenerationRecord) -> StoreResult<u32> {
        let mut w = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)?;
        let candidates = w.lookup_index(index("git_generation_id.idx", 8), &r.hash)?;
        if candidates.len() > 1 {
            return Err(BinaryDbError::corruption("duplicate Git generation index"));
        }
        if let Some(ordinal) = candidates.into_iter().next() {
            let existing = GitGenerationRecord::decode(
                &w.read_record(file("git_generation.bin", 20), ordinal)?,
            )?;
            if existing == r {
                return Ok(ordinal);
            }
            return Err(BinaryDbError::corruption("Git generation hash collision"));
        }
        let ordinal = w.append_record(file("git_generation.bin", 20), &r.encode())?;
        w.append_index_candidate(index("git_generation_id.idx", 8), &r.hash, ordinal)?;
        w.commit()?;
        Ok(ordinal)
    }
    pub fn generations(&self) -> StoreResult<Vec<GitGenerationRecord>> {
        let read = BinaryDbReadTxn::new(&self.db);
        (0..read.record_count(file("git_generation.bin", 20))?)
            .map(|ordinal| {
                GitGenerationRecord::decode(
                    &read.read_record(file("git_generation.bin", 20), ordinal)?,
                )
            })
            .collect()
    }
    pub fn commit_mappings(&self) -> Vec<GitCommitMappingRecord> {
        self.commits.values().map(|(_, r)| r.clone()).collect()
    }
    /// Test-only physical-order candidate. Runtime keeps generation/OID order:
    /// the bounded real comparison did not show a sufficient speed gain.
    #[cfg(test)]
    pub(crate) fn physical_commit_mappings_for_profile(&self) -> Vec<GitCommitMappingRecord> {
        let mut rows = self.commits.values().collect::<Vec<_>>();
        rows.sort_unstable_by_key(|(ordinal, _)| *ordinal);
        rows.into_iter().map(|(_, record)| record.clone()).collect()
    }
    pub fn append_commit(&mut self, r: GitCommitMappingRecord) -> StoreResult<u32> {
        if let Some((ordinal, existing)) = self.commits.get(&(r.generation_index, r.object_id)) {
            if existing == &r {
                return Ok(*ordinal);
            }
            return Err(BinaryDbError::corruption(
                "conflicting immutable Git commit mapping",
            ));
        }
        let mut w = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)?;
        if w.record_count(file("git_commit_mapping.bin", 100))? as usize != self.commits.len() {
            return Err(BinaryDbError::retryable_busy(
                "Git mapping authority changed; reopen",
            ));
        }
        GitGenerationRecord::decode(
            &w.read_record(file("git_generation.bin", 20), r.generation_index)?,
        )?;
        let ordinal = w.append_record(file("git_commit_mapping.bin", 100), &r.encode()?)?;
        let mut key = r.generation_index.to_le_bytes().to_vec();
        key.extend_from_slice(&r.object_id);
        w.append_index_candidate(index("git_commit_object.idx", 24), &key, ordinal)?;
        w.append_index_candidate(
            index("git_commit_snapshot.idx", 4),
            &r.snapshot_index.to_le_bytes(),
            ordinal,
        )?;
        w.commit()?;
        self.commits
            .insert((r.generation_index, r.object_id), (ordinal, r));
        Ok(ordinal)
    }
    pub fn append_identity(&self, raw: &[u8]) -> StoreResult<u32> {
        let (timestamp, minutes, name, email) = identity_parts(raw)?;
        let mut payload = u32::try_from(raw.len())
            .map_err(|_| "identity raw length")?
            .to_le_bytes()
            .to_vec();
        payload.extend_from_slice(
            &u32::try_from(name.len())
                .map_err(|_| "identity name length")?
                .to_le_bytes(),
        );
        payload.extend_from_slice(raw);
        payload.extend_from_slice(name);
        payload.extend_from_slice(email);
        let mut w = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)?;
        let range = w.append_payload(
            BinaryPayloadFileId::new("git_identity_payload.bin", 1),
            &payload,
        )?;
        let mut b = [0; 24];
        b[0] = 1;
        b[2..4].copy_from_slice(&minutes.to_le_bytes());
        b[4..8].copy_from_slice(&range.payload_len.to_le_bytes());
        b[8..16].copy_from_slice(&range.payload_offset.to_le_bytes());
        b[16..24].copy_from_slice(&timestamp.to_le_bytes());
        let ordinal = w.append_record(file("git_identity.bin", 24), &b)?;
        w.commit()?;
        Ok(ordinal)
    }
    pub fn refs(&self) -> StoreResult<Vec<(u32, String, GitRefMappingRecord)>> {
        let read = BinaryDbReadTxn::new(&self.db);
        (0..read.record_count(file("git_ref_mapping.bin", 80))?)
            .map(|ordinal| {
                let b = read.read_record(file("git_ref_mapping.bin", 80), ordinal)?;
                let r = GitRefMappingRecord::decode(&b)?;
                let name = String::from_utf8(read.read_payload(
                    BinaryPayloadFileId::new("git_ref_mapping_payload.bin", 1),
                    u64_at(&b, 24),
                    u32_at(&b, 20),
                )?)
                .map_err(|_| "Git ref name UTF-8")?;
                Ok((ordinal, name, r))
            })
            .collect()
    }
    pub fn append_ref(&self, name: &str, r: GitRefMappingRecord) -> StoreResult<u32> {
        let mut w = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)?;
        for ordinal in 0..w.record_count(file("git_ref_mapping.bin", 80))? {
            let bytes = w.read_record(file("git_ref_mapping.bin", 80), ordinal)?;
            let old = GitRefMappingRecord::decode(&bytes)?;
            if old.generation_index == r.generation_index
                && w.read_payload(
                    BinaryPayloadFileId::new("git_ref_mapping_payload.bin", 1),
                    u64_at(&bytes, 24),
                    u32_at(&bytes, 20),
                )? == name.as_bytes()
            {
                if old == r {
                    return Ok(ordinal);
                }
                // An unbound row freezes source intent before Snapshot writes.
                // A later row may add its verified native binding only.
                let mut frozen = r.clone();
                frozen.meta &= !12;
                frozen.snapshot_index_plus1 = 0;
                frozen.target_index_plus1 = 0;
                if old != frozen || old.meta & 12 != 0 || r.meta & 12 == 0 {
                    return Err(BinaryDbError::corruption("conflicting immutable Git ref"));
                }
            }
        }
        let range = w.append_payload(
            BinaryPayloadFileId::new("git_ref_mapping_payload.bin", 1),
            name.as_bytes(),
        )?;
        let ordinal = w.append_record(
            file("git_ref_mapping.bin", 80),
            &r.encode(range.payload_len, range.payload_offset)?,
        )?;
        let generation = GitGenerationRecord::decode(
            &w.read_record(file("git_generation.bin", 20), r.generation_index)?,
        )?;
        w.append_index_candidate(
            index("git_ref_lookup.idx", 12),
            &ref_key(&generation, name),
            ordinal,
        )?;
        w.commit()?;
        Ok(ordinal)
    }
    pub fn checkpoint_by_hash(
        &self,
        hash: &[u8; 8],
    ) -> StoreResult<Option<GitOperationCheckpointRecord>> {
        let read = BinaryDbReadTxn::new(&self.db);
        let candidates = read.lookup_index(index("git_operation_id.idx", 8), hash)?;
        if candidates.len() > 1 {
            return Err(BinaryDbError::corruption("duplicate operation key"));
        }
        candidates
            .first()
            .map(|ordinal| {
                GitOperationCheckpointRecord::decode(
                    &read.read_record(file("git_operation_checkpoint.bin", 44), *ordinal)?,
                )
            })
            .transpose()
    }
    pub fn generation_is_complete(&self, generation: u32) -> StoreResult<bool> {
        let read = BinaryDbReadTxn::new(&self.db);
        let checkpoint = file("git_operation_checkpoint.bin", 44);
        for i in 0..read.record_count(checkpoint.clone())? {
            let r =
                GitOperationCheckpointRecord::decode(&read.read_record(checkpoint.clone(), i)?)?;
            if r.generation_index == generation && r.status == 1 {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn checkpoints(&self) -> StoreResult<Vec<GitOperationCheckpointRecord>> {
        let read = BinaryDbReadTxn::new(&self.db);
        (0..read.record_count(file("git_operation_checkpoint.bin", 44))?)
            .map(|ordinal| {
                GitOperationCheckpointRecord::decode(
                    &read.read_record(file("git_operation_checkpoint.bin", 44), ordinal)?,
                )
            })
            .collect()
    }
    pub fn checkpoint(&self, r: GitOperationCheckpointRecord) -> StoreResult<()> {
        r.encode()?;
        let mut w = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)?;
        if w.record_count(file("git_commit_mapping.bin", 100))? as usize != self.commits.len() {
            return Err(BinaryDbError::retryable_busy(
                "Git mapping authority changed; reopen before checkpoint",
            ));
        }
        let commits = u32::try_from(
            self.commits
                .values()
                .filter(|(_, row)| row.generation_index == r.generation_index)
                .count(),
        )
        .map_err(|_| "Git checkpoint commit count overflow")?;
        let mut refs = std::collections::BTreeSet::new();
        for ordinal in 0..w.record_count(file("git_ref_mapping.bin", 80))? {
            let bytes = w.read_record(file("git_ref_mapping.bin", 80), ordinal)?;
            if GitRefMappingRecord::decode(&bytes)?.generation_index == r.generation_index {
                refs.insert(w.read_payload(
                    BinaryPayloadFileId::new("git_ref_mapping_payload.bin", 1),
                    u64_at(&bytes, 24),
                    u32_at(&bytes, 20),
                )?);
            }
        }
        if r.next_commit > commits
            || r.next_ref as usize > refs.len()
            || (r.status == 1 && (r.next_commit != commits || r.next_ref as usize != refs.len()))
        {
            return Err(BinaryDbError::corruption(
                "Git checkpoint cursors do not match immutable owners",
            ));
        }
        let key = r.operation_hash;
        let candidates = w.lookup_index(index("git_operation_id.idx", 8), &key)?;
        let mut pending = r.clone();
        pending.status = 0;
        let ordinal = if let Some(ordinal) = candidates.first() {
            let old = GitOperationCheckpointRecord::decode(
                &w.read_record(file("git_operation_checkpoint.bin", 44), *ordinal)?,
            )?;
            if old.generation_index != r.generation_index || old.plan_hash != r.plan_hash {
                return Err(BinaryDbError::corruption("checkpoint intent changed"));
            }
            if old.status == 1 {
                if r.status != 1 || old.next_commit != r.next_commit || old.next_ref != r.next_ref {
                    return Err(BinaryDbError::corruption(
                        "completed Git checkpoint cannot be demoted or changed",
                    ));
                }
                return Ok(());
            }
            w.overwrite_record(
                file("git_operation_checkpoint.bin", 44),
                *ordinal,
                &pending.encode()?,
            )?;
            *ordinal
        } else {
            let ordinal =
                w.append_record(file("git_operation_checkpoint.bin", 44), &pending.encode()?)?;
            w.append_index_candidate(index("git_operation_id.idx", 8), &key, ordinal)?;
            ordinal
        };
        if r.status == 1 {
            w.commit_with_record_marker(file("git_operation_checkpoint.bin", 44), ordinal, 1)?;
        } else {
            w.commit()?;
        }
        Ok(())
    }
}

fn identity_parts(raw: &[u8]) -> StoreResult<(i64, i16, &[u8], &[u8])> {
    let parts = raw.rsplitn(3, |b| *b == b' ').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(BinaryDbError::unsupported("noncanonical Git identity"));
    }
    let timestamp = std::str::from_utf8(parts[1])
        .map_err(|_| "identity timestamp ASCII")?
        .parse::<i64>()
        .map_err(|_| "identity timestamp i64")?;
    let zone = std::str::from_utf8(parts[0]).map_err(|_| "identity timezone ASCII")?;
    if zone.len() != 5
        || !matches!(zone.as_bytes()[0], b'+' | b'-')
        || !zone.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(BinaryDbError::unsupported("noncanonical Git timezone"));
    }
    let minutes = zone[1..3]
        .parse::<i16>()
        .map_err(|_| "identity timezone hour")?
        .checked_mul(60)
        .and_then(|h| {
            zone[3..5]
                .parse::<i16>()
                .ok()
                .and_then(|m| h.checked_add(m))
        })
        .ok_or("timezone overflow")?;
    let minutes = if zone.starts_with('-') {
        -minutes
    } else {
        minutes
    };
    let ident = parts[2];
    let start = ident
        .iter()
        .rposition(|b| *b == b'<')
        .ok_or("identity email absent")?;
    if ident.last() != Some(&b'>') {
        return Err(BinaryDbError::unsupported("identity email closure absent"));
    }
    let name = ident[..start].strip_suffix(b" ").unwrap_or(&ident[..start]);
    Ok((timestamp, minutes, name, &ident[start + 1..ident.len() - 1]))
}

fn ref_key(generation: &GitGenerationRecord, name: &str) -> Vec<u8> {
    let repository = if generation.export {
        generation.target_repository_index_plus1 - 1
    } else {
        generation.source_repository_index
    };
    let mut key = repository.to_le_bytes().to_vec();
    key.extend_from_slice(&crate::line_binary_db::line_name_hash64(name.as_bytes()).to_le_bytes());
    key
}
