use super::*;

pub(super) fn index_candidates<B>(
    read: &BinaryDbReadTxn<'_, B>,
    index: crate::binary_db::BinaryIndexId,
    key: &[u8],
) -> StoreResult<Vec<u32>>
where
    B: BinaryDb,
{
    let mut candidates = read.lookup_index(index, key)?;
    candidates.sort_unstable();
    candidates.dedup();
    candidates.reverse();
    Ok(candidates)
}
