use crate::runtime::RepoRuntime;
use ait_core::external::readiness::build_external_readiness_report;
use ait_core::external::readiness::ExternalReadinessReport;
use ait_core::external::status::inspect_external_filesystem_status_report;

pub(crate) fn external_readiness_report_for_repo(
    repo: &RepoRuntime,
) -> Result<Option<ExternalReadinessReport>, String> {
    let workspace_root = repo.workspace_root();
    let status = inspect_external_filesystem_status_report(&workspace_root, repo.repo_name())
        .map_err(|err| err.to_string())?;
    if !status.manifest_present {
        return Ok(None);
    }
    Ok(Some(build_external_readiness_report(&status.report)))
}

pub(crate) fn external_readiness_blocker_details(readiness: &ExternalReadinessReport) -> String {
    readiness
        .blockers
        .iter()
        .map(|blocker| {
            let name = blocker.name.as_deref().unwrap_or("-");
            let path = blocker.path.as_deref().unwrap_or("-");
            format!("{} {name} {path}: {}", blocker.code, blocker.message)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ait_core::json_support::{json, JsonMap};
    use std::fs;
    use std::path::Path;

    #[test]
    fn external_readiness_report_skips_repos_without_manifest() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = test_repo(temp.path());

        let report = external_readiness_report_for_repo(&repo).unwrap();

        assert!(report.is_none());
    }

    #[test]
    fn external_readiness_report_uses_the_executing_worktree_workspace() {
        let temp = tempfile::TempDir::new().unwrap();
        let canonical_root = temp.path().join("canonical");
        let worktree_root = temp.path().join("worktree");
        fs::create_dir_all(&canonical_root).unwrap();
        fs::create_dir_all(&worktree_root).unwrap();
        write_legacy_external_manifest_without_repository_index(&canonical_root);
        write_external_manifest(&worktree_root, "SNP-DB-DIRECT");
        write_external_lock(&worktree_root, "SNP-DB-DIRECT");
        write_external_marker(&worktree_root, "SNP-DB-DIRECT");
        let repo = test_worktree_repo(&canonical_root, &worktree_root);

        let report = external_readiness_report_for_repo(&repo).unwrap().unwrap();

        assert!(report.ready, "{:?}", report.blockers);
    }

    #[test]
    fn external_readiness_report_blocks_missing_lock_and_materialization() {
        let temp = tempfile::TempDir::new().unwrap();
        write_external_manifest(temp.path(), "SNP-DB-DIRECT");
        let repo = test_repo(temp.path());

        let report = external_readiness_report_for_repo(&repo).unwrap().unwrap();
        let codes = report
            .blockers
            .iter()
            .map(|blocker| blocker.code.as_str())
            .collect::<Vec<_>>();

        assert!(!report.ready);
        assert!(codes.contains(&"external_lock_missing"));
        assert!(codes.contains(&"external_materialization_missing"));
        assert!(external_readiness_blocker_details(&report).contains("ait-external.lock"));
    }

    #[test]
    fn external_readiness_report_blocks_active_local_links_before_ci() {
        let temp = tempfile::TempDir::new().unwrap();
        write_external_manifest(temp.path(), "SNP-DB-DIRECT");
        write_external_lock(temp.path(), "SNP-DB-DIRECT");
        write_external_marker(temp.path(), "SNP-DB-DIRECT");
        fs::write(
            temp.path().join("ait-external.links.toml"),
            r#"
[[link]]
name = "ait-db"
path = "../ait-db"
"#,
        )
        .unwrap();
        let repo = test_repo(temp.path());

        let report = external_readiness_report_for_repo(&repo).unwrap().unwrap();
        let codes = report
            .blockers
            .iter()
            .map(|blocker| blocker.code.as_str())
            .collect::<Vec<_>>();

        assert!(!report.ready);
        assert!(codes.contains(&"external_local_link_active"));
        assert!(external_readiness_blocker_details(&report).contains("ait-db"));
    }

    #[test]
    fn external_readiness_report_rejects_committed_sibling_manifest_paths() {
        let temp = tempfile::TempDir::new().unwrap();
        write_external_manifest_with_sibling_path(temp.path());
        let repo = test_repo(temp.path());

        let err = external_readiness_report_for_repo(&repo).unwrap_err();

        assert!(err.contains("must not escape the repository"));
        assert!(err.contains("../ait-db"));
    }

    fn test_repo(root: &Path) -> RepoRuntime {
        RepoRuntime {
            root: root.to_path_buf(),
            ait_dir: root.join(".ait"),
            config: JsonMap::from_iter([("repo_name".to_string(), json!("ait-core"))]),
            worktree_config_path: None,
        }
    }

    fn test_worktree_repo(canonical_root: &Path, worktree_root: &Path) -> RepoRuntime {
        RepoRuntime {
            root: worktree_root.to_path_buf(),
            ait_dir: canonical_root.join(".ait"),
            config: JsonMap::from_iter([
                ("repo_name".to_string(), json!("ait-core")),
                (
                    "repo_root".to_string(),
                    json!(canonical_root.to_string_lossy().to_string()),
                ),
                (
                    "workspace_root".to_string(),
                    json!(worktree_root.to_string_lossy().to_string()),
                ),
            ]),
            worktree_config_path: Some(worktree_root.join(".ait-worktree.json")),
        }
    }

    fn write_legacy_external_manifest_without_repository_index(root: &Path) {
        fs::write(
            root.join("ait-external.toml"),
            r#"
[[external]]
name = "legacy"
repo_name = "legacy"
remote = "origin"
line = "main"
snapshot = "SNP-LEGACY"
materialize_to = ".ait-external/legacy"
license = "Apache-2.0"
"#,
        )
        .unwrap();
    }

    fn write_external_manifest(root: &Path, snapshot: &str) {
        fs::write(
            root.join("ait-external.toml"),
            format!(
                r#"
[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "{snapshot}"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
version = "0.1.0"

[external.bindings.rust]
kind = "cargo-path"
path = "rust/crates/ait-db"
package = "ait-db"
"#,
            ),
        )
        .unwrap();
    }

    fn write_external_manifest_with_sibling_path(root: &Path) {
        fs::write(
            root.join("ait-external.toml"),
            r#"
[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "SNP-DB-DIRECT"
materialize_to = "../ait-db"
license = "Apache-2.0"
"#,
        )
        .unwrap();
    }

    fn write_external_lock(root: &Path, snapshot: &str) {
        fs::write(
            root.join("ait-external.lock"),
            format!(
                r#"
format = "ait.external.lock"

[[node]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "{snapshot}"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
version = "0.1.0"
parent_path = ""

[[node.binding]]
language = "rust"
kind = "cargo-path"
path = "rust/crates/ait-db"
package = "ait-db"
"#,
            ),
        )
        .unwrap();
    }

    fn write_external_marker(root: &Path, snapshot: &str) {
        let materialized = root.join(".ait-external").join("ait-db");
        fs::create_dir_all(materialized.join("rust/crates/ait-db")).unwrap();
        fs::write(
            materialized.join(".ait-external-marker.json"),
            format!(
                r#"{{
  "format": "ait.external.materialized",
  "version": 3,
  "line": "main",
  "materialize_to": ".ait-external/ait-db",
  "name": "ait-db",
  "parent_path": "",
  "remote": "origin",
  "repo_name": "ait-db",
  "repository_index": 0,
  "snapshot": "{snapshot}",
  "files": []
}}"#,
            ),
        )
        .unwrap();
    }
}
