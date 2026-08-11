use ait_server_core::foundation::ci_runtime_json::MainSeedPrewarmJson;
use ait_server_core::foundation::main_seed_prewarm::ci_main_seed_prewarm_json;
use ait_server_core::foundation::test_shard_runtime::ci_test_shard_prepare_json;
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
use std::path::PathBuf;

fn temp_root(name: &str) -> PathBuf {
    let root = env::temp_dir().join(format!(
        "ait-server-main-seed-prewarm-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp root should be created");
    root
}

#[test]
fn main_seed_prewarm_json_wrapper_preserves_status_reuse_and_manifest_shapes() {
    let root = temp_root("wrapper-status");
    let source = root.join("source");
    let seed = root.join("seeds").join("ait").join("main-seed");
    fs::create_dir_all(source.join("src")).expect("source should be created");
    fs::write(source.join("src").join("lib.rs"), "pub fn wrapper() {}\n")
        .expect("source file should be written");
    let json = MainSeedPrewarmJson::stateless();
    let request = json!({
        "repo_name": "ait",
        "main_seed_path": seed.to_string_lossy(),
        "source_repo_path": source.to_string_lossy(),
        "generation_key": "SNP-WRAPPER",
        "parallelism": 1,
        "timeout_seconds": 7,
        "required_paths": ["src/lib.rs"]
    });

    let first = json.prewarm(&request).expect("first wrapper prewarm");
    assert_eq!(first["contract"], json!("ait.server.main_seed_prewarm.v1"));
    assert_eq!(first["status"], json!("prewarmed"));
    assert_eq!(first["reused"], json!(false));
    assert_eq!(first["prewarm_once"], json!(true));
    assert_eq!(first["timeout_seconds"], json!(7));
    assert_eq!(
        first["process_environment"]["policy"],
        json!("safe_ambient_allowlist_with_explicit_overrides")
    );
    let manifest_path = PathBuf::from(first["manifest_path"].as_str().unwrap());
    let manifest: JsonValue =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest should read"))
            .expect("manifest should be JSON");
    assert_eq!(
        manifest["contract"],
        json!("ait.server.main_seed_prewarm_manifest.v1")
    );
    assert_eq!(manifest["generation_key"], json!("SNP-WRAPPER"));
    assert_eq!(manifest["timeout_seconds"], json!(7));
    assert_eq!(
        manifest["process_environment"]["ambient_inheritance"],
        json!("allowlist")
    );

    let second = json.prewarm(&request).expect("second wrapper prewarm");
    assert_eq!(second["status"], json!("reused"));
    assert_eq!(second["reused"], json!(true));
    assert_eq!(second["timeout_seconds"], json!(7));
    assert_eq!(
        second["process_environment"]["ambient_secret_forwarding"],
        json!(false)
    );
    assert_eq!(second["steps"], json!([]));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn prewarm_bootstraps_seed_once_and_reuses_generation_stamp() {
    let root = temp_root("reuse");
    let source = root.join("source");
    let seed = root.join("seeds").join("ait").join("main-seed");
    fs::create_dir_all(source.join("src")).expect("source should be created");
    fs::write(source.join("src").join("lib.rs"), "pub fn base() {}\n")
        .expect("source file should be written");

    let request = json!({
        "repo_name": "ait",
        "main_seed_path": seed.to_string_lossy(),
        "source_repo_path": source.to_string_lossy(),
        "generation_key": "SNP-ONE",
        "parallelism": 2,
        "required_paths": ["src/lib.rs", "prewarm-counter.txt"],
        "prewarm_steps": [{
            "step_id": "counter",
            "program": "/bin/sh",
            "args": ["-c", "test \"$CUSTOM_LANGUAGE_HOME\" = /workspace/custom && printf run >> prewarm-counter.txt"],
            "env": {"CUSTOM_LANGUAGE_HOME": "/workspace/custom"},
            "required_paths": ["prewarm-counter.txt"]
        }]
    });

    let first = ci_main_seed_prewarm_json(&request).expect("first prewarm should run");
    assert_eq!(first["status"], json!("prewarmed"));
    assert_eq!(first["reused"], json!(false));
    assert_eq!(
        fs::read_to_string(seed.join("prewarm-counter.txt")).expect("counter should exist"),
        "run"
    );

    let second = ci_main_seed_prewarm_json(&request).expect("second prewarm should reuse");
    assert_eq!(second["status"], json!("reused"));
    assert_eq!(second["reused"], json!(true));
    assert_eq!(second["steps"], json!([]));
    assert_eq!(
        fs::read_to_string(seed.join("prewarm-counter.txt")).expect("counter should still exist"),
        "run",
        "generation-keyed reuse must not run the prewarm command again"
    );

    fs::remove_file(seed.join("src").join("lib.rs")).expect("seed required path should be removed");
    let third = ci_main_seed_prewarm_json(&request)
        .expect("missing required path should rebuild from source repo");
    assert_eq!(third["status"], json!("prewarmed"));
    assert_eq!(third["reused"], json!(false));
    assert_eq!(third["replaced_existing_seed"], json!(true));
    assert!(
        seed.join("src").join("lib.rs").is_file(),
        "rebuild should restore required source file"
    );
    assert_eq!(
        fs::read_to_string(seed.join("prewarm-counter.txt")).expect("counter should be recreated"),
        "run"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn reuse_only_checks_seed_generation_without_mutating_stale_seed() {
    let root = temp_root("reuse-only");
    let source = root.join("source");
    let seed = root.join("seeds").join("ait").join("main-seed");
    fs::create_dir_all(&source).expect("source should be created");
    fs::write(source.join("file.txt"), "v1\n").expect("source file should be written");

    let base_request = json!({
        "repo_name": "ait",
        "main_seed_path": seed.to_string_lossy(),
        "source_repo_path": source.to_string_lossy(),
        "generation_key": "SNP-ONE",
        "prewarm_steps": [{
            "step_id": "marker",
            "program": "/bin/sh",
            "args": ["-c", "printf one > marker.txt"]
        }]
    });
    ci_main_seed_prewarm_json(&base_request).expect("initial seed should prewarm");
    assert_eq!(
        fs::read_to_string(seed.join("marker.txt")).expect("marker should exist"),
        "one"
    );

    let reused = ci_main_seed_prewarm_json(&json!({
        "repo_name": "ait",
        "main_seed_path": seed.to_string_lossy(),
        "generation_key": "SNP-ONE",
        "reuse_only": true,
        "prewarm_steps": [{
            "step_id": "marker",
            "program": "/bin/sh",
            "args": ["-c", "printf one > marker.txt"]
        }]
    }))
    .expect("matching generation should reuse");
    assert_eq!(reused["status"], json!("reused"));

    let error = ci_main_seed_prewarm_json(&json!({
        "repo_name": "ait",
        "main_seed_path": seed.to_string_lossy(),
        "generation_key": "SNP-TWO",
        "reuse_only": true,
        "prewarm_steps": [{
            "step_id": "marker",
            "program": "/bin/sh",
            "args": ["-c", "printf two > marker.txt"]
        }]
    }))
    .expect_err("stale generation should not be mutated in reuse-only mode");

    assert!(error.contains("is not current for generation"));
    assert_eq!(
        fs::read_to_string(seed.join("marker.txt")).expect("marker should remain unchanged"),
        "one"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn prewarm_steps_are_bounded_parallel_workers() {
    let root = temp_root("parallel");
    let source = root.join("source");
    let seed = root.join("seeds").join("ait").join("main-seed");
    fs::create_dir_all(&source).expect("source should exist");

    let first_script = "touch started-a; i=0; while [ ! -f started-b ] && [ $i -lt 50 ]; do i=$((i+1)); sleep 0.02; done; test -f started-b";
    let second_script = "touch started-b; i=0; while [ ! -f started-a ] && [ $i -lt 50 ]; do i=$((i+1)); sleep 0.02; done; test -f started-a";
    let result = ci_main_seed_prewarm_json(&json!({
        "repo_name": "ait",
        "main_seed_path": seed.to_string_lossy(),
        "source_repo_path": source.to_string_lossy(),
        "generation_key": "SNP-PARALLEL",
        "parallelism": 2,
        "prewarm_steps": [
            {"step_id": "a", "program": "/bin/sh", "args": ["-c", first_script]},
            {"step_id": "b", "program": "/bin/sh", "args": ["-c", second_script]}
        ]
    }))
    .expect("parallel prewarm should pass");

    assert_eq!(result["status"], json!("prewarmed"));
    assert_eq!(result["parallelism"], json!(2));
    assert_eq!(result["step_count"], json!(2));
    assert!(seed.join("started-a").exists());
    assert!(seed.join("started-b").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn shard_prepare_bootstraps_main_seed_before_materializing_shards() {
    let root = temp_root("prepare");
    let source = root.join("source");
    let seed_root = root.join("seeds");
    let seed = seed_root.join("ait").join("main-seed");
    fs::create_dir_all(source.join("src")).expect("source should be created");
    fs::write(source.join("src").join("a.rs"), "fn base() {}\n")
        .expect("source file should be written");

    let prepared = ci_test_shard_prepare_json(&json!({
        "job_id": "job-ci",
        "job_type": "patchset.ci",
        "platform": "macos",
        "materialization_strategy": "sparse_copy_up",
        "main_seed_root": seed_root.to_string_lossy(),
        "source_repo_path": source.to_string_lossy(),
        "generation_key": "SNP-PREPARE",
        "ram_shard_root": root.join("AIT_RAM-shards").to_string_lossy(),
        "admitted_cpu_tokens": 2,
        "copy_up_paths": ["src/a.rs"],
        "test_count": 4,
        "prewarm_steps": [{
            "step_id": "marker",
            "program": "/bin/sh",
            "args": ["-c", "printf warm > prewarmed.txt"],
            "required_paths": ["prewarmed.txt"]
        }],
        "required_paths": ["prewarmed.txt"],
        "payload": {
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-PREPARE"
        }
    }))
    .expect("prepare should bootstrap seed before shard materialization");

    assert_eq!(prepared["main_seed_prewarm"]["status"], json!("prewarmed"));
    assert!(seed.join("prewarmed.txt").exists());
    assert_eq!(prepared["main_seed"]["path"], json!(seed.to_string_lossy()));
    assert_eq!(prepared["thread_pool_shards"]["shard_count"], json!(2));
    for shard in prepared["thread_pool_shards"]["shards"]
        .as_array()
        .expect("shards should exist")
    {
        assert_eq!(shard["assignment"]["test_count"], json!(2));
    }

    let _ = fs::remove_dir_all(root);
}
