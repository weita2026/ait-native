//! Binary DB store path identities.

use super::*;

/// Binary DB root path wrapper used by local and remote authority adapters.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct StorePath(pub PathBuf);

impl StorePath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }
}

impl AsRef<Path> for StorePath {
    fn as_ref(&self) -> &Path {
        self.0.as_path()
    }
}

impl From<&str> for StorePath {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<&Path> for StorePath {
    fn from(value: &Path) -> Self {
        Self::new(value.to_owned())
    }
}

impl From<PathBuf> for StorePath {
    fn from(value: PathBuf) -> Self {
        Self(value)
    }
}

pub type BinaryRecordBytes = Vec<u8>;
pub type BinaryRecordBytesRef<'a> = &'a [u8];
pub type BinaryIndexKeyRef<'a> = &'a [u8];
