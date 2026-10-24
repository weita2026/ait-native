//! Explicit offline conversion. Existing snapshots and physical slots stay put.
use super::*;
use ait_core::binary_db::BinaryDb;
use ait_core::binary_db_generation::{
    activate_binary_db_generation, capture_binary_db_generation_preserving_ordinals,
    seal_binary_db_generation, validate_binary_db_generation, BinaryDbGenerationActivationOptions,
    CaptureBinaryDbGenerationOptions,
};
use ait_core::tag_store::{BinaryDbTagStore, TagRecord};
use sha2::{Digest, Sha256};

const ARCHIVED_TAGS: &str = ".ait/refs/tags.legacy-git-migration";

#[cfg(test)]
mod tests;

struct Inputs {
    mappings: Vec<JsonValue>,
    operations: Vec<JsonValue>,
    tags: Vec<TagRecord>,
    digest: [u8; 32],
}

fn direct_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        _ => {
            return Err(format!(
                "legacy input must be a direct directory: {}",
                root.display()
            ))
        }
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file()
            || entry.path().extension().is_none_or(|ext| ext != "json")
        {
            return Err("legacy migration refuses undeclared or nonregular input files".into());
        }
        paths.push(entry.path());
    }
    paths.sort();
    Ok(paths)
}

fn require_direct_tree(root: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("legacy staging authority must be direct directories and files".into());
    }
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() || !(kind.is_dir() || kind.is_file()) {
            return Err("legacy staging authority contains an alias or nonregular entry".into());
        }
        if kind.is_dir() {
            require_direct_tree(&entry.path())?;
        }
    }
    Ok(())
}

fn inputs(repo: &RepoRuntime) -> Result<Inputs, String> {
    let interop = InteropStore::new(repo);
    let canonical = repo.authoritative_repo_root();
    let tag_root = canonical.join(".ait/refs/tags");
    let archived = canonical.join(ARCHIVED_TAGS);
    if tag_root.exists() && archived.exists() {
        return Err(
            "both active and retained legacy Tag directories exist; refuse ambiguous conversion"
                .into(),
        );
    }
    let mut digest = Sha256::new();
    digest.update(b"git-legacy-conversion/v1\0");
    let mut mappings = Vec::new();
    let mut operations = Vec::new();
    let mut tags = Vec::new();
    for (label, root) in [
        ("mappings", interop.mappings_root()),
        ("operations", interop.root.join("operations")),
        (
            "tags",
            if tag_root.exists() {
                tag_root
            } else {
                archived
            },
        ),
    ] {
        for path in direct_files(&root)? {
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            digest.update(label.as_bytes());
            digest.update([0]);
            digest.update(path.file_name().unwrap().as_encoded_bytes());
            digest.update([0]);
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(&bytes);
            let value = parse_value(
                std::str::from_utf8(&bytes).map_err(|e| e.to_string())?,
                "legacy conversion input",
            )?;
            match label {
                "mappings" => {
                    if json_text(&value, "contract") != Some(MAPPING_CONTRACT)
                        || json_text(&value, "git_object_format") != Some("sha1")
                    {
                        return Err(
                            "legacy migration requires exact supported mapping contract and SHA-1"
                                .into(),
                        );
                    }
                    mappings.push(value);
                }
                "operations" => {
                    if json_text(&value, "contract") != Some(CHECKPOINT_CONTRACT) {
                        return Err("unsupported legacy checkpoint contract".into());
                    }
                    operations.push(value);
                }
                _ => {
                    if value["schema"].as_u64() != Some(1) {
                        return Err("unsupported legacy Tag schema".into());
                    }
                    let tag = new_tag_record(
                        text(&value, "name")?,
                        text(&value, "snapshot_id")?,
                        text(&value, "message")?,
                        text(&value, "created_at")?,
                    )?;
                    if tag.name != text(&value, "name")?
                        || tag.snapshot_id != text(&value, "snapshot_id")?
                        || tag.message != text(&value, "message")?
                        || tag.created_at != text(&value, "created_at")?
                    {
                        return Err("legacy Tag normalization would lose source bytes".into());
                    }
                    let time = chrono::DateTime::parse_from_rfc3339(&tag.created_at)
                        .map_err(|e| e.to_string())?;
                    if time.timestamp() < 0 || time.timestamp_subsec_nanos() != 0 {
                        return Err(
                            "legacy Tag timestamp is not representable as native whole seconds"
                                .into(),
                        );
                    }
                    tags.push(tag);
                }
            }
        }
    }
    tags.sort_by(|a, b| a.name.cmp(&b.name));
    if tags.windows(2).any(|v| v[0].name == v[1].name) {
        return Err("duplicate legacy Tag name".into());
    }
    Ok(Inputs {
        mappings,
        operations,
        tags,
        digest: digest.finalize().into(),
    })
}

fn text<'a>(row: &'a JsonValue, key: &str) -> Result<&'a str, String> {
    json_text(row, key).ok_or_else(|| format!("legacy input missing {key}"))
}
fn time(row: &JsonValue) -> Result<u64, String> {
    let value = chrono::DateTime::parse_from_rfc3339(text(row, "created_at")?)
        .map_err(|e| e.to_string())?;
    if value.timestamp_subsec_nanos() != 0 {
        return Err("legacy mapping timestamp has unsupported fractional precision".into());
    }
    value
        .timestamp()
        .try_into()
        .map_err(|_| "negative legacy mapping time".into())
}
fn identity_for(input: &Inputs, generation: &str, key: &str) -> Result<String, String> {
    let mut identities = BTreeSet::new();
    for operation in &input.operations {
        if json_text(operation, "generation_id") == Some(generation) {
            if let Some(identity) = json_text(&operation["result"], key) {
                identities.insert(identity.to_string());
            }
        }
    }
    if identities.len() != 1 {
        return Err(format!(
            "legacy migration requires one exact retained {key} identity for {generation}"
        ));
    }
    Ok(identities.into_iter().next().unwrap())
}
fn fingerprint(prefix: &str, material: &str) -> Result<[u8; 12], String> {
    bytes(&sha256_prefixed(prefix, material.as_bytes(), 24))
}

fn stage_stores(
    stage: &Path,
) -> Result<
    (
        LocalGitObjectStore,
        GitHistoryStore,
        RawGitDocumentStore,
        LocalContentBinaryDb<1>,
    ),
    String,
> {
    let content = LocalContentBinaryDb::from_db_with_roots(
        LocalBinaryDbFs::new(
            stage.join("local"),
            stage.to_path_buf(),
            AuthorityId::new("legacy-stage-content"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(ait_core::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS),
        stage.to_path_buf(),
        stage.to_path_buf(),
    );
    let interop = if stage.join(GIT_INTEROP_BUNDLE_PATH).exists() {
        GitInteropBundle::open_for_recovery(stage, true)?
            .database(stage, AuthorityId::new("legacy-stage-git"))
    } else {
        LocalBinaryDbFs::new(
            stage.join(".ait/git-interop/v1/binary-v0"),
            stage.to_path_buf(),
            AuthorityId::new("legacy-stage-git"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(GIT_OBJECT_STORE_BIN_PATHS)
        .with_declared_index_paths(GIT_OBJECT_STORE_INDEX_PATHS)
    };
    if interop.authority_root().as_path().exists() {
        // Complete slots remain immutable. Only incomplete fixed tails can be
        // discarded; full source closure still has to pass before activation.
        for (name, width) in [
            ("git_repository.bin", 28),
            ("git_object.bin", 32),
            ("git_generation.bin", 20),
            ("git_identity.bin", 24),
            ("git_commit_mapping.bin", 100),
            ("git_ref_mapping.bin", 80),
            ("git_operation_checkpoint.bin", 44),
        ] {
            let path = interop.authority_root().as_path().join(name);
            if path.exists() {
                let mut data = fs::read(&path).map_err(|e| e.to_string())?;
                if data.len() < 4 || data[..4] != 1u32.to_le_bytes() {
                    return Err(format!("legacy stage malformed {name}"));
                }
                let length = 4 + (data.len() - 4) / width * width;
                if length != data.len() {
                    data.truncate(length);
                    fs::write(path, data).map_err(|e| e.to_string())?;
                }
            }
        }
        LocalGitObjectStore::rebuild_indexes(&interop)?;
        GitHistoryStore::rebuild_indexes(&interop)?;
        RawGitDocumentStore::recover_plan_roots(&interop, &content)?;
        RawGitDocumentStore::rebuild_indexes(&interop, &content)?;
    }
    Ok((
        LocalGitObjectStore::open(interop.clone(), content.clone())?,
        GitHistoryStore::open(interop.clone())?,
        RawGitDocumentStore::open(interop, content.clone())?,
        content,
    ))
}

fn copy_closure(
    objects: &LocalGitObjectStore,
    repository: u32,
    git_dir: &Path,
    roots: Vec<String>,
) -> Result<(), String> {
    let mut reader = GitImportObjectReader::new(git_dir)?;
    let mut pending = roots;
    let mut seen = BTreeSet::new();
    let mut batch = Vec::new();
    let mut batch_bytes = 0;
    while let Some(oid) = pending.pop() {
        if !seen.insert(oid.clone()) {
            continue;
        }
        let object_kind = kind(&reader.info.request(&oid)?.object_type)?;
        let body = reader.content(&oid, kind_name(object_kind))?;
        pending.extend(
            children(object_kind, &body)?
                .into_iter()
                .map(|oid| hex(&oid)),
        );
        batch_bytes += body.len();
        batch.push((object_kind, bytes(&oid)?, body));
        if batch.len() >= 128 || batch_bytes >= 8 * 1024 * 1024 {
            persist_batch(objects, repository, &batch)?;
            batch.clear();
            batch_bytes = 0;
        }
    }
    persist_batch(objects, repository, &batch)
}

fn ordered_commit_rows<'a>(rows: &[&'a JsonValue]) -> Result<Vec<&'a JsonValue>, String> {
    let mut commits = BTreeMap::new();
    let mut parents = BTreeMap::new();
    for row in rows
        .iter()
        .copied()
        .filter(|r| json_text(r, "kind") == Some("commit"))
    {
        let oid = text(row, "git_object_id")?.to_string();
        if commits.insert(oid.clone(), row).is_some() {
            return Err("duplicate legacy commit logical key".into());
        }
        let raw = BASE64_STANDARD
            .decode(text(row, "raw_commit_base64")?)
            .map_err(|e| e.to_string())?;
        let original = raw
            .split(|b| *b == b'\n')
            .take_while(|r| !r.is_empty())
            .filter_map(|r| r.strip_prefix(b"parent "))
            .map(|oid| {
                std::str::from_utf8(oid)
                    .map(str::to_string)
                    .map_err(|_| "legacy parent OID is not ASCII".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(normalized) = row.get("parent_git_object_ids") {
            let normalized = normalized
                .as_array()
                .ok_or("legacy parents must be an array")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or("legacy parent must be text")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if normalized != original {
                return Err("legacy normalized parents differ from raw ordered parents".into());
            }
        }
        parents.insert(oid, original);
    }
    fn visit<'a>(
        oid: &str,
        commits: &BTreeMap<String, &'a JsonValue>,
        parents: &BTreeMap<String, Vec<String>>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
        result: &mut Vec<&'a JsonValue>,
    ) -> Result<(), String> {
        if done.contains(oid) {
            return Ok(());
        }
        if !active.insert(oid.into()) {
            return Err("cyclic legacy commit order".into());
        }
        for parent in &parents[oid] {
            if commits.contains_key(parent) {
                visit(parent, commits, parents, active, done, result)?;
            }
        }
        active.remove(oid);
        done.insert(oid.into());
        result.push(commits[oid]);
        Ok(())
    }
    let mut result = Vec::new();
    let mut active = BTreeSet::new();
    let mut done = BTreeSet::new();
    for oid in commits.keys() {
        visit(oid, &commits, &parents, &mut active, &mut done, &mut result)?;
    }
    Ok(result)
}

fn convert_group(
    repo: &RepoRuntime,
    input: &Inputs,
    rows: &[&JsonValue],
    objects: &LocalGitObjectStore,
    history: &mut GitHistoryStore,
    documents: &mut RawGitDocumentStore,
    content: &LocalContentBinaryDb<1>,
) -> Result<(), String> {
    let first = rows[0];
    let generation_id = text(first, "generation_id")?;
    let export = text(first, "direction")? == "export";
    let source_fingerprint = bytes::<12>(text(first, "source_repository_fingerprint")?)?;
    let interop = InteropStore::new(repo);
    let canonical = repo.authoritative_repo_root();
    let (source_identity, source_kind, git_dir) = if export {
        (
            canonical.to_string_lossy().into_owned(),
            GitRepositoryKind::AitSource,
            interop.generated_repository(),
        )
    } else {
        (
            identity_for(input, generation_id, "source")?,
            GitRepositoryKind::ImportSource,
            interop.retained_repository(text(first, "source_repository_fingerprint")?),
        )
    };
    let expected = if export {
        fingerprint(
            "ASR",
            &format!(
                "ait-repository-fingerprint/v1\n{}\n{}\n",
                repo.repo_name(),
                canonical.to_string_lossy()
            ),
        )?
    } else {
        fingerprint(
            "GSR",
            &format!("git-source-fingerprint/v1\nsha1\n{source_identity}\n"),
        )?
    };
    if expected != source_fingerprint {
        return Err("legacy source fingerprint/full identity differs".into());
    }
    let format = git_repo_bytes(&git_dir, ["rev-parse", "--show-object-format"])?;
    if format != b"sha1\n" {
        return Err("legacy ODB must use SHA-1".into());
    }
    let source = objects.ensure_repository(
        source_kind,
        if export { 0 } else { 1 },
        source_fingerprint,
        &source_identity,
    )?;
    let target = if export {
        let identity = identity_for(input, generation_id, "target")?;
        let fp = fingerprint(
            "GTR",
            &format!("git-target-fingerprint/v1\nsha1\n{identity}\n"),
        )?;
        for row in rows {
            if let Some(value) = json_text(row, "target_repository_fingerprint") {
                if bytes::<12>(value)? != fp {
                    return Err("legacy target full identity differs".into());
                }
            }
        }
        Some(objects.ensure_repository(GitRepositoryKind::ExportTarget, 1, fp, &identity)?)
    } else {
        None
    };
    let generation = history.ensure_generation(GitGenerationRecord {
        export,
        source_repository_index: source,
        target_repository_index_plus1: target.map_or(0, |v| v + 1),
        hash: bytes(generation_id)?,
    })?;
    let roots = rows
        .iter()
        .map(|row| text(row, "git_object_id").map(str::to_string))
        .collect::<Result<Vec<_>, _>>()?;
    copy_closure(objects, source, &git_dir, roots.clone())?;
    if let Some(target) = target {
        copy_closure(objects, target, &git_dir, roots)?;
    }
    let (doc_source, doc_fingerprint, doc_identity) = if export {
        let identity = git_dir
            .canonicalize()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let fp = fingerprint(
            "GSR",
            &format!("git-source-fingerprint/v1\nsha1\n{identity}\n"),
        )?;
        (
            objects.ensure_repository(GitRepositoryKind::ImportSource, 1, fp, &identity)?,
            fp,
            identity,
        )
    } else {
        (source, source_fingerprint, source_identity)
    };
    let mut reader = GitImportObjectReader::new(&git_dir)?;
    for row in ordered_commit_rows(rows)? {
        let oid = text(row, "git_object_id")?;
        let commit = read_commit_data(&git_dir, oid, &mut reader)?;
        if text(row, "git_tree_object_id")? != commit.tree_object_id
            || BASE64_STANDARD
                .decode(text(row, "raw_commit_base64")?)
                .map_err(|e| e.to_string())?
                != commit.raw_bytes
        {
            return Err("legacy raw commit/tree differs from retained ODB".into());
        }
        let view = content
            .snapshots()
            .get_snapshot_view(
                &BinaryDbReadTxn::new(content.db()),
                text(row, "snapshot_id")?,
            )?
            .ok_or("legacy Snapshot absent")?;
        if Some(text(row, "ait_root_tree_id")?) != view.root_tree_id.as_deref() {
            return Err("legacy Snapshot root differs".into());
        }
        let mut docs = Vec::new();
        for file in &commit.files {
            validate_git_artifact_path(&file.path)?;
            let native_mode = mode(&file.mode)?;
            let body = reader.content(&file.object_id, "blob")?;
            if native_mode == 0o120000 && body.contains(&0) {
                return Err("legacy symlink contains NUL".into());
            }
            if file.path.to_ascii_lowercase().ends_with(".md") {
                docs.push((file.path.clone(), native_mode, body));
            }
        }
        let documents_input = docs
            .iter()
            .map(|(path, mode, body)| RawGitDocumentInput {
                path,
                mode: *mode,
                body,
            })
            .collect::<Vec<_>>();
        if let Some(existing) = history
            .commit_mappings()
            .into_iter()
            .find(|r| r.generation_index == generation && r.object_id == bytes::<20>(oid).unwrap())
        {
            let expected_docs = docs
                .iter()
                .map(|(path, mode, body)| (path.clone(), (*mode, body.clone())))
                .collect::<BTreeMap<_, _>>();
            let actual_docs = documents
                .document_set_files(existing.document_set_tree_index)?
                .into_iter()
                .map(|(path, mode, body)| (path, (mode, body)))
                .collect::<BTreeMap<_, _>>();
            if existing.snapshot_index != view.snapshot_index
                || existing.tree_object_id != bytes::<20>(&commit.tree_object_id)?
                || existing.created_at_s != time(row)?
                || history.identity_raw(existing.author_identity_index)?
                    != identity(&commit.raw_bytes, b"author ")?
                || history.identity_raw(existing.committer_identity_index)?
                    != identity(&commit.raw_bytes, b"committer ")?
                || actual_docs != expected_docs
            {
                return Err("legacy replay differs from complete immutable mapping".into());
            }
            continue;
        }
        let document_set = documents.ensure_document_set(
            doc_source,
            doc_fingerprint,
            &doc_identity,
            &documents_input,
        )?;
        history.append_commit(GitCommitMappingRecord {
            mapping_meta: 8 | if !export { 2 } else { 0 } | if commit.signed { 1 } else { 0 },
            generation_index: generation,
            snapshot_index: view.snapshot_index,
            author_identity_index: history
                .append_identity(identity(&commit.raw_bytes, b"author ")?)?,
            committer_identity_index: history
                .append_identity(identity(&commit.raw_bytes, b"committer ")?)?,
            object_id: bytes(oid)?,
            tree_object_id: bytes(&commit.tree_object_id)?,
            lfs_pointer_count: commit
                .lfs_pointer_count
                .try_into()
                .map_err(|_| "legacy LFS count overflow")?,
            created_at_s: time(row)?,
            document_set_tree_index: document_set,
        })?;
        #[cfg(test)]
        if let Some(marker) = std::env::var_os("AIT_GIT_LEGACY_TEST_PAUSE") {
            fs::write(marker, b"legacy durable mapping before seal").map_err(|e| e.to_string())?;
            loop {
                std::thread::park();
            }
        }
    }
    // The former exporter reused imported commits without writing a new
    // export-commit JSON. Preserve that exact alias in the new generation.
    for row in rows
        .iter()
        .filter(|row| json_text(row, "kind") != Some("commit"))
    {
        let Some(id) = json_text(row, "snapshot_id") else {
            continue;
        };
        let snapshot_index = content
            .snapshots()
            .get_snapshot_view(&BinaryDbReadTxn::new(content.db()), id)?
            .ok_or("legacy ref Snapshot absent")?
            .snapshot_index;
        let mut oid = bytes::<20>(text(row, "git_object_id")?)?;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(oid) {
                return Err("legacy tag closure cycle".into());
            }
            let (record, body) = objects
                .get(source, 1, &oid)?
                .ok_or("legacy alias raw object absent")?;
            if record.kind == GitObjectKind::Tag {
                oid = children(record.kind, &body)?[0];
                continue;
            }
            if record.kind != GitObjectKind::Commit {
                break;
            }
            let mappings = history.commit_mappings();
            if !mappings
                .iter()
                .any(|r| r.generation_index == generation && r.object_id == oid)
            {
                let mut candidates = mappings
                    .into_iter()
                    .filter(|r| r.object_id == oid && r.snapshot_index == snapshot_index)
                    .collect::<Vec<_>>();
                candidates.sort_by_key(|r| r.generation_index);
                let mut alias = candidates
                    .pop()
                    .ok_or("legacy ref has no exact retained commit/Snapshot alias")?;
                alias.generation_index = generation;
                history.append_commit(alias)?;
            }
            break;
        }
    }
    let tag_store = BinaryDbTagStore::new(content.clone());
    let mut symbolic = Vec::new();
    for row in rows
        .iter()
        .filter(|row| json_text(row, "kind") != Some("commit"))
    {
        let name = text(row, "git_ref_name")?;
        run_git(["check-ref-format", name], &[], None).or_else(|e| {
            if name == "HEAD" {
                Ok(Vec::new())
            } else {
                Err(e)
            }
        })?;
        if json_text(row, "kind") == Some("symbolic_ref") {
            symbolic.push(*row);
            continue;
        }
        let oid = bytes::<20>(text(row, "git_object_id")?)?;
        let (object, _) = objects
            .get(source, 1, &oid)?
            .ok_or("legacy ref object absent")?;
        let snapshot_index_plus1 = if let Some(id) = json_text(row, "snapshot_id") {
            content
                .snapshots()
                .get_snapshot_view(&BinaryDbReadTxn::new(content.db()), id)?
                .ok_or("legacy ref Snapshot absent")?
                .snapshot_index
                + 1
        } else {
            0
        };
        let target_index_plus1 = if name.starts_with("refs/heads/") {
            let line = json_text(row, "ait_line_name")
                .or_else(|| json_text(row, "ait_name"))
                .ok_or("legacy Line name absent")?;
            let ordinal = ait_core::line_binary_db::binary_line_index_by_name(
                &BinaryDbReadTxn::new(content.db()),
                line,
            )?
            .ok_or("legacy Line absent")?;
            ordinal + 1
        } else if let Some(tag_name) = name.strip_prefix("refs/tags/") {
            let tag = json_text(row, "ait_tag_name")
                .or_else(|| json_text(row, "ait_name"))
                .unwrap_or(tag_name);
            tag_store.tag_index_by_name(tag)?.map_or(0, |v| v + 1)
        } else {
            return Err("unsupported legacy ref kind".into());
        };
        let bound = GitRefMappingRecord {
            meta: u8::from(name.starts_with("refs/tags/"))
                | if snapshot_index_plus1 > 0 { 4 } else { 0 }
                | if target_index_plus1 > 0 { 8 } else { 0 }
                | if json_text(row, "expected_previous_git_object_id").is_some() {
                    16
                } else {
                    0
                },
            object_kind: object.kind,
            generation_index: generation,
            snapshot_index_plus1,
            target_index_plus1,
            symbolic_target_index_plus1: 0,
            object_id: oid,
            expected_previous: json_text(row, "expected_previous_git_object_id")
                .map(bytes)
                .transpose()?
                .unwrap_or([0; 20]),
            created_at_s: time(row)?,
        };
        // Source export selects the frozen intent, then overlays this binding.
        // Match the ordinary native importer instead of publishing only bound refs.
        let mut frozen = bound.clone();
        frozen.meta &= !12;
        frozen.snapshot_index_plus1 = 0;
        frozen.target_index_plus1 = 0;
        history.append_ref(name, frozen)?;
        if bound.meta & 12 != 0 {
            history.append_ref(name, bound)?;
        }
    }
    for row in symbolic {
        let target_name = text(row, "git_symbolic_target")?;
        let target = history
            .refs()?
            .into_iter()
            .find(|(_, name, r)| {
                r.generation_index == generation && name == target_name && r.meta & 12 == 0
            })
            .ok_or("legacy symbolic target absent")?;
        if target.2.object_id != bytes::<20>(text(row, "git_object_id")?)? {
            return Err("legacy symbolic HEAD OID differs".into());
        }
        history.append_ref(
            "HEAD",
            GitRefMappingRecord {
                meta: 2,
                object_kind: GitObjectKind::Commit,
                generation_index: generation,
                snapshot_index_plus1: 0,
                target_index_plus1: 0,
                symbolic_target_index_plus1: target.0 + 1,
                object_id: target.2.object_id,
                expected_previous: [0; 20],
                created_at_s: time(row)?,
            },
        )?;
    }
    let count = history
        .commit_mappings()
        .iter()
        .filter(|r| r.generation_index == generation)
        .count()
        .try_into()
        .map_err(|_| "legacy commit count overflow")?;
    let refs = history
        .refs()?
        .into_iter()
        .filter(|(_, _, r)| r.generation_index == generation)
        .map(|(_, name, _)| name)
        .collect::<BTreeSet<_>>()
        .len()
        .try_into()
        .map_err(|_| "legacy ref count overflow")?;
    checkpoint(
        history,
        generation,
        bytes(&sha256_prefixed("LGM", generation_id.as_bytes(), 16))?,
        input.digest[..12].try_into().unwrap(),
        1,
        count,
        refs,
    )?;
    Ok(())
}

pub(in crate::primitives::git_interop) fn migrate(
    repo: &RepoRuntime,
    requested_stage: &Path,
    apply: bool,
) -> Result<JsonValue, String> {
    let canonical = repo
        .authoritative_repo_root()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let parent = requested_stage
        .parent()
        .ok_or("legacy staging path requires a parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let stage = parent.join(
        requested_stage
            .file_name()
            .ok_or("legacy staging path requires a name")?,
    );
    if stage.starts_with(&canonical)
        || canonical.starts_with(&stage)
        || fs::symlink_metadata(&stage).is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink())
    {
        return Err(
            "legacy staging root must be an isolated direct directory outside the repository"
                .into(),
        );
    }
    let input = inputs(repo)?;
    if input.mappings.is_empty() && input.tags.is_empty() {
        return Ok(json!({"operation":"git.migrate","status":"not_required","mutated":false}));
    }
    let identity = canonical.to_string_lossy();
    let operation = bytes(&sha256_prefixed(
        "LGM",
        format!("legacy-migration/v1\n{identity}\n").as_bytes(),
        16,
    ))?;
    let plan = input.digest[..12].try_into().unwrap();
    if exists(repo) {
        let (_, current, _, _) = stores(repo)?;
        if let Some(done) = current.checkpoint_by_hash(&operation)? {
            if done.plan_hash != plan {
                return Err(
                    "legacy conversion input changed; resume the original immutable inputs".into(),
                );
            }
            if done.status == 1 && !canonical.join(".ait/refs/tags").exists() {
                return Ok(
                    json!({"operation":"git.migrate","status":"no_op","replayed":true,"mutated":false,"runtime_json_writes":0}),
                );
            }
        }
    }
    if !stage.exists() {
        if exists(repo) {
            return Err("native Git authority already exists; use the original migration generation for replay".into());
        }
        capture_binary_db_generation_preserving_ordinals(CaptureBinaryDbGenerationOptions {
            repo_root: canonical.clone(),
            output_root: stage.clone(),
            jobs: 1,
        })?;
    }
    let manifest_bytes = fs::read(stage.join("client-manifest.json")).map_err(|e| e.to_string())?;
    let manifest = parse_value(
        std::str::from_utf8(&manifest_bytes).map_err(|e| e.to_string())?,
        "legacy generation manifest",
    )?;
    if manifest["label"] != repo.repo_name()
        || manifest["source_repo_root"] != canonical.to_string_lossy().as_ref()
        || manifest["content_projection"]["source_retained"] != true
    {
        return Err(
            "legacy staging generation belongs to a different source or remaps physical ordinals"
                .into(),
        );
    }
    require_direct_tree(&stage)?;
    let (objects, mut history, mut documents, content) = stage_stores(&stage)?;
    // The global conversion checkpoint uses a valid existing source kind.
    // Prefer the exact retained import identity. Export-only conversion uses
    // its actual offline generated ODB; Tag-only conversion has an empty,
    // private SHA-1 namespace outside every runtime authority directory.
    let checkpoint_source = if let Some(row) = input
        .mappings
        .iter()
        .find(|r| json_text(r, "direction") == Some("import"))
    {
        let source = identity_for(&input, text(row, "generation_id")?, "source")?;
        let fp = fingerprint(
            "GSR",
            &format!("git-source-fingerprint/v1\nsha1\n{source}\n"),
        )?;
        if fp != bytes::<12>(text(row, "source_repository_fingerprint")?)? {
            return Err("legacy source fingerprint/full identity differs".into());
        }
        source
    } else {
        let path = if input.mappings.is_empty() {
            let path = stage.join("legacy-tag-source.git");
            ensure_bare_repository(&path, "sha1")?;
            path
        } else {
            InteropStore::new(repo).generated_repository()
        };
        path.canonicalize()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned()
    };
    let repository = objects.ensure_repository(
        GitRepositoryKind::ImportSource,
        1,
        fingerprint(
            "GSR",
            &format!("git-source-fingerprint/v1\nsha1\n{checkpoint_source}\n"),
        )?,
        &checkpoint_source,
    )?;
    let generation = history.ensure_generation(GitGenerationRecord {
        export: false,
        source_repository_index: repository,
        target_repository_index_plus1: 0,
        hash: operation,
    })?;
    if let Some(old) = history.checkpoint_by_hash(&operation)? {
        if old.generation_index != generation || old.plan_hash != plan {
            return Err(
                "legacy conversion input changed; resume the original immutable inputs".into(),
            );
        }
    } else {
        checkpoint(&history, generation, operation, plan, 0, 0, 0)?;
    }
    let tags = BinaryDbTagStore::new(content.clone());
    for tag in &input.tags {
        if let Some(old) = tags.tag_by_name(&tag.name)? {
            if old.name != tag.name
                || old.snapshot_id != tag.snapshot_id
                || old.message != tag.message
                || chrono::DateTime::parse_from_rfc3339(&old.created_at)
                    .map_err(|e| e.to_string())?
                    .timestamp()
                    != chrono::DateTime::parse_from_rfc3339(&tag.created_at)
                        .map_err(|e| e.to_string())?
                        .timestamp()
            {
                return Err("legacy Tag conflicts with existing native authority".into());
            }
        } else {
            if tags.tag_index_by_name(&tag.name)?.is_some() {
                return Err("legacy Tag conflicts with a retained native tombstone".into());
            }
            tags.create_tag(tag)?;
        }
    }
    let mut groups = BTreeMap::<(String, String, String), Vec<&JsonValue>>::new();
    for row in &input.mappings {
        let direction = text(row, "direction")?;
        if !["import", "export"].contains(&direction) {
            return Err("unsupported legacy direction".into());
        }
        groups
            .entry((
                text(row, "generation_id")?.into(),
                text(row, "source_repository_fingerprint")?.into(),
                direction.into(),
            ))
            .or_default()
            .push(row);
    }
    let mut ordered = groups.values().collect::<Vec<_>>();
    ordered.sort_by_key(|rows| {
        (
            json_text(rows[0], "direction") == Some("export"),
            rows.iter()
                .filter_map(|r| {
                    json_text(r, "recorded_at_unix_nanos").and_then(|v| v.parse::<u128>().ok())
                })
                .max()
                .unwrap_or(0),
        )
    });
    for rows in ordered {
        convert_group(
            repo,
            &input,
            rows,
            &objects,
            &mut history,
            &mut documents,
            &content,
        )?;
    }
    // Check that grouping did not change the legacy latest-ref selections.
    let mut latest = BTreeMap::<(String, String, String, String), (&JsonValue, u128)>::new();
    for row in input
        .mappings
        .iter()
        .filter(|r| json_text(r, "kind") != Some("commit"))
    {
        let key = (
            text(row, "direction")?.to_string(),
            text(row, "source_repository_fingerprint")?.to_string(),
            json_text(row, "target_repository_fingerprint")
                .unwrap_or("")
                .to_string(),
            text(row, "git_ref_name")?.to_string(),
        );
        let at = text(row, "recorded_at_unix_nanos")?
            .parse::<u128>()
            .map_err(|_| "legacy mapping ordering timestamp invalid")?;
        if latest.get(&key).is_none_or(|(_, old)| at > *old) {
            latest.insert(key, (row, at));
        }
    }
    let generations = history.generations()?;
    let refs = history.refs()?;
    for ((direction, source_fp, target_fp, name), (expected, _)) in latest {
        let mut selected = None;
        for (_, ref_name, row) in &refs {
            if ref_name != &name {
                continue;
            }
            let g = &generations[row.generation_index as usize];
            if g.export != (direction == "export")
                || objects.repository_identity(g.source_repository_index)?.1
                    != bytes::<12>(&source_fp)?
            {
                continue;
            }
            if !target_fp.is_empty()
                && objects
                    .repository_identity(
                        g.target_repository_index_plus1
                            .checked_sub(1)
                            .ok_or("legacy target absent")?,
                    )?
                    .1
                    != bytes::<12>(&target_fp)?
            {
                continue;
            }
            selected = Some((g, row));
        }
        let (g, row) = selected.ok_or("legacy latest ref selection absent")?;
        if g.hash != bytes::<8>(text(expected, "generation_id")?)?
            || row.object_id != bytes::<20>(text(expected, "git_object_id")?)?
        {
            return Err(
                "legacy latest ref order is not representable by complete generation order".into(),
            );
        }
    }
    admission::validate(&objects, &history, &documents, &content)?;
    checkpoint(&history, generation, operation, plan, 1, 0, 0)?;
    drop((objects, history, documents, content, tags));
    {
        // The conversion was fully admitted above. Seal and activate its
        // outer container too, so replay compares one identical representation.
        let bundle = GitInteropBundle::open(&stage, true)?;
        if bundle.needs_migration()? {
            bundle.publish_admitted()?;
        }
    }
    for root in [
        stage.join("local/.locks"),
        stage.join(".ait/git-interop/v1/binary-v0/.locks"),
    ] {
        if root.exists() {
            fs::remove_dir_all(root).map_err(|e| e.to_string())?;
        }
    }
    let fingerprint = seal_binary_db_generation(&stage)?;
    validate_binary_db_generation(&stage, &repo.repo_name())?;
    if inputs(repo)?.digest != input.digest {
        return Err("legacy conversion input changed during staging".into());
    }
    if !apply {
        return Ok(
            json!({"operation":"git.migrate","status":"staged","mutated":false,"staging_root":stage,"content_fingerprint":fingerprint,"legacy_mapping_count":input.mappings.len(),"legacy_tag_count":input.tags.len(),"runtime_json_writes":0}),
        );
    }
    let activated = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
        repo_root: canonical.clone(),
        generation_root: stage.clone(),
        expected_current_authority_fingerprint: None,
    })?;
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("AIT_GIT_LEGACY_ARCHIVE_TEST_PAUSE") {
        fs::write(
            marker,
            b"native content and Git published before Tag archive",
        )
        .map_err(|e| e.to_string())?;
        loop {
            std::thread::park();
        }
    }
    if inputs(repo)?.digest != input.digest {
        return Err(
            "legacy input changed after activation; retained generation remains available".into(),
        );
    }
    let tags = canonical.join(".ait/refs/tags");
    let retained = canonical.join(ARCHIVED_TAGS);
    if tags.exists() {
        if retained.exists() {
            return Err("retained Tag evidence destination appeared".into());
        }
        fs::rename(&tags, &retained).map_err(|e| e.to_string())?;
        fs::File::open(tags.parent().unwrap())
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(
        json!({"operation":"git.migrate","status":"migrated","mutated":true,"staging_root":stage,"content_fingerprint":fingerprint,"legacy_mapping_count":input.mappings.len(),"legacy_tag_count":input.tags.len(),"retained_previous_authority":activated.retained_previous_authority,"retained_tag_evidence":retained,"runtime_json_writes":0}),
    )
}
