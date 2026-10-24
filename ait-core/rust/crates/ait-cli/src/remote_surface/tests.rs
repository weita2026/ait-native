use super::*;
use ait_core::json_support::json;
use ait_core::remote_store::{RemoteAddRecord, RemoteStoreResult};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::thread;
use tiny_http::{Response, Server};

#[derive(Default)]
struct FakeRemoteStore {
    remotes: RefCell<BTreeMap<String, RemoteRecord>>,
}

impl FakeRemoteStore {
    fn insert(&self, record: RemoteRecord) {
        self.remotes
            .borrow_mut()
            .insert(record.name.clone(), record);
    }
}

impl RemoteStore for FakeRemoteStore {
    fn remote_exists(&self, name: &str) -> RemoteStoreResult<bool> {
        Ok(self.remotes.borrow().contains_key(name))
    }

    fn list_remotes(&self) -> RemoteStoreResult<Vec<RemoteRecord>> {
        Ok(self.remotes.borrow().values().cloned().collect())
    }

    fn remote_by_name(&self, name: &str) -> RemoteStoreResult<Option<RemoteRecord>> {
        Ok(self.remotes.borrow().get(name).cloned())
    }

    fn add_remote(&self, request: &RemoteAddRecord) -> RemoteStoreResult<()> {
        let remote_id = (self.remotes.borrow().len() + 1) as i64;
        self.insert(RemoteRecord {
            remote_id,
            name: request.name.clone(),
            url: request.url.clone(),
            repo_name: request.repo_name.clone(),
            is_default_push: i64::from(request.make_default),
            is_default_pull: i64::from(request.make_default),
            created_at: request.created_at.clone(),
        });
        Ok(())
    }
}

fn write_remote_add_fixture(root: &Path, repository_index: Option<u32>) -> RepoRuntime {
    let ait_dir = root.join(".ait");
    fs::create_dir_all(&ait_dir).unwrap();
    let mut config = json!({
        "repo_name": "duplicate-name",
        "user_name": "Alice",
        "user_email": "alice@example.test",
        "default_line": "main",
        "id_namespace_prefix": "R",
        "policy_profile": "prototype",
    });
    if let Some(repository_index) = repository_index {
        config["repository_index"] = json!(repository_index);
    }
    fs::write(
        ait_dir.join("config.json"),
        encode_value_pretty_with_newline_error_string(&config).unwrap(),
    )
    .unwrap();
    fs::write(
        ait_dir.join("policy.yaml"),
        "version: 1\npolicy_id: prototype\ndefaults:\n  require_attestation: true\n  require_tests: true\n",
    )
    .unwrap();
    RepoRuntime::discover_from_path(root).unwrap()
}

type RecordedRepositoryAuthorityRequests = Vec<(String, String, String)>;
type RepositoryAuthorityServer = (
    String,
    thread::JoinHandle<RecordedRepositoryAuthorityRequests>,
);

fn spawn_repository_authority_server(
    mut response: JsonValue,
    request_count: usize,
) -> RepositoryAuthorityServer {
    response["actor_key_index"] = json!(0);
    let server = Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr();
    let handle = thread::spawn(move || {
        let mut observed = Vec::new();
        for _ in 0..request_count {
            let mut request = server.recv().unwrap();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            observed.push((
                request.method().as_str().to_string(),
                request.url().to_string(),
                body,
            ));
            request
                .respond(Response::from_string(response.to_string()).with_status_code(200))
                .unwrap();
        }
        observed
    });
    (format!("http://{address}"), handle)
}

#[test]
fn remote_add_registers_and_persists_an_unconfigured_numeric_authority() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("ait-core");
    let repo = write_remote_add_fixture(&root, None);
    let (url, handle) = spawn_repository_authority_server(
        json!({
            "contract": "ait.server.repository-registration.v1",
            "created": true,
            "repository": {
                "repository_index": 4,
                "repository_name": "ait-core",
                "namespace": "R",
                "policy_flags": 0b1000_0011,
                "tombstoned": false,
            }
        }),
        1,
    );

    let added = remote_add(
        &repo,
        &RemoteAddRequest {
            name: "origin".to_string(),
            url,
            make_default: false,
        },
    )
    .expect("remote add registers and persists the allocated Repository PK");
    let requests = handle.join().unwrap();
    let (method, path, body) = &requests[0];

    assert_eq!(method, "POST");
    assert_eq!(path, "/v1/native/repository-authorities");
    assert_eq!(
        parse_value(body, "registration body").unwrap(),
        json!({
            "repository_name": "ait-core",
            "namespace": "R",
            "policy_flags": 0b1000_0011,
            "user_name":"Alice", "user_email":"alice@example.test",
        })
    );
    assert_eq!(added["name"], json!("origin"));
    assert_eq!(added["repo_name"], json!("ait-core"));
    assert_eq!(added["patch_ci"]["required"], json!(false));
    assert_eq!(
        added["patch_ci"]["delivery_policy_default_requires_tests"],
        json!(true)
    );
    assert!(!root.join("ci").exists());
    assert_eq!(
        read_json_object(&root.join(".ait/config.json")).unwrap()["repository_index"],
        json!(4)
    );
}

#[test]
fn remote_add_with_a_configured_index_resolves_actor_without_changing_repository() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("ait-core");
    let repo = write_remote_add_fixture(&root, Some(7));
    let (url, handle) = spawn_repository_authority_server(
        json!({
            "contract": "ait.server.repository-authority.v1",
            "repository": {
                "repository_index": 7,
                "repository_name": "same-display-name",
                "namespace": "R",
                "policy_flags": 0b1000_0011,
                "tombstoned": false,
            }
        }),
        2,
    );

    let added = remote_add(
        &repo,
        &RemoteAddRequest {
            name: "origin".to_string(),
            url,
            make_default: false,
        },
    )
    .expect("configured Repository PK resolves Actor without changing Repository metadata");
    let requests = handle.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|(method, path, _)| (method.as_str(), path.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("GET", "/v1/native/repository-authorities/7"),
            ("POST", "/v1/native/repository-authorities"),
        ]
    );
    assert!(requests[0].2.is_empty());
    assert_eq!(
        parse_value(&requests[1].2, "existing registration").unwrap(),
        json!({
            "repository_index":7, "repository_name":"same-display-name",
            "namespace":"R", "policy_flags":0b1000_0011,
            "user_name":"Alice", "user_email":"alice@example.test",
        })
    );
    assert_eq!(added["repo_name"], json!("ait-core"));
    assert_eq!(
        read_json_object(&root.join(".ait/config.json")).unwrap()["repository_index"],
        json!(7)
    );
}

#[test]
fn remote_add_rejects_conflicting_or_malformed_authority_before_actor_effects() {
    let valid = json!({
        "repository_index":7, "repository_name":"existing name", "namespace":"R",
        "policy_flags":17, "tombstoned":false,
    });
    let cases = [
        ("namespace mismatch", "namespace", json!("Z")),
        ("missing namespace", "namespace", JsonValue::Null),
        ("null namespace", "namespace", JsonValue::Null),
        ("malformed namespace", "namespace", json!(7)),
        ("wrong authority", "repository_index", json!(8)),
        ("tombstoned authority", "tombstoned", json!(true)),
        ("missing name", "repository_name", JsonValue::Null),
        ("invalid policy", "policy_flags", json!(256)),
    ];
    for (label, field, value) in cases {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ait-core");
        let repo = write_remote_add_fixture(&root, Some(7));
        let before = fs::read(root.join(".ait/config.json")).unwrap();
        let before_policy = fs::read(root.join(".ait/policy.yaml")).unwrap();
        let mut authority = valid.clone();
        if label.starts_with("missing ") {
            authority.as_object_mut().unwrap().remove(field);
        } else {
            authority[field] = value;
        }
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handler_finished = std::sync::Arc::clone(&finished);
        let handler = thread::spawn(move || {
            let mut requests = Vec::new();
            let mut actor_effects = 0;
            // Keep accepting after GET: a premature POST receives a valid zero
            // key so the old candidate reaches its late namespace rejection.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let Some(mut request) = server
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .unwrap()
                else {
                    if handler_finished.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "remote add did not complete"
                    );
                    continue;
                };
                let method = request.method().as_str().to_string();
                let path = request.url().to_string();
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                if method == "POST" {
                    actor_effects += 1;
                }
                requests.push((method, path, body));
                request
                    .respond(
                        Response::from_string(
                            json!({
                                "repository":authority, "actor_key_index":0,
                            })
                            .to_string(),
                        )
                        .with_status_code(200),
                    )
                    .unwrap();
            }
            (requests, actor_effects)
        });
        let error = remote_add(
            &repo,
            &RemoteAddRequest {
                name: "origin".into(),
                url,
                make_default: true,
            },
        )
        .expect_err(label);
        finished.store(true, std::sync::atomic::Ordering::Release);
        assert!(!error.is_empty());
        let (requests, actor_effects) = handler.join().unwrap();
        assert_eq!(
            requests,
            vec![(
                "GET".into(),
                "/v1/native/repository-authorities/7".into(),
                String::new()
            )],
            "{label}"
        );
        assert_eq!(actor_effects, 0, "{label}");
        assert_eq!(
            fs::read(root.join(".ait/config.json")).unwrap(),
            before,
            "{label}"
        );
        assert_eq!(
            fs::read(root.join(".ait/policy.yaml")).unwrap(),
            before_policy,
            "{label}"
        );
        assert!(
            remote_list(&repo).unwrap().as_array().unwrap().is_empty(),
            "{label}"
        );
    }
}

#[test]
fn metadata_registration_preserves_custom_and_malformed_ci_and_required_policy() {
    for bytes in [b"user-defined CI contents\n".as_slice(), b"not JSON\xff"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ait-core");
        let repo = write_remote_add_fixture(&root, None);
        fs::create_dir(root.join("ci")).unwrap();
        let path = root.join("ci/patch_ci.json");
        fs::write(&path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o440)).unwrap();
        }
        let stamp = fs::metadata(&path).unwrap().modified().unwrap();
        let permissions = fs::metadata(&path).unwrap().permissions();
        let policy = fs::read(root.join(".ait/policy.yaml")).unwrap();
        let (url, handle) = spawn_repository_authority_server(
            json!({
                "repository": {"repository_index": 4,"repository_name": "ait-core", "namespace": "R", "policy_flags": 0b1000_0011,"tombstoned": false}
            }),
            1,
        );
        let added = remote_add(
            &repo,
            &RemoteAddRequest {
                name: "origin".into(),
                url,
                make_default: false,
            },
        )
        .unwrap();
        let requests = handle.join().unwrap();
        assert_eq!(
            parse_value(&requests[0].2, "request").unwrap()["policy_flags"],
            json!(0b1000_0011)
        );
        assert_eq!(added["patch_ci"]["status"], json!("deferred_to_workflow"));
        assert_eq!(
            added["patch_ci"]["delivery_policy_default_requires_tests"],
            json!(true)
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), stamp);
        assert_eq!(fs::metadata(&path).unwrap().permissions(), permissions);
        assert_eq!(fs::read(root.join(".ait/policy.yaml")).unwrap(), policy);
    }
}

#[test]
#[cfg(unix)]
fn remote_add_from_a_task_worktree_uses_the_authoritative_root_directory_name() {
    let temp = tempfile::tempdir().unwrap();
    let authority_root = temp.path().join("ait-core");
    write_remote_add_fixture(&authority_root, Some(7));
    let worktree_root = temp.path().join("lct-0650");
    fs::create_dir(&worktree_root).unwrap();
    std::os::unix::fs::symlink(authority_root.join(".ait"), worktree_root.join(".ait")).unwrap();
    fs::write(
        worktree_root.join(".ait-worktree.json"),
        encode_value_pretty_with_newline_error_string(&json!({
            "current_line": "feature/lct-0650",
            "repo_root": authority_root.to_string_lossy().to_string(),
            "workspace_root": worktree_root.to_string_lossy().to_string(),
            "worktree_name": "lct-0650"
        }))
        .unwrap(),
    )
    .unwrap();
    let repo = RepoRuntime::discover_from_path(&worktree_root).unwrap();
    let (url, handle) = spawn_repository_authority_server(
        json!({
            "contract": "ait.server.repository-authority.v1",
            "repository": {
                "repository_index": 7,
                "repository_name": "legacy-display-name",
                "namespace": "R",
                "policy_flags": 0b1000_0011,
                "tombstoned": false,
            }
        }),
        2,
    );

    let added = remote_add(
        &repo,
        &RemoteAddRequest {
            name: "origin".to_string(),
            url,
            make_default: false,
        },
    )
    .expect("task-worktree registration uses canonical Repository root identity");
    handle.join().unwrap();

    assert_eq!(repo.workspace_root(), worktree_root);
    assert_eq!(repo.authoritative_repo_root(), authority_root);
    assert_eq!(added["repo_name"], json!("ait-core"));
    assert_ne!(added["repo_name"], json!("lct-0650"));
}

#[test]
fn remote_add_payload_rejects_retired_and_unknown_fields_before_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("ait-core");
    let repo = write_remote_add_fixture(&root, Some(7));

    for retired in [
        json!({
            "name": "mirror",
            "url": "http://127.0.0.1:1",
            "repo_name": "invented-name"
        }),
        json!({
            "name": "mirror",
            "url": "http://127.0.0.1:1",
            "discard_export": true
        }),
    ] {
        let error = remote_add_from_payload(&repo, &retired).unwrap_err();
        assert!(error.contains("retired or unknown field"), "{error}");
    }

    assert!(remote_list(&repo).unwrap().as_array().unwrap().is_empty());
}

#[test]
fn remote_read_helpers_accept_remote_store_trait() {
    let store = FakeRemoteStore::default();
    store.insert(RemoteRecord {
        remote_id: 1,
        name: "origin".to_string(),
        url: "https://example.test/ait".to_string(),
        repo_name: Some("ait-core".to_string()),
        is_default_push: 1,
        is_default_pull: 1,
        created_at: "2026-07-04T03:00:00Z".to_string(),
    });

    let listed = remote_list_with_remote_store(&store).expect("list remotes through store");
    assert_eq!(
        listed,
        json!([{
            "remote_id": 1,
            "name": "origin",
            "url": "https://example.test/ait",
            "repo_name": "ait-core",
            "is_default_push": 1,
            "is_default_pull": 1,
            "created_at": "2026-07-04T03:00:00Z"
        }])
    );

    let shown = remote_get_with_remote_store(&store, "origin").expect("get remote through store");
    assert_eq!(shown["name"], json!("origin"));
    assert_eq!(shown["repo_name"], json!("ait-core"));
    assert_eq!(
        remote_get_with_remote_store(&store, "missing").unwrap_err(),
        "Unknown remote: missing"
    );
}

#[test]
fn remote_write_helpers_accept_remote_store_trait() {
    let store = FakeRemoteStore::default();
    ensure_remote_name_available_with_remote_store(&store, "origin")
        .expect("new remote name is available");

    remote_add_record_with_remote_store(
        &store,
        &RemoteAddRecord {
            name: "origin".to_string(),
            url: "https://example.test/ait".to_string(),
            repo_name: Some("ait-core".to_string()),
            make_default: true,
            created_at: "2026-07-04T04:00:00Z".to_string(),
        },
    )
    .expect("add remote through store");

    let stored = store
        .remote_by_name("origin")
        .expect("read fake store")
        .expect("origin was inserted");
    assert_eq!(stored.url, "https://example.test/ait");
    assert_eq!(stored.repo_name.as_deref(), Some("ait-core"));
    assert_eq!(stored.is_default_push, 1);
    assert_eq!(
        ensure_remote_name_available_with_remote_store(&store, "origin").unwrap_err(),
        "Remote origin already exists."
    );
}

#[test]
fn remote_surface_uses_runtime_config_store_for_add_and_list() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let ait_dir = root.join(".ait");
    fs::create_dir_all(&ait_dir).unwrap();
    fs::write(ait_dir.join("config.json"), "{}\n").unwrap();

    let repo = RepoRuntime {
        root: root.to_path_buf(),
        ait_dir: ait_dir.clone(),
        config: JsonMap::new(),
        worktree_config_path: None,
    };
    let store = repo
        .remote_store()
        .expect("remote store from runtime factory");

    remote_add_record_with_remote_store(
        &store,
        &RemoteAddRecord {
            name: "origin".to_string(),
            url: "https://example.test/ait".to_string(),
            repo_name: Some("ait-core".to_string()),
            make_default: true,
            created_at: "2026-07-06T07:00:00Z".to_string(),
        },
    )
    .expect("add remote through factory-backed store");

    let listed = remote_list(&repo).expect("list remotes through factory-backed store");
    assert_eq!(
        listed,
        json!([{
            "remote_id": 1,
            "name": "origin",
            "url": "https://example.test/ait",
            "repo_name": "ait-core",
            "is_default_push": 1,
            "is_default_pull": 1,
            "created_at": "2026-07-06T07:00:00Z"
        }])
    );
}

#[test]
fn repository_registration_persists_numeric_index_without_rebinding() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let ait_dir = root.join(".ait");
    fs::create_dir_all(&ait_dir).unwrap();
    fs::write(
        ait_dir.join("config.json"),
        "{\n  \"repo_name\": \"duplicate-name\",\n  \"id_namespace_prefix\": \"R\"\n}\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            ait_dir.join("config.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let repo = RepoRuntime {
        root: root.to_path_buf(),
        ait_dir,
        config: JsonMap::new(),
        worktree_config_path: None,
    };

    persist_repository_index(&repo, 4).expect("persist allocated numeric Repository PK");
    let persisted = read_json_object(&root.join(".ait/config.json")).unwrap();
    assert_eq!(persisted["repository_index"], json!(4));
    assert_eq!(persisted["repo_name"], json!("duplicate-name"));
    assert_eq!(persisted["id_namespace_prefix"], json!("R"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.join(".ait/config.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    persist_repository_index(&repo, 4).expect("same numeric Repository PK is idempotent");
    let error = persist_repository_index(&repo, 5)
        .expect_err("an existing numeric Repository PK must never be rebound");
    assert!(error.contains("Refusing to replace"), "{error}");
    assert_eq!(
        read_json_object(&root.join(".ait/config.json")).unwrap()["repository_index"],
        json!(4)
    );
    assert!(fs::read_dir(root.join(".ait")).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}
