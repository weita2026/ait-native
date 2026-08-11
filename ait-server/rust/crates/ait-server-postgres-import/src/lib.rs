//! One-shot PostgreSQL-to-Binary-DB-v0 operational migration.
//!
//! This crate is intentionally outside both release server crates.  It is the
//! only workspace member allowed to retain a PostgreSQL client after the
//! Binary-only runtime cutover.

mod activation;
mod conversion;
mod domain;
mod generation_inventory;
mod json;
mod legacy_alias_source;
mod postgres_source;
mod recovery_audit;
mod recovery_job_policy;
mod types;
mod u64_second_upgrade;

pub use activation::{activate_generation, ActivateRequest, ActivateResult};
pub use conversion::{stage_snapshot, StageRequest, StageResult};
pub use postgres_source::read_source_snapshot;
pub use recovery_audit::{audit_generation, AuditGenerationRequest, AuditGenerationResult};
pub use types::{
    SourceColumn, SourceConstraint, SourceInventory, SourceJobRow, SourceRepositoryRow,
    SourceSnapshot,
};
pub use u64_second_upgrade::{
    upgrade_u64_seconds, UpgradeU64SecondsRequest, UpgradeU64SecondsResult,
    U32_TIME_V0_SOURCE_SELECTOR, U64_SECOND_V0_TARGET_SELECTOR,
};

pub fn stage_from_postgres(request: StageRequest) -> Result<StageResult, String> {
    let snapshot = read_source_snapshot(&request.dsn)?;
    stage_snapshot(&snapshot, request)
}
