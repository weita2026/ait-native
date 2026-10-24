use super::{
    ensure_remote_repository_authority, local_policy_requires_tests,
    read_remote_repository_authority,
};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::task_workflow_http_adapter::{
    TaskWorkflowHttpClientError, TaskWorkflowHttpClientResult, TaskWorkflowRepositoryEnsurer,
    TaskWorkflowRepositoryReader,
};
use std::fs;
use tempfile::TempDir;

#[derive(Debug, Default)]
struct FakeRepositoryRemote {
    repository: Option<JsonValue>,
    read_calls: usize,
}

#[derive(Debug, Default)]
struct FakeRepositoryEnsurer {
    repository: Option<JsonValue>,
    ensure_calls: Vec<(String, String, Option<JsonValue>, Option<String>)>,
}

fn test_repo(temp: &TempDir) -> RepoRuntime {
    let mut config = JsonMap::new();
    config.insert("default_line".to_string(), json!("trunk"));
    config.insert("id_namespace_prefix".to_string(), json!("NS"));
    config.insert("repository_index".to_string(), json!(7));
    RepoRuntime {
        root: temp.path().to_path_buf(),
        ait_dir: temp.path().join(".ait"),
        config,
        worktree_config_path: None,
    }
}

impl TaskWorkflowRepositoryReader for FakeRepositoryRemote {
    fn get_repository(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.read_calls += 1;
        self.repository
            .clone()
            .ok_or_else(|| TaskWorkflowHttpClientError::Remote("failed: 404".to_string()))
    }
}

impl TaskWorkflowRepositoryEnsurer for FakeRepositoryEnsurer {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.ensure_calls.push((
            repo_name.to_string(),
            default_line.to_string(),
            policy.cloned(),
            id_namespace_prefix.map(str::to_string),
        ));
        self.repository
            .clone()
            .ok_or_else(|| TaskWorkflowHttpClientError::Remote("failed: 500".to_string()))
    }
}

#[test]
fn ensure_remote_repository_authority_registers_an_unconfigured_numeric_authority() {
    let temp = TempDir::new().unwrap();
    let mut repo = test_repo(&temp);
    repo.config.remove("repository_index");
    repo.config
        .insert("id_namespace_prefix".to_string(), json!("R"));
    let mut remote = FakeRepositoryEnsurer {
        repository: Some(json!({
            "contract": "ait.server.repository-registration.v1",
            "created": true,
            "repository": {
                "repository_index": 9,
                "repository_name": "duplicate-display-name",
                "namespace": "R",
                "policy_flags": 0b1000_0011,
                "tombstoned": false,
            }
        })),
        ensure_calls: Vec::new(),
    };

    let repository = ensure_remote_repository_authority(&repo, &mut remote, "local-display-name")
        .expect("unconfigured Repository is registered by numeric PK");

    assert_eq!(repository["repository"]["repository_index"], 9);
    assert_eq!(
        remote.ensure_calls,
        vec![(
            "local-display-name".to_string(),
            "main".to_string(),
            None,
            Some("R".to_string()),
        )]
    );
}

#[test]
fn ensure_remote_repository_authority_rejects_a_mismatched_returned_namespace() {
    let temp = TempDir::new().unwrap();
    let mut repo = test_repo(&temp);
    repo.config.remove("repository_index");
    repo.config
        .insert("id_namespace_prefix".to_string(), json!("N"));
    let mut remote = FakeRepositoryEnsurer {
        repository: Some(json!({
            "repository": {
                "repository_index": 9,
                "repository_name": "display-name",
                "namespace": "X",
                "tombstoned": false,
            }
        })),
        ensure_calls: Vec::new(),
    };

    let error = ensure_remote_repository_authority(&repo, &mut remote, "display-name")
        .expect_err("returned namespace must match the exact local namespace");
    assert!(error.contains("namespace mismatch"), "{error}");
    assert_eq!(remote.ensure_calls.len(), 1);
}

#[test]
fn read_remote_repository_authority_accepts_the_fixed_numeric_registry() {
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    let mut remote = FakeRepositoryRemote {
        repository: Some(json!({
            "contract": "ait.server.repository-authority.v1",
            "repository": {
                "repository_index": 7,
                "repository_name": "repo-main",
                "namespace": "NS",
                "policy_flags": 0,
                "tombstoned": false,
            },
            "ci_capabilities": {
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true,
                }
            }
        })),
        read_calls: 0,
    };

    let repository = {
        let remote_port: &mut dyn TaskWorkflowRepositoryReader = &mut remote;
        read_remote_repository_authority(&repo, remote_port, "repo-main").unwrap()
    };

    assert_eq!(remote.read_calls, 1);
    assert_eq!(repository["repository"]["repository_index"], 7);
    assert_eq!(repository["repository"]["repository_name"], "repo-main");
    assert_eq!(repository["repository"]["namespace"], "NS");
    assert_eq!(
        repository["ci_capabilities"]["remote_sync_capabilities"]["zstd_pack_bulk"],
        true
    );
}

#[test]
fn read_remote_repository_authority_treats_name_as_non_identity_display_data() {
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    let mut remote = FakeRepositoryRemote {
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "duplicate-display-name",
                "namespace": "NS",
                "tombstoned": false,
            }
        })),
        read_calls: 0,
    };

    read_remote_repository_authority(&repo, &mut remote, "local-display-name")
        .expect("numeric Repository PK must be the sole authority identity");
    assert_eq!(remote.read_calls, 1);
}

#[test]
fn read_remote_repository_authority_accepts_the_exact_empty_namespace() {
    let temp = TempDir::new().unwrap();
    let mut repo = test_repo(&temp);
    repo.config
        .insert("id_namespace_prefix".to_string(), json!(""));
    let mut remote = FakeRepositoryRemote {
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "display-name",
                "namespace": "",
                "tombstoned": false,
            }
        })),
        read_calls: 0,
    };

    read_remote_repository_authority(&repo, &mut remote, "display-name")
        .expect("the empty namespace is valid when both authorities select it");
    assert_eq!(remote.read_calls, 1);
}

#[test]
fn read_remote_repository_authority_rejects_invalid_or_mismatched_namespace() {
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);

    for (namespace, expected_error) in [
        (None, "missing string namespace"),
        (Some(json!(7)), "missing string namespace"),
        (Some(json!("")), "namespace mismatch"),
        (Some(json!("N")), "namespace mismatch"),
    ] {
        let mut repository = json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "display-name",
                "tombstoned": false,
            }
        });
        if let Some(namespace) = namespace {
            repository["repository"]["namespace"] = namespace;
        }
        let mut remote = FakeRepositoryRemote {
            repository: Some(repository),
            read_calls: 0,
        };

        let error = read_remote_repository_authority(&repo, &mut remote, "display-name")
            .expect_err("invalid or mismatched namespace must fail before workflow mutation");
        assert!(error.contains(expected_error), "{error}");
        assert_eq!(remote.read_calls, 1);
    }
}

#[test]
fn local_policy_test_requirement_fails_safe_when_policy_is_missing() {
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);

    assert!(local_policy_requires_tests(&repo).unwrap());

    fs::create_dir_all(temp.path().join(".ait")).unwrap();
    fs::write(
        temp.path().join(".ait/policy.yaml"),
        "version: 1\npolicy_id: prototype\ndefaults:\n  require_tests: false\n",
    )
    .unwrap();
    assert!(!local_policy_requires_tests(&repo).unwrap());
}

#[test]
fn local_registration_policy_rejects_unknown_and_malformed_fields() {
    let temp = TempDir::new().unwrap();
    let repo = test_repo(&temp);
    fs::create_dir_all(temp.path().join(".ait")).unwrap();

    fs::write(
        temp.path().join(".ait/policy.yaml"),
        "version: 1\npolicy_id: team\ndefaults:\n  require_tests: true\n",
    )
    .unwrap();
    assert!(local_policy_requires_tests(&repo)
        .unwrap_err()
        .contains("policy_id must be exact prototype"));

    fs::write(
        temp.path().join(".ait/policy.yaml"),
        "version: 1\npolicy_id: prototype\nunknown: true\ndefaults:\n  require_tests: true\n",
    )
    .unwrap();
    assert!(local_policy_requires_tests(&repo)
        .unwrap_err()
        .contains("unknown root field unknown"));

    fs::write(
        temp.path().join(".ait/policy.yaml"),
        "version: 1\npolicy_id: prototype\ndefaults:\n  require_tests: yes\n",
    )
    .unwrap();
    assert!(local_policy_requires_tests(&repo)
        .unwrap_err()
        .contains("must be exact boolean"));
}
