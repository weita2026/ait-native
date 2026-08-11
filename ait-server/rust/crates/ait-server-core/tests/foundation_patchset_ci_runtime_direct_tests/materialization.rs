#[test]
fn patchset_ci_runtime_can_create_rust_owned_runtime_paths() {
    let root = temp_root("rust-owned-paths");
    let runtime_root = root.join("runtime");
    let server_data_root = root.join("server-data");
    fs::create_dir_all(&server_data_root).expect("server data root should be created");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "server_data_root": server_data_root.to_string_lossy(),
        "ci_temp_root": runtime_root.to_string_lossy(),
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn rust_owned() -> bool { true }\\n"}
        ],
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": [
                    "test -f src/lib.rs && test \"$AIT_CI_RUNNER_PARALLELISM\" = 10 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 10 && test \"$CARGO_BUILD_JOBS\" = 10 && test \"$RUST_TEST_THREADS\" = 10"
                ]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["suite_pool"]["max_cpu_tokens"], json!(10));
    assert_eq!(
        value["suite_pool"]["scheduled_suites"][0]["cpu_tokens"],
        json!(10)
    );
    assert_eq!(value["suite_results"][0]["runner_parallelism"], json!(10));
    assert_eq!(
        value["suite_results"][0]["runner_parallelism_source"],
        json!("scheduler_admitted_cpu_tokens")
    );
    assert_eq!(
        value["suite_results"][0]["server_ci_gate"]["runner_parallelism"],
        json!(10)
    );
    let runtime_base = runtime_base_from_log(&value);
    assert!(runtime_base.starts_with(&runtime_root));
    assert!(
        !runtime_base.exists(),
        "Rust-owned run base, including outputs and manifest, should be cleaned on terminal state"
    );
    assert_eq!(value["cleanup"]["removed_scope"], json!("managed_run_base"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_materializes_snapshot_runs_one_prewarm_for_multiple_suites_and_keeps_logs_after_cleanup(
) {
    let root = temp_root("prewarm-once");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "policy_mode": "async",
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn changed() -> bool { true }\\n"}
        ],
        "prewarm_commands": [
            "test ! -f .ait-prewarm-count && printf 1 > .ait-prewarm-count && printf warm > .ait-prewarm-marker"
        ],
        "suites": [
            {
                "suite_id": "preflight_a",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": [
                        "test -f src/lib.rs && test -f .ait-prewarm-marker && printf a > suite-a.txt"
                    ]
                }
            },
            {
                "suite_id": "preflight_b",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": [
                        "test -f src/lib.rs && test -f .ait-prewarm-marker && printf b > suite-b.txt"
                    ]
                }
            }
        ]
    }));

    assert_eq!(value["contract"], json!("ait.server.patchset_ci.run.v1"));
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["native_prewarm"]["status"], json!("pass"));
    assert_eq!(value["native_prewarm"]["command_count"], json!(1));
    assert_eq!(
        value["native_prewarm"]["reports"]
            .as_array()
            .expect("prewarm reports should be an array")
            .len(),
        1
    );
    assert_eq!(value["suite_results"].as_array().unwrap().len(), 2);
    assert_eq!(value["suite_pool"]["mode"], json!("bounded_parallel"));
    assert_eq!(value["suite_pool"]["max_cpu_tokens"], json!(10));
    assert_eq!(value["suite_pool"]["prewarm_barrier"], json!(true));
    assert_eq!(value["suite_pool"]["prewarm_once"], json!(true));
    assert_eq!(value["suite_pool"]["suite_count"], json!(2));
    assert!(value["suite_results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|suite| suite["runner_kind"] == json!("rust_server_ci")));
    assert_eq!(
        value["patchset_ci_detail"]["server_ci_gate"]["python_server_ci_executor"],
        json!(false)
    );
    assert_eq!(value["patchset_ci_completion"]["ci_run_seq"], json!(1));
    assert!(value.get("attestation_update").is_none());
    assert_eq!(
        value["server_ci_gate"]["rust_patchset_ci_runtime"],
        json!(true)
    );
    assert_eq!(value["policy_job_payload"]["patchset_id"], json!("RCP-1"));
    assert!(!workspace.exists(), "cleanup should remove dirty workspace");

    for suite in value["suite_results"].as_array().unwrap() {
        let log_path = PathBuf::from(
            suite["artifacts"]["log_path"]["path"]
                .as_str()
                .expect("suite log path should be text"),
        );
        assert!(log_path.is_file(), "suite log must outlive workspace");
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_uses_main_seed_prewarm_evidence_without_per_run_prewarm() {
    let root = temp_root("main-seed-prewarm-evidence");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    fs::create_dir_all(workspace.join("src")).expect("workspace source should be created");
    fs::write(workspace.join("src").join("lib.rs"), "pub fn seeded() {}\n")
        .expect("workspace source should be written");
    fs::write(workspace.join(".ait-prewarm-marker"), "warm")
        .expect("main-seed marker should be present in copied workspace");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": false,
        "main_seed_prewarm": {
            "contract": "ait.server.main_seed_prewarm.v1",
            "status": "reused",
            "reused": true,
            "generation_key": "SNP-REV",
            "main_seed_path": root.join("seeds/ait/main-seed").to_string_lossy(),
            "step_count": 1,
            "steps": [],
            "duration_seconds": 0.001,
            "manifest_path": root.join("seeds/ait/main-seed/.ait/main-seed-prewarm.json").to_string_lossy()
        },
        "snapshot_materialization_result": {
            "contract": "ait.server.patchset_ci.snapshot_materialization_result.v1",
            "strategy": "server_main_seed",
            "json_snapshot_payload": false,
            "python_glue": false
        },
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": ["test -f src/lib.rs && test -f .ait-prewarm-marker"]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["native_prewarm"]["status"], json!("pass"));
    assert_eq!(value["native_prewarm"]["required"], json!(true));
    assert_eq!(
        value["native_prewarm"]["once_per_main_seed_generation"],
        json!(true)
    );
    assert_eq!(
        value["native_prewarm"]["once_per_patchset_ci_run"],
        json!(false)
    );
    assert_eq!(
        value["suite_pool"]["prewarm_scope"],
        json!("main_seed_generation")
    );
    assert_eq!(
        value["patchset_ci_detail"]["snapshot_materialization"]["python_glue"],
        json!(false)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_does_not_create_attestation_payload() {
    let root = temp_root("unresolved-provenance");
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
                "commands": ["true"]
            }
        }]
    }));

    assert!(value.get("attestation_update").is_none());
    assert_eq!(value["patchset_ci_completion"]["patchset_id"], json!("RCP-1"));
    assert_eq!(value["patchset_ci_completion"]["ci_run_seq"], json!(1));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]

#[test]
fn patchset_ci_runtime_records_patchset_completion_when_native_prewarm_fails() {
    let root = temp_root("prewarm-fail");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "execution_profile": "workflow_ready_foreground",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "prewarm_commands": ["printf prewarm-failed\\n; exit 12"],
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": ["printf should-not-run\\n"]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("fail"));
    assert_eq!(value["native_prewarm"]["status"], json!("fail"));
    assert_eq!(value["suite_results"], json!([]));
    assert_eq!(
        value["patchset_ci_detail"]["native_prewarm"]["status"],
        json!("fail")
    );
    assert_eq!(
        value["patchset_ci_completion"]["overall_status"],
        json!("fail")
    );
    assert!(value.get("attestation_update").is_none());
    assert!(
        !workspace.exists(),
        "cleanup should also run after prewarm failure"
    );
    let prewarm_log = PathBuf::from(
        value["native_prewarm"]["reports"][0]["log_path"]
            .as_str()
            .expect("prewarm log path should be text"),
    );
    assert!(prewarm_log.is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_runs_generic_test_discovery_sharded_cargo_adapter() {
    let root = temp_root("test-discovery-sharded-cargo");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let bin_dir = root.join("bin");
    let marker = root.join("shards.txt");
    let fmt_marker = root.join("fmt.txt");
    let exe_a = bin_dir.join("runtime-test-a");
    let exe_b = bin_dir.join("runtime-test-b");
    let exe_c = bin_dir.join("runtime-test-c");
    let ordinary_bin = bin_dir.join("runtime-ordinary-bin");
    for (exe, case_a, case_b) in [
        (&exe_a, "runtime_alpha::case_one", "runtime_alpha::case_two"),
        (&exe_b, "runtime_beta::case_one", "runtime_beta::case_two"),
        (&exe_c, "runtime_gamma::case_one", "runtime_gamma::case_two"),
    ] {
        write_script(
            exe,
            r#"#!/bin/sh
case " $* " in
  *" --list "*)
    cat <<'TESTS'
__CASE_A__: test
__CASE_B__: test
TESTS
    exit 0
    ;;
esac
if [ "${RUST_TEST_THREADS}" != "1" ]; then
  echo "expected RUST_TEST_THREADS=1" >&2
  exit 1
fi
if [ "${AIT_CI_TEST_SHARDING}" != "test_case" ]; then
  echo "expected test_case sharding" >&2
  exit 6
fi
if [ "$1" != "--exact" ]; then
  echo "expected --exact test case execution, got $*" >&2
  exit 7
fi
shift
for case_name in "$@"; do
  echo "${AIT_CI_SHARD_ID}:${case_name}" >> "${MARKER}"
done
exit 0
	"#
            .replace("__CASE_A__", case_a)
            .replace("__CASE_B__", case_b)
            .as_str(),
        );
    }
    write_script(
        &ordinary_bin,
        r#"#!/bin/sh
echo "ordinary bin artifact should not be executed" >&2
exit 91
"#,
    );
    let fake_cargo = bin_dir.join("cargo");
    write_script(
        &fake_cargo,
        &format!(
            r#"#!/bin/sh
case " $* " in
  *" fmt "*)
    echo fmt > "${{FMT_MARKER}}"
    ;;
  *" --no-run "*)
    if [ "${{CARGO_BUILD_JOBS}}" != "9" ]; then
      echo "expected CARGO_BUILD_JOBS=9" >&2
      exit 2
    fi
    case " $* " in
      *" --profile ait-ci "*) ;;
      *)
        echo "expected --profile ait-ci in cargo discovery args" >&2
        exit 4
        ;;
    esac
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":false}},"target":{{"name":"runtime-ordinary-bin","kind":["bin"]}},"executable":"{}"}}'
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":true}},"target":{{"name":"runtime-test-a","kind":["test"]}},"executable":"{}"}}'
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":true}},"target":{{"name":"runtime-test-b","kind":["test"]}},"executable":"{}"}}'
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":true}},"target":{{"name":"runtime-test-c","kind":["test"]}},"executable":"{}"}}'
    ;;
  *)
    echo "unexpected fake cargo args: $*" >&2
    exit 5
    ;;
esac
"#,
            ordinary_bin.to_string_lossy(),
            exe_a.to_string_lossy(),
            exe_b.to_string_lossy(),
            exe_c.to_string_lossy()
        ),
    );

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "admitted_cpu_tokens": 9,
        "cleanup_workspace": false,
        "env": {
            "MARKER": marker.to_string_lossy(),
            "FMT_MARKER": fmt_marker.to_string_lossy()
        },
        "materialized_files": [
            {"path": "rust/Cargo.toml", "content": "[workspace]\\n"},
            {"path": "src/lib.rs", "content": "pub fn sample() -> bool { true }\\n"}
        ],
        "suites": [{
            "suite_id": "rust_core",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": fake_cargo.to_string_lossy(),
                "manifest_path": "rust/Cargo.toml",
                "workspace": true,
                "checks": [{
                    "kind": "cargo_fmt",
                    "check_id": "rustfmt"
                }]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(
        value["suite_results"][0]["runner_kind"],
        json!("rust_test_discovery_sharded")
    );
    assert_eq!(value["suite_results"][0]["runner_parallelism"], json!(9));
    assert_eq!(
        value["suite_results"][0]["discovery"]["executable_count"],
        json!(3)
    );
    assert_eq!(
        value["suite_results"][0]["discovery"]["test_case_count"],
        json!(6)
    );
    assert_eq!(
        value["suite_results"][0]["test_shards"]["shard_by"],
        json!("test_case")
    );
    assert_eq!(
        value["suite_results"][0]["test_shards"]["shard_count"],
        json!(6)
    );
    assert_eq!(
        value["suite_results"][0]["server_ci_gate"]["generic_test_discovery_runner"],
        json!(true)
    );
    assert_eq!(
        value["suite_results"][0]["server_ci_gate"]["cargo_build_once"],
        json!(true)
    );
    assert_eq!(
        value["suite_results"][0]["server_ci_gate"]["test_case_shards"],
        json!(true)
    );
    assert_eq!(value["suite_results"][0]["checks"]["status"], json!("pass"));
    assert!(
        value["suite_results"][0]["discovery"]["build_report"]["command"]
            .as_str()
            .unwrap_or_default()
            .contains("--profile ait-ci")
    );
    let marker_text = fs::read_to_string(&marker).expect("shard marker should exist");
    assert_eq!(marker_text.lines().count(), 6);
    assert_eq!(
        fs::read_to_string(&fmt_marker).expect("fmt marker should exist"),
        "fmt\n"
    );

    let _ = fs::remove_dir_all(root);
}
