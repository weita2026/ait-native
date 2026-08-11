//! Binary DB authority, repository, scope, and write identities.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityId(pub String);

impl AuthorityId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoId(pub String);

impl RepoId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoName(pub String);

impl RepoName {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalStateScope {
    Repository,
    Line,
    Task,
    RemoteCache,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryDbFileFamily {
    Content,
    Plan,
}

impl BinaryDbFileFamily {
    fn for_relative_path(path: &StorePath) -> Self {
        let file_name = path
            .as_path()
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if file_name == "plan.bin" || file_name.starts_with("plan_") {
            Self::Plan
        } else {
            Self::Content
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct BinaryWriteContext {
    transaction_id: u64,
    command_scope: BinaryDbCommandScope,
    active: bool,
}

impl BinaryWriteContext {
    pub(super) fn new(command_scope: BinaryDbCommandScope) -> Self {
        static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            transaction_id: NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed),
            command_scope,
            active: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(command_scope: BinaryDbCommandScope) -> Self {
        Self::new(command_scope)
    }

    pub fn transaction_id(&self) -> u64 {
        self.transaction_id
    }

    pub fn command_scope(&self) -> BinaryDbCommandScope {
        self.command_scope
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn ensure_active(&self) -> StoreResult<()> {
        if self.active {
            Ok(())
        } else {
            Err(BinaryDbError::invalid_domain_data(
                "Binary DB write capability is no longer active",
            ))
        }
    }

    fn ensure_authorized_family(&self, family: BinaryDbFileFamily) -> StoreResult<()> {
        self.ensure_active()?;
        if self.command_scope.authorizes_file_family(family) {
            Ok(())
        } else {
            Err(BinaryDbError::invalid_domain_data(format!(
                "Binary DB write capability for {:?} cannot mutate {:?} files",
                self.command_scope, family
            )))
        }
    }

    pub(super) fn ensure_authorized_path(&self, path: &StorePath) -> StoreResult<()> {
        self.ensure_authorized_family(BinaryDbFileFamily::for_relative_path(path))
    }

    pub(super) fn finish(&mut self) {
        self.active = false;
    }
}

/// Logical file families covered by one read transaction.
///
/// Local and remote writer lock names are both included because the authority
/// root, rather than the read-side store type, decides which persisted files
/// are being served.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BinaryDbReadScope {
    #[default]
    All,
    Content,
    Plan,
    ContentAndPlan,
}

impl BinaryDbReadScope {
    pub const fn lock_file_names(self) -> &'static [&'static str] {
        match self {
            Self::All => BinaryDbCommandScope::all_write_lock_file_names(),
            Self::Content => &["content.write.lock", "remote-content.write.lock"],
            Self::Plan => &["plan.write.lock", "remote-plan.write.lock"],
            Self::ContentAndPlan => &[
                "content.write.lock",
                "plan.write.lock",
                "remote-content.write.lock",
                "remote-plan.write.lock",
            ],
        }
    }
}

/// Command scopes identify write-lock families owned by command-level workflows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BinaryDbCommandScope {
    #[default]
    General,
    PlanSyncLocal,
    PlanSyncLocalPlan,
    PlanSyncRemote,
    RemoteSyncLocalImport,
    PlanImport,
    SnapshotWrite,
    ContentWrite,
    Gc,
}

impl BinaryDbCommandScope {
    pub const fn lock_file_names(self) -> &'static [&'static str] {
        match self {
            // Keep every multi-lock scope in the same lexical order used by read
            // transactions. This prevents two overlapping writers from acquiring
            // the same lock families in opposite orders.
            Self::General => Self::all_write_lock_file_names(),
            Self::PlanSyncLocal => &["content.write.lock", "plan.write.lock"],
            Self::PlanSyncLocalPlan => &["plan.write.lock"],
            Self::PlanSyncRemote => &["remote-content.write.lock", "remote-plan.write.lock"],
            Self::RemoteSyncLocalImport => &["content.write.lock"],
            Self::PlanImport => &["content.write.lock", "plan.write.lock"],
            Self::SnapshotWrite => &["content.write.lock", "snapshot.write.lock"],
            Self::ContentWrite => &["content.write.lock"],
            Self::Gc => &[
                "content.write.lock",
                "gc.write.lock",
                "plan.write.lock",
                "snapshot.write.lock",
            ],
        }
    }

    pub const fn all_write_lock_file_names() -> &'static [&'static str] {
        &[
            "content.write.lock",
            "gc.write.lock",
            "global.write.lock",
            "plan.write.lock",
            "remote-content.write.lock",
            "remote-plan.write.lock",
            "snapshot.write.lock",
        ]
    }

    pub fn conflicts_with(self, other: Self) -> bool {
        self.lock_file_names()
            .iter()
            .any(|left| other.lock_file_names().contains(left))
    }

    pub const fn authorizes_file_family(self, family: BinaryDbFileFamily) -> bool {
        match self {
            Self::General => true,
            Self::Gc => matches!(
                family,
                BinaryDbFileFamily::Content | BinaryDbFileFamily::Plan
            ),
            Self::PlanSyncLocal | Self::PlanImport | Self::PlanSyncRemote => true,
            Self::PlanSyncLocalPlan => matches!(family, BinaryDbFileFamily::Plan),
            Self::RemoteSyncLocalImport | Self::SnapshotWrite | Self::ContentWrite => {
                matches!(family, BinaryDbFileFamily::Content)
            }
        }
    }
}

pub(crate) fn validate_store_relative_path(path: &Path) -> StoreResult<&Path> {
    if path.is_absolute() {
        return Err(BinaryDbError::invalid_domain_data(format!(
            "path must be relative: {}",
            path.display()
        )));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "path must not include parent traversal: {}",
                path.display()
            )));
        }
    }
    Ok(path)
}

pub(crate) fn store_path_for(authority_root: &StorePath, path: &StorePath) -> StoreResult<PathBuf> {
    let rel = validate_store_relative_path(path.as_path())?;
    Ok(authority_root.as_path().join(rel))
}
