use ait_server_core::foundation::ci_runtime_json::TestShardPlanJson;
use ait_server_core::foundation::test_shard_runtime::{
    ci_test_shard_cleanup_json, ci_test_shard_prepare_json,
};
use serde_json::json;
use std::env;
use std::fs;
use std::path::PathBuf;

fn temp_root(name: &str) -> PathBuf {
    let root = env::temp_dir().join(format!(
        "ait-server-test-shard-runtime-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp root should be created");
    root
}

fn request(root: &std::path::Path) -> serde_json::Value {
    let test_items = (0..30)
        .map(|index| format!("test-{index:02}"))
        .collect::<Vec<_>>();
    json!({
        "job_id": "job-full",
        "job_type": "repo.ci",
        "platform": "macos",
        "main_seed_root": root.join("lyravo-main-seeds").to_string_lossy(),
        "ram_shard_root": root.join("AIT_RAM-test-shards").to_string_lossy(),
        "admitted_cpu_tokens": 3,
        "copy_up_paths": ["src/a.rs"],
        "immutable_link_paths": [".git/objects"],
        "test_items": test_items,
        "payload": {
            "repo_name": "ait",
            "suite_ids": ["full_repo"],
            "plane": "nightly",
            "target_line": "main",
            "snapshot_id": "SNP-FULL"
        }
    })
}

#[test]
fn test_shard_plan_json_wrapper_preserves_prepare_cleanup_and_guard_shapes() {
    let root = temp_root("wrapper-prepare-cleanup");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(seed.join("src")).expect("seed src should be created");
    fs::write(seed.join("src").join("a.rs"), "fn base() {}\n").expect("seed file should exist");
    let payload = request(&root);
    let json = TestShardPlanJson::stateless();

    let prepared = json
        .prepare(&payload)
        .expect("wrapper prepare should succeed");
    assert_eq!(
        prepared["contract"],
        json!("ait.server.ci_test_shard_runtime.v1")
    );
    assert_eq!(prepared["operation"], json!("prepare"));
    assert_eq!(prepared["main_seed_prewarm"], json!(null));
    assert_eq!(
        prepared["cleanup_contract"]["strategy"],
        json!("single_final_dirty_cleanup")
    );

    let mut early_cleanup = payload.clone();
    early_cleanup["cleanup_reason"] = json!("all_assigned_tests_complete");
    early_cleanup["all_shards_completed"] = json!(false);
    early_cleanup["outputs_merged"] = json!(true);
    assert_eq!(
        json.cleanup(&early_cleanup)
            .expect_err("early cleanup should be guarded"),
        "`all_shards_completed` must be true before normal dirty cleanup."
    );

    let mut cleanup = payload;
    cleanup["cleanup_reason"] = json!("all_assigned_tests_complete");
    cleanup["all_shards_completed"] = json!(true);
    cleanup["outputs_merged"] = json!(true);
    let cleaned = json
        .cleanup(&cleanup)
        .expect("wrapper cleanup should succeed");
    assert_eq!(cleaned["operation"], json!("cleanup"));
    assert_eq!(cleaned["main_seed"]["preserved"], json!(true));
    assert_eq!(cleaned["dirty_cleanup"]["write_main_seed"], json!(false));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn macos_prepare_copy_up_splits_tests_and_keeps_seed_immutable() {
    let root = temp_root("macos-copy-up");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(seed.join("src")).expect("seed src should be created");
    fs::create_dir_all(seed.join(".git").join("objects")).expect("objects should be created");
    fs::write(seed.join("src").join("a.rs"), "fn base() {}\n").expect("seed file should exist");
    fs::write(seed.join(".git").join("objects").join("keep"), "object\n")
        .expect("object marker should exist");

    let prepared = ci_test_shard_prepare_json(&request(&root)).expect("prepare should succeed");

    assert_eq!(
        prepared["contract"],
        json!("ait.server.ci_test_shard_runtime.v1")
    );
    assert_eq!(prepared["strategy"], json!("apfs_clone_or_sparse_copy_up"));
    assert_eq!(
        prepared["seed_write_policy"]["whole_seed_directory_symlink"],
        json!(false)
    );
    assert_eq!(prepared["thread_pool_shards"]["shard_count"], json!(3));
    let shards = prepared["thread_pool_shards"]["shards"]
        .as_array()
        .expect("prepared shards should be array");
    for (index, shard) in shards.iter().enumerate() {
        assert_eq!(shard["assignment"]["test_count"], json!(10));
        assert_eq!(
            shard["materialization"]["copy_up_semantics"],
            json!("patch_touched_files_are_real_shard_local_files")
        );
        let repo_dir = PathBuf::from(shard["repo_dir"].as_str().expect("repo dir should exist"));
        let copied = repo_dir.join("src").join("a.rs");
        assert!(
            copied.is_file(),
            "copy-up file should exist for shard {index}"
        );
        assert!(
            !fs::symlink_metadata(&copied)
                .expect("copy-up file metadata should load")
                .file_type()
                .is_symlink(),
            "patch touched file must be a real shard-local file"
        );
        let assignment = repo_dir
            .parent()
            .expect("repo should have shard parent")
            .join("input")
            .join("assignment.json");
        assert!(assignment.is_file(), "assignment should be written");
    }

    let first_repo = PathBuf::from(
        shards[0]["repo_dir"]
            .as_str()
            .expect("repo dir should exist"),
    );
    fs::write(
        first_repo.join("src").join("a.rs"),
        "fn patchset_one() {}\n",
    )
    .expect("patchset should write shard local file");
    assert_eq!(
        fs::read_to_string(seed.join("src").join("a.rs")).expect("seed file should read"),
        "fn base() {}\n"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn macos_prepare_without_path_sets_links_seed_files_into_shard_repo() {
    let root = temp_root("macos-implicit-links");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(seed.join("tests")).expect("seed tests should be created");
    fs::write(seed.join("tests").join("repo_ci_smoke.txt"), "smoke\n")
        .expect("seed fixture should exist");

    let mut payload = request(&root);
    payload
        .as_object_mut()
        .expect("payload should be object")
        .remove("copy_up_paths");
    payload
        .as_object_mut()
        .expect("payload should be object")
        .remove("immutable_link_paths");

    let prepared = ci_test_shard_prepare_json(&payload).expect("prepare should succeed");
    let shard = &prepared["thread_pool_shards"]["shards"][0];
    assert_eq!(
        shard["materialization"]["implicit_immutable_seed_links"],
        json!(true)
    );
    let repo_dir = PathBuf::from(shard["repo_dir"].as_str().expect("repo dir should exist"));
    let linked = repo_dir.join("tests").join("repo_ci_smoke.txt");
    assert!(
        linked.is_file(),
        "implicit seed file link should exist in shard repo"
    );
    assert!(
        fs::symlink_metadata(&linked)
            .expect("linked test metadata should load")
            .file_type()
            .is_symlink(),
        "implicit seed files should be file-level symlinks, not copied files"
    );
    assert_eq!(
        fs::read_to_string(&linked).expect("linked file should read"),
        "smoke\n"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn macos_prepare_with_copy_up_paths_keeps_unmodified_seed_files_linked() {
    let root = temp_root("macos-copy-up-with-implicit-links");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(seed.join("src")).expect("seed src should be created");
    fs::create_dir_all(seed.join("rust")).expect("seed rust should be created");
    fs::write(seed.join("src").join("a.rs"), "fn base() {}\n").expect("seed file should exist");
    fs::write(seed.join("rust").join("Cargo.toml"), "[workspace]\n")
        .expect("manifest should exist");

    let mut payload = request(&root);
    payload
        .as_object_mut()
        .expect("payload should be object")
        .remove("immutable_link_paths");

    let prepared = ci_test_shard_prepare_json(&payload).expect("prepare should succeed");
    let shard = &prepared["thread_pool_shards"]["shards"][0];
    assert_eq!(
        shard["materialization"]["implicit_immutable_seed_links"],
        json!(true)
    );
    let repo_dir = PathBuf::from(shard["repo_dir"].as_str().expect("repo dir should exist"));
    let copied = repo_dir.join("src").join("a.rs");
    assert!(copied.is_file(), "copy-up file should exist");
    assert!(
        !fs::symlink_metadata(&copied)
            .expect("copy-up metadata should load")
            .file_type()
            .is_symlink(),
        "copy-up file must stay shard-local"
    );
    let linked_manifest = repo_dir.join("rust").join("Cargo.toml");
    assert!(
        linked_manifest.is_file(),
        "unmodified Cargo manifest should remain visible in shard repo"
    );
    assert!(
        fs::symlink_metadata(&linked_manifest)
            .expect("manifest metadata should load")
            .file_type()
            .is_symlink(),
        "unmodified seed files should be immutable links"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn macos_prepare_retry_resets_stale_shard_directory_before_linking_seed_files() {
    let root = temp_root("macos-retry-reset");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(seed.join("src")).expect("seed src should be created");
    fs::write(seed.join("src").join("a.rs"), "fn base() {}\n").expect("seed file should exist");
    fs::write(seed.join(".ait-github-publish-ignore"), "target\n")
        .expect("seed root file should exist");

    let mut payload = request(&root);
    payload
        .as_object_mut()
        .expect("payload should be object")
        .remove("immutable_link_paths");

    let first = ci_test_shard_prepare_json(&payload).expect("first prepare should succeed");
    let first_shard = &first["thread_pool_shards"]["shards"][0];
    let shard_path = PathBuf::from(
        first_shard["path"]
            .as_str()
            .expect("shard path should exist"),
    );
    let repo_dir = PathBuf::from(
        first_shard["repo_dir"]
            .as_str()
            .expect("repo dir should exist"),
    );
    let linked_ignore = repo_dir.join(".ait-github-publish-ignore");
    assert!(
        fs::symlink_metadata(&linked_ignore)
            .expect("linked ignore metadata should load")
            .file_type()
            .is_symlink(),
        "root seed file should be linked after first prepare"
    );
    fs::create_dir_all(repo_dir.join("target")).expect("dirty target dir should be created");
    fs::write(repo_dir.join("target").join("dirty.o"), "dirty\n")
        .expect("dirty target file should be created");
    fs::write(repo_dir.join("src").join("a.rs"), "fn dirty() {}\n")
        .expect("copy-up file should be dirtyable");

    let second = ci_test_shard_prepare_json(&payload).expect("retry prepare should succeed");
    let second_shard = &second["thread_pool_shards"]["shards"][0];
    assert_eq!(
        second_shard["prepare_reset"]["stale_shard_removed_before_prepare"],
        json!(true)
    );
    assert!(shard_path.exists(), "shard path should be recreated");
    assert!(
        !repo_dir.join("target").join("dirty.o").exists(),
        "retry prepare should remove stale generated artifacts"
    );
    assert_eq!(
        fs::read_to_string(repo_dir.join("src").join("a.rs")).expect("copy-up should read"),
        "fn base() {}\n"
    );
    assert!(
        fs::symlink_metadata(&linked_ignore)
            .expect("linked ignore metadata should load after retry")
            .file_type()
            .is_symlink(),
        "retry prepare should recreate immutable links without File exists"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn linux_prepare_dry_run_uses_overlayfs_without_copying_seed() {
    let root = temp_root("linux-overlay");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(&seed).expect("seed should be created");
    let mut payload = request(&root);
    payload["platform"] = json!("linux");
    payload["dry_run"] = json!(true);
    payload["copy_up_paths"] = json!([]);
    payload["immutable_link_paths"] = json!([]);

    let prepared = ci_test_shard_prepare_json(&payload).expect("prepare dry run should succeed");

    assert_eq!(prepared["strategy"], json!("overlayfs"));
    assert_eq!(prepared["materialization"]["selected"], json!("overlayfs"));
    let first = &prepared["thread_pool_shards"]["shards"][0];
    assert_eq!(first["materialization"]["strategy"], json!("overlayfs"));
    assert_eq!(
        first["materialization"]["copy_up_semantics"],
        json!("kernel_overlayfs_copy_up")
    );
    assert_eq!(
        first["materialization"]["whole_seed_directory_symlink"],
        json!(false)
    );
    assert_eq!(
        first["materialization"]["copy_entire_main_seed"],
        json!(false)
    );
    assert_eq!(first["materialization"]["mount_command"][0], json!("mount"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cleanup_refuses_early_normal_cleanup_and_removes_dirty_shards_after_merge() {
    let root = temp_root("cleanup");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(seed.join("src")).expect("seed src should be created");
    fs::write(seed.join("src").join("a.rs"), "fn base() {}\n").expect("seed file should exist");
    let payload = request(&root);
    let prepared = ci_test_shard_prepare_json(&payload).expect("prepare should succeed");
    let shards = prepared["thread_pool_shards"]["shards"]
        .as_array()
        .expect("prepared shards should be array");
    for shard in shards {
        let repo_dir = PathBuf::from(shard["repo_dir"].as_str().expect("repo dir should exist"));
        fs::create_dir_all(repo_dir.join("target")).expect("target should be created");
        fs::write(repo_dir.join("target").join("dirty.o"), "dirty\n")
            .expect("dirty file should be created");
    }

    let mut early_cleanup = payload.clone();
    early_cleanup["cleanup_reason"] = json!("all_assigned_tests_complete");
    early_cleanup["all_shards_completed"] = json!(false);
    early_cleanup["outputs_merged"] = json!(true);
    let error = ci_test_shard_cleanup_json(&early_cleanup).expect_err("cleanup should be guarded");
    assert!(error.contains("all_shards_completed"));

    let mut cleanup = payload;
    cleanup["cleanup_reason"] = json!("all_assigned_tests_complete");
    cleanup["all_shards_completed"] = json!(true);
    cleanup["outputs_merged"] = json!(true);
    let cleaned = ci_test_shard_cleanup_json(&cleanup).expect("cleanup should succeed");

    assert_eq!(cleaned["operation"], json!("cleanup"));
    assert_eq!(cleaned["main_seed"]["preserved"], json!(true));
    for shard in cleaned["thread_pool_shards"]["shards"]
        .as_array()
        .expect("cleaned shards should be array")
    {
        let path = PathBuf::from(shard["path"].as_str().expect("shard path should exist"));
        assert!(!path.exists(), "shard should be removed");
    }
    assert_eq!(
        fs::read_to_string(seed.join("src").join("a.rs")).expect("seed should read"),
        "fn base() {}\n"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cleanup_can_preserve_repo_dir_and_next_prepare_reuses_warm_shard_layout() {
    let root = temp_root("cleanup-preserve-repo");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(seed.join("src")).expect("seed src should be created");
    fs::create_dir_all(seed.join("rust")).expect("seed rust dir should be created");
    fs::write(seed.join("src").join("a.rs"), "fn base() {}\n").expect("seed file should exist");
    fs::write(seed.join("rust").join("Cargo.toml"), "[workspace]\n")
        .expect("workspace manifest should exist");

    let mut payload = request(&root);
    payload["preserve_repo_dir"] = json!(true);
    payload
        .as_object_mut()
        .expect("payload should be object")
        .remove("immutable_link_paths");

    let prepared = ci_test_shard_prepare_json(&payload).expect("prepare should succeed");
    let shard = &prepared["thread_pool_shards"]["shards"][0];
    let shard_path = PathBuf::from(shard["path"].as_str().expect("shard path should exist"));
    let repo_dir = PathBuf::from(shard["repo_dir"].as_str().expect("repo dir should exist"));
    fs::create_dir_all(repo_dir.join("target")).expect("target dir should be created");
    fs::write(repo_dir.join("target").join("dirty.o"), "dirty\n")
        .expect("dirty target file should be created");
    fs::write(repo_dir.join("src").join("a.rs"), "fn dirty() {}\n")
        .expect("copy-up file should be dirtyable");
    fs::write(
        repo_dir.join("src").join("generated.rs"),
        "pub fn generated() {}\n",
    )
    .expect("extra generated file should be created");

    let mut cleanup = payload.clone();
    cleanup["cleanup_reason"] = json!("all_assigned_tests_complete");
    cleanup["all_shards_completed"] = json!(true);
    cleanup["outputs_merged"] = json!(true);
    let cleaned = ci_test_shard_cleanup_json(&cleanup).expect("cleanup should succeed");

    assert_eq!(
        cleaned["strategy"],
        json!("preserve_repo_dir_restore_main_seed")
    );
    assert!(shard_path.exists(), "shard path should stay allocated");
    assert!(repo_dir.exists(), "repo dir should be preserved");
    assert_eq!(
        cleaned["thread_pool_shards"]["shards"][0]["repo_dir_exists_after_cleanup"],
        json!(true)
    );
    assert!(
        !repo_dir.join("target").join("dirty.o").exists(),
        "generated target output should be removed"
    );
    assert!(
        !repo_dir.join("src").join("generated.rs").exists(),
        "files outside main-seed should be pruned"
    );
    assert_eq!(
        cleaned["thread_pool_shards"]["shards"][0]["preserved_cleanup"]["preserved_copy_up_paths"],
        json!(["src/a.rs"])
    );
    let preserved = repo_dir.join("src").join("a.rs");
    assert_eq!(
        fs::read_to_string(&preserved).expect("preserved file should read"),
        "fn dirty() {}\n"
    );
    assert!(
        !fs::symlink_metadata(&preserved)
            .expect("preserved metadata should load")
            .file_type()
            .is_symlink(),
        "copy-up file should stay shard-local until the revision overlay updates it"
    );

    let prepared_again =
        ci_test_shard_prepare_json(&payload).expect("warm prepare reuse should succeed");
    let prepare_reset = &prepared_again["thread_pool_shards"]["shards"][0]["prepare_reset"];
    assert_eq!(
        prepare_reset["stale_shard_removed_before_prepare"],
        json!(false)
    );
    assert_eq!(
        prepare_reset["preserved_repo_dir_between_runs"],
        json!(true)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn core_token_reclaim_cleans_single_shard_without_output_merge_gate() {
    let root = temp_root("core-reclaim");
    let seed = root.join("lyravo-main-seeds").join("ait").join("main-seed");
    fs::create_dir_all(seed.join("src")).expect("seed src should be created");
    fs::write(seed.join("src").join("a.rs"), "fn base() {}\n").expect("seed file should exist");
    let mut payload = request(&root);
    payload["admitted_cpu_tokens"] = json!(1);
    payload["shard_ids"] = json!(["core-2"]);
    payload["test_count"] = json!(1);
    payload
        .as_object_mut()
        .expect("payload should be object")
        .remove("test_items");
    let prepared = ci_test_shard_prepare_json(&payload).expect("prepare should succeed");
    let shard_path = PathBuf::from(
        prepared["thread_pool_shards"]["shards"][0]["path"]
            .as_str()
            .expect("shard path should exist"),
    );
    assert!(shard_path.exists());

    let mut cleanup = payload;
    cleanup["cleanup_reason"] = json!("core_token_reclaimed");
    let cleaned = ci_test_shard_cleanup_json(&cleanup).expect("reclaim cleanup should succeed");

    assert_eq!(cleaned["cleanup_reason"], json!("core_token_reclaimed"));
    assert!(!shard_path.exists(), "reclaimed shard should be removed");
    assert!(seed.exists(), "main seed should not be removed");

    let _ = fs::remove_dir_all(root);
}
