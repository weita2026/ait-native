use super::*;

fn object_pack_row(pack_format: PackFormatKind) -> RepositoryObjectPackInventoryRow {
    RepositoryObjectPackInventoryRow {
        pack_id: "OPK-1".to_string(),
        repo_name: Some("ait-core".to_string()),
        repo_id: Some("repo-1".to_string()),
        status: "ready".to_string(),
        pack_format,
        member_count: 0,
        total_bytes: 0,
        pack_path: ".ait/objects/packs/OPK-1.zstpack".to_string(),
        pack_index_entry_name: "zstd-chunked-object-index".to_string(),
        pack_index_checksum: "sha256:object-index".to_string(),
        created_at: "2026-07-06T00:00:00Z".to_string(),
        embedded_index: ObjectPackIndexInventory {
            pack_id: "OPK-1".to_string(),
            pack_format,
            member_count: 0,
            total_bytes: 0,
            entries: Vec::new(),
        },
    }
}

fn tree_pack_row(pack_format: TreePackFormatKind) -> RepositoryTreePackInventoryRow {
    RepositoryTreePackInventoryRow {
        pack_id: "TPK-1".to_string(),
        repo_name: Some("ait-core".to_string()),
        repo_id: Some("repo-1".to_string()),
        status: "ready".to_string(),
        pack_format,
        tree_count: 0,
        total_bytes: 0,
        pack_path: ".ait/objects/tree-packs/TPK-1.zstpack".to_string(),
        pack_index_entry_name: "zstd-chunked-tree-index".to_string(),
        pack_index_checksum: "sha256:tree-index".to_string(),
        created_at: "2026-07-06T00:00:00Z".to_string(),
        embedded_index: TreePackIndexInventory {
            pack_id: "TPK-1".to_string(),
            pack_format,
            tree_count: 0,
            total_bytes: 0,
            trees: Vec::new(),
        },
    }
}

fn full_zstd_inventory() -> RepositoryPackInventory {
    let mut object_pack = object_pack_row(PackFormatKind::ZstdChunkedV1);
    object_pack.member_count = 1;
    object_pack.total_bytes = 32;
    object_pack.embedded_index.member_count = 1;
    object_pack.embedded_index.total_bytes = 32;
    object_pack
        .embedded_index
        .entries
        .push(ObjectPackIndexEntryInventory {
            entry_name: "objects/BLOB-1".to_string(),
            blob_id: "BLOB-1".to_string(),
            entry_type: "full".to_string(),
            checksum: "sha256-blob-1".to_string(),
            base_blob_id: None,
            chain_depth: 0,
        });

    let mut tree_pack = tree_pack_row(TreePackFormatKind::ZstdChunkedTreeV1);
    tree_pack.tree_count = 1;
    tree_pack.total_bytes = 64;
    tree_pack.embedded_index.tree_count = 1;
    tree_pack.embedded_index.total_bytes = 64;
    tree_pack
        .embedded_index
        .trees
        .push(TreePackIndexEntryInventory {
            tree_id: "TREE-1".to_string(),
            entry_ordinal: 0,
            entry_count: 1,
            checksum: "sha256-tree-1".to_string(),
        });

    RepositoryPackInventory {
        repo_name: "ait-core".to_string(),
        object_packs: vec![object_pack],
        tree_packs: vec![tree_pack],
        blob_locators: vec![RepositoryBlobLocatorInventoryRow {
            blob_id: "BLOB-1".to_string(),
            sha256: "sha256-blob-1".to_string(),
            size_bytes: 32,
            pack_id: "OPK-1".to_string(),
            pack_entry_name: "objects/BLOB-1".to_string(),
            pack_entry_type: "full".to_string(),
            pack_base_blob_id: None,
            pack_chain_depth: 0,
            created_at: "2026-07-06T00:00:00Z".to_string(),
        }],
        tree_locators: vec![RepositoryTreeLocatorInventoryRow {
            tree_id: "TREE-1".to_string(),
            entry_count: 1,
            tree_pack_id: "TPK-1".to_string(),
            tree_pack_checksum: "sha256-tree-1".to_string(),
            created_at: "2026-07-06T00:00:00Z".to_string(),
        }],
        snapshots: vec![RepositorySnapshotInventoryRow {
            snapshot_id: "SNP-1".to_string(),
            parent_snapshot_ids: Vec::new(),
            primary_parent_snapshot_id: None,
            parent_snapshot_id: None,
            root_tree_pack_id: "TPK-1".to_string(),
            root_entry_ordinal: 0,
            manifest_hash: "manifest-1".to_string(),
            message: Some("initial".to_string()),
            line_name: Some("main".to_string()),
            snapshot_kind: Some("line".to_string()),
            file_count: 1,
            total_bytes: 32,
            created_at: "2026-07-06T00:00:00Z".to_string(),
        }],
        line_heads: vec![RepositoryLineHeadInventoryRow {
            line_name: "main".to_string(),
            head_snapshot_id: Some("SNP-1".to_string()),
        }],
    }
}

fn conversion_object_pack_row(
    pack_id: &str,
    entries: Vec<ObjectPackIndexEntryInventory>,
) -> RepositoryObjectPackInventoryRow {
    let member_count = entries.len() as i64;
    RepositoryObjectPackInventoryRow {
        pack_id: pack_id.to_string(),
        repo_name: Some("ait-core".to_string()),
        repo_id: Some("repo-1".to_string()),
        status: "ready".to_string(),
        pack_format: PackFormatKind::ZstdChunkedV1,
        member_count,
        total_bytes: member_count * 32,
        pack_path: format!(".ait/objects/packs/{pack_id}.zstpack"),
        pack_index_entry_name: "zstd-chunked-object-index".to_string(),
        pack_index_checksum: format!("sha256:{pack_id}:object-index"),
        created_at: "2026-07-06T00:00:00Z".to_string(),
        embedded_index: ObjectPackIndexInventory {
            pack_id: pack_id.to_string(),
            pack_format: PackFormatKind::ZstdChunkedV1,
            member_count,
            total_bytes: member_count * 32,
            entries,
        },
    }
}

fn conversion_object_entry(
    blob_id: &str,
    entry_type: &str,
    base_blob_id: Option<&str>,
    chain_depth: i64,
) -> ObjectPackIndexEntryInventory {
    ObjectPackIndexEntryInventory {
        entry_name: format!("objects/{blob_id}"),
        blob_id: blob_id.to_string(),
        entry_type: entry_type.to_string(),
        checksum: format!("sha256-{blob_id}"),
        base_blob_id: base_blob_id.map(ToOwned::to_owned),
        chain_depth,
    }
}

fn conversion_blob_locator(
    blob_id: &str,
    pack_id: &str,
    entry_type: &str,
    base_blob_id: Option<&str>,
    chain_depth: i64,
) -> RepositoryBlobLocatorInventoryRow {
    RepositoryBlobLocatorInventoryRow {
        blob_id: blob_id.to_string(),
        sha256: format!("sha256-{blob_id}"),
        size_bytes: 32,
        pack_id: pack_id.to_string(),
        pack_entry_name: format!("objects/{blob_id}"),
        pack_entry_type: entry_type.to_string(),
        pack_base_blob_id: base_blob_id.map(ToOwned::to_owned),
        pack_chain_depth: chain_depth,
        created_at: "2026-07-06T00:00:00Z".to_string(),
    }
}

fn conversion_tree_pack_row(tree_ids: &[&str]) -> RepositoryTreePackInventoryRow {
    let trees = tree_ids
        .iter()
        .enumerate()
        .map(|(index, tree_id)| TreePackIndexEntryInventory {
            tree_id: (*tree_id).to_string(),
            entry_ordinal: index as i64,
            entry_count: 1,
            checksum: format!("sha256-{tree_id}"),
        })
        .collect::<Vec<_>>();
    let tree_count = trees.len() as i64;
    RepositoryTreePackInventoryRow {
        pack_id: "TPK-converted".to_string(),
        repo_name: Some("ait-core".to_string()),
        repo_id: Some("repo-1".to_string()),
        status: "ready".to_string(),
        pack_format: TreePackFormatKind::ZstdChunkedTreeV1,
        tree_count,
        total_bytes: tree_count * 64,
        pack_path: ".ait/objects/tree-packs/TPK-converted.zstpack".to_string(),
        pack_index_entry_name: "zstd-chunked-tree-index".to_string(),
        pack_index_checksum: "sha256:tree-index".to_string(),
        created_at: "2026-07-06T00:00:00Z".to_string(),
        embedded_index: TreePackIndexInventory {
            pack_id: "TPK-converted".to_string(),
            pack_format: TreePackFormatKind::ZstdChunkedTreeV1,
            tree_count,
            total_bytes: tree_count * 64,
            trees,
        },
    }
}

fn conversion_tree_locator(tree_id: &str) -> RepositoryTreeLocatorInventoryRow {
    RepositoryTreeLocatorInventoryRow {
        tree_id: tree_id.to_string(),
        entry_count: 1,
        tree_pack_id: "TPK-converted".to_string(),
        tree_pack_checksum: format!("sha256-{tree_id}"),
        created_at: "2026-07-06T00:00:00Z".to_string(),
    }
}

fn conversion_snapshot_row(
    snapshot_id: &str,
    parent_snapshot_id: Option<&str>,
    ordinal: i64,
    created_at: &str,
    line_name: Option<&str>,
    snapshot_kind: &str,
) -> RepositorySnapshotInventoryRow {
    RepositorySnapshotInventoryRow {
        snapshot_id: snapshot_id.to_string(),
        parent_snapshot_ids: parent_snapshot_id
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
        primary_parent_snapshot_id: parent_snapshot_id.map(ToOwned::to_owned),
        parent_snapshot_id: parent_snapshot_id.map(ToOwned::to_owned),
        root_tree_pack_id: "TPK-converted".to_string(),
        root_entry_ordinal: ordinal,
        manifest_hash: format!("manifest-{snapshot_id}"),
        message: Some(format!("snapshot {snapshot_id}")),
        line_name: line_name.map(ToOwned::to_owned),
        snapshot_kind: Some(snapshot_kind.to_string()),
        file_count: 1,
        total_bytes: 32,
        created_at: created_at.to_string(),
    }
}

fn conversion_path_blob(
    snapshot_id: &str,
    path: &str,
    blob_id: &str,
) -> RepositorySnapshotPathBlobInventoryRow {
    RepositorySnapshotPathBlobInventoryRow {
        snapshot_id: snapshot_id.to_string(),
        path: path.to_string(),
        blob_id: blob_id.to_string(),
    }
}

fn converted_zstd_inventory_fixture() -> ConvertedZstdInventory {
    let tree_ids = [
        "TREE-root",
        "TREE-feature",
        "TREE-child",
        "TREE-stash",
        "TREE-orphan-snapshot",
    ];
    ConvertedZstdInventory {
        inventory: RepositoryPackInventory {
            repo_name: "ait-core".to_string(),
            object_packs: vec![
                conversion_object_pack_row(
                    "OPK-main",
                    vec![
                        conversion_object_entry("BLOB-root", "full", None, 0),
                        conversion_object_entry("BLOB-child", "delta", Some("BLOB-root"), 1),
                        conversion_object_entry("BLOB-feature", "full", None, 0),
                        conversion_object_entry("BLOB-stash", "full", None, 0),
                        conversion_object_entry("BLOB-orphan-snapshot", "full", None, 0),
                    ],
                ),
                conversion_object_pack_row(
                    "OPK-orphan",
                    vec![conversion_object_entry("BLOB-unreachable", "full", None, 0)],
                ),
            ],
            tree_packs: vec![conversion_tree_pack_row(&tree_ids)],
            blob_locators: vec![
                conversion_blob_locator("BLOB-root", "OPK-main", "full", None, 0),
                conversion_blob_locator("BLOB-child", "OPK-main", "delta", Some("BLOB-root"), 1),
                conversion_blob_locator("BLOB-feature", "OPK-main", "full", None, 0),
                conversion_blob_locator("BLOB-stash", "OPK-main", "full", None, 0),
                conversion_blob_locator("BLOB-orphan-snapshot", "OPK-main", "full", None, 0),
                conversion_blob_locator("BLOB-unreachable", "OPK-orphan", "full", None, 0),
            ],
            tree_locators: tree_ids
                .iter()
                .map(|tree_id| conversion_tree_locator(tree_id))
                .collect(),
            snapshots: vec![
                conversion_snapshot_row(
                    "SNP-root",
                    None,
                    0,
                    "2026-07-06T00:00:00Z",
                    Some("main"),
                    "line",
                ),
                conversion_snapshot_row(
                    "SNP-child",
                    Some("SNP-root"),
                    2,
                    "2026-07-06T00:02:00Z",
                    Some("main"),
                    "line",
                ),
                conversion_snapshot_row(
                    "SNP-feature",
                    Some("SNP-root"),
                    1,
                    "2026-07-06T00:01:00Z",
                    Some("feature"),
                    "line",
                ),
                conversion_snapshot_row(
                    "SNP-stash",
                    Some("SNP-child"),
                    3,
                    "2026-07-06T00:03:00Z",
                    Some("main"),
                    "stash",
                ),
                conversion_snapshot_row(
                    "SNP-orphan-snapshot",
                    None,
                    4,
                    "2026-07-06T00:04:00Z",
                    None,
                    "orphan",
                ),
            ],
            line_heads: vec![
                RepositoryLineHeadInventoryRow {
                    line_name: "main".to_string(),
                    head_snapshot_id: Some("SNP-child".to_string()),
                },
                RepositoryLineHeadInventoryRow {
                    line_name: "feature".to_string(),
                    head_snapshot_id: Some("SNP-feature".to_string()),
                },
            ],
        },
        source: RepositoryPackInventorySource::ConvertedZstd {
            source_path: "fixture://converted-zstd-inventory".to_string(),
        },
        snapshot_conversion_order: vec![
            "SNP-root".to_string(),
            "SNP-feature".to_string(),
            "SNP-child".to_string(),
            "SNP-stash".to_string(),
            "SNP-orphan-snapshot".to_string(),
        ],
        snapshot_path_blobs: vec![
            conversion_path_blob("SNP-root", "file.txt", "BLOB-root"),
            conversion_path_blob("SNP-feature", "feature.txt", "BLOB-feature"),
            conversion_path_blob("SNP-child", "file.txt", "BLOB-child"),
            conversion_path_blob("SNP-stash", "stash.txt", "BLOB-stash"),
            conversion_path_blob("SNP-orphan-snapshot", "lost.txt", "BLOB-orphan-snapshot"),
        ],
        source_packed_blob_ids: vec![
            "BLOB-root".to_string(),
            "BLOB-child".to_string(),
            "BLOB-feature".to_string(),
            "BLOB-stash".to_string(),
            "BLOB-orphan-snapshot".to_string(),
            "BLOB-unreachable".to_string(),
        ],
        orphan_object_pack_ids: vec!["OPK-orphan".to_string()],
    }
}

fn set_blob_delta_metadata(
    converted: &mut ConvertedZstdInventory,
    blob_id: &str,
    base_blob_id: Option<&str>,
    chain_depth: i64,
) {
    for locator in &mut converted.inventory.blob_locators {
        if locator.blob_id == blob_id {
            locator.pack_entry_type = "delta".to_string();
            locator.pack_base_blob_id = base_blob_id.map(ToOwned::to_owned);
            locator.pack_chain_depth = chain_depth;
        }
    }
    for pack in &mut converted.inventory.object_packs {
        for entry in &mut pack.embedded_index.entries {
            if entry.blob_id == blob_id {
                entry.entry_type = "delta".to_string();
                entry.base_blob_id = base_blob_id.map(ToOwned::to_owned);
                entry.chain_depth = chain_depth;
            }
        }
    }
}

#[test]
fn repository_pack_validation_lives_outside_pack_substrate() {
    let lib_rs = include_str!("../lib.rs");
    let pack_substrate_rs = include_str!("../pack_substrate.rs");

    assert!(lib_rs.contains("pub mod repository_pack_policy;"));
    assert!(!pack_substrate_rs.contains("pub mod repository_pack_policy"));
}

#[test]
fn pack_substrate_does_not_depend_on_repository_pack_policy() {
    let pack_substrate_sources = [
        include_str!("../pack_substrate.rs"),
        include_str!("../pack_substrate/format.rs"),
        include_str!("../pack_substrate/index_json.rs"),
        include_str!("../pack_substrate/object.rs"),
        include_str!("../pack_substrate/tree_pack.rs"),
        include_str!("../pack_substrate/types.rs"),
        include_str!("../pack_substrate/util.rs"),
        include_str!("../pack_substrate/zstd.rs"),
    ];

    for source in pack_substrate_sources {
        assert!(!source.contains("repository_pack_policy"));
    }
}

#[test]
fn new_repository_first_snapshot_writes_zstd_packs() {
    assert_eq!(
        zstd_only_object_pack_write_format(),
        PackFormatKind::ZstdChunkedV1.persisted_name()
    );
    assert_eq!(
        zstd_only_tree_pack_write_format(),
        TreePackFormatKind::ZstdChunkedTreeV1.persisted_name()
    );
}

#[test]
fn converted_zstd_inventory_loads_pack_paths_and_embedded_indexes() {
    let inventory = full_zstd_inventory();

    assert_eq!(
        inventory.object_packs[0].pack_path,
        ".ait/objects/packs/OPK-1.zstpack"
    );
    assert_eq!(inventory.object_packs[0].embedded_index.pack_id, "OPK-1");
    assert_eq!(
        inventory.tree_packs[0].pack_path,
        ".ait/objects/tree-packs/TPK-1.zstpack"
    );
    assert_eq!(inventory.tree_packs[0].embedded_index.pack_id, "TPK-1");
    inventory
        .validate_zstd_only()
        .expect("full zstd inventory validates");
}

#[test]
fn converted_inventory_validation_checks_pack_indexes() {
    let mut inventory = full_zstd_inventory();
    inventory.object_packs[0].embedded_index.pack_id = "OPK-other".to_string();

    let err = inventory
        .validate_zstd_only()
        .expect_err("embedded object index mismatch must fail validation");

    assert!(err.contains("embedded index pack id mismatch"));
}

#[test]
fn converted_inventory_validation_checks_pack_paths() {
    let mut inventory = full_zstd_inventory();
    inventory.tree_packs[0].pack_path.clear();

    let err = inventory
        .validate_zstd_only()
        .expect_err("missing tree pack path must fail validation");

    assert!(err.contains("Missing tree pack path"));
}

#[test]
fn converted_inventory_validation_checks_blob_and_tree_locators() {
    let mut inventory = full_zstd_inventory();
    inventory.blob_locators[0].pack_entry_name = "objects/wrong".to_string();

    let err = inventory
        .validate_zstd_only()
        .expect_err("blob locator/index mismatch must fail validation");

    assert!(err.contains("locator entry name does not match embedded index"));

    let mut inventory = full_zstd_inventory();
    inventory.tree_locators[0].tree_pack_checksum = "wrong-tree-checksum".to_string();

    let err = inventory
        .validate_zstd_only()
        .expect_err("tree locator/index mismatch must fail validation");

    assert!(err.contains("locator checksum does not match embedded index"));
}

#[test]
fn converted_inventory_validation_checks_snapshot_root_tree_ordinals() {
    let mut inventory = full_zstd_inventory();
    inventory.snapshots[0].root_entry_ordinal = 7;

    let err = inventory
        .validate_zstd_only()
        .expect_err("unknown root ordinal must fail validation");

    assert!(err.contains("root tree ordinal 7 is not in tree pack"));
}

#[test]
fn converted_inventory_validation_checks_line_heads() {
    let mut inventory = full_zstd_inventory();
    inventory.line_heads[0].head_snapshot_id = Some("SNP-missing".to_string());

    let err = inventory
        .validate_zstd_only()
        .expect_err("unknown line head snapshot must fail validation");

    assert!(err.contains("references unknown head snapshot"));
}

#[test]
fn converted_inventory_validation_checks_server_pack_ownership() {
    let mut inventory = full_zstd_inventory();
    inventory.tree_packs[0].repo_name = Some("other-repo".to_string());

    let err = inventory
        .validate_zstd_only()
        .expect_err("wrong server tree pack owner must fail validation");

    assert!(err.contains("belongs to repository other-repo, not ait-core"));
}

#[test]
fn missing_pack_format_is_rejected_without_a_default() {
    let err = PackFormatKind::from_persisted("").expect_err("object format metadata is required");
    assert!(err.contains("Missing object pack format metadata"));

    let err =
        TreePackFormatKind::from_persisted("  ").expect_err("tree format metadata is required");
    assert!(err.contains("Missing tree-pack format metadata"));
}

#[test]
fn zstd_conversion_orders_snapshots_by_parent_topology_with_created_at_tiebreak() {
    let converted = converted_zstd_inventory_fixture();

    let summary = converted
        .validate_conversion_contract()
        .expect("converted zstd inventory should validate");

    assert_eq!(
        summary.snapshot_order,
        vec![
            "SNP-root",
            "SNP-feature",
            "SNP-child",
            "SNP-stash",
            "SNP-orphan-snapshot",
        ]
    );

    let mut wrong_order = converted_zstd_inventory_fixture();
    wrong_order.snapshot_conversion_order.swap(1, 2);

    let err = wrong_order
        .validate_conversion_contract()
        .expect_err("created_at tiebreak order must be enforced");

    assert!(err.contains("parent-topological with created_at tiebreak"));
}

#[test]
fn zstd_conversion_uses_parent_same_path_blob_as_delta_base() {
    let converted = converted_zstd_inventory_fixture();

    converted
        .validate_conversion_contract()
        .expect("child delta uses parent same-path blob");

    let mut wrong_base = converted_zstd_inventory_fixture();
    set_blob_delta_metadata(&mut wrong_base, "BLOB-child", Some("BLOB-feature"), 1);

    let err = wrong_base
        .validate_conversion_contract()
        .expect_err("delta base must come from parent same path");

    assert!(err.contains("parent snapshot same-path blob"));
}

#[test]
fn zstd_conversion_preserves_cross_line_stash_and_orphan_snapshots() {
    let converted = converted_zstd_inventory_fixture();

    let summary = converted
        .validate_conversion_contract()
        .expect("cross-line, stash, and orphan snapshots should validate");

    assert!(converted
        .inventory
        .snapshots
        .iter()
        .any(|snapshot| snapshot.line_name.as_deref() == Some("feature")));
    assert!(converted
        .inventory
        .snapshots
        .iter()
        .any(|snapshot| snapshot.snapshot_kind.as_deref() == Some("stash")));
    assert!(converted
        .inventory
        .snapshots
        .iter()
        .any(|snapshot| snapshot.snapshot_kind.as_deref() == Some("orphan")));
    assert!(summary
        .snapshot_order
        .contains(&"SNP-orphan-snapshot".to_string()));
}

#[test]
fn zstd_conversion_enforces_default_max_delta_chain_depth() {
    let mut converted = converted_zstd_inventory_fixture();
    set_blob_delta_metadata(
        &mut converted,
        "BLOB-child",
        Some("BLOB-root"),
        DEFAULT_MAX_DELTA_CHAIN_DEPTH as i64 + 1,
    );

    let err = converted
        .validate_conversion_contract()
        .expect_err("conversion validation must enforce max delta chain depth");

    assert!(err.contains("exceeds DEFAULT_MAX_DELTA_CHAIN_DEPTH"));
}

#[test]
fn zstd_conversion_keeps_unreachable_packed_blobs_in_orphan_pack() {
    let converted = converted_zstd_inventory_fixture();

    let summary = converted
        .validate_conversion_contract()
        .expect("unreachable packed source blob is retained in orphan zstd pack");

    assert_eq!(summary.source_packed_blob_count, 6);
    assert_eq!(summary.unreachable_packed_blob_count, 1);
    assert_eq!(summary.orphan_pack_count, 1);

    let mut missing_orphan_pack = converted_zstd_inventory_fixture();
    missing_orphan_pack.orphan_object_pack_ids.clear();

    let err = missing_orphan_pack
        .validate_conversion_contract()
        .expect_err("unreachable source packed blob must be retained");

    assert!(err.contains("must be retained in an orphan zstd object pack"));
}
