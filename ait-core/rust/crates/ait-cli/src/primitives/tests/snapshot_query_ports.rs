use super::*;
use crate::primitives::snapshot::{
    snapshot_ancestry_with_snapshot_store, snapshot_is_ancestor_query_with_snapshot_store,
    snapshot_merge_base_query_with_snapshot_store,
};
use ait_core::snapshot_store::{
    SnapshotParentLink, SnapshotParentLinkPage, SnapshotRecord, SnapshotStore, SnapshotStoreResult,
};
use std::cell::Cell;

#[derive(Default)]
struct MetadataOnlyDagStore {
    parents: BTreeMap<String, Vec<String>>,
    page_reads: Cell<usize>,
}

impl MetadataOnlyDagStore {
    fn from_parents(parents: BTreeMap<String, Vec<String>>) -> Self {
        Self {
            parents,
            page_reads: Cell::new(0),
        }
    }

    fn diamond() -> Self {
        Self::from_parents(BTreeMap::from([
            ("SNP-ROOT".to_string(), vec![]),
            ("SNP-LEFT".to_string(), vec!["SNP-ROOT".to_string()]),
            ("SNP-RIGHT".to_string(), vec!["SNP-ROOT".to_string()]),
            (
                "SNP-MERGE".to_string(),
                vec!["SNP-LEFT".to_string(), "SNP-RIGHT".to_string()],
            ),
        ]))
    }

    fn link(snapshot_id: &str, parents: &[String]) -> SnapshotParentLink {
        SnapshotParentLink {
            snapshot_id: snapshot_id.to_string(),
            parent_snapshot_ids: parents.to_vec(),
            primary_parent_snapshot_id: parents.first().cloned(),
            parent_snapshot_id: parents.first().cloned(),
        }
    }
}

impl SnapshotStore for MetadataOnlyDagStore {
    fn snapshot_exists(&self, snapshot_id: &str) -> SnapshotStoreResult<bool> {
        Ok(self.parents.contains_key(snapshot_id))
    }

    fn snapshot_parent_link(
        &self,
        snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<SnapshotParentLink>> {
        Ok(self
            .parents
            .get(snapshot_id)
            .map(|parents| Self::link(snapshot_id, parents)))
    }

    fn snapshot_parent_link_page(
        &self,
        cursor: usize,
        limit: usize,
    ) -> SnapshotStoreResult<SnapshotParentLinkPage> {
        self.page_reads.set(self.page_reads.get() + 1);
        if limit == 0 || cursor > self.parents.len() {
            return Err("invalid metadata page request".to_string());
        }
        let links = self
            .parents
            .iter()
            .skip(cursor)
            .take(limit)
            .map(|(snapshot_id, parents)| Self::link(snapshot_id, parents))
            .collect::<Vec<_>>();
        let next_cursor = cursor.saturating_add(limit).min(self.parents.len());
        Ok(SnapshotParentLinkPage {
            links,
            next_cursor: (next_cursor < self.parents.len()).then_some(next_cursor),
        })
    }

    fn snapshot_by_id(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<SnapshotRecord>> {
        panic!("ancestry queries must not read Snapshot payload records")
    }

    fn list_line_snapshots(&self) -> SnapshotStoreResult<Vec<SnapshotRecord>> {
        panic!("ancestry queries must not expand the Snapshot list")
    }

    fn snapshot_total_bytes(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<i64>> {
        panic!("ancestry queries must not read content totals")
    }

    fn snapshot_root_tree_pack_id(
        &self,
        _snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<String>> {
        panic!("ancestry queries must not read tree packs")
    }

    fn snapshot_kind(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        panic!("ancestry queries must not read Snapshot payload metadata")
    }

    fn snapshot_chain(&self, _snapshot_id: &str) -> SnapshotStoreResult<Vec<String>> {
        panic!("ancestry queries must use the shared DAG engine")
    }

    fn set_snapshot_kind(
        &self,
        _snapshot_id: &str,
        _snapshot_kind: &str,
    ) -> SnapshotStoreResult<usize> {
        panic!("ancestry queries are read-only")
    }
}

#[test]
fn ancestry_json_contract_is_stable_for_diamond_first_parent_and_bounds() {
    let store = MetadataOnlyDagStore::diamond();
    let ancestors = snapshot_ancestry_with_snapshot_store(
        &store,
        "SNP-MERGE",
        SnapshotAncestryDirection::Ancestors,
        false,
        10,
        10,
    )
    .unwrap();
    assert_eq!(ancestors["contract"], json!("snapshot-ancestry/v1"));
    assert_eq!(ancestors["direction"], json!("ancestors"));
    assert_eq!(ancestors["parent_mode"], json!("all_parents"));
    assert_eq!(ancestors["result_count"], json!(3));
    assert_eq!(ancestors["truncated"], json!(false));
    assert_eq!(
        ancestors["snapshots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["snapshot_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["SNP-ROOT", "SNP-LEFT", "SNP-RIGHT"]
    );

    let first_parent = snapshot_ancestry_with_snapshot_store(
        &store,
        "SNP-MERGE",
        SnapshotAncestryDirection::Ancestors,
        true,
        10,
        10,
    )
    .unwrap();
    assert_eq!(first_parent["parent_mode"], json!("first_parent"));
    assert_eq!(first_parent["result_count"], json!(2));

    let descendants = snapshot_ancestry_with_snapshot_store(
        &store,
        "SNP-RIGHT",
        SnapshotAncestryDirection::Descendants,
        false,
        10,
        10,
    )
    .unwrap();
    assert_eq!(
        descendants["snapshots"][0]["snapshot_id"],
        json!("SNP-MERGE")
    );
    assert!(store.page_reads.get() > 0);

    let first_parent_descendants = snapshot_ancestry_with_snapshot_store(
        &store,
        "SNP-RIGHT",
        SnapshotAncestryDirection::Descendants,
        true,
        10,
        10,
    )
    .unwrap();
    assert_eq!(first_parent_descendants["result_count"], json!(0));

    let bounded = snapshot_ancestry_with_snapshot_store(
        &store,
        "SNP-ROOT",
        SnapshotAncestryDirection::Descendants,
        false,
        10,
        1,
    )
    .unwrap();
    assert_eq!(bounded["result_count"], json!(1));
    assert_eq!(bounded["snapshots"][0]["snapshot_id"], json!("SNP-LEFT"));
    assert_eq!(bounded["truncated"], json!(true));
}

#[test]
fn is_ancestor_and_merge_base_json_cover_false_unknown_and_criss_cross() {
    let store = MetadataOnlyDagStore::diamond();
    let (true_payload, is_ancestor) =
        snapshot_is_ancestor_query_with_snapshot_store(&store, "SNP-RIGHT", "SNP-MERGE").unwrap();
    assert!(is_ancestor);
    assert_eq!(true_payload["contract"], json!("snapshot-is-ancestor/v1"));
    assert_eq!(true_payload["distance"], json!(1));

    let (false_payload, is_ancestor) =
        snapshot_is_ancestor_query_with_snapshot_store(&store, "SNP-LEFT", "SNP-RIGHT").unwrap();
    assert!(!is_ancestor);
    assert_eq!(false_payload["is_ancestor"], json!(false));
    assert_eq!(false_payload["distance"], JsonValue::Null);
    assert!(
        snapshot_is_ancestor_query_with_snapshot_store(&store, "SNP-MISSING", "SNP-MERGE")
            .unwrap_err()
            .contains("Unknown snapshot: SNP-MISSING")
    );

    let criss_cross = MetadataOnlyDagStore::from_parents(BTreeMap::from([
        ("SNP-ROOT".to_string(), vec![]),
        ("SNP-A1".to_string(), vec!["SNP-ROOT".to_string()]),
        ("SNP-B1".to_string(), vec!["SNP-ROOT".to_string()]),
        (
            "SNP-A2".to_string(),
            vec!["SNP-A1".to_string(), "SNP-B1".to_string()],
        ),
        (
            "SNP-B2".to_string(),
            vec!["SNP-B1".to_string(), "SNP-A1".to_string()],
        ),
        ("SNP-LEFT".to_string(), vec!["SNP-A2".to_string()]),
        ("SNP-RIGHT".to_string(), vec!["SNP-B2".to_string()]),
    ]));
    let (one, found) =
        snapshot_merge_base_query_with_snapshot_store(&criss_cross, "SNP-LEFT", "SNP-RIGHT", false)
            .unwrap();
    assert!(found);
    assert_eq!(one["contract"], json!("snapshot-merge-base/v1"));
    assert_eq!(one["merge_base_snapshot_ids"], json!(["SNP-A1"]));
    assert_eq!(one["available_merge_base_count"], json!(2));
    assert_eq!(one["ambiguous"], json!(true));

    let (all, found) =
        snapshot_merge_base_query_with_snapshot_store(&criss_cross, "SNP-LEFT", "SNP-RIGHT", true)
            .unwrap();
    assert!(found);
    assert_eq!(all["merge_base_snapshot_ids"], json!(["SNP-A1", "SNP-B1"]));
}
