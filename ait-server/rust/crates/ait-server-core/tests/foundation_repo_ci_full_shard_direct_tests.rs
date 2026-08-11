use ait_server_core::foundation::repo_ci_runtime::repo_ci_run_json;
use serde_json::json;
use std::env;
use std::fs;
use std::path::PathBuf;

fn temp_root(name: &str) -> PathBuf {
    let root = env::temp_dir().join(format!(
        "ait-server-repo-ci-full-shard-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp root should be created");
    root
}

fn full_test_items() -> Vec<String> {
    (0..6).map(|index| format!("full-case-{index}")).collect()
}

fn collected_full_test_items() -> Vec<String> {
    (0..4)
        .map(|index| format!("collected-case-{index}"))
        .collect()
}

fn full_shard_args() -> Vec<String> {
    vec![
        "-c".to_string(),
        r#"if [ "${AIT_SHARD_REPO_DIR}" = "" ]; then
  echo "AIT_SHARD_REPO_DIR missing" >&2
  exit 1
fi
if printf '%s\n' "$AIT_TEST_ITEMS_JSON" | grep -q 'failing-case'; then
  echo "explicit failing shard item" >&2
  exit 1
fi
printf '%s\n' 'explicit shard runner pass'"#
            .to_string(),
    ]
}

fn full_repo_payload(root: &PathBuf) -> serde_json::Value {
    let workspace = root.join("workspace");
    let output = root.join("output");
    json!({
        "repo_name": "ait",
        "snapshot_id": "SNP-FULL-SHARD",
        "target_line": "main",
        "plane": "nightly",
        "suite_ids": ["full_repo"],
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output.to_string_lossy(),
        "shared_cargo_target_dir": root.join(".ait/cargo-target").to_string_lossy(),
        "shared_cargo_build_dir": root.join(".ait/cargo-build").to_string_lossy(),
        "cleanup_workspace": true,
        "platform": "macos",
        "materialization_strategy": "sparse_copy_up",
        "main_seed_root": root.join("lyravo-main-seeds").to_string_lossy(),
        "ram_shard_root": root.join("AIT_RAM-shards").to_string_lossy(),
        "admitted_cpu_tokens": 3,
        "materialized_files": [
            {"path": "tests/full_suite_fixture.txt", "content": "full-suite-fixture\n"}
        ],
        "env": {
            "AIT_REPO_ROOT": root.join("leaky-repo-root").to_string_lossy(),
            "AIT_NATIVE_WORKSPACE_ROOT": root.join("leaky-native-workspace-root").to_string_lossy(),
            "AIT_WORKSPACE_ROOT": root.join("leaky-workspace-root").to_string_lossy(),
            "CARGO_TARGET_DIR": root.join("leaky-cargo-target").to_string_lossy(),
            "AIT_SHARED_CARGO_TARGET_DIR": root.join("leaky-shared-cargo-target").to_string_lossy(),
            "CARGO_BUILD_BUILD_DIR": root.join("leaky-cargo-build").to_string_lossy(),
            "AIT_SHARED_CARGO_BUILD_DIR": root.join("leaky-shared-cargo-build").to_string_lossy(),
            "AIT_EXTERNAL_CORE_REPO_ROOT": root.join("external-core").to_string_lossy(),
            "AIT_EXTERNAL_SERVER_CORE_REPO_ROOT": root.join("external-server").to_string_lossy()
        },
        "prewarm_commands": ["printf warm >> prewarm-marker.txt"],
        "suites": [{
            "suite_id": "full_repo",
            "display_name": "Full Repo",
            "plane": "nightly",
            "mode": "diagnostic",
            "default_blocking": false,
            "runner": {
                "kind": "test_shard",
                "program": "/bin/sh",
                "args": full_shard_args(),
                "test_items": full_test_items(),
                "allow_fail_fast": false,
                "collect_complete_failure_set": true,
                "env": {
                    "CARGO_TARGET_DIR": root.join("runner-leaky-cargo-target").to_string_lossy(),
                    "AIT_SHARED_CARGO_TARGET_DIR": root.join("runner-leaky-shared-cargo-target").to_string_lossy(),
                    "CARGO_BUILD_BUILD_DIR": root.join("runner-leaky-cargo-build").to_string_lossy(),
                    "AIT_SHARED_CARGO_BUILD_DIR": root.join("runner-leaky-shared-cargo-build").to_string_lossy()
                }
            },
            "main_seed_prewarm": {
                "required_paths": ["tests/full_suite_fixture.txt"]
            },
            "artifacts": {
                "log_path": "full_repo.log"
            }
        }]
    })
}

fn full_repo_zstd_only_payload(root: &PathBuf) -> serde_json::Value {
    let mut payload = full_repo_payload(root);
    payload["suite_ids"] = json!(["full_repo_zstd_only"]);
    payload["suites"][0]["suite_id"] = json!("full_repo_zstd_only");
    payload["suites"][0]["display_name"] = json!("Full Repo Zstd Only");
    payload["suites"][0]["runner"]["env"] = json!({
        "AIT_TEST_PACK_STORAGE_POLICY": "zstd_only",
        "AIT_TEST_REMOTE_SYNC_TRANSPORT": "zstd_bulk",
        "AIT_TEST_DISABLE_SNAPSHOT_ZIP_TRANSPORT": "1"
    });
    payload
}

#[test]
fn repo_ci_full_repo_runs_through_main_seed_shards_and_reuses_prewarm() {
    let root = temp_root("reuse");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    let payload = full_repo_payload(&root);

    let first = repo_ci_run_json(&payload).expect("first full repo run should pass");
    assert_eq!(first["contract"], json!("ait.server.repo_ci.run.v1"));
    assert_eq!(first["tests_status"], json!("pass"));
    assert_eq!(first["suite_status"], json!("pass"));
    assert_eq!(first["suite_failures"], json!([]));
    assert_eq!(first["selected_suite_ids"], json!(["full_repo"]));
    assert_eq!(first["native_prewarm"]["status"], json!("pass"));
    assert_eq!(
        first["native_prewarm"]["main_seed_status"],
        json!("prewarmed")
    );
    assert_eq!(first["native_prewarm"]["command_count"], json!(1));

    let suite = &first["suite_results"][0];
    assert_eq!(suite["suite_id"], json!("full_repo"));
    assert_eq!(suite["runner_kind"], json!("rust_repo_full_test_shards"));
    assert_eq!(
        suite["server_ci_gate"]["python_server_ci_executor"],
        json!(false)
    );
    assert_eq!(suite["thread_pool_shards"]["shard_count"], json!(3));
    assert_eq!(suite["execution"]["runner_parallelism"], json!(3));
    assert_eq!(suite["shard_run"]["runner"]["program"], json!("/bin/sh"));
    assert_eq!(
        suite["shard_run"]["runner"]["args"],
        json!(full_shard_args())
    );
    assert_eq!(
        suite["shard_run"]["runner"]["env"]["CARGO_TARGET_DIR"],
        json!(root.join(".ait/cargo-target").to_string_lossy())
    );
    assert_eq!(
        suite["shard_run"]["runner"]["env"]["AIT_SHARED_CARGO_TARGET_DIR"],
        json!(root.join(".ait/cargo-target").to_string_lossy())
    );
    assert_eq!(
        suite["shard_run"]["runner"]["env"]["CARGO_BUILD_BUILD_DIR"],
        json!(root.join(".ait/cargo-build").to_string_lossy())
    );
    assert_eq!(
        suite["shard_run"]["runner"]["env"]["AIT_SHARED_CARGO_BUILD_DIR"],
        json!(root.join(".ait/cargo-build").to_string_lossy())
    );
    assert!(suite["shard_run"]["runner"]["env"]["PYTHONPATH"].is_null());
    assert_eq!(suite["main_seed_prewarm"]["status"], json!("prewarmed"));
    assert_eq!(
        suite["main_seed_prewarm"]["required_paths"][0]["relative_path"],
        json!("tests/full_suite_fixture.txt")
    );
    assert_eq!(
        suite["main_seed_prewarm"]["required_paths"][0]["exists"],
        json!(true)
    );
    for shard in suite["thread_pool_shards"]["shards"].as_array().unwrap() {
        assert_eq!(shard["status"], json!("pass"));
        let stdout = shard["stdout"].as_str().unwrap_or_default();
        assert!(stdout.contains("explicit shard runner pass"), "{stdout}");
    }
    assert_eq!(
        suite["cleanup"]["cleanup_reason"],
        json!("all_assigned_tests_complete")
    );
    assert_eq!(
        fs::read_to_string(seed.join("prewarm-marker.txt")).expect("seed marker should exist"),
        "warm"
    );
    let prewarm_manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(seed.join(".ait/main-seed-prewarm.json"))
            .expect("prewarm manifest should exist"),
    )
    .expect("prewarm manifest should be valid JSON");
    let prewarm_fingerprint: serde_json::Value =
        serde_json::from_str(prewarm_manifest["fingerprint"].as_str().unwrap())
            .expect("prewarm fingerprint should be valid JSON");
    assert_eq!(
        prewarm_fingerprint["steps"][0]["env"]["AIT_REPO_ROOT"],
        json!(".")
    );
    assert_eq!(
        prewarm_fingerprint["steps"][0]["env"]["AIT_SHARED_CARGO_TARGET_DIR"],
        json!(".ait/cargo-target")
    );
    assert_eq!(
        prewarm_fingerprint["steps"][0]["env"]["CARGO_BUILD_BUILD_DIR"],
        json!(".ait/cargo-build")
    );
    assert_eq!(
        prewarm_fingerprint["steps"][0]["env"]["AIT_SHARED_CARGO_BUILD_DIR"],
        json!(".ait/cargo-build")
    );
    assert_eq!(
        prewarm_fingerprint["steps"][0]["env"]["AIT_EXTERNAL_CORE_REPO_ROOT"],
        json!(root.join("external-core").to_string_lossy())
    );
    assert_eq!(
        prewarm_fingerprint["steps"][0]["env"]["AIT_EXTERNAL_SERVER_CORE_REPO_ROOT"],
        json!(root.join("external-server").to_string_lossy())
    );

    let second = repo_ci_run_json(&payload).expect("second full repo run should reuse seed");
    assert_eq!(
        second["native_prewarm"]["main_seed_status"],
        json!("reused")
    );
    assert_eq!(
        second["suite_results"][0]["main_seed_prewarm"]["status"],
        json!("reused")
    );
    assert_eq!(
        fs::read_to_string(seed.join("prewarm-marker.txt")).expect("seed marker should remain"),
        "warm",
        "same-generation full-test runs must not rerun prewarm commands"
    );
    assert!(!root.join("workspace").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_full_repo_zstd_only_runs_through_full_test_shards() {
    let root = temp_root("zstd-only");
    let payload = full_repo_zstd_only_payload(&root);

    let result = repo_ci_run_json(&payload).expect("zstd-only full repo run should pass");
    assert_eq!(result["tests_status"], json!("pass"));
    assert_eq!(result["suite_status"], json!("pass"));
    assert_eq!(result["selected_suite_ids"], json!(["full_repo_zstd_only"]));

    let suite = &result["suite_results"][0];
    assert_eq!(suite["suite_id"], json!("full_repo_zstd_only"));
    assert_eq!(suite["runner_kind"], json!("rust_repo_full_test_shards"));
    assert_eq!(suite["thread_pool_shards"]["shard_count"], json!(3));
    assert_eq!(
        suite["shard_run"]["runner"]["env"]["AIT_TEST_PACK_STORAGE_POLICY"],
        json!("zstd_only")
    );
    assert_eq!(
        suite["shard_run"]["runner"]["env"]["AIT_TEST_REMOTE_SYNC_TRANSPORT"],
        json!("zstd_bulk")
    );
    assert_eq!(
        suite["shard_run"]["runner"]["env"]["AIT_TEST_DISABLE_SNAPSHOT_ZIP_TRANSPORT"],
        json!("1")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_full_repo_keeps_explicit_test_items_path_when_collection_source_is_declared() {
    let root = temp_root("explicit-items-win");
    let mut payload = full_repo_payload(&root);
    payload["snapshot_id"] = json!("SNP-FULL-SHARD-EXPLICIT-WINS");
    payload["suites"][0]["runner"]["test_items_source"] = json!("server_collect_once_artifact");
    payload["suites"][0]["collection"] = json!({
        "test_items_source": "server_collect_once_artifact",
        "collect_once_before_sharding": true,
        "command": "printf should-not-run >&2; exit 23",
        "output_path": "test-items.json",
        "output_format": "json_array"
    });

    let value =
        repo_ci_run_json(&payload).expect("explicit test_items must preserve existing full path");

    assert_eq!(value["tests_status"], json!("pass"));
    let suite = &value["suite_results"][0];
    assert_eq!(suite["status"], json!("pass"));
    assert!(
        suite["test_collection"].is_null(),
        "server collection must not run when explicit runner.test_items are present"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_full_repo_collects_test_items_once_before_sharding_when_declared() {
    let root = temp_root("collect-once");
    let mut payload = full_repo_payload(&root);
    payload["snapshot_id"] = json!("SNP-FULL-SHARD-COLLECT-ONCE");
    payload["suites"][0]["runner"]
        .as_object_mut()
        .expect("runner should be an object")
        .remove("test_items");
    payload["suites"][0]["runner"]["test_items_source"] = json!("server_collect_once_artifact");
    payload["suites"][0]["collection"] = json!({
        "test_items_source": "server_collect_once_artifact",
        "collect_once_before_sharding": true,
        "command": "test -f tests/full_suite_fixture.txt && test \"$CUSTOM_LANGUAGE_HOME\" = /workspace/custom && test \"$AIT_REPO_ROOT\" != /spoofed && test \"$CARGO_TARGET_DIR\" = \"$EXPECTED_CARGO_TARGET_DIR\" && test \"$AIT_SHARED_CARGO_TARGET_DIR\" = \"$EXPECTED_CARGO_TARGET_DIR\" && test \"$CARGO_BUILD_BUILD_DIR\" = \"$EXPECTED_CARGO_BUILD_DIR\" && test \"$AIT_SHARED_CARGO_BUILD_DIR\" = \"$EXPECTED_CARGO_BUILD_DIR\" && printf '%s\\n' '[\"collected-case-0\",\"collected-case-1\",\"collected-case-2\",\"collected-case-3\"]' > \"$AIT_TEST_COLLECTION_OUTPUT_PATH\"",
        "output_path": "collected-test-items.json",
        "output_format": "json_array",
        "timeout_seconds": 7,
        "env": {
            "CUSTOM_LANGUAGE_HOME": "/workspace/custom",
            "AIT_REPO_ROOT": "/spoofed",
            "CARGO_TARGET_DIR": root.join("collection-leaky-cargo-target").to_string_lossy(),
            "AIT_SHARED_CARGO_TARGET_DIR": root.join("collection-leaky-shared-cargo-target").to_string_lossy(),
            "CARGO_BUILD_BUILD_DIR": root.join("collection-leaky-cargo-build").to_string_lossy(),
            "AIT_SHARED_CARGO_BUILD_DIR": root.join("collection-leaky-shared-cargo-build").to_string_lossy(),
            "EXPECTED_CARGO_TARGET_DIR": root.join(".ait/cargo-target").to_string_lossy(),
            "EXPECTED_CARGO_BUILD_DIR": root.join(".ait/cargo-build").to_string_lossy()
        }
    });

    let value = repo_ci_run_json(&payload).expect("server collect-once full repo run should pass");

    assert_eq!(value["tests_status"], json!("pass"));
    let suite = &value["suite_results"][0];
    assert_eq!(suite["status"], json!("pass"));
    assert_eq!(
        suite["test_collection"]["source"],
        json!("server_collect_once_artifact")
    );
    assert_eq!(suite["test_collection"]["test_count"], json!(4));
    assert_eq!(suite["test_collection"]["timeout_seconds"], json!(7));
    assert_eq!(suite["test_collection"]["timed_out"], json!(false));
    assert_eq!(
        suite["test_collection"]["process_environment"]["policy"],
        json!("safe_ambient_allowlist_with_explicit_overrides")
    );
    assert_eq!(
        suite["test_collection"]["test_items"],
        json!(collected_full_test_items())
    );
    assert_eq!(
        suite["test_collection"]["collect_once_before_sharding"],
        json!(true)
    );
    let collection_artifact = PathBuf::from(
        suite["test_collection"]["artifacts"]["test_items_json"]["path"]
            .as_str()
            .expect("collection artifact path should be present"),
    );
    assert!(
        collection_artifact.is_file(),
        "collection artifact should be retained"
    );

    let mut observed = Vec::new();
    for shard in suite["thread_pool_shards"]["shards"]
        .as_array()
        .expect("shards should be an array")
    {
        observed.extend(
            shard["test_items"]
                .as_array()
                .expect("shard should report assigned test items")
                .iter()
                .map(|value| value.as_str().unwrap().to_string()),
        );
    }
    assert_eq!(observed, collected_full_test_items());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_full_test_collection_reports_timeout_and_retains_streamed_log() {
    let root = temp_root("collection-timeout");
    let mut payload = full_repo_payload(&root);
    payload["snapshot_id"] = json!("SNP-FULL-SHARD-COLLECTION-TIMEOUT");
    payload["suites"][0]["runner"]
        .as_object_mut()
        .expect("runner should be an object")
        .remove("test_items");
    payload["suites"][0]["runner"]["test_items_source"] = json!("server_collect_once_artifact");
    payload["suites"][0]["runner"]["timeout_seconds"] = json!(9);
    payload["suites"][0]["collection"] = json!({
        "test_items_source": "server_collect_once_artifact",
        "collect_once_before_sharding": true,
        "command": "printf collection-started; sleep 30",
        "output_path": "collected-test-items.json",
        "output_format": "json_array",
        "timeout_seconds": 1
    });

    let value = repo_ci_run_json(&payload).expect("collection timeout should report suite failure");

    assert_eq!(value["tests_status"], json!("fail"));
    let suite = &value["suite_results"][0];
    assert_eq!(suite["status"], json!("fail"));
    assert_eq!(suite["failure"]["phase"], json!("test_collection"));
    let message = suite["failure"]["message"]
        .as_str()
        .expect("timeout failure should contain a message");
    assert!(message.contains("timed out after 1 seconds"), "{message}");
    let log_path = root
        .join("output")
        .join("full_repo")
        .join("collection")
        .join("collection.log");
    let log = fs::read_to_string(log_path).expect("streamed collection log should remain");
    assert!(log.contains("timed_out=true"));
    assert!(log.contains("timeout_seconds=1.000"));
    assert!(log.contains("collection-started"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_full_repo_reports_missing_required_main_seed_path_as_suite_failure() {
    let root = temp_root("missing-required-path");
    let mut payload = full_repo_payload(&root);
    payload["snapshot_id"] = json!("SNP-FULL-SHARD-MISSING-REQUIRED-PATH");
    payload["suites"][0]["main_seed_prewarm"] = json!({
        "required_paths": ["docs/LOCAL_DEVELOPMENT.md"]
    });

    let value =
        repo_ci_run_json(&payload).expect("missing required seed path should report suite failure");

    assert_eq!(value["tests_status"], json!("fail"));
    assert_eq!(value["blocking_failures"], json!([]));
    assert_eq!(value["suite_status"], json!("fail"));
    assert_eq!(value["suite_failures"], json!(["full_repo"]));
    assert_eq!(value["native_prewarm"]["status"], json!("fail"));
    assert_eq!(value["native_prewarm"]["main_seed_status"], json!("fail"));

    let suite = &value["suite_results"][0];
    assert_eq!(suite["status"], json!("fail"));
    assert_eq!(suite["failure"]["phase"], json!("main_seed_prewarm"));
    assert!(suite["failure"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("docs/LOCAL_DEVELOPMENT.md"));
    assert_eq!(suite["main_seed_prewarm"]["status"], json!("fail"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_full_repo_reports_nonblocking_suite_failures_with_explicit_tests_status() {
    let root = temp_root("nonblocking-failure");
    let mut payload = full_repo_payload(&root);
    payload["snapshot_id"] = json!("SNP-FULL-SHARD-FAIL");
    payload["suites"][0]["runner"]["test_items"] = json!(["failing-case"]);

    let value = repo_ci_run_json(&payload).expect("non-blocking full repo failure should report");

    assert_eq!(value["tests_status"], json!("fail"));
    assert_eq!(value["blocking_failures"], json!([]));
    assert_eq!(value["suite_status"], json!("fail"));
    assert_eq!(value["suite_failures"], json!(["full_repo"]));
    assert_eq!(value["repo_ci_detail"]["suite_status"], json!("fail"));
    assert_eq!(
        value["repo_ci_detail"]["suite_failures"],
        json!(["full_repo"])
    );
    assert_eq!(value["suite_results"][0]["blocking"], json!(false));
    assert_eq!(value["suite_results"][0]["status"], json!("fail"));

    let _ = fs::remove_dir_all(root);
}
