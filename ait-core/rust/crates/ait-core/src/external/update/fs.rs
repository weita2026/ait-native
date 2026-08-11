use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::external::lockfile::{ExternalLockCodec, ExternalLockfile, TomlExternalLockCodec};
use crate::external::manifest::{
    ExternalManifest, ExternalManifestCodec, TomlExternalManifestCodec,
};
use crate::external::update::{ExternalPreparedUpdate, ExternalUpdateStore};
use crate::external::{ExternalError, ExternalResult};

const EXTERNAL_MANIFEST_FILE: &str = "ait-external.toml";
const EXTERNAL_LOCK_FILE: &str = "ait-external.lock";

#[derive(Debug, Clone)]
pub struct FilesystemExternalUpdateStore<M = TomlExternalManifestCodec, L = TomlExternalLockCodec> {
    repo_root: PathBuf,
    manifest_codec: M,
    lock_codec: L,
}

impl FilesystemExternalUpdateStore {
    pub fn for_repo_root(repo_root: impl Into<PathBuf>) -> ExternalResult<Self> {
        Self::new(repo_root, TomlExternalManifestCodec, TomlExternalLockCodec)
    }
}

impl<M, L> FilesystemExternalUpdateStore<M, L>
where
    M: ExternalManifestCodec,
    L: ExternalLockCodec,
{
    pub fn new(
        repo_root: impl Into<PathBuf>,
        manifest_codec: M,
        lock_codec: L,
    ) -> ExternalResult<Self> {
        let repo_root = repo_root.into();
        if repo_root.as_os_str().is_empty() {
            return Err(ExternalError::with_code(
                "external_update_repo_root",
                "external update repo root must not be empty",
            ));
        }
        Ok(Self {
            repo_root,
            manifest_codec,
            lock_codec,
        })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn manifest_path(&self) -> PathBuf {
        self.repo_root.join(EXTERNAL_MANIFEST_FILE)
    }

    fn lockfile_path(&self) -> PathBuf {
        self.repo_root.join(EXTERNAL_LOCK_FILE)
    }
}

impl<M, L> ExternalUpdateStore for FilesystemExternalUpdateStore<M, L>
where
    M: ExternalManifestCodec,
    L: ExternalLockCodec,
{
    type Prepared = PreparedFilesystemExternalUpdate;

    fn read_manifest(&self) -> ExternalResult<ExternalManifest> {
        let path = self.manifest_path();
        match fs::read(&path) {
            Ok(bytes) => self.manifest_codec.parse_manifest(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(ExternalManifest {
                externals: Vec::new(),
            }),
            Err(err) => Err(ExternalError::with_code(
                "external_update_manifest_read",
                format!("failed to read {}: {err}", path.display()),
            )),
        }
    }

    fn read_lockfile(&self) -> ExternalResult<Option<ExternalLockfile>> {
        let path = self.lockfile_path();
        match fs::read(&path) {
            Ok(bytes) => self.lock_codec.parse_lockfile(&bytes).map(Some),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(ExternalError::with_code(
                "external_update_lockfile_read",
                format!("failed to read {}: {err}", path.display()),
            )),
        }
    }

    fn prepare_update(
        &self,
        manifest: &ExternalManifest,
        lockfile: &ExternalLockfile,
    ) -> ExternalResult<Self::Prepared> {
        let manifest_bytes = self.manifest_codec.render_manifest(manifest)?;
        let lockfile_bytes = self.lock_codec.render_lockfile(lockfile)?;
        Ok(PreparedFilesystemExternalUpdate {
            manifest_path: self.manifest_path(),
            lockfile_path: self.lockfile_path(),
            manifest_bytes,
            lockfile_bytes,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PreparedFilesystemExternalUpdate {
    manifest_path: PathBuf,
    lockfile_path: PathBuf,
    manifest_bytes: Vec<u8>,
    lockfile_bytes: Vec<u8>,
}

impl ExternalPreparedUpdate for PreparedFilesystemExternalUpdate {
    fn commit(self) -> ExternalResult<()> {
        write_prepared_file(&self.manifest_path, &self.manifest_bytes, "manifest")?;
        write_prepared_file(&self.lockfile_path, &self.lockfile_bytes, "lockfile")
    }
}

fn write_prepared_file(path: &Path, bytes: &[u8], label: &str) -> ExternalResult<()> {
    fs::write(path, bytes).map_err(|err| {
        ExternalError::with_code(
            "external_update_write",
            format!("failed to write external {label} {}: {err}", path.display()),
        )
    })
}
