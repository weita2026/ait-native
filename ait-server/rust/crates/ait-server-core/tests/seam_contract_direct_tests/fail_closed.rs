#[test]
fn unsupported_command_fails_closed() {
    let output = run_seam(&["unknown-command"]);
    assert_failed_with(
        &output,
        "Unsupported ait-server-core seam command: unknown-command.",
    );
}

#[test]
fn missing_required_argument_fails_closed() {
    let output = run_seam(&["normalize-async-job-payload", "content.gc"]);
    assert_failed_with(&output, "Missing required argument `payload_json`.");
}

#[test]
fn malformed_payload_json_fails_closed() {
    let output = run_seam(&["normalize-async-job-payload", "content.gc", "{bad-json"]);
    assert_failed_with(&output, "payload_json must be valid JSON:");
}

#[test]
fn non_object_payload_json_fails_closed() {
    let output = run_seam(&["normalize-async-job-payload", "content.gc", "[]"]);
    assert_failed_with(&output, "content.gc payload must be a JSON object.");
}

#[test]
fn retired_postgres_commands_are_absent_from_the_patch_ci_seam() {
    for command in [
        "server-context",
        "patchset-store",
        "policy-store",
        "review-store",
        "worker-queue-kernel",
        "worker-queue-service",
        "postgres-runtime-probe",
    ] {
        let output = run_seam(&[command]);
        assert_failed_with(&output, "Unsupported ait-server-core seam command");
        let stderr = stderr_text(&output);
        let supported = stderr
            .split_once("Expected one of:")
            .map(|(_, supported)| supported)
            .expect("unsupported-command error should list the available patch-CI commands");
        assert!(
            !supported
                .split(',')
                .map(str::trim)
                .any(|available| available.trim_end_matches('.') == command),
            "retired PostgreSQL command must not remain in patch-CI help: {command}"
        );
    }
}
