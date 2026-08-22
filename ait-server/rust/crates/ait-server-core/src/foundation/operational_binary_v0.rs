use crate::foundation::remote_binary_db::{
    BinaryDbError, BinaryDbFileFamily, BinaryIndexId, StoreResult,
};

pub const OPERATIONAL_V0_LAYOUT_ID: u32 = 1;
pub const OPERATIONAL_BIN_HEADER_SIZE: u64 = 4;

pub const OPERATIONAL_REPOSITORY_RECORD_SIZE: u32 = 33;
pub const OPERATIONAL_NAMESPACE_INDEX_RECORD_SIZE: u32 = 8;
const OPERATIONAL_NAMESPACE_INDEX_KEY_SIZE: u32 = 4;
pub const SERVER_WORKER_JOB_RECORD_SIZE: u32 = 52;
pub const SERVER_WORKER_READY_INDEX_RECORD_SIZE: u32 = 12;
pub const SERVER_WORKER_STATE_INDEX_RECORD_SIZE: u32 = 8;
const SERVER_WORKER_READY_INDEX_KEY_SIZE: u32 = 8;
const SERVER_WORKER_STATE_INDEX_KEY_SIZE: u32 = 4;

pub const REPOSITORY_META_TOMBSTONED: u8 = 1 << 7;
pub const REPOSITORY_META_KNOWN_MASK: u8 = REPOSITORY_META_TOMBSTONED;
pub const WORKER_JOB_META_TOMBSTONED: u8 = 1 << 7;
pub const WORKER_JOB_META_KNOWN_MASK: u8 = WORKER_JOB_META_TOMBSTONED;

pub const REPOSITORY_LIFECYCLE_ACTIVE: u8 = 1;
pub const REPOSITORY_LIFECYCLE_RETIRING: u8 = 2;
pub const REPOSITORY_LIFECYCLE_PURGED: u8 = 3;

pub const WORKER_JOB_KIND_CONTENT_GC: u8 = 2;
pub const WORKER_JOB_KIND_CONTENT_OPTIMIZE: u8 = 3;
pub const WORKER_JOB_KIND_CONTENT_PACK: u8 = 4;
pub const WORKER_JOB_KIND_LAND_PROCESS: u8 = 5;
pub const WORKER_JOB_KIND_MAIN_SEED_REFRESH: u8 = 6;
pub const WORKER_JOB_KIND_PATCHSET_CI: u8 = 7;
pub const WORKER_JOB_KIND_PATCHSET_CI_AGGREGATE: u8 = 8;
pub const WORKER_JOB_KIND_POLICY_EVALUATE: u8 = 9;
pub const WORKER_JOB_KIND_RECONCILE_REPO: u8 = 10;
pub const WORKER_JOB_KIND_REPO_CI: u8 = 11;

pub const WORKER_JOB_STATE_QUEUED: u8 = 1;
pub const WORKER_JOB_STATE_RUNNING: u8 = 2;
pub const WORKER_JOB_STATE_SUCCEEDED: u8 = 3;
pub const WORKER_JOB_STATE_FAILED: u8 = 4;

pub const WORKER_JOB_OUTCOME_NONE: u8 = 0;
pub const WORKER_JOB_OUTCOME_COMPLETED: u8 = 1;
pub const WORKER_JOB_OUTCOME_SKIPPED: u8 = 2;
pub const WORKER_JOB_OUTCOME_ATTACHED: u8 = 3;
pub const WORKER_JOB_OUTCOME_SUPERSEDED: u8 = 4;
pub const WORKER_JOB_OUTCOME_FAILED: u8 = 5;

pub const WORKER_JOB_ERROR_NONE: u16 = 0;
pub const WORKER_JOB_ERROR_RETRYABLE_EXECUTION: u16 = 1;
pub const WORKER_JOB_ERROR_TERMINAL_EXECUTION: u16 = 2;
pub const WORKER_JOB_ERROR_LEASE_EXPIRED: u16 = 3;

pub const SERVER_GLOBAL_OPERATIONAL_BIN_PATHS: &[&str] =
    &["repository.bin", "repository_payload.bin"];
pub const SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS: &[&str] = &["repository_namespace.idx"];
pub const SERVER_REPOSITORY_OPERATIONAL_BIN_PATHS: &[&str] = &["worker_job.bin"];
pub const SERVER_REPOSITORY_OPERATIONAL_INDEX_PATHS: &[&str] =
    &["worker_ready.idx", "worker_state.idx"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerOperationalRootKind {
    GlobalRegistry,
    RepositoryAuthority,
}

impl ServerOperationalRootKind {
    pub const fn bin_paths(self) -> &'static [&'static str] {
        match self {
            Self::GlobalRegistry => SERVER_GLOBAL_OPERATIONAL_BIN_PATHS,
            Self::RepositoryAuthority => SERVER_REPOSITORY_OPERATIONAL_BIN_PATHS,
        }
    }

    pub const fn index_paths(self) -> &'static [&'static str] {
        match self {
            Self::GlobalRegistry => SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS,
            Self::RepositoryAuthority => SERVER_REPOSITORY_OPERATIONAL_INDEX_PATHS,
        }
    }

    pub fn admits_path(self, path: &str) -> bool {
        self.bin_paths().contains(&path) || self.index_paths().contains(&path)
    }
}

pub fn validate_operational_root_path(
    root_kind: ServerOperationalRootKind,
    path: &str,
) -> StoreResult<()> {
    if root_kind.admits_path(path) {
        Ok(())
    } else {
        Err(invalid(format!(
            "{root_kind:?} does not admit operational path {path:?}"
        )))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OperationalRepositoryRecord {
    pub repository_meta: u8,
    pub lifecycle_kind: u8,
    pub namespace_ascii: [u8; 2],
    pub policy_flags: u8,
    pub payload_len: u32,
    pub payload_offset: u64,
    pub created_at_s: u64,
    pub updated_at_s: u64,
}

impl OperationalRepositoryRecord {
    pub fn is_tombstoned(self) -> bool {
        self.repository_meta & REPOSITORY_META_TOMBSTONED != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationalRepositoryPayload {
    pub repo_name: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OperationalNamespaceIndexRecord {
    pub namespace_ascii: [u8; 2],
    pub reserved0: u16,
    pub repository_index_plus1: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerWorkerJobRecord {
    pub job_meta: u8,
    pub job_kind: u8,
    pub state_kind: u8,
    pub outcome_kind: u8,
    pub attempt_count: u16,
    pub max_attempts: u16,
    pub error_kind: u16,
    pub reserved0: u16,
    pub patchset_index_plus1: u32,
    pub snapshot_index_plus1: u32,
    pub available_at_s: u64,
    pub locked_at_s: u64,
    pub created_at_s: u64,
    pub updated_at_s: u64,
}

impl ServerWorkerJobRecord {
    pub fn is_tombstoned(self) -> bool {
        self.job_meta & WORKER_JOB_META_TOMBSTONED != 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerWorkerReadyIndexRecord {
    pub available_at_s: u64,
    pub worker_job_index_plus1: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerWorkerStateIndexRecord {
    pub state_kind: u8,
    pub reserved0: u8,
    pub reserved1: u16,
    pub worker_job_index_plus1: u32,
}

pub struct ServerOperationalBinaryV0Codec;

impl ServerOperationalBinaryV0Codec {
    pub fn repository_namespace_index_file() -> BinaryIndexId {
        BinaryIndexId::new_fixed(
            "repository_namespace.idx",
            OPERATIONAL_V0_LAYOUT_ID,
            OPERATIONAL_NAMESPACE_INDEX_KEY_SIZE,
            true,
            BinaryDbFileFamily::Queue,
        )
    }

    pub fn worker_ready_index_file() -> BinaryIndexId {
        BinaryIndexId::new_fixed(
            "worker_ready.idx",
            OPERATIONAL_V0_LAYOUT_ID,
            SERVER_WORKER_READY_INDEX_KEY_SIZE,
            true,
            BinaryDbFileFamily::Queue,
        )
    }

    pub fn worker_state_index_file() -> BinaryIndexId {
        BinaryIndexId::new_fixed(
            "worker_state.idx",
            OPERATIONAL_V0_LAYOUT_ID,
            SERVER_WORKER_STATE_INDEX_KEY_SIZE,
            true,
            BinaryDbFileFamily::Queue,
        )
    }

    pub fn encode_repository(record: OperationalRepositoryRecord) -> StoreResult<Vec<u8>> {
        validate_repository(record)?;
        let mut out = Vec::with_capacity(OPERATIONAL_REPOSITORY_RECORD_SIZE as usize);
        out.push(record.repository_meta);
        out.push(record.lifecycle_kind);
        out.extend_from_slice(&record.namespace_ascii);
        out.push(record.policy_flags);
        push_u32(&mut out, record.payload_len);
        push_u64(&mut out, record.payload_offset);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.updated_at_s);
        finish_encode(
            out,
            OPERATIONAL_REPOSITORY_RECORD_SIZE,
            "OperationalRepositoryRecord",
        )
    }

    pub fn decode_repository(raw: &[u8]) -> StoreResult<OperationalRepositoryRecord> {
        let mut input = Cursor::new(
            raw,
            OPERATIONAL_REPOSITORY_RECORD_SIZE,
            "OperationalRepositoryRecord",
        )?;
        let record = OperationalRepositoryRecord {
            repository_meta: input.u8()?,
            lifecycle_kind: input.u8()?,
            namespace_ascii: input.take()?,
            policy_flags: input.u8()?,
            payload_len: input.u32()?,
            payload_offset: input.u64()?,
            created_at_s: input.u64()?,
            updated_at_s: input.u64()?,
        };
        input.finish()?;
        validate_repository(record)?;
        Ok(record)
    }

    pub fn encode_repository_payload(
        payload: &OperationalRepositoryPayload,
    ) -> StoreResult<Vec<u8>> {
        let name = payload.repo_name.as_bytes();
        if name.is_empty() {
            return Err(invalid("Repository name is empty"));
        }
        let name_len = u16::try_from(name.len())
            .map_err(|_| invalid("Repository name exceeds the u16 payload limit"))?;
        let mut out = Vec::with_capacity(2 + name.len());
        push_u16(&mut out, name_len);
        out.extend_from_slice(name);
        Ok(out)
    }

    pub fn decode_repository_payload(raw: &[u8]) -> StoreResult<OperationalRepositoryPayload> {
        if raw.len() < 2 {
            return Err(corrupt("OperationalRepositoryPayload is truncated"));
        }
        let name_len = usize::from(u16::from_le_bytes([raw[0], raw[1]]));
        if name_len == 0 || raw.len() != name_len + 2 {
            return Err(invalid(
                "OperationalRepositoryPayload length or name is invalid",
            ));
        }
        let repo_name = std::str::from_utf8(&raw[2..])
            .map_err(|_| invalid("Repository name is not valid UTF-8"))?
            .to_string();
        Ok(OperationalRepositoryPayload { repo_name })
    }

    pub fn validate_repository_payload_binding(
        record: OperationalRepositoryRecord,
        raw: &[u8],
    ) -> StoreResult<OperationalRepositoryPayload> {
        validate_repository(record)?;
        if usize::try_from(record.payload_len).ok() != Some(raw.len()) {
            return Err(invalid(
                "Repository payload locator length does not match payload bytes",
            ));
        }
        Self::decode_repository_payload(raw)
    }

    pub fn encode_namespace_index(record: OperationalNamespaceIndexRecord) -> StoreResult<Vec<u8>> {
        validate_namespace_index(record)?;
        let mut out = Vec::with_capacity(OPERATIONAL_NAMESPACE_INDEX_RECORD_SIZE as usize);
        out.extend_from_slice(&record.namespace_ascii);
        push_u16(&mut out, record.reserved0);
        push_u32(&mut out, record.repository_index_plus1);
        finish_encode(
            out,
            OPERATIONAL_NAMESPACE_INDEX_RECORD_SIZE,
            "OperationalNamespaceIndexRecord",
        )
    }

    pub fn decode_namespace_index(raw: &[u8]) -> StoreResult<OperationalNamespaceIndexRecord> {
        let mut input = Cursor::new(
            raw,
            OPERATIONAL_NAMESPACE_INDEX_RECORD_SIZE,
            "OperationalNamespaceIndexRecord",
        )?;
        let record = OperationalNamespaceIndexRecord {
            namespace_ascii: input.take()?,
            reserved0: input.u16()?,
            repository_index_plus1: input.u32()?,
        };
        input.finish()?;
        validate_namespace_index(record)?;
        Ok(record)
    }

    pub fn encode_worker_job(record: ServerWorkerJobRecord) -> StoreResult<Vec<u8>> {
        validate_worker_job(record)?;
        let mut out = Vec::with_capacity(SERVER_WORKER_JOB_RECORD_SIZE as usize);
        out.push(record.job_meta);
        out.push(record.job_kind);
        out.push(record.state_kind);
        out.push(record.outcome_kind);
        push_u16(&mut out, record.attempt_count);
        push_u16(&mut out, record.max_attempts);
        push_u16(&mut out, record.error_kind);
        push_u16(&mut out, record.reserved0);
        push_u32(&mut out, record.patchset_index_plus1);
        push_u32(&mut out, record.snapshot_index_plus1);
        push_u64(&mut out, record.available_at_s);
        push_u64(&mut out, record.locked_at_s);
        push_u64(&mut out, record.created_at_s);
        push_u64(&mut out, record.updated_at_s);
        finish_encode(out, SERVER_WORKER_JOB_RECORD_SIZE, "ServerWorkerJobRecord")
    }

    pub fn decode_worker_job(raw: &[u8]) -> StoreResult<ServerWorkerJobRecord> {
        let mut input = Cursor::new(raw, SERVER_WORKER_JOB_RECORD_SIZE, "ServerWorkerJobRecord")?;
        let record = ServerWorkerJobRecord {
            job_meta: input.u8()?,
            job_kind: input.u8()?,
            state_kind: input.u8()?,
            outcome_kind: input.u8()?,
            attempt_count: input.u16()?,
            max_attempts: input.u16()?,
            error_kind: input.u16()?,
            reserved0: input.u16()?,
            patchset_index_plus1: input.u32()?,
            snapshot_index_plus1: input.u32()?,
            available_at_s: input.u64()?,
            locked_at_s: input.u64()?,
            created_at_s: input.u64()?,
            updated_at_s: input.u64()?,
        };
        input.finish()?;
        validate_worker_job(record)?;
        Ok(record)
    }

    pub fn encode_worker_ready_index(record: ServerWorkerReadyIndexRecord) -> StoreResult<Vec<u8>> {
        validate_worker_ready_index(record)?;
        let mut out = Vec::with_capacity(SERVER_WORKER_READY_INDEX_RECORD_SIZE as usize);
        push_u64(&mut out, record.available_at_s);
        push_u32(&mut out, record.worker_job_index_plus1);
        finish_encode(
            out,
            SERVER_WORKER_READY_INDEX_RECORD_SIZE,
            "ServerWorkerReadyIndexRecord",
        )
    }

    pub fn decode_worker_ready_index(raw: &[u8]) -> StoreResult<ServerWorkerReadyIndexRecord> {
        let mut input = Cursor::new(
            raw,
            SERVER_WORKER_READY_INDEX_RECORD_SIZE,
            "ServerWorkerReadyIndexRecord",
        )?;
        let record = ServerWorkerReadyIndexRecord {
            available_at_s: input.u64()?,
            worker_job_index_plus1: input.u32()?,
        };
        input.finish()?;
        validate_worker_ready_index(record)?;
        Ok(record)
    }

    pub fn encode_worker_state_index(record: ServerWorkerStateIndexRecord) -> StoreResult<Vec<u8>> {
        validate_worker_state_index(record)?;
        let mut out = Vec::with_capacity(SERVER_WORKER_STATE_INDEX_RECORD_SIZE as usize);
        out.push(record.state_kind);
        out.push(record.reserved0);
        push_u16(&mut out, record.reserved1);
        push_u32(&mut out, record.worker_job_index_plus1);
        finish_encode(
            out,
            SERVER_WORKER_STATE_INDEX_RECORD_SIZE,
            "ServerWorkerStateIndexRecord",
        )
    }

    pub fn decode_worker_state_index(raw: &[u8]) -> StoreResult<ServerWorkerStateIndexRecord> {
        let mut input = Cursor::new(
            raw,
            SERVER_WORKER_STATE_INDEX_RECORD_SIZE,
            "ServerWorkerStateIndexRecord",
        )?;
        let record = ServerWorkerStateIndexRecord {
            state_kind: input.u8()?,
            reserved0: input.u8()?,
            reserved1: input.u16()?,
            worker_job_index_plus1: input.u32()?,
        };
        input.finish()?;
        validate_worker_state_index(record)?;
        Ok(record)
    }
}

fn validate_repository(record: OperationalRepositoryRecord) -> StoreResult<()> {
    if record.repository_meta & !REPOSITORY_META_KNOWN_MASK != 0 {
        return Err(invalid("Repository metadata has reserved bits"));
    }
    if !matches!(
        record.lifecycle_kind,
        REPOSITORY_LIFECYCLE_ACTIVE | REPOSITORY_LIFECYCLE_RETIRING | REPOSITORY_LIFECYCLE_PURGED
    ) {
        return Err(invalid("Repository lifecycle kind is reserved"));
    }
    validate_namespace(record.namespace_ascii)?;
    if !(3..=65_537).contains(&record.payload_len)
        || record.payload_offset < OPERATIONAL_BIN_HEADER_SIZE
    {
        return Err(invalid("Repository payload locator is invalid"));
    }
    if record.created_at_s == 0
        || record.updated_at_s == 0
        || record.updated_at_s < record.created_at_s
    {
        return Err(invalid("Repository timestamps are invalid"));
    }
    Ok(())
}

pub fn validate_namespace(namespace_ascii: [u8; 2]) -> StoreResult<()> {
    let [first, second] = namespace_ascii;
    if first == 0 {
        if second == 0 {
            return Ok(());
        }
        return Err(invalid("Repository namespace has a leading zero"));
    }
    if !is_namespace_byte(first) || (second != 0 && !is_namespace_byte(second)) {
        return Err(invalid("Repository namespace contains a forbidden byte"));
    }
    Ok(())
}

fn is_namespace_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn validate_namespace_index(record: OperationalNamespaceIndexRecord) -> StoreResult<()> {
    validate_namespace(record.namespace_ascii)?;
    if record.namespace_ascii == [0, 0]
        || record.reserved0 != 0
        || record.repository_index_plus1 == 0
    {
        return Err(invalid("Repository namespace index row is invalid"));
    }
    Ok(())
}

fn validate_worker_job(record: ServerWorkerJobRecord) -> StoreResult<()> {
    if record.job_meta & !WORKER_JOB_META_KNOWN_MASK != 0 || record.reserved0 != 0 {
        return Err(invalid("Worker Job metadata or reserved field is invalid"));
    }
    if !(WORKER_JOB_KIND_CONTENT_GC..=WORKER_JOB_KIND_REPO_CI).contains(&record.job_kind) {
        return Err(invalid("Worker Job kind is unassigned or reserved"));
    }
    if !matches!(
        record.state_kind,
        WORKER_JOB_STATE_QUEUED
            | WORKER_JOB_STATE_RUNNING
            | WORKER_JOB_STATE_SUCCEEDED
            | WORKER_JOB_STATE_FAILED
    ) {
        return Err(invalid("Worker Job state kind is reserved"));
    }
    if record.max_attempts == 0 || record.attempt_count > record.max_attempts {
        return Err(invalid("Worker Job attempt budget is invalid"));
    }
    validate_worker_job_references(record)?;
    validate_worker_job_state(record)?;
    if record.available_at_s == 0
        || record.created_at_s == 0
        || record.updated_at_s == 0
        || record.updated_at_s < record.created_at_s
    {
        return Err(invalid("Worker Job timestamps are invalid"));
    }
    if record.state_kind == WORKER_JOB_STATE_RUNNING {
        if record.locked_at_s < record.created_at_s
            || record.locked_at_s > record.updated_at_s
            || record.locked_at_s == 0
        {
            return Err(invalid("running Worker Job lock time is invalid"));
        }
    } else if record.locked_at_s != 0 {
        return Err(invalid("non-running Worker Job retains a lock time"));
    }
    Ok(())
}

fn validate_worker_job_references(record: ServerWorkerJobRecord) -> StoreResult<()> {
    let patchset = record.patchset_index_plus1;
    let snapshot = record.snapshot_index_plus1;
    let valid = match record.job_kind {
        WORKER_JOB_KIND_CONTENT_GC
        | WORKER_JOB_KIND_CONTENT_OPTIMIZE
        | WORKER_JOB_KIND_CONTENT_PACK
        | WORKER_JOB_KIND_RECONCILE_REPO => patchset == 0 && snapshot == 0,
        WORKER_JOB_KIND_LAND_PROCESS
        | WORKER_JOB_KIND_PATCHSET_CI
        | WORKER_JOB_KIND_PATCHSET_CI_AGGREGATE
        | WORKER_JOB_KIND_POLICY_EVALUATE => patchset != 0 && snapshot == 0,
        WORKER_JOB_KIND_MAIN_SEED_REFRESH => patchset != 0,
        WORKER_JOB_KIND_REPO_CI => patchset == 0 && snapshot != 0,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "Worker Job domain references do not match its fixed kind",
        ))
    }
}

fn validate_worker_job_state(record: ServerWorkerJobRecord) -> StoreResult<()> {
    let valid_error = matches!(
        record.error_kind,
        WORKER_JOB_ERROR_NONE
            | WORKER_JOB_ERROR_RETRYABLE_EXECUTION
            | WORKER_JOB_ERROR_TERMINAL_EXECUTION
            | WORKER_JOB_ERROR_LEASE_EXPIRED
    );
    if !valid_error {
        return Err(invalid("Worker Job error kind is reserved"));
    }
    let valid = match record.state_kind {
        WORKER_JOB_STATE_QUEUED | WORKER_JOB_STATE_RUNNING => {
            record.outcome_kind == WORKER_JOB_OUTCOME_NONE
                && matches!(
                    record.error_kind,
                    WORKER_JOB_ERROR_NONE
                        | WORKER_JOB_ERROR_RETRYABLE_EXECUTION
                        | WORKER_JOB_ERROR_LEASE_EXPIRED
                )
        }
        WORKER_JOB_STATE_SUCCEEDED => {
            matches!(
                record.outcome_kind,
                WORKER_JOB_OUTCOME_COMPLETED
                    | WORKER_JOB_OUTCOME_SKIPPED
                    | WORKER_JOB_OUTCOME_ATTACHED
                    | WORKER_JOB_OUTCOME_SUPERSEDED
            ) && record.error_kind == WORKER_JOB_ERROR_NONE
        }
        WORKER_JOB_STATE_FAILED => {
            record.outcome_kind == WORKER_JOB_OUTCOME_FAILED
                && matches!(
                    record.error_kind,
                    WORKER_JOB_ERROR_TERMINAL_EXECUTION | WORKER_JOB_ERROR_LEASE_EXPIRED
                )
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "Worker Job state, outcome, and error combination is invalid",
        ))
    }
}

fn validate_worker_ready_index(record: ServerWorkerReadyIndexRecord) -> StoreResult<()> {
    if record.available_at_s == 0 || record.worker_job_index_plus1 == 0 {
        Err(invalid("Worker ready index row is invalid"))
    } else {
        Ok(())
    }
}

fn validate_worker_state_index(record: ServerWorkerStateIndexRecord) -> StoreResult<()> {
    if !matches!(
        record.state_kind,
        WORKER_JOB_STATE_QUEUED
            | WORKER_JOB_STATE_RUNNING
            | WORKER_JOB_STATE_SUCCEEDED
            | WORKER_JOB_STATE_FAILED
    ) || record.reserved0 != 0
        || record.reserved1 != 0
        || record.worker_job_index_plus1 == 0
    {
        Err(invalid("Worker state index row is invalid"))
    } else {
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::invalid_domain_data(message)
}

fn corrupt(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::corruption(message)
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn finish_encode(out: Vec<u8>, size: u32, label: &str) -> StoreResult<Vec<u8>> {
    if out.len() == size as usize {
        Ok(out)
    } else {
        Err(corrupt(format!(
            "{label} encoded {} bytes instead of {size}",
            out.len()
        )))
    }
}

struct Cursor<'a> {
    raw: &'a [u8],
    offset: usize,
    label: &'static str,
}

impl<'a> Cursor<'a> {
    fn new(raw: &'a [u8], size: u32, label: &'static str) -> StoreResult<Self> {
        if raw.len() != size as usize {
            return Err(corrupt(format!(
                "{label} requires {size} bytes, got {}",
                raw.len()
            )));
        }
        Ok(Self {
            raw,
            offset: 0,
            label,
        })
    }

    fn take<const N: usize>(&mut self) -> StoreResult<[u8; N]> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| corrupt(format!("{} cursor overflow", self.label)))?;
        let bytes = self
            .raw
            .get(self.offset..end)
            .ok_or_else(|| corrupt(format!("{} is truncated", self.label)))?;
        self.offset = end;
        Ok(bytes.try_into().expect("fixed slice width"))
    }

    fn u8(&mut self) -> StoreResult<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> StoreResult<u16> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    fn u32(&mut self) -> StoreResult<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> StoreResult<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn finish(self) -> StoreResult<()> {
        if self.offset == self.raw.len() {
            Ok(())
        } else {
            Err(corrupt(format!(
                "{} has {} trailing bytes",
                self.label,
                self.raw.len() - self.offset
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> OperationalRepositoryRecord {
        OperationalRepositoryRecord {
            repository_meta: 0,
            lifecycle_kind: REPOSITORY_LIFECYCLE_ACTIVE,
            namespace_ascii: *b"ac",
            policy_flags: 0b0101_0101,
            payload_len: 10,
            payload_offset: 4,
            created_at_s: 100,
            updated_at_s: 101,
        }
    }

    fn queued_patchset_ci() -> ServerWorkerJobRecord {
        ServerWorkerJobRecord {
            job_meta: 0,
            job_kind: WORKER_JOB_KIND_PATCHSET_CI,
            state_kind: WORKER_JOB_STATE_QUEUED,
            outcome_kind: WORKER_JOB_OUTCOME_NONE,
            attempt_count: 0,
            max_attempts: 3,
            error_kind: WORKER_JOB_ERROR_NONE,
            reserved0: 0,
            patchset_index_plus1: 8,
            snapshot_index_plus1: 0,
            available_at_s: 100,
            locked_at_s: 0,
            created_at_s: 90,
            updated_at_s: 91,
        }
    }

    #[test]
    fn operational_records_match_frozen_golden_bytes() {
        let repository_raw =
            ServerOperationalBinaryV0Codec::encode_repository(repository()).unwrap();
        assert_eq!(
            repository_raw.len(),
            OPERATIONAL_REPOSITORY_RECORD_SIZE as usize
        );
        assert_eq!(
            repository_raw,
            [
                0, 1, b'a', b'c', 85, 10, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 100, 0, 0, 0, 0, 0, 0,
                0, 101, 0, 0, 0, 0, 0, 0, 0,
            ]
        );
        assert_eq!(
            ServerOperationalBinaryV0Codec::decode_repository(&repository_raw).unwrap(),
            repository()
        );

        let job = queued_patchset_ci();
        let job_raw = ServerOperationalBinaryV0Codec::encode_worker_job(job).unwrap();
        assert_eq!(job_raw.len(), SERVER_WORKER_JOB_RECORD_SIZE as usize);
        assert_eq!(
            job_raw,
            [
                0, 7, 1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 100, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 90, 0, 0, 0, 0, 0, 0, 0, 91, 0, 0, 0, 0, 0, 0, 0,
            ]
        );
        assert_eq!(
            ServerOperationalBinaryV0Codec::decode_worker_job(&job_raw).unwrap(),
            job
        );
    }

    #[test]
    fn repository_payload_and_indexes_are_exact_width() {
        assert_eq!(
            ServerOperationalBinaryV0Codec::repository_namespace_index_file().fixed_key_size(),
            Some(4)
        );
        assert_eq!(
            ServerOperationalBinaryV0Codec::worker_ready_index_file().fixed_key_size(),
            Some(8)
        );
        assert_eq!(
            ServerOperationalBinaryV0Codec::worker_state_index_file().fixed_key_size(),
            Some(4)
        );
        assert!(
            ServerOperationalBinaryV0Codec::repository_namespace_index_file()
                .stores_record_index_plus_one()
        );
        assert!(ServerOperationalBinaryV0Codec::worker_ready_index_file()
            .stores_record_index_plus_one());
        assert!(ServerOperationalBinaryV0Codec::worker_state_index_file()
            .stores_record_index_plus_one());
        let payload = OperationalRepositoryPayload {
            repo_name: "ait-core".to_string(),
        };
        let raw = ServerOperationalBinaryV0Codec::encode_repository_payload(&payload).unwrap();
        assert_eq!(raw, [8, 0, b'a', b'i', b't', b'-', b'c', b'o', b'r', b'e']);
        assert_eq!(
            ServerOperationalBinaryV0Codec::decode_repository_payload(&raw).unwrap(),
            payload
        );
        assert!(
            ServerOperationalBinaryV0Codec::validate_repository_payload_binding(repository(), &raw)
                .is_ok()
        );

        let namespace = OperationalNamespaceIndexRecord {
            namespace_ascii: *b"ac",
            reserved0: 0,
            repository_index_plus1: 1,
        };
        let namespace_raw =
            ServerOperationalBinaryV0Codec::encode_namespace_index(namespace).unwrap();
        assert_eq!(namespace_raw, [b'a', b'c', 0, 0, 1, 0, 0, 0]);
        assert_eq!(
            ServerOperationalBinaryV0Codec::decode_namespace_index(&namespace_raw).unwrap(),
            namespace
        );

        let ready = ServerWorkerReadyIndexRecord {
            available_at_s: 100,
            worker_job_index_plus1: 2,
        };
        let ready_raw = ServerOperationalBinaryV0Codec::encode_worker_ready_index(ready).unwrap();
        assert_eq!(ready_raw, [100, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0]);

        let state = ServerWorkerStateIndexRecord {
            state_kind: WORKER_JOB_STATE_QUEUED,
            reserved0: 0,
            reserved1: 0,
            worker_job_index_plus1: 2,
        };
        let state_raw = ServerOperationalBinaryV0Codec::encode_worker_state_index(state).unwrap();
        assert_eq!(state_raw, [1, 0, 0, 0, 2, 0, 0, 0]);
    }

    #[test]
    fn namespace_accepts_empty_and_one_byte_but_fails_closed() {
        assert!(validate_namespace([0, 0]).is_ok());
        assert!(validate_namespace([b'a', 0]).is_ok());
        assert!(validate_namespace(*b"A-").is_ok());
        for invalid_namespace in [[0, b'a'], [b'/', 0], [b'a', b'/'], [0xff, 0]] {
            assert!(validate_namespace(invalid_namespace).is_err());
        }
    }

    #[test]
    fn worker_job_kind_references_and_state_are_fixed() {
        let mut job = queued_patchset_ci();
        job.patchset_index_plus1 = 0;
        assert!(ServerOperationalBinaryV0Codec::encode_worker_job(job).is_err());

        let mut job = queued_patchset_ci();
        job.snapshot_index_plus1 = 1;
        assert!(ServerOperationalBinaryV0Codec::encode_worker_job(job).is_err());

        let mut job = queued_patchset_ci();
        job.state_kind = WORKER_JOB_STATE_RUNNING;
        job.attempt_count = 1;
        job.locked_at_s = 90;
        assert!(ServerOperationalBinaryV0Codec::encode_worker_job(job).is_ok());

        job.outcome_kind = WORKER_JOB_OUTCOME_COMPLETED;
        assert!(ServerOperationalBinaryV0Codec::encode_worker_job(job).is_err());

        job.outcome_kind = WORKER_JOB_OUTCOME_NONE;
        job.error_kind = WORKER_JOB_ERROR_TERMINAL_EXECUTION;
        assert!(ServerOperationalBinaryV0Codec::encode_worker_job(job).is_err());
    }

    #[test]
    fn operational_root_kinds_never_admit_each_others_files() {
        for path in SERVER_GLOBAL_OPERATIONAL_BIN_PATHS
            .iter()
            .chain(SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS)
        {
            assert!(ServerOperationalRootKind::GlobalRegistry.admits_path(path));
            assert!(!ServerOperationalRootKind::RepositoryAuthority.admits_path(path));
        }
        for path in SERVER_REPOSITORY_OPERATIONAL_BIN_PATHS
            .iter()
            .chain(SERVER_REPOSITORY_OPERATIONAL_INDEX_PATHS)
        {
            assert!(ServerOperationalRootKind::RepositoryAuthority.admits_path(path));
            assert!(!ServerOperationalRootKind::GlobalRegistry.admits_path(path));
        }
        assert!(validate_operational_root_path(
            ServerOperationalRootKind::GlobalRegistry,
            "worker_job.bin"
        )
        .is_err());
        assert!(validate_operational_root_path(
            ServerOperationalRootKind::RepositoryAuthority,
            "repository.bin"
        )
        .is_err());
    }

    #[test]
    fn reserved_values_and_corrupt_widths_fail_closed() {
        let mut repository = repository();
        repository.repository_meta = 1;
        assert!(ServerOperationalBinaryV0Codec::encode_repository(repository).is_err());

        let mut job = queued_patchset_ci();
        job.job_kind = 1;
        assert!(ServerOperationalBinaryV0Codec::encode_worker_job(job).is_err());

        assert!(ServerOperationalBinaryV0Codec::decode_repository(&[0; 24]).is_err());
        assert!(ServerOperationalBinaryV0Codec::decode_worker_job(&[0; 35]).is_err());
        assert!(ServerOperationalBinaryV0Codec::decode_worker_ready_index(&[0; 7]).is_err());
    }
}
