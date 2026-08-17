use std::fs;
use std::path::{Path, PathBuf};

#[path = "../../../test_support.rs"]
mod workspace_test_support;

const RETIRED_AGENT_SESSION_EXPORTS: &[&str] = &[
    "deferred_reply_scheduler_kernel_version",
    "deferred_reply_scheduler_kernel_schema",
    "deferred_reply_scheduler_build_watch_state",
    "deferred_reply_scheduler_next_wait_timeout",
    "deferred_reply_scheduler_is_watch_due",
    "deferred_reply_scheduler_is_watch_exhausted",
    "deferred_reply_scheduler_mark_watch_inflight",
    "deferred_reply_scheduler_resolve_watch_attempt",
    "agent_outbox_contract_version",
    "agent_outbox_contract_schema",
    "agent_outbox_supported_operation_kinds",
    "agent_outbox_build_entry",
    "agent_outbox_build_headers",
    "agent_outbox_normalize_entry",
    "agent_outbox_headers",
    "agent_outbox_resolve_delivery",
    "agent_outbox_entry_is_due",
    "ait_agent_transport_envelope_build_binding_metadata",
    "ait_agent_management",
];

const SUPPORTED_AIT_AGENT_EXPORTS: &[&str] = &[
    "ait_agent_env_file_load",
    "ait_agent_telegram_message_delivery_execute",
    "ait_agent_telegram_turn_input_plan",
    "ait_agent_telegram_workflow_notification_format",
    "ait_agent_telegram_workflow_query_plan",
    "ait_agent_web_runtime_execute",
    "ait_agent_worker_capabilities",
    "ait_agent_worker_transaction",
];

#[test]
fn ait_py_rejects_retired_agent_session_compatibility_exports() {
    let compact_source = compact_source(&ait_py_source());

    for export_name in RETIRED_AGENT_SESSION_EXPORTS {
        let pyfunction_marker = format!("#[pyfunction(name=\"{export_name}\"");
        assert!(
            !compact_source.contains(pyfunction_marker.as_str()),
            "retired PyO3 function export `{export_name}` must stay removed"
        );

        let wrapper_name = format!("{export_name}_py");
        let registration_marker = format!("wrap_pyfunction!({wrapper_name}");
        assert!(
            !compact_source.contains(registration_marker.as_str()),
            "retired PyO3 module registration `{export_name}` must stay removed"
        );
    }
}

#[test]
fn ait_py_rejects_retired_doctor_overrides_and_wheel_export() {
    let compact_source = compact_source(&ait_py_source());

    for export_name in [
        "task_workflow_doctor_plan_authority_wheel",
        "plan_diagnostics_normalize_wheel_status",
        "plan_diagnostics_build_wheel_status_facts",
    ] {
        assert!(
            !compact_source.contains(export_name),
            "wheel-specific PyO3 export `{export_name}` must stay removed"
        );
    }
    assert!(
        !compact_source.contains("signature=(repo_root,*,server_data="),
        "runtime-root must not retain a hidden server-data override"
    );
    assert!(
        !compact_source.contains("task_workflow_doctor_plan_authority_py(py:Python<'_>,backend:"),
        "plan-authority must not retain a hidden backend override"
    );
}

#[test]
fn ait_py_agent_export_surface_matches_supported_external_consumers_exactly() {
    let source = ait_py_source();
    let compact_source = compact_source(&source);
    let mut expected = SUPPORTED_AIT_AGENT_EXPORTS
        .iter()
        .map(|export_name| export_name.to_string())
        .collect::<Vec<_>>();
    expected.sort();

    let mut actual = source
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("#[pyfunction(name = \"")
                .and_then(|rest| rest.split('"').next())
                .filter(|export_name| export_name.starts_with("ait_agent_"))
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    actual.sort();

    assert_eq!(actual, expected);
    assert_ait_py_exports(&compact_source, SUPPORTED_AIT_AGENT_EXPORTS);
    assert_eq!(
        compact_source
            .matches("wrap_pyfunction!(ait_agent_")
            .count(),
        SUPPORTED_AIT_AGENT_EXPORTS.len(),
        "only supported ait_agent PyO3 registrations may remain"
    );
}

#[test]
fn supported_blocking_agent_exports_release_the_python_gil() {
    let source = ait_py_source();
    for export_name in [
        "ait_agent_env_file_load",
        "ait_agent_telegram_message_delivery_execute",
        "ait_agent_web_runtime_execute",
        "ait_agent_worker_capabilities",
        "ait_agent_worker_transaction",
    ] {
        let marker = format!("#[pyfunction(name = \"{export_name}\"");
        let start = source
            .find(&marker)
            .unwrap_or_else(|| panic!("missing export marker for {export_name}"));
        let function_source = &source[start..];
        let end = function_source[marker.len()..]
            .find("#[pyfunction")
            .map(|offset| marker.len() + offset)
            .unwrap_or(function_source.len());
        assert!(
            function_source[..end].contains(".detach("),
            "{export_name} must release the Python GIL during Rust execution"
        );
    }
}

fn ait_py_source() -> String {
    let mut source_paths = Vec::new();
    let source_root = workspace_test_support::crate_root("ait-py").join("src");
    collect_rust_source_paths(&source_root, &mut source_paths);
    source_paths.sort();

    source_paths
        .into_iter()
        .map(|path| {
            fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn collect_rust_source_paths(source_root: &Path, source_paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(source_root)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", source_root.display()))
    {
        let path = entry.expect("failed to read source directory entry").path();
        if path.is_dir() {
            collect_rust_source_paths(&path, source_paths);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            source_paths.push(path);
        }
    }
}

fn compact_source(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn assert_ait_py_exports(compact_source: &str, export_names: &[&str]) {
    for export_name in export_names {
        let pyfunction_marker = format!("#[pyfunction(name=\"{export_name}\"");
        assert!(
            compact_source.contains(pyfunction_marker.as_str()),
            "missing PyO3 function export marker for `{export_name}`"
        );

        let wrapper_name = format!("{export_name}_py");
        let registration_marker = format!("wrap_pyfunction!({wrapper_name}");
        assert!(
            compact_source.contains(registration_marker.as_str()),
            "missing PyO3 module registration for `{export_name}`"
        );
    }
}
