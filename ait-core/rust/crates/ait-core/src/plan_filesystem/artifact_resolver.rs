use super::*;

#[derive(Default)]
pub struct PlanArtifactResolverFoundation;

impl ArtifactResolver for PlanArtifactResolverFoundation {
    type Error = PlanFilesystemError;

    fn normalize_markdown_artifact_path(&self, path_value: &str) -> String {
        normalize_markdown_artifact_path(path_value)
    }

    fn is_markdown_artifact_path(&self, path_value: &str) -> bool {
        is_markdown_artifact_path(path_value)
    }

    fn is_lineage_only_markdown_artifact_path(&self, path_value: &str) -> bool {
        is_lineage_only_markdown_artifact_path(path_value)
    }

    fn list_visible_workspace_paths(
        &self,
        repo_root: &str,
        ignore_rules_text: Option<&str>,
        runtime_root: Option<&str>,
    ) -> Result<Vec<String>, Self::Error> {
        list_visible_workspace_paths(repo_root, ignore_rules_text, runtime_root)
    }

    fn list_visible_markdown_artifact_paths(
        &self,
        repo_root: &str,
        ignore_rules_text: Option<&str>,
        runtime_root: Option<&str>,
    ) -> Result<Vec<String>, Self::Error> {
        list_visible_markdown_artifact_paths(repo_root, ignore_rules_text, runtime_root)
    }

    fn read_utf8_text_file(&self, path_value: &str) -> Result<String, Self::Error> {
        read_utf8_text_file(path_value)
    }

    fn read_json_file(&self, path_value: &str) -> Result<JsonValue, Self::Error> {
        read_json_file(path_value)
    }

    fn read_binary_file(&self, path_value: &str) -> Result<Vec<u8>, Self::Error> {
        read_binary_file(path_value)
    }

    fn resolve_repo_artifact_path(
        &self,
        repo_root: &str,
        path_value: &str,
        allow_missing: bool,
    ) -> Result<JsonValue, Self::Error> {
        resolve_repo_artifact_path(repo_root, path_value, allow_missing)
    }

    fn zip_archive_has_member(
        &self,
        path_value: &str,
        entry_name: &str,
    ) -> Result<bool, Self::Error> {
        zip_archive_has_member(path_value, entry_name)
    }

    fn read_zip_archive_member(
        &self,
        path_value: &str,
        entry_name: &str,
    ) -> Result<Vec<u8>, Self::Error> {
        read_zip_archive_member(path_value, entry_name)
    }
}
