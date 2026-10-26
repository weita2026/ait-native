//! Retained Snapshot, byte and workspace-lock SDK capabilities.
use crate::{repository::repository, JsonInput};
use ait_cli::primitives;
use ait_core::json_support::JsonValue;
use napi::{bindgen_prelude::Buffer, Error, Result};
use napi_derive::napi;

#[napi]
pub struct WorkspaceCommandLock {
    lock: Option<ait_cli::workspace_lock::WorkspaceCommandLock>,
    metadata: JsonValue,
}

#[napi]
impl WorkspaceCommandLock {
    #[napi(constructor)]
    pub fn new(root: String, command_name: String) -> Result<Self> {
        let lock = ait_cli::workspace_lock::WorkspaceCommandLock::acquire(
            &repository(&root)?,
            &command_name,
        )
        .map_err(Error::from_reason)?;
        let metadata = lock.metadata().clone();
        Ok(Self {
            lock: Some(lock),
            metadata,
        })
    }

    #[napi]
    pub fn metadata(&self) -> JsonValue {
        self.metadata.clone()
    }

    #[napi]
    pub fn close(&mut self) {
        self.lock.take();
    }

    #[napi]
    pub fn release(&mut self) {
        self.close();
    }

    #[napi]
    pub fn is_closed(&self) -> bool {
        self.lock.is_none()
    }
}

#[napi]
pub fn workspace_command_lock_path(root: String) -> Result<String> {
    Ok(
        ait_cli::workspace_lock::workspace_command_lock_path(&repository(&root)?)
            .to_string_lossy()
            .into_owned(),
    )
}

#[napi]
pub fn task_workflow_snapshot_chain(root: String, snapshot_id: String) -> Result<JsonValue> {
    primitives::snapshot_chain(&repository(&root)?, &snapshot_id).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_snapshot_exists(root: String, snapshot_id: String) -> Result<bool> {
    primitives::snapshot_exists(&repository(&root)?, &snapshot_id).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_blob_read_bytes(root: String, blob_id: String) -> Result<Buffer> {
    primitives::blob_read_bytes(&repository(&root)?, &blob_id)
        .map(Buffer::from)
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_blob_ensure_bytes(
    root: String,
    data: Buffer,
    path_hint: Option<String>,
) -> Result<String> {
    primitives::blob_ensure_bytes(&repository(&root)?, data.as_ref(), path_hint.as_deref())
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_workspace_delta(
    root: String,
    snapshot_id: Option<String>,
) -> Result<JsonValue> {
    primitives::workspace_delta(&repository(&root)?, snapshot_id.as_deref())
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorkspaceRestoreOptions {
    pub target_snapshot_id: Option<String>,
    pub baseline_snapshot_id: Option<String>,
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_workspace_restore(
    root: String,
    options: Option<WorkspaceRestoreOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::workspace_restore(
        &repository(&root)?,
        options.target_snapshot_id.as_deref(),
        options.baseline_snapshot_id.as_deref(),
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_workspace_restore_paths(
    root: String,
    paths: Vec<String>,
    options: Option<WorkspaceRestoreOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::workspace_restore_paths(
        &repository(&root)?,
        options.target_snapshot_id.as_deref(),
        &paths,
        options.baseline_snapshot_id.as_deref(),
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

pub(crate) fn native_count(input: Option<JsonInput>, default: usize, name: &str) -> Result<usize> {
    input
        .map(|input| {
            input
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    Error::from_reason(format!("{name} must be a nonnegative native-size integer."))
                })
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

#[napi(object)]
#[derive(Default)]
pub struct SnapshotDiffOptions {
    pub include_text: Option<bool>,
    pub max_bytes: Option<JsonInput>,
}

#[napi]
pub fn task_workflow_snapshot_diff(
    root: String,
    old_snapshot_id: String,
    new_snapshot_id: String,
    options: Option<SnapshotDiffOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::snapshot_diff(
        &repository(&root)?,
        &old_snapshot_id,
        &new_snapshot_id,
        options.include_text.unwrap_or(false),
        native_count(
            options.max_bytes,
            ait_core::object_diff::DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
            "max_bytes",
        )?,
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct SnapshotMutationOptions {
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn snapshot_diff(
    root: String,
    older: String,
    newer: String,
    options: Option<SnapshotDiffOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    ait_cli::binding_services::diff_snapshot_selectors(
        &repository(&root)?,
        &older,
        &newer,
        options.include_text.unwrap_or(false),
        native_count(
            options.max_bytes,
            ait_core::object_diff::DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
            "max_bytes",
        )?,
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn snapshot_revert(
    root: String,
    selector: String,
    options: Option<SnapshotMutationOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    ait_cli::binding_services::revert_snapshot_selector(
        &repository(&root)?,
        &selector,
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn snapshot_replay(
    root: String,
    selector: String,
    onto: Option<String>,
    options: Option<SnapshotMutationOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    ait_cli::binding_services::replay_snapshot_selector(
        &repository(&root)?,
        &selector,
        onto.as_deref(),
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct SnapshotRestoreLinesOptions {
    pub line: Option<JsonInput>,
    pub start_line: Option<JsonInput>,
    pub end_line: Option<JsonInput>,
    pub yes: Option<bool>,
}

pub(crate) fn optional_count(input: Option<JsonInput>, name: &str) -> Result<Option<usize>> {
    input
        .map(|value| native_count(Some(value), 0, name))
        .transpose()
}

#[napi]
pub fn task_workflow_snapshot_restore_lines(
    root: String,
    snapshot_id: String,
    path: String,
    options: Option<SnapshotRestoreLinesOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let request = ait_cli::snapshot_restore_surface::SnapshotRestoreLinesRequest {
        snapshot_id,
        path,
        line: optional_count(options.line, "line")?,
        start_line: optional_count(options.start_line, "start_line")?,
        end_line: optional_count(options.end_line, "end_line")?,
        apply: options.yes.unwrap_or(false),
    };
    ait_cli::snapshot_restore_surface::snapshot_restore_lines(&repository(&root)?, &request)
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_snapshot_revert(
    root: String,
    snapshot_id: String,
    options: Option<SnapshotMutationOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::snapshot_revert(
        &repository(&root)?,
        &snapshot_id,
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_snapshot_replay(
    root: String,
    snapshot_id: String,
    onto: String,
    options: Option<SnapshotMutationOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::snapshot_replay(
        &repository(&root)?,
        &snapshot_id,
        &onto,
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct SnapshotCreateExplicitOptions {
    pub message: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub update_line_ref: Option<bool>,
    pub touch_line: Option<bool>,
    pub record_workflow_metadata: Option<bool>,
}

#[napi]
pub fn task_workflow_snapshot_create_explicit(
    root: String,
    repo_name: String,
    line_name: String,
    options: Option<SnapshotCreateExplicitOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::snapshot_create_explicit(
        &repository(&root)?,
        &repo_name,
        &line_name,
        options.message.as_deref(),
        options.parent_snapshot_id.as_deref(),
        options.update_line_ref.unwrap_or(true),
        options.touch_line.unwrap_or(true),
        options.record_workflow_metadata.unwrap_or(true),
    )
    .map_err(Error::from_reason)
}

macro_rules! snapshot_text_output {
    ($(($name:ident, $target:path)),* $(,)?) => {$(
        #[napi]
        pub fn $name(payload: JsonInput) -> Result<String> { $target(&payload).map_err(Error::from_reason) }
    )*};
}
snapshot_text_output! {
    (format_snapshot_restore_lines, ait_cli::binding_output::format_snapshot_restore_lines),
    (format_snapshot_revert, ait_cli::binding_output::format_snapshot_revert),
    (format_snapshot_replay, ait_cli::binding_output::format_snapshot_replay),
    (format_snapshot_diff, ait_cli::binding_output::format_snapshot_diff),
}
