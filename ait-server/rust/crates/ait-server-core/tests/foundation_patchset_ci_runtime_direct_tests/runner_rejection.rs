#[test]
fn patchset_ci_runtime_rejects_retired_inline_binary_files() {
    let root = temp_root("retired-binary-materialized-file");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let retired_binary_key = ["content", "base64"].join("_");
    let mut materialized_file = json!({
        "path": ".artifacts/image.bin",
        "mode": "0o600"
    });
    materialized_file[retired_binary_key.as_str()] = json!("AAECA/8=");

    let err = patchset_ci_run_json(&json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": false,
        "materialized_files": [materialized_file],
        "suites": [
            {
                "suite_id": "preflight",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["test -f .artifacts/image.bin"]
                }
            }
        ]
    }))
    .expect_err("retired inline binary materialization should fail closed");
    assert!(err.contains("pack-backed materialization"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_legacy_runner_fails_closed_without_execution() {
    let root = temp_root("patchset-legacy-runner");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let error = patchset_ci_run_json(&json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "materialized_files": [],
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "python",
                "args": ["--version"]
            }
        }]
    }))
    .expect_err("legacy runner must fail closed");

    assert!(error.contains("Unsupported patchset CI runner kind `python`"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_rejects_pytest_runner_as_python_fallback() {
    let root = temp_root("patchset-pytest-rejected");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let error = patchset_ci_run_json(&json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "pytest",
                "program": "pytest",
                "args": ["--version"]
            }
        }]
    }))
    .expect_err("legacy pytest runner must fail closed");

    assert!(error.contains("no longer supported"));
    assert!(error.contains("native Rust"));
    assert!(error.contains("command_bundle"));
    assert!(error.contains("server_tg1_required"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn tg1_patchset_flow_rejects_legacy_runner_authority() {
    let root = temp_root("tg1-flow-legacy-runner-rejected");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let shared_cargo_target_dir = root.join(".ait/cargo-target");

    let error = patchset_ci_run_json(&json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "shared_cargo_target_dir": shared_cargo_target_dir.to_string_lossy(),
        "patchset_ci_flow": {
            "kind": "tg1_patchset_ci",
            "prewarm": {"required": true},
            "parallelism": {"cpu_tokens": 10, "require_exact": true},
            "runner_authority": {"rust_only": true},
            "cargo": {"shared_target_required": true}
        },
        "prewarm_commands": ["printf warm > .ait-prewarm-marker"],
        "suites": [
            {
                "suite_id": "preflight",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "python",
                    "args": ["--version"]
                }
            }
        ]
    }))
    .expect_err("tg1_patchset_ci flow should reject legacy runner suites");

    assert!(error.contains("Rust-owned suite runners"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn tg1_patchset_flow_rejects_pytest_runner_authority() {
    let root = temp_root("tg1-flow-pytest-rejected");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let shared_cargo_target_dir = root.join(".ait/cargo-target");
    fs::create_dir_all(workspace.join("src")).expect("workspace src should be created");

    let error = patchset_ci_run_json(&json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "shared_cargo_target_dir": shared_cargo_target_dir.to_string_lossy(),
        "execution_profile": "full",
        "patchset_ci_flow": {
            "kind": "tg1_patchset_ci",
            "prewarm": {"required": true},
            "parallelism": {"cpu_tokens": 10, "require_exact": true},
            "runner_authority": {"rust_only": true},
            "cargo": {"shared_target_required": true}
        },
        "prewarm_commands": ["printf warm > .ait-prewarm-marker"],
        "suites": [
            {
                "suite_id": "preflight",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "pytest",
                    "program": "/bin/sh",
                    "args": [
                        "-c",
                        "case \":${PYTHONPATH}:\" in *\":${AIT_CI_WORKSPACE_PATH}:\"*) ;; *) echo workspace missing from PYTHONPATH >&2; exit 1;; esac\ncase \":${PYTHONPATH}:\" in *\":${AIT_CI_WORKSPACE_PATH}/src:\"*) ;; *) echo workspace src missing from PYTHONPATH >&2; exit 1;; esac\nif [ \"${PYTHONDONTWRITEBYTECODE}\" != \"1\" ]; then echo bytecode writes not disabled >&2; exit 1; fi\nif [ \"${AIT_PATCHSET_CI_PREWARMED}\" != \"1\" ]; then echo prewarm marker missing >&2; exit 1; fi\nif [ \"${AIT_PATCHSET_CI_PREWARM_POLICY}\" != \"once_per_run\" ]; then echo prewarm policy not enforced >&2; exit 1; fi\nif [ \"${AIT_PATCHSET_CI_CARGO_CACHE_MODE}\" != \"prewarmed_readonly\" ]; then echo cargo cache mode not enforced >&2; exit 1; fi\nif [ \"${AIT_RUST_PREWARM_COMPACT}\" != \"0\" ]; then echo pytest worker should not compact cargo cache >&2; exit 1; fi\necho rust pytest shim pass",
                        "-n0",
                        "--dist",
                        "loadscope"
                    ]
                }
            }
        ]
    }))
    .expect_err("tg1_patchset_ci flow should reject pytest runners");

    assert!(error.contains("native Rust runners"));
    assert!(error.contains("command_bundle"));
    assert!(error.contains("server_tg1_required"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_bundle_tg1_required_is_rejected_before_external_test_runner() {
    let root = temp_root("command-bundle-tg1-rejected");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let error = patchset_ci_run_json(&json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "suites": [
            {
                "suite_id": "tg1_required",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": false,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["printf should-not-run"]
                }
            }
        ]
    }))
    .expect_err("TG1 command_bundle runner should be rejected");

    assert!(error.contains("server_tg1_required"));

    let _ = fs::remove_dir_all(root);
}
