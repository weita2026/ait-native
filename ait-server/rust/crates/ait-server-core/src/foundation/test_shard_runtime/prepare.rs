use crate::foundation::ci_runtime_json::TestShardPlanJson;
use crate::foundation::main_seed_prewarm::{
    ci_main_seed_prewarm_for_plan_json, request_has_main_seed_prewarm,
};
use crate::foundation::test_shards::ci_test_shard_plan_json_impl;
use serde_json::{json, Value as JsonValue};

use super::helpers::{
    optional_bool, path_from_json, path_string, platform_name, relative_path_array,
};
use super::materialization::{
    prepare_one_shard, PrepareOneShardInput, ShardMaterializationStrategy,
};
use super::seed_paths::guard_shard_path;
use super::{CLEANUP_STRATEGY_PRESERVE_REPO_DIR, CLEANUP_STRATEGY_REMOVE_SHARD_DIR};

pub fn ci_test_shard_prepare_json(request: &JsonValue) -> Result<JsonValue, String> {
    TestShardPlanJson::stateless().prepare(request)
}

pub(crate) fn ci_test_shard_prepare_json_impl(request: &JsonValue) -> Result<JsonValue, String> {
    let request_object = request
        .as_object()
        .ok_or_else(|| "ci-test-shard-prepare payload must be a JSON object.".to_string())?;
    let mut plan = ci_test_shard_plan_json_impl(request)?;
    let platform = platform_name(request_object);
    let strategy = ShardMaterializationStrategy::from_request(request_object, &platform)?;
    let dry_run = optional_bool(request_object, "dry_run")?.unwrap_or(false);
    let execute_overlay_mount =
        optional_bool(request_object, "execute_overlay_mount")?.unwrap_or(false);
    let prefer_apfs_clone =
        optional_bool(request_object, "prefer_apfs_clone")?.unwrap_or(platform == "macos");
    let preserve_repo_dir = optional_bool(request_object, "preserve_repo_dir")?.unwrap_or(false);
    let immutable_paths_explicit = request_object.contains_key("immutable_link_paths");
    let copy_up_paths = relative_path_array(request_object, "copy_up_paths")?;
    let immutable_link_paths = relative_path_array(request_object, "immutable_link_paths")?;
    let implicit_immutable_seed_links =
        optional_bool(request_object, "implicit_immutable_seed_links")?
            .unwrap_or(!immutable_paths_explicit);

    let mut seed_path = path_from_json(&plan["main_seed"]["path"], "main_seed.path")?;
    let main_seed_prewarm = if request_has_main_seed_prewarm(request_object) {
        let result = ci_main_seed_prewarm_for_plan_json(request, &plan)?;
        plan = ci_test_shard_plan_json_impl(request)?;
        seed_path = path_from_json(&plan["main_seed"]["path"], "main_seed.path")?;
        result
    } else {
        JsonValue::Null
    };
    if !seed_path.is_dir() {
        return Err(format!(
            "main_seed path `{}` must exist before shard prepare; bootstrap the fixed prewarmed seed first.",
            path_string(&seed_path)
        ));
    }

    let shards = plan["thread_pool_shards"]["shards"]
        .as_array()
        .ok_or_else(|| "ci-test-shard-plan did not produce shard array.".to_string())?;
    let mut prepared_shards = Vec::new();
    for shard in shards {
        let shard_path = path_from_json(&shard["path"], "thread_pool_shards.shards[].path")?;
        guard_shard_path(&seed_path, &shard_path)?;
        let prepared = prepare_one_shard(PrepareOneShardInput {
            shard,
            shard_path,
            seed_path: seed_path.clone(),
            platform: &platform,
            strategy,
            dry_run,
            execute_overlay_mount,
            prefer_apfs_clone,
            preserve_repo_dir,
            copy_up_paths: &copy_up_paths,
            immutable_link_paths: &immutable_link_paths,
            implicit_immutable_seed_links,
        })?;
        prepared_shards.push(prepared);
    }

    Ok(json!({
        "contract": "ait.server.ci_test_shard_runtime.v1",
        "operation": "prepare",
        "platform": platform,
        "strategy": strategy.name(),
        "main_seed": {
            "path": path_string(&seed_path),
            "immutable": true,
            "writable": false
        },
        "main_seed_prewarm": main_seed_prewarm,
        "seed_write_policy": {
            "write_main_seed": false,
            "whole_seed_directory_symlink": false,
            "copy_entire_main_seed": false,
            "patch_touched_files_are_shard_local": true
        },
        "execution": {
            "runner_parallelism": plan["execution"]["runner_parallelism"].clone(),
            "one_shard_dir_per_cpu_token": true,
            "input_output_partitioned_by_shard": true,
            "run_all_assigned_tests_before_cleanup": true
        },
        "materialization": {
            "linux": "overlayfs_lower_main_seed_upper_ait_ram",
            "macos": "apfs_clone_or_sparse_copy_up",
            "fallback": "sparse_copy_up_selective_links",
            "selected": strategy.name(),
            "dry_run": dry_run
        },
        "thread_pool_shards": {
            "shard_count": prepared_shards.len(),
            "shards": prepared_shards
        },
        "cleanup_contract": {
            "normal_cleanup_requires_all_shards_completed": true,
            "normal_cleanup_requires_outputs_merged": true,
            "core_token_reclaim_can_cleanup_one_or_more_shards": true,
            "strategy": if preserve_repo_dir {
                CLEANUP_STRATEGY_PRESERVE_REPO_DIR
            } else {
                CLEANUP_STRATEGY_REMOVE_SHARD_DIR
            }
        }
    }))
}
