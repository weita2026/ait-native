use super::*;
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};

type Key = (u32, u8, [u8; 20]);
const MAX_OBJECTS: usize = 1_048_576;
const MAX_EDGES: usize = 8_388_608;
const MAX_LABELS: usize = 4_194_304;

#[derive(Clone, Eq, PartialEq)]
struct FileStamp {
    size: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
fn stamp(path: &Path) -> Result<FileStamp, String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.file_type().is_file() {
        return Err("staged Git object is not a direct file".into());
    }
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(FileStamp {
        size: m.len(),
        modified: m.modified().map_err(|e| e.to_string())?,
        #[cfg(unix)]
        identity: (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()),
    })
}
struct StagedProof {
    body: [u8; 32],
    kind: GitObjectKind,
    stamp: FileStamp,
}
/// OID validation always hashes the input body. A bounded proof avoids reading
/// and recompressing an identical, unchanged object already staged here.
pub(super) struct StagedObjects<'a> {
    stage: &'a Path,
    proofs: RefCell<BTreeMap<[u8; 20], StagedProof>>,
    pub(super) compressions: Cell<usize>,
}
impl<'a> StagedObjects<'a> {
    pub(super) fn new(stage: &'a Path) -> Self {
        Self {
            stage,
            proofs: RefCell::new(BTreeMap::new()),
            compressions: Cell::new(0),
        }
    }
    pub(super) fn write(&self, kind: GitObjectKind, body: &[u8]) -> Result<[u8; 20], String> {
        let oid = git_object_id(kind, body);
        let hash: [u8; 32] = Sha256::digest(body).into();
        let text = hex(&oid);
        let path = self.stage.join("objects").join(&text[..2]).join(&text[2..]);
        if let Some(proof) = self.proofs.borrow().get(&oid) {
            if proof.body != hash || proof.kind != kind {
                return Err("staged Git object differs at the same OID".into());
            }
            if stamp(&path)? == proof.stamp {
                return Ok(oid);
            }
        }
        let compress = !path.exists();
        let result = write_object(self.stage, kind, body)?;
        if compress {
            self.compressions.set(self.compressions.get() + 1);
        }
        let mut proofs = self.proofs.borrow_mut();
        if proofs.len() >= 65_536 {
            proofs.pop_first();
        }
        proofs.insert(
            oid,
            StagedProof {
                body: hash,
                kind,
                stamp: stamp(&path)?,
            },
        );
        Ok(result)
    }
    fn discard_tag(&self, oid: &[u8; 20]) -> Result<(), String> {
        let text = hex(oid);
        let path = self.stage.join("objects").join(&text[..2]).join(&text[2..]);
        if let Some(proof) = self.proofs.borrow().get(oid) {
            if proof.kind != GitObjectKind::Tag || stamp(&path)? != proof.stamp {
                return Err("staged Tag changed before pruning".into());
            }
        }
        fs::remove_file(path).map_err(|e| e.to_string())?;
        self.proofs.borrow_mut().remove(oid);
        Ok(())
    }
}

struct Node {
    record: GitObjectRecord,
    children: Option<Vec<Key>>,
    labels: BTreeSet<u32>,
}
struct TagProof {
    target: [u8; 20],
    message: Option<(usize, [u8; 32])>,
}
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub(super) struct TraversalStats {
    pub visits: usize,
    pub edges: usize,
    pub body_reads: usize,
}
/// Namespaces remain distinct even when they contain equal SHA-1s. Bodies are
/// admitted once per namespace; only small graph/proof metadata survives batches.
pub(super) struct SourcePlan<'a> {
    content: &'a LocalContentBinaryDb<1>,
    reader: GitObjectBatchReader<'a>,
    writer: &'a StagedObjects<'a>,
    nodes: BTreeMap<Key, Node>,
    tags: BTreeMap<Key, TagProof>,
    roots: BTreeMap<Key, BTreeSet<u32>>,
    stats: TraversalStats,
    queued_edges: usize,
}
impl<'a> SourcePlan<'a> {
    pub(super) fn new(
        objects: &'a LocalGitObjectStore,
        content: &'a LocalContentBinaryDb<1>,
        writer: &'a StagedObjects<'a>,
    ) -> Result<Self, String> {
        let mut reader = objects.batch_reader();
        reader.verify()?;
        Ok(Self {
            content,
            reader,
            writer,
            nodes: BTreeMap::new(),
            tags: BTreeMap::new(),
            roots: BTreeMap::new(),
            stats: TraversalStats::default(),
            queued_edges: 0,
        })
    }
    fn record(&mut self, key: Key) -> Result<GitObjectRecord, String> {
        if let Some(node) = self.nodes.get(&key) {
            return Ok(node.record.clone());
        }
        if self.nodes.len() >= MAX_OBJECTS {
            return Err("Git export object graph budget exceeded".into());
        }
        let record = self
            .reader
            .get_record_batch(key.0, key.1, &[key.2])?
            .pop()
            .ok_or("Git export metadata absent")?;
        self.nodes.insert(
            key,
            Node {
                record: record.clone(),
                children: None,
                labels: BTreeSet::new(),
            },
        );
        Ok(record)
    }
    fn tag(&mut self, key: Key) -> Result<(), String> {
        if self.tags.contains_key(&key) {
            return Ok(());
        }
        let record = self.record(key)?;
        if record.kind != GitObjectKind::Tag {
            return Err("native Tag object kind differs".into());
        }
        let (verified, body) = self
            .reader
            .get_batch(key.0, key.1, &[key.2])?
            .pop()
            .ok_or("Tag body absent")?;
        self.stats.body_reads += 1;
        if verified != record {
            return Err("native Tag metadata changed".into());
        }
        let children = children(record.kind, &body)?;
        self.queued_edges = self
            .queued_edges
            .checked_add(children.len())
            .ok_or("Git export edge budget exceeded")?;
        if self.queued_edges > MAX_EDGES {
            return Err("Git export edge budget exceeded".into());
        }
        let message = String::from_utf8_lossy(split_object_message(&body))
            .lines()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .map(|s| (s.len(), Sha256::digest(s.as_bytes()).into()));
        self.writer.write(record.kind, &body)?;
        self.nodes.get_mut(&key).unwrap().children =
            Some(children.iter().map(|oid| (key.0, key.1, *oid)).collect());
        self.tags.insert(
            key,
            TagProof {
                target: children[0],
                message,
            },
        );
        Ok(())
    }
    pub(super) fn peel_commit(
        &mut self,
        repository: u32,
        mut oid: [u8; 20],
    ) -> Result<[u8; 20], String> {
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(oid) {
                return Err("native Git Tag cycle".into());
            }
            let key = (repository, 1, oid);
            match self.record(key)?.kind {
                GitObjectKind::Commit => return Ok(oid),
                GitObjectKind::Tag => {
                    self.tag(key)?;
                    oid = self.tags[&key].target;
                }
                _ => return Err("bound native Tag target is not a Commit".into()),
            }
        }
    }
    pub(super) fn tag_message_matches(
        &mut self,
        repository: u32,
        oid: [u8; 20],
        name: &str,
        message: Option<&str>,
    ) -> Result<bool, String> {
        let key = (repository, 1, oid);
        let record = self.record(key)?;
        let expected = if record.kind == GitObjectKind::Tag {
            self.tag(key)?;
            self.tags[&key].message
        } else {
            None
        };
        let fallback = format!("Imported Git tag {name}");
        let expected = expected
            .unwrap_or_else(|| (fallback.len(), Sha256::digest(fallback.as_bytes()).into()));
        Ok(message
            .is_some_and(|s| (s.len(), <[u8; 32]>::from(Sha256::digest(s.as_bytes()))) == expected))
    }
    pub(super) fn root(&mut self, repository: u32, oid: [u8; 20], generation: Option<u32>) {
        let labels = self.roots.entry((repository, 1, oid)).or_default();
        if let Some(generation) = generation {
            labels.insert(generation);
        }
    }
    pub(super) fn finish(
        &mut self,
        history: &GitHistoryStore,
        prepared: &mut BTreeMap<[u8; 20], PreparedCommit>,
        selected_roots: &[[u8; 20]],
    ) -> Result<TraversalStats, String> {
        let mut todo = self.roots.keys().copied().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        let read = BinaryDbReadTxn::new(self.content.db());
        while let Some(first) = todo.pop() {
            if !seen.insert(first) {
                continue;
            }
            let mut batch = vec![first];
            // Group bounded work by namespace, regardless of which ref queued it.
            while batch.len() < 128
                && todo
                    .last()
                    .is_some_and(|k| k.0 == first.0 && k.1 == first.1)
            {
                let key = todo.pop().unwrap();
                if seen.insert(key) {
                    batch.push(key);
                }
            }
            let missing = batch
                .iter()
                .filter(|key| !self.nodes.contains_key(key))
                .map(|key| key.2)
                .collect::<Vec<_>>();
            if self.nodes.len().saturating_add(missing.len()) > MAX_OBJECTS {
                return Err("Git export object graph budget exceeded".into());
            }
            if !missing.is_empty() {
                for record in self.reader.get_record_batch(first.0, first.1, &missing)? {
                    self.nodes.insert(
                        (first.0, first.1, record.object_id),
                        Node {
                            record,
                            children: None,
                            labels: BTreeSet::new(),
                        },
                    );
                }
            }
            let mut bodies = Vec::new();
            let mut batch_bytes = 0u64;
            for key in batch {
                let record = self.record(key)?;
                if let Some(children) = &self.nodes[&key].children {
                    todo.extend(children);
                    continue;
                }
                let size = self
                    .content
                    .blobs()
                    .blob_view_at(&read, record.content_blob_index)?
                    .size_bytes;
                if !bodies.is_empty() && batch_bytes.saturating_add(size) > 8 * 1024 * 1024 {
                    seen.remove(&key);
                    todo.push(key);
                    continue;
                }
                batch_bytes = batch_bytes.saturating_add(size);
                bodies.push(key.2);
            }
            if !bodies.is_empty() {
                for (record, body) in self.reader.get_batch(first.0, first.1, &bodies)? {
                    let key = (first.0, first.1, record.object_id);
                    if record != self.nodes[&key].record {
                        return Err("Git export source metadata changed".into());
                    }
                    self.stats.body_reads += 1;
                    let edges = children(record.kind, &body)?
                        .into_iter()
                        .map(|oid| (first.0, first.1, oid))
                        .collect::<Vec<_>>();
                    self.queued_edges = self
                        .queued_edges
                        .checked_add(edges.len())
                        .ok_or("Git export edge budget exceeded")?;
                    if self.queued_edges > MAX_EDGES {
                        return Err("Git export edge budget exceeded".into());
                    }
                    todo.extend(&edges);
                    if self.writer.write(record.kind, &body)? != record.object_id {
                        return Err("native export object identity mismatch".into());
                    }
                    self.nodes.get_mut(&key).unwrap().children = Some(edges);
                }
            }
        }
        // A single topological propagation keeps every generation's exact
        // closure. Roots sharing a generation add no ancestry work or labels.
        let mut incoming = seen
            .iter()
            .map(|k| (*k, 0usize))
            .collect::<BTreeMap<_, _>>();
        for key in &seen {
            for child in self.nodes[key]
                .children
                .as_ref()
                .ok_or("unread source node")?
            {
                *incoming
                    .get_mut(child)
                    .ok_or("source child was not admitted")? += 1;
                self.stats.edges += 1;
                if self.stats.edges > MAX_EDGES {
                    return Err("Git export edge budget exceeded".into());
                }
            }
        }
        let mut label_count = 0usize;
        for (key, labels) in &self.roots {
            self.nodes.get_mut(key).unwrap().labels = labels.clone();
            label_count += labels.len();
            if label_count > MAX_LABELS {
                return Err("Git export generation proof budget exceeded".into());
            }
        }
        let mut ready = incoming
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(k, _)| *k)
            .collect::<Vec<_>>();
        while let Some(key) = ready.pop() {
            self.stats.visits += 1;
            let node = &self.nodes[&key];
            let labels = node.labels.clone();
            let edges = node.children.clone().unwrap();
            for child in edges {
                // Only Commit ancestry carries CommitMapping membership.
                if self.nodes[&child].record.kind == GitObjectKind::Commit {
                    let node = self.nodes.get_mut(&child).unwrap();
                    let before = node.labels.len();
                    node.labels.extend(&labels);
                    label_count += node.labels.len() - before;
                    if label_count > MAX_LABELS {
                        return Err("Git export generation proof budget exceeded".into());
                    }
                }
                let count = incoming.get_mut(&child).unwrap();
                *count -= 1;
                if *count == 0 {
                    ready.push(child);
                }
            }
        }
        if self.stats.visits != seen.len() {
            return Err("native Git object DAG cycle".into());
        }
        let generations = history.generations()?;
        for record in history.commit_mappings() {
            let generation = generations
                .get(record.generation_index as usize)
                .ok_or("source generation absent")?;
            let key = (generation.source_repository_index, 1, record.object_id);
            if !seen.contains(&key) || !self.nodes[&key].labels.contains(&record.generation_index) {
                continue;
            }
            if let Some(old) = prepared.get(&record.object_id) {
                if old.record.snapshot_index != record.snapshot_index
                    || old.document_set != Some(record.document_set_tree_index)
                {
                    return Err("source alias has conflicting native dependencies".into());
                }
            } else {
                prepared.insert(
                    record.object_id,
                    PreparedCommit {
                        document_set: Some(record.document_set_tree_index),
                        record,
                        native: false,
                    },
                );
            }
        }
        // Peeling may inspect a bound Tag on an unselected branch. Its small
        // staged proof must not become an extra fsck root or export dependency.
        let keep = seen
            .iter()
            .map(|key| key.2)
            .chain(selected_roots.iter().copied())
            .collect::<BTreeSet<_>>();
        let discard = self
            .tags
            .keys()
            .map(|key| key.2)
            .filter(|oid| !keep.contains(oid))
            .collect::<BTreeSet<_>>();
        for oid in discard {
            self.writer.discard_tag(&oid)?;
        }
        self.reader.verify()?;
        Ok(self.stats)
    }
}
