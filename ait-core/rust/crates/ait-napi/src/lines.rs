use crate::{repository::repository, JsonInput};
use ait_cli::{binding_output as output, binding_services as services, primitives};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct LineListOptions {
    pub include_all: Option<bool>,
    pub archived: Option<bool>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_line_list(
    root: String,
    options: Option<LineListOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::line_list(
        &repository(&root)?,
        options.include_all.unwrap_or(false),
        options.archived.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn line_inventory(root: String, options: Option<LineListOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    services::line_inventory(
        &repository(&root)?,
        options.include_all.unwrap_or(false),
        options.archived.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn format_line_list(inventory: JsonInput, options: Option<LineListOptions>) -> Result<String> {
    let options = options.unwrap_or_default();
    output::format_line_list(
        &inventory,
        options.include_all.unwrap_or(false) || options.archived.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct LineCreateOptions {
    pub from_snapshot: Option<String>,
    pub switch: Option<bool>,
}

#[napi]
pub fn task_workflow_line_create(
    root: String,
    name: String,
    options: Option<LineCreateOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::line_create(
        &repository(&root)?,
        &name,
        options.from_snapshot.as_deref(),
        options.switch.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct LineSwitchOptions {
    pub restore: Option<bool>,
    pub force: Option<bool>,
}

#[napi]
pub fn task_workflow_line_switch(
    root: String,
    name: String,
    options: Option<LineSwitchOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::line_switch(
        &repository(&root)?,
        &name,
        options.restore.unwrap_or(false),
        options.force.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_line_show(root: String, name: Option<String>) -> Result<JsonValue> {
    primitives::line_show(&repository(&root)?, name.as_deref()).map_err(Error::from_reason)
}

#[napi]
pub fn line_show(
    root: String,
    name: Option<String>,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    services::show_line(&repository(&root)?, name.as_deref(), remote_name.as_deref())
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_line_archive(
    root: String,
    name: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    primitives::line_archive(&repository(&root)?, &name, remote_name.as_deref())
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_line_rename(
    root: String,
    old_name: String,
    new_name: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    primitives::line_rename(
        &repository(&root)?,
        &old_name,
        &new_name,
        remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct LineDeleteOptions {
    pub remote_name: Option<String>,
    pub confirm: Option<bool>,
}

#[napi]
pub fn task_workflow_line_delete(
    root: String,
    name: String,
    options: Option<LineDeleteOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::line_delete(
        &repository(&root)?,
        &name,
        options.remote_name.as_deref(),
        options.confirm.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_line_set_head(
    root: String,
    name: String,
    snapshot_id: Option<String>,
) -> Result<JsonValue> {
    primitives::line_set_head(&repository(&root)?, &name, snapshot_id.as_deref())
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct LineMergeOptions {
    pub source: Option<String>,
    pub message: Option<String>,
    pub continue_merge: Option<bool>,
    pub abort_merge: Option<bool>,
}

#[napi]
pub fn line_merge(root: String, options: Option<LineMergeOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::line_merge(
        &repository(&root)?,
        options.source.as_deref(),
        options.message.as_deref(),
        options.continue_merge.unwrap_or(false),
        options.abort_merge.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct LineCleanupOptions {
    pub idle_for: Option<String>,
    pub cleanup_kind: Option<String>,
    pub limit: Option<JsonInput>,
    pub include_protected: Option<bool>,
    pub apply: Option<bool>,
}

fn cleanup_limit(value: Option<JsonInput>) -> Result<Option<usize>> {
    value
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| Error::from_reason("limit must be a native-size unsigned integer."))
        })
        .transpose()
}

#[napi]
pub fn task_workflow_line_cleanup(
    root: String,
    options: Option<LineCleanupOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::line_cleanup(
        &repository(&root)?,
        options.idle_for.as_deref(),
        options.cleanup_kind.as_deref(),
        cleanup_limit(options.limit)?,
        options.include_protected.unwrap_or(false),
        options.apply.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct LineCleanupOutputOptions {
    pub all: Option<bool>,
    pub include_protected: Option<bool>,
    pub cleanup_kind: Option<String>,
    pub limit: Option<JsonInput>,
}

#[napi]
pub fn format_line_cleanup(
    payload: JsonInput,
    idle_for: String,
    options: Option<LineCleanupOutputOptions>,
) -> Result<String> {
    let options = options.unwrap_or_default();
    output::format_line_cleanup(
        &payload,
        options.all.unwrap_or(false),
        options.include_protected.unwrap_or(false),
        &idle_for,
        options.cleanup_kind.as_deref(),
        cleanup_limit(options.limit)?,
    )
    .map_err(Error::from_reason)
}

macro_rules! line_text {
    ($(($name:ident, $target:path)),* $(,)?) => {$(
        #[napi]
        pub fn $name(payload: JsonInput) -> Result<String> { $target(&payload).map_err(Error::from_reason) }
    )*};
}

line_text! {
    (format_line_create, output::format_line_create),
    (format_line_switch, output::format_line_switch),
    (format_line_show, output::format_line_show),
    (format_line_archive, output::format_line_archive),
    (format_line_rename, output::format_line_rename),
    (format_line_delete, output::format_line_delete),
    (format_line_merge, output::format_line_merge),
}
