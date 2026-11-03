//! Complete mirror ref sets use the existing generation/ref/checkpoint owners.
//! Absence in an admitted full-set generation is the deletion baseline.
use super::*;
use ait_core::binary_db::BinaryFileId;

pub(super) fn baseline_operation(identity: &str, generation: [u8; 8]) -> Result<[u8; 8], String> {
    bytes(&sha256_prefixed(
        "GMB",
        format!("mirror-baseline/v1\n{identity}\n{}\n", hex(&generation)).as_bytes(),
        16,
    ))
}
pub(in crate::primitives::git_interop) fn latest_mirror_baseline(
    repo: &RepoRuntime,
    identity: &str,
) -> Result<[u8; 8], String> {
    if !exists(repo) {
        return Ok([0; 8]);
    }
    let (objects, history, _, _) = read_stores(repo)?;
    let mut latest = [0; 8];
    for (i, g) in history.generations()?.iter().enumerate() {
        if !g.export
            && objects.repository_identity(g.source_repository_index)?.2 == identity
            && history
                .checkpoint_by_hash(&baseline_operation(identity, g.hash)?)?
                .is_some_and(|r| r.status == 1 && r.generation_index == i as u32)
        {
            latest = g.hash;
        }
    }
    Ok(latest)
}

pub(in crate::primitives::git_interop) fn record_mirror_baseline(
    repo: &RepoRuntime,
    source: &SourceInfo,
    selected: &BTreeMap<String, ExportRef>,
) -> Result<(), String> {
    let (objects, mut history, documents, content) = stores(repo)?;
    let git_refs = source
        .refs
        .iter()
        .filter(|r| r.name.starts_with("refs/heads/") || r.name.starts_with("refs/tags/"))
        .map(|r| (r.name.clone(), r.object_id.clone()))
        .collect::<BTreeMap<_, _>>();
    let material = format!(
        "mirror-full-ref-set/v1\n{}\n{}\n{}\n{}\n",
        source.source_identity,
        source.head_symbolic_ref.as_deref().unwrap_or("none"),
        source.head_object_id.as_deref().unwrap_or("none"),
        git_refs
            .iter()
            .map(|(name, oid)| format!(
                "{name} {oid} {}",
                selected
                    .get(name)
                    .map(super::super::mirror::ait_ref_state_id)
                    .unwrap_or_else(|| "none".into())
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let plan = bytes(&sha256_prefixed("GMP", material.as_bytes(), 24))?;
    let mut prior = [0; 8];
    for (i, g) in history.generations()?.iter().enumerate() {
        if !g.export
            && objects.repository_identity(g.source_repository_index)?.2 == source.source_identity
            && history
                .checkpoint_by_hash(&baseline_operation(&source.source_identity, g.hash)?)?
                .is_some_and(|r| r.status == 1 && r.generation_index == i as u32)
        {
            prior = g.hash;
        }
    }
    if prior != [0; 8]
        && history
            .checkpoint_by_hash(&baseline_operation(&source.source_identity, prior)?)?
            .is_some_and(|r| r.plan_hash == plan)
    {
        return Ok(());
    }
    let hash = bytes(&sha256_prefixed(
        "GIT-MIR",
        format!("{material}previous={}\n", hex(&prior)).as_bytes(),
        16,
    ))?;
    let operation = baseline_operation(&source.source_identity, hash)?;
    if let Some(done) = history.checkpoint_by_hash(&operation)? {
        if done.plan_hash != plan {
            return Err("mirror full-set baseline intent changed".into());
        }
        if done.status == 1 {
            return Ok(());
        }
    }
    let repository = objects.ensure_repository(
        GitRepositoryKind::ImportSource,
        1,
        bytes(&source.fingerprint)?,
        &source.source_identity,
    )?;
    let generation = history.ensure_generation(GitGenerationRecord {
        export: false,
        source_repository_index: repository,
        target_repository_index_plus1: 0,
        hash,
    })?;
    let existing = history
        .refs()?
        .into_iter()
        .filter(|(_, _, r)| r.generation_index == generation)
        .collect::<Vec<_>>();
    let created = existing.first().map(|(_, _, r)| r.created_at_s).unwrap_or(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs(),
    );
    let interop = objects.db().clone();
    let read = BinaryDbReadTxn::new(&interop);
    let mut namespaces = Vec::new();
    for i in 0..read.record_count(BinaryFileId::new("git_repository.bin", 1, 28))? {
        let (kind, _, identity) = objects.repository_identity(i)?;
        if identity == source.source_identity
            && matches!(
                kind,
                GitRepositoryKind::ImportSource | GitRepositoryKind::ExportTarget
            )
        {
            namespaces.push(i);
        }
    }
    drop(read);
    let mut todo = git_refs
        .values()
        .map(|oid| bytes::<20>(oid))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(oid) = source.head_object_id.as_deref() {
        todo.push(bytes(oid)?);
    }
    let mut seen = BTreeSet::new();
    let mut batch = Vec::new();
    let mut batch_bytes = 0;
    while let Some(oid) = todo.pop() {
        if !seen.insert(oid) {
            continue;
        }
        let mut found = None;
        for namespace in &namespaces {
            if let Some(value) = objects.get(*namespace, 1, &oid)? {
                found = Some(value);
                break;
            }
        }
        let (record, body) = found
            .ok_or("mirror baseline raw closure is absent from admitted endpoint namespaces")?;
        todo.extend(children(record.kind, &body)?);
        batch_bytes += body.len();
        batch.push((record.kind, oid, body));
        if batch.len() >= 128 || batch_bytes >= 8 * 1024 * 1024 {
            persist_batch(&objects, repository, &batch)?;
            batch.clear();
            batch_bytes = 0;
        }
    }
    persist_batch(&objects, repository, &batch)?;
    let tag_store = crate::tag_surface::tag_store(repo)?;
    let mut branch_ordinal = None;
    for (name, oid_text) in &git_refs {
        let oid = bytes::<20>(oid_text)?;
        let (object, _) = objects
            .get(repository, 1, &oid)?
            .ok_or("mirror baseline ref absent")?;
        let (snapshot_plus, target_plus, clock) = if let Some(native) = selected.get(name) {
            let view = content
                .snapshots()
                .get_snapshot_view(&BinaryDbReadTxn::new(content.db()), &native.snapshot_id)?
                .ok_or("mirror baseline Snapshot absent")?;
            let target = if native.ait_kind == "tag" {
                tag_store.tag_index_by_name(&native.ait_name)?
            } else {
                ait_core::line_binary_db::binary_line_index_by_name(
                    &BinaryDbReadTxn::new(content.db()),
                    &native.ait_name,
                )?
            }
            .ok_or("mirror baseline native target absent")?;
            let mut commit = oid;
            loop {
                let (object, body) = objects
                    .get(repository, 1, &commit)?
                    .ok_or("mirror baseline peel absent")?;
                if object.kind == GitObjectKind::Tag {
                    commit = children(object.kind, &body)?[0];
                } else if object.kind == GitObjectKind::Commit {
                    break;
                } else {
                    return Err("native mirror ref does not peel to a commit".into());
                }
            }
            let mappings = history.commit_mappings();
            if !mappings
                .iter()
                .any(|r| r.generation_index == generation && r.object_id == commit)
            {
                let mut alias = mappings
                    .into_iter()
                    .find(|r| r.object_id == commit && r.snapshot_index == view.snapshot_index)
                    .ok_or("mirror baseline exact commit/Snapshot alias absent")?;
                alias.generation_index = generation;
                history.append_commit(alias)?;
            }
            let clock = if native.ait_kind == "tag" {
                DateTime::parse_from_rfc3339(
                    native
                        .created_at
                        .as_deref()
                        .ok_or("mirror Tag time absent")?,
                )
                .map_err(|_| "mirror Tag time invalid")?
                .timestamp()
                .try_into()
                .map_err(|_| "mirror Tag time negative")?
            } else {
                created
            };
            (view.snapshot_index + 1, target + 1, clock)
        } else {
            if !name.starts_with("refs/tags/") {
                return Err("Git branch has no mirrored native target".into());
            }
            (0, 0, created)
        };
        let ordinal = history.append_ref(
            name,
            GitRefMappingRecord {
                meta: u8::from(name.starts_with("refs/tags/"))
                    | if snapshot_plus > 0 { 12 } else { 0 },
                object_kind: object.kind,
                generation_index: generation,
                snapshot_index_plus1: snapshot_plus,
                target_index_plus1: target_plus,
                symbolic_target_index_plus1: 0,
                object_id: oid,
                expected_previous: [0; 20],
                created_at_s: clock,
            },
        )?;
        if source.head_symbolic_ref.as_deref() == Some(name) {
            branch_ordinal = Some(ordinal + 1);
        }
    }
    if let Some(oid) = source.head_object_id.as_deref() {
        let symbolic = branch_ordinal.is_some();
        history.append_ref(
            "HEAD",
            GitRefMappingRecord {
                meta: if symbolic { 2 } else { 32 },
                object_kind: GitObjectKind::Commit,
                generation_index: generation,
                snapshot_index_plus1: 0,
                target_index_plus1: 0,
                symbolic_target_index_plus1: branch_ordinal.unwrap_or(0),
                object_id: bytes(oid)?,
                expected_previous: [0; 20],
                created_at_s: created,
            },
        )?;
    }
    admission::validate(&objects, &history, &documents, &content)?;
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("AIT_GIT_MIRROR_BASELINE_TEST_PAUSE") {
        fs::write(marker, b"mirror full-set owners durable before completion")
            .map_err(|e| e.to_string())?;
        loop {
            std::thread::park();
        }
    }
    let commits = history
        .commit_mappings()
        .iter()
        .filter(|r| r.generation_index == generation)
        .count()
        .try_into()
        .map_err(|_| "mirror baseline cursor overflow")?;
    let refs = history
        .refs()?
        .iter()
        .filter(|(_, _, r)| r.generation_index == generation)
        .map(|(_, name, _)| name)
        .collect::<BTreeSet<_>>()
        .len()
        .try_into()
        .map_err(|_| "mirror baseline cursor overflow")?;
    checkpoint(&history, generation, operation, plan, 1, commits, refs)?;
    Ok(())
}
