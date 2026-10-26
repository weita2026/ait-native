use super::*;
#[test]
fn partitioned_git_batches_keep_kinds_duplicate_aliases_old_packs_and_reuse() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("native");
    let objects = LocalGitObjectStore::open(
        db(&root.join("git"), &root),
        content(&root.join("content"), &root),
    )
    .unwrap();
    let repo = objects
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [31; 12], "partitioned")
        .unwrap();
    let shared = b"same raw body";
    let other = b"other source body";
    let a = git_object_id(GitObjectKind::Blob, shared);
    let b = git_object_id(GitObjectKind::Blob, other);
    let empty = [];
    let tree = git_object_id(GitObjectKind::Tree, &empty);
    let blob = git_object_id(GitObjectKind::Blob, &empty);
    let inputs = [
        GitObjectInput {
            kind: GitObjectKind::Blob,
            format: 1,
            object_id: &a,
            body: shared,
        },
        GitObjectInput {
            kind: GitObjectKind::Blob,
            format: 1,
            object_id: &a,
            body: shared,
        },
        GitObjectInput {
            kind: GitObjectKind::Blob,
            format: 1,
            object_id: &b,
            body: other,
        },
        GitObjectInput {
            kind: GitObjectKind::Tree,
            format: 1,
            object_id: &tree,
            body: &empty,
        },
        GitObjectInput {
            kind: GitObjectKind::Blob,
            format: 1,
            object_id: &blob,
            body: &empty,
        },
    ];
    let indices = objects
        .put_batch_with_delta_hints_and_partitions(
            repo,
            &inputs,
            &[Some("alias.txt"), Some("doc.md"), None, None, None],
            &[true, false, false, false, true],
        )
        .unwrap();
    assert_eq!(indices[0], indices[1]);
    let paths = fs::read_dir(root.join(".ait/objects/packs"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    assert_eq!(
        crate::pack_substrate::read_pack_index(paths[0].to_str().unwrap()).unwrap()["chunk_count"],
        2
    );
    let before = fs::read(&paths[0]).unwrap();
    for input in inputs {
        let (record, body) = objects.get(repo, 1, input.object_id).unwrap().unwrap();
        assert_eq!(record.kind, input.kind);
        assert_eq!(body, input.body);
    }
    assert!(objects
        .put_batch_with_delta_hints_and_partitions(
            repo,
            &[GitObjectInput {
                kind: GitObjectKind::Tree,
                format: 1,
                object_id: &tree,
                body: &empty
            }],
            &[None],
            &[true]
        )
        .is_err());
    objects
        .put_batch_with_delta_hints_and_partitions(
            repo,
            &[GitObjectInput {
                kind: GitObjectKind::Blob,
                format: 1,
                object_id: &a,
                body: shared,
            }],
            &[None],
            &[true],
        )
        .unwrap();
    assert_eq!(fs::read(&paths[0]).unwrap(), before);
    assert_eq!(
        fs::read_dir(root.join(".ait/objects/packs"))
            .unwrap()
            .count(),
        1
    );
    let late = b"prior generic data";
    let late_id = git_object_id(GitObjectKind::Blob, late);
    objects
        .put(repo, GitObjectKind::Blob, 1, &late_id, late)
        .unwrap();
    let generic = fs::read_dir(root.join(".ait/objects/packs"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p != &paths[0])
        .unwrap();
    let generic_bytes = fs::read(&generic).unwrap();
    objects
        .put_batch_with_delta_hints_and_partitions(
            repo,
            &[GitObjectInput {
                kind: GitObjectKind::Blob,
                format: 1,
                object_id: &late_id,
                body: late,
            }],
            &[Some("later.md")],
            &[true],
        )
        .unwrap();
    assert_eq!(fs::read(&generic).unwrap(), generic_bytes);
    assert_eq!(objects.get(repo, 1, &late_id).unwrap().unwrap().1, late);
}
