use super::*;

/// Transient physical write plan. Logical member ordinals remain unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectPackChunkLayout {
    pub(crate) chunks: Vec<Vec<usize>>,
}
impl ObjectPackChunkLayout {
    pub(crate) fn new(
        sizes: &[usize],
        priorities: Option<&[bool]>,
        target: usize,
    ) -> Result<Self, String> {
        if priorities.is_some_and(|p| p.len() != sizes.len()) {
            return Err("pack layout requires one classification per member".into());
        }
        let mut order = (0..sizes.len()).collect::<Vec<_>>();
        if let Some(classes) = priorities {
            order.sort_by_key(|i| (!classes[*i], *i));
        }
        let target = target.max(1);
        let mut chunks = Vec::new();
        let mut pending = Vec::new();
        let mut bytes = 0usize;
        let mut previous = None;
        for i in order {
            let class = priorities.map(|p| p[i]);
            if !pending.is_empty() && (previous != class || bytes.saturating_add(sizes[i]) > target)
            {
                chunks.push(std::mem::take(&mut pending));
                bytes = 0;
            }
            pending.push(i);
            bytes = bytes.saturating_add(sizes[i]);
            previous = class;
            if bytes >= target {
                chunks.push(std::mem::take(&mut pending));
                bytes = 0;
            }
        }
        if !pending.is_empty() {
            chunks.push(pending);
        }
        Ok(Self { chunks })
    }
    pub(crate) fn for_members(
        members: &[ObjectPackWriteMember],
        priority_ids: Option<&BTreeSet<String>>,
    ) -> Self {
        let sizes = members.iter().map(|m| m.data.len()).collect::<Vec<_>>();
        let classes = priority_ids.map(|ids| {
            members
                .iter()
                .map(|m| ids.contains(&m.blob_id))
                .collect::<Vec<_>>()
        });
        Self::new(&sizes, classes.as_deref(), ZSTD_CHUNKED_DEFAULT_CHUNK_BYTES)
            .expect("one size/class per typed member")
    }
    pub(crate) fn validate(&self, count: usize) -> Result<(), String> {
        let mut seen = vec![false; count];
        for chunk in &self.chunks {
            if chunk.is_empty() {
                return Err("pack layout has empty member group".into());
            }
            for i in chunk {
                if *i >= count || seen[*i] {
                    return Err("pack layout repeats or exceeds member ordinal".into());
                }
                seen[*i] = true;
            }
        }
        if seen.iter().any(|v| !*v) {
            return Err("pack layout omits member ordinal".into());
        }
        Ok(())
    }
    /// Canonical geometry keeps its existing identity; changed physical layout has its own domain.
    pub(crate) fn extend_identity_seed(
        &self,
        members: &[ObjectPackWriteMember],
        seed: &mut Vec<u8>,
    ) {
        if *self == Self::for_members(members, None) {
            return;
        }
        seed.extend_from_slice(b"\nnative-object-pack/chunk-layout/v1\0");
        seed.extend_from_slice(&(self.chunks.len() as u64).to_le_bytes());
        for chunk in &self.chunks {
            seed.extend_from_slice(&(chunk.len() as u64).to_le_bytes());
            for i in chunk {
                seed.extend_from_slice(&(*i as u64).to_le_bytes());
            }
        }
    }
}
