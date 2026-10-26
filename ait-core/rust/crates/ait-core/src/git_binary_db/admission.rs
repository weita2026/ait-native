//! Admit cross-root owner dependencies before runtime reads or writes.
use super::*;
use crate::binary_db::{BinaryDb, BinaryDbReadTxn};
use crate::content_binary_db::{BinarySnapshotView, LocalContentBinaryDb};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[path = "admission/raw_trees.rs"]
mod raw_trees;
use raw_trees::RawTreeReadCache;
#[path = "admission/body_identities.rs"]
mod body_identities;
use body_identities::BodyIdentityMemo;

#[derive(Clone, Copy)]
struct AdmittedNamespace {
    repository: u32,
    closure_owner: u32,
}

#[cfg(test)]
type BindingReuseHook = Box<
    dyn FnOnce(
        &mut crate::content_binary_db::BinaryBlobReadSession,
        &mut crate::content_binary_db::BinaryDbTreeReadCache,
        &mut crate::content_binary_db::BinaryDbTreeReadCache,
    ),
>;
#[cfg(test)]
thread_local! {
    static BINDING_REUSE_HOOK: std::cell::RefCell<Option<BindingReuseHook>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) fn set_binding_reuse_hook(hook: BindingReuseHook) {
    BINDING_REUSE_HOOK.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(hook);
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BodyIdentity {
    size: u64,
    sha256: [u8; 32],
}
fn native_body_identity(
    content: &LocalContentBinaryDb<1>,
    session: &mut GitObjectReadSession<'_>,
    index: u32,
    verified: &mut BTreeSet<u32>,
    memo: &mut BodyIdentityMemo,
) -> Result<BodyIdentity, String> {
    if content.db().authority_root() != session.content_read.db().authority_root() {
        return Err("Blob identity read scope must share content authority".into());
    }
    if let Some(identity) = memo.ordinal(index) {
        return Ok(identity);
    }
    let view = content.blobs().blob_view_at(&session.content_read, index)?;
    if view.record.is_pruned() || view.record.is_tombstone() {
        return Err("unavailable Git native Blob".into());
    }
    if verified.insert(index) {
        content.blobs().read_blob_bytes_for_view_with_session(
            &session.content_read,
            &view,
            session.body_read,
            0,
            None,
        )?;
    }
    let identity = BodyIdentity {
        size: view.size_bytes,
        sha256: view.record.sha256,
    };
    memo.remember_ordinal(index, identity.clone())?;
    Ok(identity)
}

pub fn validate_git_content_closure(
    objects: &LocalGitObjectStore,
    history: &GitHistoryStore,
    documents: &RawGitDocumentStore,
    content: &LocalContentBinaryDb<1>,
) -> Result<(), String> {
    validate_git_content_closure_with_session(
        objects,
        history,
        documents,
        content,
        &mut crate::content_binary_db::BinaryBlobReadSession::for_git_admission(),
    )
}

pub(crate) fn validate_git_content_closure_with_session(
    objects: &LocalGitObjectStore,
    history: &GitHistoryStore,
    documents: &RawGitDocumentStore,
    content: &LocalContentBinaryDb<1>,
    body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
) -> Result<(), String> {
    validate_git_content_closure_inner(
        objects,
        history,
        documents,
        content,
        body_read,
        true,
        #[cfg(test)]
        None,
    )
}

#[cfg(test)]
pub(super) fn validate_git_content_closure_legacy_for_profile(
    objects: &LocalGitObjectStore,
    history: &GitHistoryStore,
    documents: &RawGitDocumentStore,
    content: &LocalContentBinaryDb<1>,
) -> Result<(), String> {
    validate_git_content_closure_inner(
        objects,
        history,
        documents,
        content,
        &mut crate::content_binary_db::BinaryBlobReadSession::for_git_admission(),
        false,
        None,
    )
}

/// Explicit bounded profile only: all selected mapping/body/FK checks remain;
/// a subset does not claim complete ref/checkpoint admission.
#[cfg(test)]
pub(super) fn validate_mapping_order_for_profile(
    objects: &LocalGitObjectStore,
    history: &GitHistoryStore,
    documents: &RawGitDocumentStore,
    content: &LocalContentBinaryDb<1>,
    physical: bool,
    limit: Option<usize>,
) -> Result<(), String> {
    validate_git_content_closure_inner(
        objects,
        history,
        documents,
        content,
        &mut crate::content_binary_db::BinaryBlobReadSession::for_git_admission(),
        true,
        Some((physical, limit, false)),
    )
}

fn validate_git_content_closure_inner(
    objects: &LocalGitObjectStore,
    history: &GitHistoryStore,
    documents: &RawGitDocumentStore,
    content: &LocalContentBinaryDb<1>,
    body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
    reuse: bool,
    #[cfg(test)] profile: Option<(bool, Option<usize>, bool)>,
) -> Result<(), String> {
    let generations = history.generations()?;
    let mappings = history.commit_mappings();
    #[cfg(test)]
    let mappings = if let Some((physical, limit, _)) = profile {
        let mut selected = history
            .physical_commit_mappings_for_profile()
            .into_iter()
            .take(limit.unwrap_or(usize::MAX))
            .collect::<Vec<_>>();
        if !physical {
            selected.sort_unstable_by_key(|r| (r.generation_index, r.object_id));
        }
        selected
    } else {
        mappings
    };
    let mut verified = BTreeSet::new();
    let mut verified_bodies = BTreeSet::new();
    let identity_memo_enabled = reuse;
    #[cfg(test)]
    let identity_memo_enabled = profile.map_or(identity_memo_enabled, |(_, _, enabled)| enabled);
    let mut identity_memo = BodyIdentityMemo::new(if identity_memo_enabled {
        16 * 1024 * 1024
    } else {
        0
    });
    // Exact immutable component ordinals, bounded and local to this admission.
    // Complete namespace inventory equivalence permits operation-local reuse.
    // Every distinct native body/OID/kind still requires full raw admission.
    let mut bindings = BTreeSet::new();
    let mut docsets = BTreeMap::new();
    let mut docset_bytes = 0usize;
    let mut session = objects.read_session(body_read);
    let mut tree_cache = crate::content_binary_db::BinaryDbTreeReadCache::for_git_admission();
    let mut document_trees = crate::content_binary_db::BinaryDbTreeReadCache::for_git_admission();
    session.verify_metadata_authority()?;
    let namespace_signatures = { session.namespace_body_signatures()? };
    let mut owners = BTreeMap::new();
    let namespace_owners = namespace_signatures
        .iter()
        .map(|(&repository, &signature)| {
            let owner = if reuse {
                *owners.entry(signature).or_insert(repository)
            } else {
                repository
            };
            (repository, owner)
        })
        .collect::<BTreeMap<_, _>>();
    let mut raw_trees = RawTreeReadCache::new(if reuse { 16 * 1024 * 1024 } else { 0 });

    for mapping in &mappings {
        #[cfg(test)]
        {}
        // Logical bodies never cross a mapping. Only bounded verified chunks do.
        session.body_read.verify_admission_stamps()?;
        session.body_read.clear_logical_values();
        let generation = generations
            .get(mapping.generation_index as usize)
            .ok_or("Git mapping generation FK absent")?;
        let repository = generation.source_repository_index;
        let (record, raw) = session
            .get(repository, 1, &mapping.object_id)?
            .ok_or("Git mapping raw commit absent")?;
        if record.kind != GitObjectKind::Commit
            || raw.split(|b| *b == b'\n').next()
                != Some(format!("tree {}", hex(&mapping.tree_object_id)).as_bytes())
            || identity(&raw, b"author ")?
                != mapping_identity(history, &session, mapping.author_identity_index, reuse)?
            || identity(&raw, b"committer ")?
                != mapping_identity(history, &session, mapping.committer_identity_index, reuse)?
        {
            return Err("Git mapping raw tree/identity binding differs".into());
        }
        verify_closure(
            &mut session,
            AdmittedNamespace {
                repository,
                closure_owner: namespace_owners[&repository],
            },
            mapping.object_id,
            &mut verified,
            &mut verified_bodies,
            &mut raw_trees,
        )?;
        let view = snapshot(
            content,
            &session.content_read,
            mapping.snapshot_index,
            reuse,
        )?;
        let root = session
            .record(repository, 1, &mapping.tree_object_id)?
            .ok_or("Git mapping root Tree absent")?;
        if root.kind != GitObjectKind::Tree
            || !verified.contains(&(namespace_owners[&repository], mapping.tree_object_id))
        {
            return Err("Git mapping root Tree was not admitted".into());
        }
        // Native Blob ordinal identifies fully verified raw bytes, including
        // SHA256; a SHA1 collision across repositories cannot reuse this proof.
        let binding = (
            *namespace_signatures
                .get(&repository)
                .ok_or("Git mapping namespace absent")?,
            root.content_blob_index,
            mapping.snapshot_index,
            mapping.document_set_tree_index,
        );
        if bindings.contains(&binding) {
            #[cfg(test)]
            BINDING_REUSE_HOOK.with(|slot| {
                if let Some(hook) = slot.borrow_mut().take() {
                    hook(session.body_read, &mut tree_cache, &mut document_trees);
                }
            });
            #[cfg(test)]
            crate::binary_db::read_profile::note("mapping_binding_reuses", 0);
            continue;
        }

        #[cfg(test)]
        crate::binary_db::read_profile::note("mapping_binding_validations", 0);
        #[cfg(test)]
        let identity_phase = std::time::Instant::now();
        let expected = tree_file_identities(
            &mut session,
            AdmittedNamespace {
                repository,
                closure_owner: namespace_owners[&repository],
            },
            mapping.tree_object_id,
            &verified,
            &mut verified_bodies,
            content,
            IdentityReadCaches {
                trees: &mut raw_trees,
                blobs: &mut identity_memo,
            },
        )?;
        let mut actual = if identity_memo_enabled {
            body_identities::code_file_identities(
                content,
                &mut session,
                &view.snapshot_id,
                &mut tree_cache,
                &mut verified_bodies,
                &mut identity_memo,
            )?
        } else {
            let mut actual = BTreeMap::new();
            for row in content.snapshots().snapshot_tree_file_rows_with_cache(
                &session.content_read,
                &view.snapshot_id,
                &mut tree_cache,
            )? {
                if row.path.to_ascii_lowercase().ends_with(".md") {
                    return Err("Git code Snapshot contains Markdown".into());
                }
                let native_mode = mode(snapshot_mode_to_git_mode(&row.mode)?)?;
                if actual
                    .insert(
                        row.path,
                        (native_mode, {
                            let view = content
                                .blobs()
                                .get_blob_view(&session.content_read, &row.blob_id)?
                                .ok_or("Git code Blob absent")?;
                            native_body_identity(
                                content,
                                &mut session,
                                view.blob_index,
                                &mut verified_bodies,
                                &mut identity_memo,
                            )?
                        }),
                    )
                    .is_some()
                {
                    return Err("duplicate Git code Snapshot path".into());
                }
            }
            actual
        };
        if !docsets.contains_key(&mapping.document_set_tree_index) {
            let rows = documents.document_set_files_with_read_scope(
                mapping.document_set_tree_index,
                session.body_read,
                &session.content_read,
                &mut document_trees,
            )?;
            let rows = rows
                .into_iter()
                .map(|(path, mode, body)| {
                    (
                        path,
                        (
                            mode,
                            BodyIdentity {
                                size: body.len() as u64,
                                sha256: Sha256::digest(&body).into(),
                            },
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let size = rows.keys().map(|path| path.len() + 80).sum::<usize>();
            if docset_bytes.saturating_add(size) > 16 * 1024 * 1024 {
                docsets.clear();
                docset_bytes = 0;
            }
            docset_bytes += size;
            docsets.insert(mapping.document_set_tree_index, rows);
        }
        for (path, (mode, body)) in &docsets[&mapping.document_set_tree_index] {
            if actual.insert(path.clone(), (*mode, body.clone())).is_some() {
                return Err("Git DocSet/code Snapshot collision".into());
            }
        }
        #[cfg(test)]
        crate::binary_db::read_profile::note(
            "algo_mapping_identity_phase_ns",
            identity_phase.elapsed().as_nanos() as u64,
        );
        if actual != expected {
            return Err("Git mapping code Snapshot/DocSet differs from raw source tree".into());
        }
        if bindings.len() == 4096 {
            bindings.clear();
        }
        bindings.insert(binding);
        if docset_bytes > 16 * 1024 * 1024 {
            docsets.clear();
            docset_bytes = 0;
        }
    }
    #[cfg(test)]
    if profile.is_some_and(|(_, limit, _)| limit.is_some()) {
        session.verify_metadata_authority()?;
        tree_cache.verify_admission_stamps()?;
        document_trees.verify_admission_stamps()?;
        session.body_read.verify_admission_stamps()?;
        return Ok(());
    }

    let refs = history.refs()?;

    for checkpoint in history.checkpoints()? {
        let commits = mappings
            .iter()
            .filter(|r| r.generation_index == checkpoint.generation_index)
            .count();
        let names = refs
            .iter()
            .filter(|(_, _, r)| r.generation_index == checkpoint.generation_index)
            .map(|(_, name, _)| name)
            .collect::<BTreeSet<_>>();
        if checkpoint.next_commit as usize > commits
            || checkpoint.next_ref as usize > names.len()
            || (checkpoint.status == 1
                && (checkpoint.next_commit as usize != commits
                    || checkpoint.next_ref as usize != names.len()))
        {
            return Err("Git checkpoint cursor/owner closure differs".into());
        }
    }
    let tag_store = crate::tag_store::BinaryDbTagStore::new(content.clone());
    for (_, _, row) in &refs {
        if row.snapshot_index_plus1 != 0 {
            snapshot(
                content,
                &session.content_read,
                row.snapshot_index_plus1 - 1,
                reuse,
            )?;
        }
        if row.target_index_plus1 != 0 {
            match row.meta & 3 {
                0 => {
                    let read = &session.content_read;
                    let line = crate::line_binary_db::BinaryLineCodec::<1>::decode_record(
                        &read.read_record(
                            crate::line_binary_db::BinaryLineCodec::<1>::record_file(),
                            row.target_index_plus1 - 1,
                        )?,
                    )?;
                    crate::line_binary_db::binary_line_name_at(read, row.target_index_plus1 - 1)?;
                    if let Some(head) = line.head_snapshot_index() {
                        snapshot(content, &session.content_read, head, reuse)?;
                    }
                }
                1 => tag_store.validate_tag_ordinal(row.target_index_plus1 - 1)?,
                _ => return Err("Git symbolic ref unexpectedly binds native target".into()),
            }
        }
        let generation = generations
            .get(row.generation_index as usize)
            .ok_or("Git ref generation FK absent")?;
        if !history.generation_is_complete(row.generation_index)? {
            continue;
        }
        let repository = if generation.export {
            generation.target_repository_index_plus1 - 1
        } else {
            generation.source_repository_index
        };
        let (record, _) = session
            .get(repository, 1, &row.object_id)?
            .ok_or("completed Git ref raw object absent")?;
        if record.kind != row.object_kind {
            return Err("Git ref raw object kind differs".into());
        }
        if row.snapshot_index_plus1 != 0 {
            let mut oid = row.object_id;
            let mut seen = BTreeSet::new();
            loop {
                if !seen.insert(oid) {
                    return Err("cyclic bound Git tag".into());
                }
                let (record, body) = session
                    .get(repository, 1, &oid)?
                    .ok_or("bound Git ref object absent")?;
                if record.kind == GitObjectKind::Tag {
                    oid = children(record.kind, &body)?[0];
                    continue;
                }
                if record.kind != GitObjectKind::Commit
                    || !mappings.iter().any(|mapping| {
                        mapping.generation_index == row.generation_index
                            && mapping.object_id == oid
                            && mapping.snapshot_index.checked_add(1)
                                == Some(row.snapshot_index_plus1)
                    })
                {
                    return Err("Git bound ref commit/Snapshot mapping differs".into());
                }
                break;
            }
        }
        verify_closure(
            &mut session,
            AdmittedNamespace {
                repository,
                closure_owner: namespace_owners[&repository],
            },
            row.object_id,
            &mut verified,
            &mut verified_bodies,
            &mut raw_trees,
        )?;
    }
    session.verify_metadata_authority()?;
    tree_cache.verify_admission_stamps()?;
    document_trees.verify_admission_stamps()?;
    session.body_read.verify_admission_stamps()?;
    Ok(())
}
fn verify_closure(
    session: &mut GitObjectReadSession<'_>,
    namespace: AdmittedNamespace,
    root: [u8; 20],
    verified: &mut BTreeSet<(u32, [u8; 20])>,
    verified_bodies: &mut BTreeSet<u32>,
    raw_trees: &mut RawTreeReadCache,
) -> Result<(), String> {
    let AdmittedNamespace {
        repository,
        closure_owner,
    } = namespace;

    let mut pending = vec![root];
    while let Some(oid) = pending.pop() {
        if !verified.insert((closure_owner, oid)) {
            #[cfg(test)]
            crate::binary_db::read_profile::note("raw_closure_reuses", 0);
            continue;
        }
        let (record, body) = session
            .get(repository, 1, &oid)?
            .ok_or("Git raw closure incomplete")?;
        verified_bodies.insert(record.content_blob_index);

        #[cfg(test)]
        crate::binary_db::read_profile::note("raw_closure_validations", body.len() as u64);
        if record.kind == GitObjectKind::Tree {
            pending.extend(
                raw_trees
                    .remember(&record, &body)?
                    .iter()
                    .map(|(_, _, oid)| *oid),
            );
        } else {
            pending.extend(children(record.kind, &body)?);
        }
    }

    Ok(())
}
struct IdentityReadCaches<'a> {
    trees: &'a mut RawTreeReadCache,
    blobs: &'a mut BodyIdentityMemo,
}
fn tree_file_identities(
    session: &mut GitObjectReadSession<'_>,
    namespace: AdmittedNamespace,
    root: [u8; 20],
    verified: &BTreeSet<(u32, [u8; 20])>,
    verified_bodies: &mut BTreeSet<u32>,
    content: &LocalContentBinaryDb<1>,
    caches: IdentityReadCaches<'_>,
) -> Result<BTreeMap<String, (u16, BodyIdentity)>, String> {
    let IdentityReadCaches {
        trees: raw_trees,
        blobs: identity_memo,
    } = caches;
    let AdmittedNamespace {
        repository,
        closure_owner,
    } = namespace;
    let mut output = BTreeMap::new();

    let mut pending = vec![(String::new(), root, BTreeSet::new())];
    while let Some((prefix, oid, mut ancestors)) = pending.pop() {
        if !ancestors.insert(oid) {
            return Err("cyclic raw Git tree".into());
        }
        if !verified.contains(&(closure_owner, oid)) {
            return Err("Git tree body was not admitted".into());
        }
        let entries = raw_trees.entries(session, repository, &oid)?;
        for (mode, name, child) in entries.iter() {
            #[cfg(test)]
            crate::binary_db::read_profile::note("algo_raw_tree_edge_checks", 0);
            let (mode, name, child) = (*mode, name.clone(), *child);
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            validate_git_artifact_path(&path)?;
            if mode == 0o40000 {
                pending.push((path, child, ancestors.clone()));
            } else {
                #[cfg(test)]
                crate::binary_db::read_profile::note("algo_raw_leaf_fk_checks", 0);
                let record = session
                    .record(repository, 1, &child)?
                    .ok_or("Git tree Blob absent")?;
                if !verified.contains(&(closure_owner, child)) {
                    return Err("Git leaf body was not admitted".into());
                }
                let body = native_body_identity(
                    content,
                    session,
                    record.content_blob_index,
                    verified_bodies,
                    identity_memo,
                )?;
                if record.kind != GitObjectKind::Blob || output.insert(path, (mode, body)).is_some()
                {
                    return Err("Git tree leaf object/path differs".into());
                }
            }
        }
    }

    Ok(output)
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
fn mapping_identity(
    history: &GitHistoryStore,
    session: &GitObjectReadSession<'_>,
    index: u32,
    reuse: bool,
) -> Result<Vec<u8>, String> {
    if reuse {
        history
            .identity_raw_with_read(session.git_read(), index)
            .map_err(Into::into)
    } else {
        history.identity_raw(index).map_err(Into::into)
    }
}

fn snapshot(
    content: &LocalContentBinaryDb<1>,
    read: &BinaryDbReadTxn<'_, crate::binary_db::LocalBinaryDbFs>,
    index: u32,
    reuse: bool,
) -> Result<BinarySnapshotView, String> {
    if reuse {
        content
            .snapshots()
            .snapshot_view_at(read, index)
            .map_err(Into::into)
    } else {
        let read = BinaryDbReadTxn::new(content.db());
        content
            .snapshots()
            .snapshot_view_at(&read, index)
            .map_err(Into::into)
    }
}

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
pub(crate) fn tree_entries(body: &[u8]) -> Result<Vec<(u16, String, [u8; 20])>, String> {
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

fn snapshot_mode_to_git_mode(mode: &str) -> Result<&'static str, String> {
    let normalized = mode.trim().trim_start_matches("0o");
    let bits = u32::from_str_radix(normalized, 8)
        .map_err(|_| format!("Invalid AIT Snapshot mode {mode:?}."))?;
    if bits & 0o170000 == 0o120000 {
        return Ok("120000");
    }
    if bits & 0o170000 != 0 && bits & 0o170000 != 0o100000 {
        return Err(format!(
            "Unsupported AIT Snapshot file type in mode {mode:?}."
        ));
    }
    Ok(if bits & 0o111 != 0 {
        "100755"
    } else {
        "100644"
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
