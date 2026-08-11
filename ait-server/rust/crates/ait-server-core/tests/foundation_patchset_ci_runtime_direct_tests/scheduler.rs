#[test]
fn patchset_ci_run_json_wrapper_preserves_scheduler_suite_and_completion_shapes() {
    let root = temp_root("wrapper-scheduler-suite");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = PatchsetCiRunJson::stateless()
        .run(&json!({
            "patchset": base_patchset(),
            "change": base_change(),
            "workspace_path": workspace.to_string_lossy(),
            "output_dir": output_dir.to_string_lossy(),
            "cleanup_workspace": true,
            "admitted_cpu_tokens": 9,
            "scheduler_admission": {
                "decision": {
                    "kind": "admit",
                    "job_id": "42",
                    "job": {
                        "job_kind": "patchset.ci",
                        "cpu_tokens": 9,
                        "token_pools": ["global_cpu_tokens", "ci_full_shared_cpu_tokens"]
                    }
                },
                "policy": {
                    "host_cpu_cores": 10,
                    "full_test_job_cpu_tokens": 9
                }
            },
            "materialized_files": [
                {"path": "src/lib.rs", "content": "pub fn wrapper_patchset() -> bool { true }\\n"}
            ],
            "suites": [{
                "suite_id": "rust_core",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 9"]
                }
            }]
        }))
        .expect("wrapper patchset CI should run");

    assert_eq!(value["contract"], json!("ait.server.patchset_ci.run.v1"));
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(
        value["scheduler_admission"]["decision"]["kind"],
        json!("admit")
    );
    assert_eq!(value["suite_results"][0]["suite_id"], json!("rust_core"));
    assert_eq!(
        value["patchset_ci_detail"]["scheduler"]["runner_parallelism_source"],
        json!("scheduler_admitted_cpu_tokens")
    );
    assert_eq!(
        value["patchset_ci_detail"]["suite_results"][0]["status"],
        json!("pass")
    );
    assert_eq!(
        value["patchset_ci_completion"]["suite_result_count"],
        json!(1)
    );
    assert!(value.get("attestation_update").is_none());
    assert_eq!(
        value["server_ci_gate"]["rust_patchset_ci_runtime"],
        json!(true)
    );
    assert_eq!(value["cleanup"]["status"], json!("cleaned"));
    assert!(
        !workspace.exists(),
        "wrapper run should preserve cleanup ownership"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_records_scheduler_admission_evidence() {
    let root = temp_root("scheduler-admission-evidence");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "admitted_cpu_tokens": 9,
        "scheduler_admission": {
            "decision": {
                "kind": "admit",
                "job_id": "42",
                "job": {
                    "job_kind": "patchset.ci",
                    "cpu_tokens": 9,
                    "token_pools": ["global_cpu_tokens", "ci_full_shared_cpu_tokens", "full_test_cpu_tokens"]
                }
            },
            "policy": {
                "host_cpu_cores": 10,
                "full_test_job_cpu_tokens": 9
            }
        },
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn scheduled() -> bool { true }\\n"}
        ],
        "suites": [{
            "suite_id": "rust_core",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": ["test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 9"]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["admitted_cpu_tokens"], json!(9));
    assert_eq!(
        value["scheduler_admission"]["decision"]["job"]["cpu_tokens"],
        json!(9)
    );
    assert_eq!(
        value["patchset_ci_detail"]["scheduler"]["runner_parallelism"],
        json!(9)
    );
    assert_eq!(
        value["patchset_ci_detail"]["scheduler_admission"]["decision"]["kind"],
        json!("admit")
    );
    assert_eq!(
        value["patchset_ci_detail"]["scheduler"]["scheduler_admission"]
            ["decision"]["job"]["cpu_tokens"],
        json!(9)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_runs_suites_in_bounded_parallel_after_one_prewarm() {
    let root = temp_root("bounded-suite-pool");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "suite_pool_tokens": 9,
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn changed() -> bool { true }\\n"}
        ],
        "prewarm_commands": [
            "printf warm > .ait-prewarm-marker"
        ],
        "suites": [
            {
                "suite_id": "parallel_a",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": [
                        "test -f .ait-prewarm-marker && touch suite-a-start && i=0 && while [ ! -f suite-b-start ] && [ $i -lt 200 ]; do i=$((i + 1)); sleep 0.02; done; test -f suite-b-start"
                    ]
                }
            },
            {
                "suite_id": "parallel_b",
                "plane": "patchset",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": [
                        "test -f .ait-prewarm-marker && touch suite-b-start && i=0 && while [ ! -f suite-a-start ] && [ $i -lt 200 ]; do i=$((i + 1)); sleep 0.02; done; test -f suite-a-start"
                    ]
                }
            }
        ]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["native_prewarm"]["status"], json!("pass"));
    assert_eq!(value["suite_pool"]["max_cpu_tokens"], json!(9));
    assert_eq!(
        value["suite_pool"]["scheduled_suites"][0]["cpu_tokens"],
        json!(1)
    );
    assert_eq!(
        value["suite_pool"]["scheduled_suites"][1]["cpu_tokens"],
        json!(1)
    );
    assert!(value["suite_results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|suite| suite["status"] == json!("pass")));

    let _ = fs::remove_dir_all(root);
}
