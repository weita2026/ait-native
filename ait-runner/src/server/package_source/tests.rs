use super::*;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use tempfile::TempDir;

const TOKEN: &str = "00112233445566778899aabbccddeeff";

fn claim(hash: &str) -> BinaryClaim {
    let key = WorkerJobKey::new(RepositoryIndex::new(4), WorkerJobIndex::new(0));
    let request = NativeJobRequest::parse_bounded(&serde_json::to_vec(&json!({
        "contract":NATIVE_JOB_CONTRACT,"source":{"kind":"workflow_package","repository_index":4,
            "package_hash":hash,"base_snapshot_id":"SNP-000000000001","revision_snapshot_id":"SNP-000000000003"},
        "command":{"argv":["./ci/run"]}
    })).unwrap()).unwrap();
    BinaryClaim {
        proof: WorkerLeaseProof::new(key, 3, TOKEN).unwrap(),
        lease_expires_at_s: u32::MAX,
        job_kind: 7,
        request,
    }
}

fn reference(claim: &BinaryClaim) -> WorkflowPackageReference {
    let SourceSpec::WorkflowPackage {
        package_hash,
        base_snapshot_id,
        revision_snapshot_id,
        ..
    } = &claim.request.source
    else {
        unreachable!()
    };
    WorkflowPackageReference {
        key: claim.proof.key(),
        package_hash: package_hash.clone(),
        base_snapshot_id: base_snapshot_id.clone(),
        revision_snapshot_id: revision_snapshot_id.clone(),
    }
}

type Request = (String, Vec<(String, String)>, Vec<u8>);

fn request(stream: &mut std::net::TcpStream) -> Request {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut input = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    input.read_line(&mut line).unwrap();
    let request = line.trim().to_string();
    let mut headers = Vec::new();
    loop {
        line.clear();
        assert!(input.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').unwrap();
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .map(|(_, value)| value.parse().unwrap())
        .unwrap_or(0);
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes).unwrap();
    (request, headers, bytes)
}

fn server(
    status: u16,
    headers: Vec<(String, String)>,
    bytes: Vec<u8>,
) -> (String, thread::JoinHandle<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        let request = request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 {status} response\r\nContent-Length: {}\r\nConnection: close\r\n",
            bytes.len()
        )
        .unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        stream.write_all(b"\r\n").unwrap();
        stream.write_all(&bytes).unwrap();
        request
    });
    (url, handle)
}

fn headers(hash: &str) -> Vec<(String, String)> {
    vec![
        ("Content-Type".into(), WORKFLOW_PACKAGE_MEDIA_TYPE.into()),
        ("X-Ait-Package-Sha256".into(), hash.into()),
    ]
}

#[test]
fn package_get_carries_exact_claim_proof_and_accepts_original_bytes_without_snapshot_fallback() {
    let bytes = b"original package bytes".to_vec();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let claim = claim(&hash);
    let reference = reference(&claim);
    let (url, server) = server(200, headers(&hash), bytes.clone());
    let client = ServerClient::new(&url, Some("worker-auth".into())).unwrap();
    let control = ExecutionControl::default();
    let provider = ClaimedSourceProvider {
        client: &client,
        claim: &claim,
        control: &control,
    };
    let root = TempDir::new().unwrap();
    let path = root.path().join("source.zst");
    assert_eq!(
        provider
            .download_workflow_package(&reference, &path, 100)
            .unwrap(),
        bytes.len() as u64
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
    let (line, headers, body) = server.join().unwrap();
    assert_eq!(
        line,
        "GET /v1/native/repository-authorities/4/worker-jobs/0 HTTP/1.1"
    );
    assert!(body.is_empty());
    for (name, value) in [
        ("accept", WORKFLOW_PACKAGE_MEDIA_TYPE),
        ("x-ait-package-sha256", hash.as_str()),
        ("x-ait-worker-attempt-count", "3"),
        ("x-ait-worker-lease-token", TOKEN),
        ("authorization", "Bearer worker-auth"),
    ] {
        let values: Vec<_> = headers
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(values, vec![value], "{name}");
    }
}

#[test]
fn package_get_rejects_missing_wrong_duplicate_headers_auth_failures_missing_sources_and_redirects()
{
    let hash = "a".repeat(64);
    let claim = claim(&hash);
    let reference = reference(&claim);
    let mut missing = headers(&hash);
    missing.pop();
    let mut wrong_hash = headers(&"b".repeat(64));
    let mut duplicate = headers(&hash);
    duplicate.push(("X-Ait-Package-Sha256".into(), hash.clone()));
    let mut wrong_media = headers(&hash);
    wrong_media[0].1 = "application/json".into();
    let cases = [
        (200, missing),
        (200, std::mem::take(&mut wrong_hash)),
        (200, duplicate),
        (200, wrong_media),
        (403, headers(&hash)),
        (404, headers(&hash)),
        (409, headers(&hash)),
        (302, vec![("Location".into(), "/must-not-follow".into())]),
    ];
    for (status, headers) in cases {
        let (url, server) = server(status, headers, b"unavailable".to_vec());
        let client = ServerClient::new(&url, None).unwrap();
        let control = ExecutionControl::default();
        let provider = ClaimedSourceProvider {
            client: &client,
            claim: &claim,
            control: &control,
        };
        let root = TempDir::new().unwrap();
        assert!(
            provider
                .download_workflow_package(&reference, &root.path().join("source"), 100)
                .is_err(),
            "{status}"
        );
        assert_eq!(
            server.join().unwrap().0,
            "GET /v1/native/repository-authorities/4/worker-jobs/0 HTTP/1.1"
        );
    }
}

#[test]
fn package_get_rejects_a_changed_job_or_invalid_attempt_proof_before_network_access() {
    let mut claim = claim(&"a".repeat(64));
    let mut reference = reference(&claim);
    let client = ServerClient::new("http://127.0.0.1:1", None).unwrap();
    let control = ExecutionControl::default();
    let root = TempDir::new().unwrap();
    reference.key.worker_job_index = WorkerJobIndex::new(1);
    let provider = ClaimedSourceProvider {
        client: &client,
        claim: &claim,
        control: &control,
    };
    assert!(matches!(
        provider.download_workflow_package(&reference, &root.path().join("source"), 100),
        Err(RunnerError::InvalidRequest(_))
    ));
    reference.key = claim.proof.key();
    for (attempt, token) in [(0, TOKEN), (3, "")] {
        claim.proof.attempt_count = attempt;
        claim.proof.lease_token = token.into();
        let provider = ClaimedSourceProvider {
            client: &client,
            claim: &claim,
            control: &control,
        };
        assert!(matches!(
            provider.download_workflow_package(&reference, &root.path().join("source"), 100),
            Err(RunnerError::InvalidRequest(_))
        ));
    }
}

#[cfg(unix)]
#[test]
fn heartbeat_cancel_and_lease_loss_stop_the_owned_process_tree_and_clean_the_attempt() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for status in [200, 409] {
        let source = TempDir::new().unwrap();
        fs::create_dir(source.path().join("ci")).unwrap();
        fs::write(source.path().join("ci/run.sh"),b"#!/bin/sh\nset -eu\nsleep 30 &\nchild=$!\nprintf '%s' \"$child\" > \"$PID_FILE\"\nwait \"$child\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            source.path().join("ci/run.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let attempts = TempDir::new().unwrap();
        let signal = TempDir::new().unwrap();
        let pid_path = signal.path().join("child.pid");
        let marker = pid_path.clone();
        let claim = claim(&"a".repeat(64));
        let proof = claim.proof.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let server = thread::spawn(move || {
            let mut captured = Vec::new();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !stopped.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                captured.push(request(&mut stream));
                let (response_status, payload) = if !marker.exists() {
                    (
                        200,
                        json!({"contract":BINARY_QUEUE_SERVICE_CONTRACT,"operation":"heartbeat",
                        "job":{"repository_index":4,"worker_job_index":0,"attempt_count":3,"state_kind":2},"cancel_requested":false}),
                    )
                } else if status == 200 {
                    (
                        200,
                        json!({"contract":BINARY_QUEUE_SERVICE_CONTRACT,"operation":"heartbeat",
                        "job":{"repository_index":4,"worker_job_index":0,"attempt_count":3,"state_kind":2},"cancel_requested":true}),
                    )
                } else {
                    (status, json!({"detail":"stale worker lease"}))
                };
                let bytes = serde_json::to_vec(&payload).unwrap();
                write!(stream,"HTTP/1.1 {response_status} response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();
                stream.write_all(&bytes).unwrap();
            }
            captured
        });
        let heartbeat = HeartbeatLease::start_binary(
            ServerClient::new(&url, None).unwrap(),
            proof,
            Duration::from_millis(50),
        );
        let control = heartbeat.control.clone();
        let execution_control = control.clone();
        let executor = NativeExecutor::new(crate::ExecutorConfig {
            source_root: source.path().into(),
            attempt_root: attempts.path().into(),
        });
        let job = NativeJobRequest {
            contract: NATIVE_JOB_CONTRACT.into(),
            label: None,
            suite_id: None,
            timeout_ms: 30_000,
            source: SourceSpec::LocalDirectory { path: ".".into() },
            command: crate::protocol::CommandSpec {
                argv: vec!["./ci/run".into()],
                working_directory: ".".into(),
                environment: std::collections::BTreeMap::from([(
                    "PID_FILE".into(),
                    pid_path.to_string_lossy().into_owned(),
                )]),
            },
        };
        let execution = thread::spawn(move || {
            executor.execute_with_control(&job, None, None, &execution_control)
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while !control.is_cancelled() {
            if execution.is_finished() {
                panic!(
                    "execution ended before its cancellation signal: {:?}",
                    execution.join().unwrap()
                );
            }
            assert!(
                std::time::Instant::now() < deadline,
                "heartbeat did not stop execution"
            );
            thread::sleep(Duration::from_millis(2));
        }
        let heartbeat_result = heartbeat.finish();
        stop.store(true, Ordering::SeqCst);
        assert_eq!(
            heartbeat_result.is_ok(),
            status == 200,
            "{heartbeat_result:?}"
        );
        assert!(matches!(
            execution.join().unwrap(),
            Err(RunnerError::Cancelled)
        ));
        assert_eq!(fs::read_dir(attempts.path()).unwrap().count(), 0);
        let pid = fs::read_to_string(&pid_path)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let mut gone = false;
        for _ in 0..50 {
            // SAFETY: signal zero only probes the exact child created by this test.
            if unsafe { libc::kill(pid, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                gone = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(gone, "owned child survived cancellation");
        let captured = server.join().unwrap();
        assert!(!captured.is_empty());
        for (_, _, body) in captured {
            let body: JsonValue = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["attempt_count"], 3);
            assert_eq!(body["lease_token"], TOKEN);
        }
    }
}

#[test]
fn cancelled_execution_reports_existing_nonretryable_failure_with_its_original_proof() {
    let claim = claim(&"a".repeat(64));
    let reply = json!({"contract":BINARY_QUEUE_SERVICE_CONTRACT,"operation":"fail",
        "job":{"repository_index":4,"worker_job_index":0,"attempt_count":3,"state_kind":4}});
    let (url, server) = server(
        200,
        vec![("Content-Type".into(), "application/json".into())],
        serde_json::to_vec(&reply).unwrap(),
    );
    let client = ServerClient::new(&url, None).unwrap();
    let error = client
        .fail_binary_after_claim(&claim, RunnerError::Cancelled)
        .unwrap_err();
    assert!(error.to_string().contains("acknowledged fail"));
    let (_, _, body) = server.join().unwrap();
    let body: JsonValue = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["attempt_count"], 3);
    assert_eq!(body["lease_token"], TOKEN);
    assert_eq!(body["detail"]["retryable"], false);
}

#[test]
fn lease_changes_during_download_do_not_block_heartbeats_or_leave_an_owned_attempt() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let bytes = vec![42u8; 128 * 1024];
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let claim = claim(&hash);
    let heartbeat_path = worker_job_operation_path(claim.proof.key(), "heartbeat").unwrap();
    let fail_path = worker_job_operation_path(claim.proof.key(), "fail").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let copying = Arc::new(AtomicBool::new(false));
    let changed = Arc::new(AtomicBool::new(false));
    let heartbeats = Arc::new(AtomicUsize::new(0));
    let stopped = stop.clone();
    let active = copying.clone();
    let lease = changed.clone();
    let observed = heartbeats.clone();
    let server = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut workers = Vec::new();
        while !stopped.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let copying = active.clone();
                    let changed = lease.clone();
                    let heartbeats = observed.clone();
                    let hash = hash.clone();
                    let heartbeat_path = heartbeat_path.clone();
                    let fail_path = fail_path.clone();
                    workers.push(thread::spawn(move || {
                        let (line,headers,body) = request(&mut stream);
                        let path = line.split_whitespace().nth(1).unwrap();
                        if line.starts_with("GET ") {
                            assert_eq!(path,"/v1/native/repository-authorities/4/worker-jobs/0");
                            assert!(headers.iter().any(|(key,value)| key == "x-ait-worker-attempt-count" && value == "3"));
                            assert!(headers.iter().any(|(key,value)| key == "x-ait-worker-lease-token" && value == TOKEN));
                            copying.store(true,Ordering::SeqCst);
                            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {WORKFLOW_PACKAGE_MEDIA_TYPE}\r\nX-Ait-Package-Sha256: {hash}\r\nContent-Length: 131072\r\nConnection: close\r\n\r\n*").unwrap();
                            let deadline = std::time::Instant::now()+Duration::from_secs(3);
                            while heartbeats.load(Ordering::SeqCst)<2 {
                                assert!(std::time::Instant::now()<deadline,"download starved its heartbeats");
                                thread::sleep(Duration::from_millis(2));
                            }
                            changed.store(true,Ordering::SeqCst);
                            // The source copy loses its authorized lease and terminates
                            // without serving the remainder to the stale attempt.
                            return;
                        }
                        let body:JsonValue = serde_json::from_slice(&body).unwrap();
                        assert_eq!(body["attempt_count"],3);
                        assert_eq!(body["lease_token"],TOKEN);
                        let (status,payload) = if path == heartbeat_path {
                            if copying.load(Ordering::SeqCst) { heartbeats.fetch_add(1,Ordering::SeqCst); }
                            if changed.load(Ordering::SeqCst) { (409,json!({"detail":"stale worker lease"})) }
                            else { (200,json!({"contract":BINARY_QUEUE_SERVICE_CONTRACT,"operation":"heartbeat",
                                "job":{"repository_index":4,"worker_job_index":0,"attempt_count":3,"state_kind":2}})) }
                        } else {
                            assert_eq!(path,fail_path);
                            (409,json!({"detail":"stale worker lease cannot release another attempt"}))
                        };
                        let bytes = serde_json::to_vec(&payload).unwrap();
                        write!(stream,"HTTP/1.1 {status} response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();
                        stream.write_all(&bytes).unwrap();
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let attempts = TempDir::new().unwrap();
    let source = TempDir::new().unwrap();
    let executor = NativeExecutor::new(crate::ExecutorConfig {
        source_root: source.path().into(),
        attempt_root: attempts.path().into(),
    });
    let result = ServerClient::new(&url, None)
        .unwrap()
        .execute_binary_claimed(&executor, &claim, Duration::from_millis(10), None);
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    assert!(result.is_err());
    assert!(changed.load(Ordering::SeqCst));
    assert!(heartbeats.load(Ordering::SeqCst) >= 2);
    assert_eq!(fs::read_dir(attempts.path()).unwrap().count(), 0);
}

#[test]
fn package_and_lease_errors_never_echo_the_claim_token() {
    let claim = claim(&"a".repeat(64));
    for package in [true, false] {
        let (url, server) = server(
            403,
            vec![("Content-Type".into(), "application/json".into())],
            serde_json::to_vec(&json!({"detail":format!("invalid proof {TOKEN}")})).unwrap(),
        );
        let client = ServerClient::new(&url, None).unwrap();
        let error = if package {
            let control = ExecutionControl::default();
            let provider = ClaimedSourceProvider {
                client: &client,
                claim: &claim,
                control: &control,
            };
            let root = TempDir::new().unwrap();
            provider
                .download_workflow_package(&reference(&claim), &root.path().join("source"), 100)
                .unwrap_err()
        } else {
            client
                .heartbeat_binary_job(&claim.proof, Duration::from_secs(1))
                .unwrap_err()
        };
        let message = error.to_string();
        assert!(message.contains("403"));
        assert!(!message.contains(TOKEN));
        server.join().unwrap();
    }
}

#[cfg(unix)]
#[test]
fn cancellation_received_during_final_heartbeat_join_cannot_report_success() {
    use std::os::unix::fs::PermissionsExt;
    let source = TempDir::new().unwrap();
    fs::create_dir(source.path().join("ci")).unwrap();
    fs::write(source.path().join("ci/run.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(
        source.path().join("ci/run.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let attempts = TempDir::new().unwrap();
    let mut claim = claim(&"a".repeat(64));
    claim.request.source = SourceSpec::LocalDirectory { path: ".".into() };
    let control = ExecutionControl::default();
    let executor = NativeExecutor::new(crate::ExecutorConfig {
        source_root: source.path().into(),
        attempt_root: attempts.path().into(),
    });
    let execution = executor.execute_with_control(&claim.request, None, None, &control);
    assert!(execution.as_ref().unwrap().cleanup.attempt_root_removed);
    assert!(!control.is_cancelled());

    // Model a last in-flight heartbeat returning cancellation exactly when
    // finish joins it. No network response waits on process timing or cleanup.
    let (stop, stopped) = mpsc::channel();
    let heartbeat_control = control.clone();
    let heartbeat = HeartbeatLease {
        stop,
        thread: thread::spawn(move || {
            stopped.recv().unwrap();
            heartbeat_control.cancel();
        }),
        error: Arc::new(Mutex::new(None)),
        control,
    };
    let reply = json!({"contract":BINARY_QUEUE_SERVICE_CONTRACT,"operation":"fail",
        "job":{"repository_index":4,"worker_job_index":0,"attempt_count":3,"state_kind":4}});
    let (url, server) = server(
        200,
        vec![("Content-Type".into(), "application/json".into())],
        serde_json::to_vec(&reply).unwrap(),
    );
    assert!(
        ServerClient::new(&url, None)
            .unwrap()
            .finish_binary_execution(&claim, execution, heartbeat, None)
            .is_err()
    );
    let (line, _, body) = server.join().unwrap();
    assert!(line.contains(":fail"));
    let body: JsonValue = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["detail"]["retryable"], false);
    assert_eq!(fs::read_dir(attempts.path()).unwrap().count(), 0);
}
