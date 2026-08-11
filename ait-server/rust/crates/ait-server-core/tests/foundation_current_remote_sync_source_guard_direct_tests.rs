use std::fs;
use std::path::{Path, PathBuf};

fn rust_sources(root: &Path, sources: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("source directory should be readable") {
        let path = entry.expect("source entry should be readable").path();
        if path.is_dir() {
            rust_sources(&path, sources);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            sources.push(path);
        }
    }
}

#[test]
fn product_sources_exclude_retired_snapshot_transport_and_physical_pack_surfaces() {
    let core_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let roots = [
        core_manifest.join("src"),
        core_manifest
            .parent()
            .expect("workspace crates directory should exist")
            .join("ait-server/src"),
    ];
    let forbidden = [
        ["snapshot", "_pack"].concat(),
        ["snapshot", "_bundle"].concat(),
        ["SNAPSHOT", "_BUNDLE"].concat(),
        ["zip", "_snapshot", "_bundle"].concat(),
        ["legacy", "_snapshot", "_bundle"].concat(),
        ["import", "_snapshot", "_pack"].concat(),
        ["export", "_snapshot", "_pack"].concat(),
        ["Legacy", "Snapshot", "Pack", "Endpoint"].concat(),
        ["application/vnd.ait.", "snapshot", "-pack+", "zip"].concat(),
        ["remote_sync.", "snapshot", "_bundle.", "zip"].concat(),
        ["zip", "_deflate"].concat(),
        ["PACK_FORMAT_KIND_", "ZIP"].concat(),
        ["TREE_PACK_FORMAT_KIND_", "ZIP"].concat(),
        ["COMPAT_MAX_DELTA_CHAIN_", "READ_DEPTH"].concat(),
        ["select_blob_locator_for_repo_", "or_legacy"].concat(),
        ["build_", "pack_index"].concat(),
        ["build_tree_", "pack_index"].concat(),
        ["ait-pack-", "v2"].concat(),
        ["ait-tree-pack-", "v1"].concat(),
        ["pack", "-index.json"].concat(),
        ["tree-pack", "-index.json"].concat(),
        ["postgres_binary_", "migration"].concat(),
        ["ait-server-binary-db-", "migrate"].concat(),
        ["workflow_binary_", "backfill"].concat(),
        ["workflow_binary_read_", "parity"].concat(),
        ["workflow_binary_", "recovery"].concat(),
        ["workflow_binary_full_cutover_", "inventory"].concat(),
        ["workflow_binary_safety_", "gates"].concat(),
        ["Offline", "Maintenance"].concat(),
        ["offline_", "maintenance"].concat(),
        ["pre_v0_schema_", "compatibility"].concat(),
        ["pre_v0_test_", "fixture"].concat(),
        ["with_pre_v0_", "schema_compatibility"].concat(),
        ["postgres-remote-binary-db-", "migration"].concat(),
        ["workflow_binary.", "backfill_manifest"].concat(),
        ["migration_", "snapshot"].concat(),
        ["/v1/native/admin/workflow-binary/", "status"].concat(),
        ["BLOB_LOCATOR_", "BACKFILL_SQL"].concat(),
        ["TREE_PACK_OWNER_", "BACKFILL"].concat(),
        ["tree_pack_has_repository_", "root_snapshot"].concat(),
        ["alter table tree_packs add ", "column"].concat(),
        ["ensure_plan_revision_blob_", "locator_columns"].concat(),
        ["plan_revision_blob_", "select_expr"].concat(),
        ["pub mod workflow_binary_", "adapter"].concat(),
        ["pub mod workflow_binary_", "payload_codec"].concat(),
        ["pub mod workflow_binary_", "read"].concat(),
        ["pub mod workflow_binary_", "schema"].concat(),
        ["pub mod workflow_binary_", "write"].concat(),
        ["pub mod server_domain_", "stores"].concat(),
        ["workflow_binary.", "shadow_status"].concat(),
        ["SERVER_WORKFLOW_BINARY_", "CANONICAL_ENV"].concat(),
    ];
    let mut sources = Vec::new();
    for root in roots {
        rust_sources(&root, &mut sources);
    }
    sources.sort();

    for source in sources {
        let text = fs::read_to_string(&source).expect("Rust source should be UTF-8");
        for retired in &forbidden {
            assert!(
                !text.contains(retired),
                "{} still contains retired source fragment {retired:?}",
                source.display()
            );
        }
    }
}

#[test]
fn server_runtime_accepts_only_the_binary_v0_operational_generation_contract() {
    let server_runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace crates directory should exist")
        .join("ait-server/src/operational_binary_runtime.rs");
    let source =
        fs::read_to_string(server_runtime).expect("operational runtime source should be UTF-8");

    assert!(source.contains("ait.server.binary_v0.operational_generation.v1"));
    assert!(source.contains("manifest.layout_id != 1"));
    assert!(source.contains("manifest.status != \"validated_inactive\""));
    assert!(source.contains("manifest.global_registry != \"global\""));
    assert!(source.contains("manifest.repository_authorities != \"repositories\""));
}
