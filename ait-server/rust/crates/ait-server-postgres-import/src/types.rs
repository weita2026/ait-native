use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SOURCE_DATABASE: &str = "ait_native";
pub const REPOSITORY_TABLE: &str = "ait_native_content.repositories";
pub const JOB_TABLE: &str = "ait_native_control.jobs";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceColumn {
    pub table_name: String,
    pub column_name: String,
    pub sql_type: String,
    pub not_null: bool,
    pub generated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceConstraint {
    pub table_name: String,
    pub constraint_name: String,
    pub constraint_type: String,
    pub definition: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceInventory {
    pub columns: Vec<SourceColumn>,
    pub constraints: Vec<SourceConstraint>,
    pub repository_count: u64,
    pub job_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRepositoryRow {
    pub repo_id: String,
    pub repo_name: String,
    pub default_line: String,
    pub id_namespace_prefix: String,
    pub policy_json: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lifecycle_state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceJobRow {
    pub job_id: i64,
    pub repo_name: String,
    pub repo_id: String,
    pub job_type: String,
    pub state: String,
    pub payload_json: String,
    pub result_json: String,
    pub attempt_count: i32,
    pub max_attempts: i32,
    pub available_at: DateTime<Utc>,
    pub locked_at: Option<DateTime<Utc>>,
    pub locked_by: Option<String>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSnapshot {
    pub database_name: String,
    pub inventory_before: SourceInventory,
    pub inventory_after: SourceInventory,
    pub repositories: Vec<SourceRepositoryRow>,
    pub jobs: Vec<SourceJobRow>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct SourceManifest {
    pub schema: String,
    pub status: String,
    pub authority_backend: String,
    pub layout_id: u32,
    pub repositories: Vec<SourceManifestRepository>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct SourceManifestRepository {
    pub repo_name: String,
    pub repo_id: String,
    pub storage_generation: u64,
    pub authority_relative_path: String,
}
