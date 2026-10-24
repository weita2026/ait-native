use super::*;

fn recursive_fixture(root: &Path) -> (ExternalManifest, ExternalLockfile) {
    let manifest = test_manifest(vec![
        test_external("parent", "parent", "SNP-P", ".ait-external/parent"),
        test_external(
            "sibling",
            "sibling",
            "SNP-S",
            ".ait-external/parent-sibling",
        ),
    ]);
    let mut lock = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    for (parent, name) in [
        (".ait-external/parent", "child"),
        (".ait-external/parent/.ait-external/child", "grandchild"),
        (".ait-external/parent-sibling", "child"),
    ] {
        let mut node = ExternalLockNode::from_direct_declaration(&test_external(
            name,
            name,
            "SNP-C",
            &format!("{parent}/.ait-external/{name}"),
        ));
        node.parent_path = parent.into();
        lock.nodes.push(node);
    }
    write_external_status_authority(root, &manifest, &lock);
    FilesystemExternalMaterializer::new(root, FixtureExternalContentSource)
        .unwrap()
        .materialize_lockfile(&lock, &ExternalMaterializationOptions::recursive())
        .unwrap();
    (manifest, lock)
}

fn assert_parent_blocked(root: &Path) {
    let status = inspect_external_filesystem_status_report(root, "fixture").unwrap();
    let parent = status
        .report
        .externals
        .iter()
        .find(|entry| entry.name == "parent")
        .unwrap();
    assert_eq!(parent.state, ExternalStatusState::Dirty);
    let roots = inspect_operational_external_projection_roots(root, "fixture").unwrap();
    assert!(!roots.contains(&".ait-external/parent".to_string()));
    assert!(roots.contains(&".ait-external/parent-sibling".to_string()));
}

#[test]
fn external_recursive_ownership_validates_complete_graph_and_warm_cache() {
    let temp = tempfile::tempdir().unwrap();
    let (_, lock) = recursive_fixture(temp.path());
    for _ in 0..2 {
        let status = inspect_external_filesystem_status_report(temp.path(), "fixture").unwrap();
        assert!(status
            .report
            .externals
            .iter()
            .all(|entry| entry.state == ExternalStatusState::Materialized));
        assert_eq!(
            inspect_operational_external_projection_roots(temp.path(), "fixture")
                .unwrap()
                .len(),
            5
        );
    }
    // The single-node API does not grant unverified delegation from a disk marker.
    let parent = &lock.nodes[0];
    assert_eq!(
        inspect_external_materialization(temp.path(), parent)
            .unwrap()
            .state,
        ExternalObservedMaterializationState::Dirty
    );
}

#[test]
fn external_recursive_ownership_rejects_parent_and_child_file_changes() {
    for relative in [
        ".ait-external/parent",
        ".ait-external/parent/.ait-external/child",
        ".ait-external/parent/.ait-external/child/.ait-external/grandchild",
    ] {
        for mutation in [
            "modified",
            "missing",
            "extra",
            "fake-marker",
            "identity",
            "outdated",
            "missing-marker",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let (_, lock) = recursive_fixture(temp.path());
            // Populate fingerprints before each mutation to exercise cache invalidation.
            inspect_operational_external_projection_roots(temp.path(), "fixture").unwrap();
            let dir = temp.path().join(relative);
            let file = dir.join("AIT_EXTERNAL_SNAPSHOT");
            let marker = dir.join(EXTERNAL_MATERIALIZER_MARKER);
            match mutation {
                "modified" => {
                    let len = std::fs::metadata(&file).unwrap().len();
                    std::fs::write(file, vec![b'X'; len as usize]).unwrap();
                }
                "missing" => std::fs::remove_file(file).unwrap(),
                "extra" => std::fs::write(dir.join("unexpected"), "extra").unwrap(),
                "fake-marker" => {
                    let fake = dir.join(".ait-external/unlocked");
                    std::fs::create_dir_all(&fake).unwrap();
                    // Even a marker-only unregistered subtree must not be invisible.
                    std::fs::copy(marker, fake.join(EXTERNAL_MATERIALIZER_MARKER)).unwrap();
                }
                "identity" | "outdated" => {
                    let record = ExternalMaterializerMarkerJson::filesystem()
                        .read_marker(&marker)
                        .unwrap();
                    let ExternalMaterializerMarkerRecord::V3(record) = record else {
                        panic!("v3");
                    };
                    let mut forged = lock
                        .nodes
                        .iter()
                        .find(|node| node.materialize_to == relative)
                        .unwrap()
                        .clone();
                    if mutation == "identity" {
                        forged.parent_path = "forged".into();
                    } else {
                        forged.snapshot = "SNP-OTHER".into();
                    }
                    ExternalMaterializerMarkerJson::filesystem()
                        .write_marker(&marker, &forged, &record.files)
                        .unwrap();
                }
                "missing-marker" => std::fs::remove_file(marker).unwrap(),
                _ => unreachable!(),
            }
            if relative == ".ait-external/parent" && mutation == "outdated" {
                let status =
                    inspect_external_filesystem_status_report(temp.path(), "fixture").unwrap();
                assert_eq!(
                    status
                        .report
                        .externals
                        .iter()
                        .find(|e| e.name == "parent")
                        .unwrap()
                        .state,
                    ExternalStatusState::Outdated
                );
                assert!(
                    !inspect_operational_external_projection_roots(temp.path(), "fixture")
                        .unwrap()
                        .contains(&relative.to_string())
                );
            } else {
                assert_parent_blocked(temp.path());
            }
        }
    }
}

#[test]
fn external_recursive_ownership_never_exempts_sibling_or_missing_child() {
    let temp = tempfile::tempdir().unwrap();
    let (manifest, mut lock) = recursive_fixture(temp.path());
    inspect_operational_external_projection_roots(temp.path(), "fixture").unwrap();
    lock.nodes.retain(|node| node.parent_path.is_empty());
    write_external_status_authority(temp.path(), &manifest, &lock);
    let roots = inspect_operational_external_projection_roots(temp.path(), "fixture").unwrap();
    assert!(
        roots.is_empty(),
        "removing graph ownership must invalidate the warm cache"
    );

    let absent = tempfile::tempdir().unwrap();
    let (_, lock) = recursive_fixture(absent.path());
    FilesystemExternalMaterializer::new(absent.path(), FixtureExternalContentSource)
        .unwrap()
        .materialize_lockfile(&lock, &ExternalMaterializationOptions::no_recursive())
        .unwrap();
    let status = inspect_external_filesystem_status_report(absent.path(), "fixture").unwrap();
    assert_eq!(status.report.summary.missing, 3);
    assert!(
        inspect_operational_external_projection_roots(absent.path(), "fixture")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn external_recursive_ownership_rejects_invalid_lock_topology_before_writes() {
    for mutation in [
        "orphan",
        "escape",
        "overlap",
        "duplicate",
        "alias",
        "wrong-parent",
        "outside-parent",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (_, mut lock) = recursive_fixture(temp.path());
        match mutation {
            "orphan" => lock.nodes[2].parent_path = ".ait-external/unknown".into(),
            "escape" => lock.nodes[2].materialize_to = "../outside".into(),
            "overlap" => lock.nodes[2].parent_path.clear(),
            "duplicate" => lock.nodes[2].materialize_to = lock.nodes[0].materialize_to.clone(),
            "alias" => lock.nodes[2].materialize_to = format!("./{}", lock.nodes[0].materialize_to),
            "wrong-parent" => lock.nodes[3].parent_path = lock.nodes[0].materialize_to.clone(),
            "outside-parent" => lock.nodes[2].materialize_to = ".ait-external/elsewhere".into(),
            _ => unreachable!(),
        }
        assert!(lock.validate().is_err(), "{mutation}");
        let empty = tempfile::tempdir().unwrap();
        assert!(
            FilesystemExternalMaterializer::new(empty.path(), FixtureExternalContentSource)
                .unwrap()
                .materialize_lockfile(&lock, &ExternalMaterializationOptions::recursive())
                .is_err()
        );
        assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0);
    }
}

#[cfg(unix)]
#[test]
fn external_recursive_ownership_rejects_nested_symlinks() {
    for directory_link in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        recursive_fixture(temp.path());
        inspect_operational_external_projection_roots(temp.path(), "fixture").unwrap();
        let child = temp.path().join(".ait-external/parent/.ait-external/child");
        let outside = tempfile::tempdir().unwrap();
        if directory_link {
            std::fs::remove_dir_all(&child).unwrap();
            std::os::unix::fs::symlink(outside.path(), &child).unwrap();
        } else {
            std::os::unix::fs::symlink(outside.path(), child.join("escape")).unwrap();
        }
        let result = inspect_operational_external_projection_roots(temp.path(), "fixture");
        assert!(
            result.is_err()
                || !result
                    .unwrap()
                    .contains(&".ait-external/parent".to_string())
        );
        let result = inspect_external_filesystem_status_report(temp.path(), "fixture");
        assert!(
            result.is_err()
                || result
                    .unwrap()
                    .report
                    .externals
                    .iter()
                    .any(|entry| entry.name == "parent" && entry.dirty)
        );
    }
}

#[test]
fn external_recursive_ownership_rejects_overlapping_marker_claim_and_prefix_escape() {
    let temp = tempfile::tempdir().unwrap();
    let (_, lock) = recursive_fixture(temp.path());
    inspect_operational_external_projection_roots(temp.path(), "fixture").unwrap();
    let parent = &lock.nodes[0];
    let marker_path = temp
        .path()
        .join(&parent.materialize_to)
        .join(EXTERNAL_MATERIALIZER_MARKER);
    let ExternalMaterializerMarkerRecord::V3(mut marker) =
        ExternalMaterializerMarkerJson::filesystem()
            .read_marker(&marker_path)
            .unwrap()
    else {
        panic!("v3");
    };
    let child_file = ".ait-external/child/AIT_EXTERNAL_SNAPSHOT";
    let content = std::fs::read(temp.path().join(&parent.materialize_to).join(child_file)).unwrap();
    marker.files.push(ExternalMaterializerMarkerFileEntry::new(
        child_file,
        sha256_hex(&content),
    ));
    ExternalMaterializerMarkerJson::filesystem()
        .write_marker(&marker_path, parent, &marker.files)
        .unwrap();
    assert_parent_blocked(temp.path());

    let temp = tempfile::tempdir().unwrap();
    recursive_fixture(temp.path());
    let path = temp
        .path()
        .join(".ait-external/parent/.ait-external/child-other");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("unexpected"), "must not inherit child ownership").unwrap();
    assert_parent_blocked(temp.path());
}

#[test]
fn external_recursive_ownership_linked_child_blocks_parent_projection() {
    let temp = tempfile::tempdir().unwrap();
    recursive_fixture(temp.path());
    inspect_operational_external_projection_roots(temp.path(), "fixture").unwrap();
    std::fs::write(
        temp.path().join(EXTERNAL_LINKS_FILE),
        render_external_local_link_overrides(&[ExternalLocalLinkOverride {
            name: "grandchild".into(),
            path: "../local-child".into(),
        }])
        .unwrap(),
    )
    .unwrap();
    assert_parent_blocked(temp.path());
}
