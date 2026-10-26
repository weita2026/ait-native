use crate::{repository::repository, JsonInput};
use ait_cli::tag_surface as service;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn tag_create(
    root: String,
    name: String,
    message: String,
    snapshot_id: Option<String>,
) -> Result<JsonValue> {
    service::tag_create(
        &repository(&root)?,
        service::TagCreateRequest {
            name,
            message,
            snapshot_id,
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn tag_list(root: String) -> Result<JsonValue> {
    service::tag_list(&repository(&root)?).map_err(Error::from_reason)
}

#[napi]
pub fn tag_show(root: String, name: String) -> Result<JsonValue> {
    service::tag_show(&repository(&root)?, &name).map_err(Error::from_reason)
}

#[napi]
pub fn tag_delete(root: String, name: String) -> Result<JsonValue> {
    service::tag_delete(&repository(&root)?, &name).map_err(Error::from_reason)
}

#[napi]
pub fn resolve_snapshot_ref(root: String, value: String) -> Result<String> {
    service::resolve_snapshot_ref(&repository(&root)?, &value).map_err(Error::from_reason)
}

macro_rules! tag_text_output {
    ($(($name:ident, $target:path)),* $(,)?) => {$(
        #[napi]
        pub fn $name(payload: JsonInput) -> Result<String> { $target(&payload).map_err(Error::from_reason) }
    )*};
}

tag_text_output! {
    (format_tag_create, ait_cli::binding_output::format_tag_create),
    (format_tag_list, ait_cli::binding_output::format_tag_list),
    (format_tag_show, ait_cli::binding_output::format_tag_show),
    (format_tag_delete, ait_cli::binding_output::format_tag_delete),
}
