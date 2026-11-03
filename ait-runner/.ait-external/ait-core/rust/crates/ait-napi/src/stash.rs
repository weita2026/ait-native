use crate::{repository::repository, JsonInput};
use ait_cli::{binding_services as service, primitives};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct StashSaveOptions {
    pub message: Option<String>,
    pub keep_workspace: Option<bool>,
}

#[napi]
pub fn stash_save(root: String, options: Option<StashSaveOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::save_stash(
        &repository(&root)?,
        options.message.as_deref(),
        options.keep_workspace.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn stash_apply(root: String, stash_id: String, force: Option<bool>) -> Result<JsonValue> {
    service::apply_stash(&repository(&root)?, &stash_id, force.unwrap_or(false))
        .map_err(Error::from_reason)
}

#[napi]
pub fn stash_pop(root: String, stash_id: String, force: Option<bool>) -> Result<JsonValue> {
    service::pop_stash(&repository(&root)?, &stash_id, force.unwrap_or(false))
        .map_err(Error::from_reason)
}

#[napi]
pub fn stash_drop(root: String, stash_id: String) -> Result<JsonValue> {
    service::drop_stash(&repository(&root)?, &stash_id).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_stash_list(root: String) -> Result<JsonValue> {
    primitives::stash_list(&repository(&root)?).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_stash_show(root: String, stash_id: String) -> Result<JsonValue> {
    primitives::stash_show(&repository(&root)?, &stash_id).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_stash_drop(root: String, stash_id: String) -> Result<JsonValue> {
    primitives::stash_drop(&repository(&root)?, &stash_id).map_err(Error::from_reason)
}

macro_rules! stash_text_output {
    ($(($name:ident, $target:path)),* $(,)?) => {$(
        #[napi]
        pub fn $name(payload: JsonInput) -> Result<String> { $target(&payload).map_err(Error::from_reason) }
    )*};
}

stash_text_output! {
    (format_stash_save, ait_cli::binding_output::format_stash_save),
    (format_stash_list, ait_cli::binding_output::format_stash_list),
    (format_stash_show, ait_cli::binding_output::format_stash_show),
    (format_stash_apply, ait_cli::binding_output::format_stash_apply),
    (format_stash_pop, ait_cli::binding_output::format_stash_pop),
    (format_stash_drop, ait_cli::binding_output::format_stash_drop),
}
