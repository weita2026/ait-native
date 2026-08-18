use ait_server_core::foundation::operational_binary_v0::{
    validate_namespace, REPOSITORY_LIFECYCLE_ACTIVE, REPOSITORY_LIFECYCLE_PURGED,
    REPOSITORY_LIFECYCLE_RETIRING,
};
use ait_server_core::foundation::remote_binary_db::{
    sync_filesystem_directory, sync_filesystem_file, BinaryDbError, StoreResult,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub(crate) const REMOTE_EXPORT_SCHEMA: &str = "ait.remote-export.v1";
pub(crate) const REMOTE_EXPORT_STATE_COMPLETE: &str = "complete";
pub(crate) const REMOTE_AUTHORITY_FILE_MEDIA_TYPE: &str =
    "application/vnd.ait.remote-authority-file.v1";

const PURGE_JOURNAL_SCHEMA: &str = "ait.server.repository-purge-journal.v1";
const PURGE_JOURNAL_FILE: &str = ".repository-purge.journal.json";
const PURGE_JOURNAL_TEMP: &str = ".repository-purge.journal.tmp";
const PURGE_PATCHSET_BEFORE: &str = ".repository-purge.patchset.before";
const PURGE_WORKER_JOB_BEFORE: &str = ".repository-purge.worker-job.before";
const PURGE_PATCHSET_RESTORE: &str = ".repository-purge.patchset.restore";
const PURGE_WORKER_JOB_RESTORE: &str = ".repository-purge.worker-job.restore";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteExportFile {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteExportManifest {
    pub(crate) schema: String,
    pub(crate) state: String,
    pub(crate) repo_name: String,
    pub(crate) namespace: String,
    pub(crate) exported_at_s: u64,
    pub(crate) files: Vec<RemoteExportFile>,
}

impl RemoteExportManifest {
    pub(crate) fn validate(&self) -> StoreResult<()> {
        if self.schema != REMOTE_EXPORT_SCHEMA
            || self.state != REMOTE_EXPORT_STATE_COMPLETE
            || self.repo_name.is_empty()
            || self.exported_at_s == 0
            || self.files.is_empty()
        {
            return Err(invalid("remote export manifest envelope is invalid"));
        }
        let namespace = namespace_ascii(&self.namespace)?;
        validate_namespace(namespace)?;
        let mut prior: Option<&str> = None;
        for file in &self.files {
            canonical_relative_path(&file.path)?;
            if file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(invalid(format!(
                    "remote export file {} has an invalid SHA-256",
                    file.path
                )));
            }
            if prior.is_some_and(|value| value >= file.path.as_str()) {
                return Err(invalid(
                    "remote export files must be strictly path-sorted and unique",
                ));
            }
            prior = Some(&file.path);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PurgeJournal {
    schema: String,
    patchset: JournalFile,
    worker_job: JournalFile,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalFile {
    size: u64,
    sha256: String,
}

pub(crate) fn namespace_ascii(namespace: &str) -> StoreResult<[u8; 2]> {
    if !namespace.is_ascii() || namespace.len() > 2 {
        return Err(invalid(
            "Repository namespace must contain zero, one, or two ASCII bytes",
        ));
    }
    let mut bytes = [0_u8; 2];
    bytes[..namespace.len()].copy_from_slice(namespace.as_bytes());
    validate_namespace(bytes)?;
    Ok(bytes)
}

pub(crate) fn collect_export_files(authority_root: &Path) -> StoreResult<Vec<RemoteExportFile>> {
    require_real_directory(authority_root)?;
    let mut paths = Vec::new();
    collect_paths(authority_root, authority_root, &mut paths)?;
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let relative = relative_string(authority_root, &path)?;
        let (size, sha256) = sha256_file(&path)?;
        files.push(RemoteExportFile {
            path: relative,
            size,
            sha256,
        });
    }
    Ok(files)
}

pub(crate) fn validate_staged_archive(
    data_root: &Path,
    manifest: &RemoteExportManifest,
) -> StoreResult<()> {
    manifest.validate()?;
    let actual = collect_export_files(data_root)?;
    if actual != manifest.files {
        return Err(invalid(
            "staged restore authority does not exactly match its export manifest",
        ));
    }
    Ok(())
}

pub(crate) fn read_export_file(
    authority_root: &Path,
    expected: &RemoteExportFile,
) -> StoreResult<Vec<u8>> {
    let relative = canonical_relative_path(&expected.path)?;
    let path = authority_root.join(relative);
    let bytes = read_regular_file(&path)?;
    let size = u64::try_from(bytes.len())
        .map_err(|_| invalid("remote authority file length exceeds u64"))?;
    if size != expected.size || sha256_bytes(&bytes) != expected.sha256 {
        return Err(corrupt(format!(
            "remote authority file {} changed after export manifest creation",
            expected.path
        )));
    }
    Ok(bytes)
}

pub(crate) fn write_staged_file(
    data_root: &Path,
    expected: &RemoteExportFile,
    bytes: &[u8],
) -> StoreResult<()> {
    let actual_size =
        u64::try_from(bytes.len()).map_err(|_| invalid("restore upload length exceeds u64"))?;
    if actual_size != expected.size || sha256_bytes(bytes) != expected.sha256 {
        return Err(invalid(format!(
            "restore upload {} does not match its declared size and SHA-256",
            expected.path
        )));
    }
    require_real_directory(data_root)?;
    let relative = canonical_relative_path(&expected.path)?;
    let target = data_root.join(relative);
    if let Some(parent) = target.parent() {
        ensure_real_directory_tree(data_root, parent)?;
    }
    let temporary = target.with_extension(format!(
        "{}upload",
        target
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}."))
            .unwrap_or_default()
    ));
    if temporary.exists() {
        remove_regular_file(&temporary)?;
    }
    write_new_disposable(&temporary, bytes)?;
    if target.exists() {
        remove_regular_file(&target)?;
    }
    fs::rename(&temporary, &target).map_err(|error| {
        BinaryDbError::io(
            format!(
                "publish staged restore file {} as {}",
                temporary.display(),
                target.display()
            ),
            error,
        )
    })
}

pub(crate) fn copy_manifest_files(
    data_root: &Path,
    authority_root: &Path,
    manifest: &RemoteExportManifest,
) -> StoreResult<()> {
    require_real_directory(data_root)?;
    require_real_directory(authority_root)?;
    for file in &manifest.files {
        let relative = canonical_relative_path(&file.path)?;
        let source = data_root.join(&relative);
        let target = authority_root.join(&relative);
        if let Some(parent) = target.parent() {
            ensure_real_directory_tree(authority_root, parent)?;
        }
        let bytes = read_regular_file(&source)?;
        if u64::try_from(bytes.len()).ok() != Some(file.size) || sha256_bytes(&bytes) != file.sha256
        {
            return Err(corrupt(format!(
                "staged restore file {} changed during activation",
                file.path
            )));
        }
        write_new_sync(&target, &bytes)?;
    }
    sync_tree_directories(authority_root)
}

pub(crate) fn canonical_relative_path(value: &str) -> StoreResult<PathBuf> {
    if value.is_empty()
        || value.contains('\\')
        || value.contains('\0')
        || value.starts_with('/')
        || value.ends_with('/')
    {
        return Err(invalid("remote authority file path is not canonical"));
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
        || path
            .components()
            .any(|component| component.as_os_str().to_str().is_none())
    {
        return Err(invalid("remote authority file path is not canonical"));
    }
    let normalized = path
        .components()
        .map(|component| component.as_os_str().to_str().expect("validated UTF-8"))
        .collect::<Vec<_>>()
        .join("/");
    if normalized != value {
        return Err(invalid("remote authority file path is not canonical"));
    }
    Ok(path.to_path_buf())
}

pub(crate) fn prepare_purge_journal(authority_root: &Path) -> StoreResult<()> {
    require_real_directory(authority_root)?;
    for name in purge_journal_names() {
        if authority_root.join(name).exists() {
            return Err(corrupt(
                "Repository purge journal was not recovered before a new purge",
            ));
        }
    }
    let patchset = authority_root.join("patchset.bin");
    let worker_job = authority_root.join("worker_job.bin");
    let patchset_before = authority_root.join(PURGE_PATCHSET_BEFORE);
    let worker_job_before = authority_root.join(PURGE_WORKER_JOB_BEFORE);
    copy_regular_file_sync(&patchset, &patchset_before)?;
    copy_regular_file_sync(&worker_job, &worker_job_before)?;
    let (patchset_size, patchset_sha256) = sha256_file(&patchset_before)?;
    let (worker_job_size, worker_job_sha256) = sha256_file(&worker_job_before)?;
    let journal = PurgeJournal {
        schema: PURGE_JOURNAL_SCHEMA.to_string(),
        patchset: JournalFile {
            size: patchset_size,
            sha256: patchset_sha256,
        },
        worker_job: JournalFile {
            size: worker_job_size,
            sha256: worker_job_sha256,
        },
    };
    let mut bytes = serde_json::to_vec_pretty(&journal)
        .map_err(|error| BinaryDbError::other(format!("encode purge journal: {error}")))?;
    bytes.push(b'\n');
    write_new_sync(&authority_root.join(PURGE_JOURNAL_TEMP), &bytes)?;
    fs::rename(
        authority_root.join(PURGE_JOURNAL_TEMP),
        authority_root.join(PURGE_JOURNAL_FILE),
    )
    .map_err(|error| {
        BinaryDbError::io(
            format!(
                "publish Repository purge journal in {}",
                authority_root.display()
            ),
            error,
        )
    })?;
    sync_directory(authority_root)
}

pub(crate) fn recover_purge_journal(authority_root: &Path, lifecycle_kind: u8) -> StoreResult<()> {
    require_real_directory(authority_root)?;
    let journal_path = authority_root.join(PURGE_JOURNAL_FILE);
    if !journal_path.exists() {
        for name in [
            PURGE_JOURNAL_TEMP,
            PURGE_PATCHSET_BEFORE,
            PURGE_WORKER_JOB_BEFORE,
            PURGE_PATCHSET_RESTORE,
            PURGE_WORKER_JOB_RESTORE,
        ] {
            remove_regular_file_if_exists(&authority_root.join(name))?;
        }
        return sync_directory(authority_root);
    }
    let journal: PurgeJournal = serde_json::from_slice(&read_regular_file(&journal_path)?)
        .map_err(|error| corrupt(format!("decode Repository purge journal: {error}")))?;
    if journal.schema != PURGE_JOURNAL_SCHEMA {
        return Err(corrupt("Repository purge journal schema is unsupported"));
    }
    validate_before_image(
        &authority_root.join(PURGE_PATCHSET_BEFORE),
        &journal.patchset,
    )?;
    validate_before_image(
        &authority_root.join(PURGE_WORKER_JOB_BEFORE),
        &journal.worker_job,
    )?;
    match lifecycle_kind {
        REPOSITORY_LIFECYCLE_RETIRING => {
            atomic_copy_replace(
                &authority_root.join(PURGE_PATCHSET_BEFORE),
                &authority_root.join("patchset.bin"),
                &authority_root.join(PURGE_PATCHSET_RESTORE),
            )?;
            atomic_copy_replace(
                &authority_root.join(PURGE_WORKER_JOB_BEFORE),
                &authority_root.join("worker_job.bin"),
                &authority_root.join(PURGE_WORKER_JOB_RESTORE),
            )?;
            finish_purge_journal(authority_root)
        }
        REPOSITORY_LIFECYCLE_PURGED => finish_purge_journal(authority_root),
        REPOSITORY_LIFECYCLE_ACTIVE => Err(corrupt(
            "active Repository has an impossible purge recovery journal",
        )),
        _ => Err(corrupt("Repository lifecycle kind is unsupported")),
    }
}

pub(crate) fn finish_purge_journal(authority_root: &Path) -> StoreResult<()> {
    for name in purge_journal_names() {
        remove_regular_file_if_exists(&authority_root.join(name))?;
    }
    sync_directory(authority_root)
}

pub(crate) fn restore_staging_parent(generation_root: &Path) -> StoreResult<PathBuf> {
    let parent = generation_root
        .parent()
        .ok_or_else(|| invalid("activated generation has no parent"))?;
    let identity = sha256_bytes(generation_root.as_os_str().as_encoded_bytes());
    let path = parent.join(format!(".repository-restores-{}", &identity[..16]));
    ensure_real_directory(&path)?;
    Ok(path)
}

pub(crate) fn create_restore_session_directory(
    staging_parent: &Path,
    token: &str,
) -> StoreResult<PathBuf> {
    require_real_directory(staging_parent)?;
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("restore session token is invalid"));
    }
    let session = staging_parent.join(token);
    fs::create_dir(&session).map_err(|error| {
        BinaryDbError::io(
            format!("create restore session directory {}", session.display()),
            error,
        )
    })?;
    let data = session.join("data");
    fs::create_dir(&data).map_err(|error| {
        BinaryDbError::io(
            format!("create restore data directory {}", data.display()),
            error,
        )
    })?;
    sync_directory(&session)?;
    sync_directory(staging_parent)?;
    Ok(session)
}

fn collect_paths(root: &Path, current: &Path, output: &mut Vec<PathBuf>) -> StoreResult<()> {
    for entry in fs::read_dir(current)
        .map_err(|error| BinaryDbError::io(format!("read {}", current.display()), error))?
    {
        let entry = entry.map_err(|error| {
            BinaryDbError::io(format!("read entry in {}", current.display()), error)
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| BinaryDbError::io(format!("inspect {}", path.display()), error))?;
        if metadata.file_type().is_symlink() {
            return Err(invalid(format!(
                "remote authority contains symlink {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            collect_paths(root, &path, output)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(invalid(format!(
                "remote authority path {} is not a regular file",
                path.display()
            )));
        }
        #[cfg(unix)]
        if metadata.nlink() != 1 {
            return Err(invalid(format!(
                "remote authority file {} is shared through a hard link",
                path.display()
            )));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("remote authority filename is not UTF-8"))?;
        if name.ends_with(".lock") {
            continue;
        }
        if name.ends_with(".journal")
            || name.ends_with(".rewrite")
            || name.ends_with(".rebuild")
            || purge_journal_names().contains(&name.as_str())
        {
            return Err(corrupt(format!(
                "remote authority has unrecovered transient file {}",
                relative_string(root, &path)?
            )));
        }
        output.push(path);
    }
    Ok(())
}

fn relative_string(root: &Path, path: &Path) -> StoreResult<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid("authority path escaped its root"))?;
    let value = relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| invalid("authority path is not UTF-8"))
        })
        .collect::<StoreResult<Vec<_>>>()?
        .join("/");
    canonical_relative_path(&value)?;
    Ok(value)
}

fn validate_before_image(path: &Path, expected: &JournalFile) -> StoreResult<()> {
    let (size, sha256) = sha256_file(path)?;
    if size != expected.size || sha256 != expected.sha256 {
        return Err(corrupt(format!(
            "Repository purge before-image {} is corrupt",
            path.display()
        )));
    }
    Ok(())
}

fn atomic_copy_replace(source: &Path, target: &Path, temporary: &Path) -> StoreResult<()> {
    remove_regular_file_if_exists(temporary)?;
    copy_regular_file_sync(source, temporary)?;
    reject_symlink_or_non_file_if_present(target)?;
    fs::rename(temporary, target).map_err(|error| {
        BinaryDbError::io(
            format!(
                "restore Repository purge before-image {} as {}",
                source.display(),
                target.display()
            ),
            error,
        )
    })?;
    sync_directory(
        target
            .parent()
            .ok_or_else(|| invalid("purge target has no parent"))?,
    )
}

fn purge_journal_names() -> [&'static str; 6] {
    [
        PURGE_JOURNAL_FILE,
        PURGE_JOURNAL_TEMP,
        PURGE_PATCHSET_BEFORE,
        PURGE_WORKER_JOB_BEFORE,
        PURGE_PATCHSET_RESTORE,
        PURGE_WORKER_JOB_RESTORE,
    ]
}

fn sha256_file(path: &Path) -> StoreResult<(u64, String)> {
    reject_symlink_or_non_file_if_present(path)?;
    let mut file = File::open(path)
        .map_err(|error| BinaryDbError::io(format!("open {}", path.display()), error))?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| BinaryDbError::io(format!("read {}", path.display()), error))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        size = size
            .checked_add(u64::try_from(count).expect("buffer length fits u64"))
            .ok_or_else(|| invalid("authority file size exceeds u64"))?;
    }
    Ok((size, format!("{:x}", digest.finalize())))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_regular_file(path: &Path) -> StoreResult<Vec<u8>> {
    reject_symlink_or_non_file_if_present(path)?;
    fs::read(path).map_err(|error| BinaryDbError::io(format!("read {}", path.display()), error))
}

fn copy_regular_file_sync(source: &Path, target: &Path) -> StoreResult<()> {
    reject_symlink_or_non_file_if_present(source)?;
    reject_symlink_or_non_file_if_present(target)?;
    fs::copy(source, target).map_err(|error| {
        BinaryDbError::io(
            format!("copy {} to {}", source.display(), target.display()),
            error,
        )
    })?;
    sync_filesystem_file(target)
        .map_err(|error| BinaryDbError::io(format!("sync {}", target.display()), error))
}

fn write_new_sync(path: &Path, bytes: &[u8]) -> StoreResult<()> {
    reject_symlink_or_non_file_if_present(path)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| BinaryDbError::io(format!("create {}", path.display()), error))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| BinaryDbError::io(format!("write {}", path.display()), error))
}

fn write_new_disposable(path: &Path, bytes: &[u8]) -> StoreResult<()> {
    // Restore sessions are memory-local and unpublished until commit. The
    // commit path revalidates and durably copies every file before appending
    // the Repository record, so syncing disposable upload staging here only
    // duplicates authority durability work and cannot make a session
    // recoverable after a server restart.
    reject_symlink_or_non_file_if_present(path)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| BinaryDbError::io(format!("create {}", path.display()), error))?;
    file.write_all(bytes)
        .map_err(|error| BinaryDbError::io(format!("write {}", path.display()), error))
}

fn ensure_real_directory(path: &Path) -> StoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(invalid(format!(
            "{} is not a real directory",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| {
                BinaryDbError::io(format!("create directory {}", path.display()), error)
            })?;
            require_real_directory(path)
        }
        Err(error) => Err(BinaryDbError::io(
            format!("inspect directory {}", path.display()),
            error,
        )),
    }
}

fn ensure_real_directory_tree(root: &Path, path: &Path) -> StoreResult<()> {
    require_real_directory(root)?;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid("directory creation escaped its root"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(invalid("directory path is not canonical"));
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(invalid(format!(
                    "{} is not a real directory",
                    current.display()
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    BinaryDbError::io(format!("create directory {}", current.display()), error)
                })?;
                sync_directory(
                    current
                        .parent()
                        .ok_or_else(|| invalid("created directory has no parent"))?,
                )?;
            }
            Err(error) => {
                return Err(BinaryDbError::io(
                    format!("inspect directory {}", current.display()),
                    error,
                ))
            }
        }
    }
    Ok(())
}

fn require_real_directory(path: &Path) -> StoreResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| BinaryDbError::io(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid(format!(
            "{} is not a real directory",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink_or_non_file_if_present(path: &Path) -> StoreResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(BinaryDbError::io(
                format!("inspect {}", path.display()),
                error,
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid(format!("{} is not a regular file", path.display())));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(invalid(format!(
            "{} is shared through a hard link",
            path.display()
        )));
    }
    Ok(())
}

fn remove_regular_file(path: &Path) -> StoreResult<()> {
    reject_symlink_or_non_file_if_present(path)?;
    fs::remove_file(path)
        .map_err(|error| BinaryDbError::io(format!("remove {}", path.display()), error))
}

fn remove_regular_file_if_exists(path: &Path) -> StoreResult<()> {
    if path.exists() {
        remove_regular_file(path)?;
    }
    Ok(())
}

fn sync_tree_directories(current: &Path) -> StoreResult<()> {
    let mut children = BTreeSet::new();
    for entry in fs::read_dir(current)
        .map_err(|error| BinaryDbError::io(format!("read {}", current.display()), error))?
    {
        let entry = entry.map_err(|error| {
            BinaryDbError::io(format!("read entry in {}", current.display()), error)
        })?;
        let metadata = entry.metadata().map_err(|error| {
            BinaryDbError::io(format!("inspect {}", entry.path().display()), error)
        })?;
        if metadata.is_dir() {
            children.insert(entry.path());
        }
    }
    for child in children {
        sync_tree_directories(&child)?;
    }
    sync_directory(current)
}

fn sync_directory(path: &Path) -> StoreResult<()> {
    sync_filesystem_directory(path)
        .map_err(|error| BinaryDbError::io(format!("sync directory {}", path.display()), error))
}

fn invalid(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::invalid_domain_data(message)
}

fn corrupt(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::corruption(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ait-server-repository-retirement-{label}-{}-{}",
                std::process::id(),
                TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn purge_journal_rolls_back_retiring_and_rolls_forward_purged() {
        let directory = TestDirectory::new("purge-journal");
        let patchset_before = 1_u32.to_le_bytes();
        let worker_before = [1_u8, 2, 3, 4];
        fs::write(directory.0.join("patchset.bin"), patchset_before).unwrap();
        fs::write(directory.0.join("worker_job.bin"), worker_before).unwrap();

        prepare_purge_journal(&directory.0).unwrap();
        fs::write(directory.0.join("patchset.bin"), b"mutated-patchset").unwrap();
        fs::write(directory.0.join("worker_job.bin"), b"mutated-worker").unwrap();
        recover_purge_journal(&directory.0, REPOSITORY_LIFECYCLE_RETIRING).unwrap();
        assert_eq!(
            fs::read(directory.0.join("patchset.bin")).unwrap(),
            patchset_before
        );
        assert_eq!(
            fs::read(directory.0.join("worker_job.bin")).unwrap(),
            worker_before
        );
        assert!(purge_journal_names()
            .iter()
            .all(|name| !directory.0.join(name).exists()));

        prepare_purge_journal(&directory.0).unwrap();
        fs::write(directory.0.join("patchset.bin"), b"purged-patchset").unwrap();
        fs::write(directory.0.join("worker_job.bin"), b"purged-worker").unwrap();
        recover_purge_journal(&directory.0, REPOSITORY_LIFECYCLE_PURGED).unwrap();
        assert_eq!(
            fs::read(directory.0.join("patchset.bin")).unwrap(),
            b"purged-patchset"
        );
        assert_eq!(
            fs::read(directory.0.join("worker_job.bin")).unwrap(),
            b"purged-worker"
        );
        assert!(purge_journal_names()
            .iter()
            .all(|name| !directory.0.join(name).exists()));
    }

    #[test]
    fn manifest_paths_are_sorted_canonical_and_transient_files_fail_closed() {
        let directory = TestDirectory::new("manifest");
        fs::create_dir(directory.0.join(".ait")).unwrap();
        fs::write(directory.0.join(".ait").join("object.bin"), b"object").unwrap();
        fs::write(directory.0.join("worker-queue.lock"), b"runtime lock").unwrap();
        fs::write(directory.0.join("patchset.bin"), 1_u32.to_le_bytes()).unwrap();
        let files = collect_export_files(&directory.0).unwrap();
        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            [".ait/object.bin", "patchset.bin"]
        );
        for invalid in ["", "/absolute", "../escape", "a//b", "a/./b", "a\\b"] {
            assert!(canonical_relative_path(invalid).is_err(), "{invalid}");
        }
        fs::write(
            directory.0.join("server-workflow.write.journal"),
            b"pending",
        )
        .unwrap();
        assert!(collect_export_files(&directory.0).is_err());
    }
}
