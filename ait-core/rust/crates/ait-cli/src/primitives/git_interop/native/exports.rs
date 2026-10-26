//! Prepared raw objects are temporary; only existing fixed/native authorities are published.
use super::*;
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
mod source_plan;
use source_plan::{SourcePlan, StagedObjects};
#[cfg(test)]
thread_local! {
    static EXPORT_COMPRESSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[derive(Clone, Copy, Default)]
struct PublicationControl {
    stop_after_objects: bool,
    #[cfg(test)]
    pause_after_refs: bool,
}
#[derive(Clone)]
struct HeadBaseline {
    binding: Option<String>,
}
#[derive(Clone)]
struct Anchor {
    repository: u32,
    generation: u32,
    record: GitCommitMappingRecord,
}
#[derive(Clone)]
struct BuiltCommit {
    oid: [u8; 20],
    document_set: Option<u32>,
}

fn export_stage(repo: &RepoRuntime) -> Result<TempDir, String> {
    let parent = repo.authoritative_repo_root().join(".ait/git");
    fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let stage = TempBuilder::new()
        .prefix(".export-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    ensure_bare_repository(&stage.path().join("objects.git"), "sha1")?;
    Ok(stage)
}
#[cfg(test)]
fn new_stage() -> Result<TempDir, String> {
    let stage = TempBuilder::new()
        .prefix("ait-native-git-export-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    ensure_bare_repository(&stage.path().join("objects.git"), "sha1")?;
    Ok(stage)
}
fn write_object(stage: &Path, kind: GitObjectKind, body: &[u8]) -> Result<[u8; 20], String> {
    let oid = git_object_id(kind, body);
    let name = hex(&oid);
    let directory = stage.join("objects").join(&name[..2]);
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(&name[2..]);
    let header = format!("{} {}\0", kind.name(), body.len());
    if fs::symlink_metadata(&path).is_ok() {
        let existing = verified_loose_object_bytes(&path, &oid)
            .map_err(|e| format!("staged Git object differs at the same OID: {e}"))?;
        if existing.len() != header.len() + body.len()
            || !existing.starts_with(header.as_bytes())
            || existing[header.len()..] != *body
        {
            return Err("staged Git object differs at the same OID".into());
        }
        return Ok(oid);
    }
    // This ODB is disposable staging, never native authority. Git fsck checks
    // every staged object before any ref or checkpoint publication.
    let mut temporary = tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
    {
        #[cfg(test)]
        EXPORT_COMPRESSIONS.with(|n| n.set(n.get() + 1));
        let mut compressed = ZlibEncoder::new(temporary.as_file_mut(), Compression::default());
        compressed
            .write_all(header.as_bytes())
            .map_err(|e| e.to_string())?;
        compressed.write_all(body).map_err(|e| e.to_string())?;
        compressed.finish().map_err(|e| e.to_string())?;
    }
    match temporary.persist_noclobber(&path) {
        Ok(_) => {}
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let file = fs::File::open(&path).map_err(|e| e.to_string())?;
            let encoded_bytes = file.metadata().map_err(|e| e.to_string())?.len();
            let mut reader = ZlibDecoder::new(file);
            let mut existing = Vec::new();
            reader
                .read_to_end(&mut existing)
                .map_err(|e| e.to_string())?;
            if reader.total_in() != encoded_bytes
                || existing.len() != header.len() + body.len()
                || !existing.starts_with(header.as_bytes())
                || existing[header.len()..] != *body
            {
                return Err("staged Git object differs at the same OID".into());
            }
        }
        Err(error) => return Err(error.error.to_string()),
    }
    Ok(oid)
}
pub(super) fn verified_loose_object_bytes(path: &Path, oid: &[u8; 20]) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("staged Git object is not a direct file".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let encoded_bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let mut reader = ZlibDecoder::new(file);
    let mut raw = Vec::new();
    reader.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let header_end = raw
        .iter()
        .position(|b| *b == 0)
        .ok_or("invalid loose Git header")?;
    let header =
        std::str::from_utf8(&raw[..header_end]).map_err(|_| "non-ASCII loose Git header")?;
    let (kind_text, _) = header.split_once(' ').ok_or("invalid loose Git header")?;
    let object_kind = kind(kind_text)?;
    let body = &raw[header_end + 1..];
    if reader.total_in() != encoded_bytes
        || header != format!("{} {}", object_kind.name(), body.len())
        || git_object_id(object_kind, body) != *oid
    {
        return Err("loose Git object header/OID differs".into());
    }
    Ok(raw)
}
fn stage_closure(
    stage: &Path,
    roots: &[[u8; 20]],
) -> Result<Vec<(GitObjectKind, [u8; 20])>, String> {
    let mut todo = roots.to_vec();
    let mut seen = BTreeSet::new();
    let mut closure = Vec::new();
    let mut reader = GitImportObjectReader::new(stage)?;
    while let Some(oid) = todo.pop() {
        if !seen.insert(oid) {
            continue;
        }
        let oid_text = hex(&oid);
        let object_kind = kind(&reader.info.request(&oid_text)?.object_type)?;
        let body = reader.content(&oid_text, object_kind.name())?;
        todo.extend(children(object_kind, &body)?);
        closure.push((object_kind, oid));
    }
    Ok(closure)
}
pub(super) fn source(
    repo: &RepoRuntime,
    target: &str,
    generation: Option<&str>,
    object: Option<&str>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let target = inspect_target(target)?;
    if target.object_format != "sha1" {
        return Err("Git export requires SHA-1 target".into());
    }
    let bundle = stores_mode(repo, !dry_run)?;
    let (objects, history, _, content) = &bundle;
    let (generation_index, generation_record) = select(history, generation)?;
    let all_refs = history.refs()?;
    let mut refs = all_refs
        .iter()
        .filter(|(_, _, r)| r.generation_index == generation_index && r.meta & 12 == 0)
        .cloned()
        .collect::<Vec<_>>();
    for (_, name, row) in &mut refs {
        if let Some((_, _, bound)) = all_refs.iter().rev().find(|(_, n, r)| {
            n == name && r.generation_index == generation_index && r.meta & 4 != 0
        }) {
            *row = bound.clone();
        }
    }
    if let Some(oid) = object {
        let oid = bytes::<20>(oid)?;
        if !history
            .commit_mappings()
            .iter()
            .any(|r| r.generation_index == generation_index && r.object_id == oid)
        {
            return Err("selected Git commit has no admitted mapping".into());
        }
        refs = vec![(
            0,
            "HEAD".into(),
            GitRefMappingRecord {
                meta: 32,
                object_kind: GitObjectKind::Commit,
                generation_index,
                snapshot_index_plus1: 0,
                target_index_plus1: 0,
                symbolic_target_index_plus1: 0,
                object_id: oid,
                expected_previous: [0; 20],
                created_at_s: 0,
            },
        )];
    }
    if refs.is_empty() {
        return Err("selected generation has no frozen refs".into());
    }
    let stage = export_stage(repo)?;
    let stage_dir = stage.path().join("objects.git");
    let roots = refs.iter().map(|(_, _, r)| r.object_id).collect::<Vec<_>>();
    let writer = StagedObjects::new(&stage_dir);
    let mut source = SourcePlan::new(objects, content, &writer)?;
    for root in &roots {
        source.root(
            generation_record.source_repository_index,
            *root,
            Some(generation_index),
        );
    }
    let mut prepared = BTreeMap::new();
    let source_stats = source.finish(history, &mut prepared, &roots)?;
    drop(source);
    let staged_compressions = writer.compressions.get();
    let closure = stage_closure(&stage_dir, &roots)?;
    let commits = prepared.into_values().collect::<Vec<_>>();
    let exact = closure
        .iter()
        .filter(|(k, _)| *k == GitObjectKind::Commit)
        .count();
    let input = PreparedExport {
        target,
        stage,
        refs,
        closure,
        commits,
        intent: format!(
            "source {} {}",
            hex(&generation_record.hash),
            object.unwrap_or("all")
        ),
        source_generation: Some(format!(
            "GIT-IMP-{}",
            hex(&generation_record.hash).to_ascii_uppercase()
        )),
        exact,
        native: 0,
        source_stats,
        staged_compressions,
    };
    publish(
        repo,
        input,
        Some(bundle),
        dry_run,
        PublicationControl::default(),
    )
}

type ExportMemoKey = (String, Option<(u32, [u8; 20])>);
struct CurrentBuilder<'a> {
    content: &'a LocalContentBinaryDb<1>,
    bundle: Option<&'a Bundle>,
    stage: &'a Path,
    writer: &'a StagedObjects<'a>,
    source: Option<SourcePlan<'a>>,
    mappings: BTreeMap<u32, Vec<Anchor>>,
    anchors: BTreeMap<u32, Vec<Anchor>>,
    prepared: BTreeMap<[u8; 20], PreparedCommit>,
    memo: BTreeMap<ExportMemoKey, BuiltCommit>,
    visiting: BTreeSet<String>,
}
impl CurrentBuilder<'_> {
    fn exact_anchor(&self, index: u32) -> Result<Option<Anchor>, String> {
        let candidates = self
            .anchors
            .get(&index)
            .or_else(|| self.mappings.get(&index));
        let Some(candidates) = candidates else {
            return Ok(None);
        };
        let unique = candidates
            .iter()
            .map(|a| a.record.object_id)
            .collect::<BTreeSet<_>>();
        if unique.len() > 1 {
            return Err(format!("source anchor ambiguity at Snapshot ordinal {index}; select an exact imported generation and Git commit"));
        }
        Ok(candidates.first().cloned())
    }
    fn load_anchor(&mut self, anchor: &Anchor) -> Result<BuiltCommit, String> {
        self.source
            .as_mut()
            .ok_or("source anchor without native authority")?
            .root(
                anchor.repository,
                anchor.record.object_id,
                Some(anchor.generation),
            );
        Ok(BuiltCommit {
            oid: anchor.record.object_id,
            document_set: Some(anchor.record.document_set_tree_index),
        })
    }
    fn build(&mut self, id: &str, preferred: Option<Anchor>) -> Result<BuiltCommit, String> {
        let key = (
            id.to_string(),
            preferred
                .as_ref()
                .map(|a| (a.generation, a.record.object_id)),
        );
        if let Some(built) = self.memo.get(&key) {
            return Ok(built.clone());
        }
        if !self.visiting.insert(id.into()) {
            return Err("native Snapshot DAG cycle".into());
        }
        let view = self
            .content
            .snapshots()
            .get_snapshot_view(&BinaryDbReadTxn::new(self.content.db()), id)?
            .ok_or_else(|| format!("Unknown AIT Snapshot: {id}"))?;
        let exact = match &preferred {
            Some(a) if a.record.snapshot_index == view.snapshot_index => Some(a.clone()),
            Some(_) => None,
            None => self.exact_anchor(view.snapshot_index)?,
        };
        let result = if let Some(anchor) = exact {
            self.load_anchor(&anchor)?
        } else {
            let mut parents = Vec::new();
            for (at, parent) in view.parent_snapshot_ids.iter().enumerate() {
                parents.push(self.build(parent, if at == 0 { preferred.clone() } else { None })?);
            }
            let docset = parents.first().and_then(|p| p.document_set);
            let index_temp = TempBuilder::new()
                .prefix("ait-native-export-index-")
                .tempdir_in(self.stage.parent().ok_or("export stage parent absent")?)
                .map_err(|e| e.to_string())?;
            let env = vec![(
                OsString::from("GIT_INDEX_FILE"),
                index_temp.path().join("index").into_os_string(),
            )];
            git_repo_bytes_os(
                self.stage,
                vec!["read-tree".into(), "--empty".into()],
                &env,
                None,
            )?;
            let mut paths = BTreeSet::new();
            for row in self.content.snapshot_tree_file_rows(Some(id))? {
                validate_git_artifact_path(&row.path)?;
                if is_markdown_path(&row.path) {
                    return Err("code Snapshot unexpectedly contains Markdown".into());
                }
                let body = self.content.read_blob_bytes(&row.blob_id)?;
                let git_mode = snapshot_mode_to_git_mode(&row.mode)?;
                self.add_file(&env, &row.path, git_mode, &body)?;
                paths.insert(row.path);
            }
            if let Some(docset) = docset {
                let (_, _, documents, _) = self.bundle.ok_or("DocSet without native authority")?;
                for (path, mode, body) in documents.document_set_files(docset)? {
                    if !paths.insert(path.clone()) {
                        return Err("source Markdown collides with code Snapshot".into());
                    }
                    self.add_file(
                        &env,
                        &path,
                        match mode {
                            0o644 => "100644",
                            0o755 => "100755",
                            0o120000 => "120000",
                            _ => return Err("unsupported source Markdown mode".into()),
                        },
                        &body,
                    )?;
                }
            }
            let tree = bytes::<20>(
                git_repo_text_with_env(self.stage, vec!["write-tree".into()], &env)?.trim(),
            )?;
            let identity = format!("AIT Snapshot <ait@local> {} +0000", view.created_at_s);
            let mut raw = format!("tree {}\n", hex(&tree));
            for parent in &parents {
                raw.push_str(&format!("parent {}\n", hex(&parent.oid)));
            }
            raw.push_str(&format!("author {identity}\ncommitter {identity}\n\n"));
            raw.push_str(
                view.payload
                    .message
                    .as_deref()
                    .unwrap_or(&format!("AIT Snapshot {id}")),
            );
            if !raw.ends_with('\n') {
                raw.push('\n');
            }
            let oid = self.writer.write(GitObjectKind::Commit, raw.as_bytes())?;
            self.prepared.insert(
                oid,
                PreparedCommit {
                    record: GitCommitMappingRecord {
                        mapping_meta: 12,
                        generation_index: 0,
                        snapshot_index: view.snapshot_index,
                        author_identity_index: 0,
                        committer_identity_index: 0,
                        object_id: oid,
                        tree_object_id: tree,
                        lfs_pointer_count: 0,
                        created_at_s: 0,
                        document_set_tree_index: docset.unwrap_or(0),
                    },
                    document_set: docset,
                    native: true,
                },
            );
            BuiltCommit {
                oid,
                document_set: docset,
            }
        };
        self.visiting.remove(id);
        self.memo.insert(key, result.clone());
        Ok(result)
    }
    fn add_file(
        &self,
        env: &[(OsString, OsString)],
        path: &str,
        mode: &str,
        body: &[u8],
    ) -> Result<(), String> {
        let oid = self.writer.write(GitObjectKind::Blob, body)?;
        git_repo_bytes_os(
            self.stage,
            vec![
                "update-index".into(),
                "--add".into(),
                "--cacheinfo".into(),
                mode.into(),
                hex(&oid).into(),
                path.into(),
            ],
            env,
            None,
        )?;
        Ok(())
    }
}
fn git_repo_text_with_env(
    stage: &Path,
    args: Vec<OsString>,
    env: &[(OsString, OsString)],
) -> Result<String, String> {
    String::from_utf8(git_repo_bytes_os(stage, args, env, None)?)
        .map_err(|_| "Git OID ASCII".into())
}
fn is_markdown_path(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|p| p.to_ascii_lowercase().ends_with(".md"))
}

fn symbolic_head(git_dir: &Path) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", git_dir.display()))
        .args(["symbolic-ref", "--quiet", "HEAD"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8(output.stdout)
                .map_err(|_| "HEAD target UTF-8")?
                .trim()
                .to_string(),
        ));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(format!(
        "cannot read symbolic HEAD: {}",
        String::from_utf8_lossy(&output.stderr)
    ))
}
fn desired_head(
    refs: &[(u32, String, GitRefMappingRecord)],
    row: &GitRefMappingRecord,
) -> Result<Option<String>, String> {
    if row.meta & 3 != 2 {
        return Ok(None);
    }
    Ok(Some(
        refs.iter()
            .find(|(i, _, _)| i.checked_add(1) == Some(row.symbolic_target_index_plus1))
            .ok_or("symbolic HEAD export target absent")?
            .1
            .clone(),
    ))
}
fn reverify_target(
    git_dir: &Path,
    refs: &[(u32, String, GitRefMappingRecord)],
) -> Result<(), String> {
    for (_, name, row) in refs {
        if git_ref_object_id(git_dir, name)?.as_deref() != Some(hex(&row.object_id).as_str()) {
            return Err(format!(
                "Git export completed target changed at {name}; explicit reconciliation required"
            ));
        }
        if name == "HEAD" && symbolic_head(git_dir)? != desired_head(refs, row)? {
            return Err("Git export completed target HEAD binding changed; explicit reconciliation required".into());
        }
    }
    git_repo_bytes(git_dir, ["fsck", "--strict", "--full"])?;
    Ok(())
}
fn publish_ref_transaction(
    git_dir: &Path,
    refs: &[(u32, String, GitRefMappingRecord)],
    baseline: Option<&HeadBaseline>,
    control: PublicationControl,
) -> Result<(), String> {
    #[cfg(not(test))]
    let _ = control;
    let mut script = String::from("start\n");
    let (_, _, head) = refs
        .iter()
        .find(|(_, name, _)| name == "HEAD")
        .ok_or("export HEAD intent absent")?;
    let wanted_binding = desired_head(refs, head)?;
    let old_binding = symbolic_head(git_dir)?;
    let mut head_bytes =
        fs::read(git_dir.join("HEAD")).map_err(|e| format!("read Git HEAD: {e}"))?;
    let actual_head = git_ref_object_id(git_dir, "HEAD")?.unwrap_or_else(|| ZERO_SHA1.into());
    let expected_text = old_binding
        .as_ref()
        .map(|name| format!("ref: {name}"))
        .unwrap_or_else(|| actual_head.clone());
    if std::str::from_utf8(&head_bytes)
        .map_err(|_| "Git HEAD is not ASCII")?
        .trim()
        != expected_text
    {
        return Err("Git HEAD changed during publication preflight".into());
    }
    let mut all_refs_published = true;
    let mut observed = BTreeMap::new();
    for (_, name, row) in refs.iter().filter(|(_, name, _)| name != "HEAD") {
        let actual = git_ref_object_id(git_dir, name)?.unwrap_or_else(|| ZERO_SHA1.into());
        all_refs_published &= actual == hex(&row.object_id);
        observed.insert(name.as_str(), actual);
    }
    let referent_published = old_binding.as_ref().is_some_and(|binding| {
        refs.iter()
            .any(|(_, name, row)| name == binding && actual_head == hex(&row.object_id))
    });
    let known_binding = old_binding == wanted_binding
        || baseline.is_some_and(|old| old.binding == old_binding)
        || actual_head == ZERO_SHA1;
    if !known_binding {
        return Err(
            "Git export refuses changed symbolic HEAD binding; explicit reconciliation required"
                .into(),
        );
    }
    if actual_head != hex(&head.object_id)
        && actual_head != hex(&head.expected_previous)
        && !(all_refs_published && referent_published)
    {
        return Err(
            "Git export refuses changed HEAD OID: frozen compare-and-swap evidence differs".into(),
        );
    }
    for (_, name, row) in refs {
        if name == "HEAD" {
            continue;
        }
        let actual = observed
            .get(name.as_str())
            .ok_or("observed export ref absent")?;
        let wanted = hex(&row.object_id);
        if actual != &wanted && actual != &hex(&row.expected_previous) {
            return Err(format!(
                "Git export refuses to overwrite {name}: frozen compare-and-swap evidence differs"
            ));
        }
        script.push_str(&format!("update {name} {wanted} {actual}\n"));
    }
    if let Some(binding) = &old_binding {
        if !observed.contains_key(binding.as_str()) {
            script.push_str(&format!("verify {binding} {actual_head}\n"));
        }
    }
    // Normalize an unborn target before branch publication. Its frozen zero
    // previous OID proves that no prior public commit is being replaced.
    // Resume then accepts the exact desired binding, never an arbitrary alias.
    if actual_head == ZERO_SHA1 && baseline.is_none() {
        if let Some(binding) = &wanted_binding {
            let normalized = format!("ref: {binding}\n").into_bytes();
            cas_head_bytes(git_dir, &head_bytes, &normalized)?;
            head_bytes = normalized;
        }
    }
    script.push_str("prepare\ncommit\n");
    git_repo_bytes_os(
        git_dir,
        vec![
            "-c".into(),
            "core.fsync=reference".into(),
            "update-ref".into(),
            "--stdin".into(),
        ],
        &[],
        Some(script.as_bytes()),
    )?;
    #[cfg(test)]
    if control.pause_after_refs {
        let marker = std::env::var_os("AIT_GIT_PUBLICATION_TEST_MARKER").expect("test marker path");
        fs::write(marker, b"refs-durable").expect("write test marker");
        loop {
            std::thread::park();
        }
    }
    // The files ref backend implicitly logs HEAD when its referent changes.
    // An explicit HEAD update in that transaction is rejected by Git. Publish
    // all ordinary refs together, then CAS exact HEAD bytes separately.
    // The native checkpoint remains pending until both phases and fsck pass;
    // replay accepts already-published refs and retries the frozen HEAD intent.
    let wanted = wanted_binding
        .map(|name| format!("ref: {name}\n"))
        .unwrap_or_else(|| format!("{}\n", hex(&head.object_id)));
    cas_head_bytes(git_dir, &head_bytes, wanted.as_bytes())?;
    reverify_target(git_dir, refs)
}
fn cas_head_bytes(git_dir: &Path, expected: &[u8], wanted: &[u8]) -> Result<(), String> {
    struct HeadLock {
        path: PathBuf,
        owned: bool,
    }
    impl Drop for HeadLock {
        fn drop(&mut self) {
            if self.owned {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
    let path = git_dir.join("HEAD");
    let metadata = fs::symlink_metadata(&path).map_err(|e| format!("Git HEAD metadata: {e}"))?;
    if !metadata.file_type().is_file() {
        return Err("Git HEAD must be a regular file for exact CAS".into());
    }
    let mut guard = HeadLock {
        path: git_dir.join("HEAD.lock"),
        owned: false,
    };
    let mut lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&guard.path)
        .map_err(|e| format!("Git HEAD CAS lock: {e}"))?;
    guard.owned = true;
    if fs::read(&path).map_err(|e| e.to_string())? != expected {
        return Err(
            "Git HEAD bytes changed before CAS; rerun only after explicit reconciliation".into(),
        );
    }
    lock.set_permissions(metadata.permissions())
        .map_err(|e| e.to_string())?;
    lock.write_all(wanted).map_err(|e| e.to_string())?;
    lock.sync_all()
        .map_err(|e| format!("Git HEAD dependency fsync: {e}"))?;
    fs::rename(&guard.path, &path).map_err(|e| format!("Git HEAD CAS publish: {e}"))?;
    guard.owned = false;
    fs::File::open(git_dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("Git HEAD directory fsync: {e}"))?;
    Ok(())
}

pub(super) fn current(
    repo: &RepoRuntime,
    target: &str,
    all: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    current_with_control(repo, target, all, dry_run, false)
}
pub(super) fn current_with_control(
    repo: &RepoRuntime,
    target: &str,
    all: bool,
    dry_run: bool,
    stop_after_objects: bool,
) -> Result<JsonValue, String> {
    current_with_publication_control(
        repo,
        target,
        all,
        dry_run,
        PublicationControl {
            stop_after_objects,
            #[cfg(test)]
            pause_after_refs: false,
        },
    )
}
fn current_with_publication_control(
    repo: &RepoRuntime,
    target: &str,
    all: bool,
    dry_run: bool,
    control: PublicationControl,
) -> Result<JsonValue, String> {
    let target = inspect_target(target)?;
    if target.object_format != "sha1" {
        return Err("Git export requires SHA-1 target".into());
    }
    let selected = selected_export_refs(repo, all)?;
    if selected.is_empty() {
        return Err("Git export selected no AIT lines or tags with Snapshot heads.".into());
    }
    for r in &selected {
        run_git(["check-ref-format", &r.git_ref_name], &[], None)?;
    }
    let bundle = if exists(repo) {
        Some(stores_mode(repo, !dry_run)?)
    } else {
        None
    };
    let content = repo.binary_db_stores::<1>().content();
    let stage = export_stage(repo)?;
    let stage_dir = stage.path().join("objects.git");
    let writer = StagedObjects::new(&stage_dir);
    let mut source = bundle
        .as_ref()
        .map(|(objects, _, _, content)| SourcePlan::new(objects, content, &writer))
        .transpose()?;
    let mut refs = Vec::new();
    let mut wanted_anchors = BTreeMap::new();
    let mut mappings: BTreeMap<u32, Vec<Anchor>> = BTreeMap::new();
    let mut anchors: BTreeMap<u32, Vec<Anchor>> = BTreeMap::new();
    let mut imported_head = None;
    let mut raw_only = Vec::new();
    let selected_ordinals = {
        let read = BinaryDbReadTxn::new(content.db());
        let tags = if selected.iter().any(|r| r.ait_kind == "tag") {
            crate::tag_surface::tag_store(repo)?.tag_ordinals_by_name()?
        } else {
            BTreeMap::new()
        };
        let mut ordinals = BTreeMap::new();
        for r in &selected {
            let ordinal = if r.ait_kind == "line" {
                ait_core::line_binary_db::binary_line_index_by_name(&read, &r.ait_name)?
            } else {
                tags.get(&r.ait_name).copied()
            }
            .ok_or("selected native ref ordinal absent")?;
            ordinals.insert(r.git_ref_name.clone(), ordinal);
        }
        ordinals
    };
    if let Some((_, history, _, _)) = &bundle {
        let generations = history.generations()?;
        let records = history.commit_mappings();
        let records_by_key = records
            .iter()
            .map(|r| ((r.generation_index, r.object_id), r))
            .collect::<BTreeMap<_, _>>();
        let source_refs = history.refs()?;
        let mut records_by_generation: BTreeMap<u32, Vec<&GitCommitMappingRecord>> =
            BTreeMap::new();
        for record in &records {
            records_by_generation
                .entry(record.generation_index)
                .or_default()
                .push(record);
        }
        let mut refs_by_generation = BTreeMap::<u32, Vec<_>>::new();
        for row in &source_refs {
            refs_by_generation
                .entry(row.2.generation_index)
                .or_default()
                .push(row);
        }
        let bound_names = source_refs
            .iter()
            .filter(|(_, _, r)| r.meta & 4 != 0)
            .map(|(_, n, r)| (r.generation_index, n.as_str()))
            .collect::<BTreeSet<_>>();
        let names_by_ordinal = source_refs
            .iter()
            .map(|(at, n, _)| (*at, n))
            .collect::<BTreeMap<_, _>>();
        let mut selected_targets: BTreeMap<(bool, u32), Vec<&str>> = BTreeMap::new();
        for r in &selected {
            selected_targets
                .entry((r.ait_kind == "tag", selected_ordinals[&r.git_ref_name] + 1))
                .or_default()
                .push(&r.git_ref_name);
        }
        let mut latest = BTreeMap::new();
        let complete = history
            .checkpoints()?
            .into_iter()
            .filter(|row| row.status == 1)
            .map(|row| row.generation_index)
            .collect::<BTreeSet<_>>();
        for (i, g) in generations.iter().enumerate() {
            if !g.export && complete.contains(&(i as u32)) {
                latest.insert(g.source_repository_index, i as u32);
            }
        }
        for (i, g) in generations.iter().enumerate().filter(|(i, g)| {
            !g.export && latest.get(&g.source_repository_index) == Some(&(*i as u32))
        }) {
            for r in records_by_generation
                .get(&(i as u32))
                .into_iter()
                .flatten()
                .copied()
            {
                mappings.entry(r.snapshot_index).or_default().push(Anchor {
                    repository: g.source_repository_index,
                    generation: i as u32,
                    record: r.clone(),
                });
            }
            for (_, name, row) in refs_by_generation
                .get(&(i as u32))
                .into_iter()
                .flatten()
                .copied()
            {
                if row.meta & 4 != 0 {
                    let commit = if row.object_kind == GitObjectKind::Commit {
                        row.object_id
                    } else {
                        source
                            .as_mut()
                            .unwrap()
                            .peel_commit(g.source_repository_index, row.object_id)?
                    };
                    let record = records_by_key
                        .get(&(i as u32, commit))
                        .copied()
                        .ok_or("bound source ref lacks commit mapping")?
                        .clone();
                    let anchor = Anchor {
                        repository: g.source_repository_index,
                        generation: i as u32,
                        record,
                    };
                    anchors
                        .entry(anchor.record.snapshot_index)
                        .or_default()
                        .push(anchor.clone());
                    if let Some(names) = selected_targets
                        .get(&(name.starts_with("refs/tags/"), row.target_index_plus1))
                    {
                        for name in names {
                            wanted_anchors
                                .insert((*name).to_string(), (anchor.clone(), row.clone()));
                        }
                    }
                } else if all
                    && name.starts_with("refs/tags/")
                    && !bound_names.contains(&(i as u32, name.as_str()))
                {
                    raw_only.push((g.source_repository_index, name.clone(), row.clone()));
                }
                if name == "HEAD" && row.meta & 3 == 2 {
                    imported_head = row
                        .symbolic_target_index_plus1
                        .checked_sub(1)
                        .and_then(|at| names_by_ordinal.get(&at))
                        .map(|name| (*name).clone());
                }
            }
        }
    }
    let mut builder = CurrentBuilder {
        content: &content,
        bundle: bundle.as_ref(),
        stage: &stage_dir,
        writer: &writer,
        source,
        mappings,
        anchors,
        prepared: BTreeMap::new(),
        memo: BTreeMap::new(),
        visiting: BTreeSet::new(),
    };
    let mut built_by_snapshot: BTreeMap<String, Option<BuiltCommit>> = BTreeMap::new();
    for r in &selected {
        let anchor = wanted_anchors.get(&r.git_ref_name);
        let built = if r.ait_kind == "tag" && anchor.is_none() {
            if let Some(built) = built_by_snapshot
                .get(&r.snapshot_id)
                .and_then(Option::as_ref)
            {
                built.clone()
            } else {
                builder.build(&r.snapshot_id, None)?
            }
        } else {
            builder.build(&r.snapshot_id, anchor.map(|(a, _)| a.clone()))?
        };
        built_by_snapshot
            .entry(r.snapshot_id.clone())
            .and_modify(|old| {
                if old.as_ref().is_some_and(|old| old.oid != built.oid) {
                    *old = None;
                }
            })
            .or_insert_with(|| Some(built.clone()));
        let view = content
            .snapshots()
            .get_snapshot_view(&BinaryDbReadTxn::new(content.db()), &r.snapshot_id)?
            .ok_or("selected Snapshot absent")?;
        let target_index = selected_ordinals[&r.git_ref_name];
        let (oid, object_kind) = if r.ait_kind == "tag" {
            let reusable = if let Some((a, row)) = anchor {
                a.record.object_id == built.oid
                    && builder.source.as_mut().unwrap().tag_message_matches(
                        a.repository,
                        row.object_id,
                        &r.ait_name,
                        r.message.as_deref(),
                    )?
                    && r.created_at.as_deref()
                        == Some(
                            epoch_text(imported_tag_created_at(
                                &bundle.as_ref().unwrap().0,
                                &bundle.as_ref().unwrap().1,
                                row,
                            )?)?
                            .as_str(),
                        )
            } else {
                false
            };
            if let Some((a, row)) = anchor.filter(|_| reusable) {
                builder
                    .source
                    .as_mut()
                    .unwrap()
                    .root(a.repository, row.object_id, None);
                (row.object_id, row.object_kind)
            } else {
                let date = git_date(r.created_at.as_deref().ok_or("native Tag time absent")?)?;
                let raw=format!("object {}\ntype commit\ntag {}\ntagger AIT Snapshot <ait@local> {date}\n\n{}\n",hex(&built.oid),r.ait_name,r.message.as_deref().unwrap_or(""));
                (
                    writer.write(GitObjectKind::Tag, raw.as_bytes())?,
                    GitObjectKind::Tag,
                )
            }
        } else {
            (built.oid, GitObjectKind::Commit)
        };
        let ordinal = refs.len().try_into().map_err(|_| "ref ordinal overflow")?;
        refs.push((
            ordinal,
            r.git_ref_name.clone(),
            GitRefMappingRecord {
                meta: 12 | u8::from(r.ait_kind == "tag"),
                object_kind,
                generation_index: 0,
                snapshot_index_plus1: view.snapshot_index + 1,
                target_index_plus1: target_index + 1,
                symbolic_target_index_plus1: 0,
                object_id: oid,
                expected_previous: [0; 20],
                created_at_s: 0,
            },
        ));
    }
    for (repository, name, row) in raw_only {
        if refs.iter().any(|(_, n, _)| n == &name) {
            continue;
        }
        builder
            .source
            .as_mut()
            .unwrap()
            .root(repository, row.object_id, None);
        refs.push((
            refs.len().try_into().map_err(|_| "ref ordinal overflow")?,
            name,
            row,
        ));
    }
    let current = repo.current_line_name()?;
    let head = imported_head
        .filter(|name| refs.iter().any(|(_, n, _)| n == name))
        .or_else(|| {
            refs.iter()
                .find(|(_, n, _)| n == &format!("refs/heads/{current}"))
                .map(|(_, n, _)| n.clone())
        })
        .or_else(|| {
            refs.iter()
                .find(|(_, n, _)| n.starts_with("refs/heads/"))
                .map(|(_, n, _)| n.clone())
        })
        .ok_or("export has no branch for HEAD")?;
    let (head_target, _, head_row) = refs.iter().find(|(_, n, _)| n == &head).unwrap();
    let mut head_row = head_row.clone();
    head_row.meta = 2;
    head_row.snapshot_index_plus1 = 0;
    head_row.target_index_plus1 = 0;
    head_row.symbolic_target_index_plus1 = head_target + 1;
    refs.push((
        refs.len().try_into().map_err(|_| "ref ordinal overflow")?,
        "HEAD".into(),
        head_row,
    ));
    let source_stats = if let Some(mut source) = builder.source.take() {
        source.finish(
            &bundle.as_ref().unwrap().1,
            &mut builder.prepared,
            &refs.iter().map(|(_, _, r)| r.object_id).collect::<Vec<_>>(),
        )?
    } else {
        source_plan::TraversalStats::default()
    };
    let staged_compressions = writer.compressions.get();
    let closure = stage_closure(
        &stage_dir,
        &refs.iter().map(|(_, _, r)| r.object_id).collect::<Vec<_>>(),
    )?;
    let reachable = closure.iter().map(|(_, oid)| *oid).collect::<BTreeSet<_>>();
    let commits = builder
        .prepared
        .into_iter()
        .filter(|(oid, _)| reachable.contains(oid))
        .map(|(_, c)| c)
        .collect::<Vec<_>>();
    let native = commits.iter().filter(|c| c.native).count();
    let exact = closure
        .iter()
        .filter(|(k, _)| *k == GitObjectKind::Commit)
        .count()
        - native;
    let input = PreparedExport {
        target,
        stage,
        refs,
        closure,
        commits,
        intent: format!(
            "current all={all}\n{}",
            selected
                .iter()
                .map(|r| format!("{} {}", r.git_ref_name, r.snapshot_id))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        source_generation: None,
        exact,
        native,
        source_stats,
        staged_compressions,
    };
    publish(repo, input, bundle, dry_run, control)
}
pub(super) type Bundle = (
    LocalGitObjectStore,
    GitHistoryStore,
    RawGitDocumentStore,
    LocalContentBinaryDb<1>,
);
#[derive(Clone)]
struct PreparedCommit {
    record: GitCommitMappingRecord,
    document_set: Option<u32>,
    native: bool,
}
struct PreparedExport {
    target: TargetInfo,
    stage: TempDir,
    refs: Vec<(u32, String, GitRefMappingRecord)>,
    closure: Vec<(GitObjectKind, [u8; 20])>,
    commits: Vec<PreparedCommit>,
    intent: String,
    source_generation: Option<String>,
    exact: usize,
    native: usize,
    source_stats: source_plan::TraversalStats,
    staged_compressions: usize,
}
fn publish(
    repo: &RepoRuntime,
    input: PreparedExport,
    bundle: Option<Bundle>,
    dry_run: bool,
    control: PublicationControl,
) -> Result<JsonValue, String> {
    let target = input.target.clone();
    let refs = &input.refs;
    let closure = &input.closure;
    let seen = closure.iter().map(|(_, oid)| *oid).collect::<BTreeSet<_>>();
    let exact = input.exact;
    let stage_dir = input.stage.path().join("objects.git");
    if target.object_format != "sha1" {
        return Err("Git export requires SHA-1 target".into());
    }
    let plan_text = format!(
        "native-export/v2\n{}\n{}\n{}",
        target.fingerprint,
        input.intent,
        refs.iter()
            .map(|(_, n, r)| format!(
                "{n} {} {} {}",
                hex(&r.object_id),
                r.snapshot_index_plus1,
                r.target_index_plus1
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let plan = sha256_prefixed("GEP", plan_text.as_bytes(), 24);
    let export_id = sha256_prefixed(
        "GIT-EXP",
        format!("{}\n{plan}", target.fingerprint).as_bytes(),
        16,
    );
    let operation = sha256_prefixed(
        "GIO-EXPORT",
        format!("{}\n{plan}", target.fingerprint).as_bytes(),
        16,
    );
    let head = refs
        .iter()
        .find(|(_, n, r)| n == "HEAD" && r.meta & 3 == 2)
        .and_then(|(_, _, head)| {
            refs.iter()
                .find(|(i, _, _)| i.checked_add(1) == Some(head.symbolic_target_index_plus1))
        })
        .map(|(_, n, _)| n.clone());
    let old_checkpoint = match &bundle {
        Some((_, history, _, _)) => history.checkpoint_by_hash(&bytes(&operation)?)?,
        None => None,
    };
    let report = |status: &str| json!({"contract":INTEROP_CONTRACT,"operation":"export","status":status,"operation_id":operation,"generation_id":export_id,"source_generation_id":input.source_generation,"plan_hash":plan,"object_count":closure.len(),"snapshot_count":input.commits.len(),"exact_git_object_reuse_count":exact,"native_commit_count":input.native,"ref_count":refs.iter().filter(|(_,n,_)|n!="HEAD").count(),"force_updated":false,"compare_and_swap":true,"head_symbolic_ref":head,"fsck":if status=="dry_run"{"pending"}else{"passed"},"dry_run":status=="dry_run","replayed":status=="no_op","resumed":old_checkpoint.as_ref().is_some_and(|r|r.status==0),"mutated":status=="completed","storage":"native_binary","metadata_bundle":GIT_INTEROP_BUNDLE_PATH,"source_traversal":{"visits":input.source_stats.visits,"edges":input.source_stats.edges,"body_reads":input.source_stats.body_reads,"staged_compressions":input.staged_compressions},"mapping_json_count":0});
    if dry_run {
        return Ok(report("dry_run"));
    }
    let (objects, mut history, documents, content) = match bundle {
        Some(bundle) => bundle,
        None => stores(repo)?,
    };
    if let Some(old) = &old_checkpoint {
        if old.plan_hash != bytes::<12>(&plan)? {
            return Err("export checkpoint does not match the current immutable plan".into());
        }
        if old.status == 1 {
            reverify_target(&target.git_dir, refs)?;
            return Ok(report("no_op"));
        }
    }
    let ait_fingerprint = sha256_prefixed(
        "ASR",
        format!(
            "ait-repository-fingerprint/v1\n{}\n{}\n",
            repo.repo_name(),
            repo.authoritative_repo_root().to_string_lossy()
        )
        .as_bytes(),
        24,
    );
    let ait_source = objects.ensure_repository(
        GitRepositoryKind::AitSource,
        0,
        bytes(&ait_fingerprint)?,
        &format!(
            "{}\n{}",
            repo.repo_name(),
            repo.authoritative_repo_root().to_string_lossy()
        ),
    )?;
    let target_repository = objects.ensure_repository(
        GitRepositoryKind::ExportTarget,
        1,
        bytes(&target.fingerprint)?,
        target.path.to_str().ok_or("non-UTF8 target identity")?,
    )?;
    let export_generation = history.ensure_generation(GitGenerationRecord {
        export: true,
        source_repository_index: ait_source,
        target_repository_index_plus1: target_repository
            .checked_add(1)
            .ok_or("target ordinal overflow")?,
        hash: bytes(&export_id)?,
    })?;
    let generations = history.generations()?;
    let old_refs = history.refs()?;
    let mut head_baseline = None;
    for (_, name, row) in old_refs.iter().rev() {
        if name != "HEAD"
            || row.generation_index == export_generation
            || !history.generation_is_complete(row.generation_index)?
        {
            continue;
        }
        let g = generations
            .get(row.generation_index as usize)
            .ok_or("HEAD generation absent")?;
        let matching_endpoint = if g.export {
            g.target_repository_index_plus1 == target_repository + 1
        } else {
            objects.repository_identity(g.source_repository_index)?.2
                == target.path.to_string_lossy()
        };
        if matching_endpoint {
            head_baseline = Some(HeadBaseline {
                binding: desired_head(&old_refs, row)?,
            });
            break;
        }
    }
    let created = old_refs
        .iter()
        .find(|(_, _, r)| r.generation_index == export_generation)
        .map(|(_, _, r)| r.created_at_s)
        .unwrap_or(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs(),
        );
    let mut desired = refs.clone();
    desired.sort_by_key(|(_, name, _)| name == "HEAD");
    let mut frozen_refs = Vec::new();
    let mut ordinal_map = BTreeMap::new();
    for (original_ordinal, name, source_ref) in desired {
        let frozen = old_refs
            .iter()
            .find(|(_, n, r)| n == &name && r.generation_index == export_generation)
            .cloned();
        let (ordinal, row) = if let Some((ordinal, _, row)) = frozen {
            (ordinal, row)
        } else {
            let actual = if target.existed {
                git_ref_object_id(&target.git_dir, &name)?
            } else {
                None
            };
            let mut previous = None;
            for candidate in old_refs.iter().rev() {
                let (_, n, r) = candidate;
                let Some(g) = generations.get(r.generation_index as usize) else {
                    continue;
                };
                let matching_endpoint = if g.export {
                    g.target_repository_index_plus1 == target_repository + 1
                } else {
                    objects.repository_identity(g.source_repository_index)?.2
                        == target.path.to_string_lossy()
                };
                if n == &name
                    && matching_endpoint
                    && history.generation_is_complete(r.generation_index)?
                {
                    previous = Some(candidate);
                    break;
                }
            }
            if let Some(actual) = &actual {
                if name == "HEAD" {
                    let binding = symbolic_head(&target.git_dir)?;
                    if binding != desired_head(refs, &source_ref)?
                        && head_baseline
                            .as_ref()
                            .is_none_or(|old| old.binding != binding)
                    {
                        return Err("Git export refuses changed symbolic HEAD binding; explicit reconciliation required".into());
                    }
                }
                if actual != &hex(&source_ref.object_id)
                    && previous.is_none_or(|(_, _, r)| actual != &hex(&r.object_id))
                {
                    return Err(format!(
                        "Git export refuses to overwrite {name}: compare-and-swap evidence differs"
                    ));
                }
            }
            let mut row = source_ref;
            row.generation_index = export_generation;
            row.created_at_s = created;
            if row.meta & 3 == 2 {
                row.symbolic_target_index_plus1 = *ordinal_map
                    .get(&row.symbolic_target_index_plus1.saturating_sub(1))
                    .ok_or("symbolic HEAD export target absent")?;
            }
            row.meta |= 16;
            row.expected_previous = actual.as_deref().map(bytes).transpose()?.unwrap_or([0; 20]);
            let ordinal = history.append_ref(&name, row.clone())?;
            (ordinal, row)
        };
        ordinal_map.insert(
            original_ordinal,
            ordinal.checked_add(1).ok_or("ref ordinal overflow")?,
        );
        frozen_refs.push((ordinal, name, row));
    }
    checkpoint(
        &history,
        export_generation,
        bytes(&operation)?,
        bytes(&plan)?,
        0,
        0,
        0,
    )?;
    git_repo_bytes(&stage_dir, ["fsck", "--strict", "--full"])?;
    // Raw object aliases and owner dependencies become durable before Git ref publication.
    let mut batch = Vec::new();
    let mut batch_bytes = 0;
    let mut reader = GitImportObjectReader::new(&stage_dir)?;
    for (kind, id) in closure {
        let body = reader.content(&hex(id), kind_name(*kind))?;
        batch_bytes += body.len();
        batch.push((*kind, *id, body));
        if batch.len() >= 128 || batch_bytes >= 8 * 1024 * 1024 {
            persist_batch(&objects, ait_source, &batch)?;
            persist_batch(&objects, target_repository, &batch)?;
            batch.clear();
            batch_bytes = 0;
        }
    }
    persist_batch(&objects, ait_source, &batch)?;
    persist_batch(&objects, target_repository, &batch)?;
    let existing = history
        .commit_mappings()
        .into_iter()
        .filter(|r| r.generation_index == export_generation)
        .map(|r| (r.object_id, r))
        .collect::<BTreeMap<_, _>>();
    let empty = if input
        .commits
        .iter()
        .any(|c| c.document_set.is_none() && !existing.contains_key(&c.record.object_id))
    {
        Some(content.ensure_raw_trees(&[Vec::new()], &mut RawTreeSession::default())?[0].tree_index)
    } else {
        None
    };
    for commit in &input.commits {
        if !seen.contains(&commit.record.object_id) {
            continue;
        }
        let mut row = commit.record.clone();
        row.generation_index = export_generation;
        if let Some(old) = existing.get(&row.object_id) {
            if commit.document_set.is_none()
                && !documents
                    .document_set_files(old.document_set_tree_index)?
                    .is_empty()
            {
                return Err("pure-native export resumed with nonempty DocSet".into());
            }
            row.document_set_tree_index =
                commit.document_set.unwrap_or(old.document_set_tree_index);
            if commit.native {
                row.author_identity_index = old.author_identity_index;
                row.committer_identity_index = old.committer_identity_index;
                row.created_at_s = old.created_at_s;
            }
            history.append_commit(row)?;
            continue;
        }
        row.document_set_tree_index = commit
            .document_set
            .or(empty)
            .ok_or("export DocSet absent")?;
        if commit.native {
            let raw = git_repo_bytes(&stage_dir, ["cat-file", "commit", &hex(&row.object_id)])?;
            row.author_identity_index = history.append_identity(identity(&raw, b"author ")?)?;
            row.committer_identity_index =
                history.append_identity(identity(&raw, b"committer ")?)?;
            row.created_at_s = created;
        }
        history.append_commit(row)?;
    }
    checkpoint(
        &history,
        export_generation,
        bytes(&operation)?,
        bytes(&plan)?,
        0,
        input
            .commits
            .len()
            .try_into()
            .map_err(|_| "commit cursor overflow")?,
        0,
    )?;
    let target_dir = prepare_export_target(&target)?;
    for (_, name, row) in frozen_refs.iter().filter(|(_, n, _)| n != "HEAD") {
        let actual = git_ref_object_id(&target_dir, name)?.unwrap_or_else(|| ZERO_SHA1.into());
        if actual != hex(&row.object_id) && actual != hex(&row.expected_previous) {
            return Err(format!(
                "Git export refuses to overwrite {name}: frozen compare-and-swap evidence differs"
            ));
        }
    }
    copy_verified_objects(
        &stage_dir.join("objects"),
        &target_dir.join("objects"),
        &seen,
    )?;
    if control.stop_after_objects {
        return Err(format!("Injected Git mirror interruption after object transfer for {operation}; rerun the same command before public ref movement."));
    }
    publish_ref_transaction(&target_dir, &frozen_refs, head_baseline.as_ref(), control)?;
    git_repo_bytes(&target_dir, ["fsck", "--strict", "--full"])?;
    checkpoint(
        &history,
        export_generation,
        bytes(&operation)?,
        bytes(&plan)?,
        1,
        input
            .commits
            .len()
            .try_into()
            .map_err(|_| "commit cursor overflow")?,
        frozen_refs
            .len()
            .try_into()
            .map_err(|_| "ref cursor overflow")?,
    )?;
    Ok(report("completed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    use tempfile::TempDir;

    #[test]
    fn unselected_annotated_tag_proofs_do_not_add_private_fsck_roots() {
        let (root, repo, target) = fixture("main");
        let source = root.path().join("source");
        git(
            &source,
            &[
                "tag",
                "-a",
                "private-work",
                "work",
                "-m",
                "unselected branch tag",
            ],
        );
        let tag = git(&source, &["rev-parse", "refs/tags/private-work"]);
        let head = git(&source, &["rev-parse", "main"]);
        git_import(&repo, source.to_str().unwrap(), true, false).unwrap();
        fs::rename(source.join(".git"), root.path().join("source-odb.offline")).unwrap();
        current(&repo, target.to_str().unwrap(), false, false).unwrap();
        assert_eq!(git(&target, &["rev-parse", "HEAD"]), head);
        assert!(!Command::new("git")
            .arg("-C")
            .arg(&target)
            .args(["cat-file", "-e", &tag])
            .status()
            .unwrap()
            .success());
        git(&target, &["fsck", "--strict", "--full"]);
    }

    #[test]
    fn unbound_native_tag_cannot_choose_between_distinct_source_aliases_of_one_snapshot() {
        let (root, repo, target) = fixture("work");
        let source = root.path().join("source");
        let old = git(&source, &["rev-parse", "HEAD"]);
        git(&source, &["branch", "older", &old]);
        fs::write(source.join("README.md"), b"Markdown-only successor\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "Markdown alias"]);
        git_import(&repo, source.to_str().unwrap(), true, false).unwrap();
        let id = {
            let (_, history, _, content) = read_stores(&repo).unwrap();
            let old = bytes::<20>(&old).unwrap();
            let index = history
                .commit_mappings()
                .into_iter()
                .find(|row| row.object_id == old)
                .unwrap()
                .snapshot_index;
            snapshot(&content, index).unwrap().snapshot_id
        };
        crate::tag_surface::tag_store(&repo)
            .unwrap()
            .ensure_imported_tag(
                &new_tag_record("native-alias", &id, "native tag", "2026-10-15T00:00:00Z").unwrap(),
            )
            .unwrap();
        let error = current(&repo, target.to_str().unwrap(), true, true).unwrap_err();
        assert!(error.contains("source anchor ambiguity"), "{error}");
    }

    #[test]
    fn overlapping_source_roots_read_and_compress_each_union_object_once() {
        let (root, repo, _) = fixture("work");
        let source = root.path().join("source");
        git(&source, &["tag", "-a", "annotated", "-m", "annotation"]);
        git(&source, &["tag", "light"]);
        git_import(&repo, source.to_str().unwrap(), true, false).unwrap();
        let (objects, history, _, content) = read_stores(&repo).unwrap();
        let (generation, record) = select(&history, None).unwrap();
        let rows = history
            .refs()
            .unwrap()
            .into_iter()
            .filter(|(_, _, r)| r.generation_index == generation)
            .collect::<Vec<_>>();
        let mut measurements = Vec::new();
        for copies in [1, 32] {
            let stage = new_stage().unwrap();
            let stage_dir = stage.path().join("objects.git");
            let writer = StagedObjects::new(&stage_dir);
            let mut plan = SourcePlan::new(&objects, &content, &writer).unwrap();
            for _ in 0..copies {
                for (_, _, r) in &rows {
                    if r.object_kind == GitObjectKind::Tag {
                        plan.peel_commit(record.source_repository_index, r.object_id)
                            .unwrap();
                    }
                    plan.root(
                        record.source_repository_index,
                        r.object_id,
                        Some(generation),
                    );
                }
            }
            let mut prepared = BTreeMap::new();
            let stats = plan.finish(&history, &mut prepared, &[]).unwrap();
            assert_eq!(stats.visits, stats.body_reads);
            assert_eq!(stats.visits, writer.compressions.get());
            assert_eq!(prepared.len(), 2);
            git(&stage_dir, &["fsck", "--strict", "--full"]);
            measurements.push((stats, writer.compressions.get()));
        }
        assert_eq!(measurements[0], measurements[1]);
    }

    fn git(root: &Path, args: &[&str]) -> String {
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
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }
    fn fixture(initial_head: &str) -> (TempDir, RepoRuntime, PathBuf) {
        let root = TempDir::new().unwrap();
        let source = root.path().join("source");
        let ait = root.path().join("ait");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&ait).unwrap();
        git(&source, &["init", "-b", "main"]);
        git(&source, &["config", "user.name", "Publication fixture"]);
        git(&source, &["config", "user.email", "publication@local"]);
        fs::write(source.join("code.txt"), b"main code\n").unwrap();
        fs::write(source.join("README.md"), b"source docs\r\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "main"]);
        git(&source, &["checkout", "-b", "work"]);
        fs::write(source.join("code.txt"), b"work code\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "work"]);
        git(&source, &["checkout", initial_head]);
        init_repo(&InitRequest {
            root: ait.clone(),
            name: Some("publication".into()),
            default_line: "main".into(),
            policy_profile: "prototype".into(),
            default_author_mode: "ai_with_human_review".into(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let repo = RepoRuntime::discover_from_path(&ait).unwrap();
        git_import(&repo, source.to_str().unwrap(), true, false).unwrap();
        let target = root.path().join("export.git");
        (root, repo, target)
    }

    #[test]
    fn staged_loose_object_reuse_is_exact_and_damage_is_preserved() {
        let stage = new_stage().unwrap();
        let root = stage.path().join("objects.git");
        let body = b"raw binary\r\n\xff\0artifact\n";
        let oid = write_object(&root, GitObjectKind::Blob, body).unwrap();
        assert_eq!(oid, git_object_id(GitObjectKind::Blob, body));
        assert_eq!(
            git_repo_bytes(&root, ["cat-file", "blob", &hex(&oid)]).unwrap(),
            body
        );
        let name = hex(&oid);
        let path = root.join("objects").join(&name[..2]).join(&name[2..]);
        let original = fs::read(&path).unwrap();
        assert_eq!(write_object(&root, GitObjectKind::Blob, body).unwrap(), oid);
        assert_eq!(fs::read(&path).unwrap(), original);
        let mut wrong = ZlibEncoder::new(Vec::new(), Compression::default());
        wrong.write_all(b"blob 4\0evil").unwrap();
        let wrong = wrong.finish().unwrap();
        fs::write(&path, &wrong).unwrap();
        assert!(write_object(&root, GitObjectKind::Blob, body)
            .unwrap_err()
            .contains("differs at the same OID"));
        assert_eq!(fs::read(&path).unwrap(), wrong);
        fs::write(&path, b"corrupt").unwrap();
        assert!(write_object(&root, GitObjectKind::Blob, body).is_err());
        assert_eq!(fs::read(path).unwrap(), b"corrupt");
    }

    #[test]
    fn loose_publication_accepts_equivalent_zlib_and_preserves_bad_oid() {
        let stage = new_stage().unwrap();
        let root = stage.path().join("objects.git");
        let body = b"same original immutable Git body\r\n\xff\0";
        let oid = write_object(&root, GitObjectKind::Blob, body).unwrap();
        let name = hex(&oid);
        let target = TempDir::new().unwrap();
        let directory = target.path().join(&name[..2]);
        fs::create_dir(&directory).unwrap();
        let path = directory.join(&name[2..]);
        let mut different = ZlibEncoder::new(Vec::new(), Compression::none());
        different
            .write_all(format!("blob {}\0", body.len()).as_bytes())
            .unwrap();
        different.write_all(body).unwrap();
        let different = different.finish().unwrap();
        fs::write(&path, &different).unwrap();
        assert_ne!(
            different,
            fs::read(root.join("objects").join(&name[..2]).join(&name[2..])).unwrap()
        );
        let selected = BTreeSet::from([oid]);
        copy_verified_objects(&root.join("objects"), target.path(), &selected).unwrap();
        assert_eq!(fs::read(&path).unwrap(), different);
        let mut invalid = ZlibEncoder::new(Vec::new(), Compression::default());
        invalid.write_all(b"blob 4\0evil").unwrap();
        let invalid = invalid.finish().unwrap();
        fs::write(&path, &invalid).unwrap();
        assert!(copy_verified_objects(&root.join("objects"), target.path(), &selected).is_err());
        assert_eq!(fs::read(&path).unwrap(), invalid);
    }

    #[test]
    fn publication_crash_child() {
        let Some(root) = std::env::var_os("AIT_GIT_PUBLICATION_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let repo = RepoRuntime::discover_from_path(&root.join("ait")).unwrap();
        current_with_publication_control(
            &repo,
            root.join("export.git").to_str().unwrap(),
            true,
            false,
            PublicationControl {
                pause_after_refs: true,
                ..PublicationControl::default()
            },
        )
        .unwrap();
        panic!("publication child should be killed at its durable boundary");
    }

    #[cfg(unix)]
    #[test]
    fn publication_resumes_after_process_kill_between_refs_and_head() {
        use std::os::unix::process::ExitStatusExt;
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let (root, repo, target) = fixture("main");
        current(&repo, target.to_str().unwrap(), true, false).unwrap();
        let source = root.path().join("source");
        fs::write(
            source.join("main-advance.txt"),
            b"changed old HEAD referent\n",
        )
        .unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "advance main"]);
        git(&source, &["checkout", "work"]);
        git_import(&repo, source.to_str().unwrap(), true, false).unwrap();
        let marker = root.path().join("publication-paused");
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "primitives::git_interop::native::exports::tests::publication_crash_child",
                    "--nocapture",
                ])
                .env("AIT_GIT_PUBLICATION_TEST_ROOT", root.path())
                .env("AIT_GIT_PUBLICATION_TEST_MARKER", &marker)
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
        assert!(marker.exists(), "child did not reach durable refs boundary");
        assert_eq!(git(&target, &["symbolic-ref", "HEAD"]), "refs/heads/main");
        assert_ne!(
            git(&target, &["rev-parse", "refs/heads/main"]),
            git(&target, &["rev-parse", "refs/heads/work"])
        );
        child.0.kill().unwrap();
        assert_eq!(child.0.wait().unwrap().signal(), Some(9));
        let before = read_native_file(&repo, "git_commit_mapping.bin");
        let pending = read_stores(&repo).unwrap().1;
        assert!(pending
            .generations()
            .unwrap()
            .iter()
            .enumerate()
            .any(|(i, g)| g.export && !pending.generation_is_complete(i as u32).unwrap()));
        drop(pending);
        let resumed = current(&repo, target.to_str().unwrap(), true, false).unwrap();
        assert_eq!(resumed["status"], json!("completed"));
        assert_eq!(resumed["resumed"], json!(true));
        assert_eq!(git(&target, &["symbolic-ref", "HEAD"]), "refs/heads/work");
        assert_eq!(before, read_native_file(&repo, "git_commit_mapping.bin"));
        assert_eq!(
            current(&repo, target.to_str().unwrap(), true, false).unwrap()["status"],
            json!("no_op")
        );
        git(&target, &["fsck", "--strict", "--full"]);
        assert!(!root
            .path()
            .join("ait/.ait/git-interop/v1/mappings")
            .exists());
        println!("actual SIGKILL after durable ordinary refs; fresh process state resumes HEAD binding, exact mappings and strict fsck; 0 mapping JSON");
    }

    #[test]
    fn publication_rejects_same_oid_foreign_head_before_ref_changes() {
        let (root, repo, target) = fixture("work");
        current(&repo, target.to_str().unwrap(), true, false).unwrap();
        let old = git(&target, &["rev-parse", "refs/heads/work"]);
        git(&target, &["update-ref", "refs/heads/foreign", &old]);
        git(&target, &["symbolic-ref", "HEAD", "refs/heads/foreign"]);
        fs::write(root.path().join("ait/native.txt"), b"new native work\n").unwrap();
        crate::primitives::workspace::snapshot_create_in_current_workspace(
            &repo,
            Some("native advance"),
        )
        .unwrap();
        let before = git(
            &target,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        );
        let error = current(&repo, target.to_str().unwrap(), true, false).unwrap_err();
        assert!(error.contains("changed symbolic HEAD binding"), "{error}");
        assert_eq!(
            before,
            git(
                &target,
                &["for-each-ref", "--format=%(refname) %(objectname)"]
            )
        );
        assert_eq!(
            git(&target, &["symbolic-ref", "HEAD"]),
            "refs/heads/foreign"
        );
        git(&target, &["symbolic-ref", "HEAD", "refs/heads/work"]);
        current(&repo, target.to_str().unwrap(), true, false).unwrap();
    }

    #[test]
    fn exact_head_cas_rejects_changed_bytes_and_preserves_foreign_lock() {
        let root = TempDir::new().unwrap();
        let head = root.path().join("HEAD");
        fs::write(&head, b"ref: refs/heads/third\n").unwrap();
        assert!(cas_head_bytes(
            root.path(),
            b"ref: refs/heads/main\n",
            b"ref: refs/heads/work\n"
        )
        .is_err());
        assert_eq!(fs::read(&head).unwrap(), b"ref: refs/heads/third\n");
        assert!(!root.path().join("HEAD.lock").exists());
        fs::write(root.path().join("HEAD.lock"), b"another writer").unwrap();
        assert!(cas_head_bytes(
            root.path(),
            b"ref: refs/heads/third\n",
            b"ref: refs/heads/work\n"
        )
        .is_err());
        assert_eq!(
            fs::read(root.path().join("HEAD.lock")).unwrap(),
            b"another writer"
        );
    }
}
