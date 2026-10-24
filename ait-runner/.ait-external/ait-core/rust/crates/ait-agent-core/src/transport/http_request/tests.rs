use super::{
    agent_transport_http_error_message, agent_transport_http_execute_bytes_request,
    agent_transport_http_execute_json_request_json,
    agent_transport_http_execute_multipart_json_request_json,
    agent_transport_http_execute_multipart_json_request_with_bytes,
    agent_transport_http_invalid_timeout_message, agent_transport_http_plan_json_request_json,
    agent_transport_http_plan_multipart_request_json, agent_transport_http_response_payload_json,
    agent_transport_http_timeout_message, agent_transport_http_transport_error_message,
    agent_transport_http_url_error_message, AgentTransportHttpBytesExecution,
};
use ait_core::json_support::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn plans_json_request_headers_method_and_body() {
    let planned = agent_transport_http_plan_json_request_json(&json!({
        "method": "post",
        "payload": {"text": "你好", "ok": true},
        "headers": {
            "X-Trace": 7,
            "Accept": "application/custom"
        }
    }))
    .expect("json request should plan");
    assert_eq!(planned["method"], "POST");
    assert_eq!(planned["headers"]["Accept"], "application/custom");
    assert_eq!(planned["headers"]["Content-Type"], "application/json");
    assert_eq!(planned["headers"]["X-Trace"], "7");
    assert_eq!(planned["body_text"], "{\"ok\":true,\"text\":\"你好\"}");
}

#[test]
fn plans_json_request_without_payload() {
    let planned = agent_transport_http_plan_json_request_json(&json!({
        "method": "get",
        "headers": {"X-Flag": true}
    }))
    .expect("json request should plan");
    assert_eq!(planned["method"], "GET");
    assert_eq!(planned["headers"]["Accept"], "application/json");
    assert!(planned["headers"].get("Content-Type").is_none());
    assert_eq!(planned["headers"]["X-Flag"], "True");
    assert!(planned["body_text"].is_null());
}

#[test]
fn plans_multipart_request_frame_like_python_helpers() {
    let planned = agent_transport_http_plan_multipart_request_json(&json!({
        "boundary": "aittelegram-abc",
        "fields": {
            "caption": "hello",
            "skip": null,
            "count": 3
        },
        "file_field": "document",
        "file_name": "x.txt",
        "mime_type": "text/plain"
    }))
    .expect("multipart request should plan");
    assert_eq!(planned["method"], "POST");
    assert_eq!(planned["headers"]["Accept"], "application/json");
    assert_eq!(
        planned["headers"]["Content-Type"],
        "multipart/form-data; boundary=aittelegram-abc"
    );
    assert_eq!(
        planned["file_prefix_text"],
        "--aittelegram-abc\r\nContent-Disposition: form-data; name=\"caption\"\r\n\r\nhello\r\n--aittelegram-abc\r\nContent-Disposition: form-data; name=\"count\"\r\n\r\n3\r\n--aittelegram-abc\r\nContent-Disposition: form-data; name=\"document\"; filename=\"x.txt\"\r\nContent-Type: text/plain\r\n\r\n"
    );
    assert_eq!(planned["file_suffix_text"], "\r\n--aittelegram-abc--\r\n");
}

#[test]
fn classifies_empty_json_and_text_response_payloads() {
    assert_eq!(
        agent_transport_http_response_payload_json("  \n "),
        json!({"kind": "json", "value": {}})
    );
    assert_eq!(
        agent_transport_http_response_payload_json("{\"ok\":true}"),
        json!({"kind": "json", "value": {"ok": true}})
    );
    assert_eq!(
        agent_transport_http_response_payload_json("not json"),
        json!({"kind": "text", "value": "not json"})
    );
}

#[test]
fn executes_json_request_and_classifies_json_response() {
    let (url, request_rx, handle) = serve_once("200 OK", "{\"ok\":true}");
    let result = agent_transport_http_execute_json_request_json(&json!({
        "url": url,
        "method": "post",
        "payload": {"text": "hi"},
        "headers": {"X-Trace": 7},
        "timeout_seconds": 5.0
    }))
    .expect("json execution should return payload");

    assert_eq!(result["ok"], true);
    assert_eq!(result["method"], "POST");
    assert_eq!(result["status_code"], 200);
    assert_eq!(result["response_kind"], "json");
    assert_eq!(result["payload"], json!({"ok": true}));

    let raw_request = request_rx.recv().expect("server should capture request");
    let lower_request = raw_request.to_ascii_lowercase();
    assert!(raw_request.starts_with("POST / HTTP/1.1"));
    assert!(lower_request.contains("accept: application/json"));
    assert!(lower_request.contains("content-type: application/json"));
    assert!(lower_request.contains("x-trace: 7"));
    assert!(raw_request.ends_with("{\"text\":\"hi\"}"));
    handle.join().expect("server thread should finish");
}

#[test]
fn executes_multipart_request_and_classifies_json_response() {
    let (url, request_rx, handle) = serve_once("200 OK", "{\"ok\":true}");
    let result = agent_transport_http_execute_multipart_json_request_json(&json!({
        "url": url,
        "boundary": "aittelegram-test",
        "fields": {"caption": "hello", "skip": null},
        "file_field": "document",
        "file_name": "x.txt",
        "file_bytes": [102, 105, 108, 101, 45, 98, 121, 116, 101, 115],
        "mime_type": "text/plain",
        "headers": {"X-Trace": 7},
        "timeout_seconds": 5.0
    }))
    .expect("multipart execution should return payload");

    assert_eq!(result["ok"], true);
    assert_eq!(result["method"], "POST");
    assert_eq!(result["status_code"], 200);
    assert_eq!(result["response_kind"], "json");
    assert_eq!(result["payload"], json!({"ok": true}));

    let raw_request = request_rx.recv().expect("server should capture request");
    let lower_request = raw_request.to_ascii_lowercase();
    assert!(raw_request.starts_with("POST / HTTP/1.1"));
    assert!(lower_request.contains("accept: application/json"));
    assert!(lower_request.contains("content-type: multipart/form-data; boundary=aittelegram-test"));
    assert!(lower_request.contains("x-trace: 7"));
    assert!(raw_request.contains("Content-Disposition: form-data; name=\"caption\""));
    assert!(raw_request.contains("hello"));
    assert!(raw_request
        .contains("Content-Disposition: form-data; name=\"document\"; filename=\"x.txt\""));
    assert!(raw_request.contains("Content-Type: text/plain"));
    assert!(raw_request.contains("file-bytes\r\n--aittelegram-test--"));
    assert!(!raw_request.contains("skip"));
    handle.join().expect("server thread should finish");
}

#[test]
fn executes_multipart_request_with_bytes_kept_out_of_band() {
    let (url, request_rx, handle) = serve_once("200 OK", "{\"ok\":true}");
    let request = json!({
        "url": url,
        "boundary": "aittelegram-typed",
        "fields": {"caption": "hello"},
        "file_field": "document",
        "file_name": "x.bin",
        "mime_type": "application/octet-stream",
        "timeout_seconds": 5.0
    });
    assert!(request.get("file_bytes").is_none());

    let result = agent_transport_http_execute_multipart_json_request_with_bytes(
        &request,
        b"typed-file-bytes",
    )
    .expect("typed multipart execution should return payload");

    assert_eq!(result["ok"], true);
    assert_eq!(result["payload"], json!({"ok": true}));
    let raw_request = request_rx.recv().expect("server should capture request");
    assert!(raw_request.contains("typed-file-bytes\r\n--aittelegram-typed--"));
    handle.join().expect("server thread should finish");
}

#[test]
fn executes_bytes_request_and_returns_payload_bytes() {
    let (url, request_rx, handle) = serve_once("200 OK", "file-bytes");
    let result = agent_transport_http_execute_bytes_request(&json!({
        "url": url,
        "method": "get",
        "headers": {"X-Trace": 7},
        "timeout_seconds": 5.0
    }))
    .expect("byte execution should return payload");

    let AgentTransportHttpBytesExecution::Success {
        method,
        status_code,
        payload,
        ..
    } = result
    else {
        panic!("byte execution should succeed");
    };
    assert_eq!(method, "GET");
    assert_eq!(status_code, 200);
    assert_eq!(payload, b"file-bytes");

    let raw_request = request_rx.recv().expect("server should capture request");
    let lower_request = raw_request.to_ascii_lowercase();
    assert!(raw_request.starts_with("GET / HTTP/1.1"));
    assert!(lower_request.contains("x-trace: 7"));
    handle.join().expect("server thread should finish");
}

#[test]
fn executes_bytes_request_and_returns_structured_http_errors() {
    let (url, _request_rx, handle) = serve_once("404 Not Found", "missing-file");
    let result = agent_transport_http_execute_bytes_request(&json!({
        "url": url,
        "method": "get",
        "timeout_seconds": 5.0
    }))
    .expect("http error should return structured payload");

    let AgentTransportHttpBytesExecution::Error(payload) = result else {
        panic!("byte execution should return an error payload");
    };
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error_kind"], "http");
    assert_eq!(payload["method"], "GET");
    assert_eq!(payload["status_code"], 404);
    assert_eq!(payload["detail"], "missing-file");
    assert_eq!(payload["reason"], "Not Found");
    assert_eq!(
        payload["message"],
        format!(
            "GET {} failed: 404 missing-file",
            payload["url"].as_str().unwrap()
        )
    );
    handle.join().expect("server thread should finish");
}

#[test]
fn executes_json_request_and_classifies_empty_and_text_responses() {
    let (empty_url, _empty_request_rx, empty_handle) = serve_once("200 OK", "");
    let empty = agent_transport_http_execute_json_request_json(&json!({
        "url": empty_url,
        "method": "get",
        "timeout_seconds": 5.0
    }))
    .expect("empty execution should return payload");
    assert_eq!(empty["ok"], true);
    assert_eq!(empty["response_kind"], "json");
    assert_eq!(empty["payload"], json!({}));
    empty_handle.join().expect("empty server should finish");

    let (text_url, _text_request_rx, text_handle) = serve_once("200 OK", "not-json");
    let text = agent_transport_http_execute_json_request_json(&json!({
        "url": text_url,
        "method": "get",
        "timeout_seconds": 5.0
    }))
    .expect("text execution should return payload");
    assert_eq!(text["ok"], true);
    assert_eq!(text["response_kind"], "text");
    assert_eq!(text["payload"], "not-json");
    text_handle.join().expect("text server should finish");
}

#[test]
fn executes_json_request_and_returns_structured_http_errors() {
    let (url, _request_rx, handle) = serve_once("418 I'm a Teapot", "short");
    let result = agent_transport_http_execute_json_request_json(&json!({
        "url": url,
        "method": "post",
        "payload": {"x": 1},
        "timeout_seconds": 5.0
    }))
    .expect("http error should return structured payload");

    assert_eq!(result["ok"], false);
    assert_eq!(result["error_kind"], "http");
    assert_eq!(result["method"], "POST");
    assert_eq!(result["status_code"], 418);
    assert_eq!(result["detail"], "short");
    assert_eq!(result["reason"], "I'm a teapot");
    assert_eq!(
        result["message"],
        format!("POST {} failed: 418 short", result["url"].as_str().unwrap())
    );
    handle.join().expect("server thread should finish");
}

#[test]
fn rejects_invalid_timeout_as_structured_execution_error() {
    let result = agent_transport_http_execute_json_request_json(&json!({
        "url": "http://127.0.0.1:1",
        "method": "get",
        "timeout_seconds": 0.0,
        "timeout_repr": "0.0"
    }))
    .expect("invalid timeout should return structured payload");

    assert_eq!(result["ok"], false);
    assert_eq!(result["error_kind"], "invalid_timeout");
    assert_eq!(
        result["message"],
        "GET http://127.0.0.1:1 failed: invalid timeout value 0.0."
    );
}

#[test]
fn renders_transport_error_messages_like_python_helpers() {
    assert_eq!(
        agent_transport_http_timeout_message("post", "https://x.test", Some(5.0)),
        "POST https://x.test timed out after 5 seconds."
    );
    assert_eq!(
        agent_transport_http_invalid_timeout_message("post", "https://x.test", "inf"),
        "POST https://x.test failed: invalid timeout value inf."
    );
    assert_eq!(
        agent_transport_http_error_message(
            "post",
            "https://x.test",
            400,
            Some("bad request"),
            Some("Bad Request")
        ),
        "POST https://x.test failed: 400 bad request"
    );
    assert_eq!(
        agent_transport_http_error_message(
            "post",
            "https://x.test",
            400,
            Some(""),
            Some("Bad Request")
        ),
        "POST https://x.test failed: 400 Bad Request"
    );
    assert_eq!(
        agent_transport_http_url_error_message("get", "https://x.test", "dns"),
        "GET https://x.test failed: dns"
    );
    assert_eq!(
        agent_transport_http_transport_error_message("get", "https://x.test", "reset"),
        "GET https://x.test failed: reset"
    );
}

fn serve_once(
    status_line: &'static str,
    body: &'static str,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test server should accept");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout should be set");
        let raw_request = read_http_request(&mut stream);
        tx.send(raw_request).expect("request should be captured");
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("response should be written");
    });
    (url, rx, handle)
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if request_complete(&buffer) {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn request_complete(buffer: &[u8]) -> bool {
    let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let header_text = String::from_utf8_lossy(&buffer[..header_end]).to_ascii_lowercase();
    let content_length = header_text
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    buffer.len() >= header_end + 4 + content_length
}
