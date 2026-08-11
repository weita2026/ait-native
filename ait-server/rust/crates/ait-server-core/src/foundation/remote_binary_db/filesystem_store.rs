use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerBinaryDbFilesystemStore;

impl ServerBinaryDbFileStore for ServerBinaryDbFilesystemStore {
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read_bytes(&self, path: &Path) -> StoreResult<Vec<u8>> {
        fs::read(path).map_err(|err| {
            BinaryDbError::io(format!("read Binary DB file {}", path.display()), err)
        })
    }

    fn read_to_string(&self, path: &Path) -> StoreResult<String> {
        fs::read_to_string(path).map_err(|err| {
            BinaryDbError::io(format!("read Binary DB text file {}", path.display()), err)
        })
    }
}

impl ServerBinaryDbByteStore for ServerBinaryDbFilesystemStore {
    fn read_range(&self, path: &Path, offset: u64, len: u32) -> StoreResult<Vec<u8>> {
        let mut file = File::open(path).map_err(|err| {
            BinaryDbError::io(format!("open Binary DB file {}", path.display()), err)
        })?;
        file.seek(SeekFrom::Start(offset)).map_err(|err| {
            BinaryDbError::io(format!("seek Binary DB file {}", path.display()), err)
        })?;
        let len = usize::try_from(len).map_err(|_| {
            BinaryDbError::invalid_domain_data("Binary DB read range exceeds usize")
        })?;
        let mut bytes = vec![0_u8; len];
        file.read_exact(&mut bytes).map_err(|err| {
            BinaryDbError::io(format!("read Binary DB range {}", path.display()), err)
        })?;
        Ok(bytes)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        match fs::metadata(path) {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(BinaryDbError::io(
                format!("read Binary DB metadata {}", path.display()),
                err,
            )),
        }
    }

    fn create_parent_dirs(&self, path: &Path) -> StoreResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                BinaryDbError::io(
                    format!("create Binary DB parent directory {}", parent.display()),
                    err,
                )
            })?;
        }
        Ok(())
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        self.create_parent_dirs(path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .map_err(|err| {
                BinaryDbError::io(format!("open Binary DB file {}", path.display()), err)
            })?;
        let offset = file.seek(SeekFrom::End(0)).map_err(|err| {
            BinaryDbError::io(format!("seek Binary DB file {}", path.display()), err)
        })?;
        file.write_all(bytes).map_err(|err| {
            BinaryDbError::io(format!("append Binary DB file {}", path.display()), err)
        })?;
        Ok(offset)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> StoreResult<()> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|err| {
                BinaryDbError::io(format!("open Binary DB file {}", path.display()), err)
            })?;
        file.seek(SeekFrom::Start(offset)).map_err(|err| {
            BinaryDbError::io(format!("seek Binary DB file {}", path.display()), err)
        })?;
        file.write_all(bytes).map_err(|err| {
            BinaryDbError::io(format!("overwrite Binary DB file {}", path.display()), err)
        })
    }

    fn truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        let file = OpenOptions::new().write(true).open(path).map_err(|err| {
            BinaryDbError::io(format!("open Binary DB file {}", path.display()), err)
        })?;
        file.set_len(len).map_err(|err| {
            BinaryDbError::io(format!("truncate Binary DB file {}", path.display()), err)
        })
    }

    fn remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(BinaryDbError::io(
                format!("remove Binary DB file {}", path.display()),
                err,
            )),
        }
    }
}

impl ServerBinaryDbDurabilityStore for ServerBinaryDbFilesystemStore {
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|err| {
                BinaryDbError::io(format!("sync Binary DB file {}", path.display()), err)
            })
    }

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        File::open(path)
            .and_then(|file| file.sync_data())
            .map_err(|err| {
                BinaryDbError::io(format!("sync Binary DB file data {}", path.display()), err)
            })
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|err| {
                BinaryDbError::io(format!("sync Binary DB directory {}", path.display()), err)
            })
    }
}

pub(super) fn store_path_for(root: &StorePath, relative: &StorePath) -> StoreResult<PathBuf> {
    let path = relative.as_path();
    if path.is_absolute() {
        return Err(BinaryDbError::invalid_domain_data(
            "absolute paths are not allowed",
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(BinaryDbError::invalid_domain_data(
            "parent traversal in file path is not allowed",
        ));
    }
    Ok(root.as_path().join(path))
}
