use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "remote_binary_db/ids.rs"]
mod ids;
pub use ids::*;
#[path = "remote_binary_db/admission.rs"]
mod admission;
pub use admission::*;
#[path = "remote_binary_db/store.rs"]
mod store;
pub use store::*;
#[path = "remote_binary_db/locks.rs"]
mod locks;
use locks::*;
pub use locks::{BinaryDbCommandLockSet, BinaryDbReadLockSet, BinaryDbRecoveryAdmissionLock};
#[path = "remote_binary_db/journal.rs"]
mod journal;
use journal::*;
pub use journal::{
    ServerBinaryDbPersistentJournalContractRow, SERVER_BINARY_DB_PERSISTENT_JOURNAL_CONTRACT,
};
#[path = "remote_binary_db/txn.rs"]
mod txn;
pub use txn::*;

#[path = "remote_binary_db/errors.rs"]
mod errors;
pub use errors::*;
#[path = "remote_binary_db/filesystem_store.rs"]
mod filesystem_store;
use filesystem_store::store_path_for;
pub use filesystem_store::{
    sync_filesystem_directory, sync_filesystem_file, sync_filesystem_file_data,
    ServerBinaryDbFilesystemStore,
};
#[path = "remote_binary_db/contracts.rs"]
mod contracts;
pub(crate) use contracts::private::BinaryDbJournalIo;
pub use contracts::*;
#[path = "remote_binary_db/conformance.rs"]
mod conformance;
pub use conformance::*;
#[path = "remote_binary_db/filesystem_db.rs"]
mod filesystem_db;
pub use filesystem_db::*;

#[cfg(test)]
#[path = "remote_binary_db/test_support.rs"]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "remote_binary_db/tests.rs"]
mod tests;
