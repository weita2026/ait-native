use super::*;
use crate::content_binary_db::LocalContentBinaryDb;
use crate::line_binary_db::BinaryDbLineStore;
use crate::line_store::LineStore;
use crate::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotTreeReadStore, SnapshotAuthoringOptions,
};
use crate::task_store::TaskStore;
use crate::workflow_binary_db::{
    LocalChangeRecord, LocalLandRecord, LocalTaskRecord, SnapshotLinkRecord,
};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    repo: PathBuf,
    db: PathBuf,
    namespace: String,
    receipt_count: u32,
}

fn family(root: &Path, leaf: &str, layout: u32, rows: &[u8]) {
    let mut bytes = layout.to_le_bytes().to_vec();
    bytes.extend_from_slice(rows);
    fs::write(root.join(leaf), bytes).unwrap();
}

fn append_change(f: &Fixture, task: u32, ordinal: u8, previous: u32, published: bool) -> u32 {
    let path = f.db.join("change.bin");
    let mut bytes = fs::read(&path).unwrap_or_else(|_| 1u32.to_le_bytes().to_vec());
    let mut titles =
        fs::read(f.db.join("change_payload.bin")).unwrap_or_else(|_| 1u32.to_le_bytes().to_vec());
    let title = format!(
        "{} Task {task}, revision {ordinal}",
        if ordinal == 0 {
            "Bootstrap differs from Task title"
        } else {
            "Real revision"
        }
    );
    let row = LocalChangeRecord {
        change_meta: if task < f.receipt_count { 2 } else { 0 },
        local_meta: u8::from(published),
        payload_len: title.len() as u16,
        change_ordinal: ordinal,
        change_state: 0,
        reserved1: 0,
        payload_offset: titles.len() as u64,
        task_index: task,
        previous_change_index_plus1: previous,
        fork_snapshot_index_plus1: 1,
        published_remote_change_ordinal_plus1: u8::from(published) * 3,
        reserved2: 0,
        reserved3: 0,
        created_at_s: 1792454400 + u64::from(ordinal),
        updated_at_s: 1792454500 + u64::from(ordinal),
        published_at_s: if published { 1792454600 } else { 0 },
        base_line_index_plus1: 1,
        archived_at_s: 0,
    };
    let index = (bytes.len() - 4) / 68;
    bytes.extend_from_slice(&row.encode().unwrap());
    titles.extend_from_slice(title.as_bytes());
    fs::write(path, bytes).unwrap();
    fs::write(f.db.join("change_payload.bin"), titles).unwrap();
    index as u32
}

fn links(f: &Fixture, bindings: &[(u32, u32, u8, u32, u32)]) {
    let changes = fs::read(f.db.join("change.bin")).unwrap();
    let mut rows = Vec::new();
    let mut payload = 1u32.to_le_bytes().to_vec();
    for (snapshot, &(task, change, ordinal, previous_task, previous_change)) in
        bindings.iter().enumerate()
    {
        let fields = [
            "dirty-source-tree".to_string(),
            "main".into(),
            format!("L{}T-{:04}", f.namespace, task + 1),
            format!("C-{:02}", changes[4 + (change as usize - 1) * 68 + 4] + 1),
            "ai_with_human_review".into(),
        ];
        let mut text = Vec::new();
        for field in &fields {
            text.extend_from_slice(&(field.len() as u16).to_le_bytes());
        }
        for field in &fields {
            text.extend_from_slice(field.as_bytes());
        }
        rows.extend_from_slice(&[1 | 8 | 16 | 32, ordinal]);
        rows.extend_from_slice(&(text.len() as u16).to_le_bytes());
        rows.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        for value in [
            task,
            change,
            snapshot as u32,
            previous_task,
            previous_change,
        ] {
            rows.extend_from_slice(&value.to_le_bytes());
        }
        rows.extend_from_slice(&1792454500u64.to_le_bytes());
        payload.extend_from_slice(&text);
    }
    family(&f.db, "snapshot_link.bin", 1, &rows);
    fs::write(f.db.join("snapshot_link_payload.bin"), payload).unwrap();
}

fn fixture() -> Fixture {
    fixture_with("ait-server", "SE", 235, 132, 4, Some(168))
}

fn fixture_with(
    name: &str,
    namespace: &str,
    task_count: u32,
    receipt_count: u32,
    snapshot_count: u32,
    revision_task: Option<u32>,
) -> Fixture {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().canonicalize().unwrap();
    let db = repo.join(".ait/binary-db");
    fs::create_dir_all(&db).unwrap();
    fs::write(
        repo.join(".ait/config.json"),
        serde_json::to_vec(&serde_json::json!({"repo_name":name,"id_namespace_prefix":namespace}))
            .unwrap(),
    )
    .unwrap();
    let f = Fixture {
        _temp: temp,
        repo,
        db,
        namespace: namespace.to_string(),
        receipt_count,
    };
    let content = LocalContentBinaryDb::<1>::new(
        f.db.clone(),
        f.repo.as_path(),
        AuthorityId::new("local:server-fixture"),
        LocalStateScope::Repository,
    );
    let line = BinaryDbLineStore::<_, 1>::new(content.db().clone());
    line.create_line("main", None, "2026-10-20T00:00:00Z")
        .unwrap();
    let mut parents = Vec::new();
    for number in 0..snapshot_count {
        fs::write(
            f.repo.join("source.txt"),
            format!("source snapshot {number}"),
        )
        .unwrap();
        let result = content
            .create_snapshot_content_with_workflow_binding(
                name,
                "main",
                &parents,
                Some("fixture"),
                false,
                SnapshotAuthoringOptions::default(),
                None,
            )
            .unwrap();
        parents = vec![result["snapshot_id"].as_str().unwrap().to_string()];
    }
    line.set_line_head("main", Some(&parents[0]), "2026-10-20T00:00:00Z")
        .unwrap();
    let store =
        BinaryDbWorkflowStore::<_, 1>::new_with_namespace(content.db().clone(), name, namespace);
    store
        .create_task(
            name,
            "Task title",
            "Keep intent",
            Some(namespace),
            None,
            None,
            None,
        )
        .unwrap();
    let template = fs::read(f.db.join("task.bin")).unwrap()[4..].to_vec();
    let mut tasks = Vec::new();
    for index in 0..task_count {
        let mut task = LocalTaskRecord::decode(&template).unwrap();
        if index < receipt_count {
            task.task_meta |= 0x40;
            task.closed_at_s = task.updated_at_s;
        }
        tasks.extend_from_slice(&task.encode().unwrap());
    }
    family(&f.db, "task.bin", 1, &tasks);
    for task in 0..task_count {
        let index = append_change(&f, task, 0, 0, false);
        if Some(task) == revision_task {
            append_change(&f, task, 1, index + 1, false);
        }
    }
    let mut receipts = Vec::new();
    for task in 0..receipt_count {
        receipts.extend_from_slice(
            &LocalLandRecord {
                land_meta: 2 | 8 | 16,
                land_ordinal: 0,
                change_ordinal_plus1: 0,
                failure_kind: 0,
                task_index: task,
                previous_task_land_index_plus1: 0,
                previous_change_land_index_plus1: 0,
                pre_land_target_snapshot_index_plus1: 1,
                landed_snapshot_index_plus1: 2,
                submitted_at_s: 1792454500,
                updated_at_s: 1792454500,
                target_line_index_plus1: 1,
            }
            .encode()
            .unwrap(),
        );
    }
    family(&f.db, "land.bin", 1, &receipts);
    if revision_task == Some(168) {
        links(&f, &[(0, 1, 0, 0, 0), (168, 170, 0, 0, 0)]);
    } else {
        let bindings = (0..task_count.min(snapshot_count))
            .map(|task| (task, task + 1, 0, 0, 0))
            .collect::<Vec<_>>();
        links(&f, &bindings);
    }
    f
}

fn affected(f: &Fixture) -> Vec<(&'static str, Option<Vec<u8>>)> {
    super::super::local_change_cleanup::CHANGED_FILES
        .iter()
        .map(|leaf| (*leaf, fs::read(f.db.join(leaf)).ok()))
        .collect()
}

#[test]
fn server_baseline_conversion_preserves_success_content_and_target_reopen() {
    let f = fixture();
    let tasks = fs::read(f.db.join("task.bin")).unwrap();
    let snapshots = fs::read(f.db.join("snapshot.bin")).unwrap();
    let receipts = fs::read(f.db.join("land.bin")).unwrap();
    let started = std::time::Instant::now();
    admit_local_receipt_upgrade(&f.repo, &f.db, "ait-server").unwrap();
    let conversion_elapsed = started.elapsed();
    assert_eq!(fs::read(f.db.join("task.bin")).unwrap(), tasks);
    assert_eq!(fs::read(f.db.join("snapshot.bin")).unwrap(), snapshots);
    let converted = fs::read(f.db.join("land.bin")).unwrap();
    assert_eq!(converted.len(), receipts.len());
    assert_eq!(&converted[4..], &receipts[4..]);
    let changes = fs::read(f.db.join("change.bin")).unwrap();
    let retained = LocalChangeRecord::decode(&changes[4..]).unwrap();
    assert_eq!(
        (
            retained.task_index,
            retained.change_ordinal,
            retained.previous_change_index_plus1
        ),
        (168, 0, 0)
    );
    assert_eq!(retained.fork_snapshot_index_plus1, 1);
    assert!(!f.db.join("snapshot_link_payload.bin").exists());
    let links = fs::read(f.db.join("snapshot_link.bin")).unwrap();
    let first = SnapshotLinkRecord::decode(&links[4..26]).unwrap();
    let second = SnapshotLinkRecord::decode(&links[26..48]).unwrap();
    assert_eq!((first.task, first.change_plus1, first.snapshot), (0, 0, 0));
    assert_eq!(
        (second.task, second.change_plus1, second.snapshot),
        (168, 1, 1)
    );
    let before = affected(&f);
    let started = std::time::Instant::now();
    let (_, guard) =
        super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
            &f.repo,
            &f.db,
            "ait-server",
        )
        .unwrap();
    let runtime_open_elapsed = started.elapsed();
    drop(guard);
    assert_eq!(affected(&f), before);
    eprintln!("isolated Server conversion={conversion_elapsed:?}; target runtime-open={runtime_open_elapsed:?}");
    let content = LocalContentBinaryDb::<1>::new(
        f.db.clone(),
        f.repo.as_path(),
        AuthorityId::new("local:server-fixture"),
        LocalStateScope::Repository,
    );
    let store =
        BinaryDbWorkflowStore::<_, 1>::new_with_namespace(content.db().clone(), "ait-server", "SE");
    let task = store.get_task("LSET-0001").unwrap();
    assert_eq!(task["status"], "completed");
    assert!(task["local_finish"]["change_ref"].is_null());
    let id = task["local_finish"]["landed_snapshot_id"].as_str().unwrap();
    let files = content.snapshot_tree_file_rows(Some(id)).unwrap();
    assert_eq!(
        content.read_blob_bytes(&files[0].blob_id).unwrap(),
        b"source snapshot 1"
    );
}

#[test]
fn server_appended_revisions_remap_receipts_links_and_dirty_cursors() {
    let f = fixture();
    let mut tasks = fs::read(f.db.join("task.bin")).unwrap();
    let mut task = LocalTaskRecord::decode(&tasks[4 + 234 * 64..][..64]).unwrap();
    task.local_meta = 1;
    task.published_remote_task_index = 700;
    task.published_at_s = task.updated_at_s;
    task.task_meta |= 0x40;
    task.closed_at_s = task.updated_at_s;
    tasks.extend_from_slice(&task.encode().unwrap());
    fs::write(f.db.join("task.bin"), &tasks).unwrap();
    let initial = append_change(&f, 235, 0, 0, false);
    let revision1 = append_change(&f, 235, 1, initial + 1, false);
    let revision2 = append_change(&f, 235, 2, revision1 + 1, true);
    let existing_revision2 = append_change(&f, 168, 2, 170, false);
    let path = f.db.join("change.bin");
    let mut bytes = fs::read(&path).unwrap();
    let raw = &mut bytes[4 + existing_revision2 as usize * 68..][..68];
    let mut canceled = LocalChangeRecord::decode(raw).unwrap();
    canceled.change_meta = 3;
    canceled.change_state = 1;
    canceled.archived_at_s = canceled.updated_at_s;
    canceled.fork_snapshot_index_plus1 = 0;
    raw.copy_from_slice(&canceled.encode().unwrap());
    fs::write(path, bytes).unwrap();
    links(
        &f,
        &[
            (168, 170, 0, 0, 0),
            (235, revision1 + 1, 0, 0, 0),
            (168, existing_revision2 + 1, 1, 1, 0),
            (235, revision2 + 1, 1, 2, 0),
        ],
    );
    let mut receipts = fs::read(f.db.join("land.bin")).unwrap();
    let mut receipt = LocalLandRecord::decode(&receipts[4..48]).unwrap();
    receipt.task_index = revision2;
    receipt.change_ordinal_plus1 = 2;
    receipts.extend_from_slice(&receipt.encode().unwrap());
    fs::write(f.db.join("land.bin"), &receipts).unwrap();
    let worktree = f.repo.join("dirty-worktree");
    fs::create_dir_all(worktree.join(".ait-worktree")).unwrap();
    fs::create_dir_all(f.repo.join(".ait/worktrees")).unwrap();
    fs::write(worktree.join("uncommitted.rs"), "keep actual dirty work").unwrap();
    let value = serde_json::json!({"repo_root":f.repo,"path":worktree,"bound_task_id":"LSET-0236","bound_change_ref":"LSET-0236/C-03","pending_snapshot_id":"SNP-UNCHANGED"});
    fs::write(
        f.repo.join(".ait/worktrees/dirty.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    fs::write(
        worktree.join(".ait-worktree.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let mut cursor = vec![0; 32];
    cursor[..4].copy_from_slice(&1u32.to_le_bytes());
    cursor[4] = 5;
    cursor[8..12].copy_from_slice(&235u32.to_le_bytes());
    cursor[12..16].copy_from_slice(&(revision2 + 1).to_le_bytes());
    cursor[20..24].copy_from_slice(&3u32.to_le_bytes());
    fs::write(worktree.join(".ait-worktree/cursor.bin"), &cursor).unwrap();
    let initial_worktree = f.repo.join("initial-worktree");
    fs::create_dir_all(initial_worktree.join(".ait-worktree")).unwrap();
    let initial = serde_json::json!({"repo_root":f.repo,"path":initial_worktree,"bound_task_id":"LSET-0001","bound_change_id":"C-01"});
    fs::write(
        f.repo.join(".ait/worktrees/initial.json"),
        serde_json::to_vec(&initial).unwrap(),
    )
    .unwrap();
    fs::write(
        initial_worktree.join(".ait-worktree.json"),
        serde_json::to_vec(&initial).unwrap(),
    )
    .unwrap();
    let mut initial_cursor = vec![0; 32];
    initial_cursor[..4].copy_from_slice(&1u32.to_le_bytes());
    initial_cursor[4] = 7;
    initial_cursor[12..16].copy_from_slice(&1u32.to_le_bytes());
    initial_cursor[16..20].copy_from_slice(&1u32.to_le_bytes());
    initial_cursor[20..24].copy_from_slice(&2u32.to_le_bytes());
    fs::write(
        initial_worktree.join(".ait-worktree/cursor.bin"),
        &initial_cursor,
    )
    .unwrap();
    admit_local_receipt_upgrade(&f.repo, &f.db, "ait-server").unwrap();
    let bytes = fs::read(f.db.join("change.bin")).unwrap();
    let changes = bytes[4..]
        .chunks_exact(68)
        .map(|raw| LocalChangeRecord::decode(raw).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        changes
            .iter()
            .map(|row| (
                row.task_index,
                row.change_ordinal,
                row.previous_change_index_plus1
            ))
            .collect::<Vec<_>>(),
        [(168, 0, 0), (235, 0, 0), (235, 1, 2), (168, 1, 1)]
    );
    assert_eq!(changes[2].published_remote_change_ordinal_plus1, 3);
    assert_eq!(changes[2].published_at_s, 1792454600);
    assert_eq!(
        (
            changes[3].change_meta,
            changes[3].change_state,
            changes[3].archived_at_s,
            changes[3].fork_snapshot_index_plus1
        ),
        (3, 1, canceled.archived_at_s, 0)
    );
    let receipts = fs::read(f.db.join("land.bin")).unwrap();
    let last = LocalLandRecord::decode(&receipts[4 + 132 * 44..]).unwrap();
    assert_eq!((last.task_index, last.change_ordinal_plus1), (235, 2));
    let links = fs::read(f.db.join("snapshot_link.bin")).unwrap();
    assert_eq!(
        links[4..]
            .chunks_exact(22)
            .map(|raw| SnapshotLinkRecord::decode(raw).unwrap().change_plus1)
            .collect::<Vec<_>>(),
        [1, 2, 4, 3]
    );
    let after: serde_json::Value =
        serde_json::from_slice(&fs::read(worktree.join(".ait-worktree.json")).unwrap()).unwrap();
    assert_eq!(after["bound_change_ref"], "LSET-0236/C-02");
    assert_eq!(after["pending_snapshot_id"], "SNP-UNCHANGED");
    cursor[12..16].copy_from_slice(&3u32.to_le_bytes());
    assert_eq!(
        fs::read(worktree.join(".ait-worktree/cursor.bin")).unwrap(),
        cursor
    );
    assert_eq!(
        fs::read_to_string(worktree.join("uncommitted.rs")).unwrap(),
        "keep actual dirty work"
    );
    let initial: serde_json::Value =
        serde_json::from_slice(&fs::read(initial_worktree.join(".ait-worktree.json")).unwrap())
            .unwrap();
    assert!(initial["bound_change_id"].is_null());
    initial_cursor[4] = 6;
    initial_cursor[12..16].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        fs::read(initial_worktree.join(".ait-worktree/cursor.bin")).unwrap(),
        initial_cursor
    );
    let before = affected(&f);
    let (_, guard) =
        super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
            &f.repo,
            &f.db,
            "ait-server",
        )
        .unwrap();
    drop(guard);
    assert_eq!(affected(&f), before);
}

#[test]
fn server_unsupported_sources_reject_before_metadata_mutation() {
    for fault in [
        "namespace",
        "owner",
        "partial",
        "task_width",
        "snapshot_width",
        "change_chain",
        "revision_publication",
        "title_bounds",
        "duplicate_receipt",
        "receipt_snapshot",
        "link_snapshot",
        "link_owner",
        "duplicate_link",
        "link_previous",
        "index_width",
        "interrupted_change_cleanup",
    ] {
        let f = fixture();
        let patch = |leaf: &str, offset: usize, raw: &[u8]| {
            let path = f.db.join(leaf);
            let mut bytes = fs::read(&path).unwrap();
            bytes[offset..offset + raw.len()].copy_from_slice(raw);
            fs::write(path, bytes).unwrap();
        };
        match fault {
            "namespace" => fs::write(
                f.repo.join(".ait/config.json"),
                br#"{"repo_name":"ait-server","id_namespace_prefix":"S"}"#,
            )
            .unwrap(),
            "owner" => fs::write(
                f.repo.join(".ait/config.json"),
                br#"{"repo_name":"other","id_namespace_prefix":"SE"}"#,
            )
            .unwrap(),
            "partial" => patch("snapshot_link.bin", 0, &2u32.to_le_bytes()),
            "task_width" => {
                let mut b = fs::read(f.db.join("task.bin")).unwrap();
                b.pop();
                fs::write(f.db.join("task.bin"), b).unwrap();
            }
            "snapshot_width" => {
                let mut b = fs::read(f.db.join("snapshot.bin")).unwrap();
                b.pop();
                fs::write(f.db.join("snapshot.bin"), b).unwrap();
            }
            "change_chain" => patch("change.bin", 4 + 169 * 68 + 20, &1u32.to_le_bytes()),
            "revision_publication" => {
                patch("change.bin", 4 + 169 * 68 + 1, &[1]);
                patch("change.bin", 4 + 169 * 68 + 28, &[1]);
            }
            "title_bounds" => patch("change.bin", 4 + 8, &u64::MAX.to_le_bytes()),
            "duplicate_receipt" => {
                let mut bytes = fs::read(f.db.join("land.bin")).unwrap();
                bytes.extend_from_within(4..48);
                fs::write(f.db.join("land.bin"), bytes).unwrap();
            }
            "receipt_snapshot" => patch("land.bin", 4 + 20, &5u32.to_le_bytes()),
            "link_snapshot" => patch("snapshot_link.bin", 4 + 20, &4u32.to_le_bytes()),
            "link_owner" => patch("snapshot_link.bin", 4 + 12, &1u32.to_le_bytes()),
            "duplicate_link" => patch("snapshot_link.bin", 4 + 40 + 20, &0u32.to_le_bytes()),
            "link_previous" => patch("snapshot_link.bin", 4 + 24, &1u32.to_le_bytes()),
            "index_width" => family(&f.db, "change_snapshot_index.bin", 1, &[0]),
            "interrupted_change_cleanup" => family(&f.db, "change.bin", 1, &[]),
            _ => unreachable!(),
        }
        let before = affected(&f);
        assert!(
            admit_local_receipt_upgrade(&f.repo, &f.db, "ait-server").is_err(),
            "{fault}"
        );
        assert_eq!(affected(&f), before, "{fault}");
    }
}

#[test]
fn server_missing_change_owner_and_foreign_cursor_fail_closed() {
    for fault in [
        "change_owner",
        "cursor_namespace",
        "cursor_owner",
        "cursor_missing_task",
    ] {
        let f = fixture();
        if fault == "change_owner" {
            let path = f.db.join("land.bin");
            let mut bytes = fs::read(&path).unwrap();
            bytes[8..12].copy_from_slice(&999u32.to_le_bytes());
            fs::write(path, bytes).unwrap();
        } else {
            fs::create_dir_all(f.repo.join(".ait/worktrees")).unwrap();
            let mut value = serde_json::json!({"repo_root":f.repo,"bound_task_id":if fault=="cursor_namespace" {"LCT-0169"} else {"LSET-0169"},"bound_change_ref":"LSET-0001/C-01"});
            if fault == "cursor_missing_task" {
                value["bound_task_id"] = "LSET-9999".into();
                value["bound_change_ref"] = serde_json::Value::Null;
            }
            fs::write(
                f.repo.join(".ait/worktrees/foreign.json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
        }
        let before = affected(&f);
        assert!(
            admit_local_receipt_upgrade(&f.repo, &f.db, "ait-server").is_err(),
            "{fault}"
        );
        assert_eq!(affected(&f), before);
    }
}

#[test]
fn known_native_format_admits_twenty_tasks_without_repository_or_count_approval() {
    for (name, namespace) in [
        ("ait-native", "N"),
        ("ait-web", "WB"),
        ("other-owner", ""),
        ("unrelated", "C"),
        ("ait-core", "C"),
        ("ait-runner", "R"),
    ] {
        let f = fixture_with(name, namespace, 20, 19, 20, None);
        let tasks = fs::read(f.db.join("task.bin")).unwrap();
        let snapshots = fs::read(f.db.join("snapshot.bin")).unwrap();
        let receipts = fs::read(f.db.join("land.bin")).unwrap();
        #[cfg(unix)]
        let snapshot_inode = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(f.db.join("snapshot.bin")).unwrap().ino()
        };
        let (_, guard) =
            super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
                &f.repo, &f.db, name,
            )
            .unwrap();
        drop(guard);
        assert_eq!(fs::read(f.db.join("task.bin")).unwrap(), tasks);
        assert_eq!(fs::read(f.db.join("snapshot.bin")).unwrap(), snapshots);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(f.db.join("snapshot.bin")).unwrap().ino(),
                snapshot_inode
            );
        }
        assert!(!f.db.join("change.bin").exists());
        assert!(!f.db.join("change_payload.bin").exists());
        let target_receipts = fs::read(f.db.join("land.bin")).unwrap();
        assert_eq!(target_receipts.len(), 4 + 19 * 44);
        assert_eq!(&target_receipts[4..], &receipts[4..]);
        let links = fs::read(f.db.join("snapshot_link.bin")).unwrap();
        assert_eq!(links.len(), 4 + 20 * 22);
        for (index, raw) in links[4..].chunks_exact(22).enumerate() {
            let link = SnapshotLinkRecord::decode(raw).unwrap();
            assert_eq!(
                (link.task, link.change_plus1, link.snapshot, link.ordinal),
                (index as u32, 0, index as u32, 0)
            );
        }
        let before = affected(&f);
        let (_, guard) =
            super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
                &f.repo, &f.db, name,
            )
            .unwrap();
        drop(guard);
        assert_eq!(affected(&f), before);
    }
}

#[test]
fn known_format_preserves_real_revisions_and_publication_without_historical_disposition() {
    let f = fixture_with("ait-native", "N", 20, 19, 22, None);
    let path = f.db.join("task.bin");
    let mut tasks = fs::read(&path).unwrap();
    let raw = &mut tasks[4 + 19 * 64..][..64];
    let mut task = LocalTaskRecord::decode(raw).unwrap();
    task.local_meta = 1;
    task.published_remote_task_index = 55;
    task.published_at_s = task.updated_at_s;
    raw.copy_from_slice(&task.encode().unwrap());
    fs::write(path, &tasks).unwrap();
    let revision1 = append_change(&f, 19, 1, 20, true);
    let revision2 = append_change(&f, 19, 2, revision1 + 1, false);
    let path = f.db.join("change.bin");
    let mut changes = fs::read(&path).unwrap();
    let raw = &mut changes[4 + revision2 as usize * 68..][..68];
    let mut canceled = LocalChangeRecord::decode(raw).unwrap();
    canceled.change_meta = 3;
    canceled.change_state = 1;
    canceled.archived_at_s = canceled.updated_at_s;
    raw.copy_from_slice(&canceled.encode().unwrap());
    fs::write(path, changes).unwrap();
    let mut bindings = (0..20)
        .map(|task| (task, task + 1, 0, 0, 0))
        .collect::<Vec<_>>();
    bindings.extend([(19, revision1 + 1, 1, 20, 0), (19, revision2 + 1, 2, 21, 0)]);
    links(&f, &bindings);
    let (_, guard) =
        super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
            &f.repo,
            &f.db,
            "ait-native",
        )
        .unwrap();
    drop(guard);
    assert_eq!(fs::read(f.db.join("task.bin")).unwrap(), tasks);
    let changes = fs::read(f.db.join("change.bin")).unwrap();
    assert_eq!(changes.len(), 4 + 2 * 68);
    let first = LocalChangeRecord::decode(&changes[4..72]).unwrap();
    let second = LocalChangeRecord::decode(&changes[72..140]).unwrap();
    assert_eq!(
        (
            first.task_index,
            first.change_ordinal,
            first.local_meta,
            first.published_remote_change_ordinal_plus1
        ),
        (19, 0, 1, 3)
    );
    assert_eq!(
        (
            second.task_index,
            second.change_ordinal,
            second.previous_change_index_plus1,
            second.change_state,
            second.archived_at_s
        ),
        (19, 1, 1, 1, canceled.archived_at_s)
    );
    let links = fs::read(f.db.join("snapshot_link.bin")).unwrap();
    assert_eq!(
        SnapshotLinkRecord::decode(&links[4 + 20 * 22..][..22])
            .unwrap()
            .change_plus1,
        1
    );
    assert_eq!(
        SnapshotLinkRecord::decode(&links[4 + 21 * 22..][..22])
            .unwrap()
            .change_plus1,
        2
    );
}

#[test]
fn another_owner_with_core_namespace_keeps_its_publication_edge() {
    for (name, remote) in [("another-owner", 981), ("ait-core", 55)] {
        let f = fixture_with(name, "C", 20, 0, 21, None);
        let path = f.db.join("task.bin");
        let mut tasks = fs::read(&path).unwrap();
        let raw = &mut tasks[4 + 2 * 64..][..64];
        let mut task = LocalTaskRecord::decode(raw).unwrap();
        task.task_meta = 0x80;
        task.local_meta = 1;
        task.published_remote_task_index = remote;
        task.published_at_s = task.updated_at_s;
        raw.copy_from_slice(&task.encode().unwrap());
        fs::write(path, tasks).unwrap();
        let revision = append_change(&f, 2, 1, 3, true);
        let path = f.db.join("change.bin");
        let mut changes = fs::read(&path).unwrap();
        let raw = &mut changes[4 + revision as usize * 68..][..68];
        let mut canceled = LocalChangeRecord::decode(raw).unwrap();
        canceled.change_meta = 3;
        canceled.change_state = 1;
        canceled.archived_at_s = canceled.updated_at_s;
        canceled.published_remote_change_ordinal_plus1 = 1;
        raw.copy_from_slice(&canceled.encode().unwrap());
        fs::write(path, changes).unwrap();
        let mut bindings = (0..20)
            .map(|task| (task, task + 1, 0, 0, 0))
            .collect::<Vec<_>>();
        bindings.push((2, revision + 1, 1, 3, 0));
        links(&f, &bindings);
        let (_, guard) =
            super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
                &f.repo, &f.db, name,
            )
            .unwrap();
        drop(guard);
        let changes = fs::read(f.db.join("change.bin")).unwrap();
        let retained = LocalChangeRecord::decode(&changes[4..]).unwrap();
        assert_eq!(
            (
                retained.task_index,
                retained.change_ordinal,
                retained.local_meta,
                retained.published_remote_change_ordinal_plus1
            ),
            (2, 0, 1, 1)
        );
        assert_eq!(retained.published_at_s, canceled.published_at_s);
    }
}

fn legacy_overlay(f: &Fixture, task: u32, selected: Option<u32>) -> (PathBuf, Vec<u8>, Vec<u8>) {
    let worktree = f.repo.join("legacy-task");
    fs::create_dir_all(worktree.join(".ait-worktree")).unwrap();
    fs::create_dir_all(f.repo.join(".ait/worktrees")).unwrap();
    let task_id = format!("L{}T-{:04}", f.namespace, task + 1);
    let registry = serde_json::json!({"name":"legacy-task", "repo_root":f.repo,"path":worktree,"bound_task_id":task_id,"bound_change_id":selected.map(|_| "C-01")});
    fs::write(
        f.repo.join(".ait/worktrees/legacy-task.json"),
        serde_json::to_vec(&registry).unwrap(),
    )
    .unwrap();
    let overlay = serde_json::json!({"created_at":"2026-10-20T00:00:00Z","current_line":"main","materialized_snapshot_id":"SNP-UNCHANGED","repo_root":f.repo,"workspace_root":worktree,"worktree_name":"legacy-task"});
    let overlay = serde_json::to_vec(&overlay).unwrap();
    fs::write(worktree.join(".ait-worktree.json"), &overlay).unwrap();
    fs::write(worktree.join("dirty.rs"), "keep unfinished work").unwrap();
    let mut cursor = vec![0; 32];
    cursor[..4].copy_from_slice(&1u32.to_le_bytes());
    cursor[4] = 4 | u8::from(selected.is_some());
    cursor[8..12].copy_from_slice(&task.to_le_bytes());
    cursor[12..16].copy_from_slice(&selected.map_or(0, |index| index + 1).to_le_bytes());
    cursor[20..24].copy_from_slice(&2u32.to_le_bytes());
    fs::write(worktree.join(".ait-worktree/cursor.bin"), &cursor).unwrap();
    (worktree, overlay, cursor)
}

#[test]
fn registered_legacy_overlay_without_task_fields_stays_unchanged() {
    for case in ["numeric_cursor", "no_cursor", "task_without_change"] {
        let f = fixture_with("ait-server", "SE", 20, 19, 20, None);
        let task = if case == "task_without_change" {
            let path = f.db.join("task.bin");
            let mut tasks = fs::read(&path).unwrap();
            tasks.extend_from_within(4 + 19 * 64..4 + 20 * 64);
            fs::write(path, tasks).unwrap();
            20
        } else {
            19
        };
        let selected = (case != "task_without_change").then_some(task);
        let (worktree, overlay, mut cursor) = legacy_overlay(&f, task, selected);
        if case == "no_cursor" {
            fs::remove_file(worktree.join(".ait-worktree/cursor.bin")).unwrap();
        }
        let (_, guard) =
            super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
                &f.repo,
                &f.db,
                "ait-server",
            )
            .unwrap();
        drop(guard);
        assert_eq!(
            fs::read(worktree.join(".ait-worktree.json")).unwrap(),
            overlay
        );
        assert_eq!(
            fs::read_to_string(worktree.join("dirty.rs")).unwrap(),
            "keep unfinished work"
        );
        if case != "no_cursor" {
            cursor[4] &= !1;
            cursor[12..16].copy_from_slice(&0u32.to_le_bytes());
            assert_eq!(
                fs::read(worktree.join(".ait-worktree/cursor.bin")).unwrap(),
                cursor
            );
        }
        let registry: serde_json::Value = serde_json::from_slice(
            &fs::read(f.repo.join(".ait/worktrees/legacy-task.json")).unwrap(),
        )
        .unwrap();
        assert!(registry["bound_change_id"].is_null());
    }
}

#[test]
fn legacy_overlay_conflicts_reject_before_any_conversion() {
    for fault in [
        "explicit_task",
        "second_task",
        "foreign_repo",
        "foreign_path",
        "foreign_name",
        "cursor_task",
        "cursor_change",
        "missing_path",
        "missing_name",
    ] {
        let f = fixture_with("ait-server", "SE", 20, 19, 20, None);
        let (worktree, _, _) = legacy_overlay(&f, 19, Some(19));
        let overlay_path = worktree.join(".ait-worktree.json");
        let mut overlay: serde_json::Value =
            serde_json::from_slice(&fs::read(&overlay_path).unwrap()).unwrap();
        match fault {
            "explicit_task" => overlay["bound_task_id"] = "LSET-0019".into(),
            "second_task" => {
                overlay["bound_task_id"] = "LSET-0020".into();
                overlay["task_id"] = "LSET-0019".into();
            }
            "foreign_repo" | "foreign_path" => {
                let other = f.repo.join("foreign");
                fs::create_dir(&other).unwrap();
                overlay[if fault == "foreign_repo" {
                    "repo_root"
                } else {
                    "workspace_root"
                }] = serde_json::json!(other);
            }
            "foreign_name" => overlay["worktree_name"] = "another-task".into(),
            "missing_path" => {
                overlay.as_object_mut().unwrap().remove("workspace_root");
            }
            "missing_name" => {
                overlay.as_object_mut().unwrap().remove("worktree_name");
            }
            "cursor_task" | "cursor_change" => {
                let path = worktree.join(".ait-worktree/cursor.bin");
                let mut bytes = fs::read(&path).unwrap();
                let (offset, value) = if fault == "cursor_task" {
                    (8, 18u32)
                } else {
                    (12, 1u32)
                };
                bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                fs::write(path, bytes).unwrap();
            }
            _ => unreachable!(),
        }
        let overlay = serde_json::to_vec(&overlay).unwrap();
        fs::write(&overlay_path, &overlay).unwrap();
        let before = affected(&f);
        assert!(
            admit_local_receipt_upgrade(&f.repo, &f.db, "ait-server").is_err(),
            "{fault}"
        );
        assert_eq!(affected(&f), before, "{fault}");
        assert_eq!(fs::read(overlay_path).unwrap(), overlay);
        assert_eq!(
            fs::read_to_string(worktree.join("dirty.rs")).unwrap(),
            "keep unfinished work"
        );
    }
}

#[cfg(unix)]
#[test]
fn dangling_optional_cursor_is_not_silently_treated_as_absent() {
    let f = fixture_with("ait-native", "N", 20, 19, 20, None);
    let (worktree, overlay, _) = legacy_overlay(&f, 19, Some(19));
    let cursor = worktree.join(".ait-worktree/cursor.bin");
    fs::remove_file(&cursor).unwrap();
    std::os::unix::fs::symlink(worktree.join("missing-cursor"), &cursor).unwrap();
    let before = affected(&f);
    assert!(admit_local_receipt_upgrade(&f.repo, &f.db, "ait-native").is_err());
    assert_eq!(affected(&f), before);
    assert_eq!(
        fs::read(worktree.join(".ait-worktree.json")).unwrap(),
        overlay
    );
}
