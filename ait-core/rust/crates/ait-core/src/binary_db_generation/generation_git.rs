//! Preserve the existing optional Git root with its complete content FK closure.
use super::GenerationResult;
use crate::binary_db::{
    AuthorityId, LocalBinaryDbFs, LocalStateScope, REPOSITORY_BINARY_DB_BIN_PATHS,
    REPOSITORY_BINARY_DB_INDEX_PATHS,
};
use crate::content_binary_db::LocalContentBinaryDb;
use crate::git_binary_db::{
    GitHistoryStore, GitInteropBundle, LocalGitObjectStore, RawGitDocumentStore,
    GIT_INTEROP_BUNDLE_PATH, GIT_OBJECT_STORE_BIN_PATHS, GIT_OBJECT_STORE_INDEX_PATHS,
};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) const GIT_DIRECTORY: &str = ".ait/git-interop/v1/binary-v0";
pub(super) const GIT_PREFIX: &str = ".ait/git-interop/v1/binary-v0/";

pub(super) fn git_repo(root: &Path) -> GenerationResult<&Path> {
    if !root.ends_with(GIT_DIRECTORY) {
        return Err("invalid optional Git root".into());
    }
    root.ancestors()
        .nth(4)
        .ok_or_else(|| "Git root has no repository".into())
}
pub(super) fn git_exists(root: &Path) -> bool {
    git_repo(root).is_ok_and(GitInteropBundle::exists)
}
pub(super) fn git_file(relative: &str) -> bool {
    relative == GIT_INTEROP_BUNDLE_PATH || relative.starts_with(GIT_PREFIX)
}
pub(super) fn git_database(root: &Path, pack_repo: &Path) -> GenerationResult<LocalBinaryDbFs> {
    let repo = git_repo(root)?;
    if repo.join(GIT_INTEROP_BUNDLE_PATH).exists() {
        return Ok(GitInteropBundle::open(repo, false)?
            .database(pack_repo, AuthorityId::new("generation-git")));
    }
    Ok(LocalBinaryDbFs::new(
        root.to_path_buf(),
        pack_repo.to_path_buf(),
        AuthorityId::new("generation-git"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(GIT_OBJECT_STORE_BIN_PATHS)
    .with_declared_index_paths(GIT_OBJECT_STORE_INDEX_PATHS))
}

pub(super) fn declared_git_leaf(name: &str) -> bool {
    !name.contains('/')
        && (GIT_OBJECT_STORE_BIN_PATHS.contains(&name)
            || GIT_OBJECT_STORE_INDEX_PATHS.contains(&name))
}

pub(super) fn git_inputs(root: &Path) -> GenerationResult<Vec<(PathBuf, String)>> {
    let archive = git_repo(root)
        .ok()
        .map(|repo| repo.join(GIT_INTEROP_BUNDLE_PATH));
    if let Some((archive, m)) =
        archive.and_then(|path| fs::symlink_metadata(&path).ok().map(|m| (path, m)))
    {
        if !m.file_type().is_file() {
            return Err("Git bundle must be a direct file".into());
        }
        // Native admission resolves any interrupted retirement against this archive.
        return Ok(vec![(archive, GIT_INTEROP_BUNDLE_PATH.to_string())]);
    }
    let metadata = match fs::symlink_metadata(root) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("Git authority must be a direct directory".into());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Git authority path is not UTF-8")?;
        if name == ".locks" && kind.is_dir() {
            continue;
        }
        if !kind.is_file() || !declared_git_leaf(&name) {
            return Err(format!("undeclared Git authority entry: {name:?}"));
        }
        files.push((entry.path(), format!("{GIT_PREFIX}{name}")));
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(files)
}

pub(super) fn raw_revision_slots_pinned(authority: &Path) -> GenerationResult<bool> {
    let rows = match fs::read(authority.join("plan_revision.bin")) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if rows.len() < 4 || rows[..4] != 1u32.to_le_bytes() || (rows.len() - 4) % 56 != 0 {
        return Err("cannot capture invalid PlanRevision prefix".into());
    }
    Ok(rows[4..].chunks_exact(56).any(|row| row[0] & 32 != 0))
}

pub(super) fn validate_git_root(
    git_root: &Path,
    content_root: &Path,
    pack_repo_root: &Path,
    detached: bool,
) -> GenerationResult<()> {
    if git_inputs(git_root)?.is_empty() {
        return Ok(());
    }
    let content_db = LocalBinaryDbFs::new(
        content_root.to_path_buf(),
        pack_repo_root.to_path_buf(),
        AuthorityId::new("generation-git-content"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
    let content_db = if detached {
        content_db.for_detached_generation_without_locks()
    } else {
        content_db
    };
    let content = LocalContentBinaryDb::<1>::from_db_with_roots(
        content_db,
        pack_repo_root.to_path_buf(),
        pack_repo_root.to_path_buf(),
    );
    let git_db = git_database(git_root, pack_repo_root)?;
    let git_db = if detached {
        git_db.for_detached_generation_without_locks()
    } else {
        git_db
    };
    let objects = LocalGitObjectStore::open_for_staged_repack(git_db.clone(), content.clone())?;
    let mut session = crate::content_binary_db::BinaryBlobReadSession::for_git_admission();
    let read = crate::binary_db::BinaryDbReadTxn::new(&git_db);
    let mut object_read = objects.read_session(&mut session);
    for index in 0..read.record_count(crate::git_binary_db::git_object_file())? {
        if index.is_multiple_of(128) {
            object_read.body_read.clear_logical_values();
        }
        let row = crate::git_binary_db::GitObjectRecord::decode(
            &read.read_record(crate::git_binary_db::git_object_file(), index)?,
        )?;
        object_read
            .get(row.repository_index, 1, &row.object_id)?
            .ok_or("generation Git object disappeared")?;
    }
    drop(object_read);
    drop(read);
    let history = GitHistoryStore::open(git_db.clone())?;
    let documents = RawGitDocumentStore::open(git_db, content.clone())?;
    crate::git_binary_db::validate_git_content_closure_with_session(
        &objects,
        &history,
        &documents,
        &content,
        &mut session,
    )
}
