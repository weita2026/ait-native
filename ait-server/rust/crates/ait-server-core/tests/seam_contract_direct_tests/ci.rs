#[test]
fn ci_test_shard_plan_command_covers_full_repo_ram_shards_and_seed_lifecycle() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-seam-ci-shard-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let seed_root = root.join("lyravo-main-seeds");
    let ram_root = root.join("AIT_RAM-shards");
    fs::create_dir_all(seed_root.join("ait").join("main-seed"))
        .expect("seed path should be created");

    let payload = json!({
        "job_id": "job-full",
        "job_type": "repo.ci",
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": ram_root.to_string_lossy(),
        "admitted_cpu_tokens": 3,
        "test_count": 30,
        "payload": {
            "repo_name": "ait",
            "suite_ids": ["full_repo"],
            "plane": "nightly",
            "target_line": "main",
            "snapshot_id": "SNP-FULL"
        }
    })
    .to_string();

    let value = stdout_json(&run_seam(&["ci-test-shard-plan", &payload]));
    assert_eq!(value["contract"], json!("ait.server.ci_test_shards.v1"));
    assert_eq!(value["full_test"], json!(true));
    assert_eq!(value["main_seed"]["storage_class"], json!("lyravo_ssd"));
    assert_eq!(value["main_seed"]["available"], json!(true));
    assert_eq!(
        value["thread_pool_shards"]["storage_class"],
        json!("AIT_RAM")
    );
    assert_eq!(value["thread_pool_shards"]["shard_count"], json!(3));
    assert_eq!(value["execution"]["runner_parallelism"], json!(3));
    assert_eq!(
        value["thread_pool_shards"]["shards"][0]["materialization"]["source"],
        json!("immutable_main_seed")
    );
    assert_eq!(
        value["thread_pool_shards"]["shards"][0]["materialization"]["whole_seed_directory_symlink"],
        json!(false)
    );
    assert_eq!(
        value["thread_pool_shards"]["shards"][0]["input"]["test_index_range"],
        json!({"start": 0, "end_exclusive": 10})
    );
    assert_eq!(
        value["thread_pool_shards"]["shards"][0]["cleanup"]["when"],
        json!("core_token_reclaimed")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_main_seed_prewarm_command_runs_rust_owned_generation_gate() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-seam-main-seed-prewarm-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let source = root.join("source");
    let seed = root.join("seeds").join("ait").join("main-seed");
    fs::create_dir_all(source.join("src")).expect("source should be created");
    fs::write(source.join("src").join("lib.rs"), "pub fn base() {}\n")
        .expect("source file should be written");

    let payload = json!({
        "repo_name": "ait",
        "main_seed_path": seed.to_string_lossy(),
        "source_repo_path": source.to_string_lossy(),
        "generation_key": "SNP-SEAM",
        "parallelism": 2,
        "required_paths": ["src/lib.rs", "warm.txt"],
        "prewarm_steps": [{
            "step_id": "marker",
            "program": "/bin/sh",
            "args": ["-c", "printf warm > warm.txt"],
            "required_paths": ["warm.txt"]
        }]
    })
    .to_string();

    let first = stdout_json(&run_seam(&["ci-main-seed-prewarm", &payload]));
    assert_eq!(first["contract"], json!("ait.server.main_seed_prewarm.v1"));
    assert_eq!(first["status"], json!("prewarmed"));
    assert_eq!(first["parallelism"], json!(2));
    assert!(seed.join("warm.txt").exists());

    let second = stdout_json(&run_seam(&["ci-main-seed-prewarm", &payload]));
    assert_eq!(second["status"], json!("reused"));
    assert_eq!(second["steps"], json!([]));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_test_shard_prepare_and_cleanup_commands_materialize_and_clear_shards() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-seam-ci-shard-runtime-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let seed_root = root.join("lyravo-main-seeds");
    let seed = seed_root.join("ait").join("main-seed");
    fs::create_dir_all(seed.join("src")).expect("seed src should be created");
    fs::write(seed.join("src").join("a.rs"), "fn base() {}\n").expect("seed file should exist");

    let payload_value = json!({
        "job_id": "job-ci",
        "job_type": "patchset.ci",
        "platform": "macos",
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": root.join("AIT_RAM-shards").to_string_lossy(),
        "admitted_cpu_tokens": 1,
        "test_count": 2,
        "copy_up_paths": ["src/a.rs"],
        "payload": {
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "unit",
            "revision_snapshot_id": "SNP-1"
        }
    });
    let payload = payload_value.to_string();

    let prepared = stdout_json(&run_seam(&["ci-test-shard-prepare", &payload]));
    assert_eq!(
        prepared["contract"],
        json!("ait.server.ci_test_shard_runtime.v1")
    );
    assert_eq!(
        prepared["thread_pool_shards"]["shards"][0]["assignment"]["test_index_range"],
        json!({"start": 0, "end_exclusive": 2})
    );
    let repo_dir = PathBuf::from(
        prepared["thread_pool_shards"]["shards"][0]["repo_dir"]
            .as_str()
            .expect("repo dir should be text"),
    );
    assert!(repo_dir.join("src").join("a.rs").is_file());
    fs::write(repo_dir.join("src").join("a.rs"), "fn patched() {}\n")
        .expect("shard should be writable");
    assert_eq!(
        fs::read_to_string(seed.join("src").join("a.rs")).expect("seed should read"),
        "fn base() {}\n"
    );

    let cleanup_payload = {
        let mut value = payload_value;
        value["cleanup_reason"] = json!("all_assigned_tests_complete");
        value["all_shards_completed"] = json!(true);
        value["outputs_merged"] = json!(true);
        value.to_string()
    };
    let cleaned = stdout_json(&run_seam(&["ci-test-shard-cleanup", &cleanup_payload]));
    assert_eq!(cleaned["operation"], json!("cleanup"));
    assert_eq!(cleaned["main_seed"]["preserved"], json!(true));
    assert!(!repo_dir.exists());
    assert_eq!(
        fs::read_to_string(seed.join("src").join("a.rs")).expect("seed should still read"),
        "fn base() {}\n"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_test_shard_run_command_executes_and_cleans_rust_owned_shards() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-seam-ci-shard-run-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let seed_root = root.join("lyravo-main-seeds");
    let seed = seed_root.join("ait").join("main-seed");
    fs::create_dir_all(&seed).expect("seed should be created");

    let payload = json!({
        "job_id": "job-tg1",
        "job_type": "patchset.ci",
        "platform": "macos",
        "materialization_strategy": "sparse_copy_up",
        "host_cpu_cores": 10,
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": root.join("AIT_RAM-shards").to_string_lossy(),
        "merged_output_dir": root.join("merged").to_string_lossy(),
        "copy_up_paths": [],
        "immutable_link_paths": [],
        "test_items": [
            "tests/tg1_fixture::test_0",
            "tests/tg1_fixture::test_1",
            "tests/tg1_fixture::test_2",
            "tests/tg1_fixture::test_3",
            "tests/tg1_fixture::test_4",
            "tests/tg1_fixture::test_5"
        ],
        "payload": {
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-1"
        },
        "runner": {
            "kind": "command",
            "program": "/bin/sh",
            "args": ["-c", "printf '%s\\n' \"$AIT_SHARD_ID\" > \"$AIT_SHARD_OUTPUT_DIR/shard.txt\""],
            "append_test_items": false
        }
    })
    .to_string();

    let result = stdout_json(&run_seam(&["ci-test-shard-run", &payload]));

    assert_eq!(result["contract"], json!("ait.server.ci_test_shard_run.v1"));
    assert_eq!(result["status"], json!("pass"));
    assert_eq!(result["execution"]["runner_parallelism"], json!(10));
    assert_eq!(
        result["cleanup"]["cleanup_reason"],
        json!("all_assigned_tests_complete")
    );
    assert!(PathBuf::from(
        result["artifacts"]["summary_json"]["path"]
            .as_str()
            .expect("summary path should be text")
    )
    .is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_command_bundle_run_command_executes_rust_owned_bundle() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-seam-ci-command-bundle-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    fs::create_dir_all(&workspace).expect("workspace should be created");

    let payload = json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "runner": {
            "kind": "command_bundle",
            "prewarm_commands": ["printf warm > prewarm-marker.txt"],
            "commands": ["test -f prewarm-marker.txt && printf seam > seam-marker.txt"]
        }
    })
    .to_string();

    let value = stdout_json(&run_seam(&["ci-command-bundle-run", &payload]));

    assert_eq!(
        value["contract"],
        json!("ait.server.ci_command_bundle_run.v1")
    );
    assert_eq!(value["status"], json!("pass"));
    assert_eq!(value["prewarm"]["reports"].as_array().unwrap().len(), 1);
    assert!(PathBuf::from(value["artifacts"]["log_path"]["path"].as_str().unwrap()).is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_run_command_executes_rust_runtime_from_stdin() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-seam-patchset-ci-run-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let payload = json!({
        "patchset": {
            "patchset_id": "RCP-SEAM",
            "change_id": "RCC-SEAM",
            "base_snapshot_id": "SNP-BASE",
            "revision_snapshot_id": "SNP-REV",
            "ci_run_seq": 1
        },
        "change": {
            "repo_name": "ait",
            "change_id": "RCC-SEAM"
        },
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn seam() {}\n"}
        ],
        "prewarm_commands": ["printf warm > prewarm-marker.txt"],
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": ["test -f src/lib.rs && test \"$(tail -c 1 src/lib.rs | od -An -tx1 | tr -d '[:space:]')\" = 0a && test -f prewarm-marker.txt"]
            }
        }]
    })
    .to_string();

    let value = stdout_json(&run_seam_with_stdin(&["patchset-ci-run", "-"], &payload));

    assert_eq!(value["contract"], json!("ait.server.patchset_ci.run.v1"));
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["patchset_ci_completion"]["ci_run_seq"], json!(1));
    assert!(value["patchset_ci_completion"].get("job_state").is_none());
    assert_eq!(
        value["suite_results"][0]["runner_kind"],
        json!("rust_server_ci")
    );
    assert_eq!(
        value["server_ci_gate"]["python_server_ci_executor"],
        json!(false)
    );
    assert!(!workspace.exists());
    assert!(PathBuf::from(
        value["suite_results"][0]["artifacts"]["log_path"]["path"]
            .as_str()
            .unwrap()
    )
    .is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_host_status_summary_command_shapes_tg1_payload() {
    let payload = json!({
        "patchset_id": "RP-1",
        "change_id": "RC-1",
        "repo_name": "ait",
        "ci_completed_at_s": 1_783_814_400_u64,
        "embedded_patchset_ci": {
            "run_seq": 1,
            "completed_at_s": 1_783_814_400_u64,
            "overall_status": "pass",
            "tests_status": "pass",
            "selected_suite_count": 2,
            "suite_result_count": 2,
            "blocking_failure_count": 0
        },
        "jobs": [{
            "job_id": 11,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "payload": {"patchset_id": "RP-1"},
            "result": {
                "tests_status": "pass",
                "selected_suite_ids": ["preflight", "tg1_required"],
                "suite_results": [{
                    "suite_id": "tg1_required",
                    "status": "pass",
                    "tg1_required_summary": {
                        "status": "pass",
                        "live_count": 24,
                        "minimum_count": 24
                    }
                }]
            }
        }],
        "recent_limit": 5
    })
    .to_string();

    let value = stdout_json(&run_seam_with_stdin(
        &["patchset-ci-host", "status-summary", "-"],
        &payload,
    ));

    assert_eq!(value["patchset_id"], json!("RP-1"));
    assert_eq!(value["latest_job"]["job_id"], json!(11));
    assert_eq!(value["tg1_required"]["live_count"], json!(24));
    assert_eq!(value["rerun"]["cli"], json!("ait patchset rerun-ci RP-1"));
}

#[test]
fn patchset_ci_host_suite_catalog_ignores_non_patch_ci_catalog_from_stdin() {
    let payload = json!({
        "snapshot_files": [
            {
                "path": "ci/not_patch_ci.json",
                "content": r#"{
                    "suite_id": "preflight",
                    "plane": "patchset",
                    "mode": "gate",
                    "default_blocking": true,
                    "runner": {"kind": "command_bundle", "commands": ["cargo test"]}
                }"#
            }
        ]
    })
    .to_string();

    let value = stdout_json(&run_seam_with_stdin(
        &["patchset-ci-host", "suite-catalog", "-"],
        &payload,
    ));

    assert_eq!(
        value["contract"],
        json!("ait.server.patchset_ci.suite_catalog.v1")
    );
    assert_eq!(value["suite_count"], json!(0));
    assert_eq!(value["catalog_paths"], json!([]));
    assert_eq!(value["suites"], json!([]));
}

#[test]
fn repo_ci_run_command_executes_rust_runtime_from_stdin() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-seam-repo-ci-run-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let payload = json!({
        "repo_name": "ait",
        "snapshot_id": "SNP-REPO",
        "target_line": "main",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn repo_ci() {}\n"}
        ],
        "prewarm_commands": ["printf warm > repo-prewarm.txt"],
        "suites": [{
            "suite_id": "nightly_smoke",
            "plane": "nightly",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": ["test -f src/lib.rs && test \"$(tail -c 1 src/lib.rs | od -An -tx1 | tr -d '[:space:]')\" = 0a && test -f repo-prewarm.txt"]
            }
        }]
    })
    .to_string();

    let value = stdout_json(&run_seam_with_stdin(&["repo-ci-run", "-"], &payload));

    assert_eq!(value["contract"], json!("ait.server.repo_ci.run.v1"));
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(
        value["suite_results"][0]["runner_kind"],
        json!("rust_repo_ci")
    );
    assert_eq!(
        value["server_ci_gate"]["python_server_ci_executor"],
        json!(false)
    );
    assert!(!workspace.exists());
    assert!(PathBuf::from(
        value["suite_results"][0]["artifacts"]["log_path"]["path"]
            .as_str()
            .unwrap()
    )
    .is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_workflow_ready_evidence_command_requires_tg1_server_evidence() {
    let value = stdout_json(&run_seam(&[
        "patchset-ci-workflow-ready-evidence",
        r#"{"manifests":[{"suite_id":"preflight","plane":"patchset","mode":"gate","default_blocking":true},{"suite_id":"tg1_required","plane":"patchset","mode":"gate","default_blocking":false,"runner":{"kind":"server_tg1_required"}}],"suite_evidence":{"preflight":{"runner_kind":"rust_server_ci","status":"pass"},"tg1_required":{"runner_kind":"rust_server_tg1_required","status":"pass","tg1_required_summary":{"status":"pass","validation_status":"pass","live_count":33,"minimum_count":33,"scheduler":{"authority":"server_scheduler","thread_pool_owner":"server","requested_cpu_tokens":10,"admitted_cpu_tokens":10,"runner_parallelism_source":"scheduler_admitted_cpu_tokens"},"lifecycle":{"init_policy":"once_per_run","prewarm_policy":"main_seed_once_per_run","prewarm_once":true,"finish_policy":"once_per_run","finish_report_count":1,"cleanup_policy":"all_tests_reclaimed_no_dirty","cleanup_status":"cleaned"},"thread_pool_shards":{"shard_count":10,"shards":[]},"cleanup":{"status":"cleaned","policy":"all_tests_reclaimed_no_dirty"}}}}}"#,
    ]));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(
        value["selected_suite_ids"],
        json!(["preflight", "tg1_required"])
    );
    assert_eq!(
        value["server_ci_gate"]["python_server_ci_executor"],
        json!(false)
    );
    assert_eq!(value["server_ci_gate"]["tg1_required"], json!(true));
}
