use super::*;
use crate::plan_http_client::PlanHttpClientManager;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

impl PlanSyncLocalArtifactBodySource for BoundaryBlobStore {
    fn read_plan_revision_artifact_body(&self, _: &str, _: &JsonValue) -> Option<String> {
        None
    }
}

impl PlanSyncZstdPackStore for BoundaryBlobStore {
    fn existing_zstd_object_pack_bundle(
        &self,
        _: &str,
        _: &str,
        _: i64,
    ) -> Result<Option<PlanSyncZstdObjectPackBundle>, String> {
        unreachable!("fixture has no packed artifact")
    }
    fn upsert_zstd_object_pack_metadata(
        &self,
        _: PlanSyncZstdObjectPackMetadata<'_>,
    ) -> Result<(), String> {
        unreachable!("fixture has no packed artifact")
    }
    fn existing_zstd_tree_pack_bundle(
        &self,
        _: &str,
        _: &str,
        _: &[JsonValue],
    ) -> Result<Option<PlanSyncZstdTreePackBundle>, String> {
        unreachable!("fixture has no packed artifact")
    }
    fn upsert_zstd_tree_pack_metadata(
        &self,
        _: PlanSyncZstdTreePackMetadata<'_>,
    ) -> Result<(), String> {
        unreachable!("fixture has no packed artifact")
    }
}

type RecordedRequests = Vec<(String, String, JsonValue)>;

fn transport(responses: Vec<(u16, JsonValue)>) -> (String, thread::JoinHandle<RecordedRequests>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, payload) in responses {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("attribution fixture accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap().to_string();
            let path = parts.next().unwrap().to_string();
            let mut length = 0;
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let body = if bytes.is_empty() {
                JsonValue::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            requests.push((method, path, body));
            let body = payload.to_string();
            write!(stream, "HTTP/1.1 {status} response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    (base_url, handle)
}

struct Case {
    repo: &'static str,
    repository_index: u32,
    plan: &'static str,
    remote_plan: &'static str,
    path: &'static str,
    selector: &'static str,
    first_revision: u32,
}

impl Case {
    fn request(&self) -> SyncRequest {
        let mut request = boundary_request(false);
        request.repo_name = self.repo.into();
        request.remote_repo_name = Some(self.repo.into());
        request.repository_index = Some(RepositoryIndex::new(self.repository_index));
        request.id_namespace_prefix = Some("R".into());
        request.target = self.path.into();
        request
    }

    fn repository(&self) -> JsonValue {
        json!({"repository_index":self.repository_index,"repository_name":self.repo,
            "namespace":"R","policy_flags":17,"tombstoned":false})
    }

    fn registration(&self, actor: u32) -> Vec<(u16, JsonValue)> {
        vec![
            (200, json!({"repository":self.repository()})),
            (
                200,
                json!({"repository":self.repository(),"actor_key_index":actor}),
            ),
        ]
    }

    fn local_revision(&self, id: u32) -> JsonValue {
        json!({
            "plan_id":self.plan, "plan_revision_id":format!("plan-revision:{id}"),
            "revision_number":id - self.first_revision + 1, "title_snapshot":format!("Revision {id}"),
            "artifact_path":self.path,"artifact_selector":self.selector,
            "artifact_heading":"Execution plan", "items":[{
                "plan_item_ref":format!("{}/execute",self.selector), "text":"Implement the adapter",
                "checkbox_state":"unchecked", "heading_path":[], "line_number":3
            }], "publication_state":"local_draft", "actor_key_index":null,
            "created_by":null, "published_plan_revision_id":null
        })
    }

    fn remote_revision(&self, local: &JsonValue, id: u32, actor: u32) -> JsonValue {
        let mut row = local.clone();
        row["plan_id"] = json!(self.remote_plan);
        row["plan_revision_id"] = json!(format!("plan-revision:{id}"));
        row["actor_key_index"] = json!(actor);
        row
    }

    fn remote_plan(&self, revision: &JsonValue) -> JsonValue {
        json!({"plan_id":self.remote_plan,"repo_name":self.repo,"status":"active",
            "head_revision_id":revision["plan_revision_id"],"head_revision":revision})
    }
}

struct ExactPublicationMapping<'a>(&'a BoundaryLocalStore, &'a Case);

impl PlanSyncLocalPlanStore for ExactPublicationMapping<'_> {
    fn get_plan(&self, id: &str) -> Result<JsonValue, String> {
        self.0.get_plan(id)
    }
}

impl PlanSyncLocalRevisionStore for ExactPublicationMapping<'_> {
    fn list_plan_revisions(&self, id: &str) -> Result<Vec<JsonValue>, String> {
        self.0.list_plan_revisions(id)
    }
    fn get_plan_revision_artifact(
        &self,
        id: &str,
    ) -> Result<Option<PlanSyncLocalRevisionArtifact>, String> {
        self.0.get_plan_revision_artifact(id)
    }
}

impl PlanSyncLocalPublicationStore for ExactPublicationMapping<'_> {
    fn mark_plan_published(
        &self,
        id: &str,
        remote: Option<&str>,
        published: &str,
        head: Option<&str>,
        mappings: &[(String, String)],
        at: &str,
    ) -> Result<JsonValue, String> {
        assert_eq!(
            mappings,
            &[
                (
                    format!("plan-revision:{}", self.1.first_revision),
                    "plan-revision:201".into()
                ),
                (
                    format!("plan-revision:{}", self.1.first_revision + 1),
                    "plan-revision:202".into()
                ),
            ]
        );
        self.0
            .mark_plan_published(id, remote, published, head, mappings, at)
    }
}

fn publish(
    case: &Case,
    request: &SyncRequest,
    local: &BoundaryLocalStore,
    client: &mut PlanHttpClientManager,
) -> Result<JsonValue, String> {
    let content = BoundaryBlobStore::default();
    super::super::publish::local_plan_publish(
        request,
        &ExactPublicationMapping(local, case),
        &content,
        &content,
        &FilesystemFileIoStore,
        &BoundaryIdentitySource::default(),
        client,
        case.plan,
        None,
    )
}

fn cross_repository_cases() -> [Case; 2] {
    [
        Case {
            repo: "ait-runner",
            repository_index: 7,
            plan: "PR-60",
            remote_plan: "PR-61",
            path: "docs/sprints/hash-ready-finish-runner-20261020.md",
            selector: "ait-runner/hash-ready-finish-runner-20261020",
            first_revision: 146,
        },
        Case {
            repo: "another-repository",
            repository_index: 27,
            plan: "PR-4",
            remote_plan: "PR-8",
            path: "plans/another.md",
            selector: "another/plan",
            first_revision: 3,
        },
    ]
}

#[test]
fn default_actor_attribution_publishes_seed_replay_and_retries_across_repositories() {
    for case in cross_repository_cases() {
        let first = case.local_revision(case.first_revision);
        let second = case.local_revision(case.first_revision + 1);
        let local = BoundaryLocalStore::default();
        local.existing_plans.borrow_mut().insert(
            case.plan.into(),
            json!({
                "plan_id":case.plan,"repo_name":case.repo,"status":"active",
                "publication_state":"local_draft","head_revision_id":second["plan_revision_id"]
            }),
        );
        local
            .revisions
            .borrow_mut()
            .insert(case.plan.into(), vec![second.clone(), first.clone()]);
        let seed = case.remote_revision(&first, 201, 9);
        let replay = case.remote_revision(&second, 202, 11);
        let absent = (404, json!({"detail":format!("Unknown plan: {}",case.plan)}));

        let mut responses = case.registration(9);
        responses.extend([
            absent.clone(),
            (200, case.remote_plan(&seed)),
            (
                503,
                json!({"detail":"replay unavailable after seed publication"}),
            ),
        ]);
        let (url, server) = transport(responses);
        let mut request = case.request();
        let mut client = build_http_client_manager(&request, &url).unwrap();
        assert!(publish(&case, &request, &local, &mut client)
            .unwrap_err()
            .contains("503"));
        assert!(local.calls.borrow().is_empty());
        let first_requests = server.join().unwrap();
        assert_eq!(
            first_requests[1].2["repository_index"],
            case.repository_index
        );
        for (_, path, body) in &first_requests[2..] {
            assert!(path.starts_with(&format!(
                "/v1/native/repository-authorities/{}/",
                case.repository_index
            )));
            if !body.is_null() {
                assert_eq!(body["actor_key_index"], 9, "{path}");
            }
        }

        // The next operation uses its configured default Actor. Exact seed
        // reuse preserves the first operation's immutable author and identity.
        let mut responses = case.registration(11);
        responses.extend([
            absent,
            (200, case.remote_plan(&seed)),
            (200, case.remote_plan(&replay)),
        ]);
        let (url, server) = transport(responses);
        request.user_name = Some("Bea".into());
        request.user_email = Some("bea@example".into());
        let mut client = build_http_client_manager(&request, &url).unwrap();
        let result = publish(&case, &request, &local, &mut client).unwrap();
        assert_eq!(result["plan_id"], case.plan);
        assert_eq!(result["published_plan_id"], case.remote_plan);
        assert_eq!(result["published_head_revision_id"], "plan-revision:202");
        let retried = server.join().unwrap();
        assert_eq!(retried[1].2["user_name"], "Bea");
        for (_, path, body) in &retried[2..] {
            if !body.is_null() {
                assert_eq!(body["actor_key_index"], 11, "{path}");
            }
        }
        assert_eq!(
            retried[4].2["expected_head_revision_id"],
            "plan-revision:201"
        );
        assert_eq!(seed["actor_key_index"], 9);

        // Recover exact existing history using reads only, with no reassignment
        // of known authors and no Local/Remote ordinal aliasing.
        local
            .existing_plans
            .borrow_mut()
            .get_mut(case.plan)
            .unwrap()["published_plan_id"] = json!(case.remote_plan);
        let mut responses = case.registration(13);
        responses.extend([
            (200, case.remote_plan(&replay)),
            (200, json!([seed, replay])),
        ]);
        let (url, server) = transport(responses);
        let mut client = build_http_client_manager(&request, &url).unwrap();
        let result = publish(&case, &request, &local, &mut client).unwrap();
        assert_eq!(result["publish_action"], "mapped");
        assert_eq!(result["published_revision_count"], 0);
        assert!(server.join().unwrap()[2..]
            .iter()
            .all(|(method, _, _)| method == "GET"));
    }
}

#[test]
fn default_actor_attribution_rejects_missing_and_ambiguous_configured_identity() {
    let case = &cross_repository_cases()[1];
    for field in ["user_name", "user_email"] {
        let mut request = case.request();
        if field == "user_name" {
            request.user_name = None;
        } else {
            request.user_email = None;
        }
        assert!(
            matches!(build_http_client_manager(&request,"http://127.0.0.1:1"),Err(error) if error.contains(field))
        );
    }
    let (url, server) = transport(vec![
        (200, json!({"repository":case.repository()})),
        (
            409,
            json!({"detail":"Ambiguous configured default Actor identity"}),
        ),
    ]);
    assert!(
        matches!(build_http_client_manager(&case.request(),&url),Err(error) if error.contains("Ambiguous configured default Actor identity"))
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|(_, path, _)| !path.contains("/sprints")));
}
