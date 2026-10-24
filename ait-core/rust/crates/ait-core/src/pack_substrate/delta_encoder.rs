//! Bounded shifted-byte matches; the persisted Git copy/insert codec is unchanged.
use super::*;

const CANDIDATES_PER_HASH: usize = 8;
const COMPARISON_BUDGET: usize = 2 * 1024 * 1024;

#[derive(Default, Debug)]
pub(super) struct Work {
    pub(super) comparisons: usize,
    pub(super) candidates: usize,
    pub(super) anchors: usize,
    pub(super) exhausted: bool,
}

pub(super) fn build(base: &[u8], target: &[u8]) -> Vec<u8> {
    build_with_budget(base, target, COMPARISON_BUDGET).0
}

fn seed_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |h, b| {
        h.wrapping_mul(257).wrapping_add(u64::from(*b) + 1)
    })
}

pub(super) fn build_with_budget(base: &[u8], target: &[u8], budget: usize) -> (Vec<u8>, Work) {
    let mut work = Work::default();
    let mut out = encode_size_varint(base.len());
    out.extend(encode_size_varint(target.len()));
    let seed = if base.len().max(target.len()) < 8192 {
        8
    } else {
        16
    };
    // Policy bounds both indexing and target scanning. Larger objects use valid
    // literals; callers retain the existing size and compressed-archive gates.
    if base.len().min(target.len()) < seed || base.len().max(target.len()) > MAX_DELTA_BLOB_BYTES {
        append_insert_instructions(&mut out, target);
        return (out, work);
    }
    let mut index = BTreeMap::<u64, Vec<usize>>::new();
    for offset in (0..=base.len() - seed).step_by(seed) {
        let offsets = index
            .entry(seed_hash(&base[offset..offset + seed]))
            .or_default();
        if offsets.len() < CANDIDATES_PER_HASH {
            offsets.push(offset);
            work.anchors += 1;
        }
    }
    let power = (0..seed - 1).fold(1u64, |p, _| p.wrapping_mul(257));
    let mut at = 0usize;
    let mut literal = 0usize;
    let mut hash = seed_hash(&target[..seed]);
    'search: while at + seed <= target.len() {
        let mut best: Option<(usize, usize, usize)> = None;
        if let Some(offsets) = index.get(&hash) {
            for &offset in offsets {
                work.candidates += 1;
                let mut len = 0;
                while offset + len < base.len() && at + len < target.len() {
                    if work.comparisons == budget {
                        work.exhausted = true;
                        break 'search;
                    }
                    work.comparisons += 1;
                    if base[offset + len] != target[at + len] {
                        break;
                    }
                    len += 1;
                }
                // Hash collisions never emit copies without exact byte equality.
                if len < seed {
                    continue;
                }
                let mut left = 0;
                while offset > left && at - left > literal {
                    if work.comparisons == budget {
                        work.exhausted = true;
                        break 'search;
                    }
                    work.comparisons += 1;
                    if base[offset - left - 1] != target[at - left - 1] {
                        break;
                    }
                    left += 1;
                }
                let candidate = (offset - left, at - left, len + left);
                if best.is_none_or(|b| candidate.2 > b.2) {
                    best = Some(candidate);
                }
            }
        }
        if let Some((offset, start, len)) = best {
            append_insert_instructions(&mut out, &target[literal..start]);
            append_copy_instructions(&mut out, offset, len);
            at = start + len;
            literal = at;
            if at + seed <= target.len() {
                hash = seed_hash(&target[at..at + seed]);
            }
        } else {
            if at + seed < target.len() {
                hash = hash
                    .wrapping_sub((u64::from(target[at]) + 1).wrapping_mul(power))
                    .wrapping_mul(257)
                    .wrapping_add(u64::from(target[at + seed]) + 1);
            }
            at += 1;
        }
    }
    // Exhaustion keeps completed instructions and emits the untouched suffix.
    // There is no unbounded secondary diff or retry.
    append_insert_instructions(&mut out, &target[literal..]);
    (out, work)
}
