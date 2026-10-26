use super::*;
use ait_core::attest_json::RemoteVerificationState;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

fn statement() -> JsonValue {
    json!({
        "verification_state": "pending", "require_tests_pass": false,
        "require_human_review": true, "require_lint_pass": true, "revoked": true,
    })
}

#[test]
fn compact_statement_preserves_each_verification_state_and_requirement() {
    for (text, expected) in [
        ("unknown", RemoteVerificationState::Unknown),
        ("pending", RemoteVerificationState::Pending),
        ("pass", RemoteVerificationState::Pass),
        ("fail", RemoteVerificationState::Fail),
    ] {
        let mut input = statement();
        input["verification_state"] = json!(text);
        let actual = parse_compact_attestation_statement(input).unwrap();
        assert_eq!(actual.verification_state, expected);
        assert!(!actual.require_tests_pass);
        assert!(actual.require_human_review && actual.require_lint_pass && actual.revoked);
    }
}

#[test]
fn compact_statement_rejects_legacy_missing_and_malformed_fields() {
    for field in [
        "evaluation_summary",
        "provenance_summary",
        "detail",
        "ci_backed",
    ] {
        let mut input = statement();
        input[field] = json!({});
        assert!(parse_compact_attestation_statement(input)
            .unwrap_err()
            .contains(field));
    }
    for input in [JsonValue::Null, json!([]), json!("statement")] {
        assert!(parse_compact_attestation_statement(input).is_err());
    }
    let mut invalid_state = statement();
    invalid_state["verification_state"] = json!("not_required");
    let mut invalid_bool = statement();
    invalid_bool["require_tests_pass"] = json!("false");
    let mut missing = statement();
    missing.as_object_mut().unwrap().remove("revoked");
    for input in [invalid_state, invalid_bool, missing] {
        assert!(parse_compact_attestation_statement(input).is_err());
    }
}

fn compact_http_fixture() -> (String, thread::JoinHandle<Vec<(String, JsonValue)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let candidate = json!({
        "repository_index": 0, "task_id": "RT-0001", "change_id": null,
        "change_ref": null, "patchset_id": "RT-0001/P-01",
        "base_snapshot_id": "SNP-000000000001", "revision_snapshot_id": "SNP-000000000002",
        "author_mode": "ai_with_human_review", "invalidated": false,
    });
    let mut response = statement();
    response
        .as_object_mut()
        .unwrap()
        .extend(candidate.as_object().unwrap().clone());
    response["attestation_id"] = json!("RT-0001/P-01/AT-01");
    response["ci_backed"] = json!(false);
    response["evaluation_summary"] = json!({"tests":"none", "lint":"none"});
    let responses = [
        (200, candidate.clone()),
        (
            200,
            json!({"task_id":"RT-0001", "status":"active", "successful_land":null,
            "current_patchset_id":"RT-0001/P-01"}),
        ),
        (200, candidate),
        (404, json!({"detail":"Unknown attestation"})),
        (200, response),
    ];
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, response) in responses {
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
                    Err(error) => panic!("compact binding fixture: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" {
                    break;
                }
                if let Some((key, value)) = header.split_once(':') {
                    if key.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let body = if bytes.is_empty() {
                JsonValue::Null
            } else {
                ait_core::json_support::JsonCodec::parse_value(
                    std::str::from_utf8(&bytes).unwrap(),
                    "request",
                )
                .unwrap()
            };
            requests.push((request, body));
            let body = response.to_string();
            write!(stream, "HTTP/1.1 {status} response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    (format!("http://{address}"), handle)
}

#[test]
fn compact_attestation_uses_the_actual_pyo3_boundary() {
    use pyo3::exceptions::PyTypeError;
    let (url, server) = compact_http_fixture();
    Python::initialize();
    Python::attach(|py| {
        // A configured native adapter isolates this binding test from the
        // unrelated public client's repository-context setup.
        let adapter = HttpWorkflowCloseoutRemote::new(PlanHttpClientConfig {
            base_url: url,
            repository_index: Some(ait_core::server_operational::RepositoryIndex::new(0)),
            headers: BTreeMap::new(),
            default_timeout_ms: 5000,
            retry_attempts: 0,
            retry_backoff_ms: 0,
            pool_max_idle_per_host: 1,
        })
        .unwrap();
        let remote = Py::new(py, HttpWorkflowCloseoutRemotePy { adapter }).unwrap();
        let remote = remote.bind(py).as_any();
        let input = PyDict::new(py);
        input.set_item("verification_state", "pending").unwrap();
        for (field, value) in [
            ("require_tests_pass", false),
            ("require_human_review", true),
            ("require_lint_pass", true),
            ("revoked", true),
        ] {
            input.set_item(field, value).unwrap();
        }
        // Omitting both optional parameters exercises their public defaults.
        let result = remote
            .call_method1(
                "put_attestation",
                ("RT-0001/P-01", "ai_with_human_review", &input),
            )
            .unwrap();
        let result = parse_json_value(result, "response").unwrap();
        for (key, value) in statement().as_object().unwrap() {
            assert_eq!(&result[key], value);
        }
        assert_eq!(result["attestation_id"], "RT-0001/P-01/AT-01");
        assert_eq!(result["ci_backed"], false);
        assert_eq!(result["evaluation_summary"]["tests"], "none");
        let signature: String = remote
            .getattr("put_attestation")
            .unwrap()
            .getattr("__text_signature__")
            .unwrap()
            .extract()
            .unwrap();
        assert!(signature.contains("repo_name=None") && signature.contains("exact_id=False"));
        for field in [
            "evaluation_summary",
            "provenance_summary",
            "detail",
            "ci_backed",
        ] {
            let invalid = input.copy().unwrap();
            invalid.set_item(field, false).unwrap();
            let error = remote
                .call_method1(
                    "put_attestation",
                    ("RT-0001/P-01", "ai_with_human_review", invalid),
                )
                .unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py));
            assert!(error.to_string().contains(field));
        }
        let invalid = input.copy().unwrap();
        invalid.set_item("require_tests_pass", "false").unwrap();
        assert!(remote
            .call_method1(
                "put_attestation",
                ("RT-0001/P-01", "ai_with_human_review", invalid)
            )
            .unwrap_err()
            .is_instance_of::<PyValueError>(py));
        let legacy = PyDict::new(py);
        legacy.set_item("evaluation_summary", &input).unwrap();
        let error = remote
            .call_method(
                "put_attestation",
                ("RT-0001/P-01", "ai_with_human_review"),
                Some(&legacy),
            )
            .unwrap_err();
        assert!(error.is_instance_of::<PyTypeError>(py));
        let error = remote
            .call_method1(
                "put_attestation",
                (
                    "RT-0001/P-01",
                    "ai_with_human_review",
                    &input,
                    &input,
                    &input,
                    "repo",
                    false,
                ),
            )
            .unwrap_err();
        assert!(error.is_instance_of::<PyTypeError>(py));
    });
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests[4].0.starts_with("PUT "));
    let mut expected = statement();
    expected["author_mode"] = json!("ai_with_human_review");
    assert_eq!(requests[4].1, expected);
}
