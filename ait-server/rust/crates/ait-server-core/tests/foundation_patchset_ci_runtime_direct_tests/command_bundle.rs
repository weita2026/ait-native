#[test]
fn patchset_ci_runtime_partitions_command_bundle_across_scheduler_shards() {
    let root = temp_root("command-bundle-shards");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let shard_root = root.join("thread-pool-shards");
    let mut shards = Vec::new();
    for index in 0..3 {
        let shard_path = shard_root.join(format!("shard-{index}"));
        let repo_dir = shard_path.join("repo");
        let shard_output_dir = shard_path.join("output");
        fs::create_dir_all(repo_dir.join("src")).expect("shard repo source should be created");
        fs::create_dir_all(&shard_output_dir).expect("shard output should be created");
        fs::write(
            repo_dir.join("src").join("lib.rs"),
            format!("pub fn shard_{index}() -> bool {{ true }}\n"),
        )
        .expect("shard repo source should be written");
        shards.push(json!({
            "shard_id": format!("shard-{index}"),
            "repo_dir": repo_dir.to_string_lossy(),
            "output_dir": shard_output_dir.to_string_lossy()
        }));
    }

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "suite_pool_tokens": 3,
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn primary_workspace() -> bool { true }\\n"}
        ],
        "snapshot_materialization_result": {
            "contract": "ait.server.patchset_ci.snapshot_materialization_result.v1",
            "strategy": "server_main_seed",
            "json_snapshot_payload": false,
            "python_glue": false,
            "thread_pool_shards": {
                "shard_count": 3,
                "shards": shards
            }
        },
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": [
                    "test -f src/lib.rs && test \"$AIT_CI_RUNNER_PARALLELISM\" = 1 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 1 && test \"$CARGO_BUILD_JOBS\" = 1 && test \"$RUST_TEST_THREADS\" = 1 && printf command-1 > command-1.txt",
                    "test -f src/lib.rs && test \"$AIT_CI_RUNNER_PARALLELISM\" = 1 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 1 && test \"$CARGO_BUILD_JOBS\" = 1 && test \"$RUST_TEST_THREADS\" = 1 && printf command-2 > command-2.txt",
                    "test -f src/lib.rs && test \"$AIT_CI_RUNNER_PARALLELISM\" = 1 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 1 && test \"$CARGO_BUILD_JOBS\" = 1 && test \"$RUST_TEST_THREADS\" = 1 && printf command-3 > command-3.txt"
                ]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    let suite = &value["suite_results"][0];
    assert_eq!(suite["runner_parallelism"], json!(3));
    assert_eq!(
        suite["command_bundle_shards"]["distribution"],
        json!("commands_partitioned_across_server_worktrees")
    );
    assert_eq!(suite["command_bundle_shards"]["shard_count"], json!(3));
    assert_eq!(suite["thread_pool_shards"]["shard_count"], json!(3));
    assert_eq!(
        suite["server_ci_gate"]["rust_command_bundle_shards"],
        json!(true)
    );
    let reports = suite["command_reports"]
        .as_array()
        .expect("command reports should be an array");
    assert_eq!(reports.len(), 3);
    for (index, report) in reports.iter().enumerate() {
        assert_eq!(report["index"], json!(index + 1));
        assert_eq!(report["shard_command_index"], json!(1));
        assert_eq!(report["shard_id"], json!(format!("shard-{index}")));
    }
    for index in 0..3 {
        assert!(
            shard_root
                .join(format!("shard-{index}/repo/command-{}", index + 1))
                .with_extension("txt")
                .is_file(),
            "command {} should run in shard-{index} repo",
            index + 1
        );
    }
    let summary_path = PathBuf::from(
        suite["artifacts"]["summary_json"]["path"]
            .as_str()
            .expect("sharded summary path should be text"),
    );
    let log_path = PathBuf::from(
        suite["artifacts"]["log_path"]["path"]
            .as_str()
            .expect("sharded log path should be text"),
    );
    assert!(summary_path.is_file());
    assert!(log_path.is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_runtime_keeps_cargo_command_bundle_on_single_source_identity() {
    let root = temp_root("cargo-command-bundle-single-source");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let shard_root = root.join("thread-pool-shards");
    fs::create_dir_all(workspace.join("src")).expect("workspace source should be created");
    fs::write(
        workspace.join("src").join("lib.rs"),
        "pub fn primary_workspace() -> bool { true }\n",
    )
    .expect("workspace source should be written");

    let mut shards = Vec::new();
    for index in 0..3 {
        let shard_path = shard_root.join(format!("shard-{index}"));
        let repo_dir = shard_path.join("repo");
        let shard_output_dir = shard_path.join("output");
        fs::create_dir_all(repo_dir.join("src")).expect("shard repo source should be created");
        fs::create_dir_all(&shard_output_dir).expect("shard output should be created");
        fs::write(
            repo_dir.join("src").join("lib.rs"),
            format!("pub fn shard_{index}() -> bool {{ true }}\n"),
        )
        .expect("shard repo source should be written");
        shards.push(json!({
            "shard_id": format!("shard-{index}"),
            "repo_dir": repo_dir.to_string_lossy(),
            "output_dir": shard_output_dir.to_string_lossy()
        }));
    }

    let value = run_patchset_ci(json!({
        "patchset": base_patchset(),
        "change": base_change(),
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "suite_pool_tokens": 3,
        "prewarm_commands": [
            ": cargo; test \"$AIT_CI_RUNNER_PARALLELISM\" = 3 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 3 && test \"$CARGO_BUILD_JOBS\" = 3 && test \"$RUST_TEST_THREADS\" = 3 && printf prewarm > prewarm-marker.txt"
        ],
        "main_seed_prewarm": {
            "contract": "ait.server.main_seed_prewarm.v1",
            "status": "reused",
            "generation_key": "SNP-BASE",
            "step_count": 1,
            "parallelism": 3,
            "steps": []
        },
        "snapshot_materialization_result": {
            "contract": "ait.server.patchset_ci.snapshot_materialization_result.v1",
            "strategy": "server_main_seed",
            "json_snapshot_payload": false,
            "python_glue": false,
            "thread_pool_shards": {
                "shard_count": 3,
                "shards": shards
            }
        },
        "suites": [{
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": [
                    ": cargo; test -f src/lib.rs && test \"$AIT_CI_RUNNER_PARALLELISM\" = 3 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 3 && test \"$CARGO_BUILD_JOBS\" = 3 && test \"$RUST_TEST_THREADS\" = 3 && printf command-1 > command-1.txt",
                    ": cargo; test -f src/lib.rs && test \"$AIT_CI_RUNNER_PARALLELISM\" = 3 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 3 && test \"$CARGO_BUILD_JOBS\" = 3 && test \"$RUST_TEST_THREADS\" = 3 && printf command-2 > command-2.txt",
                    ": cargo; test -f src/lib.rs && test \"$AIT_CI_RUNNER_PARALLELISM\" = 3 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 3 && test \"$CARGO_BUILD_JOBS\" = 3 && test \"$RUST_TEST_THREADS\" = 3 && printf command-3 > command-3.txt"
                ]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert!(workspace.join("prewarm-marker.txt").is_file());
    assert!(workspace.join("command-1.txt").is_file());
    assert!(workspace.join("command-2.txt").is_file());
    assert!(workspace.join("command-3.txt").is_file());
    for index in 0..3 {
        assert!(
            !shard_root
                .join(format!("shard-{index}/repo/command-1.txt"))
                .is_file(),
            "cargo command bundle should not run in shard-{index}"
        );
    }
    let suite = &value["suite_results"][0];
    assert_eq!(suite["runner_parallelism"], json!(3));
    assert!(suite.get("command_bundle_shards").is_none());
    assert!(suite.get("thread_pool_shards").is_none());
    assert_eq!(
        suite["server_ci_gate"]["cargo_source_identity_policy"],
        json!("single_workspace_prewarm")
    );
    assert_eq!(suite["server_ci_gate"]["prewarm_parallelism"], json!(3));
    assert_eq!(
        suite["server_ci_gate"]["prewarm_uses_runner_workspace"],
        json!(true)
    );

    let _ = fs::remove_dir_all(root);
}
