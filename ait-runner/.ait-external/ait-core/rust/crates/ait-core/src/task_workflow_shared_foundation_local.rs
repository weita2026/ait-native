use crate::json_support::JsonValue;

use crate::plan_filesystem::{
    read_binary_file, read_json_file, read_utf8_text_file, resolve_repo_artifact_path,
};

pub fn task_workflow_read_utf8_text_file(path_value: &str) -> Result<String, String> {
    read_utf8_text_file(path_value).map_err(|err| format!("{err:?}"))
}

pub fn task_workflow_read_json_file(path_value: &str) -> Result<JsonValue, String> {
    read_json_file(path_value).map_err(|err| format!("{err:?}"))
}

pub fn task_workflow_read_binary_file(path_value: &str) -> Result<Vec<u8>, String> {
    read_binary_file(path_value).map_err(|err| format!("{err:?}"))
}

pub fn task_workflow_resolve_repo_artifact_path(
    repo_root: &str,
    path_value: &str,
    allow_missing: bool,
) -> Result<JsonValue, String> {
    resolve_repo_artifact_path(repo_root, path_value, allow_missing)
        .map_err(|err| format!("{err:?}"))
}
