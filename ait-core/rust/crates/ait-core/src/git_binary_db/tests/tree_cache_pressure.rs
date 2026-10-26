use super::*;
use crate::content_binary_db::{BinaryDbTreeReadCache, BinaryTreeEntryView};

fn entry_bytes(rows: &[BinaryTreeEntryView]) -> usize {
    128 + rows
        .iter()
        .map(|e| {
            std::mem::size_of::<BinaryTreeEntryView>()
                + e.entry_name.capacity()
                + e.entry_type.capacity()
                + e.target_id.capacity()
                + e.mode.as_ref().map_or(0, String::capacity)
        })
        .sum::<usize>()
}
type TreeRowIdentity = (u32, String, String, String, Option<u64>, Option<String>);
fn identity(rows: &[BinaryTreeEntryView]) -> Vec<TreeRowIdentity> {
    rows.iter()
        .map(|e| {
            (
                e.entry_ordinal,
                e.entry_name.clone(),
                e.entry_type.clone(),
                e.target_id.clone(),
                e.size_bytes,
                e.mode.clone(),
            )
        })
        .collect()
}
fn sequence() -> Vec<usize> {
    vec![
        0, 1, 0, 2, 0, 3, 0, 1, 0, 2, 0, 3, 0, 1, 0, 2, 0, 3, 0, 1, 0, 2, 0, 3,
    ]
}
fn run(
    c: &LocalContentBinaryDb<1>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    ids: &[String],
    expected: &[Vec<BinaryTreeEntryView>],
    entries: usize,
    archives: usize,
    legacy: bool,
) -> serde_json::Value {
    let mut cache = BinaryDbTreeReadCache::policy_budgets_for_test(entries, archives, legacy);
    crate::binary_db::read_profile::reset();
    let started = std::time::Instant::now();
    let mut peak = 0;
    for i in sequence() {
        let rows = c
            .trees()
            .list_tree_entry_views_with_cache(read, &ids[i], &mut cache)
            .unwrap();
        assert_eq!(identity(&rows), identity(&expected[i]));
        peak = peak.max(cache.retained_payload_bytes_for_test());
        assert!(cache.retained_payload_bytes_for_test() <= entries + archives);
    }
    cache.verify_admission_stamps().unwrap();
    serde_json::json!({"policy":if legacy{"wholesale_clear"}else{"item_lru"},"requests":24,"entry_budget":entries,"archive_budget":archives,"peak_retained_payload_bytes":peak,"all_entry_fields_exact":true,"seconds":started.elapsed().as_secs_f64(),"counters":crate::binary_db::read_profile::snapshot()})
}
#[test]
fn bounded_lru_keeps_hot_tree_when_working_set_exceeds_capacity() {
    let tmp = tempdir().unwrap();
    let workspace = tmp.path().join("repo");
    let c = content(&tmp.path().join("db"), &workspace);
    let bodies = (0..128u32)
        .map(|i| i.to_le_bytes().repeat(16))
        .collect::<Vec<_>>();
    let indexes = c
        .ensure_blob_bytes_batch(&bodies.iter().map(Vec::as_slice).collect::<Vec<_>>())
        .unwrap();
    let mut groups = Vec::new();
    for variant in 0..4 {
        let mut files = (0..128)
            .map(|i| (format!("file-{i:04}.rs"), indexes[i], 0o644))
            .collect::<Vec<_>>();
        files[0].1 = indexes[variant];
        groups.push(files);
    }
    let roots = c
        .ensure_raw_trees(&groups, &mut Default::default())
        .unwrap();
    let ids = roots.iter().map(|r| r.tree_id.clone()).collect::<Vec<_>>();
    let read = BinaryDbReadTxn::new(c.db());
    let mut prep = BinaryDbTreeReadCache::default();
    let expected = ids
        .iter()
        .map(|id| {
            c.trees()
                .list_tree_entry_views_with_cache(&read, id, &mut prep)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let budget = expected.iter().map(|r| entry_bytes(r)).max().unwrap() * 2 + 1024;
    assert!(expected.iter().map(|r| entry_bytes(r)).sum::<usize>() > budget);
    drop(prep);
    let before = run(&c, &read, &ids, &expected, budget, 2 * 1024 * 1024, true);
    let after = run(&c, &read, &ids, &expected, budget, 2 * 1024 * 1024, false);
    assert!(
        after["counters"]["tree_entry_hits"][0].as_u64().unwrap()
            > before["counters"]["tree_entry_hits"][0]
                .as_u64()
                .unwrap_or(0)
    );
    assert!(
        after["counters"]["tree_entry_decodes"][0].as_u64().unwrap()
            < before["counters"]["tree_entry_decodes"][0]
                .as_u64()
                .unwrap()
    );
    // Reader eviction must never discard immutable file stamps.
    let mut cache = BinaryDbTreeReadCache::policy_budgets_for_test(0, 1, false);
    c.trees()
        .list_tree_entry_views_with_cache(&read, &ids[0], &mut cache)
        .unwrap();
    assert_eq!(cache.archive_count(), 0);
    for p in fs::read_dir(workspace.join(".ait/objects/tree-packs"))
        .unwrap()
        .map(|e| e.unwrap().path())
    {
        let mut bytes = fs::read(&p).unwrap();
        bytes[0] ^= 0x80;
        fs::write(&p, bytes).unwrap();
    }
    assert!(cache.verify_admission_stamps().is_err());
    println!(
        "TREE_CACHE_PRESSURE {}",
        serde_json::json!({"status":"passed","before":before,"after":after,"evicted_pack_same_length_mutation_rejected":true})
    );
}
