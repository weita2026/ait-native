use crate::json_support::{json, JsonValue};

use crate::external::lockfile::{
    ExternalLockCodec, ExternalLockNode, ExternalLockfile, TomlExternalLockCodec,
};
use crate::external::ExternalResult;

pub const EXTERNAL_RELEASE_LOCKFILE_PATH: &str = "ait-external.lock";

pub fn external_release_closure_metadata_from_lockfile_bytes(
    bytes: &[u8],
) -> ExternalResult<JsonValue> {
    let lockfile = TomlExternalLockCodec.parse_lockfile(bytes)?;
    Ok(external_release_closure_metadata(&lockfile))
}

pub fn external_release_closure_metadata(lockfile: &ExternalLockfile) -> JsonValue {
    let nodes = lockfile.sorted_nodes();
    let canonical_snapshots = nodes
        .iter()
        .map(|node| {
            json!({
                "identity": external_lock_node_identity(node),
                "name": node.name,
                "parent_path": node.parent_path,
                "repo_name": node.repo_name,
                "repository_index": node.repository_index,
                "remote": node.remote,
                "line": node.line,
                "snapshot": node.snapshot,
                "materialize_to": node.materialize_to,
            })
        })
        .collect::<Vec<_>>();
    let version_labels = nodes
        .iter()
        .filter_map(|node| {
            node.version.as_ref().map(|version| {
                json!({
                    "identity": external_lock_node_identity(node),
                    "name": node.name,
                    "parent_path": node.parent_path,
                    "repo_name": node.repo_name,
                    "repository_index": node.repository_index,
                    "version": version,
                    "snapshot": node.snapshot,
                })
            })
        })
        .collect::<Vec<_>>();
    let root_count = nodes
        .iter()
        .filter(|node| node.parent_path.is_empty())
        .count();
    json!({
        "source": EXTERNAL_RELEASE_LOCKFILE_PATH,
        "format": lockfile.format,
        "closure": lockfile.to_json_value(),
        "canonical_snapshots": canonical_snapshots,
        "version_labels": version_labels,
        "summary": {
            "node_count": nodes.len(),
            "root_count": root_count,
            "version_label_count": version_labels.len(),
        },
    })
}

fn external_lock_node_identity(node: &ExternalLockNode) -> String {
    if node.parent_path.is_empty() {
        node.name.clone()
    } else {
        format!("{}:{}", node.parent_path, node.name)
    }
}
