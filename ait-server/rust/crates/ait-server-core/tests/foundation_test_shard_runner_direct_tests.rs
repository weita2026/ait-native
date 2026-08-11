use ait_server_core::foundation::test_shard_runner::ci_test_shard_run_json;
use serde_json::json;
use std::env;
use std::fs;
use std::path::PathBuf;

fn temp_root(name: &str) -> PathBuf {
    let root = env::temp_dir().join(format!(
        "ait-server-test-shard-runner-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp root should be created");
    root
}

#[test]
fn tg1_shard_run_defaults_to_ten_rust_owned_shards_and_cleans_dirty_dirs() {
    let root = temp_root("tg1-ten-shards");
    let seed_root = root.join("lyravo-main-seeds");
    let ram_root = root.join("AIT_RAM-test-shards");
    let merged_output = root.join("merged");
    let seed = seed_root.join("ait").join("main-seed");
    fs::create_dir_all(&seed).expect("seed should be present");
    let test_items = (0..24)
        .map(|index| format!("tests/tg1_fixture::test_{index:02}"))
        .collect::<Vec<_>>();

    let result = ci_test_shard_run_json(&json!({
        "job_id": "job-tg1",
        "job_type": "patchset.ci",
        "host_cpu_cores": 10,
        "scheduler_posture": "local_co_resident",
        "platform": "macos",
        "materialization_strategy": "sparse_copy_up",
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": ram_root.to_string_lossy(),
        "merged_output_dir": merged_output.to_string_lossy(),
        "copy_up_paths": [],
        "immutable_link_paths": [],
        "test_items": test_items,
        "payload": {
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-TG1"
        },
        "runner": {
            "kind": "command",
            "program": "/bin/sh",
            "args": [
                "-c",
                "test \"$CUSTOM_LANGUAGE_HOME\" = /workspace/custom && test \"$AIT_SHARD_ID\" != spoofed && test \"$AIT_REPO_ROOT\" = \"$AIT_SHARD_REPO_DIR\" && printf '%s\\n' \"$AIT_SHARD_ID:$AIT_TEST_ITEMS\" > \"$AIT_SHARD_OUTPUT_DIR/items.txt\""
            ],
            "append_test_items": false,
            "env": {
                "CUSTOM_LANGUAGE_HOME": "/workspace/custom",
                "AIT_SHARD_ID": "spoofed",
                "AIT_REPO_ROOT": "/spoofed"
            }
        },
        "artifacts": {
            "summary_json": "tg1-summary.json",
            "log_path": "tg1.log"
        }
    }))
    .expect("sharded TG1 run should succeed");

    assert_eq!(result["contract"], json!("ait.server.ci_test_shard_run.v1"));
    assert_eq!(result["status"], json!("pass"));
    assert_eq!(
        result["runner"]["process_environment"]["policy"],
        json!("safe_ambient_allowlist_with_explicit_overrides")
    );
    assert_eq!(result["execution"]["runner_parallelism"], json!(10));
    assert_eq!(result["thread_pool_shards"]["shard_count"], json!(10));
    let shards = result["thread_pool_shards"]["shards"]
        .as_array()
        .expect("shards should be present");
    for (index, shard) in shards.iter().enumerate() {
        assert_eq!(shard["status"], json!("pass"));
        assert_eq!(shard["test_count"], json!(if index < 4 { 3 } else { 2 }));
        let shard_repo = PathBuf::from(shard["repo_dir"].as_str().expect("repo dir"));
        assert!(
            !shard_repo.exists(),
            "cleanup should remove per-core dirty shard repo"
        );
    }
    let summary_path = PathBuf::from(
        result["artifacts"]["summary_json"]["path"]
            .as_str()
            .expect("summary path should exist"),
    );
    let log_path = PathBuf::from(
        result["artifacts"]["log_path"]["path"]
            .as_str()
            .expect("log path should exist"),
    );
    assert!(summary_path.is_file());
    assert!(log_path.is_file());
    assert_eq!(result["cleanup"]["operation"], json!("cleanup"));
    assert_eq!(
        result["cleanup"]["cleanup_reason"],
        json!("all_assigned_tests_complete")
    );
    assert!(seed.exists(), "main seed must be preserved");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn shard_runner_streams_large_output_and_reports_timeout() {
    let root = temp_root("large-output-timeout");
    let seed_root = root.join("lyravo-main-seeds");
    let ram_root = root.join("AIT_RAM-test-shards");
    let merged_output = root.join("merged");
    let seed = seed_root.join("ait").join("main-seed");
    fs::create_dir_all(&seed).expect("seed should be present");

    let result = ci_test_shard_run_json(&json!({
        "job_id": "job-timeout",
        "job_type": "repo.ci",
        "host_cpu_cores": 1,
        "admitted_cpu_tokens": 1,
        "scheduler_posture": "local_co_resident",
        "platform": "macos",
        "materialization_strategy": "sparse_copy_up",
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": ram_root.to_string_lossy(),
        "merged_output_dir": merged_output.to_string_lossy(),
        "copy_up_paths": [],
        "immutable_link_paths": [],
        "test_items": ["large-output-case"],
        "cleanup": false,
        "payload": {
            "repo_name": "ait",
            "snapshot_id": "SNP-TIMEOUT",
            "suite_id": "full_repo"
        },
        "runner": {
            "kind": "command",
            "program": "/bin/sh",
            "args": [
                "-c",
                "head -c 1048576 /dev/zero | tr '\\000' x; printf shard-tail; sleep 30"
            ],
            "append_test_items": false,
            "timeout_seconds": 1
        }
    }))
    .expect("shard timeout should return bounded failure evidence");

    assert_eq!(result["status"], json!("fail"));
    assert_eq!(result["runner"]["timeout_seconds"], json!(1));
    let shard = &result["thread_pool_shards"]["shards"][0];
    assert_eq!(shard["status"], json!("fail"));
    assert_eq!(shard["timed_out"], json!(true));
    assert_eq!(shard["timeout_seconds"], json!(1));
    assert_eq!(shard["stdout_bytes"], json!(1_048_586));
    let stdout_tail = shard["stdout"]
        .as_str()
        .expect("bounded stdout tail should be text");
    assert!(stdout_tail.len() <= 8_000);
    assert!(stdout_tail.ends_with("shard-tail"));
    let log_path = PathBuf::from(
        shard["log_path"]
            .as_str()
            .expect("shard log path should be text"),
    );
    assert!(
        fs::metadata(&log_path)
            .expect("shard log should exist")
            .len()
            > 1_048_576
    );
    let log = fs::read_to_string(&log_path).expect("shard log should remain readable");
    assert!(log.contains("timed_out=true"));
    assert!(!log_path.with_extension("stdout.tmp").exists());
    assert!(!log_path.with_extension("stderr.tmp").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn shard_runner_rejects_invalid_timeout_before_materialization() {
    for invalid in [json!(0), json!(-1), json!(86_401), json!("1")] {
        let error = ci_test_shard_run_json(&json!({
            "runner": {
                "kind": "command",
                "program": "/bin/true",
                "timeout_seconds": invalid
            }
        }))
        .expect_err("invalid timeout should fail closed");
        assert!(error.contains("timeout_seconds"), "{error}");
    }
}
