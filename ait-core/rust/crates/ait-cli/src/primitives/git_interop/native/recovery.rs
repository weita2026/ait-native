//! Explicit recovery of optional Git tails and indexes. Never reclaims a
//! complete fixed owner or a physical Plan/PlanRevision slot.
use super::*;
use ait_core::binary_db::{BinaryDb, BinaryDbCommandLockSet, BinaryDbCommandScope, BinaryFileId};

const FIXED: &[(&str, usize)] = &[
    ("git_repository.bin", 28),
    ("git_object.bin", 32),
    ("git_generation.bin", 20),
    ("git_identity.bin", 24),
    ("git_commit_mapping.bin", 100),
    ("git_ref_mapping.bin", 80),
    ("git_operation_checkpoint.bin", 44),
];
pub(super) fn recover(repo: &RepoRuntime, apply: bool) -> Result<JsonValue, String> {
    if !exists(repo) {
        return Ok(json!({"operation":"git.recover","status":"not_required","mutated":false}));
    }
    let canonical = repo.authoritative_repo_root();
    let _activation_guard = ait_core::binary_db::BinaryDbReadLockSet::try_acquire(
        &ait_core::binary_db_generation::binary_db_activation_lock_root(&canonical),
    )?;
    let source = recovery_db(repo, apply)?;
    let legacy_source = repo
        .authoritative_repo_root()
        .join(GIT_INTEROP_LEGACY_DIRECTORY);
    let repair_legacy = !repo
        .authoritative_repo_root()
        .join(GIT_INTEROP_BUNDLE_PATH)
        .exists();
    let mut owner_lock =
        BinaryDbCommandLockSet::acquire(source.authority_root(), BinaryDbCommandScope::General)?;
    // Explicit recovery must precede generic Plan admission, which correctly
    // blocks unpublished raw owners. Never use the runtime factory's failed
    // admission or test fallback as a recovery authority.
    let content_root = canonical.join(".ait/binary-db");
    let selected = fs::symlink_metadata(&content_root).map_err(|error| error.to_string())?;
    let (authority, pack_root) = if selected.file_type().is_symlink() {
        let generation = ait_core::binary_db_generation::admit_activated_binary_db_generation(
            &content_root,
            &repo.repo_name(),
        )?;
        (generation.authority_root, generation.generation_root)
    } else if selected.is_dir() {
        (content_root, canonical.clone())
    } else {
        return Err("Git recovery requires an existing selected content authority".into());
    };
    let content = LocalContentBinaryDb::<1>::from_db_with_roots(
        LocalBinaryDbFs::new(
            authority,
            canonical.clone(),
            AuthorityId::new("git-content-recovery"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS),
        canonical,
        pack_root,
    );
    let mut content_lock = BinaryDbCommandLockSet::acquire(
        content.db().authority_root(),
        BinaryDbCommandScope::General,
    )?;
    let temp = TempBuilder::new()
        .prefix(".interop-recovery-")
        .tempdir_in(repo.authoritative_repo_root().join(".ait/git"))
        .map_err(|e| e.to_string())?;
    let stage_root = temp.path().join("binary-v0");
    fs::create_dir(&stage_root).map_err(|e| e.to_string())?;
    let content_stage_root = temp.path().join("content");
    fs::create_dir(&content_stage_root).map_err(|e| e.to_string())?;
    let content_source_root = content.db().authority_root().as_path();
    for name in ait_core::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS
        .iter()
        .chain(ait_core::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS)
    {
        let path = content_source_root.join(name);
        if !path.exists() {
            continue;
        }
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
        {
            return Err(format!(
                "Git recovery refuses non-regular content authority {name}"
            ));
        }
        let mut bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if let Some(width) = match *name {
            "plan.bin" => Some(48),
            "plan_revision.bin" => Some(56),
            _ => None,
        } {
            if bytes.len() < 4 || bytes[..4] != 1u32.to_le_bytes() {
                return Err(format!("Git recovery blocks malformed {name} layout"));
            }
            bytes.truncate(4 + (bytes.len() - 4) / width * width);
        }
        fs::write(content_stage_root.join(name), bytes).map_err(|e| e.to_string())?;
    }
    let content_stage = LocalContentBinaryDb::from_db_with_roots(
        LocalBinaryDbFs::new(
            content_stage_root.clone(),
            content.repo_root().clone(),
            AuthorityId::new("git-content-recovery-stage"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS),
        content.workspace_root().clone(),
        content.pack_root().clone(),
    );
    let source_root = source.authority_root().as_path();
    for name in GIT_OBJECT_STORE_BIN_PATHS
        .iter()
        .chain(GIT_OBJECT_STORE_INDEX_PATHS)
    {
        let path = source_root.join(name);
        if !path.exists() {
            continue;
        }
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
        {
            return Err(format!(
                "Git recovery refuses non-regular authority file {name}"
            ));
        }
        let mut bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if let Some((_, width)) = FIXED.iter().find(|(file, _)| file == name) {
            let header = 1u32.to_le_bytes();
            if bytes.len() < 4 {
                if bytes != header[..bytes.len()] {
                    return Err(format!("Git recovery refuses unsupported layout in {name}"));
                }
                bytes = header.to_vec();
            } else {
                if bytes[..4] != header {
                    return Err(format!("Git recovery refuses unsupported layout in {name}"));
                }
                bytes.truncate(4 + (bytes.len() - 4) / width * width);
            }
        }
        fs::write(stage_root.join(name), bytes).map_err(|e| e.to_string())?;
    }
    let stage = LocalBinaryDbFs::new(
        stage_root.clone(),
        repo.authoritative_repo_root(),
        AuthorityId::new("git-recovery-stage"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(GIT_OBJECT_STORE_BIN_PATHS)
    .with_declared_index_paths(GIT_OBJECT_STORE_INDEX_PATHS);
    // Preserve every complete owner's range. Only an unreferenced payload
    // suffix may disappear; a short referenced body is blocked, never guessed.
    for (owner, width, payload, length_offset, offset_offset) in [
        ("git_repository.bin", 28, "git_repository_payload.bin", 4, 8),
        ("git_identity.bin", 24, "git_identity_payload.bin", 4, 8),
        (
            "git_ref_mapping.bin",
            80,
            "git_ref_mapping_payload.bin",
            20,
            24,
        ),
    ] {
        let read = BinaryDbReadTxn::new(&stage);
        let mut end = 4u64;
        for ordinal in 0..read.record_count(BinaryFileId::new(owner, 1, width))? {
            let row = read.read_record(BinaryFileId::new(owner, 1, width), ordinal)?;
            let offset =
                u64::from_le_bytes(row[offset_offset..offset_offset + 8].try_into().unwrap());
            let length =
                u32::from_le_bytes(row[length_offset..length_offset + 4].try_into().unwrap());
            if offset < 4 {
                return Err(format!("Git recovery blocks invalid {payload} owner range"));
            }
            end = end.max(
                offset
                    .checked_add(u64::from(length))
                    .ok_or("Git payload range overflow")?,
            );
        }
        drop(read);
        let path = stage_root.join(payload);
        if !path.exists() {
            if end > 4 {
                return Err(format!("Git recovery blocks missing referenced {payload}"));
            }
            continue;
        }
        let mut bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() < 4 && end == 4 && bytes == 1u32.to_le_bytes()[..bytes.len()] {
            bytes = 1u32.to_le_bytes().to_vec();
        }
        if bytes.len() < 4 || bytes[..4] != 1u32.to_le_bytes() || (bytes.len() as u64) < end {
            return Err(format!("Git recovery blocks damaged referenced {payload}"));
        }
        bytes.truncate(usize::try_from(end).map_err(|_| "Git payload length exceeds usize")?);
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
    }
    LocalGitObjectStore::rebuild_indexes(&stage)?;
    GitHistoryStore::rebuild_indexes(&stage)?;
    let cursor_history = GitHistoryStore::open(stage.clone())?;
    let mappings = cursor_history.commit_mappings();
    let refs = cursor_history.refs()?;
    for mut checkpoint in cursor_history.checkpoints()? {
        if checkpoint.status != 0 {
            continue;
        }
        let commits = u32::try_from(
            mappings
                .iter()
                .filter(|row| row.generation_index == checkpoint.generation_index)
                .count(),
        )
        .map_err(|_| "Git mapping count exceeds u32")?;
        let names = u32::try_from(
            refs.iter()
                .filter(|(_, _, row)| row.generation_index == checkpoint.generation_index)
                .map(|(_, name, _)| name)
                .collect::<BTreeSet<_>>()
                .len(),
        )
        .map_err(|_| "Git ref count exceeds u32")?;
        if checkpoint.next_commit > commits || checkpoint.next_ref > names {
            // Pending cursors are progress hints. A torn hint can move back to
            // the admitted owner prefix; frozen intent and completed markers
            // are never changed or inferred.
            checkpoint.next_commit = checkpoint.next_commit.min(commits);
            checkpoint.next_ref = checkpoint.next_ref.min(names);
            cursor_history.checkpoint(checkpoint)?;
        }
    }
    RawGitDocumentStore::recover_plan_roots(&stage, &content_stage)?;
    RawGitDocumentStore::rebuild_indexes(&stage, &content_stage)?;
    let objects = LocalGitObjectStore::open(stage.clone(), content_stage.clone())?;
    let history = GitHistoryStore::open(stage.clone())?;
    let documents = RawGitDocumentStore::open(stage.clone(), content_stage.clone())?;
    admission::validate(&objects, &history, &documents, &content_stage)
        .map_err(|e| format!("Git recovery blocked before mutation: {e}"))?;
    let mut changes = Vec::new();
    let mut content_changes = Vec::new();
    for name in ["plan_payload.bin", "plan_revision.bin", "plan.bin"] {
        let path = content_stage_root.join(name);
        if !path.exists() {
            continue;
        }
        let wanted = fs::read(&path).map_err(|e| e.to_string())?;
        if fs::read(content_source_root.join(name)).ok().as_deref() != Some(wanted.as_slice()) {
            content_changes.push((name, wanted));
        }
    }
    for name in GIT_OBJECT_STORE_BIN_PATHS
        .iter()
        .chain(GIT_OBJECT_STORE_INDEX_PATHS)
    {
        let path = stage_root.join(name);
        if !path.exists() {
            continue;
        }
        let wanted = fs::read(&path).map_err(|e| e.to_string())?;
        let old = fs::read(source_root.join(name)).ok();
        if old.as_deref() != Some(wanted.as_slice()) {
            changes.push((*name, wanted));
        }
    }
    if apply {
        // Dependencies and reserved Plan owners precede repaired Git indexes.
        // Each complete prefix replacement is restartable under both locks.
        for (name, bytes) in &content_changes {
            content.db().replace_file_atomically(
                &content_source_root.join(name),
                bytes,
                "Git recovered content owner prefix",
            )?;
            fs::File::open(content_source_root)
                .and_then(|file| file.sync_all())
                .map_err(|e| e.to_string())?;
            #[cfg(test)]
            if *name == "plan.bin" {
                if let Some(marker) = std::env::var_os("AIT_GIT_RECOVERY_TEST_PAUSE") {
                    fs::write(marker, b"recovered Plan root before Git indexes")
                        .map_err(|e| e.to_string())?;
                    loop {
                        std::thread::park();
                    }
                }
            }
        }
        for (name, bytes) in &changes {
            source.replace_file_atomically(
                &source_root.join(name),
                bytes,
                "Git recovery admitted prefix",
            )?;
            if repair_legacy {
                source.replace_file_atomically(
                    &legacy_source.join(name),
                    bytes,
                    "Git recovered legacy prefix before bundle migration",
                )?;
            }
        }
        if repair_legacy && legacy_source.exists() {
            fs::File::open(&legacy_source)
                .and_then(|file| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        fs::File::open(source_root)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        for (name, bytes) in &changes {
            if fs::read(source_root.join(name)).map_err(|e| e.to_string())? != *bytes {
                return Err("Git recovery publication differs from admitted stage".into());
            }
        }
        for (name, bytes) in &content_changes {
            if fs::read(content_source_root.join(name)).map_err(|e| e.to_string())? != *bytes {
                return Err("Git recovered content owner differs from admitted stage".into());
            }
        }
    }
    content_lock.release()?;
    owner_lock.release()?;
    if apply {
        source.checkpoint_directory()?;
    }
    Ok(
        json!({"operation":"git.recover","status":if changes.is_empty()&&content_changes.is_empty(){"clean"}else if apply{"repaired"}else{"repairable"},
        "mutated":apply&&(!changes.is_empty()||!content_changes.is_empty()),"repair_files":content_changes.iter().map(|(n,_)|*n).chain(changes.iter().map(|(n,_)|*n)).collect::<Vec<_>>(),
        "complete_fixed_owners_preserved":true,"plan_revision_ordinals_preserved":true,"mapping_json_count":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    fn git(root: &Path, args: &[&str]) -> Vec<u8> {
        let out = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }
    #[test]
    fn recovery_process_child() {
        let Some(root) = std::env::var_os("AIT_GIT_RECOVERY_TEST_ROOT") else {
            return;
        };
        let repo = RepoRuntime::discover_from_path(&PathBuf::from(root).join("ait")).unwrap();
        recover(&repo, true).unwrap();
        panic!("recovery child must be killed at its admitted Plan boundary");
    }
    #[test]
    fn import_process_child() {
        let Some(root) = std::env::var_os("AIT_GIT_RECOVERY_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let repo = RepoRuntime::discover_from_path(&root.join("ait")).unwrap();
        git_import(&repo, root.join("source").to_str().unwrap(), true, false).unwrap();
        panic!("import child must be killed at its durable mapping boundary");
    }
    #[cfg(unix)]
    #[test]
    fn recovery_sigkill_preserves_raw_oid_markdown_and_physical_ordinals() {
        use std::os::unix::process::ExitStatusExt;
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let ait = temp.path().join("ait");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&ait).unwrap();
        git(&source, &["init", "-b", "main"]);
        git(&source, &["config", "user.name", "Recovery"]);
        git(&source, &["config", "user.email", "recovery@local"]);
        fs::write(source.join("code.txt"), b"code\n").unwrap();
        fs::write(source.join("README.md"), b"first\r\n\xff\x00").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "first"]);
        init_repo(&InitRequest {
            root: ait.clone(),
            name: Some("recovery".into()),
            default_line: "main".into(),
            policy_profile: "prototype".into(),
            default_author_mode: "ai_with_human_review".into(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let repo = RepoRuntime::discover_from_path(&ait).unwrap();
        git_import(&repo, source.to_str().unwrap(), true, false).unwrap();
        let plan_path = ait.join(".ait/binary-db/plan.bin");
        let old_plans = fs::read(&plan_path).unwrap();
        fs::write(source.join("Guide.MD"), b"new\r\n\x00\xfe").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "new path"]);
        let oid = git(&source, &["rev-parse", "HEAD"]);
        let source_objects = git(
            &source,
            &["rev-list", "--objects", "--all", "--no-object-names"],
        );
        let raw = String::from_utf8(source_objects)
            .unwrap()
            .lines()
            .map(|oid| (oid.to_string(), git(&source, &["cat-file", "-p", oid])))
            .collect::<Vec<_>>();
        let import_marker = temp.path().join("mapping-paused");
        let mut importing = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "primitives::git_interop::native::recovery::tests::import_process_child",
                    "--nocapture",
                ])
                .env("AIT_GIT_RECOVERY_TEST_ROOT", temp.path())
                .env("AIT_GIT_IMPORT_TEST_PAUSE", &import_marker)
                .stdin(Stdio::null())
                .spawn()
                .unwrap(),
        );
        for _ in 0..400 {
            if import_marker.exists() || importing.0.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            import_marker.exists(),
            "import did not reach durable mapping boundary"
        );
        importing.0.kill().unwrap();
        assert_eq!(importing.0.wait().unwrap().signal(), Some(9));
        let interrupted_mapping = read_native_file(&repo, "git_commit_mapping.bin");
        assert!(stores(&repo)
            .unwrap()
            .1
            .checkpoints()
            .unwrap()
            .iter()
            .any(|row| row.status == 0 && row.next_commit == 1));
        recover(&repo, true).unwrap();
        let resumed = git_import(&repo, source.to_str().unwrap(), true, false).unwrap();
        assert_eq!(resumed["imported_commit_count"], json!(0));
        assert_eq!(
            read_native_file(&repo, "git_commit_mapping.bin"),
            interrupted_mapping
        );
        let complete_plans = fs::read(&plan_path).unwrap();
        let revisions_path = ait.join(".ait/binary-db/plan_revision.bin");
        let revisions = fs::read(&revisions_path).unwrap();
        let payload_path = ait.join(".ait/binary-db/plan_revision_payload.bin");
        let revision_payload = fs::read(&payload_path).unwrap();
        let mappings = read_native_file(&repo, "git_commit_mapping.bin");
        fs::write(&plan_path, &old_plans).unwrap();
        mutate_native_files(&repo, |decoded| {
            for (name, width) in [
                ("git_document_plan.idx", 40),
                ("git_document_revision.idx", 50),
            ] {
                let path = decoded.join(name);
                let mut bytes = fs::read(&path).unwrap();
                bytes.truncate(bytes.len() - width);
                fs::write(path, bytes).unwrap();
            }
        });
        assert_eq!(
            recover(&repo, false).unwrap()["status"],
            json!("repairable")
        );
        assert_eq!(fs::read(&plan_path).unwrap(), old_plans);
        assert_eq!(fs::read(&revisions_path).unwrap(), revisions);
        let marker = temp.path().join("recovery-paused");
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "primitives::git_interop::native::recovery::tests::recovery_process_child",
                    "--nocapture",
                ])
                .env("AIT_GIT_RECOVERY_TEST_ROOT", temp.path())
                .env("AIT_GIT_RECOVERY_TEST_PAUSE", &marker)
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
            "child did not reach recovered owner boundary"
        );
        child.0.kill().unwrap();
        assert_eq!(child.0.wait().unwrap().signal(), Some(9));
        let fresh = RepoRuntime::discover_from_path(&ait).unwrap();
        assert_eq!(recover(&fresh, true).unwrap()["status"], json!("repaired"));
        assert_eq!(recover(&fresh, true).unwrap()["status"], json!("clean"));
        assert_eq!(read_native_file(&fresh, "git_commit_mapping.bin"), mappings);
        assert_eq!(fs::read(&revisions_path).unwrap(), revisions);
        assert_eq!(fs::read(&payload_path).unwrap(), revision_payload);
        assert_eq!(&fs::read(&plan_path).unwrap()[..old_plans.len()], old_plans);
        assert_eq!(fs::read(&plan_path).unwrap().len(), complete_plans.len());
        fs::rename(source.join(".git"), temp.path().join("original-odb.hidden")).unwrap();
        let target = temp.path().join("recovered.git");
        exports::current(&fresh, target.to_str().unwrap(), true, false).unwrap();
        assert_eq!(git(&target, &["rev-parse", "HEAD"]), oid);
        for (oid, body) in raw {
            assert_eq!(git(&target, &["cat-file", "-p", &oid]), body);
        }
        assert_eq!(git(&target, &["show", "HEAD:Guide.MD"]), b"new\r\n\x00\xfe");
        git(&target, &["fsck", "--strict", "--full"]);
        assert!(!root(&fresh).parent().unwrap().join("operations").exists());
        assert!(!root(&fresh).parent().unwrap().join("mappings").exists());
        println!("actual recovery SIGKILL, fresh-process resume: original OIDs, raw Markdown, complete mapping/revision bytes and reserved physical Plan slots unchanged; original ODB absent; 0 mapping JSON");
    }
}
