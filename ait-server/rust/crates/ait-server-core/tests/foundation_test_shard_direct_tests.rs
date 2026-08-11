use ait_server_core::foundation::ci_runtime_json::TestShardPlanJson;
use ait_server_core::foundation::test_shards::ci_test_shard_plan_json;
use serde_json::json;
use std::env;
use std::fs;
use std::path::PathBuf;

fn temp_root(name: &str) -> PathBuf {
    let root = env::temp_dir().join(format!(
        "ait-server-test-shard-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp root should be created");
    root
}

#[test]
fn test_shard_plan_json_wrapper_preserves_plan_shape_and_error_text() {
    let root = temp_root("wrapper-plan");
    let seed_root = root.join("lyravo-main-seeds");
    let ram_root = root.join("AIT_RAM-test-shards");
    fs::create_dir_all(seed_root.join("ait").join("main-seed"))
        .expect("main seed should be present");
    let json = TestShardPlanJson::stateless();

    let plan = json
        .plan(&json!({
            "job_id": "job-full",
            "job_type": "repo.ci",
            "main_seed_root": seed_root.to_string_lossy(),
            "ram_shard_root": ram_root.to_string_lossy(),
            "admitted_cpu_tokens": 2,
            "test_items": ["test-a", "test-b", "test-c", "test-d"],
            "payload": {
                "repo_name": "ait",
                "suite_ids": ["full_repo"],
                "plane": "nightly",
                "target_line": "main",
                "snapshot_id": "SNP-FULL"
            }
        }))
        .expect("wrapper plan should build");
    assert_eq!(plan["contract"], json!("ait.server.ci_test_shards.v1"));
    assert_eq!(plan["thread_pool_shards"]["shard_count"], json!(2));
    assert_eq!(
        plan["execution"]["runner_parallelism_source"],
        json!("scheduler_admitted_cpu_tokens")
    );
    assert_eq!(
        plan["cleanup"]["strategy"],
        json!("single_final_dirty_cleanup")
    );

    let error = json
        .plan(&json!({
            "job_type": "content.gc",
            "payload": {"repo_name": "ait"}
        }))
        .expect_err("unsupported job type should fail");
    assert_eq!(
        error,
        "`job_type` must be patchset.ci or repo.ci for test shard planning, got content.gc."
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_full_test_uses_main_seed_and_one_ram_shard_per_cpu_token() {
    let root = temp_root("repo-full-seed");
    let seed_root = root.join("lyravo-main-seeds");
    let ram_root = root.join("AIT_RAM-test-shards");
    fs::create_dir_all(seed_root.join("ait").join("main-seed"))
        .expect("main seed should be present");

    let test_items = (0..30)
        .map(|index| format!("test-{index:02}"))
        .collect::<Vec<_>>();

    let plan = ci_test_shard_plan_json(&json!({
        "job_id": "job-full",
        "job_type": "repo.ci",
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": ram_root.to_string_lossy(),
        "admitted_cpu_tokens": 3,
        "test_items": test_items,
        "payload": {
            "repo_name": "ait",
            "suite_ids": ["full_repo"],
            "plane": "nightly",
            "target_line": "main",
            "snapshot_id": "SNP-FULL"
        }
    }))
    .expect("repo full test shard plan should build");

    assert_eq!(plan["contract"], json!("ait.server.ci_test_shards.v1"));
    assert_eq!(plan["full_test"], json!(true));
    assert_eq!(plan["main_seed"]["available"], json!(true));
    assert_eq!(plan["main_seed"]["storage_class"], json!("lyravo_ssd"));
    assert_eq!(plan["main_seed"]["prewarmed_environment"], json!(true));
    assert_eq!(
        plan["main_seed"]["lifecycle"]["cleanup_when_idle"],
        json!(true)
    );
    assert_eq!(
        plan["thread_pool_shards"]["storage_class"],
        json!("AIT_RAM")
    );
    assert_eq!(
        plan["thread_pool_shards"]["pool_id"],
        json!("repo-ci-nightly-main-full_repo")
    );
    assert_eq!(plan["thread_pool_shards"]["shard_count"], json!(3));
    assert_eq!(
        plan["execution"]["one_shard_dir_per_cpu_token"],
        json!(true)
    );
    assert_eq!(
        plan["execution"]["input_output_partitioned_by_shard"],
        json!(true)
    );

    let shards = plan["thread_pool_shards"]["shards"]
        .as_array()
        .expect("shards should be an array");
    assert_eq!(shards.len(), 3);
    for (index, shard) in shards.iter().enumerate() {
        assert_eq!(shard["shard_id"], json!(format!("shard-{index}")));
        assert_eq!(shard["storage_class"], json!("AIT_RAM"));
        assert_eq!(
            shard["materialization"]["source"],
            json!("immutable_main_seed")
        );
        assert_eq!(
            shard["materialization"]["method"],
            json!("platform_adaptive_overlay_or_copy_up")
        );
        assert_eq!(
            shard["materialization"]["copy_entire_main_seed"],
            json!(false)
        );
        assert_eq!(
            shard["materialization"]["whole_seed_directory_symlink"],
            json!(false)
        );
        assert_eq!(
            shard["materialization"]["writable_layer_required"],
            json!(true)
        );
        assert_eq!(shard["input"]["test_count"], json!(10));
        assert_eq!(shard["output"]["partitioned_by_shard"], json!(true));
        assert_eq!(shard["cleanup"]["when"], json!("core_token_reclaimed"));
        assert_eq!(shard["lifecycle"]["cleanup_on_core_reclaim"], json!(true));
        assert!(shard["path"]
            .as_str()
            .expect("shard path should be text")
            .contains("AIT_RAM-test-shards/ait/thread-pool-shards"));
    }
    assert_eq!(shards[0]["input"]["test_items"][0], json!("test-00"));
    assert_eq!(shards[0]["input"]["test_items"][9], json!("test-09"));
    assert_eq!(shards[1]["input"]["test_items"][0], json!("test-10"));
    assert_eq!(shards[2]["input"]["test_items"][9], json!("test-29"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn tg1_defaults_to_ten_shards_when_tokens_are_not_explicit() {
    let root = temp_root("tg1-default");
    let seed_root = root.join("lyravo-main-seeds");
    let ram_root = root.join("AIT_RAM-test-shards");
    fs::create_dir_all(seed_root.join("ait").join("main-seed"))
        .expect("main seed should be present");
    let test_items = (0..24)
        .map(|index| format!("tests/tg1_fixture::test_{index:02}"))
        .collect::<Vec<_>>();

    let plan = ci_test_shard_plan_json(&json!({
        "job_id": "job-tg1",
        "job_type": "patchset.ci",
        "host_cpu_cores": 10,
        "scheduler_posture": "local_co_resident",
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": ram_root.to_string_lossy(),
        "test_items": test_items,
        "payload": {
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-TG1"
        }
    }))
    .expect("TG1 shard plan should build");

    assert_eq!(plan["full_test"], json!(false));
    assert_eq!(
        plan["thread_pool_shards"]["pool_id"],
        json!("patchset-ci-tg1_required")
    );
    assert_eq!(plan["thread_pool_shards"]["admitted_cpu_tokens"], json!(10));
    assert_eq!(plan["thread_pool_shards"]["shard_count"], json!(10));
    let shards = plan["thread_pool_shards"]["shards"]
        .as_array()
        .expect("shards should be an array");
    assert_eq!(shards.len(), 10);
    assert!(shards[..4]
        .iter()
        .all(|shard| shard["input"]["test_count"] == json!(3)));
    assert!(shards[4..]
        .iter()
        .all(|shard| shard["input"]["test_count"] == json!(2)));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn full_repo_defaults_to_ten_dedicated_server_full_test_tokens() {
    let root = temp_root("full-ten-default");
    let seed_root = root.join("lyravo-main-seeds");
    let ram_root = root.join("AIT_RAM-test-shards");
    fs::create_dir_all(seed_root.join("ait").join("main-seed"))
        .expect("main seed should be present");
    let test_items = (0..30)
        .map(|index| format!("test-{index:02}"))
        .collect::<Vec<_>>();

    let plan = ci_test_shard_plan_json(&json!({
        "job_id": "job-full-dynamic",
        "job_type": "repo.ci",
        "host_cpu_cores": 32,
        "scheduler_posture": "dedicated_server",
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": ram_root.to_string_lossy(),
        "test_items": test_items,
        "payload": {
            "repo_name": "ait",
            "suite_ids": ["full_repo"],
            "plane": "nightly",
            "target_line": "main",
            "snapshot_id": "SNP-FULL"
        }
    }))
    .expect("full repo shard plan should build");

    assert_eq!(plan["full_test"], json!(true));
    assert_eq!(plan["thread_pool_shards"]["admitted_cpu_tokens"], json!(10));
    assert_eq!(plan["thread_pool_shards"]["shard_count"], json!(10));
    assert_eq!(
        plan["thread_pool_shards"]["shards"][0]["input"]["test_count"],
        json!(3)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn patchset_ci_without_seed_bootstraps_fixed_main_seed_then_shards() {
    let root = temp_root("patchset-no-seed");
    let seed_root = root.join("lyravo-main-seeds");
    let ram_root = root.join("AIT_RAM-test-shards");
    fs::create_dir_all(&seed_root).expect("seed root should exist without repo seed");

    let plan = ci_test_shard_plan_json(&json!({
        "job_id": "job-ci",
        "job_type": "patchset.ci",
        "main_seed_root": seed_root.to_string_lossy(),
        "ram_shard_root": ram_root.to_string_lossy(),
        "admitted_cpu_tokens": 1,
        "shard_ids": ["core-7"],
        "test_count": 7,
        "payload": {
            "repo_name": "ait-core",
            "patchset_id": "RP-1",
            "suite_id": "unit",
            "revision_snapshot_id": "SNP-1"
        }
    }))
    .expect("patchset CI shard plan should build");

    assert_eq!(plan["full_test"], json!(false));
    assert_eq!(plan["main_seed"]["available"], json!(false));
    assert_eq!(
        plan["thread_pool_shards"]["pool_id"],
        json!("patchset-ci-unit")
    );
    assert!(
        !plan["thread_pool_shards"]["root"]
            .as_str()
            .expect("shard root should be text")
            .contains("SNP-1"),
        "stable shard pool root must not include the patchset revision snapshot"
    );
    assert!(
        !plan["thread_pool_shards"]["root"]
            .as_str()
            .expect("shard root should be text")
            .contains("RP-1"),
        "stable shard pool root must not include the patchset id"
    );
    assert_eq!(
        plan["main_seed"]["action"],
        json!("bootstrap_fixed_main_seed")
    );
    assert_eq!(
        plan["materialization"]["source"],
        json!("bootstrap_fixed_main_seed_then_writable_shards")
    );
    assert_eq!(plan["thread_pool_shards"]["shard_count"], json!(1));
    assert_eq!(
        plan["thread_pool_shards"]["shards"][0]["shard_id"],
        json!("core-7")
    );
    assert_eq!(
        plan["thread_pool_shards"]["shards"][0]["materialization"]
            ["bootstrap_main_seed_if_missing"],
        json!(true)
    );
    assert_eq!(
        plan["thread_pool_shards"]["shards"][0]["input"]["test_index_range"],
        json!({"start": 0, "end_exclusive": 7})
    );
    assert_eq!(
        plan["thread_pool_shards"]["shards"][0]["cleanup"]["strategy"],
        json!("single_final_dirty_cleanup")
    );

    let _ = fs::remove_dir_all(root);
}
