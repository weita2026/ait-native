#[test]
fn patchset_ci_runtime_cleans_workspace_when_suite_errors() {
    let root = temp_root("error-cleanup");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let error = patchset_ci_run_json(&json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn should_cleanup() {}\\n"}
        ],
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {"kind": "unsupported"}
        }]
    }))
    .expect_err("unsupported runner should fail the runtime");

    assert!(error.contains("Unsupported patchset CI runner kind"));
    assert!(
        !workspace.exists(),
        "workspace must be cleaned even when suite execution returns Err"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_cleanup_removes_readonly_suite_artifacts() {
    let root = temp_root("readonly-cleanup");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": [
                    "mkdir -p readonly/child && printf locked > readonly/child/file.txt && chmod 0555 readonly/child readonly"
                ]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["cleanup"]["status"], json!("cleaned"));
    assert!(
        !workspace.exists(),
        "cleanup should remove readonly directories created by suite commands"
    );

    let _ = fs::remove_dir_all(root);
}
