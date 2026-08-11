//! Binary DB durability policy boundaries.

use super::*;

pub trait BinaryDbFsyncPolicy {
    fn sync_file(&self, path: &Path) -> StoreResult<()>;

    fn sync_directory(&self, path: &Path) -> StoreResult<()>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BinaryDbNoopFsyncPolicy;

impl BinaryDbFsyncPolicy for BinaryDbNoopFsyncPolicy {
    fn sync_file(&self, _path: &Path) -> StoreResult<()> {
        Ok(())
    }

    fn sync_directory(&self, _path: &Path) -> StoreResult<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BinaryDbStdFsyncPolicy;

impl BinaryDbFsyncPolicy for BinaryDbStdFsyncPolicy {
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        FilesystemFileIoStore.sync_file(path).map_err(|e| {
            file_io_error_to_binary(format!("sync Binary DB file {}", path.display()), e)
        })
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        FilesystemFileIoStore.sync_dir(path).map_err(|e| {
            file_io_error_to_binary(format!("sync Binary DB directory {}", path.display()), e)
        })
    }
}
