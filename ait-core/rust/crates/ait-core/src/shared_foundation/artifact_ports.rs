use crate::json_support::JsonValue;

pub trait ArtifactResolver {
    type Error;

    fn normalize_markdown_artifact_path(&self, path_value: &str) -> String;
    fn is_markdown_artifact_path(&self, path_value: &str) -> bool;
    fn is_lineage_only_markdown_artifact_path(&self, path_value: &str) -> bool;
    fn list_visible_workspace_paths(
        &self,
        repo_root: &str,
        ignore_rules_text: Option<&str>,
        runtime_root: Option<&str>,
    ) -> Result<Vec<String>, Self::Error>;
    fn list_visible_markdown_artifact_paths(
        &self,
        repo_root: &str,
        ignore_rules_text: Option<&str>,
        runtime_root: Option<&str>,
    ) -> Result<Vec<String>, Self::Error>;
    fn read_utf8_text_file(&self, path_value: &str) -> Result<String, Self::Error>;
    fn read_json_file(&self, path_value: &str) -> Result<JsonValue, Self::Error>;
    fn read_binary_file(&self, path_value: &str) -> Result<Vec<u8>, Self::Error>;
    fn resolve_repo_artifact_path(
        &self,
        repo_root: &str,
        path_value: &str,
        allow_missing: bool,
    ) -> Result<JsonValue, Self::Error>;
    fn zip_archive_has_member(
        &self,
        path_value: &str,
        entry_name: &str,
    ) -> Result<bool, Self::Error>;
    fn read_zip_archive_member(
        &self,
        path_value: &str,
        entry_name: &str,
    ) -> Result<Vec<u8>, Self::Error>;
}
