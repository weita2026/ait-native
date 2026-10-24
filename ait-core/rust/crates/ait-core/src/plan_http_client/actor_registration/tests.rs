use super::*;
use crate::json_support::{json, JsonCodec};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

fn repository() -> JsonValue {
    json!({"repository_index":0, "repository_name":"exact Existing Name",
        "namespace":"R", "policy_flags":17, "tombstoned":false})
}

type RecordedRequests = Vec<(String, String, JsonValue)>;

fn fixture(
    responses: Vec<(u16, JsonValue)>,
) -> (PlanHttpClientManager, thread::JoinHandle<RecordedRequests>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
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
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("fixture accept: {error}"),
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
                JsonCodec::parse_value(std::str::from_utf8(&bytes).unwrap(), "request").unwrap()
            };
            requests.push((method, path, body));
            let body = payload.to_string();
            write!(stream, "HTTP/1.1 {status} response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        requests
    });
    let client = PlanHttpClientManager::new(PlanHttpClientConfig {
        base_url: url,
        repository_index: Some(RepositoryIndex::new(0)),
        default_timeout_ms: 5000,
        ..PlanHttpClientConfig::default()
    })
    .unwrap();
    (client, handle)
}

#[test]
fn effective_config_identity_trims_without_case_or_byte_folding() {
    let identity = RemoteActorIdentity::new(" Alice 名 ", " Alice@EXAMPLE.test ").unwrap();
    assert_eq!(identity.user_name(), "Alice 名");
    assert_eq!(identity.user_email(), "Alice@EXAMPLE.test");
    assert!(RemoteActorIdentity::new(&"名".repeat(85), "e").is_ok());
    for (name, email) in [
        (" ", "e".to_string()),
        ("Alice", "\n".to_string()),
        ("Alice", "x".repeat(256)),
    ] {
        assert!(RemoteActorIdentity::new(name, &email).is_err());
    }
    assert!(RemoteActorIdentity::new(&"名".repeat(86), "e").is_err());
}

#[test]
fn existing_repository_resolves_zero_actor_and_all_plan_mutations_carry_it() {
    let mut responses = vec![
        (200, json!({"repository":repository()})),
        (200, json!({"repository":repository(), "actor_key_index":0})),
    ];
    responses.extend((0..4).map(|_| (200, json!({"plan_id":"PR-0"}))));
    responses.push((
        200,
        json!({"plan_id":"PR-0", "actor_key_index":null,
        "head_revision":{"actor_key_index":0}}),
    ));
    let (mut client, server) = fixture(responses);
    let identity = RemoteActorIdentity::new(" Alice ", " A@Example ").unwrap();
    client
        .ensure_repository_with_actor("ignored local name", "main", None, Some("R"), &identity)
        .unwrap();
    assert_eq!(client.require_actor_key_index().unwrap(), 0);
    client
        .create_plan(
            "repo",
            "title",
            "docs/a.md",
            None,
            "title",
            &[],
            None,
            "draft",
            None,
            "markdown",
            Some("# title\n"),
        )
        .unwrap();
    client.update_plan_status("PR-0", "archived").unwrap();
    client
        .revise_plan(
            "PR-0",
            "docs/a.md",
            None,
            "title",
            &[],
            None,
            None,
            "markdown",
            Some("# title\n"),
            Some("plan-revision:0"),
        )
        .unwrap();
    client
        .put_plan_revision_artifacts("PR-0", "plan-revision:1", &[])
        .unwrap();
    let read = client.get_plan("PR-0").unwrap();
    assert!(read["actor_key_index"].is_null());
    assert_eq!(read["head_revision"]["actor_key_index"], 0);
    let requests = server.join().unwrap();
    assert_eq!(
        (&*requests[0].0, &*requests[0].1),
        ("GET", "/v1/native/repository-authorities/0")
    );
    assert_eq!(
        requests[1].2,
        json!({"repository_index":0,
        "repository_name":"exact Existing Name", "namespace":"R", "policy_flags":17,
        "user_name":"Alice", "user_email":"A@Example"})
    );
    for request in &requests[2..6] {
        assert_eq!(request.2["actor_key_index"], 0);
    }
    assert_eq!(
        requests[4].2["expected_head_revision_id"],
        "plan-revision:0"
    );
    assert_eq!(requests[2].2["artifact_body"], "# title\n");
}

#[test]
fn unsupported_peer_or_invalid_key_cannot_mutate_plan() {
    for key in [
        None,
        Some(json!(null)),
        Some(json!(-1)),
        Some(json!(1.5)),
        Some(json!(true)),
        Some(json!("0")),
        Some(json!(u32::MAX)),
        Some(json!(u64::MAX)),
    ] {
        let mut response = json!({"repository":repository()});
        if let Some(key) = key {
            response["actor_key_index"] = key;
        }
        let (mut client, server) = fixture(vec![
            (200, json!({"repository":repository()})),
            (200, response),
        ]);
        let identity = RemoteActorIdentity::new("Alice", "a@example").unwrap();
        assert!(client
            .ensure_repository_with_actor("repo", "main", None, None, &identity)
            .is_err());
        assert!(client.update_plan_status("PR-0", "archived").is_err());
        assert_eq!(server.join().unwrap().len(), 2);
    }
}

#[test]
fn missing_existing_repository_never_falls_back_to_registration() {
    let (mut client, server) = fixture(vec![(404, json!({"detail":"missing repository"}))]);
    let identity = RemoteActorIdentity::new("Alice", "a@example").unwrap();
    assert!(client
        .ensure_repository_with_actor("repo", "main", None, None, &identity)
        .is_err());
    assert!(client.require_actor_key_index().is_err());
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn failed_identity_reresolution_discards_previous_request_key() {
    let (mut client, server) = fixture(vec![
        (200, json!({"repository":repository()})),
        (200, json!({"repository":repository(), "actor_key_index":0})),
        (200, json!({"repository":repository()})),
        (409, json!({"detail":"authority changed"})),
    ]);
    let alice = RemoteActorIdentity::new("Alice", "a@example").unwrap();
    let bob = RemoteActorIdentity::new("Bob", "b@example").unwrap();
    client
        .ensure_repository_with_actor("repo", "main", None, None, &alice)
        .unwrap();
    assert!(client
        .ensure_repository_with_actor("repo", "main", None, None, &bob)
        .is_err());
    assert!(client.update_plan_status("PR-0", "archived").is_err());
    let requests = server.join().unwrap();
    assert_eq!(requests[1].2["user_name"], "Alice");
    assert_eq!(requests[3].2["user_name"], "Bob");
    assert_eq!(requests.len(), 4);
}

#[test]
fn composite_boundary_uses_registered_actor_and_preserves_nested_identity() {
    // Equal Local indexes in different callers are deliberately unrelated to
    // the Repository Actor key returned for each effective config identity.
    for (name, actor) in [("Alice", 0), ("Bob", 9)] {
        let (mut client, server) = fixture(vec![
            (200, json!({"repository":repository()})),
            (
                200,
                json!({"repository":repository(), "actor_key_index":actor}),
            ),
            (200, json!({"task_id":"RT-1", "plan":{"actor_key_index":0}})),
        ]);
        let identity = RemoteActorIdentity::new(name, "user@example").unwrap();
        client
            .ensure_repository_with_actor("repo", "main", None, None, &identity)
            .unwrap();
        let payload = json!({"contract":"task-start-atomic/v2", "actor_key_index":88,
            "idempotency_key":"same-domain-key", "plan_item_ref":"card/item",
            "plan":{"action":"existing", "plan_id":"PR-0", "plan_revision_id":"plan-revision:0",
                "expected_head_revision_id":"plan-revision:0"},
            "task":{"title":"Task", "intent":"Intent"}, "change":null});
        let response = client.start_plan_bound_task("repo", &payload).unwrap();
        assert_eq!(response["plan"]["actor_key_index"], 0);
        let requests = server.join().unwrap();
        assert_eq!(
            requests[2].1,
            "/v1/native/repository-authorities/0/task-start"
        );
        let mut expected = payload;
        expected["actor_key_index"] = json!(actor);
        assert_eq!(requests[2].2, expected);
        assert!(requests[2].2["plan"].get("actor_key_index").is_none());
    }
    let (mut client, server) = fixture(vec![]);
    assert!(client
        .start_plan_bound_task("repo", &json!({"contract":"task-start-atomic/v2"}))
        .is_err());
    assert!(server.join().unwrap().is_empty());
}

#[test]
fn lazy_workflow_registration_precedes_mutation_and_keeps_exact_request_fields() {
    let mut responses = vec![
        (200, json!({"repository":repository()})),
        (200, json!({"repository":repository(),"actor_key_index":0})),
    ];
    responses.extend((0..7).map(|_| (200, json!({"actor_key_index":0}))));
    let (mut client, server) = fixture(responses);
    client.set_actor_registration(
        "repo",
        Ok(RemoteActorIdentity::new("Alice", "a@example").unwrap()),
        Some("R"),
    );
    client
        .create_task("repo", "Task", "Intent", None, None, None, None)
        .unwrap();
    client
        .create_change("repo", "RT-1", "Change", "main", None, None, None)
        .unwrap();
    client.close_change("RT-1/C-01", "canceled").unwrap();
    client.close_task("RT-1", "canceled").unwrap();
    client
        .prepare_history_promotion(
            "repo",
            &json!({"contract":"history-promotion-prepare/v3","idempotency_key":"history-key"}),
        )
        .unwrap();
    let expected = crate::workflow_expected_identity::WorkflowExpectedIdentity {
        repository_index: 0,
        patchset_id: "RT-1/P-01".into(),
        base_snapshot_id: "SNP-000000000001".into(),
        revision_snapshot_id: "SNP-000000000002".into(),
    };
    client
        .submit_task_land_guarded(
            "RT-1",
            Some("main"),
            "direct",
            "exact-retry",
            None,
            Some(&expected),
        )
        .unwrap();
    client
        .submit_land("RT-1/C-01", Some("RT-1/P-01"), "main", "direct", None)
        .unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 9);
    assert_eq!(
        (&*requests[0].0, &*requests[0].1),
        ("GET", "/v1/native/repository-authorities/0")
    );
    assert_eq!(
        (&*requests[1].0, &*requests[1].1),
        ("POST", "/v1/native/repository-authorities")
    );
    for (_, _, body) in &requests[2..] {
        assert_eq!(body["actor_key_index"], 0);
    }
    assert_eq!(requests[6].2["idempotency_key"], "history-key");
    assert_eq!(requests[7].2["idempotency_key"], "exact-retry");
    assert_eq!(
        requests[7].2["expected_identity"]["revision_snapshot_id"],
        "SNP-000000000002"
    );
}

#[test]
fn missing_identity_preserves_reads_and_select_probes_but_rejects_writes() {
    let (mut client, server) = fixture(vec![
        (200, json!({"task_id":"RT-1","actor_key_index":null})),
        (200, json!({"patchset_id":"RT-1/P-01"})),
        (200, json!({})),
    ]);
    client.set_actor_registration(
        "repo",
        Err("effective AIT config user_name is missing".into()),
        Some("R"),
    );
    assert!(client.get_task("RT-1", None).unwrap()["actor_key_index"].is_null());
    client.select_patchset("RT-1/C-01", "RT-1/P-01").unwrap();
    client
        .get_remote_snapshots_existence("repo", &["SNP-000000000001".into()])
        .unwrap();
    assert!(client
        .close_task("RT-1", "canceled")
        .unwrap_err()
        .to_string()
        .contains("user_name"));
    assert!(client
        .put_remote_zstd_object_pack("repo", "PCK-test", &[])
        .is_err());
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests
        .iter()
        .all(|(_, _, body)| body.get("actor_key_index").is_none()));
}
