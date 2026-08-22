use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct StoreGeneration(u64);

impl StoreGeneration {
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerBinaryDbAuthorityMode {
    ServingAuthority,
    TestFixture,
}

impl ServerBinaryDbAuthorityMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ServingAuthority => "serving_authority",
            Self::TestFixture => "test_fixture",
        }
    }

    pub const fn is_serving_authority(self) -> bool {
        matches!(self, Self::ServingAuthority)
    }

    pub const fn is_test_fixture(self) -> bool {
        matches!(self, Self::TestFixture)
    }
}

pub type BinaryRecordBytes = Vec<u8>;
pub type BinaryRecordBytesRef<'a> = &'a [u8];
pub type BinaryIndexKeyRef<'a> = &'a [u8];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BinaryDbFileFamily {
    /// Task, Change, Patchset, Review, Attestation, Policy, and Land records,
    /// payloads, and indexes. Repository content and Plan state are excluded.
    Workflow,
    /// Plan, Plan revision, and Plan item records and payloads. The immutable
    /// content that stores a Markdown artifact is excluded.
    Plan,
    /// Server operational Repository-registry and Repository-local Worker Job
    /// records and rebuildable indexes. Root-kind admission keeps the global
    /// registry files separate from each Repository's Worker Job family.
    Queue,
    /// Immutable physical object/tree pack bytes and their publish paths.
    /// Object/tree metadata and locators are Content, not RepositoryPack.
    RepositoryPack,
    /// Line, snapshot, blob, object/tree metadata, payloads, and indexes.
    /// Physical pack bytes, Plan state, and workflow state are excluded.
    Content,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct StorePath(PathBuf);

impl StorePath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for StorePath {
    fn as_ref(&self) -> &Path {
        self.as_path()
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

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct BinaryFileId {
    relative_path: StorePath,
    record_size: u32,
    layout_id: u32,
    family: BinaryDbFileFamily,
}

impl BinaryFileId {
    pub(in crate::foundation) fn new(
        path: impl Into<StorePath>,
        layout_id: u32,
        record_size: u32,
        family: BinaryDbFileFamily,
    ) -> Self {
        Self {
            relative_path: path.into(),
            record_size,
            layout_id,
            family,
        }
    }

    pub fn as_str(&self) -> &str {
        self.relative_path
            .as_path()
            .to_str()
            .expect("server Binary DB file ids must be UTF-8 paths")
    }

    pub fn relative_path(&self) -> &StorePath {
        &self.relative_path
    }

    pub fn record_size(&self) -> u32 {
        self.record_size
    }

    pub fn layout_id(&self) -> u32 {
        self.layout_id
    }

    pub fn family(&self) -> BinaryDbFileFamily {
        self.family
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct BinaryPayloadFileId {
    relative_path: StorePath,
    layout_id: u32,
    family: BinaryDbFileFamily,
}

impl BinaryPayloadFileId {
    pub(in crate::foundation) fn new(
        path: impl Into<StorePath>,
        layout_id: u32,
        family: BinaryDbFileFamily,
    ) -> Self {
        Self {
            relative_path: path.into(),
            layout_id,
            family,
        }
    }

    pub fn as_str(&self) -> &str {
        self.relative_path
            .as_path()
            .to_str()
            .expect("server Binary DB payload ids must be UTF-8 paths")
    }

    pub fn relative_path(&self) -> &StorePath {
        &self.relative_path
    }

    pub fn layout_id(&self) -> u32 {
        self.layout_id
    }

    pub fn family(&self) -> BinaryDbFileFamily {
        self.family
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct BinaryIndexId {
    relative_path: StorePath,
    layout_id: u32,
    fixed_key_size: Option<u32>,
    stores_record_index_plus_one: bool,
    family: BinaryDbFileFamily,
}

impl BinaryIndexId {
    #[cfg(test)]
    pub(in crate::foundation) fn new(
        path: impl Into<StorePath>,
        layout_id: u32,
        family: BinaryDbFileFamily,
    ) -> Self {
        Self {
            relative_path: path.into(),
            layout_id,
            fixed_key_size: None,
            stores_record_index_plus_one: false,
            family,
        }
    }

    pub(in crate::foundation) fn new_fixed(
        path: impl Into<StorePath>,
        layout_id: u32,
        key_size: u32,
        stores_record_index_plus_one: bool,
        family: BinaryDbFileFamily,
    ) -> Self {
        Self {
            relative_path: path.into(),
            layout_id,
            fixed_key_size: Some(key_size),
            stores_record_index_plus_one,
            family,
        }
    }

    pub fn as_str(&self) -> &str {
        self.relative_path
            .as_path()
            .to_str()
            .expect("server Binary DB index ids must be UTF-8 paths")
    }

    pub fn relative_path(&self) -> &StorePath {
        &self.relative_path
    }

    pub fn layout_id(&self) -> u32 {
        self.layout_id
    }

    pub fn fixed_key_size(&self) -> Option<u32> {
        self.fixed_key_size
    }

    pub fn stores_record_index_plus_one(&self) -> bool {
        self.stores_record_index_plus_one
    }

    pub fn family(&self) -> BinaryDbFileFamily {
        self.family
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadRange {
    pub payload_offset: u64,
    pub payload_len: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub struct BinaryWriteContext {
    command_scope: BinaryDbCommandScope,
    active: bool,
}

impl BinaryWriteContext {
    pub fn command_scope(&self) -> BinaryDbCommandScope {
        self.command_scope
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub(in crate::foundation::remote_binary_db) const fn new(
        command_scope: BinaryDbCommandScope,
    ) -> Self {
        Self {
            command_scope,
            active: true,
        }
    }

    #[cfg(test)]
    pub(crate) const fn test_fixture(command_scope: BinaryDbCommandScope) -> Self {
        Self::new(command_scope)
    }

    pub(in crate::foundation::remote_binary_db) fn ensure_active(&self) -> StoreResult<()> {
        if self.active {
            Ok(())
        } else {
            Err(BinaryDbError::invalid_domain_data(
                "Binary DB write capability is no longer active",
            ))
        }
    }

    pub(in crate::foundation::remote_binary_db) fn ensure_authorized_family(
        &self,
        family: BinaryDbFileFamily,
    ) -> StoreResult<()> {
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

    pub(in crate::foundation::remote_binary_db) fn finish(&mut self) {
        self.active = false;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BinaryDbReadScope(u8);

impl Default for BinaryDbReadScope {
    fn default() -> Self {
        Self::ALL
    }
}

impl BinaryDbReadScope {
    const WORKFLOW_BIT: u8 = 1 << 0;
    const PLAN_BIT: u8 = 1 << 1;
    const QUEUE_BIT: u8 = 1 << 2;
    const REPOSITORY_PACK_BIT: u8 = 1 << 3;
    const CONTENT_BIT: u8 = 1 << 4;

    pub const ALL: Self = Self(
        Self::WORKFLOW_BIT
            | Self::PLAN_BIT
            | Self::QUEUE_BIT
            | Self::REPOSITORY_PACK_BIT
            | Self::CONTENT_BIT,
    );
    pub const WORKFLOW: Self = Self(Self::WORKFLOW_BIT);
    pub const PLAN: Self = Self(Self::PLAN_BIT);
    pub const QUEUE: Self = Self(Self::QUEUE_BIT);
    pub const REPOSITORY_PACK: Self = Self(Self::REPOSITORY_PACK_BIT);
    pub const CONTENT: Self = Self(Self::CONTENT_BIT);

    pub const fn for_family(family: BinaryDbFileFamily) -> Self {
        match family {
            BinaryDbFileFamily::Workflow => Self::WORKFLOW,
            BinaryDbFileFamily::Plan => Self::PLAN,
            BinaryDbFileFamily::Queue => Self::QUEUE,
            BinaryDbFileFamily::RepositoryPack => Self::REPOSITORY_PACK,
            BinaryDbFileFamily::Content => Self::CONTENT,
        }
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn includes_family(self, family: BinaryDbFileFamily) -> bool {
        self.0 & Self::for_family(family).0 != 0
    }

    pub const fn is_subset_of(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }

    pub(crate) fn includes_lock_file_name(self, name: &str) -> bool {
        match name {
            "server-workflow.write.lock" => self.includes_family(BinaryDbFileFamily::Workflow),
            "server-plan.write.lock" => self.includes_family(BinaryDbFileFamily::Plan),
            "server-queue.write.lock" => self.includes_family(BinaryDbFileFamily::Queue),
            "server-repository-pack.write.lock" => {
                self.includes_family(BinaryDbFileFamily::RepositoryPack)
            }
            "server-content.write.lock" => self.includes_family(BinaryDbFileFamily::Content),
            "global.write.lock" => self == Self::ALL,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BinaryDbCommandScope {
    /// Offline whole-authority maintenance only. Serving request paths must use
    /// one of the exact family or composite scopes below.
    #[default]
    General,
    ServerWorkflow,
    ServerPlan,
    ServerQueue,
    ServerRepositoryPack,
    ServerContent,
    /// A serving Plan-bound Task start owns Plan plus Workflow so the selected
    /// Plan head, Task, and initial Change become visible atomically.
    ServerTaskStart,
    /// The only serving composite mutation: Content plus Workflow for an
    /// atomic line advance and Land workflow append.
    ServerLand,
    /// A remote-sync metadata commit. Raw pack bytes were published earlier,
    /// so the transaction owns Content only despite its historical name.
    ServerRemoteSyncCommit,
}

impl BinaryDbCommandScope {
    pub const ALL: [Self; 9] = [
        Self::General,
        Self::ServerContent,
        Self::ServerLand,
        Self::ServerPlan,
        Self::ServerQueue,
        Self::ServerRemoteSyncCommit,
        Self::ServerRepositoryPack,
        Self::ServerTaskStart,
        Self::ServerWorkflow,
    ];

    pub const fn lock_file_names(self) -> &'static [&'static str] {
        match self {
            Self::General => &[
                "global.write.lock",
                "server-content.write.lock",
                "server-plan.write.lock",
                "server-queue.write.lock",
                "server-repository-pack.write.lock",
                "server-workflow.write.lock",
            ],
            Self::ServerWorkflow => &["server-workflow.write.lock"],
            Self::ServerPlan => &["server-plan.write.lock"],
            Self::ServerQueue => &["server-queue.write.lock"],
            Self::ServerRepositoryPack => &["server-repository-pack.write.lock"],
            Self::ServerContent => &["server-content.write.lock"],
            Self::ServerTaskStart => &["server-plan.write.lock", "server-workflow.write.lock"],
            Self::ServerLand => &["server-content.write.lock", "server-workflow.write.lock"],
            Self::ServerRemoteSyncCommit => &["server-content.write.lock"],
        }
    }

    /// Complete data-family write set declared before recovery admission.
    /// This is the authorization authority; callers cannot add another family
    /// after a transaction has begun.
    pub const fn write_scope(self) -> BinaryDbReadScope {
        match self {
            Self::General => BinaryDbReadScope::ALL,
            Self::ServerWorkflow => BinaryDbReadScope::WORKFLOW,
            Self::ServerPlan => BinaryDbReadScope::PLAN,
            Self::ServerQueue => BinaryDbReadScope::QUEUE,
            Self::ServerRepositoryPack => BinaryDbReadScope::REPOSITORY_PACK,
            Self::ServerContent | Self::ServerRemoteSyncCommit => BinaryDbReadScope::CONTENT,
            Self::ServerTaskStart => BinaryDbReadScope::PLAN.union(BinaryDbReadScope::WORKFLOW),
            Self::ServerLand => BinaryDbReadScope::CONTENT.union(BinaryDbReadScope::WORKFLOW),
        }
    }

    pub const fn authorizes(self, requested: Self) -> bool {
        match self {
            Self::General => true,
            Self::ServerLand => matches!(
                requested,
                Self::ServerLand | Self::ServerContent | Self::ServerWorkflow
            ),
            Self::ServerTaskStart => matches!(
                requested,
                Self::ServerTaskStart | Self::ServerPlan | Self::ServerWorkflow
            ),
            Self::ServerRemoteSyncCommit => matches!(
                requested,
                Self::ServerRemoteSyncCommit | Self::ServerContent
            ),
            _ => self as u8 == requested as u8,
        }
    }

    pub const fn authorizes_file_family(self, family: BinaryDbFileFamily) -> bool {
        self.write_scope().includes_family(family)
    }

    pub fn conflicts_with(self, other: Self) -> bool {
        self.lock_file_names()
            .iter()
            .any(|name| other.lock_file_names().contains(name))
    }

    pub(in crate::foundation::remote_binary_db) const fn journal_file_name(self) -> &'static str {
        match self {
            Self::General => "global.write.journal",
            Self::ServerWorkflow => "server-workflow.write.journal",
            Self::ServerPlan => "server-plan.write.journal",
            Self::ServerQueue => "server-queue.write.journal",
            Self::ServerRepositoryPack => "server-repository-pack.write.journal",
            Self::ServerContent => "server-content.write.journal",
            Self::ServerTaskStart => "server-task-start.write.journal",
            Self::ServerLand => "server-land.write.journal",
            Self::ServerRemoteSyncCommit => "server-remote-sync-commit.write.journal",
        }
    }

    pub const fn all_write_lock_file_names() -> &'static [&'static str] {
        &[
            "global.write.lock",
            "server-content.write.lock",
            "server-plan.write.lock",
            "server-queue.write.lock",
            "server-repository-pack.write.lock",
            "server-workflow.write.lock",
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryDbCompatibilityPrimitive {
    ErrorKind,
    StorePath,
    FileFamily,
    FileId,
    PayloadFileId,
    IndexId,
    PayloadRange,
    ReadScope,
    ReadTxn,
    WriteTxn,
    CommandScope,
    FsyncPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BinaryDbCompatibilityContractRow {
    pub primitive: BinaryDbCompatibilityPrimitive,
    pub server_type: &'static str,
    pub ait_core_reference: &'static str,
    pub guarantee: &'static str,
}

pub const AIT_CORE_BINARY_DB_COMPATIBILITY_CONTRACT: &[BinaryDbCompatibilityContractRow] = &[
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::ErrorKind,
        server_type: "BinaryDbErrorKind",
        ait_core_reference: "ait_core::binary_db::BinaryDbErrorKind",
        guarantee: "same error categories: retryable busy, corruption, layout mismatch, missing data, invalid domain data, io, unsupported, other",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::StorePath,
        server_type: "StorePath",
        ait_core_reference: "ait_core::binary_db::StorePath",
        guarantee: "relative and authority-root paths remain PathBuf-backed and never require ait external materialization",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::FileFamily,
        server_type: "BinaryDbFileFamily",
        ait_core_reference: "ait_core::binary_db::{BinaryFileId, BinaryPayloadFileId, BinaryIndexId}",
        guarantee: "every record, payload, and index id carries an immutable logical file family used to authorize writes before journal tracking or filesystem mutation",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::FileId,
        server_type: "BinaryFileId",
        ait_core_reference: "ait_core::binary_db::BinaryFileId",
        guarantee: "relative path, layout id, fixed record size, and server file family are preserved by value",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::PayloadFileId,
        server_type: "BinaryPayloadFileId",
        ait_core_reference: "ait_core::binary_db::BinaryPayloadFileId",
        guarantee: "relative path, layout id, and server file family are preserved by value",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::IndexId,
        server_type: "BinaryIndexId",
        ait_core_reference: "ait_core::binary_db::BinaryIndexId",
        guarantee: "relative path, layout id, optional fixed key width, record-index-plus-one encoding, and server file family are preserved by value",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::PayloadRange,
        server_type: "PayloadRange",
        ait_core_reference: "ait_core::binary_db::PayloadRange",
        guarantee: "payload offset is u64 and payload length is u32",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::ReadScope,
        server_type: "BinaryDbReadScope",
        ait_core_reference: "ait_core::binary_db::BinaryDbReadScope",
        guarantee: "all-family remains the compatibility default while logical content and Plan reads lock only matching writer families; server-only workflow, queue, and repository-pack families extend the same typed scope model",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::ReadTxn,
        server_type: "BinaryDbReadTxn",
        ait_core_reference: "ait_core::binary_db::BinaryDbReadTxn",
        guarantee: "read transaction acquires shared process locks over its typed Binary DB read scope before delegating reads and releases without clearing write-lock metadata",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::WriteTxn,
        server_type: "BinaryDbWriteTxn",
        ait_core_reference: "ait_core::binary_db::BinaryDbWriteTxn",
        guarantee: "write transaction owns a command scope, write context, durable default fsync policy, append and rollback-safe overwrite boundaries, commit, abort, blocking begin, and explicit try_begin busy checks",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::CommandScope,
        server_type: "BinaryDbCommandScope",
        ait_core_reference: "ait_core::binary_db::BinaryDbCommandScope",
        guarantee: "command scopes map to named write lock families; server content and workflow have independent lock names",
    },
    BinaryDbCompatibilityContractRow {
        primitive: BinaryDbCompatibilityPrimitive::FsyncPolicy,
        server_type: "BinaryDbFsyncPolicy",
        ait_core_reference: "ait_core::binary_db::BinaryDbFsyncPolicy",
        guarantee: "store-backed fsync is the production default while std and noop remain explicit policies; all expose full-file, data-and-readable-length, and directory durability hooks",
    },
];

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RepoId(String);

impl RepoId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RepoName(String);

impl RepoName {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StoreGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl fmt::Display for RepoName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}
