//! Exact Git source history through native Binary DB and shared zstd content.
//! JSON here is command presentation only, never persistent mapping authority.
use super::*;
use ait_core::binary_db::{AuthorityId, BinaryDbReadTxn, LocalBinaryDbFs, LocalStateScope};
use ait_core::content_binary_db::adapters::local::RawTreeSession;
use ait_core::content_binary_db::{BinarySnapshotView, LocalContentBinaryDb};
use ait_core::git_binary_db::*;

pub(super) mod migration;
mod mirror_baseline;
pub(super) use mirror_baseline::{latest_mirror_baseline, record_mirror_baseline};

#[cfg(test)]
fn root(repo: &RepoRuntime) -> PathBuf {
    InteropStore::new(repo).root.join("binary-v0")
}
pub(super) fn exists(repo: &RepoRuntime) -> bool {
    GitInteropBundle::exists(&repo.authoritative_repo_root())
}
pub(super) fn admit_fresh(repo: &RepoRuntime) -> Result<bool, String> {
    if !exists(repo) && !InteropStore::new(repo).load_mappings()?.is_empty() {
        return Err("legacy_migration_required: preserve old JSON/ODB evidence; native conversion has not been admitted".into());
    }
    Ok(true)
}
fn recovery_db(repo: &RepoRuntime, apply: bool) -> Result<LocalBinaryDbFs, String> {
    let canonical = repo.authoritative_repo_root();
    let bundle = GitInteropBundle::open_for_recovery(&canonical, true)?;
    if apply {
        bundle.retire_recovery_duplicates()?;
    }
    Ok(bundle.database(
        &canonical,
        AuthorityId::new(format!("git:{}", repo.repo_name())),
    ))
}
fn stores(repo: &RepoRuntime) -> Result<exports::Bundle, String> {
    stores_mode(repo, true)
}
fn read_stores(repo: &RepoRuntime) -> Result<exports::Bundle, String> {
    stores_mode(repo, false)
}
#[cfg(test)]
fn read_native_file(repo: &RepoRuntime, name: &str) -> Vec<u8> {
    let bundle =
        GitInteropBundle::open_for_recovery(&repo.authoritative_repo_root(), false).unwrap();
    fs::read(bundle.decoded_root().join(name)).unwrap()
}
#[cfg(test)]
fn mutate_native_files(repo: &RepoRuntime, edit: impl FnOnce(&Path)) {
    let canonical = repo.authoritative_repo_root();
    let bundle = GitInteropBundle::open_for_recovery(&canonical, true).unwrap();
    edit(bundle.decoded_root());
    let mut candidate = tempfile::NamedTempFile::new_in(canonical.join(".ait/git")).unwrap();
    let mut zstd = zstd::stream::write::Encoder::new(candidate.as_file_mut(), 3).unwrap();
    zstd.include_checksum(true).unwrap();
    let mut archive = tar::Builder::new(zstd);
    for name in GIT_OBJECT_STORE_BIN_PATHS
        .iter()
        .chain(GIT_OBJECT_STORE_INDEX_PATHS)
    {
        let mut file = fs::File::open(bundle.decoded_root().join(name)).unwrap();
        let mut header = tar::Header::new_ustar();
        header.set_size(file.metadata().unwrap().len());
        header.set_mode(0o600);
        header.set_cksum();
        archive.append_data(&mut header, *name, &mut file).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
    candidate.as_file().sync_all().unwrap();
    candidate
        .persist(canonical.join(GIT_INTEROP_BUNDLE_PATH))
        .unwrap();
}
fn stores_mode(repo: &RepoRuntime, writable: bool) -> Result<exports::Bundle, String> {
    let canonical = repo.authoritative_repo_root();
    let bundle = GitInteropBundle::open(&canonical, writable)?;
    let content = repo.binary_db_stores::<1>().content();
    let db = bundle.database(
        &canonical,
        AuthorityId::new(format!("git:{}", repo.repo_name())),
    );
    let objects = LocalGitObjectStore::open(db.clone(), content.clone())?;
    let history = GitHistoryStore::open(db.clone())?;
    let documents = RawGitDocumentStore::open(db, content.clone())?;
    admission::validate(&objects, &history, &documents, &content)?;
    if writable && bundle.needs_migration()? {
        bundle.publish_admitted()?;
    }
    Ok((objects, history, documents, content))
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn bytes<const N: usize>(text: &str) -> Result<[u8; N], String> {
    let text = text.rsplit('-').next().unwrap_or(text);
    if text.len() != N * 2 || !text.is_ascii() {
        return Err("invalid exact Git identity width".into());
    }
    let mut result = [0; N];
    for (i, v) in result.iter_mut().enumerate() {
        *v = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| "invalid Git identity hex")?;
    }
    Ok(result)
}
fn kind(name: &str) -> Result<GitObjectKind, String> {
    match name {
        "commit" => Ok(GitObjectKind::Commit),
        "tree" => Ok(GitObjectKind::Tree),
        "blob" => Ok(GitObjectKind::Blob),
        "tag" => Ok(GitObjectKind::Tag),
        _ => Err("unsupported Git object type".into()),
    }
}
fn kind_name(kind: GitObjectKind) -> &'static str {
    match kind {
        GitObjectKind::Commit => "commit",
        GitObjectKind::Tree => "tree",
        GitObjectKind::Blob => "blob",
        GitObjectKind::Tag => "tag",
    }
}
fn mode(text: &str) -> Result<u16, String> {
    match text {
        "100644" => Ok(0o644),
        "100755" => Ok(0o755),
        "120000" => Ok(0o120000),
        _ => Err("unsupported Git file mode".into()),
    }
}
fn identity<'a>(commit: &'a [u8], prefix: &[u8]) -> Result<&'a [u8], String> {
    commit
        .split(|b| *b == b'\n')
        .take_while(|row| !row.is_empty())
        .find_map(|row| row.strip_prefix(prefix))
        .ok_or_else(|| "raw Git identity missing".into())
}
fn snapshot(content: &LocalContentBinaryDb<1>, index: u32) -> Result<BinarySnapshotView, String> {
    let read = BinaryDbReadTxn::new(content.db());
    content
        .snapshots()
        .snapshot_view_at(&read, index)
        .map_err(Into::into)
}

fn imported_tag_created_at(
    objects: &LocalGitObjectStore,
    history: &GitHistoryStore,
    row: &GitRefMappingRecord,
) -> Result<u64, String> {
    let generations = history.generations()?;
    let generation = generations
        .get(row.generation_index as usize)
        .ok_or("Tag generation absent")?;
    if !generation.export {
        let identity = objects
            .repository_identity(generation.source_repository_index)?
            .2;
        if history
            .checkpoint_by_hash(&mirror_baseline::baseline_operation(
                &identity,
                generation.hash,
            )?)?
            .is_some_and(|r| r.status == 1 && r.generation_index == row.generation_index)
        {
            return Ok(row.created_at_s);
        }
    }
    // Ordinary generations freeze operation time. The first imported binding
    // records Tag creation; incremental imports must not advance that metadata.
    let mut clock = None;
    for (_, _, prior) in history.refs()?.into_iter().filter(|(_, _, r)| {
        r.meta & 13 == 13
            && r.target_index_plus1 == row.target_index_plus1
            && r.object_id == row.object_id
            && r.generation_index <= row.generation_index
            && generations
                .get(r.generation_index as usize)
                .is_some_and(|g| !g.export)
    }) {
        let source = &generations[prior.generation_index as usize];
        let identity = objects
            .repository_identity(source.source_repository_index)?
            .2;
        let baseline = history
            .checkpoint_by_hash(&mirror_baseline::baseline_operation(
                &identity,
                source.hash,
            )?)?
            .is_some_and(|r| r.status == 1 && r.generation_index == prior.generation_index);
        if clock.is_none() || baseline {
            clock = Some(prior.created_at_s);
        }
    }
    Ok(clock.unwrap_or(row.created_at_s))
}
fn checkpoint(
    history: &GitHistoryStore,
    generation: u32,
    operation: [u8; 8],
    plan: [u8; 12],
    status: u8,
    next_commit: u32,
    next_ref: u32,
) -> Result<(), String> {
    history.checkpoint(GitOperationCheckpointRecord {
        status,
        generation_index: generation,
        operation_hash: operation,
        plan_hash: plan,
        next_commit,
        next_ref,
        updated_at_s: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs(),
    })?;
    Ok(())
}

pub(super) struct ImportPlan<'a> {
    pub git_dir: &'a Path,
    pub source: &'a SourceInfo,
    pub heads: &'a [ImportedRef],
    pub tags: &'a [ImportedRef],
    pub commits: &'a [CommitData],
    pub operation: &'a str,
    pub plan: &'a str,
    pub classifications: JsonValue,
}
pub(super) fn import(repo: &RepoRuntime, input: ImportPlan<'_>) -> Result<JsonValue, String> {
    let ImportPlan {
        git_dir,
        source,
        heads,
        tags,
        commits,
        operation,
        plan,
        classifications,
    } = input;
    let mut markdown_oids = BTreeSet::new();
    // Validate every path/mode and raw symlink before touching native authority.
    let mut reader = GitImportObjectReader::new(git_dir)?;
    for commit in commits {
        for file in &commit.files {
            validate_git_artifact_path(&file.path)?;
            if file.path.to_ascii_lowercase().ends_with(".md") {
                markdown_oids.insert(bytes::<20>(&file.object_id)?);
            }
            let native_mode = mode(&file.mode)?;
            if native_mode == 0o120000 && reader.content(&file.object_id, "blob")?.contains(&0) {
                return Err("Git symlink target contains NUL".into());
            }
        }
    }
    let (objects, mut history, mut documents, content) = stores(repo)?;
    let source_index = objects.ensure_repository(
        GitRepositoryKind::ImportSource,
        1,
        bytes(&source.fingerprint)?,
        &source.source_identity,
    )?;
    let generation = history.ensure_generation(GitGenerationRecord {
        export: false,
        source_repository_index: source_index,
        target_repository_index_plus1: 0,
        hash: bytes(&source.generation_id)?,
    })?;
    let operation_hash = bytes(operation)?;
    let plan_hash = bytes(plan)?;
    let old_checkpoint = history.checkpoint_by_hash(&operation_hash)?;
    if let Some(old) = &old_checkpoint {
        if old.generation_index != generation || old.plan_hash != plan_hash {
            return Err("native checkpoint does not match the current immutable plan".into());
        }
    }
    let previous_refs = history.refs()?;
    let mut planned = heads
        .iter()
        .chain(tags)
        .map(|r| (r.source_name.clone(), r.object_id.clone()))
        .collect::<BTreeMap<_, _>>();
    if let Some(oid) = &source.head_object_id {
        planned.insert("HEAD".into(), oid.clone());
    }
    let prior_planned = previous_refs
        .iter()
        .filter(|(_, _, r)| r.generation_index == generation && r.meta & 12 == 0)
        .map(|(_, name, r)| (name.clone(), hex(&r.object_id)))
        .collect::<BTreeMap<_, _>>();
    if !prior_planned.is_empty() && prior_planned != planned {
        return Err(
            "native generation ref intent changed; choose the original source selection".into(),
        );
    }
    if old_checkpoint.as_ref().is_some_and(|r| r.status == 1) {
        return Ok(
            json!({"contract":INTEROP_CONTRACT,"operation":"import","status":"no_op","replayed":true,"operation_id":operation,"generation_id":source.generation_id,"plan_hash":plan,"source":source.source_identity,"source_repository_fingerprint":source.fingerprint,"git_object_format":"sha1","commit_count":commits.len(),"imported_commit_count":0,"reused_commit_count":commits.len(),"line_count":heads.len(),"tag_count":tags.len(),"classifications":classifications,"dry_run":false,"mutated":false,"blockers":[],"resume_supported":true,"head_symbolic_ref":source.head_symbolic_ref,"symbolic_head_mapped":source.head_symbolic_ref.is_some(),"head_object_id":source.head_object_id,"storage":"native_binary","mapping_json_count":0}),
        );
    }
    let created = previous_refs
        .iter()
        .find(|(_, _, r)| r.generation_index == generation)
        .map(|(_, _, r)| r.created_at_s)
        .unwrap_or(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs(),
        );
    let mut branch_ordinal = None;
    let tag_store = crate::tag_surface::tag_store(repo)?;
    // Heads precede HEAD so symbolic HEAD owns an exact ref ordinal.
    for (name, oid) in planned
        .iter()
        .filter(|(n, _)| n.as_str() != "HEAD")
        .chain(planned.iter().filter(|(n, _)| n.as_str() == "HEAD"))
    {
        let object_kind = kind(&reader.info.request(oid)?.object_type)?;
        let symbolic = if name == "HEAD" {
            source
                .head_symbolic_ref
                .as_ref()
                .and_then(|target| planned.get(target))
                .is_some()
        } else {
            false
        };
        let symbolic_target = if symbolic {
            branch_ordinal.ok_or("symbolic HEAD source ref absent")?
        } else {
            0
        };
        let r = GitRefMappingRecord {
            meta: if symbolic {
                2
            } else if name == "HEAD" {
                32
            } else if name.starts_with("refs/tags/") {
                1
            } else {
                0
            },
            object_kind,
            generation_index: generation,
            snapshot_index_plus1: 0,
            target_index_plus1: 0,
            symbolic_target_index_plus1: symbolic_target,
            object_id: bytes(oid)?,
            expected_previous: [0; 20],
            // Resume uses the saved source intent, independent of mutable Tags.
            created_at_s: previous_refs
                .iter()
                .find(|(_, prior_name, r)| {
                    prior_name == name && r.generation_index == generation && r.meta & 12 == 0
                })
                .map_or(created, |(_, _, r)| r.created_at_s),
        };
        let ordinal = history.append_ref(name, r)?;
        if source.head_symbolic_ref.as_deref() == Some(name) {
            branch_ordinal = Some(ordinal.checked_add(1).ok_or("ref overflow")?);
        }
    }
    checkpoint(
        &history,
        generation,
        operation_hash,
        plan_hash,
        0,
        old_checkpoint.as_ref().map_or(0, |r| r.next_commit),
        0,
    )?;
    let roots = planned.values().cloned().collect::<BTreeSet<_>>();
    let mut args = vec![
        OsString::from("rev-list"),
        OsString::from("--objects"),
        OsString::from("--no-object-names"),
    ];
    args.extend(roots.into_iter().map(OsString::from));
    let closure = git_repo_bytes_os(git_dir, args, &[], None)?;
    let closure = String::from_utf8(closure).map_err(|_| "Git closure OIDs are not ASCII")?;
    let mut batch = Vec::<(GitObjectKind, [u8; 20], Vec<u8>)>::new();
    let mut batch_bytes = 0usize;
    let mut delta_hints = GitObjectDeltaHints::default();
    let mut batch_hints = Vec::new();
    let mut batch_partitions = Vec::new();
    for oid in closure.lines() {
        let info = reader.info.request(oid)?;
        let object_kind = kind(&info.object_type)?;
        // Preserve legacy single oversized-object support; this corpus has none.
        if !batch.is_empty() && batch_bytes.saturating_add(info.size) > 8 * 1024 * 1024 {
            persist_batch_with_delta_hints(
                &objects,
                source_index,
                &batch,
                &batch_hints,
                &batch_partitions,
            )?;
            batch.clear();
            batch_hints.clear();
            batch_partitions.clear();
            batch_bytes = 0;
        }
        let id = bytes::<20>(oid)?;
        let body = reader.content(oid, kind_name(object_kind))?;
        if body.len() != info.size {
            return Err("Git object size changed while importing raw closure".into());
        }
        batch_hints.push(delta_hints.observe(source_index, object_kind, id, &body));
        batch_partitions.push(object_kind == GitObjectKind::Blob && markdown_oids.contains(&id));
        batch_bytes += body.len();
        batch.push((object_kind, id, body));
        if batch.len() >= 128 || batch_bytes >= 8 * 1024 * 1024 {
            persist_batch_with_delta_hints(
                &objects,
                source_index,
                &batch,
                &batch_hints,
                &batch_partitions,
            )?;
            batch.clear();
            batch_hints.clear();
            batch_partitions.clear();
            batch_bytes = 0;
        }
    }
    persist_batch_with_delta_hints(
        &objects,
        source_index,
        &batch,
        &batch_hints,
        &batch_partitions,
    )?;
    let mut mapped = history
        .commit_mappings()
        .into_iter()
        .filter(|r| r.generation_index == generation)
        .map(|r| (hex(&r.object_id), r))
        .collect::<BTreeMap<_, _>>();
    let generations = history.generations()?;
    let prior_mappings = history
        .commit_mappings()
        .into_iter()
        .filter(|r| {
            r.generation_index != generation
                && generations
                    .get(r.generation_index as usize)
                    .is_some_and(|g| !g.export && g.source_repository_index == source_index)
        })
        .map(|r| (hex(&r.object_id), r))
        .collect::<BTreeMap<_, _>>();
    let mut tree_session = RawTreeSession::default();
    let mut imported = 0usize;
    for (i, commit) in commits.iter().enumerate() {
        if !mapped.contains_key(&commit.object_id) {
            if let Some(previous) = prior_mappings.get(&commit.object_id) {
                let mut alias = previous.clone();
                alias.generation_index = generation;
                history.append_commit(alias.clone())?;
                mapped.insert(commit.object_id.clone(), alias);
                checkpoint(
                    &history,
                    generation,
                    operation_hash,
                    plan_hash,
                    0,
                    u32::try_from(i + 1).map_err(|_| "commit cursor overflow")?,
                    0,
                )?;
                continue;
            }
            let mut parents = commit
                .parent_object_ids
                .iter()
                .map(|p| {
                    let record = mapped.get(p).ok_or("native Git parent mapping absent")?;
                    Ok(snapshot(&content, record.snapshot_index)?.snapshot_id)
                })
                .collect::<Result<Vec<_>, String>>()?;
            // Distinct source commits may project to one code Snapshot when
            // their differences are Markdown-only. Keep the first occurrence.
            let mut seen_parent_snapshots = BTreeSet::new();
            parents.retain(|parent| seen_parent_snapshots.insert(parent.clone()));
            let mut code = Vec::new();
            let mut code_files = Vec::new();
            let mut doc_files = Vec::new();
            for file in &commit.files {
                let oid = bytes::<20>(&file.object_id)?;
                if file.path.to_ascii_lowercase().ends_with(".md") {
                    doc_files.push(RawGitDocumentObjectInput {
                        path: &file.path,
                        mode: mode(&file.mode)?,
                        object_id: oid,
                    });
                } else {
                    code_files.push((file.path.clone(), oid, mode(&file.mode)?));
                }
            }
            for batch in code_files.chunks(128) {
                let ids = batch.iter().map(|(_, oid, _)| *oid).collect::<Vec<_>>();
                for ((path, _, mode), record) in
                    batch
                        .iter()
                        .zip(objects.get_record_batch(source_index, 1, &ids)?)
                {
                    if record.kind != GitObjectKind::Blob {
                        return Err("native source file is not a Blob".into());
                    }
                    code.push((path.clone(), record.content_blob_index, *mode));
                }
            }
            let document_set = documents.ensure_document_set_from_objects(
                &objects,
                source_index,
                bytes(&source.fingerprint)?,
                &source.source_identity,
                &doc_files,
            )?;
            let message = String::from_utf8_lossy(&commit.message_bytes);
            let view = content.create_git_code_snapshot(
                &repo.repo_name(),
                &format!("git/import/{}", source.fingerprint.to_ascii_lowercase()),
                &parents,
                Some(message.trim_end_matches(['\r', '\n'])),
                &code,
                &mut tree_session,
            )?;
            let record = GitCommitMappingRecord {
                mapping_meta: 10 | if commit.signed { 1 } else { 0 },
                generation_index: generation,
                snapshot_index: view.snapshot_index,
                author_identity_index: history
                    .append_identity(identity(&commit.raw_bytes, b"author ")?)?,
                committer_identity_index: history
                    .append_identity(identity(&commit.raw_bytes, b"committer ")?)?,
                object_id: bytes(&commit.object_id)?,
                tree_object_id: bytes(&commit.tree_object_id)?,
                lfs_pointer_count: u32::try_from(commit.lfs_pointer_count)
                    .map_err(|_| "LFS count overflow")?,
                created_at_s: created,
                document_set_tree_index: document_set,
            };
            history.append_commit(record.clone())?;
            #[cfg(test)]
            if let Some(marker) = std::env::var_os("AIT_GIT_IMPORT_TEST_PAUSE") {
                fs::write(marker, b"durable mapping before checkpoint cursor")
                    .map_err(|error| error.to_string())?;
                loop {
                    std::thread::park();
                }
            }
            mapped.insert(commit.object_id.clone(), record);
            imported += 1;
        }
        checkpoint(
            &history,
            generation,
            operation_hash,
            plan_hash,
            0,
            u32::try_from(i + 1).map_err(|_| "commit cursor overflow")?,
            0,
        )?;
    }
    let lines = repo.binary_db_stores::<1>().lines();
    let mut imported_lines = Vec::new();
    for (ref_index, head) in heads.iter().enumerate() {
        let mapping = mapped
            .get(&head.object_id)
            .ok_or("branch lacks native mapping")?;
        let view = snapshot(&content, mapping.snapshot_index)?;
        let name = head
            .source_name
            .strip_prefix("refs/heads/")
            .unwrap_or("git-head");
        let previous = lines.line_by_name(name)?;
        let line = match previous {
            None => lines.create_line(name, Some(&view.snapshot_id), &system_event_timestamp())?,
            Some(line) if line.head_snapshot_id.as_deref() == Some(&view.snapshot_id) => line,
            Some(line) if line.head_snapshot_id.is_none() => lines.compare_and_swap_line_head(
                name,
                None,
                Some(&view.snapshot_id),
                &system_event_timestamp(),
            )?,
            Some(line) => {
                let previous = previous_refs.iter().rev().find(|(_, n, r)| {
                    n == &head.source_name
                        && r.generation_index != generation
                        && r.meta & 4 != 0
                        && generations
                            .get(r.generation_index as usize)
                            .is_some_and(|g| !g.export && g.source_repository_index == source_index)
                });
                let expected = previous
                    .map(|(_, _, r)| snapshot(&content, r.snapshot_index_plus1 - 1))
                    .transpose()?
                    .map(|v| v.snapshot_id);
                if expected.as_deref() != line.head_snapshot_id.as_deref() || expected.is_none() {
                    return Err(format!(
                        "Git import refuses to overwrite advanced AIT Line {name}"
                    ));
                }
                lines.compare_and_swap_line_head(
                    name,
                    line.head_snapshot_id.as_deref(),
                    Some(&view.snapshot_id),
                    &system_event_timestamp(),
                )?
            }
        };
        imported_lines.push(json!({"git_ref_name":head.source_name,"line_id":line.line_id,"line_name":line.line_name,"snapshot_id":view.snapshot_id}));
        let planned_row = history
            .refs()?
            .into_iter()
            .find(|(_, n, r)| {
                n == &head.source_name && r.generation_index == generation && r.meta & 12 == 0
            })
            .ok_or("planned branch absent")?
            .2;
        if !history.refs()?.iter().any(|(_, n, r)| {
            n == &head.source_name && r.generation_index == generation && r.meta & 4 != 0
        }) {
            let mut bound = planned_row;
            bound.meta |= 12;
            bound.target_index_plus1 = ait_core::line_binary_db::binary_line_index_by_name(
                &BinaryDbReadTxn::new(content.db()),
                name,
            )?
            .ok_or("imported Line ordinal absent")?
            .checked_add(1)
            .ok_or("Line ordinal overflow")?;
            bound.snapshot_index_plus1 = mapping
                .snapshot_index
                .checked_add(1)
                .ok_or("snapshot overflow")?;
            history.append_ref(&head.source_name, bound)?;
        }
        checkpoint(
            &history,
            generation,
            operation_hash,
            plan_hash,
            0,
            commits
                .len()
                .try_into()
                .map_err(|_| "commit cursor overflow")?,
            (ref_index + 1)
                .try_into()
                .map_err(|_| "ref cursor overflow")?,
        )?;
    }
    for (ref_index, tag) in tags.iter().enumerate() {
        let name = tag
            .source_name
            .strip_prefix("refs/tags/")
            .ok_or("non-tag in tag selection")?;
        let mut oid = bytes::<20>(&tag.object_id)?;
        let mut seen = BTreeSet::new();
        let mut message = None;
        while seen.insert(oid) {
            let (record, body) = objects
                .get(source_index, 1, &oid)?
                .ok_or("tag closure incomplete")?;
            if record.kind == GitObjectKind::Tag {
                if message.is_none() {
                    message = body.windows(2).position(|v| v == b"\n\n").map(|at| {
                        String::from_utf8_lossy(&body[at + 2..])
                            .lines()
                            .map(str::trim)
                            .find(|v| !v.is_empty())
                            .unwrap_or("")
                            .to_string()
                    });
                }
                oid = children(record.kind, &body)?[0];
                continue;
            }
            if record.kind == GitObjectKind::Commit {
                let mapping = mapped.get(&hex(&oid)).ok_or("tag commit mapping absent")?;
                let view = snapshot(&content, mapping.snapshot_index)?;
                let message = message
                    .as_deref()
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Imported Git tag {name}"));
                let frozen_tag_time = history
                    .refs()?
                    .into_iter()
                    .find(|(_, n, r)| {
                        n == &tag.source_name
                            && r.generation_index == generation
                            && r.meta & 12 == 0
                    })
                    .ok_or("frozen tag absent")?
                    .2
                    .created_at_s;
                let tag_created = DateTime::<Utc>::from_timestamp(
                    i64::try_from(frozen_tag_time).map_err(|_| "Tag timestamp overflow")?,
                    0,
                )
                .ok_or("Tag timestamp out of range")?
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                let tag_index = tag_store.ensure_imported_tag(&new_tag_record(
                    name,
                    &view.snapshot_id,
                    &message,
                    &tag_created,
                )?)?;
                let refs = history.refs()?;
                if !refs.iter().any(|(_, n, r)| {
                    n == &tag.source_name && r.generation_index == generation && r.meta & 4 != 0
                }) {
                    let mut bound = refs
                        .into_iter()
                        .find(|(_, n, r)| {
                            n == &tag.source_name
                                && r.generation_index == generation
                                && r.meta & 12 == 0
                        })
                        .ok_or("frozen tag absent")?
                        .2;
                    bound.meta |= 12;
                    bound.target_index_plus1 =
                        tag_index.checked_add(1).ok_or("Tag ordinal overflow")?;
                    bound.snapshot_index_plus1 = mapping
                        .snapshot_index
                        .checked_add(1)
                        .ok_or("tag Snapshot overflow")?;
                    history.append_ref(&tag.source_name, bound)?;
                }
            }
            break;
        }
        checkpoint(
            &history,
            generation,
            operation_hash,
            plan_hash,
            0,
            commits
                .len()
                .try_into()
                .map_err(|_| "commit cursor overflow")?,
            (heads.len() + ref_index + 1)
                .try_into()
                .map_err(|_| "ref cursor overflow")?,
        )?;
    }
    checkpoint(
        &history,
        generation,
        operation_hash,
        plan_hash,
        1,
        commits
            .len()
            .try_into()
            .map_err(|_| "commit count overflow")?,
        planned.len().try_into().map_err(|_| "ref count overflow")?,
    )?;
    Ok(
        json!({"contract":INTEROP_CONTRACT,"operation":"import","status":if old_checkpoint.as_ref().is_some_and(|r|r.status==1) {"no_op"} else {"completed"},"operation_id":operation,"generation_id":source.generation_id,"plan_hash":plan,"source":source.source_identity,"source_repository_fingerprint":source.fingerprint,"git_object_format":"sha1","commit_count":commits.len(),"imported_commit_count":imported,"reused_commit_count":commits.len()-imported,"line_count":heads.len(),"tag_count":tags.len(),"imported_lines":imported_lines,"classifications":classifications,"dry_run":false,"mutated":imported>0 || old_checkpoint.as_ref().is_none_or(|r|r.status!=1),"blockers":[],"resume_supported":true,"head_symbolic_ref":source.head_symbolic_ref,"symbolic_head_mapped":source.head_symbolic_ref.is_some(),"head_object_id":source.head_object_id,"storage":"native_binary","mapping_json_count":0}),
    )
}
fn persist_batch(
    store: &LocalGitObjectStore,
    source: u32,
    batch: &[(GitObjectKind, [u8; 20], Vec<u8>)],
) -> Result<(), String> {
    if !batch.is_empty() {
        store.put_batch(
            source,
            &batch
                .iter()
                .map(|(kind, id, body)| GitObjectInput {
                    kind: *kind,
                    format: 1,
                    object_id: id,
                    body,
                })
                .collect::<Vec<_>>(),
        )?;
    }
    Ok(())
}

fn persist_batch_with_delta_hints(
    store: &LocalGitObjectStore,
    source: u32,
    batch: &[(GitObjectKind, [u8; 20], Vec<u8>)],
    hints: &[String],
    partitions: &[bool],
) -> Result<(), String> {
    if !batch.is_empty() {
        store.put_batch_with_delta_hints_and_partitions(
            source,
            &batch
                .iter()
                .map(|(kind, id, body)| GitObjectInput {
                    kind: *kind,
                    format: 1,
                    object_id: id,
                    body,
                })
                .collect::<Vec<_>>(),
            &hints
                .iter()
                .map(|hint| Some(hint.as_str()))
                .collect::<Vec<_>>(),
            partitions,
        )?;
    }
    Ok(())
}

fn select(
    history: &GitHistoryStore,
    generation: Option<&str>,
) -> Result<(u32, GitGenerationRecord), String> {
    let all = history
        .generations()?
        .into_iter()
        .enumerate()
        .filter(|(_, g)| !g.export && generation.is_none_or(|s| bytes::<8>(s).ok() == Some(g.hash)))
        .collect::<Vec<_>>();
    if all.is_empty()
        || (all.len() != 1
            && (generation.is_some()
                || all
                    .iter()
                    .map(|(_, g)| g.source_repository_index)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != 1))
    {
        return Err(
            "choose one exact imported --generation; Snapshot aliases do not select Git history"
                .into(),
        );
    }
    // Resolve the last frozen source state for one repository. This selects
    // refs in an exact generation; Snapshot aliases never choose an OID.
    let (index, record) = all.into_iter().next_back().unwrap();
    let index = index.try_into().map_err(|_| "generation overflow")?;
    if !history.generation_is_complete(index)? {
        return Err("selected native Git generation is incomplete; resume import first".into());
    }
    Ok((index, record))
}
// Traverse only selected raw Git roots. A path label never enters object identity.
fn children(kind: GitObjectKind, body: &[u8]) -> Result<Vec<[u8; 20]>, String> {
    match kind {
        GitObjectKind::Blob => Ok(Vec::new()),
        GitObjectKind::Commit => body
            .split(|b| *b == b'\n')
            .take_while(|r| !r.is_empty())
            .filter_map(|r| {
                r.strip_prefix(b"tree ")
                    .or_else(|| r.strip_prefix(b"parent "))
            })
            .map(|r| bytes(std::str::from_utf8(r).map_err(|_| "commit OID ASCII")?))
            .collect(),
        GitObjectKind::Tag => Ok(vec![bytes(
            std::str::from_utf8(
                body.split(|b| *b == b'\n')
                    .next()
                    .ok_or("tag header absent")?
                    .strip_prefix(b"object ")
                    .ok_or("tag target absent")?,
            )
            .map_err(|_| "tag OID ASCII")?,
        )?]),
        GitObjectKind::Tree => Ok(tree_entries(body)?
            .into_iter()
            .map(|(_, _, id)| id)
            .collect()),
    }
}
fn tree_entries(body: &[u8]) -> Result<Vec<(u16, String, [u8; 20])>, String> {
    let mut result = Vec::new();
    let mut at = 0;
    while at < body.len() {
        let space = body[at..]
            .iter()
            .position(|b| *b == b' ')
            .ok_or("tree mode absent")?
            + at;
        let nul = body[space + 1..]
            .iter()
            .position(|b| *b == 0)
            .ok_or("tree path absent")?
            + space
            + 1;
        let end = nul.checked_add(21).ok_or("tree overflow")?;
        if end > body.len() {
            return Err("truncated tree OID".into());
        }
        let mode = std::str::from_utf8(&body[at..space]).map_err(|_| "tree mode ASCII")?;
        let mode = if mode == "40000" {
            0o40000
        } else {
            self::mode(mode)?
        };
        let name = std::str::from_utf8(&body[space + 1..nul])
            .map_err(|_| "non-UTF8 Git path")?
            .to_string();
        validate_git_artifact_path(&name)?;
        if name.contains('/') {
            return Err("noncanonical tree segment".into());
        }
        result.push((mode, name, body[nul + 1..end].try_into().unwrap()));
        at = end;
    }
    Ok(result)
}
mod admission;
mod recovery;
pub(super) fn recover(repo: &RepoRuntime, apply: bool) -> Result<JsonValue, String> {
    recovery::recover(repo, apply)
}
mod exports;
pub(super) fn export_mirror(
    repo: &RepoRuntime,
    target: &str,
    dry_run: bool,
    stop_after_objects: bool,
) -> Result<JsonValue, String> {
    exports::current_with_control(repo, target, true, dry_run, stop_after_objects)
}
pub(super) fn mirror_projection(
    repo: &RepoRuntime,
    endpoint_identity: &str,
    endpoint_fingerprint: &str,
) -> Result<Vec<JsonValue>, String> {
    if !exists(repo) {
        return Ok(Vec::new());
    }
    let (objects, history, _, content) = read_stores(repo)?;
    let generations = history.generations()?;
    let refs = history.refs()?;
    let mut latest = BTreeMap::new();
    let mut projection = Vec::new();
    let mut endpoint_names = BTreeSet::new();
    for (at, generation) in generations.iter().enumerate() {
        if !history.generation_is_complete(at as u32)? {
            continue;
        }
        let repository = if generation.export {
            generation
                .target_repository_index_plus1
                .checked_sub(1)
                .ok_or("export target absent")?
        } else {
            generation.source_repository_index
        };
        let (_, fingerprint, stored_identity) = objects.repository_identity(repository)?;
        if stored_identity != endpoint_identity {
            continue;
        }
        endpoint_names.extend(
            refs.iter()
                .filter(|(_, name, r)| {
                    r.generation_index == at as u32
                        && (name.starts_with("refs/heads/") || name.starts_with("refs/tags/"))
                })
                .map(|(_, name, _)| name.clone()),
        );
        let baseline = history
            .checkpoint_by_hash(&mirror_baseline::baseline_operation(
                endpoint_identity,
                generation.hash,
            )?)?
            .is_some_and(|r| r.status == 1 && r.generation_index == at as u32);
        if baseline {
            let live = refs
                .iter()
                .filter(|(_, _, r)| r.generation_index == at as u32)
                .map(|(_, name, _)| name.as_str())
                .collect::<BTreeSet<_>>();
            for name in endpoint_names
                .iter()
                .filter(|name| !live.contains(name.as_str()))
            {
                latest.insert(name.clone(),json!({"kind":"mirror_ref","created_at":format!("{at:010}"),"endpoint_repository_fingerprint":endpoint_fingerprint,"git_ref_name":name,"git_object_id":null,"ait_state_id":null,"ait_kind":if name.starts_with("refs/tags/"){"tag"}else{"line"},"ait_name":name.strip_prefix("refs/tags/").or_else(||name.strip_prefix("refs/heads/")).unwrap()}));
            }
        }
        for (_, name, row) in refs.iter().filter(|(_, name, r)| {
            r.generation_index == at as u32 && name.starts_with("refs/tags/") && r.meta & 12 == 0
        }) {
            let mut oid = row.object_id;
            let mut seen = BTreeSet::new();
            loop {
                if !seen.insert(oid) {
                    return Err("cyclic raw mirror Tag".into());
                }
                let (object, body) = objects
                    .get(repository, 1, &oid)?
                    .ok_or("raw mirror Tag absent")?;
                if object.kind == GitObjectKind::Tag {
                    oid = children(object.kind, &body)?[0];
                    continue;
                }
                if object.kind != GitObjectKind::Commit {
                    latest.insert(name.clone(),json!({"kind":"mirror_ref","created_at":format!("{at:010}"),"endpoint_repository_fingerprint":endpoint_fingerprint,"git_ref_name":name,"git_object_id":hex(&row.object_id),"ait_state_id":null,"ait_kind":"tag","ait_name":name.strip_prefix("refs/tags/").unwrap()}));
                }
                break;
            }
        }
        for (_, name, row) in refs.iter().filter(|(_, name, r)| {
            r.generation_index == at as u32
                && r.meta & 12 == 12
                && (name.starts_with("refs/heads/") || name.starts_with("refs/tags/"))
        }) {
            let is_tag = name.starts_with("refs/tags/");
            let ait_name = name
                .strip_prefix(if is_tag { "refs/tags/" } else { "refs/heads/" })
                .unwrap();
            let view = snapshot(&content, row.snapshot_index_plus1 - 1)?;
            let (message, created) = if is_tag {
                let (object_record, raw) = objects
                    .get(repository, 1, &row.object_id)?
                    .ok_or("mirrored Tag body absent")?;
                let tag_message = if object_record.kind == GitObjectKind::Tag {
                    split_object_message(&raw)
                } else {
                    b""
                };
                let message = String::from_utf8_lossy(tag_message)
                    .lines()
                    .map(str::trim)
                    .find(|s| !s.is_empty())
                    .map(str::to_string)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("Imported Git tag {ait_name}"));
                let seconds = imported_tag_created_at(&objects, &history, row)?;
                (Some(message), Some(epoch_text(seconds)?))
            } else {
                let read = BinaryDbReadTxn::new(content.db());
                let line = ait_core::line_binary_db::BinaryLineCodec::<1>::decode_record(
                    &read.read_record(
                        ait_core::line_binary_db::BinaryLineCodec::<1>::record_file(),
                        row.target_index_plus1 - 1,
                    )?,
                )?;
                (
                    None,
                    if line.created_at_s == 0 {
                        None
                    } else {
                        Some(epoch_text(line.created_at_s)?)
                    },
                )
            };
            let previous = ExportRef {
                git_ref_name: name.clone(),
                snapshot_id: view.snapshot_id.clone(),
                ait_kind: if is_tag { "tag" } else { "line" }.into(),
                ait_name: ait_name.into(),
                ait_identity: None,
                message,
                created_at: created,
            };
            let air = super::mirror::ait_ref_state_id(&previous);
            projection.push(json!({"kind":if is_tag {"tag"}else {"ref"},"direction":if generation.export {"export"}else {"import"},"source_repository_fingerprint":if generation.export {None}else {Some(format!("GSR-{}",hex(&fingerprint).to_ascii_uppercase()))},"target_repository_fingerprint":if generation.export {Some(format!("GTR-{}",hex(&fingerprint).to_ascii_uppercase()))}else {None},"git_ref_name":name,"git_object_id":hex(&row.object_id),"snapshot_id":view.snapshot_id,"ait_state_id":air}));
            latest.insert(name.clone(),json!({"kind":"mirror_ref","created_at":format!("{at:010}"),"endpoint_repository_fingerprint":endpoint_fingerprint,"git_ref_name":name,"git_object_id":hex(&row.object_id),"ait_state_id":air,"ait_kind":previous.ait_kind,"ait_name":previous.ait_name}));
        }
    }
    projection.extend(latest.into_values());
    Ok(projection)
}
fn epoch_text(seconds: u64) -> Result<String, String> {
    Ok(DateTime::<Utc>::from_timestamp(
        i64::try_from(seconds).map_err(|_| "timestamp overflow")?,
        0,
    )
    .ok_or("timestamp out of range")?
    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}
pub(super) fn export(
    repo: &RepoRuntime,
    target: &str,
    generation: Option<&str>,
    object: Option<&str>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    exports::source(repo, target, generation, object, dry_run)
}
pub(super) fn export_current(
    repo: &RepoRuntime,
    target: &str,
    all: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    exports::current(repo, target, all, dry_run)
}

fn copy_verified_objects(
    source: &Path,
    target: &Path,
    selected: &BTreeSet<[u8; 20]>,
) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if name_text.len() != 2 || !name_text.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let dest = target.join(&name);
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        for object in fs::read_dir(entry.path()).map_err(|e| e.to_string())? {
            let object = object.map_err(|e| e.to_string())?;
            let oid_text = format!("{name_text}{}", object.file_name().to_string_lossy());
            if !bytes::<20>(&oid_text).is_ok_and(|oid| selected.contains(&oid)) {
                continue;
            }
            let destination = dest.join(object.file_name());
            if destination.exists() {
                if fs::read(&destination).map_err(|e| e.to_string())?
                    != fs::read(object.path()).map_err(|e| e.to_string())?
                {
                    let oid = bytes::<20>(&oid_text)?;
                    if exports::verified_loose_object_bytes(&destination, &oid)?
                        != exports::verified_loose_object_bytes(&object.path(), &oid)?
                    {
                        return Err("target loose object collision".into());
                    }
                }
            } else {
                let mut input = fs::File::open(object.path()).map_err(|e| e.to_string())?;
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .map_err(|e| e.to_string())?;
                std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
                output.sync_all().map_err(|e| e.to_string())?;
            }
        }
        fs::File::open(&dest)
            .and_then(|d| d.sync_all())
            .map_err(|e| e.to_string())?;
    }
    fs::File::open(target)
        .and_then(|d| d.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub(super) fn restore(
    repo: &RepoRuntime,
    generation: &str,
    object: &str,
    target: &Path,
) -> Result<JsonValue, String> {
    if target.exists() {
        return Err("Git restore target must be a new directory".into());
    }
    let (objects, history, documents, _) = read_stores(repo)?;
    let (gen, g) = select(&history, Some(generation))?;
    let oid = bytes::<20>(object)?;
    let mapping = history
        .commit_mappings()
        .into_iter()
        .find(|r| r.generation_index == gen && r.object_id == oid)
        .ok_or("exact Git commit mapping absent")?;
    let docs = documents
        .document_set_files(mapping.document_set_tree_index)?
        .into_iter()
        .map(|(p, m, b)| (p, (m, b)))
        .collect::<BTreeMap<_, _>>();
    let mut leaves = Vec::new();
    collect_files(
        &objects,
        g.source_repository_index,
        &mapping.tree_object_id,
        "",
        &mut leaves,
    )?;
    let actual_docs = leaves
        .iter()
        .filter(|(p, _, _)| p.to_ascii_lowercase().ends_with(".md"))
        .map(|(p, m, b)| (p.clone(), (*m, b.clone())))
        .collect::<BTreeMap<_, _>>();
    if actual_docs != docs {
        return Err("DocSet differs from exact source Git tree".into());
    }
    let parent = target.parent().ok_or("restore target parent absent")?;
    let stage = TempBuilder::new()
        .prefix(".ait-git-restore-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    for (path, mode, body) in &leaves {
        let path = stage.path().join(path);
        fs::create_dir_all(path.parent().ok_or("artifact parent absent")?)
            .map_err(|e| e.to_string())?;
        write_restored_file(&path, *mode, body)?;
    }
    fs::rename(stage.path(), target).map_err(|e| e.to_string())?;
    Ok(
        json!({"contract":INTEROP_CONTRACT,"operation":"restore","status":"completed","generation_id":generation,"git_object_id":object,"file_count":leaves.len(),"markdown_count":docs.len(),"target":target,"mutated":true,"storage":"native_binary"}),
    )
}
fn collect_files(
    store: &LocalGitObjectStore,
    source: u32,
    tree: &[u8; 20],
    prefix: &str,
    output: &mut Vec<(String, u16, Vec<u8>)>,
) -> Result<(), String> {
    let (record, body) = store.get(source, 1, tree)?.ok_or("source Tree absent")?;
    if record.kind != GitObjectKind::Tree {
        return Err("source Tree type mismatch".into());
    }
    for (mode, name, id) in tree_entries(&body)? {
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        validate_git_artifact_path(&path)?;
        if mode == 0o40000 {
            collect_files(store, source, &id, &path, output)?;
        } else {
            let (record, body) = store.get(source, 1, &id)?.ok_or("source Blob absent")?;
            if record.kind != GitObjectKind::Blob {
                return Err("source Blob type mismatch".into());
            }
            output.push((path, mode, body));
        }
    }
    Ok(())
}
#[cfg(unix)]
fn write_restored_file(path: &Path, mode: u16, body: &[u8]) -> Result<(), String> {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{symlink, PermissionsExt},
    };
    if mode == 0o120000 {
        if body.contains(&0) {
            return Err("symlink contains NUL".into());
        }
        symlink(OsStr::from_bytes(body), path).map_err(|e| e.to_string())?;
    } else {
        fs::write(path, body).map_err(|e| e.to_string())?;
        fs::set_permissions(path, fs::Permissions::from_mode(u32::from(mode)))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[cfg(not(unix))]
fn write_restored_file(path: &Path, mode: u16, body: &[u8]) -> Result<(), String> {
    if mode == 0o120000 {
        return Err("exact symlink restore requires supported native platform".into());
    }
    fs::write(path, body).map_err(|e| e.to_string())
}
