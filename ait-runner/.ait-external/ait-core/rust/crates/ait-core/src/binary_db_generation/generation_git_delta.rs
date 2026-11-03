//! Private ordinal-preserving conversion through the existing generation protocol.
use super::*;
use crate::binary_db::{
    AuthorityId, BinaryDb, BinaryDbCommandScope, BinaryDbReadLockSet, BinaryDbReadTxn,
    BinaryDbWriteTxn, LocalBinaryDbFs, LocalStateScope, REPOSITORY_BINARY_DB_BIN_PATHS,
    REPOSITORY_BINARY_DB_INDEX_PATHS,
};
use crate::content_binary_db::{
    BinaryBlobCodec, BinaryDbContentWriteCoordinator, BinaryDbObjectPackMemberWriteInput,
    BinaryDbObjectPackStore, BinaryDbObjectPackWriteInput, BinaryObjectPackCodec,
    BinaryObjectPackMemberCodec, BinaryObjectPackMemberKind, LocalContentBinaryDb,
};
use crate::git_binary_db::{
    git_object_file, GitObjectDeltaHints, GitObjectRecord, LocalGitObjectStore,
};
use crate::pack_substrate::{
    default_object_pack_relative_path, estimated_typed_pack_archive_bytes,
    write_typed_pack_archive_with_format, PackCandidate, CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::time::Instant;

#[derive(Clone, Debug, Serialize)]
pub struct GitDeltaRepackReport {
    pub capture: CaptureBinaryDbGenerationReport,
    pub git_objects: usize,
    pub rewritten_blob_locators: usize,
    pub delta_members: usize,
    pub added_pack_bytes: u64,
    pub capture_seconds: f64,
    pub repack_seconds: f64,
    pub seal_seconds: f64,
    pub source_ordinals_and_immutable_bytes_preserved: bool,
    pub resumed_existing_stage: bool,
    pub resumed_blob_locators: usize,
    pub resumed_objects: usize,
    pub guard_seconds: f64,
    pub metadata_admission_seconds: f64,
}

pub fn stage_binary_db_git_delta_repack(
    options: CaptureBinaryDbGenerationOptions,
) -> GenerationResult<GitDeltaRepackReport> {
    let started = Instant::now();
    let capture = capture_binary_db_generation_preserving_ordinals(options)?;
    let capture_seconds = started.elapsed().as_secs_f64();
    repack_captured_generation(capture, capture_seconds, None)
}

pub fn resume_binary_db_git_delta_repack(
    repo_root: &Path,
    generation_root: &Path,
) -> GenerationResult<GitDeltaRepackReport> {
    resume_repack(repo_root, generation_root, None)
}

pub fn restage_binary_db_git_delta_whole_packs(
    repo_root: &Path,
    generation_root: &Path,
    original_capture_manifest: &Path,
) -> GenerationResult<GitDeltaRepackReport> {
    resume_repack(repo_root, generation_root, Some(original_capture_manifest))
}

fn resume_repack(
    repo_root: &Path,
    generation_root: &Path,
    original_manifest: Option<&Path>,
) -> GenerationResult<GitDeltaRepackReport> {
    let started = Instant::now();
    let repo = repo_root.canonicalize().map_err(|e| e.to_string())?;
    let root = generation_root.canonicalize().map_err(|e| e.to_string())?;
    if repo.starts_with(&root) || root.starts_with(&repo) {
        return Err("delta resume requires a separate inactive generation".into());
    }
    let _activation_guard =
        BinaryDbReadLockSet::try_acquire(&binary_db_activation_lock_root(&repo))
            .map_err(|e| format!("delta resume source activation is active: {e}"))?;
    let source_db = LocalBinaryDbFs::new(
        repo.join(".ait/binary-db"),
        repo.clone(),
        AuthorityId::new("delta-resume-source"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
    let source_read = BinaryDbReadTxn::new(&source_db);
    source_read.read_lock_paths().map_err(|e| e.to_string())?;
    let _git_guard = crate::git_binary_db::GitInteropBundle::open(&repo, false)
        .map_err(|e| format!("delta resume source Git writer is active: {e}"))?;
    let mut baseline = if let Some(path) = original_manifest {
        admit_partial_stage_with_manifest(&repo, &root, &direct_file_bytes(path)?)?
    } else {
        admit_partial_stage(&repo, &root)?
    };
    if let Some(path) = original_manifest {
        let db = LocalBinaryDbFs::new(
            root.join("local"),
            root.clone(),
            AuthorityId::new("whole-pack-reset"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
        let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ContentWrite)?;
        for index in &baseline.rewritten {
            let at = 4 + *index as usize * 64;
            write.overwrite_record(
                BinaryBlobCodec::<1>::record_file(),
                *index,
                &baseline.original_blobs[at..at + 64],
            )?;
        }
        write.commit()?;
        let manifest = direct_file_bytes(path)?;
        let staged = root.join(format!(".client-manifest-reset-{}", std::process::id()));
        let mut out = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staged)
            .map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut out, &manifest).map_err(|e| e.to_string())?;
        out.sync_all().map_err(|e| e.to_string())?;
        fs::rename(staged, root.join("client-manifest.json")).map_err(|e| e.to_string())?;
        fs::File::open(&root)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        eprintln!("whole-pack reset: restored {} original locators; all prior packs/member/index slots retained", baseline.rewritten.len());
        baseline.rewritten.clear();
        baseline.delta_members = 0;
        baseline.added_pack_bytes = 0;
    }
    baseline.guard_seconds = started.elapsed().as_secs_f64();
    eprintln!(
        "delta-resume guard passed: recovered_locators={} seconds={:.3}",
        baseline.rewritten.len(),
        baseline.guard_seconds
    );
    let capture = CaptureBinaryDbGenerationReport {
        output_root: root,
        repo_name: baseline.manifest.label.clone(),
        worker_count: baseline.manifest.worker_count,
        source_authority_fingerprint: baseline.manifest.source_authority_fingerprint.clone(),
        content_fingerprint: baseline.manifest.content_fingerprint.clone(),
        file_count: baseline.manifest.files.len(),
    };
    repack_captured_generation(capture, 0.0, Some(baseline))
}

fn repack_captured_generation(
    capture: CaptureBinaryDbGenerationReport,
    capture_seconds: f64,
    baseline: Option<ResumeBaseline>,
) -> GenerationResult<GitDeltaRepackReport> {
    let root = &capture.output_root;
    let local = root.join("local");
    let resumed_existing_stage = baseline.is_some();
    let mut immutable = baseline
        .as_ref()
        .map(|b| b.immutable.clone())
        .unwrap_or_default();
    let mut appended = baseline
        .as_ref()
        .map(|b| b.appended.clone())
        .unwrap_or_default();
    if baseline.is_none() {
        for (base, prefix) in [
            (local.clone(), "local/"),
            (
                root.join(".ait/git-interop/v1/binary-v0"),
                ".ait/git-interop/v1/binary-v0/",
            ),
        ] {
            if !base.exists() {
                continue;
            }
            for entry in fs::read_dir(base).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let relative = format!("{prefix}{name}");
                let data = fs::read(entry.path()).map_err(|e| e.to_string())?;
                if prefix == "local/"
                    && [
                        "object_pack.bin",
                        "object_pack_member.bin",
                        "object_pack_id.idx",
                    ]
                    .contains(&name.as_str())
                {
                    appended.push((relative, data));
                } else if relative != "local/blob.bin" {
                    immutable.push((relative, Sha256::digest(data).to_vec()));
                }
            }
        }
        let archive = root.join(crate::git_binary_db::GIT_INTEROP_BUNDLE_PATH);
        if archive.exists() {
            immutable.push((
                crate::git_binary_db::GIT_INTEROP_BUNDLE_PATH.into(),
                Sha256::digest(direct_file_bytes(&archive)?).to_vec(),
            ));
        }
    }
    let db = LocalBinaryDbFs::new(
        local.clone(),
        root.clone(),
        AuthorityId::new("git-delta-stage"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
    let content =
        LocalContentBinaryDb::<1>::from_db_with_roots(db.clone(), root.clone(), root.clone());
    let git_db = super::generation_git::git_database(
        &root.join(super::generation_git::GIT_DIRECTORY),
        root,
    )?;
    let admission_started = Instant::now();
    let objects = LocalGitObjectStore::open_for_staged_repack(git_db.clone(), content.clone())?;
    let read = BinaryDbReadTxn::new(&git_db);
    let rows = (0..read.record_count(git_object_file())?)
        .map(|index| GitObjectRecord::decode(&read.read_record(git_object_file(), index)?))
        .collect::<Result<Vec<_>, _>>()?;
    drop(read);
    let metadata_admission_seconds = admission_started.elapsed().as_secs_f64();
    let original_blobs = if let Some(b) = &baseline {
        b.original_blobs.clone()
    } else {
        fs::read(local.join("blob.bin")).map_err(|e| e.to_string())?
    };
    let read = BinaryDbReadTxn::new(content.db());
    let mut original_pack_roots = BTreeMap::<u32, usize>::new();
    for bytes in original_blobs[4..].chunks_exact(64) {
        let blob = BinaryBlobCodec::<1>::decode_record(bytes)?;
        if blob.is_tombstone() || blob.is_pruned() {
            continue;
        }
        if let Some(member) = blob.pack_member_index() {
            let pack = content
                .object_packs()
                .object_pack_member_view_at(&read, member)?
                .record
                .pack_index;
            *original_pack_roots.entry(pack).or_default() += 1;
        }
    }
    let mut groups = BTreeMap::<u32, Vec<&GitObjectRecord>>::new();
    let mut seen = BTreeSet::new();
    for row in &rows {
        if !seen.insert(row.content_blob_index) {
            continue;
        }
        let at = 4 + row.content_blob_index as usize * 64;
        let blob = BinaryBlobCodec::<1>::decode_record(
            original_blobs
                .get(at..at + 64)
                .ok_or("original Git Blob FK absent")?,
        )?;
        let member = blob
            .pack_member_index()
            .ok_or("original Git Blob lacks selected member")?;
        let pack = content
            .object_packs()
            .object_pack_member_view_at(&read, member)?
            .record
            .pack_index;
        groups.entry(pack).or_default().push(row);
    }
    drop(read);
    let mut paths = GitObjectDeltaHints::default();
    let mut heads = BTreeMap::new();
    let mut rewritten = baseline
        .as_ref()
        .map(|b| b.rewritten.clone())
        .unwrap_or_default();
    let resumed_blob_locators = rewritten.len();
    let resumed_objects = rewritten.len();
    let mut delta_members = baseline
        .as_ref()
        .map(|b| b.delta_members)
        .unwrap_or_default();
    let mut added_pack_bytes = baseline
        .as_ref()
        .map(|b| b.added_pack_bytes)
        .unwrap_or_default();
    let started = Instant::now();
    let mut body_read = crate::content_binary_db::BinaryBlobReadSession::default();
    let mut processed = 0;
    for (group_number, (pack, group)) in groups.into_iter().enumerate() {
        body_read.clear_logical_values();
        let raw_bytes = group.iter().try_fold(0u64, |sum, row| {
            let at = 4 + row.content_blob_index as usize * 64;
            Ok::<_, String>(
                sum + BinaryBlobCodec::<1>::decode_record(&original_blobs[at..at + 64])?.size_bytes,
            )
        })?;
        let complete = original_pack_roots.get(&pack).copied() == Some(group.len());
        let bounded = group.len() <= 128 && raw_bytes <= 8 * 1024 * 1024;
        let recovered = group
            .iter()
            .filter(|row| rewritten.contains(&row.content_blob_index))
            .count();
        if recovered != 0 && recovered != group.len() {
            return Err("whole-pack resume found partially converted original pack".into());
        }
        let mut bodies = Vec::new();
        for row in group {
            let (_, body) = objects
                .get_with_read_session(row.repository_index, 1, &row.object_id, &mut body_read)?
                .ok_or("Git source object absent in captured generation")?;
            let hint = paths.observe(row.repository_index, row.kind, row.object_id, &body);
            if complete && bounded && recovered == 0 {
                bodies.push((row.content_blob_index, body, hint));
            } else {
                heads.insert(hint, row.content_blob_index);
            }
            processed += 1;
        }
        if !bodies.is_empty() {
            let (changed, deltas, stored) = repack_batch(&content, root, &bodies, &mut heads)?;
            rewritten.extend(changed);
            delta_members += deltas;
            added_pack_bytes += stored;
        }
        if (group_number + 1).is_multiple_of(10) {
            eprintln!("whole-pack repack groups={} objects={processed}/{} locators={} deltas={delta_members} added_bytes={added_pack_bytes} seconds={:.3}", group_number+1, rows.len(), rewritten.len(), started.elapsed().as_secs_f64());
        }
    }
    let after = fs::read(local.join("blob.bin")).map_err(|e| e.to_string())?;
    if after.len() != original_blobs.len() {
        return Err("delta conversion changed Blob ordinal count".into());
    }
    for (index, (old, new)) in original_blobs[4..]
        .chunks_exact(64)
        .zip(after[4..].chunks_exact(64))
        .enumerate()
    {
        if old[..12] != new[..12]
            || old[16..] != new[16..]
            || (!rewritten.contains(&(index as u32)) && old != new)
        {
            return Err(format!(
                "delta conversion changed immutable Blob fields at {index}"
            ));
        }
    }
    let repack_seconds = started.elapsed().as_secs_f64();
    for (relative, hash) in immutable {
        if Sha256::digest(fs::read(root.join(&relative)).map_err(|e| e.to_string())?).as_slice()
            != hash
        {
            return Err(format!(
                "delta conversion changed immutable file {relative}"
            ));
        }
    }
    for (relative, before) in appended {
        let after = fs::read(root.join(&relative)).map_err(|e| e.to_string())?;
        if !after.starts_with(&before) {
            return Err(format!(
                "delta conversion changed physical record prefix {relative}"
            ));
        }
    }
    if let Some(b) = &baseline {
        verify_source_unchanged(b)?;
    }
    eprintln!("delta-repack complete: objects={processed} locators={} deltas={delta_members} seconds={repack_seconds:.3}; sealing", rewritten.len());
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("AIT_GIT_DELTA_STAGE_PAUSE") {
        fs::write(marker, b"inactive replacement locators ready before seal")
            .map_err(|e| e.to_string())?;
        loop {
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        }
    }
    drop((objects, content, db, git_db));
    for path in [
        local.join(".locks"),
        root.join(".ait/git-interop/v1/binary-v0/.locks"),
    ] {
        if path.exists() {
            fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
    }
    let started = Instant::now();
    seal_binary_db_generation(root)?;
    let seal_seconds = started.elapsed().as_secs_f64();
    if let Some(b) = &baseline {
        verify_source_unchanged(b)?;
    }
    Ok(GitDeltaRepackReport {
        capture,
        git_objects: rows.len(),
        rewritten_blob_locators: rewritten.len(),
        delta_members,
        added_pack_bytes,
        capture_seconds,
        repack_seconds,
        seal_seconds,
        source_ordinals_and_immutable_bytes_preserved: true,
        resumed_existing_stage,
        resumed_blob_locators,
        resumed_objects,
        guard_seconds: baseline
            .as_ref()
            .map(|b| b.guard_seconds)
            .unwrap_or_default(),
        metadata_admission_seconds,
    })
}

#[derive(Deserialize)]
struct ResumeManifest {
    schema: String,
    label: String,
    worker_count: usize,
    source_repo_root: PathBuf,
    source_authority_root: PathBuf,
    source_authority_fingerprint: String,
    content_fingerprint: String,
    layout_ids: BTreeMap<String, u32>,
    files: Vec<super::generation_activation::ClientManifestFile>,
    content_projection: ResumeProjection,
    validation: ResumeValidation,
}

#[derive(Deserialize)]
struct ResumeProjection {
    source_retained: bool,
}
#[derive(Deserialize)]
struct ResumeValidation {
    status: String,
}

struct ResumeBaseline {
    repo: PathBuf,
    manifest: ResumeManifest,
    original_blobs: Vec<u8>,
    immutable: Vec<(String, Vec<u8>)>,
    appended: Vec<(String, Vec<u8>)>,
    rewritten: BTreeSet<u32>,
    delta_members: usize,
    added_pack_bytes: u64,
    guard_seconds: f64,
}

fn direct_file_bytes(path: &Path) -> GenerationResult<Vec<u8>> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.file_type().is_file() || path.canonicalize().map_err(|e| e.to_string())? != path {
        return Err(format!(
            "delta resume requires a direct file: {}",
            path.display()
        ));
    }
    fs::read(path).map_err(|e| e.to_string())
}

fn source_file(repo: &Path, relative: &str) -> PathBuf {
    if let Some(name) = relative.strip_prefix("local/") {
        repo.join(".ait/binary-db").join(name)
    } else {
        repo.join(relative)
    }
}

fn source_bytes(
    repo: &Path,
    file: &super::generation_activation::ClientManifestFile,
) -> GenerationResult<Vec<u8>> {
    let path = source_file(repo, &file.relative_path);
    let bytes = if path.exists() {
        direct_file_bytes(&path)?
    } else {
        // Capture may emit a declared empty schema leaf absent from the source.
        // It must be exactly the existing layout-1 header, never invented data.
        if file.byte_size != 4
            || !(file.relative_path.starts_with("local/")
                || file
                    .relative_path
                    .starts_with(".ait/git-interop/v1/binary-v0/"))
        {
            return Err(format!(
                "captured source file is absent: {}",
                path.display()
            ));
        }
        1u32.to_le_bytes().to_vec()
    };
    let hash = Sha256::digest(&bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    if bytes.len() as u64 != file.byte_size || hash != file.sha256 {
        return Err(format!(
            "delta resume source file differs from capture: {}",
            file.relative_path
        ));
    }
    Ok(bytes)
}

fn verify_source_unchanged(baseline: &ResumeBaseline) -> GenerationResult<()> {
    let found = super::generation_activation::fingerprint_direct_authority_contents(
        &baseline.repo.join(".ait/binary-db"),
    )?;
    if found != baseline.manifest.source_authority_fingerprint {
        return Err(format!(
            "delta resume source CAS differs: expected {}, found {found}",
            baseline.manifest.source_authority_fingerprint
        ));
    }
    for file in &baseline.manifest.files {
        source_bytes(&baseline.repo, file)?;
    }
    Ok(())
}

fn admit_partial_stage(repo: &Path, root: &Path) -> GenerationResult<ResumeBaseline> {
    admit_partial_stage_with_manifest(
        repo,
        root,
        &direct_file_bytes(&root.join("client-manifest.json"))?,
    )
}

fn admit_partial_stage_with_manifest(
    repo: &Path,
    root: &Path,
    bytes: &[u8],
) -> GenerationResult<ResumeBaseline> {
    let manifest: ResumeManifest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if manifest.schema != "ait.binary-db-local-generation-manifest.v2"
        || !manifest.content_projection.source_retained
        || manifest.validation.status != "passed"
        || manifest.worker_count == 0
        || manifest.source_repo_root != repo
        || manifest.source_authority_root != repo.join(".ait/binary-db")
        || manifest.layout_ids
            != ["content", "line", "plan", "stash", "workflow"]
                .into_iter()
                .map(|s| (s.to_string(), 1))
                .collect()
        || super::generation_activation::fingerprint_files(&manifest.files)
            != manifest.content_fingerprint
    {
        return Err("delta resume requires the original ordinal-preserving capture manifest and source identity".into());
    }
    let mut baseline = ResumeBaseline {
        repo: repo.to_path_buf(),
        manifest,
        original_blobs: Vec::new(),
        immutable: Vec::new(),
        appended: Vec::new(),
        rewritten: BTreeSet::new(),
        delta_members: 0,
        added_pack_bytes: 0,
        guard_seconds: 0.0,
    };
    let mut previous = None;
    for file in &baseline.manifest.files {
        super::generation_activation::validate_generation_file_path(&file.relative_path)?;
        if previous.is_some_and(|s: &str| s >= file.relative_path.as_str()) {
            return Err("delta resume manifest paths must be strictly sorted".into());
        }
        previous = Some(file.relative_path.as_str());
    }
    verify_source_unchanged(&baseline)?;
    for file in &baseline.manifest.files {
        let old = source_bytes(repo, file)?;
        let new = direct_file_bytes(&root.join(&file.relative_path))?;
        if file.relative_path == "local/blob.bin" {
            if old.len() != new.len()
                || old.len() < 4
                || (old.len() - 4) % 64 != 0
                || old[..4] != new[..4]
            {
                return Err("delta resume changed Blob count/header".into());
            }
            for (index, (a, b)) in old[4..]
                .chunks_exact(64)
                .zip(new[4..].chunks_exact(64))
                .enumerate()
            {
                if a[..12] != b[..12] || a[16..] != b[16..] {
                    return Err(format!(
                        "delta resume changed immutable Blob fields at {index}"
                    ));
                }
                if a != b {
                    baseline.rewritten.insert(index as u32);
                }
            }
            baseline.original_blobs = old;
        } else if [
            "local/object_pack.bin",
            "local/object_pack_member.bin",
            "local/object_pack_id.idx",
        ]
        .contains(&file.relative_path.as_str())
        {
            if !new.starts_with(&old) {
                return Err(format!(
                    "delta resume changed immutable prefix {}",
                    file.relative_path
                ));
            }
            baseline.appended.push((file.relative_path.clone(), old));
        } else {
            if old != new {
                return Err(format!(
                    "delta resume changed immutable file {}",
                    file.relative_path
                ));
            }
            baseline
                .immutable
                .push((file.relative_path.clone(), Sha256::digest(old).to_vec()));
        }
    }
    if baseline.original_blobs.is_empty() || baseline.appended.len() != 3 {
        return Err("delta resume lacks original Blob/pack/member/index authority".into());
    }
    let old_count = |name: &str, width: usize| -> GenerationResult<u32> {
        let bytes = &baseline
            .appended
            .iter()
            .find(|(n, _)| n == name)
            .ok_or("missing append prefix")?
            .1;
        if bytes.len() < 4 || (bytes.len() - 4) % width != 0 {
            return Err("malformed captured append prefix".into());
        }
        u32::try_from((bytes.len() - 4) / width).map_err(|e| e.to_string())
    };
    let old_packs = old_count("local/object_pack.bin", 32)?;
    let old_members = old_count("local/object_pack_member.bin", 16)?;
    let db = LocalBinaryDbFs::new(
        root.join("local"),
        root.to_path_buf(),
        AuthorityId::new("delta-resume-admission"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
    let content = LocalContentBinaryDb::<1>::from_db_with_roots(
        db.clone(),
        root.to_path_buf(),
        root.to_path_buf(),
    );
    let read = BinaryDbReadTxn::new(&db);
    let pack_count = read.record_count(BinaryObjectPackCodec::<1>::record_file())?;
    let member_count = read.record_count(BinaryObjectPackMemberCodec::<1>::record_file())?;
    let mut next_member = old_members;
    let mut recovered = BTreeSet::new();
    for pack_index in old_packs..pack_count {
        let pack = content
            .object_packs()
            .object_pack_view_at(&read, pack_index)?;
        if pack.record.pack_meta != 1
            || pack.record.pack_format_kind != 1
            || pack.record.first_member_index != next_member
            || !(1..=128).contains(&pack.record.member_count)
        {
            return Err("delta resume replacement pack is not a complete ready append".into());
        }
        let mut deltas = 0;
        for index in next_member
            ..next_member
                .checked_add(pack.record.member_count)
                .ok_or("member range overflow")?
        {
            let member = content
                .object_packs()
                .object_pack_member_view_at(&read, index)?;
            let blob = content
                .blobs()
                .blob_view_at(&read, member.record.blob_index)?;
            if member.record.pack_index != pack_index
                || blob.pack_member_index != Some(index)
                || !baseline.rewritten.contains(&member.record.blob_index)
                || !recovered.insert(member.record.blob_index)
                || member.record.is_tombstone()
            {
                return Err("delta resume has an incomplete replacement locator transaction; preserve stage for bounded recovery".into());
            }
            deltas += usize::from(member.record.member_kind() == BinaryObjectPackMemberKind::Delta);
        }
        if deltas == 0 {
            return Err("delta resume appended a replacement pack without delta members".into());
        }
        baseline.delta_members += deltas;
        baseline.added_pack_bytes += direct_file_bytes(&root.join(&pack.pack_path))?.len() as u64;
        next_member += pack.record.member_count;
    }
    if next_member != member_count || recovered != baseline.rewritten {
        return Err("delta resume appended members and selected Blob locators disagree".into());
    }
    Ok(baseline)
}

#[derive(Clone)]
struct PackCostClosure {
    roots: Vec<u32>,
    selected_members: Vec<Option<u32>>,
    members: Vec<(u32, Option<u32>)>,
    pack_bytes: Vec<u64>,
}

impl PackCostClosure {
    fn bytes(&self) -> GenerationResult<u64> {
        fn visit(
            state: &PackCostClosure,
            blob: u32,
            visiting: &mut BTreeSet<u32>,
            done: &mut BTreeSet<u32>,
            packs: &mut BTreeSet<u32>,
        ) -> GenerationResult<()> {
            if done.contains(&blob) {
                return Ok(());
            }
            if visiting.len() > crate::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH {
                return Err("pack-cost Blob closure exceeds safety read depth".into());
            }
            if !visiting.insert(blob) {
                return Err("cyclic pack-cost Blob closure".into());
            }
            let member = state
                .selected_members
                .get(blob as usize)
                .copied()
                .flatten()
                .ok_or("missing pack-cost Blob locator")?;
            let (pack, base) = state
                .members
                .get(member as usize)
                .copied()
                .ok_or("invalid pack-cost member FK")?;
            if state.pack_bytes.get(pack as usize).is_none() {
                return Err("invalid pack-cost pack FK".into());
            }
            packs.insert(pack);
            if let Some(base) = base {
                visit(state, base, visiting, done, packs)?;
            }
            visiting.remove(&blob);
            done.insert(blob);
            Ok(())
        }
        let mut packs = BTreeSet::new();
        let mut done = BTreeSet::new();
        for root in &self.roots {
            visit(self, *root, &mut BTreeSet::new(), &mut done, &mut packs)?;
        }
        packs.into_iter().try_fold(0u64, |sum, pack| {
            sum.checked_add(self.pack_bytes[pack as usize])
                .ok_or_else(|| "pack-cost overflow".into())
        })
    }

    fn replacement_saves(
        &self,
        members: &[(u32, Option<u32>)],
        bytes: u64,
    ) -> GenerationResult<bool> {
        let before = self.bytes()?;
        let mut proposed = self.clone();
        let pack = u32::try_from(proposed.pack_bytes.len()).map_err(|e| e.to_string())?;
        proposed.pack_bytes.push(bytes);
        for (blob, base) in members {
            let member = u32::try_from(proposed.members.len()).map_err(|e| e.to_string())?;
            proposed.members.push((pack, *base));
            *proposed
                .selected_members
                .get_mut(*blob as usize)
                .ok_or("invalid replacement Blob FK")? = Some(member);
        }
        Ok(proposed.bytes()? < before)
    }
}

fn pack_cost_closure(
    content: &LocalContentBinaryDb<1>,
    root: &Path,
) -> GenerationResult<PackCostClosure> {
    let read = BinaryDbReadTxn::new(content.db());
    let blobs =
        fs::read(content.db().authority_root().0.join("blob.bin")).map_err(|e| e.to_string())?;
    let members = fs::read(
        content
            .db()
            .authority_root()
            .0
            .join("object_pack_member.bin"),
    )
    .map_err(|e| e.to_string())?;
    let mut state = PackCostClosure {
        roots: Vec::new(),
        selected_members: Vec::new(),
        members: Vec::new(),
        pack_bytes: Vec::new(),
    };
    for (index, bytes) in blobs[4..].chunks_exact(64).enumerate() {
        let row = BinaryBlobCodec::<1>::decode_record(bytes)?;
        let member = row.pack_member_index_plus1.checked_sub(1);
        state.selected_members.push(member);
        if !row.is_tombstone() && !row.is_pruned() && member.is_some() {
            state.roots.push(index as u32);
        }
    }
    for bytes in members[4..].chunks_exact(16) {
        let row = BinaryObjectPackMemberCodec::<1>::decode_record(bytes)?;
        state
            .members
            .push((row.pack_index, row.base_blob_index_plus1.checked_sub(1)));
    }
    for index in 0..read.record_count(BinaryObjectPackCodec::<1>::record_file())? {
        let pack = content.object_packs().object_pack_view_at(&read, index)?;
        state.pack_bytes.push(
            fs::metadata(root.join(pack.pack_path))
                .map_err(|e| e.to_string())?
                .len(),
        );
    }
    Ok(state)
}

fn repack_batch(
    content: &LocalContentBinaryDb<1>,
    root: &Path,
    bodies: &[(u32, Vec<u8>, String)],
    heads: &mut BTreeMap<String, u32>,
) -> GenerationResult<(Vec<u32>, usize, u64)> {
    let read = BinaryDbReadTxn::new(content.db());
    let mut logical = BTreeMap::new();
    for (index, body, hint) in bodies {
        let view = content.blobs().blob_view_at(&read, *index)?;
        if view.size_bytes != body.len() as u64
            || view.record.sha256 != Sha256::digest(body).as_slice()
        {
            return Err("captured Git Blob body/hash differs".into());
        }
        logical.insert(
            view.blob_id.clone(),
            (*index, body, hint, view.record.created_at_s),
        );
    }
    let candidates = logical
        .iter()
        .map(|(id, (_, body, hint, _))| PackCandidate {
            entry_name: format!("blobs/{id}"),
            blob_id: id.clone(),
            data: body.to_vec(),
            path_hint: Some((*hint).clone()),
            chain_depth: 0,
        })
        .collect::<Vec<_>>();
    drop(read);
    let members = content.raw_delta_pack_members(candidates, heads)?;
    let deltas = members.iter().filter(|m| m.entry_type == "delta").count();
    if deltas == 0 {
        for (index, _, hint) in bodies {
            heads.insert(hint.clone(), *index);
        }
        return Ok((Vec::new(), 0, 0));
    }
    let mut replacement = Vec::new();
    let read = BinaryDbReadTxn::new(content.db());
    for member in &members {
        let index = logical
            .get(&member.blob_id)
            .ok_or("replacement Blob absent")?
            .0;
        let base = member
            .base_blob_id
            .as_deref()
            .map(|id| {
                content
                    .blobs()
                    .get_blob_view(&read, id)?
                    .map(|view| view.blob_index)
                    .ok_or_else(|| "replacement base Blob absent".to_string())
            })
            .transpose()?;
        replacement.push((index, base));
    }
    drop(read);
    if !pack_cost_closure(content, root)?.replacement_saves(
        &replacement,
        estimated_typed_pack_archive_bytes(&members)? as u64,
    )? {
        for (index, _, hint) in bodies {
            heads.insert(hint.clone(), *index);
        }
        return Ok((Vec::new(), 0, 0));
    }
    let mut seed = b"native-git-delta-repack/whole-pack-v1\n".to_vec();
    for member in &members {
        seed.extend_from_slice(member.blob_id.as_bytes());
        seed.extend_from_slice(&Sha256::digest(&member.data));
        seed.extend_from_slice(&(member.chain_depth as u64).to_le_bytes());
        if let Some(base) = &member.base_blob_id {
            seed.extend_from_slice(base.as_bytes());
        }
    }
    let digest = Sha256::digest(seed);
    let suffix = digest[..6]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<String>();
    let pack_id = format!("PCK-{suffix}");
    let relative = default_object_pack_relative_path(&pack_id);
    let path = root.join(&relative);
    if path.exists() {
        return Err(format!(
            "replacement pack already exists; refusing overwrite: {}",
            path.display()
        ));
    }
    let stats = write_typed_pack_archive_with_format(
        path.to_str().ok_or("pack path is not UTF-8")?,
        &pack_id,
        CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        &members,
        crate::repository_pack_policy::zstd_only_object_pack_write_format(),
    )?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len()
        != estimated_typed_pack_archive_bytes(&members)? as u64
    {
        return Err("delta pack size estimate differs from actual codec".into());
    }
    let read = BinaryDbReadTxn::new(content.db());
    let first = read
        .record_count(BinaryDbObjectPackStore::<LocalBinaryDbFs, 1>::object_pack_member_file())?;
    drop(read);
    let metadata = members
        .iter()
        .map(|m| {
            let (_, body, _, created) =
                logical.get(&m.blob_id).ok_or("replacement member absent")?;
            Ok(BinaryDbObjectPackMemberWriteInput {
                blob_id: m.blob_id.clone(),
                sha256: Sha256::digest(body)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                size_bytes: body.len() as i64,
                pack_entry_type: m.entry_type.clone(),
                pack_base_blob_id: m.base_blob_id.clone(),
                pack_chain_depth: m.chain_depth as i64,
                created_at: chrono::DateTime::from_timestamp(*created as i64, 0)
                    .ok_or("invalid source Blob time")?
                    .to_rfc3339(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    )
    .record_object_pack_metadata(
        BinaryDbCommandScope::ContentWrite,
        &BinaryDbObjectPackWriteInput {
            pack_id,
            pack_rel_path: relative,
            pack_format: stats["pack_format"]
                .as_str()
                .ok_or("pack format absent")?
                .into(),
            member_count: members.len() as i64,
            total_bytes: stats["total_bytes"]
                .as_i64()
                .ok_or("pack byte count absent")?,
            created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            members: metadata,
        },
    )?;
    let check = BinaryDbReadTxn::new(content.db());
    if check
        .record_count(BinaryDbObjectPackStore::<LocalBinaryDbFs, 1>::object_pack_member_file())?
        != first + members.len() as u32
    {
        return Err("replacement pack did not append exact member range".into());
    }
    for (offset, member) in members.iter().enumerate() {
        let view = content
            .object_packs()
            .object_pack_member_view_at(&check, first + offset as u32)?;
        if view.blob_id != member.blob_id || view.record.blob_index != logical[&member.blob_id].0 {
            return Err("replacement member Blob backreference differs".into());
        }
    }
    drop(check);
    let mut write = BinaryDbWriteTxn::begin(content.db(), BinaryDbCommandScope::ContentWrite)?;
    let mut changed = Vec::new();
    for (offset, member) in members.iter().enumerate() {
        let (index, _, _, _) = logical
            .get(&member.blob_id)
            .ok_or("Blob locator source absent")?;
        let raw = write.read_record(BinaryBlobCodec::<1>::record_file(), *index)?;
        let mut record = BinaryBlobCodec::<1>::decode_record(&raw)?;
        record.pack_member_index_plus1 = first
            .checked_add(offset as u32)
            .and_then(|v| v.checked_add(1))
            .ok_or("replacement member index overflow")?;
        write.overwrite_record(
            BinaryBlobCodec::<1>::record_file(),
            *index,
            &BinaryBlobCodec::<1>::encode_record(&record)?,
        )?;
        changed.push(*index);
    }
    write.commit()?;
    for (index, _, hint) in bodies {
        heads.insert(hint.clone(), *index);
    }
    Ok((
        changed,
        deltas,
        fs::metadata(path).map_err(|e| e.to_string())?.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_store::BlobStore;
    use crate::git_binary_db::{git_object_id, GitObjectInput, GitObjectKind, GitRepositoryKind};
    use crate::git_binary_db::{GIT_OBJECT_STORE_BIN_PATHS, GIT_OBJECT_STORE_INDEX_PATHS};
    use std::process::{Command, Stdio};
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn net_pack_gate_rejects_partial_overlap_and_external_base_pins() {
        let partial = PackCostClosure {
            roots: vec![0, 1],
            selected_members: vec![Some(0), Some(1)],
            members: vec![(0, None), (0, None)],
            pack_bytes: vec![100],
        };
        assert!(
            !partial.replacement_saves(&[(0, None)], 40).unwrap(),
            "40 replacement bytes still retain the whole 100-byte old pack"
        );
        assert!(
            partial
                .replacement_saves(&[(0, None), (1, Some(0))], 80)
                .unwrap(),
            "whole-pack replacement retires the old 100-byte pack"
        );
        assert!(
            !partial
                .replacement_saves(&[(0, None), (1, Some(0))], 100)
                .unwrap(),
            "equal net bytes must not rewrite"
        );
        let external = PackCostClosure {
            roots: vec![1],
            selected_members: vec![Some(0), Some(1)],
            members: vec![(0, None), (1, None)],
            pack_bytes: vec![90, 100],
        };
        assert!(
            !external.replacement_saves(&[(1, Some(0))], 20).unwrap(),
            "new delta pins an additional 90-byte external base pack"
        );
        assert!(
            external.replacement_saves(&[(1, Some(0))], 5).unwrap(),
            "base-inclusive 95-byte closure beats 100 bytes"
        );
    }

    fn fixture(repo: &Path) -> Vec<([u8; 20], Vec<u8>)> {
        fs::create_dir_all(repo.join(".ait")).unwrap();
        fs::write(repo.join(".ait/config.json"), br#"{"repo_name":"fixture"}"#).unwrap();
        let db = LocalBinaryDbFs::new(
            repo.join(".ait/binary-db"),
            repo.to_path_buf(),
            AuthorityId::new("delta-test"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
        let content = LocalContentBinaryDb::<1>::from_db_with_roots(
            db,
            repo.to_path_buf(),
            repo.to_path_buf(),
        );
        let git_db = LocalBinaryDbFs::new(
            repo.join(".ait/git-interop/v1/binary-v0"),
            repo.to_path_buf(),
            AuthorityId::new("delta-test-git"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(GIT_OBJECT_STORE_BIN_PATHS)
        .with_declared_index_paths(GIT_OBJECT_STORE_INDEX_PATHS);
        let objects = LocalGitObjectStore::open(git_db, content).unwrap();
        let source = objects
            .ensure_repository(
                GitRepositoryKind::ImportSource,
                1,
                [4; 12],
                "fixture-source",
            )
            .unwrap();
        let mut state = 0x12345678u32;
        let base = (0..32768)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect::<Vec<_>>();
        let mut target = base.clone();
        target[15000] ^= 1;
        let mut first = vec![(
            GitObjectKind::Blob,
            git_object_id(GitObjectKind::Blob, &base),
            base.clone(),
        )];
        for n in 0..127 {
            let body = format!("filler-{n}").into_bytes();
            first.push((
                GitObjectKind::Tag,
                git_object_id(GitObjectKind::Tag, &body),
                body,
            ));
        }
        objects
            .put_batch(
                source,
                &first
                    .iter()
                    .map(|(kind, id, body)| GitObjectInput {
                        kind: *kind,
                        format: 1,
                        object_id: id,
                        body,
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let target_id = git_object_id(GitObjectKind::Blob, &target);
        objects
            .put(source, GitObjectKind::Blob, 1, &target_id, &target)
            .unwrap();
        vec![
            (git_object_id(GitObjectKind::Blob, &base), base),
            (target_id, target),
        ]
    }

    #[test]
    fn ordinal_preserving_delta_generation_activation_and_retirement() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        let expected = fixture(&repo);
        let original_content = LocalContentBinaryDb::<1>::new(
            repo.join(".ait/binary-db"),
            repo.clone(),
            AuthorityId::new("net-source"),
            LocalStateScope::Repository,
        );
        let before_pack_bytes = pack_cost_closure(&original_content, &repo)
            .unwrap()
            .bytes()
            .unwrap();
        let git_before =
            fs::read(repo.join(".ait/git-interop/v1/binary-v0/git_object.bin")).unwrap();
        let blob_before = fs::read(repo.join(".ait/binary-db/blob.bin")).unwrap();
        let stage = temp.path().join("stage");
        let report = stage_binary_db_git_delta_repack(CaptureBinaryDbGenerationOptions {
            repo_root: repo.clone(),
            output_root: stage.clone(),
            jobs: 1,
        })
        .unwrap();
        assert!(report.delta_members > 0);
        let staged_db = LocalBinaryDbFs::new(
            stage.join("local"),
            stage.clone(),
            AuthorityId::new("net-stage"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS)
        .for_detached_generation_without_locks();
        let staged_content =
            LocalContentBinaryDb::<1>::from_db_with_roots(staged_db, stage.clone(), stage.clone());
        let after_pack_bytes = pack_cost_closure(&staged_content, &stage)
            .unwrap()
            .bytes()
            .unwrap();
        assert!(
            after_pack_bytes < before_pack_bytes,
            "the entire selected pack/base closure must shrink"
        );
        eprintln!("actual selected pack closure: before={before_pack_bytes} after={after_pack_bytes}; retained old files remain separate");
        assert_eq!(
            fs::read(repo.join(".ait/binary-db/blob.bin")).unwrap(),
            blob_before
        );
        let activated = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo.clone(),
            generation_root: stage,
            expected_current_authority_fingerprint: None,
        })
        .unwrap();
        assert!(activated.retained_previous_authority.is_some());
        assert_eq!(
            fs::read(repo.join(".ait/git-interop/v1/binary-v0/git_object.bin")).unwrap(),
            git_before
        );
        let content = LocalContentBinaryDb::<1>::new(
            repo.join(".ait/binary-db"),
            repo.clone(),
            AuthorityId::new("verify"),
            LocalStateScope::Repository,
        );
        for (_, body) in &expected {
            let id = crate::content_binary_db::blob_id_from_sha256(&Sha256::digest(body).into());
            assert_eq!(content.blobs().read_blob_bytes(&id).unwrap(), *body);
        }
        use crate::local_content_gc::LocalContentOrphanPackPruneStore;
        content.prune_orphan_packs().unwrap();
        for (_, body) in &expected {
            let id = crate::content_binary_db::blob_id_from_sha256(&Sha256::digest(body).into());
            assert_eq!(content.blobs().read_blob_bytes(&id).unwrap(), *body);
        }
        capture_binary_db_generation_preserving_ordinals(CaptureBinaryDbGenerationOptions {
            repo_root: repo.clone(),
            output_root: temp.path().join("recapture"),
            jobs: 1,
        })
        .unwrap();
        let correct = fs::read(repo.join(".ait/binary-db/blob.bin")).unwrap();
        let hash = Sha256::digest(&expected[1].1);
        let ordinal = correct[4..]
            .chunks_exact(64)
            .position(|row| row[32..64] == hash[..])
            .unwrap();
        let mut invalid = correct.clone();
        invalid[4 + ordinal * 64 + 12..4 + ordinal * 64 + 16]
            .copy_from_slice(&blob_before[4 + ordinal * 64 + 12..4 + ordinal * 64 + 16]);
        fs::write(repo.join(".ait/binary-db/blob.bin"), invalid).unwrap();
        assert!(
            capture_binary_db_generation_preserving_ordinals(CaptureBinaryDbGenerationOptions {
                repo_root: repo.clone(),
                output_root: temp.path().join("invalid-selected-retired"),
                jobs: 1
            })
            .is_err(),
            "selected retired member must fail even in ordinal-preserving mode"
        );
        fs::write(repo.join(".ait/binary-db/blob.bin"), correct).unwrap();
    }

    #[test]
    #[ignore = "owned activation SIGKILL worker"]
    fn delta_activation_worker() {
        let Some(root) = std::env::var_os("AIT_DELTA_ACTIVATION_WORKER_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: root.join("repo"),
            generation_root: root.join("stage"),
            expected_current_authority_fingerprint: None,
        })
        .unwrap();
    }

    #[test]
    #[ignore = "owned staging SIGKILL worker"]
    fn delta_staging_worker() {
        let Some(root) = std::env::var_os("AIT_DELTA_ACTIVATION_WORKER_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        stage_binary_db_git_delta_repack(CaptureBinaryDbGenerationOptions {
            repo_root: root.join("repo"),
            output_root: root.join("stage"),
            jobs: 1,
        })
        .unwrap();
    }

    #[test]
    fn delta_generation_actual_sigkill_before_seal_keeps_active_source() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        fixture(&repo);
        let before = fs::read(repo.join(".ait/binary-db/blob.bin")).unwrap();
        let marker = temp.path().join("staged.marker");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "binary_db_generation::generation_git_delta::tests::delta_staging_worker",
                "--ignored",
            ])
            .env("AIT_DELTA_ACTIVATION_WORKER_ROOT", temp.path())
            .env("AIT_GIT_DELTA_STAGE_PAUSE", &marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !marker.exists() {
            assert!(Instant::now() < deadline);
            assert!(child.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(
            fs::read(repo.join(".ait/binary-db/blob.bin")).unwrap(),
            before
        );
        assert!(
            validate_binary_db_generation(&temp.path().join("stage"), "fixture").is_err(),
            "unsealed partial conversion must never activate"
        );
        let stage = temp.path().join("stage");
        let stage_blob_path = stage.join("local/blob.bin");
        let staged_blobs = fs::read(&stage_blob_path).unwrap();
        let source_blob_path = repo.join(".ait/binary-db/blob.bin");
        let mut changed_source = before.clone();
        changed_source[4 + 16] ^= 1;
        fs::write(&source_blob_path, changed_source).unwrap();
        assert!(resume_binary_db_git_delta_repack(&repo, &stage)
            .unwrap_err()
            .contains("source CAS"));
        fs::write(&source_blob_path, &before).unwrap();
        let git_path = stage.join(".ait/git-interop/v1/binary-v0/git_object.bin");
        let git_bytes = fs::read(&git_path).unwrap();
        let mut wrong_git = git_bytes.clone();
        *wrong_git.last_mut().unwrap() ^= 1;
        fs::write(&git_path, wrong_git).unwrap();
        assert!(resume_binary_db_git_delta_repack(&repo, &stage)
            .unwrap_err()
            .contains("immutable file"));
        fs::write(&git_path, git_bytes).unwrap();
        let member_path = stage.join("local/object_pack_member.bin");
        let member_bytes = fs::read(&member_path).unwrap();
        let mut wrong_prefix = member_bytes.clone();
        wrong_prefix[4] ^= 1;
        fs::write(&member_path, wrong_prefix).unwrap();
        assert!(resume_binary_db_git_delta_repack(&repo, &stage)
            .unwrap_err()
            .contains("immutable prefix"));
        fs::write(&member_path, member_bytes).unwrap();
        let mut incomplete = staged_blobs.clone();
        for (index, (a, b)) in before[4..]
            .chunks_exact(64)
            .zip(staged_blobs[4..].chunks_exact(64))
            .enumerate()
        {
            if a != b {
                let at = 4 + index * 64 + 12;
                incomplete[at..at + 4].copy_from_slice(&a[12..16]);
                break;
            }
        }
        fs::write(&stage_blob_path, incomplete).unwrap();
        assert!(resume_binary_db_git_delta_repack(&repo, &stage)
            .unwrap_err()
            .contains("incomplete replacement locator"));
        fs::write(&stage_blob_path, &staged_blobs).unwrap();
        let report = resume_binary_db_git_delta_repack(&repo, &stage).unwrap();
        assert!(report.resumed_existing_stage);
        assert_eq!(report.capture_seconds, 0.0);
        assert_eq!(report.resumed_objects, 1);
        assert_eq!(report.resumed_blob_locators, 1);
        assert_eq!(report.rewritten_blob_locators, 1);
        assert_eq!(fs::read(source_blob_path).unwrap(), before);
        activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo,
            generation_root: stage,
            expected_current_authority_fingerprint: None,
        })
        .unwrap();
    }

    #[test]
    #[ignore = "explicit sealed partial-root staged repack"]
    fn explicit_partial_root_delta_repack() {
        let repo = PathBuf::from(std::env::var_os("AIT_GIT_DELTA_REPACK_REPO").unwrap());
        let stage = PathBuf::from(std::env::var_os("AIT_GIT_DELTA_REPACK_STAGE").unwrap());
        let output = PathBuf::from(std::env::var_os("AIT_GIT_DELTA_REPACK_REPORT").unwrap());
        let report =
            if let Some(original) = std::env::var_os("AIT_GIT_DELTA_REPACK_ORIGINAL_MANIFEST") {
                restage_binary_db_git_delta_whole_packs(&repo, &stage, &PathBuf::from(original))
            } else if std::env::var("AIT_GIT_DELTA_REPACK_RESUME").as_deref() == Ok("1") {
                resume_binary_db_git_delta_repack(&repo, &stage)
            } else {
                stage_binary_db_git_delta_repack(CaptureBinaryDbGenerationOptions {
                    repo_root: repo,
                    output_root: stage,
                    jobs: 1,
                })
            }
            .unwrap();
        super::super::generation_manifest::write_json(&output, &report).unwrap();
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }

    #[test]
    fn delta_generation_actual_sigkill_after_atomic_activation_replays() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        fixture(&repo);
        let stage = temp.path().join("stage");
        stage_binary_db_git_delta_repack(CaptureBinaryDbGenerationOptions {
            repo_root: repo.clone(),
            output_root: stage.clone(),
            jobs: 1,
        })
        .unwrap();
        let before = fs::read(repo.join(".ait/git-interop/v1/binary-v0/git_object.bin")).unwrap();
        let marker = temp.path().join("activated.marker");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "binary_db_generation::generation_git_delta::tests::delta_activation_worker",
                "--ignored",
            ])
            .env("AIT_DELTA_ACTIVATION_WORKER_ROOT", temp.path())
            .env("AIT_GIT_CAPTURE_TEST_PAUSE_MARKER", &marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !marker.exists() {
            assert!(Instant::now() < deadline, "activation marker timeout");
            assert!(
                child.try_wait().unwrap().is_none(),
                "worker ended before pause"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo.clone(),
            generation_root: stage,
            expected_current_authority_fingerprint: None,
        })
        .unwrap();
        assert_eq!(
            fs::read(repo.join(".ait/git-interop/v1/binary-v0/git_object.bin")).unwrap(),
            before
        );
        capture_binary_db_generation_preserving_ordinals(CaptureBinaryDbGenerationOptions {
            repo_root: repo,
            output_root: temp.path().join("after"),
            jobs: 1,
        })
        .unwrap();
    }
}
