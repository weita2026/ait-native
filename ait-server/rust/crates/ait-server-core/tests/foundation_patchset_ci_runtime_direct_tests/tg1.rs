#[test]
fn workflow_ready_profile_runs_tg1_even_when_manifest_is_not_default_blocking_and_fails_closed() {
    let root = temp_root("workflow-ready-tg1");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let source_repo = root.join("source-repo");
    fs::create_dir_all(&source_repo).expect("source repo should be created");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "execution_profile": "workflow_ready_foreground",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "tg1": {
            "source_repo_root": source_repo.to_string_lossy(),
            "node_ids": ["tests/tg1_fixture::test_one"],
            "minimum_count": 2,
            "workers": 3,
            "program": "/bin/sh",
            "args": shard_command_args()
        },
        "suites": [
            {
                "suite_id": "preflight",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["printf preflight > preflight.txt"]
                }
            },
            {
                "suite_id": "tg1_required",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": false,
                "runner": {"kind": "server_tg1_required"}
            }
        ]
    }));

    assert_eq!(
        value["execution_profile"],
        json!("workflow_ready_foreground")
    );
    assert_eq!(value["tests_status"], json!("fail"));
    assert_eq!(
        value["patchset_ci_detail"]["selected_suite_ids"],
        json!(["preflight", "tg1_required"])
    );
    assert_eq!(value["blocking_failures"], json!(["tg1_required"]));
    let tg1 = value["suite_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|suite| suite["suite_id"] == json!("tg1_required"))
        .expect("tg1 suite should run");
    assert_eq!(tg1["blocking"], json!(true));
    assert_eq!(tg1["runner_kind"], json!("rust_server_tg1_required"));
    assert_eq!(tg1["status"], json!("fail"));
    assert_eq!(tg1["tg1_required_summary"]["live_count"], json!(1));
    assert_eq!(tg1["tg1_required_summary"]["minimum_count"], json!(2));
    assert!(value["patchset_ci_detail"]["workflow_ready_evidence_error"]
        .as_str()
        .expect("workflow ready evidence error should be text")
        .contains("not passing"));
    assert_eq!(
        tg1["server_ci_gate"]["python_server_ci_executor"],
        json!(false)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn server_tg1_required_strips_tests_prefix_when_source_root_stores_tests_at_repo_root() {
    let root = temp_root("tg1-strip-tests-prefix");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let source_repo = root.join("source-repo");
    let cli_dir = source_repo.join("cli");
    fs::create_dir_all(&cli_dir).expect("source repo test dir should be created");
    fs::write(cli_dir.join("tg1_fixture.txt"), "tg1 fixture\n")
        .expect("source fixture should be written");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "tg1": {
            "source_repo_root": source_repo.to_string_lossy(),
            "node_ids": ["tests/cli/tg1_fixture.txt::test_ok"],
            "minimum_count": 1,
            "workers": 1,
            "program": "/bin/sh",
            "args": shard_command_args()
        },
        "suites": [
            {
                "suite_id": "preflight",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["printf preflight > preflight.txt"]
                }
            },
            {
                "suite_id": "tg1_required",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": false,
                "runner": {"kind": "server_tg1_required", "minimum_count": 1}
            }
        ]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    let tg1 = value["suite_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|suite| suite["suite_id"] == json!("tg1_required"))
        .expect("tg1 suite should run");
    assert_eq!(tg1["status"], json!("pass"));
    assert_eq!(tg1["tg1_required_summary"]["live_count"], json!(1));
    assert_eq!(
        tg1["tg1_required_summary"]["runner"]["status"],
        json!("pass")
    );
    assert_eq!(
        tg1["tg1_required_summary"]["thread_pool_shards"]["shards"][0]["test_items"],
        json!(["cli/tg1_fixture.txt::test_ok"])
    );
    assert_eq!(
        tg1["tg1_required_summary"]["scheduler"]["authority"],
        json!("server_scheduler")
    );
    assert_eq!(
        tg1["tg1_required_summary"]["scheduler"]["admitted_cpu_tokens"],
        json!(1)
    );
    assert_eq!(
        tg1["tg1_required_summary"]["lifecycle"]["finish_report_count"],
        json!(1)
    );
    assert_eq!(
        tg1["tg1_required_summary"]["cleanup"]["status"],
        json!("cleaned")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn server_tg1_required_uses_at_template_native_defaults() {
    let root = temp_root("tg1-at-template-native-defaults");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let source_repo = root.join("source-repo");
    let tests_dir = source_repo.join("tests");
    let fake_bin_dir = root.join("bin");
    fs::create_dir_all(&tests_dir).expect("source repo test dir should be created");
    fs::create_dir_all(&fake_bin_dir).expect("fake native bin dir should be created");
    fs::write(
        tests_dir.join("test_tg1_default.py"),
        "def test_default_template():\n    assert True\n",
    )
    .expect("source TG1 test should be written");
    let fake_ait_cli = fake_bin_dir.join("ait-cli");
    fs::write(
        &fake_ait_cli,
        r#"#!/bin/sh
if [ "$1" != "test" ] || [ "$2" != "patchset-ci" ] || [ "$3" != "tg1-required" ] || [ "$4" != "--json" ]; then
  echo "expected native TG1 command, got: $*" >&2
  exit 2
fi
shift 4
if [ "$1" != "tests/test_tg1_default.py::test_default_template" ]; then
  echo "expected appended TG1 node id, got: $*" >&2
  exit 3
fi
if [ -n "${PYTHONPATH:-}" ]; then
  echo "native TG1 runner must not receive PYTHONPATH" >&2
  exit 4
fi
if [ "${AIT_PATCHSET_CI_PREWARMED}" != "1" ]; then
  echo "expected server prewarm marker" >&2
  exit 5
fi
if [ "${AIT_PATCHSET_CI_PREWARM_POLICY}" != "once_per_run" ]; then
  echo "expected once_per_run prewarm policy" >&2
  exit 6
fi
if [ "${AIT_PATCHSET_CI_CARGO_CACHE_MODE}" != "prewarmed_readonly" ]; then
  echo "expected prewarmed readonly cargo cache" >&2
  exit 7
fi
if [ "${AIT_RUST_PREWARM_COMPACT}" != "0" ]; then
  echo "expected native TG1 shards to leave the prewarmed cargo cache intact" >&2
  exit 8
fi
if [ "${CARGO_BUILD_JOBS}" != "1" ]; then
  echo "expected one cargo job per TG1 scheduler token" >&2
  exit 9
fi
printf "fake native template pass\n"
"#,
    )
    .expect("fake native executable should be written");
    #[cfg(unix)]
    fs::set_permissions(&fake_ait_cli, fs::Permissions::from_mode(0o755))
        .expect("fake native executable bit should be set");
    let path_env = format!(
        "{}:{}",
        fake_bin_dir.to_string_lossy(),
        env::var("PATH").unwrap_or_default()
    );

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "tg1": {
            "source_repo_root": source_repo.to_string_lossy(),
            "node_ids": ["tests/test_tg1_default.py::test_default_template"],
            "minimum_count": 1,
            "requested_cpu_tokens": 1,
            "env": {
                "PATH": path_env
            }
        },
        "suites": [
            {
                "suite_id": "tg1_required",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "server_tg1_required",
                    "namespace": "AT",
                    "template": "AT.server_tg1_required",
                    "minimum_count": 1,
                    "requested_cpu_tokens": 1
                }
            }
        ]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    let tg1 = &value["suite_results"][0];
    let runner_details = &tg1["tg1_required_summary"]["runner"]["details"];
    assert_eq!(runner_details["program"], json!("ait-cli"));
    assert_eq!(
        runner_details["args"],
        json!(["test", "patchset-ci", "tg1-required", "--json"])
    );
    assert_eq!(runner_details["append_test_items"], json!(true));
    assert_eq!(runner_details["env"]["PYTHONPATH"], JsonValue::Null);
    assert_eq!(
        runner_details["env"]["AIT_PATCHSET_CI_PREWARMED"],
        json!("1")
    );
    assert_eq!(
        runner_details["env"]["AIT_PATCHSET_CI_PREWARM_POLICY"],
        json!("once_per_run")
    );
    assert_eq!(
        runner_details["env"]["AIT_PATCHSET_CI_CARGO_CACHE_MODE"],
        json!("prewarmed_readonly")
    );
    assert_eq!(
        runner_details["env"]["AIT_RUST_PREWARM_COMPACT"],
        json!("0")
    );
    assert_eq!(runner_details["env"]["AIT_TG1_NATIVE_RUNNER"], json!("1"));
    assert_eq!(
        runner_details["env"]["AIT_TG1_RUNNER_AUTHORITY"],
        json!("rust")
    );
    assert_eq!(runner_details["env"]["CARGO_BUILD_JOBS"], json!("1"));
    assert_eq!(
        tg1["tg1_required_summary"]["thread_pool_shards"]["shards"][0]["test_items"],
        json!(["tests/test_tg1_default.py::test_default_template"])
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn server_tg1_required_uses_scheduler_admitted_thread_pool_tokens_and_cleanup() {
    let root = temp_root("tg1-scheduler-thread-pool");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let source_repo = root.join("source-repo");
    let cli_dir = source_repo.join("cli");
    fs::create_dir_all(&cli_dir).expect("source repo test dir should be created");
    fs::write(cli_dir.join("tg1_fixture.txt"), "tg1 fixture\n")
        .expect("source fixture should be written");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "suite_pool_tokens": 4,
        "tg1": {
            "source_repo_root": source_repo.to_string_lossy(),
            "node_ids": [
                "tests/cli/tg1_fixture.txt::test_one",
                "tests/cli/tg1_fixture.txt::test_two",
                "tests/cli/tg1_fixture.txt::test_three",
                "tests/cli/tg1_fixture.txt::test_four"
            ],
            "minimum_count": 4,
            "workers": 8,
            "program": "/bin/sh",
            "args": shard_command_args()
        },
        "suites": [
            {
                "suite_id": "tg1_required",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": false,
                "runner": {
                    "kind": "server_tg1_required",
                    "minimum_count": 4
                }
            }
        ]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(
        value["suite_pool"]["scheduled_suites"][0]["cpu_tokens"],
        json!(4)
    );
    let tg1 = value["suite_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|suite| suite["suite_id"] == json!("tg1_required"))
        .expect("tg1 suite should run");
    assert_eq!(tg1["runner_kind"], json!("rust_server_tg1_required"));
    assert_eq!(
        tg1["tg1_required_summary"]["scheduler"]["requested_cpu_tokens"],
        json!(8)
    );
    assert_eq!(
        tg1["tg1_required_summary"]["scheduler"]["admitted_cpu_tokens"],
        json!(4)
    );
    assert_eq!(
        tg1["tg1_required_summary"]["thread_pool_shards"]["shard_count"],
        json!(4)
    );
    assert_eq!(
        tg1["tg1_required_summary"]["lifecycle"]["finish_report_count"],
        json!(1)
    );
    assert_eq!(
        tg1["tg1_required_summary"]["cleanup"]["status"],
        json!("cleaned")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn server_tg1_required_uses_ait_test_static_contract_when_inventory_nodes_are_empty() {
    let root = temp_root("tg1-static-contract-fallback");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let source_repo = root.join("ait-test");
    fs::create_dir_all(&source_repo).expect("source repo should be created");
    write_static_ait_test_tg1_contract(&source_repo);

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "tg1": {
            "source_repo_root": source_repo.to_string_lossy(),
            "repo_name": "ait-test",
            "node_ids": [],
            "program": "/bin/sh",
            "args": native_runner_env_command_args()
        },
        "suites": [
            {
                "suite_id": "tg1_required",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "server_tg1_required",
                    "repo_name": "ait-test",
                    "test_group_id": "TG-1"
                }
            }
        ]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    let tg1 = &value["suite_results"][0];
    assert_eq!(tg1["runner_kind"], json!("rust_server_tg1_required"));
    assert_eq!(
        tg1["tg1_required_summary"]["membership_source"],
        json!("ait_test_static_descriptor")
    );
    assert_eq!(tg1["tg1_required_summary"]["live_count"], json!(33));
    assert_eq!(tg1["tg1_required_summary"]["minimum_count"], json!(33));
    assert_eq!(
        tg1["tg1_required_summary"]["thread_pool_shards"]["shard_count"],
        json!(10)
    );
    let runner_env = &tg1["tg1_required_summary"]["runner"]["details"]["env"];
    assert_eq!(runner_env["PYTHONPATH"], JsonValue::Null);
    assert_eq!(runner_env["AIT_TG1_NATIVE_RUNNER"], json!("1"));
    assert_eq!(runner_env["AIT_TG1_RUNNER_AUTHORITY"], json!("rust"));
    assert_eq!(runner_env["CARGO_BUILD_JOBS"], json!("1"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn tg1_patchset_flow_runs_all_suites_after_one_prewarm_with_fixed_ten_tokens_and_shared_cargo_cache(
) {
    let root = temp_root("tg1-patchset-flow");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let shared_cargo_target_dir = root.join(".ait/cargo-target");
    let shared_cargo_build_dir = root.join(".ait/cargo-build");
    let source_repo = root.join("source-repo");
    let source_cli_dir = source_repo.join("cli");
    fs::create_dir_all(&source_cli_dir).expect("source repo test dir should be created");
    fs::write(source_cli_dir.join("tg1_fixture.txt"), "tg1 fixture\n")
        .expect("source TG1 fixture should be written");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "shared_cargo_target_dir": shared_cargo_target_dir.to_string_lossy(),
        "shared_cargo_build_dir": shared_cargo_build_dir.to_string_lossy(),
        "patchset_ci_flow": {
            "kind": "tg1_patchset_ci",
            "suite_selection": {"include_modes": ["gate"]},
            "prewarm": {"policy": "once_per_run", "required": true},
            "parallelism": {"policy": "fixed", "cpu_tokens": 10, "require_exact": true},
            "runner_authority": {"rust_only": true},
            "finish": {"policy": "aggregate_after_all_suites"},
            "cargo": {"shared_target_required": true}
        },
        "materialized_files": [],
        "prewarm_commands": [
            "test \"$CARGO_TARGET_DIR\" = \"$AIT_SHARED_CARGO_TARGET_DIR\" && test \"$CARGO_BUILD_BUILD_DIR\" = \"$AIT_SHARED_CARGO_BUILD_DIR\" && printf warm > .ait-prewarm-marker"
        ],
        "tg1": {
            "source_repo_root": source_repo.to_string_lossy(),
            "node_ids": [
                "tests/cli/tg1_fixture.txt::test_one",
                "tests/cli/tg1_fixture.txt::test_two",
                "tests/cli/tg1_fixture.txt::test_three"
            ],
            "minimum_count": 3,
            "requested_cpu_tokens": 10,
            "program": "/bin/sh",
            "args": shard_command_args()
        },
        "suites": [
            {
                "suite_id": "stable_smoke",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["printf stable > stable-smoke.txt"]
                }
            },
            {
                "suite_id": "tg1_required",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "server_tg1_required",
                    "minimum_count": 3,
                    "requested_cpu_tokens": 10
                }
            }
        ]
    }));

    assert_eq!(value["execution_profile"], json!("tg1_patchset_ci"));
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["native_prewarm"]["status"], json!("pass"));
    assert_eq!(value["native_prewarm"]["command_count"], json!(1));
    assert_eq!(value["suite_pool"]["max_cpu_tokens"], json!(10));
    assert_eq!(
        value["suite_pool"]["finish_policy"],
        json!("aggregate_after_all_suites")
    );
    assert_eq!(value["suite_pool"]["finish_report_count"], json!(1));
    assert_eq!(
        value["patchset_ci_detail"]["selected_suite_ids"],
        json!(["stable_smoke", "tg1_required"])
    );
    assert_eq!(
        value["patchset_ci_detail"]["flow"]["contract"],
        json!("ait.server.patchset_ci.tg1_flow.v1")
    );
    assert_eq!(
        value["patchset_ci_detail"]["flow"]["parallelism"]["cpu_tokens"],
        json!(10)
    );
    assert_eq!(
        value["patchset_ci_detail"]["flow"]["cargo"]["shared_cargo_target_dir"],
        json!(shared_cargo_target_dir.to_string_lossy().to_string())
    );
    assert_eq!(
        value["patchset_ci_detail"]["flow"]["cargo"]["shared_cargo_build_dir"],
        json!(shared_cargo_build_dir.to_string_lossy().to_string())
    );
    assert_eq!(
        value["patchset_ci_detail"]["flow_finish"]["all_selected_suites_completed"],
        json!(true)
    );
    assert_eq!(
        value["patchset_ci_detail"]["flow_finish"]["finish_report_count"],
        json!(1)
    );
    let tg1 = value["suite_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|suite| suite["suite_id"] == json!("tg1_required"))
        .expect("tg1 suite should run");
    assert_eq!(
        tg1["tg1_required_summary"]["scheduler"]["requested_cpu_tokens"],
        json!(10)
    );
    assert_eq!(
        tg1["tg1_required_summary"]["scheduler"]["admitted_cpu_tokens"],
        json!(10)
    );
    assert_eq!(
        tg1["tg1_required_summary"]["thread_pool_shards"]["shard_count"],
        json!(10)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn tg1_patchset_flow_can_be_enabled_from_suite_manifest_json() {
    let root = temp_root("tg1-suite-manifest-flow");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let shared_cargo_target_dir = root.join(".ait/cargo-target");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "shared_cargo_target_dir": shared_cargo_target_dir.to_string_lossy(),
        "prewarm_commands": ["test \"$CARGO_TARGET_DIR\" = \"$AIT_SHARED_CARGO_TARGET_DIR\""],
        "materialized_files": [],
        "suites": [
            {
                "suite_id": "stable_smoke",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "patchset_ci_flow": {
                    "kind": "tg1_patchset_ci",
                    "suite_selection": {"include_modes": ["gate"]},
                    "prewarm": {"required": true},
                    "parallelism": {"cpu_tokens": 10, "require_exact": true},
                    "runner_authority": {"rust_only": true},
                    "finish": {"policy": "aggregate_after_all_suites"},
                    "cargo": {"shared_target_required": true}
                },
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["printf stable > stable-smoke.txt"]
                }
            }
        ]
    }));

    assert_eq!(value["execution_profile"], json!("tg1_patchset_ci"));
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["suite_pool"]["max_cpu_tokens"], json!(10));
    assert_eq!(
        value["patchset_ci_detail"]["flow"]["kind"],
        json!("tg1_patchset_ci")
    );
    assert_eq!(
        value["patchset_ci_detail"]["flow_finish"]["all_selected_suites_completed"],
        json!(true)
    );

    let _ = fs::remove_dir_all(root);
}
