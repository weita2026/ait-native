use super::*;

const BINARY_DB_TXN_JOURNAL_HEADER: &str = "ait-binary-db-rollback-journal-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerBinaryDbPersistentJournalContractRow {
    pub invariant: &'static str,
    pub guarantee: &'static str,
}

pub const SERVER_BINARY_DB_PERSISTENT_JOURNAL_CONTRACT:
    &[ServerBinaryDbPersistentJournalContractRow] = &[
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "journal header",
        guarantee: "persistent rollback journals retain ait-binary-db-rollback-journal-v1 and recover length plus before-image entries",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "relative paths",
        guarantee: "journal entries are UTF-8 relative paths and cannot escape the authority root",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "original lengths",
        guarantee: "each touched file captures existence and original length before mutation",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "append durability boundary",
        guarantee: "each recovery entry persists journal contents and readable length before protected mutation; the already-durable journal directory entry is not resynced per append",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "overwrite before-images",
        guarantee: "in-place fixed-record writes persist and fsync original bytes before mutation",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "commit cleanup",
        guarantee: "commit fsyncs touched files/directories and removes the scoped journal",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "post-commit lock cleanup",
        guarantee: "after journal removal and directory sync establish the durable commit point, lock cleanup failure is observable commit outcome metadata and never a retryable ordinary failure",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "abort cleanup",
        guarantee: "abort and drop roll back tracked files and remove the scoped journal",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "stale recovery",
        guarantee: "new writes recover every stale journal whose command-scope lock set overlaps the requested writer before proceeding",
    },
    ServerBinaryDbPersistentJournalContractRow {
        invariant: "corrupt journal",
        guarantee: "malformed journal data fails closed and releases the writer lock",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
enum BinaryDbTxnJournalEntry {
    File {
        relative_path: StorePath,
        existed: bool,
        original_len: u64,
    },
    BeforeImage {
        relative_path: StorePath,
        offset: u64,
        bytes: Vec<u8>,
    },
}

#[derive(Debug)]
pub(super) struct BinaryDbTxnJournal {
    path: PathBuf,
    entries: Vec<BinaryDbTxnJournalEntry>,
    active: bool,
}

impl BinaryDbTxnJournal {
    pub(super) fn create_new<B, F>(
        db: &B,
        command_scope: BinaryDbCommandScope,
        fsync_policy: &F,
    ) -> StoreResult<Self>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        let path = Self::journal_path(db.authority_root(), command_scope);
        if db.metadata_len(&path)?.is_some() {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB journal {} already exists before transaction creation",
                path.display()
            )));
        }
        db.journal_append_bytes(
            &path,
            format!("{BINARY_DB_TXN_JOURNAL_HEADER}\n").as_bytes(),
        )?;
        fsync_policy.sync_file(&path)?;
        if let Some(parent) = path.parent() {
            fsync_policy.sync_directory(parent)?;
        }
        Ok(Self {
            path,
            entries: Vec::new(),
            active: true,
        })
    }

    pub(super) fn overlapping_recovery_scopes<B>(
        db: &B,
        requested_scope: BinaryDbCommandScope,
    ) -> Vec<BinaryDbCommandScope>
    where
        B: BinaryDb + ?Sized,
    {
        let pending = BinaryDbCommandScope::ALL
            .iter()
            .copied()
            .filter(|scope| db.path_exists(&Self::journal_path(db.authority_root(), *scope)))
            .collect::<Vec<_>>();
        let mut scopes = vec![requested_scope];
        let mut lock_names = requested_scope
            .lock_file_names()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        loop {
            let mut changed = false;
            for scope in &pending {
                if scopes.contains(scope)
                    || !scope
                        .lock_file_names()
                        .iter()
                        .any(|name| lock_names.contains(name))
                {
                    continue;
                }
                scopes.push(*scope);
                lock_names.extend(scope.lock_file_names().iter().copied());
                changed = true;
            }
            if !changed {
                break;
            }
        }
        scopes
    }

    pub(super) fn recover_existing<B, F>(
        db: &B,
        command_scope: BinaryDbCommandScope,
        fsync_policy: &F,
    ) -> StoreResult<()>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        let path = Self::journal_path(db.authority_root(), command_scope);
        if !db.path_exists(&path) {
            return Ok(());
        }
        let entries = Self::read_entries(db, &path)?;
        Self::rollback_entries(db, &entries, fsync_policy)?;
        db.journal_remove_file_if_exists(&path)?;
        if let Some(parent) = path.parent() {
            fsync_policy.sync_directory(parent)?;
        }
        Ok(())
    }

    pub(super) fn track_relative_paths<B, F>(
        &mut self,
        db: &B,
        relatives: &[StorePath],
        fsync_policy: &F,
    ) -> StoreResult<Vec<bool>>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        #[cfg(feature = "perfetto-tracing")]
        let _trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.binary_db.journal.track_relative_paths",
        );
        let mut pending = Vec::new();
        let mut existed_by_path = Vec::with_capacity(relatives.len());
        for relative in relatives {
            let existing =
                self.entries
                    .iter()
                    .chain(pending.iter())
                    .find_map(|entry| match entry {
                        BinaryDbTxnJournalEntry::File {
                            relative_path,
                            existed,
                            ..
                        } if relative_path == relative => Some(*existed),
                        _ => None,
                    });
            if let Some(existed) = existing {
                existed_by_path.push(existed);
                continue;
            }
            let absolute_path = store_path_for(db.authority_root(), relative)?;
            let metadata_len = db.metadata_len(&absolute_path)?;
            let existed = metadata_len.is_some();
            pending.push(BinaryDbTxnJournalEntry::File {
                relative_path: relative.clone(),
                existed,
                original_len: metadata_len.unwrap_or(0),
            });
            existed_by_path.push(existed);
        }
        self.append_entries(db, &pending, fsync_policy)?;
        self.entries.extend(pending);
        Ok(existed_by_path)
    }

    pub(super) fn track_before_image<B, F>(
        &mut self,
        db: &B,
        relative: &StorePath,
        offset: u64,
        bytes: &[u8],
        fsync_policy: &F,
    ) -> StoreResult<()>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        self.track_before_images(db, relative, &[(offset, bytes.to_vec())], fsync_policy)
    }

    pub(super) fn requires_before_image(
        &self,
        relative: &StorePath,
        offset: u64,
    ) -> StoreResult<bool> {
        let original_len = self
            .entries
            .iter()
            .find_map(|entry| match entry {
                BinaryDbTxnJournalEntry::File {
                    relative_path,
                    original_len,
                    ..
                } if relative_path == relative => Some(*original_len),
                _ => None,
            })
            .ok_or_else(|| {
                BinaryDbError::corruption(format!(
                    "Binary DB journal has no file boundary for '{}'",
                    relative.as_path().display()
                ))
            })?;
        // Appended records at or beyond the transaction's original file
        // boundary are removed by rollback truncation. Recording their
        // transient bytes as before-images is redundant and turns one bounded
        // batch into one journal fsync per appended owner.
        Ok(offset < original_len)
    }

    pub(super) fn track_before_images<B, F>(
        &mut self,
        db: &B,
        relative: &StorePath,
        before_images: &[(u64, BinaryRecordBytes)],
        fsync_policy: &F,
    ) -> StoreResult<()>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        if before_images.is_empty() {
            return Ok(());
        }
        let entries = before_images
            .iter()
            .map(|(offset, bytes)| BinaryDbTxnJournalEntry::BeforeImage {
                relative_path: relative.clone(),
                offset: *offset,
                bytes: bytes.clone(),
            })
            .collect::<Vec<_>>();
        self.append_entries(db, &entries, fsync_policy)?;
        self.entries.extend(entries);
        Ok(())
    }

    pub(super) fn commit<B, F>(&mut self, db: &B, fsync_policy: &F) -> StoreResult<()>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        if !self.active {
            return Ok(());
        }
        db.journal_remove_file_if_exists(&self.path)?;
        if let Some(parent) = self.path.parent() {
            fsync_policy.sync_directory(parent)?;
        }
        self.active = false;
        Ok(())
    }

    pub(super) fn abort<B, F>(&mut self, db: &B, fsync_policy: &F) -> StoreResult<()>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        if !self.active {
            return Ok(());
        }
        Self::rollback_entries(db, &self.entries, fsync_policy)?;
        db.journal_remove_file_if_exists(&self.path)?;
        if let Some(parent) = self.path.parent() {
            fsync_policy.sync_directory(parent)?;
        }
        self.active = false;
        Ok(())
    }

    fn journal_path(root: &StorePath, command_scope: BinaryDbCommandScope) -> PathBuf {
        root.as_path().join(command_scope.journal_file_name())
    }

    fn append_entries<B, F>(
        &self,
        db: &B,
        entries: &[BinaryDbTxnJournalEntry],
        fsync_policy: &F,
    ) -> StoreResult<()>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        if entries.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "perfetto-tracing")]
        let _trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.binary_db.journal.append_entries",
        );
        let mut encoded = String::new();
        for entry in entries {
            let line = match entry {
                BinaryDbTxnJournalEntry::File {
                    relative_path,
                    existed,
                    original_len,
                } => format!(
                    "file\t{}\t{}\t{}",
                    if *existed { 1 } else { 0 },
                    original_len,
                    Self::encode_relative_path(relative_path.as_path())?
                ),
                BinaryDbTxnJournalEntry::BeforeImage {
                    relative_path,
                    offset,
                    bytes,
                } => format!(
                    "before\t{}\t{}\t{}",
                    offset,
                    encode_hex(bytes),
                    Self::encode_relative_path(relative_path.as_path())?
                ),
            };
            encoded.push_str(&line);
            encoded.push('\n');
        }
        #[cfg(feature = "perfetto-tracing")]
        let append_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.binary_db.journal.append_entries.write",
        );
        db.journal_append_bytes(&self.path, encoded.as_bytes())?;
        #[cfg(feature = "perfetto-tracing")]
        drop(append_trace);
        // The journal directory entry was made durable by `create_new`.
        // Appending a recovery record changes only the existing journal file:
        // persist its bytes and readable length before the protected mutation,
        // without repeating an unrelated parent-directory full sync.
        #[cfg(feature = "perfetto-tracing")]
        let sync_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.binary_db.journal.append_entries.sync_file_data",
        );
        fsync_policy.sync_file_data(&self.path)?;
        #[cfg(feature = "perfetto-tracing")]
        drop(sync_trace);
        Ok(())
    }

    fn read_entries<B>(db: &B, path: &Path) -> StoreResult<Vec<BinaryDbTxnJournalEntry>>
    where
        B: BinaryDb + ?Sized,
    {
        let contents = db.read_to_string(path)?;
        let mut lines = contents.lines();
        let header = lines
            .next()
            .ok_or_else(|| BinaryDbError::corruption("Binary DB journal is missing header"))?;
        if header != BINARY_DB_TXN_JOURNAL_HEADER {
            return Err(BinaryDbError::corruption(format!(
                "unsupported Binary DB journal header: {header}"
            )));
        }

        let mut entries = Vec::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.splitn(4, '\t');
            let kind = parts.next().ok_or_else(|| {
                BinaryDbError::corruption("Binary DB journal entry is missing kind")
            })?;
            match kind {
                "file" => {
                    let existed = match parts.next() {
                        Some("0") => false,
                        Some("1") => true,
                        Some(value) => {
                            return Err(BinaryDbError::corruption(format!(
                                "invalid Binary DB journal existence flag: {value}"
                            )));
                        }
                        None => {
                            return Err(BinaryDbError::corruption(
                                "Binary DB journal entry is missing existence flag",
                            ));
                        }
                    };
                    let original_len = parts
                        .next()
                        .ok_or_else(|| {
                            BinaryDbError::corruption("Binary DB journal entry is missing length")
                        })?
                        .parse::<u64>()
                        .map_err(|err| {
                            BinaryDbError::corruption(format!(
                                "invalid Binary DB journal length: {err}"
                            ))
                        })?;
                    let relative = parts.next().ok_or_else(|| {
                        BinaryDbError::corruption("Binary DB journal entry is missing path")
                    })?;
                    let relative_path = StorePath::new(relative);
                    let _ = store_path_for(db.authority_root(), &relative_path)?;
                    entries.push(BinaryDbTxnJournalEntry::File {
                        relative_path,
                        existed,
                        original_len,
                    });
                }
                "before" => {
                    let offset = parts
                        .next()
                        .ok_or_else(|| {
                            BinaryDbError::corruption(
                                "Binary DB before-image entry is missing offset",
                            )
                        })?
                        .parse::<u64>()
                        .map_err(|err| {
                            BinaryDbError::corruption(format!(
                                "invalid Binary DB before-image offset: {err}"
                            ))
                        })?;
                    let bytes = decode_hex(parts.next().ok_or_else(|| {
                        BinaryDbError::corruption("Binary DB before-image entry is missing bytes")
                    })?)?;
                    let relative = parts.next().ok_or_else(|| {
                        BinaryDbError::corruption("Binary DB before-image entry is missing path")
                    })?;
                    let relative_path = StorePath::new(relative);
                    let _ = store_path_for(db.authority_root(), &relative_path)?;
                    entries.push(BinaryDbTxnJournalEntry::BeforeImage {
                        relative_path,
                        offset,
                        bytes,
                    });
                }
                _ => {
                    return Err(BinaryDbError::corruption(format!(
                        "unsupported Binary DB journal entry kind: {kind}"
                    )));
                }
            }
        }
        Ok(entries)
    }

    fn rollback_entries<B, F>(
        db: &B,
        entries: &[BinaryDbTxnJournalEntry],
        fsync_policy: &F,
    ) -> StoreResult<()>
    where
        B: BinaryDb + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        for entry in entries.iter().rev() {
            let relative_path = match entry {
                BinaryDbTxnJournalEntry::File { relative_path, .. }
                | BinaryDbTxnJournalEntry::BeforeImage { relative_path, .. } => relative_path,
            };
            let absolute_path = store_path_for(db.authority_root(), relative_path)?;
            match entry {
                BinaryDbTxnJournalEntry::File {
                    existed,
                    original_len,
                    ..
                } => {
                    if *existed {
                        db.journal_truncate_file(&absolute_path, *original_len)?;
                        fsync_policy.sync_file(&absolute_path)?;
                    } else {
                        db.journal_remove_file_if_exists(&absolute_path)?;
                    }
                }
                BinaryDbTxnJournalEntry::BeforeImage { offset, bytes, .. } => {
                    db.journal_overwrite_range(&absolute_path, *offset, bytes)?;
                    fsync_policy.sync_file(&absolute_path)?;
                }
            }
            if let Some(parent) = absolute_path.parent() {
                fsync_policy.sync_directory(parent)?;
            }
        }
        Ok(())
    }

    fn encode_relative_path(path: &Path) -> StoreResult<&str> {
        let text = path.to_str().ok_or_else(|| {
            BinaryDbError::invalid_domain_data("Binary DB journal paths must be UTF-8")
        })?;
        if text.contains('\t') || text.contains('\n') || text.contains('\r') {
            return Err(BinaryDbError::invalid_domain_data(
                "Binary DB journal paths cannot contain control separators",
            ));
        }
        Ok(text)
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

fn decode_hex(value: &str) -> StoreResult<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(BinaryDbError::corruption(
            "Binary DB before-image hex has odd length",
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|err| {
                BinaryDbError::corruption(format!("invalid before-image hex: {err}"))
            })?;
            u8::from_str_radix(text, 16).map_err(|err| {
                BinaryDbError::corruption(format!("invalid before-image hex: {err}"))
            })
        })
        .collect()
}
