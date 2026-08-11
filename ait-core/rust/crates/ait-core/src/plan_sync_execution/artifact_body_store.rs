use super::artifact_body_ports::PlanSyncLocalArtifactBodySource;
use super::text_field;
use crate::file_io::{FileIoStore, FilesystemFileIoStore};
use crate::json_support::JsonValue;
use std::path::Path;

pub(super) struct FilesystemPlanSyncLocalArtifactBodySource<S = FilesystemFileIoStore> {
    store: S,
}

impl<S> FilesystemPlanSyncLocalArtifactBodySource<S> {
    pub(super) fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S> PlanSyncLocalArtifactBodySource for FilesystemPlanSyncLocalArtifactBodySource<S>
where
    S: FileIoStore,
{
    fn read_plan_revision_artifact_body(
        &self,
        repo_root: &str,
        revision: &JsonValue,
    ) -> Option<String> {
        let artifact_path = text_field(revision, "artifact_path")?;
        let path = Path::new(repo_root).join(&artifact_path);
        self.store.read_to_string(&path).ok()
    }
}
