use crate::foundation::ci_runtime_json::TestShardPlanJson;
use crate::foundation::test_shards::ci_test_shard_plan_json_impl;
use serde_json::{json, Value as JsonValue};
use std::fs;

use super::helpers::{
    optional_bool, path_from_json, path_string, relative_path_array, required_text,
};
use super::process::run_unmount;
use super::seed_paths::{guard_shard_path, preserve_repo_dir_cleanup};
use super::{CLEANUP_STRATEGY_PRESERVE_REPO_DIR, CLEANUP_STRATEGY_REMOVE_SHARD_DIR};

pub fn ci_test_shard_cleanup_json(request: &JsonValue) -> Result<JsonValue, String> {
    TestShardPlanJson::stateless().cleanup(request)
}

pub(crate) fn ci_test_shard_cleanup_json_impl(request: &JsonValue) -> Result<JsonValue, String> {
    let request_object = request
        .as_object()
        .ok_or_else(|| "ci-test-shard-cleanup payload must be a JSON object.".to_string())?;
    let cleanup_reason = required_text(request_object, "cleanup_reason")?;
    if cleanup_reason != "all_assigned_tests_complete" && cleanup_reason != "core_token_reclaimed" {
        return Err(
            "`cleanup_reason` must be all_assigned_tests_complete or core_token_reclaimed."
                .to_string(),
        );
    }
    if cleanup_reason == "all_assigned_tests_complete" {
        let all_shards_completed =
            optional_bool(request_object, "all_shards_completed")?.unwrap_or(false);
        let outputs_merged = optional_bool(request_object, "outputs_merged")?.unwrap_or(false);
        if !all_shards_completed {
            return Err(
                "`all_shards_completed` must be true before normal dirty cleanup.".to_string(),
            );
        }
        if !outputs_merged {
            return Err("`outputs_merged` must be true before normal dirty cleanup.".to_string());
        }
    }

    let dry_run = optional_bool(request_object, "dry_run")?.unwrap_or(false);
    let execute_overlay_unmount =
        optional_bool(request_object, "execute_overlay_unmount")?.unwrap_or(false);
    let preserve_repo_dir = cleanup_reason == "all_assigned_tests_complete"
        && optional_bool(request_object, "preserve_repo_dir")?.unwrap_or(false);
    let copy_up_paths = if preserve_repo_dir {
        relative_path_array(request_object, "copy_up_paths")?
    } else {
        Vec::new()
    };
    let plan = ci_test_shard_plan_json_impl(request)?;
    let seed_path = path_from_json(&plan["main_seed"]["path"], "main_seed.path")?;
    let shards = plan["thread_pool_shards"]["shards"]
        .as_array()
        .ok_or_else(|| "ci-test-shard-plan did not produce shard array.".to_string())?;

    let mut cleaned_shards = Vec::new();
    for shard in shards {
        let shard_path = path_from_json(&shard["path"], "thread_pool_shards.shards[].path")?;
        guard_shard_path(&seed_path, &shard_path)?;
        let repo_dir = shard_path.join("repo");
        let mut unmount = JsonValue::Null;
        if execute_overlay_unmount && repo_dir.exists() && !dry_run {
            unmount = run_unmount(&repo_dir)?;
        }
        let existed_before = shard_path.exists();
        let preserved_cleanup = if preserve_repo_dir && existed_before && !dry_run {
            preserve_repo_dir_cleanup(&seed_path, &shard_path, &repo_dir, &copy_up_paths)?
        } else {
            JsonValue::Null
        };
        if existed_before && !dry_run && !preserve_repo_dir {
            fs::remove_dir_all(&shard_path).map_err(|exc| {
                format!(
                    "Failed to remove shard directory `{}`: {exc}",
                    path_string(&shard_path)
                )
            })?;
        }
        let repo_dir_exists_after_cleanup = repo_dir.exists();
        cleaned_shards.push(json!({
            "shard_id": shard["shard_id"].clone(),
            "path": path_string(&shard_path),
            "repo_dir": path_string(&repo_dir),
            "existed_before_cleanup": existed_before,
            "removed": existed_before && !dry_run && !preserve_repo_dir,
            "repo_dir_preserved": existed_before && !dry_run && preserve_repo_dir,
            "repo_dir_exists_after_cleanup": repo_dir_exists_after_cleanup,
            "unmount": unmount,
            "preserved_cleanup": preserved_cleanup,
            "released_core_token": true
        }));
    }

    Ok(json!({
        "contract": "ait.server.ci_test_shard_runtime.v1",
        "operation": "cleanup",
        "cleanup_reason": cleanup_reason,
        "dry_run": dry_run,
        "strategy": if preserve_repo_dir {
            CLEANUP_STRATEGY_PRESERVE_REPO_DIR
        } else {
            CLEANUP_STRATEGY_REMOVE_SHARD_DIR
        },
        "main_seed": {
            "path": path_string(&seed_path),
            "preserved": true,
            "immutable": true
        },
        "dirty_cleanup": {
            "remove_shard_directories": !preserve_repo_dir,
            "reset_tracked_files_in_shard": true,
            "remove_untracked_files_in_shard": true,
            "remove_generated_artifacts_in_shard": true,
            "write_main_seed": false
        },
        "thread_pool_shards": {
            "shard_count": cleaned_shards.len(),
            "shards": cleaned_shards
        }
    }))
}
