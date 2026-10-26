use std::fs;
use std::path::Path;

use crate::external::lockfile::ExternalLockNode;
use crate::external::materializer::ExternalContentSource;
use crate::external::{ExternalError, ExternalResult};

#[derive(Debug, Default, Clone, Copy)]
pub struct FixtureExternalContentSource;

impl ExternalContentSource for FixtureExternalContentSource {
    fn materialize_content(
        &self,
        node: &ExternalLockNode,
        destination: &Path,
    ) -> ExternalResult<()> {
        fs::write(
            destination.join("AIT_EXTERNAL_SNAPSHOT"),
            format!(
                "name={}\nrepo_name={}\nsnapshot={}\n",
                node.name, node.repo_name, node.snapshot
            ),
        )
        .map_err(|err| {
            ExternalError::with_code(
                "external_materializer_write",
                format!("failed to write fixture external content: {err}"),
            )
        })
    }
}
