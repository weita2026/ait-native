use super::*;
use crate::binary_db::BinaryDb;

#[test]
fn frozen_tag_ordinals_reuse_metadata_and_keep_tombstone_slots() {
    use crate::tag_store::{new_tag_record, BinaryDbTagStore, TagStore};
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let content = content(&temp.path().join("content"), &workspace);
    let view = content
        .create_git_code_snapshot(
            "tags",
            "main",
            &[],
            Some("tags"),
            &[],
            &mut Default::default(),
        )
        .unwrap();
    let tags = BinaryDbTagStore::new(content);
    for at in 0..32 {
        tags.create_tag(
            &new_tag_record(
                &format!("tag-{at}"),
                &view.snapshot_id,
                "tag",
                "2026-10-15T00:00:00Z",
            )
            .unwrap(),
        )
        .unwrap();
    }
    tags.delete_tag("tag-7").unwrap();
    crate::binary_db::read_profile::reset();
    let ordinals = tags.tag_ordinals_by_name().unwrap();
    assert_eq!(ordinals.len(), 32);
    assert_eq!(ordinals["tag-7"], 7);
    let counts = crate::binary_db::read_profile::snapshot();
    assert_eq!(counts["full_record:tag.bin"].0, 1);
    assert_eq!(counts["full_record:snapshot.bin"].0, 1);
    assert_eq!(tags.list_tags().unwrap().len(), 31);
    assert!(tags.tag_by_name("tag-7").unwrap().is_none());
    assert_eq!(tags.tag_index_by_name("tag-7").unwrap(), Some(7));
}

#[test]
fn metadata_batch_checks_once_and_rejects_key_count_and_midcall_drift() {
    use std::sync::atomic::Ordering;
    let temp = tempdir().unwrap();
    let root = temp.path().join("native");
    let interop = db(&root.join("git"), &root);
    let content = content(&root.join("content"), &root);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    let repo = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [13; 12],
            "metadata-batch",
        )
        .unwrap();
    let bodies = [b"first".as_slice(), b"second".as_slice()];
    let ids = bodies
        .iter()
        .map(|body| git_object_id(GitObjectKind::Blob, body))
        .collect::<Vec<_>>();
    for (body, oid) in bodies.iter().zip(&ids) {
        objects
            .put(repo, GitObjectKind::Blob, 1, oid, body)
            .unwrap();
    }
    let wanted = (0..128).map(|at| ids[at % 2]).collect::<Vec<_>>();
    let reads = objects.body_reads.load(Ordering::Relaxed);
    crate::binary_db::read_profile::reset();
    let rows = objects.get_record_batch(repo, 1, &wanted).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.object_id).collect::<Vec<_>>(),
        wanted
    );
    assert_eq!(objects.body_reads.load(Ordering::Relaxed), reads);
    assert_eq!(
        crate::binary_db::read_profile::snapshot()["git_index_count_checks"].0,
        1
    );
    assert!(objects
        .get_record_batch(repo, 1, &vec![ids[0]; 129])
        .is_err());
    assert!(objects.get_record_batch(repo, 2, &[]).is_err());
    assert!(objects.get_record_batch(repo, 1, &[]).unwrap().is_empty());
    let path = interop.authority_root().as_path().join("git_object.bin");
    let original = fs::read(&path).unwrap();
    let mut damaged = original.clone();
    damaged[16] ^= 1;
    fs::write(&path, damaged).unwrap();
    assert!(objects
        .get_record_batch(repo, 1, &[ids[0]])
        .unwrap_err()
        .to_string()
        .contains("different key"));
    fs::write(&path, original).unwrap();
    let index = interop.authority_root().as_path().join("git_object.idx");
    let original = fs::read(&index).unwrap();
    let mut damaged = original.clone();
    damaged.push(0);
    fs::write(&index, damaged).unwrap();
    assert!(objects.get_record_batch(repo, 1, &[ids[0]]).is_err());
    fs::write(&index, original).unwrap();
    let hook_path = index.clone();
    super::store::set_metadata_batch_test_hook(Box::new(move || {
        std::fs::File::open(hook_path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(123456))
            .unwrap();
    }));
    assert_eq!(
        objects
            .get_record_batch(repo, 1, &[ids[0]])
            .unwrap_err()
            .kind(),
        BinaryDbErrorKind::RetryableBusy
    );
    assert_eq!(objects.get_record_batch(repo, 1, &ids).unwrap().len(), 2);
    let third = git_object_id(GitObjectKind::Blob, b"third");
    objects
        .put(repo, GitObjectKind::Blob, 1, &third, b"third")
        .unwrap();
    assert_eq!(
        objects.get_record_batch(repo, 1, &[third]).unwrap()[0].object_id,
        third
    );
    let other = LocalGitObjectStore::open(interop, content).unwrap();
    let fourth = git_object_id(GitObjectKind::Blob, b"fourth");
    other
        .put(repo, GitObjectKind::Blob, 1, &fourth, b"fourth")
        .unwrap();
    assert_eq!(
        objects
            .get_record_batch(repo, 1, &[ids[0]])
            .unwrap_err()
            .kind(),
        BinaryDbErrorKind::RetryableBusy
    );
}

use crate::binary_db::{
    AuthorityId, BinaryDbErrorKind, BinaryDbReadTxn, LocalBinaryDbFs, LocalStateScope,
};
use crate::content_binary_db::{LocalContentBinaryDb, BLOB_BIN, OBJECT_PACK_BIN};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;

#[path = "tests/admission_reuse.rs"]
mod admission_reuse;
#[path = "tests/delta_samples.rs"]
mod delta_samples;
#[path = "tests/document_open_reader.rs"]
mod document_open_reader;
#[path = "tests/tree_cache_pressure.rs"]
mod tree_cache_pressure;

#[test]
#[ignore = "explicit representative mapping/MD admission timing"]
fn representative_mapping_md_admission_cost() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let interop = db(&temp.path().join("interop"), &workspace);
    let content = content(&temp.path().join("content"), &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    let source = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [6; 12],
            "representative-source",
        )
        .unwrap();
    let generation = GitHistoryStore::open(interop.clone())
        .unwrap()
        .ensure_generation(GitGenerationRecord {
            export: false,
            source_repository_index: source,
            target_repository_index_plus1: 0,
            hash: [9; 8],
        })
        .unwrap();
    let bodies = (0..256usize)
        .map(|i| {
            let mut body = vec![b'x'; 8192];
            body[..8].copy_from_slice(&(i as u64).to_le_bytes());
            body
        })
        .collect::<Vec<_>>();
    let ids = bodies
        .iter()
        .map(|body| git_object_id(GitObjectKind::Blob, body))
        .collect::<Vec<_>>();
    for offset in [0usize, 128] {
        let inputs = (offset..offset + 128)
            .map(|i| GitObjectInput {
                kind: GitObjectKind::Blob,
                format: 1,
                object_id: &ids[i],
                body: &bodies[i],
            })
            .collect::<Vec<_>>();
        objects.put_batch(source, &inputs).unwrap();
    }
    let mut tree = Vec::new();
    let mut code = Vec::new();
    let mut names = Vec::new();
    for (i, oid) in ids.iter().enumerate() {
        let name = if i < 128 {
            format!("code{i:03}.txt")
        } else {
            format!("doc{i:03}.md")
        };
        tree.extend_from_slice(format!("100644 {name}\0").as_bytes());
        tree.extend_from_slice(oid);
        if i < 128 {
            code.push((
                name.clone(),
                objects
                    .get_record(source, 1, oid)
                    .unwrap()
                    .unwrap()
                    .content_blob_index,
                0o644,
            ));
        }
        names.push(name);
    }
    let tree_id = git_object_id(GitObjectKind::Tree, &tree);
    objects
        .put(source, GitObjectKind::Tree, 1, &tree_id, &tree)
        .unwrap();
    let identity = b"Representative <bench@local> 1700000000 +0000";
    let raw = format!(
        "tree {}\nauthor {}\ncommitter {}\n\nrepresentative\n",
        tree_id
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        std::str::from_utf8(identity).unwrap(),
        std::str::from_utf8(identity).unwrap()
    )
    .into_bytes();
    let commit = git_object_id(GitObjectKind::Commit, &raw);
    objects
        .put(source, GitObjectKind::Commit, 1, &commit, &raw)
        .unwrap();
    let mut documents = RawGitDocumentStore::open(interop.clone(), content.clone()).unwrap();
    let inputs = (128..256)
        .map(|i| RawGitDocumentInput {
            path: &names[i],
            mode: 0o644,
            body: &bodies[i],
        })
        .collect::<Vec<_>>();
    let docs = documents
        .ensure_document_set(source, [6; 12], "representative-source", &inputs)
        .unwrap();
    let view = content
        .create_git_code_snapshot(
            "native",
            "bench",
            &[],
            Some("representative"),
            &code,
            &mut Default::default(),
        )
        .unwrap();
    let mut history = GitHistoryStore::open(interop).unwrap();
    let author = history.append_identity(identity).unwrap();
    let committer = history.append_identity(identity).unwrap();
    history
        .append_commit(GitCommitMappingRecord {
            mapping_meta: 10,
            generation_index: generation,
            snapshot_index: view.snapshot_index,
            author_identity_index: author,
            committer_identity_index: committer,
            object_id: commit,
            tree_object_id: tree_id,
            lfs_pointer_count: 0,
            created_at_s: 1700000000,
            document_set_tree_index: docs,
        })
        .unwrap();
    let mirror = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [5; 12],
            "representative-mirror",
        )
        .unwrap();
    for offset in [0usize, 128] {
        objects
            .put_batch(
                mirror,
                &(offset..offset + 128)
                    .map(|i| GitObjectInput {
                        kind: GitObjectKind::Blob,
                        format: 1,
                        object_id: &ids[i],
                        body: &bodies[i],
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
    }
    objects
        .put(mirror, GitObjectKind::Tree, 1, &tree_id, &tree)
        .unwrap();
    objects
        .put(mirror, GitObjectKind::Commit, 1, &commit, &raw)
        .unwrap();
    let second_generation = history
        .ensure_generation(GitGenerationRecord {
            export: false,
            source_repository_index: mirror,
            target_repository_index_plus1: 0,
            hash: [10; 8],
        })
        .unwrap();
    let mut duplicate = history.commit_mappings()[0].clone();
    duplicate.generation_index = second_generation;
    history.append_commit(duplicate).unwrap();
    crate::binary_db::read_profile::reset();
    let started = std::time::Instant::now();
    validate_git_content_closure(&objects, &history, &documents, &content).unwrap();
    let counts = crate::binary_db::read_profile::snapshot();
    assert_eq!(counts["mapping_binding_validations"].0, 1);
    assert_eq!(counts["mapping_binding_reuses"].0, 1);
    // Corrupt data only after the first binding was verified and just before
    // its exact duplicate would skip traversal. Evict every data reader first.
    for kind in ["object", "tree", "metadata"] {
        let path = if kind == "metadata" {
            temp.path().join("interop/git_generation.bin")
        } else {
            let directory = workspace.join(if kind == "object" {
                ".ait/objects/packs"
            } else {
                ".ait/objects/tree-packs"
            });
            fs::read_dir(directory)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.extension().is_some_and(|e| e == "zstpack"))
                .unwrap()
        };
        let original = fs::read(&path).unwrap();
        let changed_path = path.clone();
        let mut changed = original.clone();
        changed[0] ^= 0x80;
        super::admission::set_binding_reuse_hook(Box::new(move |body, code_trees, doc_trees| {
            body.evict_readers_for_test();
            code_trees.clear_archives();
            code_trees.clear_tree_entries();
            doc_trees.clear_archives();
            doc_trees.clear_tree_entries();
            fs::write(&changed_path, changed).unwrap();
        }));
        let failure =
            validate_git_content_closure(&objects, &history, &documents, &content).unwrap_err();
        assert!(
            failure.contains("changed") || failure.contains("layout"),
            "{kind}: {failure}"
        );
        fs::write(&path, original).unwrap();
        validate_git_content_closure(&objects, &history, &documents, &content).unwrap();
    }
    // A different complete namespace inventory cannot share the proof,
    // even when this commit and root bytes still match.
    let extra_body = b"unrelated independently valid namespace object";
    let extra_oid = git_object_id(GitObjectKind::Blob, extra_body);
    objects
        .put(mirror, GitObjectKind::Blob, 1, &extra_oid, extra_body)
        .unwrap();
    crate::binary_db::read_profile::reset();
    validate_git_content_closure(&objects, &history, &documents, &content).unwrap();
    let counts = crate::binary_db::read_profile::snapshot();
    assert_eq!(counts["mapping_binding_validations"].0, 2);
    assert!(!counts.contains_key("mapping_binding_reuses"));
    let mut bad_code = code.clone();
    bad_code[0].1 = objects
        .get_record(source, 1, &ids[128])
        .unwrap()
        .unwrap()
        .content_blob_index;
    let wrong = content
        .create_git_code_snapshot(
            "native",
            "bench",
            &[],
            Some("wrong"),
            &bad_code,
            &mut Default::default(),
        )
        .unwrap();
    let third_generation = history
        .ensure_generation(GitGenerationRecord {
            export: false,
            source_repository_index: source,
            target_repository_index_plus1: 0,
            hash: [11; 8],
        })
        .unwrap();
    let mut mismatch = history.commit_mappings()[0].clone();
    mismatch.generation_index = third_generation;
    mismatch.snapshot_index = wrong.snapshot_index;
    history.append_commit(mismatch).unwrap();
    assert!(
        validate_git_content_closure(&objects, &history, &documents, &content)
            .unwrap_err()
            .contains("differs from raw source tree")
    );
    println!(
        "mapping admission code_files=128 md_files=128 raw_bytes=2097152 wall_seconds={:.6}",
        started.elapsed().as_secs_f64()
    );
}

#[test]
fn repeated_mapping_binding_reuses_only_identical_verified_components() {
    representative_mapping_md_admission_cost();
}

#[test]
#[ignore = "explicit scattered-pack MD capture timing and cache lifetime"]
fn scattered_pack_md_capture_cost_and_call_boundary() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let interop = db(&temp.path().join("interop"), &workspace);
    let content = content(&temp.path().join("content"), &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    let source = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [8; 12],
            "scattered-source",
        )
        .unwrap();
    let mut selected = Vec::new();
    let mut selected_bodies = Vec::new();
    for pack in 0..16usize {
        let bodies = (0..128usize)
            .map(|member| {
                let mut body = vec![b'x'; 8192];
                body[..8].copy_from_slice(&((pack * 128 + member) as u64).to_le_bytes());
                body
            })
            .collect::<Vec<_>>();
        let ids = bodies
            .iter()
            .map(|body| git_object_id(GitObjectKind::Blob, body))
            .collect::<Vec<_>>();
        let inputs = ids
            .iter()
            .zip(&bodies)
            .map(|(oid, body)| GitObjectInput {
                kind: GitObjectKind::Blob,
                format: 1,
                object_id: oid,
                body,
            })
            .collect::<Vec<_>>();
        objects.put_batch(source, &inputs).unwrap();
        selected.push(ids[0]);
        selected_bodies.push(bodies[0].clone());
    }
    let names = (0..16).map(|i| format!("doc{i:02}.md")).collect::<Vec<_>>();
    let object_inputs = names
        .iter()
        .zip(&selected)
        .enumerate()
        .map(|(i, (path, object_id))| RawGitDocumentObjectInput {
            path,
            mode: if i % 2 == 0 { 0o644 } else { 0o755 },
            object_id: *object_id,
        })
        .collect::<Vec<_>>();
    let mut documents = RawGitDocumentStore::open(interop.clone(), content.clone()).unwrap();
    let other_content = self::content(&temp.path().join("other-content"), &workspace);
    let mut other_docs = RawGitDocumentStore::open(interop.clone(), other_content).unwrap();
    let root = documents
        .ensure_document_set_from_objects(
            &objects,
            source,
            [8; 12],
            "scattered-source",
            &object_inputs,
        )
        .unwrap();
    let started = std::time::Instant::now();
    let old_bodies = objects.get_batch(source, 1, &selected).unwrap();
    let raw = object_inputs
        .iter()
        .zip(&old_bodies)
        .map(|(input, (_, body))| RawGitDocumentInput {
            path: input.path,
            mode: input.mode,
            body,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        documents
            .ensure_document_set(source, [8; 12], "scattered-source", &raw)
            .unwrap(),
        root
    );
    let separate = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    assert_eq!(
        documents
            .ensure_document_set_from_objects(
                &objects,
                source,
                [8; 12],
                "scattered-source",
                &object_inputs
            )
            .unwrap(),
        root
    );
    let shared = started.elapsed().as_secs_f64();
    let restored = documents.document_set_files(root).unwrap();
    for (i, (path, mode, body)) in restored.iter().enumerate() {
        assert_eq!(path, &names[i]);
        assert_eq!(*mode, object_inputs[i].mode);
        assert_eq!(body, &selected_bodies[i]);
    }
    println!("scattered MD capture source_packs=16 source_chunk_bytes=16777216 md_files=16 md_raw_bytes=131072 separate_seconds={separate:.6} shared_seconds={shared:.6}");

    assert!(other_docs
        .ensure_document_set_from_objects(
            &objects,
            source,
            [8; 12],
            "scattered-source",
            &object_inputs
        )
        .unwrap_err()
        .to_string()
        .contains("same object/content authorities"));
    // A new call must reread source bytes, even after the identical DocSet was reused.
    let packs = workspace.join(".ait/objects/packs");
    for entry in fs::read_dir(packs).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "zstpack") {
            fs::write(path, b"corrupt").unwrap();
        }
    }
    assert!(documents
        .ensure_document_set_from_objects(
            &objects,
            source,
            [8; 12],
            "scattered-source",
            &object_inputs
        )
        .is_err());
}

#[test]
fn metadata_only_lookup_reads_no_body_and_preserves_authority_drift_guard() {
    use std::sync::atomic::Ordering;
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let interop = db(&temp.path().join("interop"), &workspace);
    let content = content(&temp.path().join("content"), &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    let repo = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [8; 12],
            "body-count-source",
        )
        .unwrap();
    let body = vec![7; 16384];
    let oid = git_object_id(GitObjectKind::Blob, &body);
    objects
        .put(repo, GitObjectKind::Blob, 1, &oid, &body)
        .unwrap();
    let before = objects.body_reads.load(Ordering::Relaxed);
    for _ in 0..128 {
        assert!(objects.get_record(repo, 1, &oid).unwrap().is_some());
    }
    assert_eq!(objects.body_reads.load(Ordering::Relaxed), before);
    for _ in 0..128 {
        assert_eq!(objects.get(repo, 1, &oid).unwrap().unwrap().1, body);
    }
    assert_eq!(objects.body_reads.load(Ordering::Relaxed) - before, 128);
    println!("metadata lookups=128 body_reads=0; full lookups=128 body_reads=128");
    let record = objects.get_record(repo, 1, &oid).unwrap().unwrap();
    let read = BinaryDbReadTxn::new(content.db());
    let mut view = content
        .blobs()
        .blob_view_at(&read, record.content_blob_index)
        .unwrap();
    let mut session = crate::content_binary_db::BinaryBlobReadSession::default();
    assert_eq!(
        content
            .blobs()
            .read_blob_bytes_for_view_with_session(&read, &view, &mut session, 0, None)
            .unwrap(),
        body
    );
    view.record.sha256[31] ^= 1;
    assert!(content
        .blobs()
        .read_blob_bytes_for_view_with_session(&read, &view, &mut session, 0, None)
        .unwrap_err()
        .to_string()
        .contains("metadata/hash mismatch"));
    drop(read);
    let other = LocalGitObjectStore::open(interop, content).unwrap();
    let second = git_object_id(GitObjectKind::Blob, b"second");
    other
        .put(repo, GitObjectKind::Blob, 1, &second, b"second")
        .unwrap();
    assert_eq!(
        objects.get_record(repo, 1, &oid).unwrap_err().kind(),
        BinaryDbErrorKind::RetryableBusy
    );
}

#[test]
#[ignore = "explicit isolated-copy profile of retained partial import"]
fn retained_partial_object_readback_and_new_batch_cost() {
    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            assert!(!entry.file_type().unwrap().is_symlink());
            if path.is_dir() {
                copy_tree(&path, &target.join(entry.file_name()));
            } else {
                fs::copy(path, target.join(entry.file_name())).unwrap();
            }
        }
    }
    let root = std::path::PathBuf::from(std::env::var_os("AIT_GIT_BENCH_ROOT").unwrap());
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    copy_tree(&root.join("native"), &workspace);
    let interop = db(&workspace.join(".ait/git-interop/v1/binary-v0"), &workspace);
    let content = content(&workspace.join(".ait/binary-db"), &workspace);
    let started = std::time::Instant::now();
    let objects = LocalGitObjectStore::open(interop, content).unwrap();
    let readback = started.elapsed().as_secs_f64();
    let bytes = fs::read(workspace.join(".ait/git-interop/v1/binary-v0/git_object.bin")).unwrap();
    let stored = bytes[4..]
        .chunks_exact(32)
        .map(|row| {
            row[12..32]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(stored.len(), 25727);
    let remaining = fs::read_to_string(root.join("source-closure-oids.txt"))
        .unwrap()
        .lines()
        .filter(|oid| !stored.contains(*oid))
        .skip(128)
        .take(384)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut child = Command::new("git")
        .arg("--git-dir")
        .arg(root.join("source.git"))
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::{BufRead, Read};
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut inputs = Vec::new();
    for oid in remaining {
        writeln!(child.stdin.as_mut().unwrap(), "{oid}").unwrap();
        child.stdin.as_mut().unwrap().flush().unwrap();
        let mut header = String::new();
        stdout.read_line(&mut header).unwrap();
        let fields = header.split_whitespace().collect::<Vec<_>>();
        assert_eq!(fields[0], oid);
        let kind = match fields[1] {
            "blob" => GitObjectKind::Blob,
            "tree" => GitObjectKind::Tree,
            "commit" => GitObjectKind::Commit,
            "tag" => GitObjectKind::Tag,
            _ => panic!("kind"),
        };
        let mut body = vec![0; fields[2].parse::<usize>().unwrap()];
        stdout.read_exact(&mut body).unwrap();
        let mut newline = [0];
        stdout.read_exact(&mut newline).unwrap();
        assert_eq!(newline, [b'\n']);
        let id = git_object_id(kind, &body);
        inputs.push((kind, id, body));
    }
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    println!("partial root readback objects=25727 seconds={readback:.6}");
    for (batch, rows) in inputs.chunks(128).enumerate() {
        let values = rows
            .iter()
            .map(|(kind, id, body)| GitObjectInput {
                kind: *kind,
                format: 1,
                object_id: id,
                body,
            })
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        objects.put_batch(0, &values).unwrap();
        println!(
            "new object batch={batch} objects={} raw_bytes={} seconds={:.6}",
            rows.len(),
            rows.iter().map(|(_, _, body)| body.len()).sum::<usize>(),
            started.elapsed().as_secs_f64()
        );
    }
}

#[test]
fn raw_document_crash_child() {
    let Some(root) = std::env::var_os("AIT_RAW_GIT_DOC_TEST_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let workspace = root.join("native");
    let mut documents = RawGitDocumentStore::open(
        db(&root.join("interop"), &workspace),
        content(&root.join("content"), &workspace),
    )
    .unwrap();
    documents
        .ensure_document_set(
            0,
            [6; 12],
            "crash-source",
            &[
                RawGitDocumentInput {
                    path: "README.md",
                    mode: 0o755,
                    body: b"raw\r\n\xff\x00",
                },
                RawGitDocumentInput {
                    path: "link.MD",
                    mode: 0o120000,
                    body: b"README.md",
                },
            ],
        )
        .unwrap();
    panic!("child must be killed before Plan root publication");
}

#[cfg(unix)]
#[test]
fn raw_document_sigkill_preserves_complete_revision_slots_before_plan_publication() {
    use std::os::unix::process::ExitStatusExt;
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let cr = temp.path().join("content");
    let interop = db(&temp.path().join("interop"), &workspace);
    let content = content(&cr, &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    objects
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [6; 12], "crash-source")
        .unwrap();
    let marker = temp.path().join("raw-document-paused");
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "git_binary_db::tests::raw_document_crash_child",
                "--nocapture",
            ])
            .env("AIT_RAW_GIT_DOC_TEST_ROOT", temp.path())
            .env("AIT_RAW_GIT_DOC_TEST_PAUSE", &marker)
            .stdin(Stdio::null())
            .spawn()
            .unwrap(),
    );
    for _ in 0..400 {
        if marker.exists() || child.0.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        marker.exists(),
        "child did not reach durable revision dependency boundary"
    );
    child.0.kill().unwrap();
    assert_eq!(child.0.wait().unwrap().signal(), Some(9));
    let revisions = fs::read(cr.join("plan_revision.bin")).unwrap();
    let payload = fs::read(cr.join("plan_revision_payload.bin")).unwrap();
    assert_eq!(revisions.len(), 4 + 2 * 56);
    let generic = crate::plan_binary_db::repair_plan_binary_db_authority(&cr);
    println!("SIGKILL raw dependency: complete revision bytes before={}, after generic recovery={}, blocked={}",revisions.len(),fs::read(cr.join("plan_revision.bin")).map(|bytes|bytes.len()).unwrap_or(0),generic.is_err());
    assert!(
        generic.is_err(),
        "generic recovery must block rather than reclaim complete raw revision slots"
    );
    assert_eq!(fs::read(cr.join("plan_revision.bin")).unwrap(), revisions);
    assert_eq!(
        fs::read(cr.join("plan_revision_payload.bin")).unwrap(),
        payload
    );
    assert!(RawGitDocumentStore::recover_plan_roots(&interop, &content).unwrap());
    assert!(!RawGitDocumentStore::recover_plan_roots(&interop, &content).unwrap());
    RawGitDocumentStore::rebuild_indexes(&interop, &content).unwrap();
    let mut documents = RawGitDocumentStore::open(interop.clone(), content.clone()).unwrap();
    let tree = documents
        .ensure_document_set(
            0,
            [6; 12],
            "crash-source",
            &[
                RawGitDocumentInput {
                    path: "README.md",
                    mode: 0o755,
                    body: b"raw\r\n\xff\x00",
                },
                RawGitDocumentInput {
                    path: "link.MD",
                    mode: 0o120000,
                    body: b"README.md",
                },
            ],
        )
        .unwrap();
    assert_eq!(
        documents.document_set_files(tree).unwrap(),
        vec![
            ("README.md".into(), 0o755, b"raw\r\n\xff\x00".to_vec()),
            ("link.MD".into(), 0o120000, b"README.md".to_vec())
        ]
    );
    assert_eq!(fs::read(cr.join("plan_revision.bin")).unwrap(), revisions);
    assert_eq!(
        fs::read(cr.join("plan_revision_payload.bin")).unwrap(),
        payload
    );
    crate::plan_binary_db::repair_plan_binary_db_authority(&cr).unwrap();
    assert_eq!(fs::read(cr.join("plan_revision.bin")).unwrap(), revisions);
    assert_eq!(
        BinaryDbReadTxn::new(content.db())
            .record_count(
                crate::plan_binary_db::BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_file()
            )
            .unwrap(),
        2
    );
    println!("SIGKILL typed recovery: two exact permanent Plan/revision slots, raw CRLF/NUL/nonUTF8 Markdown and symlink modes preserved; replay no-op; 0 mapping JSON");
}

#[test]
fn native_history_freezes_ref_intent_and_requires_complete_checkpoint() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("repo");
    let interop = db(&temp.path().join("interop"), &workspace);
    let objects = LocalGitObjectStore::open(
        interop.clone(),
        content(&temp.path().join("content"), &workspace),
    )
    .unwrap();
    let source = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [6; 12],
            "history source",
        )
        .unwrap();
    let history = GitHistoryStore::open(interop.clone()).unwrap();
    assert_eq!(
        objects.repository_identity(source).unwrap(),
        (
            GitRepositoryKind::ImportSource,
            [6; 12],
            "history source".to_string()
        )
    );
    let generation = history
        .ensure_generation(GitGenerationRecord {
            export: false,
            source_repository_index: source,
            target_repository_index_plus1: 0,
            hash: [7; 8],
        })
        .unwrap();
    let frozen = GitRefMappingRecord {
        meta: 0,
        object_kind: GitObjectKind::Commit,
        generation_index: generation,
        snapshot_index_plus1: 0,
        target_index_plus1: 0,
        symbolic_target_index_plus1: 0,
        object_id: [8; 20],
        expected_previous: [0; 20],
        created_at_s: 9,
    };
    assert_eq!(
        history
            .append_ref("refs/heads/main", frozen.clone())
            .unwrap(),
        0
    );
    let mut bound = frozen.clone();
    bound.meta |= 4;
    bound.snapshot_index_plus1 = 1;
    let mut changed_intent = bound.clone();
    changed_intent.meta |= 16;
    changed_intent.expected_previous = [1; 20];
    assert!(history
        .append_ref("refs/heads/main", changed_intent)
        .is_err());
    let mut changed_kind = bound.clone();
    changed_kind.meta |= 1;
    assert!(history.append_ref("refs/heads/main", changed_kind).is_err());
    assert_eq!(
        history
            .append_ref("refs/heads/main", bound.clone())
            .unwrap(),
        1
    );
    assert_eq!(history.append_ref("refs/heads/main", bound).unwrap(), 1);
    let mut conflict = frozen;
    conflict.object_id = [9; 20];
    assert!(history.append_ref("refs/heads/main", conflict).is_err());
    assert!(!history.generation_is_complete(generation).unwrap());
    let mut checkpoint = GitOperationCheckpointRecord {
        status: 0,
        generation_index: generation,
        operation_hash: [10; 8],
        plan_hash: [11; 12],
        next_commit: 0,
        next_ref: 0,
        updated_at_s: 12,
    };
    history.checkpoint(checkpoint.clone()).unwrap();
    assert!(!history.generation_is_complete(generation).unwrap());
    assert_eq!(
        history
            .checkpoint_by_hash(&[10; 8])
            .unwrap()
            .unwrap()
            .next_commit,
        0
    );
    checkpoint.status = 1;
    checkpoint.next_ref = 1;
    history.checkpoint(checkpoint.clone()).unwrap();
    assert!(history.generation_is_complete(generation).unwrap());
    let before = fs::read(temp.path().join("interop/git_operation_checkpoint.bin")).unwrap();
    let mut invalid = checkpoint.clone();
    invalid.next_commit = 1;
    assert!(history.checkpoint(invalid).is_err());
    let mut demoted = checkpoint.clone();
    demoted.status = 0;
    assert!(history.checkpoint(demoted).is_err());
    let mut short = checkpoint.clone();
    short.next_ref = 0;
    assert!(history.checkpoint(short).is_err());
    assert_eq!(
        before,
        fs::read(temp.path().join("interop/git_operation_checkpoint.bin")).unwrap()
    );
    checkpoint.plan_hash = [12; 12];
    assert!(history.checkpoint(checkpoint).is_err());
    assert_eq!(
        before,
        fs::read(temp.path().join("interop/git_operation_checkpoint.bin")).unwrap()
    );
    history
        .append_identity(b"Name\xff <fixture@example.invalid> 1234 -0000")
        .unwrap();
    assert!(history
        .append_identity("Name <email> 1234 +é12".as_bytes())
        .is_err());
    let fixed_before = [
        "git_generation.bin",
        "git_ref_mapping.bin",
        "git_identity.bin",
        "git_operation_checkpoint.bin",
    ]
    .map(|name| fs::read(temp.path().join("interop").join(name)).unwrap());
    fs::remove_file(temp.path().join("interop/git_ref_lookup.idx")).unwrap();
    assert!(GitHistoryStore::open(interop.clone()).is_err());
    GitHistoryStore::rebuild_indexes(&interop).unwrap();
    GitHistoryStore::open(interop.clone()).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(temp.path().join("interop/git_operation_id.idx"))
        .unwrap()
        .write_all(&[0; 12])
        .unwrap();
    assert!(GitHistoryStore::open(interop.clone()).is_err());
    GitHistoryStore::rebuild_indexes(&interop).unwrap();
    GitHistoryStore::open(interop).unwrap();
    assert_eq!(
        fixed_before,
        [
            "git_generation.bin",
            "git_ref_mapping.bin",
            "git_identity.bin",
            "git_operation_checkpoint.bin"
        ]
        .map(|name| fs::read(temp.path().join("interop").join(name)).unwrap())
    );
}

fn db(root: &Path, workspace: &Path) -> LocalBinaryDbFs {
    LocalBinaryDbFs::new(
        root,
        workspace,
        AuthorityId::new("git-fixture"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(GIT_OBJECT_STORE_BIN_PATHS)
    .with_declared_index_paths(GIT_OBJECT_STORE_INDEX_PATHS)
}

fn content(root: &Path, workspace: &Path) -> LocalContentBinaryDb<1> {
    LocalContentBinaryDb::new(
        root,
        workspace,
        AuthorityId::new("git-fixture"),
        LocalStateScope::Repository,
    )
}

fn git(root: &Path, args: &[&str], input: Option<&[u8]>) -> Vec<u8> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(bytes) = input {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .trim()
        .to_string()
}
fn oid(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|part| u8::from_str_radix(std::str::from_utf8(part).unwrap(), 16).unwrap())
        .collect()
}

fn stored_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|name| name == ".locks") {
            continue;
        }
        if path.is_dir() {
            files.extend(stored_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn native_git_history_survives_odb_removal_and_shares_blobs() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"], None);
    fs::write(source.join("code.txt"), b"code-v1\n").unwrap();
    fs::write(source.join("README.md"), b"first\r\n\xff\0raw\r").unwrap();
    git(&source, &["add", "."], None);
    git(&source, &["commit", "-m", "base"], None);
    let base = text(&git(&source, &["rev-parse", "HEAD"], None));
    fs::write(source.join("README.md"), b"second\r\nraw\xff\0\r").unwrap();
    git(&source, &["add", "."], None);
    git(&source, &["commit", "-m", "MD only"], None);
    fs::rename(source.join("README.md"), source.join("Guide.MD")).unwrap();
    fs::write(source.join("code.txt"), b"code-v2\n").unwrap();
    git(&source, &["add", "."], None);
    git(&source, &["commit", "-m", "rename and code"], None);
    let main = text(&git(&source, &["rev-parse", "HEAD"], None));
    let tree = text(&git(&source, &["rev-parse", "HEAD^{tree}"], None));
    git(
        &source,
        &["tag", "-a", "tree-tag", "-m", "tree tag", &tree],
        None,
    );
    git(
        &source,
        &["tag", "-a", "nested", "-m", "nested tag", "tree-tag"],
        None,
    );
    let refs = text(&git(
        &source,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
        None,
    ));
    git(&source, &["checkout", "--detach", &base], None);
    let objects = text(&git(&source, &["rev-list", "--objects", "--all"], None));
    let ids: BTreeSet<_> = objects
        .lines()
        .map(|line| line.split_whitespace().next().unwrap().to_string())
        .collect();
    let interop_root = temp.path().join("interop");
    let content_root = temp.path().join("content");
    let packs = temp.path().join("native");
    let store =
        LocalGitObjectStore::open(db(&interop_root, &packs), content(&content_root, &packs))
            .unwrap();
    let source_index = store
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [1; 12],
            "fixture-source",
        )
        .unwrap();
    let alias_index = store
        .ensure_repository(GitRepositoryKind::AitSource, 0, [2; 12], "fixture-ait")
        .unwrap();
    let mut unique_bodies = BTreeSet::new();
    let mut raw_bytes = 0usize;
    for id in &ids {
        let kind = match text(&git(&source, &["cat-file", "-t", id], None)).as_str() {
            "commit" => GitObjectKind::Commit,
            "tree" => GitObjectKind::Tree,
            "blob" => GitObjectKind::Blob,
            "tag" => GitObjectKind::Tag,
            other => panic!("{other}"),
        };
        let body = git(&source, &["cat-file", kind.name(), id], None);
        raw_bytes += body.len();
        unique_bodies.insert(body.clone());
        let index = store.put(source_index, kind, 1, &oid(id), &body).unwrap();
        assert_eq!(
            store.put(source_index, kind, 1, &oid(id), &body).unwrap(),
            index
        );
        store.put(alias_index, kind, 1, &oid(id), &body).unwrap();
        let (source_row, _) = store.get(source_index, 1, &oid(id)).unwrap().unwrap();
        let (alias_row, _) = store.get(alias_index, 1, &oid(id)).unwrap().unwrap();
        assert_eq!(source_row.content_blob_index, alias_row.content_blob_index);
    }
    // Export reads only the admitted binary records and native zstd packs.
    fs::rename(source.join(".git"), temp.path().join("source-odb-held")).unwrap();
    drop(store);
    let store =
        LocalGitObjectStore::open(db(&interop_root, &packs), content(&content_root, &packs))
            .unwrap();
    let output = temp.path().join("export.git");
    fs::create_dir(&output).unwrap();
    git(&output, &["init", "--bare"], None);
    for id in &ids {
        let (record, body) = store.get(source_index, 1, &oid(id)).unwrap().unwrap();
        assert_eq!(
            text(&git(
                &output,
                &["hash-object", "-w", "--stdin", "-t", record.kind.name()],
                Some(&body)
            )),
            *id
        );
    }
    for line in refs.lines() {
        let mut fields = line.split_whitespace();
        git(
            &output,
            &["update-ref", fields.next().unwrap(), fields.next().unwrap()],
            None,
        );
    }
    fs::write(output.join("HEAD"), format!("{base}\n")).unwrap();
    git(&output, &["fsck", "--strict"], None);
    assert_eq!(
        text(&git(&output, &["rev-parse", "refs/heads/main"], None)),
        main
    );
    assert_eq!(
        git(&output, &["show", &format!("{base}:README.md")], None),
        b"first\r\n\xff\0raw\r"
    );
    assert_eq!(
        git(&output, &["show", &format!("{main}:Guide.MD")], None),
        b"second\r\nraw\xff\0\r"
    );
    assert_eq!(text(&git(&output, &["rev-parse", "nested^{}"], None)), tree);
    let read = BinaryDbReadTxn::new(store.db());
    assert_eq!(
        read.record_count(git_object_file()).unwrap(),
        (2 * ids.len()) as u32
    );
    drop(read);
    let native = content(&content_root, &packs);
    let read = BinaryDbReadTxn::new(native.db());
    assert_eq!(
        read.record_count(crate::binary_db::BinaryFileId::new(
            BLOB_BIN,
            1,
            crate::content_binary_db::BLOB_RECORD_SIZE
        ))
        .unwrap(),
        unique_bodies.len() as u32
    );
    assert_eq!(
        read.record_count(crate::binary_db::BinaryFileId::new(
            OBJECT_PACK_BIN,
            1,
            crate::content_binary_db::OBJECT_PACK_RECORD_SIZE
        ))
        .unwrap(),
        unique_bodies.len() as u32
    );
    let authority_files: Vec<_> = fs::read_dir(&interop_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect();
    assert_eq!(authority_files.len(), 5);
    assert!(!authority_files
        .iter()
        .any(|path| path.extension().is_some_and(|ext| ext == "json")));
    println!("native Git object fixture: {} source objects, {} aliases, {} unique Blob bodies, {raw_bytes} source body bytes, 5 interop files, 0 mapping JSON; object.bin={} bytes, object.idx={} bytes",
        ids.len(),2*ids.len(),unique_bodies.len(),fs::metadata(interop_root.join("git_object.bin")).unwrap().len(),fs::metadata(interop_root.join("git_object.idx")).unwrap().len());
    let binary_files = [stored_files(&interop_root), stored_files(&content_root)].concat();
    let pack_files = stored_files(&packs);
    assert_eq!(pack_files.len(), unique_bodies.len());
    assert!(binary_files
        .iter()
        .chain(&pack_files)
        .all(|path| { path.extension().is_none_or(|extension| extension != "json") }));
    let binary_bytes: u64 = binary_files
        .iter()
        .map(|path| fs::metadata(path).unwrap().len())
        .sum();
    let pack_bytes: u64 = pack_files
        .iter()
        .map(|path| fs::metadata(path).unwrap().len())
        .sum();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let allocated_bytes: u64 = binary_files
            .iter()
            .chain(&pack_files)
            .map(|path| fs::metadata(path).unwrap().blocks() * 512)
            .sum();
        println!("native footprint (excluding locks and source/output ODBs): {} binary/index/payload files ({binary_bytes} bytes), {} existing zstd object packs ({pack_bytes} bytes), {} logical bytes, {allocated_bytes} allocated bytes, 0 JSON files/bytes",
            binary_files.len(),pack_files.len(),binary_bytes+pack_bytes);
    }
}

#[test]
fn bounded_batches_reuse_metadata_and_create_two_packs_for_128_bodies() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("interop");
    let cr = temp.path().join("content");
    let workspace = temp.path().join("native");
    let store = LocalGitObjectStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    let source = store
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [4; 12], "batch-source")
        .unwrap();
    let alias = store
        .ensure_repository(GitRepositoryKind::AitSource, 0, [5; 12], "batch-alias")
        .unwrap();
    let bodies = (0..128)
        .map(|n| format!("bounded raw object {n}\r\n").into_bytes())
        .collect::<Vec<_>>();
    let ids = bodies
        .iter()
        .map(|body| git_object_id(GitObjectKind::Blob, body))
        .collect::<Vec<_>>();
    let inputs = bodies
        .iter()
        .zip(&ids)
        .map(|(body, id)| GitObjectInput {
            kind: GitObjectKind::Blob,
            format: 1,
            object_id: id,
            body,
        })
        .collect::<Vec<_>>();
    let first = store.put_batch(source, &inputs[..64]).unwrap();
    let second = store.put_batch(source, &inputs[64..]).unwrap();
    assert_eq!(
        store.put_batch(source, &inputs).unwrap(),
        [first, second].concat()
    );
    store.put_batch(alias, &inputs).unwrap();
    assert_eq!(stored_files(&workspace).len(), 2);
    assert_eq!(
        BinaryDbReadTxn::new(store.db())
            .record_count(git_object_file())
            .unwrap(),
        256
    );
    for (body, id) in bodies.iter().zip(&ids) {
        assert_eq!(store.get(alias, 1, id).unwrap().unwrap().1, *body);
    }
    println!("bounded batch fixture: 128 unique bodies, 256 aliases, 2 existing zstd packs, 0 JSON; live Git key/ordinal cache and incremental Blob metadata session");
}

#[test]
fn raw_plan_docsets_keep_exact_history_without_tasks_or_text_normalization() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("interop");
    let cr = temp.path().join("content");
    let workspace = temp.path().join("native");
    let store = LocalGitObjectStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    let source = store
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [6; 12],
            "raw-doc-source",
        )
        .unwrap();
    let mut documents =
        RawGitDocumentStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    let raw = b"first\r\n\xff\0\r";
    let first = documents
        .ensure_document_set(
            source,
            [6; 12],
            "raw-doc-source",
            &[
                RawGitDocumentInput {
                    path: "README.md",
                    mode: 0o755,
                    body: raw,
                },
                RawGitDocumentInput {
                    path: "docs/Link.MD",
                    mode: 0o120000,
                    body: b"../README.md",
                },
            ],
        )
        .unwrap();
    let second = documents
        .ensure_document_set(
            source,
            [6; 12],
            "raw-doc-source",
            &[RawGitDocumentInput {
                path: "Guide.MD",
                mode: 0o644,
                body: b"changed\r\n\xff\0",
            }],
        )
        .unwrap();
    let empty = documents
        .ensure_document_set(source, [6; 12], "raw-doc-source", &[])
        .unwrap();
    crate::binary_db::read_profile::reset();
    assert_eq!(
        documents.document_set_files(first).unwrap(),
        vec![
            ("README.md".into(), 0o755, raw.to_vec()),
            ("docs/Link.MD".into(), 0o120000, b"../README.md".to_vec())
        ]
    );
    assert_eq!(
        crate::binary_db::read_profile::snapshot()["tree_pack_locator_builds"].0,
        1
    );
    assert_eq!(
        documents.document_set_files(second).unwrap(),
        vec![("Guide.MD".into(), 0o644, b"changed\r\n\xff\0".to_vec())]
    );
    assert!(documents.document_set_files(empty).unwrap().is_empty());
    let records = fs::read(cr.join("plan_revision.bin")).unwrap();
    drop(documents);
    crate::binary_db::read_profile::reset();
    let mut documents =
        RawGitDocumentStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    assert_eq!(
        crate::binary_db::read_profile::snapshot()["tree_pack_locator_builds"].0,
        1
    );
    assert_eq!(
        documents
            .ensure_document_set(
                source,
                [6; 12],
                "raw-doc-source",
                &[RawGitDocumentInput {
                    path: "Guide.MD",
                    mode: 0o644,
                    body: b"changed\r\n\xff\0"
                },]
            )
            .unwrap(),
        second
    );
    assert_eq!(fs::read(cr.join("plan_revision.bin")).unwrap(), records);
    assert!(!cr.join("task.bin").exists());
    assert!(!cr.join("plan_item.bin").exists());
    let plan_before = fs::read(cr.join("plan.bin")).unwrap();
    fs::remove_file(root.join("git_document_revision.idx")).unwrap();
    assert!(RawGitDocumentStore::open(db(&root, &workspace), content(&cr, &workspace)).is_err());
    RawGitDocumentStore::rebuild_indexes(&db(&root, &workspace), &content(&cr, &workspace))
        .unwrap();
    let rebuilt =
        RawGitDocumentStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    assert_eq!(rebuilt.document_set_files(first).unwrap()[0].2, raw);
    assert_eq!(records, fs::read(cr.join("plan_revision.bin")).unwrap());
    assert_eq!(plan_before, fs::read(cr.join("plan.bin")).unwrap());
}

#[test]
fn missing_indexes_require_recovery_and_rebuild_preserves_bodies_and_ordinals() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("interop");
    let cr = temp.path().join("content");
    let workspace = temp.path().join("native");
    let store = LocalGitObjectStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    let repo = store
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [3; 12], "repo")
        .unwrap();
    let body = b"raw\r\n\xff\0";
    let id = git_object_id(GitObjectKind::Blob, body);
    let ordinal = store.put(repo, GitObjectKind::Blob, 1, &id, body).unwrap();
    let records = fs::read(root.join("git_object.bin")).unwrap();
    fs::remove_file(root.join("git_object.idx")).unwrap();
    assert!(store
        .put(repo, GitObjectKind::Blob, 1, &id, body)
        .unwrap_err()
        .contains("rebuild"));
    assert!(LocalGitObjectStore::open(db(&root, &workspace), content(&cr, &workspace)).is_err());
    drop(store);
    LocalGitObjectStore::rebuild_indexes(&db(&root, &workspace)).unwrap();
    let store = LocalGitObjectStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    assert_eq!(
        store.put(repo, GitObjectKind::Blob, 1, &id, body).unwrap(),
        ordinal
    );
    assert_eq!(store.get(repo, 1, &id).unwrap().unwrap().1, body);
    assert_eq!(fs::read(root.join("git_object.bin")).unwrap(), records);
    // An index with a valid size but an incorrect key is rejected on reopen.
    let mut bytes = fs::read(root.join("git_object.idx")).unwrap();
    bytes[9] ^= 1;
    fs::write(root.join("git_object.idx"), bytes).unwrap();
    drop(store);
    assert!(LocalGitObjectStore::open(db(&root, &workspace), content(&cr, &workspace)).is_err());
}

#[test]
fn unsupported_format_and_wrong_oid_do_not_create_content_or_records() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("interop");
    let cr = temp.path().join("content");
    let workspace = temp.path().join("native");
    let store = LocalGitObjectStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    assert_eq!(
        store
            .put(0, GitObjectKind::Blob, 2, &[0; 32], b"x")
            .unwrap_err()
            .kind(),
        BinaryDbErrorKind::Unsupported
    );
    assert_eq!(
        store
            .put(0, GitObjectKind::Blob, 1, &[0; 32], b"x")
            .unwrap_err()
            .kind(),
        BinaryDbErrorKind::Unsupported
    );
    assert!(store
        .put(0, GitObjectKind::Blob, 1, &[0; 20], b"x")
        .is_err());
    assert!(!cr.exists());
    assert!(!workspace.exists());
    assert!(!root.join("git_object.bin").exists());
    assert!(!root.join("git_repository.bin").exists());
}

#[test]
fn mapping_and_descriptor_enforce_authorized_widths_modes_and_raw_bytes() {
    let mapping = GitCommitMappingRecord {
        mapping_meta: 15,
        generation_index: 1,
        snapshot_index: 2,
        author_identity_index: 3,
        committer_identity_index: 4,
        object_id: [5; 20],
        tree_object_id: [6; 20],
        lfs_pointer_count: 7,
        created_at_s: u64::MAX,
        document_set_tree_index: 0,
    };
    let bytes = mapping.encode().unwrap();
    assert_eq!(&bytes[20..44], &[0; 24]);
    assert_eq!(&bytes[96..100], &[0; 4]);
    assert_eq!(GitCommitMappingRecord::decode(&bytes).unwrap(), mapping);
    assert_eq!(
        GitCommitMappingRecord::decode(&bytes[..96])
            .unwrap_err()
            .kind(),
        BinaryDbErrorKind::LayoutMismatch
    );
    let mut legacy_ranges = bytes;
    legacy_ranges[2] = 1;
    assert!(GitCommitMappingRecord::decode(&legacy_ranges).is_err());
    let body = b"\xff\0raw\r\n\r";
    let descriptor = GitDocumentVersionDescriptor {
        plan_revision_index: 0,
        git_mode: 0o755,
        raw_body_length: body.len() as u64,
        raw_body_sha256: Sha256::digest(body).into(),
    };
    let bytes = descriptor.encode().unwrap();
    assert_eq!(
        GitDocumentVersionDescriptor::decode(&bytes).unwrap(),
        descriptor
    );
    descriptor.verify_body(body).unwrap();
    assert!(descriptor.verify_body(b"normalized").is_err());
    let mut symlink = descriptor.clone();
    symlink.git_mode = 0o120000;
    assert!(symlink.verify_body(body).is_err());
    let mut gitlink = descriptor;
    gitlink.git_mode = 0o160000;
    assert!(gitlink.encode().is_err());
}

#[test]
#[ignore = "explicit isolated real mapping IO/decode/index profile"]
fn actual_mapping_read_cost_profile() {
    let input_path =
        std::path::PathBuf::from(std::env::var_os("AIT_MAPPING_PROFILE_INPUT").unwrap());
    let input =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&input_path).unwrap())
            .unwrap();
    let workspace = std::path::PathBuf::from(input["native_copy"].as_str().unwrap());
    let interop = db(&workspace.join(".ait/git-interop/v1/binary-v0"), &workspace);
    let content = content(&workspace.join(".ait/binary-db"), &workspace);
    let objects = LocalGitObjectStore::open_for_staged_repack(interop, content).unwrap();
    let oids = input["oids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| {
            let text = value.as_str().unwrap();
            let mut out = [0; 20];
            for (i, byte) in out.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap();
            }
            out
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for budget in [32usize, 128] {
        let mut session = crate::content_binary_db::BinaryBlobReadSession::for_cache_profile(
            budget * 1024 * 1024,
        );
        for mapping in 0..3 {
            session.clear_logical_values();
            crate::binary_db::read_profile::reset();
            let started = std::time::Instant::now();
            let mut bytes = 0usize;
            let mut peak = 0usize;
            for _phase in 0..2 {
                for oid in &oids {
                    let (record, body) = objects
                        .get_with_read_session(0, 1, oid, &mut session)
                        .unwrap()
                        .unwrap();
                    assert_eq!(record.kind, GitObjectKind::Blob);
                    assert_eq!(git_object_id(record.kind, &body), *oid);
                    bytes += body.len();
                    peak = peak.max(session.profile_cached_chunk_bytes());
                    assert!(peak <= budget * 1024 * 1024);
                }
            }
            let row = serde_json::json!({"budget_mib":budget,"mapping":mapping,"references":oids.len()*2,"logical_bytes_verified":bytes,"seconds":started.elapsed().as_secs_f64(),"peak_cached_chunk_bytes":peak,"io_counters":crate::binary_db::read_profile::snapshot()});
            println!("{:?}", row);
            rows.push(row);
        }
    }
    let result = serde_json::json!({"status":"passed","logical_payload_cleared_each_mapping":true,"source_oid_and_native_sha_checks_retained":true,"rows":rows});
    fs::write(
        input_path
            .parent()
            .unwrap()
            .join("baseline-cache-profile.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "explicit isolated real mapping readonly metadata reuse profile"]
fn actual_mapping_read_reuse_cost_profile() {
    let input_path =
        std::path::PathBuf::from(std::env::var_os("AIT_MAPPING_PROFILE_INPUT").unwrap());
    let input: serde_json::Value = serde_json::from_slice(&fs::read(&input_path).unwrap()).unwrap();
    let workspace = std::path::PathBuf::from(input["native_copy"].as_str().unwrap());
    let interop = db(&workspace.join(".ait/git-interop/v1/binary-v0"), &workspace);
    let content = content(&workspace.join(".ait/binary-db"), &workspace);
    let objects = LocalGitObjectStore::open_for_staged_repack(interop, content).unwrap();
    let oids = input["oids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let text = v.as_str().unwrap();
            let mut id = [0; 20];
            for (i, b) in id.iter_mut().enumerate() {
                *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap();
            }
            id
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for budget in [32usize, 128, 256, 512] {
        let mut bodies = crate::content_binary_db::BinaryBlobReadSession::for_cache_profile(
            budget * 1024 * 1024,
        );
        crate::binary_db::read_profile::reset();
        let mut session = objects.read_session(&mut bodies);
        let setup_transactions = crate::binary_db::read_profile::snapshot();
        for mapping in 0..3 {
            session.body_read.clear_logical_values();
            crate::binary_db::read_profile::reset();
            let started = std::time::Instant::now();
            let mut bytes = 0usize;
            let mut peak = 0usize;
            for _phase in 0..2 {
                for oid in &oids {
                    let (record, body) = session.get(0, 1, oid).unwrap().unwrap();
                    assert_eq!(record.kind, GitObjectKind::Blob);
                    assert_eq!(git_object_id(record.kind, &body), *oid);
                    bytes += body.len();
                    peak = peak.max(session.body_read.profile_cached_chunk_bytes());
                    assert!(peak <= budget * 1024 * 1024);
                }
            }
            let row = serde_json::json!({"budget_mib":budget,"mapping":mapping,"references":oids.len()*2,"logical_bytes_verified":bytes,"seconds":started.elapsed().as_secs_f64(),"peak_cached_chunk_bytes":peak,"setup_transactions":setup_transactions,"io_counters":crate::binary_db::read_profile::snapshot()});
            println!("{row}");
            rows.push(row);
        }
    }
    let result = serde_json::json!({"status":"passed","metadata_transactions_readonly_and_scoped":true,"logical_bodies_cleared_each_mapping":true,"all_full_body_sha_oid_checks_retained":true,"rows":rows});
    fs::write(
        input_path
            .parent()
            .unwrap()
            .join("readonly-reuse-profile.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}

#[test]
fn readonly_body_batches_share_metadata_and_recheck_pack_mutation() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let interop = db(&temp.path().join("interop"), &workspace);
    let content = content(&temp.path().join("content"), &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    let repo = objects
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [7; 12], "reuse-guard")
        .unwrap();
    let body = vec![7; 16384];
    let oid = git_object_id(GitObjectKind::Blob, &body);
    objects
        .put(repo, GitObjectKind::Blob, 1, &oid, &body)
        .unwrap();
    crate::binary_db::read_profile::reset();
    assert_eq!(
        objects
            .get_record_batch(repo, 1, &[oid; 128])
            .unwrap()
            .len(),
        128
    );
    let counts = crate::binary_db::read_profile::snapshot();
    assert_eq!(counts["full_record:git_object.bin"].0, 1);
    assert_eq!(counts["read_transactions"].0, 1);
    crate::binary_db::read_profile::reset();
    assert!(objects
        .get_batch(repo, 1, &[oid; 128])
        .unwrap()
        .iter()
        .all(|(_, b)| *b == body));
    let counts = crate::binary_db::read_profile::snapshot();
    assert_eq!(counts["full_record:git_object.bin"].0, 1);
    assert_eq!(counts["full_record:blob.bin"].0, 1);
    assert_eq!(counts["read_transactions"].0, 2);
    assert!(objects.get_batch(repo, 1, &[oid; 129]).is_err());
    assert!(objects.get_record_batch(repo, 1, &[oid; 129]).is_err());
    assert!(objects.get_batch(repo, 2, &[oid]).is_err());
    assert!(objects.get_record_batch(repo, 2, &[oid]).is_err());
    assert!(objects.get_batch(repo, 1, &[[9; 20]]).is_err());
    assert!(objects.get_record_batch(repo, 1, &[[9; 20]]).is_err());
    {
        crate::binary_db::read_profile::reset();
        let mut reader = objects.batch_reader();
        for _ in 0..4 {
            assert_eq!(
                reader.get_record_batch(repo, 1, &[oid; 128]).unwrap().len(),
                128
            );
            assert!(reader
                .get_batch(repo, 1, &[oid; 128])
                .unwrap()
                .iter()
                .all(|(_, bytes)| *bytes == body));
        }
        let counts = crate::binary_db::read_profile::snapshot();
        assert_eq!(counts["full_record:git_object.bin"].0, 1);
        assert_eq!(counts["full_record:blob.bin"].0, 1);
        let path = interop.authority_root().as_path().join("git_object.idx");
        fs::write(&path, fs::read(&path).unwrap()).unwrap();
        assert!(reader.get_record_batch(repo, 1, &[oid]).is_err());
        assert!(reader.get_record_batch(repo, 1, &[oid]).is_err());
    }
    let mut bodies = crate::content_binary_db::BinaryBlobReadSession::for_git_admission();
    {
        let mut session = objects.read_session(&mut bodies);
        assert_eq!(session.get(repo, 1, &oid).unwrap().unwrap().1, body);
    }
    bodies.clear_logical_values();
    let mut operation_reader = objects.batch_reader();
    assert_eq!(
        operation_reader.get_batch(repo, 1, &[oid]).unwrap()[0].1,
        body
    );
    let packs = workspace.join(".ait/objects/packs");
    for entry in fs::read_dir(packs).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "zstpack") {
            fs::write(path, b"corrupt changed pack").unwrap();
        }
    }
    assert!(objects
        .read_session(&mut bodies)
        .get(repo, 1, &oid)
        .is_err());
    assert!(objects.get_batch(repo, 1, &[oid]).is_err());
    assert!(operation_reader.get_batch(repo, 1, &[oid]).is_err());
    drop(operation_reader);
    assert!(LocalGitObjectStore::open(interop, content).is_err());
}

#[test]
fn docset_source_reads_share_metadata_and_release_locks_before_writes() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let root = temp.path().join("interop");
    let cr = temp.path().join("content");
    let objects =
        LocalGitObjectStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    let repo = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [4; 12],
            "docset-source-reuse",
        )
        .unwrap();
    let bodies = [b"exact\r\n\xff\0".as_slice(), b"nested old MD\r".as_slice()];
    let ids = bodies.map(|raw| git_object_id(GitObjectKind::Blob, raw));
    objects
        .put_batch(
            repo,
            &ids.iter()
                .zip(bodies)
                .map(|(id, body)| GitObjectInput {
                    kind: GitObjectKind::Blob,
                    format: 1,
                    object_id: id,
                    body,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let mut docs =
        RawGitDocumentStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    let inputs = [
        RawGitDocumentObjectInput {
            path: "README.md",
            mode: 0o755,
            object_id: ids[0],
        },
        RawGitDocumentObjectInput {
            path: "docs/Guide.MD",
            mode: 0o644,
            object_id: ids[1],
        },
    ];
    crate::binary_db::read_profile::reset();
    let first = docs
        .ensure_document_set_from_objects(&objects, repo, [4; 12], "docset-source-reuse", &inputs)
        .unwrap();
    assert_eq!(
        crate::binary_db::read_profile::snapshot()["full_record:git_object.bin"].0,
        1
    );
    assert_eq!(
        docs.document_set_files(first).unwrap(),
        vec![
            ("README.md".into(), 0o755, bodies[0].to_vec()),
            ("docs/Guide.MD".into(), 0o644, bodies[1].to_vec())
        ]
    );
    crate::binary_db::read_profile::reset();
    RawGitDocumentStore::open(db(&root, &workspace), content(&cr, &workspace)).unwrap();
    assert_eq!(
        crate::binary_db::read_profile::snapshot()["tree_pack_archive_opens"].0,
        1
    );
    let other = content(&temp.path().join("other-authority"), &workspace);
    let other_read = BinaryDbReadTxn::new(other.db());
    assert!(docs
        .document_set_files_with_read_scope(
            first,
            &mut Default::default(),
            &other_read,
            &mut Default::default()
        )
        .unwrap_err()
        .to_string()
        .contains("share the content authority"));
    drop(other_read);
    let revisions = fs::read(cr.join("plan_revision.bin")).unwrap();
    crate::binary_db::read_profile::reset();
    assert_eq!(
        docs.ensure_document_set_from_objects(
            &objects,
            repo,
            [4; 12],
            "docset-source-reuse",
            &inputs
        )
        .unwrap(),
        first
    );
    assert_eq!(
        crate::binary_db::read_profile::snapshot()["full_record:git_object.bin"].0,
        1
    );
    assert_eq!(fs::read(cr.join("plan_revision.bin")).unwrap(), revisions);
    for e in fs::read_dir(workspace.join(".ait/objects/packs")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "zstpack") {
            fs::write(p, b"changed corrupt pack").unwrap();
        }
    }
    assert!(docs
        .ensure_document_set_from_objects(&objects, repo, [4; 12], "docset-source-reuse", &inputs)
        .is_err());
    assert_eq!(fs::read(cr.join("plan_revision.bin")).unwrap(), revisions);
}

#[test]
fn admission_metadata_reuse_is_bounded_and_rejects_changed_authority() {
    use crate::content_binary_db::BinaryDbTreeReadCache;
    use crate::local_snapshot::LocalSnapshotTreeReadStore;
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let gr = temp.path().join("interop");
    let cr = temp.path().join("content");
    let c = content(&cr, &workspace);
    let objects = LocalGitObjectStore::open(db(&gr, &workspace), c.clone()).unwrap();
    let repo = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [3; 12],
            "scoped-admission",
        )
        .unwrap();
    let bodies = (0..512u32)
        .map(|i| i.to_le_bytes().repeat(8))
        .collect::<Vec<_>>();
    let oids = bodies
        .iter()
        .map(|b| git_object_id(GitObjectKind::Blob, b))
        .collect::<Vec<_>>();
    for offset in (0..512).step_by(128) {
        let inputs = (offset..offset + 128)
            .map(|i| GitObjectInput {
                kind: GitObjectKind::Blob,
                format: 1,
                object_id: &oids[i],
                body: &bodies[i],
            })
            .collect::<Vec<_>>();
        objects.put_batch(repo, &inputs).unwrap();
    }
    let code = oids
        .iter()
        .enumerate()
        .map(|(i, oid)| {
            (
                format!("nested/file-{i:04}.rs"),
                objects
                    .get_record(repo, 1, oid)
                    .unwrap()
                    .unwrap()
                    .content_blob_index,
                0o644,
            )
        })
        .collect::<Vec<_>>();
    let view = c
        .create_git_code_snapshot(
            "repo",
            "line",
            &[],
            Some("profile"),
            &code,
            &mut Default::default(),
        )
        .unwrap();
    let mut measurements = Vec::new();
    for optimized in [false, true] {
        crate::binary_db::read_profile::reset();
        let started = std::time::Instant::now();
        let mut bodies = crate::content_binary_db::BinaryBlobReadSession::for_git_admission();
        let session = objects.read_session(&mut bodies);
        let mut trees = BinaryDbTreeReadCache::for_git_admission();
        for _ in 0..16 {
            for oid in &oids {
                let record = if optimized {
                    session.record(repo, 1, oid)
                } else {
                    session.legacy_record_for_profile(repo, 1, oid)
                }
                .unwrap()
                .unwrap();
                assert_eq!(record.object_id, *oid);
            }
            let rows = if optimized {
                c.snapshots().snapshot_tree_file_rows_with_cache(
                    &session.content_read,
                    &view.snapshot_id,
                    &mut trees,
                )
            } else {
                c.snapshot_tree_file_rows(Some(&view.snapshot_id))
            }
            .unwrap();
            assert_eq!(rows.len(), 512);
            assert!(rows
                .iter()
                .all(|r| r.path.ends_with(".rs") && r.mode == "0o644"));
        }
        if optimized {
            session.verify_metadata_authority().unwrap();
            trees.verify_admission_stamps().unwrap();
            assert!(session.record(repo, 2, &oids[0]).is_err());
            assert!(session.record(repo + 1, 1, &oids[0]).is_err());
            assert!(session.record(repo, 1, &[0xff; 20]).unwrap().is_none());
        }
        let counts = crate::binary_db::read_profile::snapshot();
        measurements.push(serde_json::json!({"optimized":optimized,"metadata_lookups":8192,"native_tree_roots":16,"native_leaf_rows":8192,"seconds":started.elapsed().as_secs_f64(),"counters":counts}));
        if optimized {
            assert_eq!(counts["git_index_count_checks"].0, 2);
            assert_eq!(counts["tree_entry_decodes"].0, 2);
            assert!(trees.retained_payload_bytes_for_test() <= 32 * 1024 * 1024);
            let mut pressure = BinaryDbTreeReadCache::with_admission_budget_for_test(1024);
            assert_eq!(
                c.snapshots()
                    .snapshot_tree_file_rows_with_cache(
                        &session.content_read,
                        &view.snapshot_id,
                        &mut pressure
                    )
                    .unwrap()
                    .len(),
                512
            );
            assert!(pressure.retained_payload_bytes_for_test() <= 2048);
            let path = gr.join("git_object.bin");
            let mut bytes = fs::read(&path).unwrap();
            bytes[0] ^= 1;
            fs::write(&path, bytes).unwrap();
            assert!(session.verify_metadata_authority().is_err());
            trees.clear_archives();
            trees.clear_tree_entries();
            for entry in fs::read_dir(workspace.join(".ait/objects/tree-packs")).unwrap() {
                fs::write(entry.unwrap().path(), b"corrupt verified Tree pack").unwrap();
            }
            assert!(trees.verify_admission_stamps().is_err());
            assert!(pressure.verify_admission_stamps().is_err());
        } else {
            assert_eq!(counts["git_index_count_checks"].0, 8192);
            assert_eq!(counts["tree_entry_decodes"].0, 32);
        }
    }
    println!(
        "ADMISSION_REUSE_PROFILE {}",
        serde_json::to_string(&measurements).unwrap()
    );
}

#[path = "tests/partitioned_raw.rs"]
mod partitioned_raw;
