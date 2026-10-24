//! Compare the actual admission path at equal existing Tree/Blob cache budgets.
use super::*;
use crate::binary_db::BinaryDb;

#[test]
fn admission_reuses_snapshot_transaction_raw_trees_and_exact_namespaces() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let interop = db(&temp.path().join("interop"), &workspace);
    let content = content(&temp.path().join("content"), &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    let mut history = GitHistoryStore::open(interop.clone()).unwrap();
    let source = objects
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [2; 12], "shared-source")
        .unwrap();
    let mirror = objects
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [3; 12], "shared-mirror")
        .unwrap();
    let bodies = (0..128u32)
        .map(|i| i.to_le_bytes().repeat(256))
        .collect::<Vec<_>>();
    let oids = bodies
        .iter()
        .map(|b| git_object_id(GitObjectKind::Blob, b))
        .collect::<Vec<_>>();
    let inputs = oids
        .iter()
        .zip(&bodies)
        .map(|(object_id, body)| GitObjectInput {
            kind: GitObjectKind::Blob,
            format: 1,
            object_id,
            body,
        })
        .collect::<Vec<_>>();
    for repository in [source, mirror] {
        objects.put_batch(repository, &inputs).unwrap();
    }
    let mut tree = Vec::new();
    let mut code = Vec::new();
    for (i, oid) in oids.iter().enumerate() {
        let name = format!("file-{i:03}.rs");
        tree.extend_from_slice(format!("100644 {name}\0").as_bytes());
        tree.extend_from_slice(oid);
        code.push((
            format!("nested/{name}"),
            objects
                .get_record(source, 1, oid)
                .unwrap()
                .unwrap()
                .content_blob_index,
            0o644,
        ));
    }
    let tree_id = git_object_id(GitObjectKind::Tree, &tree);
    for repository in [source, mirror] {
        objects
            .put(repository, GitObjectKind::Tree, 1, &tree_id, &tree)
            .unwrap();
    }
    let mut documents = RawGitDocumentStore::open(interop, content.clone()).unwrap();
    let docs = documents
        .ensure_document_set(source, [2; 12], "shared-source", &[])
        .unwrap();
    let identity = b"Reuse <reuse@local> 1700000000 +0000";
    let author = history.append_identity(identity).unwrap();
    let committer = history.append_identity(identity).unwrap();
    let generation = history
        .ensure_generation(GitGenerationRecord {
            export: false,
            source_repository_index: source,
            target_repository_index_plus1: 0,
            hash: [1; 8],
        })
        .unwrap();
    let mut previous = None;
    let mut parent_snapshot = Vec::new();
    let mut commits = Vec::new();
    for i in 0..16 {
        let varying_body = format!("varied code {i}").into_bytes();
        let varying_id = git_object_id(GitObjectKind::Blob, &varying_body);
        let mut root = b"40000 nested\0".to_vec();
        root.extend_from_slice(&tree_id);
        root.extend_from_slice(b"100644 varying.rs\0");
        root.extend_from_slice(&varying_id);
        let root_id = git_object_id(GitObjectKind::Tree, &root);
        for repository in [source, mirror] {
            objects
                .put(
                    repository,
                    GitObjectKind::Blob,
                    1,
                    &varying_id,
                    &varying_body,
                )
                .unwrap();
            objects
                .put(repository, GitObjectKind::Tree, 1, &root_id, &root)
                .unwrap();
        }
        let mut varied_code = code.clone();
        varied_code.push((
            "varying.rs".into(),
            objects
                .get_record(source, 1, &varying_id)
                .unwrap()
                .unwrap()
                .content_blob_index,
            0o644,
        ));
        let parents = previous
            .map(|p: [u8; 20]| format!("parent {}\n", hex(&p)))
            .unwrap_or_default();
        let raw = format!(
            "tree {}\n{parents}author {}\ncommitter {}\n\ncommit {i}\n",
            hex(&root_id),
            std::str::from_utf8(identity).unwrap(),
            std::str::from_utf8(identity).unwrap()
        )
        .into_bytes();
        let oid = git_object_id(GitObjectKind::Commit, &raw);
        for repository in [source, mirror] {
            objects
                .put(repository, GitObjectKind::Commit, 1, &oid, &raw)
                .unwrap();
        }
        let snapshot = content
            .create_git_code_snapshot(
                "native",
                "reuse",
                &parent_snapshot,
                Some(&format!("commit {i}")),
                &varied_code,
                &mut Default::default(),
            )
            .unwrap();
        parent_snapshot = vec![snapshot.snapshot_id];
        let mapping = GitCommitMappingRecord {
            mapping_meta: 10,
            generation_index: generation,
            snapshot_index: snapshot.snapshot_index,
            author_identity_index: author,
            committer_identity_index: committer,
            object_id: oid,
            tree_object_id: root_id,
            lfs_pointer_count: 0,
            created_at_s: 1700000000,
            document_set_tree_index: docs,
        };
        history.append_commit(mapping.clone()).unwrap();
        commits.push(mapping);
        previous = Some(oid);
    }
    let mirror_generation = history
        .ensure_generation(GitGenerationRecord {
            export: false,
            source_repository_index: mirror,
            target_repository_index_plus1: 0,
            hash: [2; 8],
        })
        .unwrap();
    for mut mapping in commits {
        mapping.generation_index = mirror_generation;
        history.append_commit(mapping).unwrap();
    }
    let reopened = LocalGitObjectStore::open(objects.db().clone(), content.clone()).unwrap();
    assert_eq!(
        reopened
            .body_reads
            .load(std::sync::atomic::Ordering::Relaxed),
        177
    );
    let public_before = history.commit_mappings();
    let physical = history.physical_commit_mappings_for_profile();
    assert_ne!(public_before, physical);
    let mut physical_sorted = physical.clone();
    physical_sorted.sort_unstable_by_key(|r| (r.generation_index, r.object_id));
    assert_eq!(physical_sorted, public_before);
    let mapping_bytes = fs::read(
        objects
            .db()
            .authority_root()
            .as_path()
            .join("git_commit_mapping.bin"),
    )
    .unwrap();
    for physical in [false, true] {
        super::super::admission::validate_mapping_order_for_profile(
            &objects, &history, &documents, &content, physical, None,
        )
        .unwrap();
    }
    assert_eq!(history.commit_mappings(), public_before);
    assert_eq!(
        fs::read(
            objects
                .db()
                .authority_root()
                .as_path()
                .join("git_commit_mapping.bin")
        )
        .unwrap(),
        mapping_bytes
    );
    let mut rows = Vec::new();
    for reuse in [false, true] {
        crate::binary_db::read_profile::reset();
        let started = std::time::Instant::now();
        if reuse {
            validate_git_content_closure(&objects, &history, &documents, &content).unwrap();
        } else {
            super::super::admission::validate_git_content_closure_legacy_for_profile(
                &objects, &history, &documents, &content,
            )
            .unwrap();
        }
        rows.push(serde_json::json!({"reuse":reuse,"seconds":started.elapsed().as_secs_f64(),"counts":crate::binary_db::read_profile::snapshot()}));
    }
    println!(
        "admission_reuse_profile {}",
        serde_json::to_string(&rows).unwrap()
    );
    assert!(
        rows[1]["counts"]["read_transactions"][0].as_u64().unwrap() + 30
            < rows[0]["counts"]["read_transactions"][0].as_u64().unwrap()
    );
    assert_eq!(rows[1]["counts"]["raw_tree_parses"][0], 17);
    assert_eq!(rows[1]["counts"]["raw_tree_hits"][0], 32);
    assert_eq!(rows[0]["counts"]["raw_tree_parses"][0], 66);
    assert_eq!(
        rows[1]["counts"]["raw_closure_validations"][0]
            .as_u64()
            .unwrap()
            * 2,
        rows[0]["counts"]["raw_closure_validations"][0]
            .as_u64()
            .unwrap()
    );
    println!(
        "admission_reuse_profile {}",
        serde_json::to_string(&rows).unwrap()
    );
}

fn hex(oid: &[u8]) -> String {
    oid.iter().map(|b| format!("{b:02x}")).collect()
}
