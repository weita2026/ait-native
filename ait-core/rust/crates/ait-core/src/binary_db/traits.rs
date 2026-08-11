//! Injected Binary DB storage boundaries.

use super::*;

pub trait BinaryDbFileStore:
    FileIoStore + FileIoByteStore + FileIoDurabilityStore + FileIoLockStore
{
}

impl<T> BinaryDbFileStore for T where
    T: FileIoStore + FileIoByteStore + FileIoDurabilityStore + FileIoLockStore
{
}
