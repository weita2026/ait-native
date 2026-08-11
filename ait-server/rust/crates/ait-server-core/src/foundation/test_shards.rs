use crate::foundation::ci_runtime_json::TestShardPlanJson;
use crate::foundation::scheduler::{SchedulerDeploymentPosture, SchedulerPolicy};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::env;
use std::path::{Path, PathBuf};

const MAIN_SEED_DIR_NAME: &str = "main-seed";
const DEFAULT_MAIN_SEED_MAX_IDLE_SECONDS: i64 = 7 * 24 * 60 * 60;
const DEFAULT_SHARD_MAX_IDLE_SECONDS: i64 = 60 * 60;
const TG1_REQUIRED_CPU_TOKENS: usize = 10;

pub fn ci_test_shard_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    TestShardPlanJson::stateless().plan(request)
}

pub(crate) fn ci_test_shard_plan_json_impl(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "ci-test-shard-plan payload must be a JSON object.".to_string())?;
    let job_type = required_text(request, "job_type")?;
    let payload = required_object(request, "payload")?;
    if job_type != "patchset.ci" && job_type != "repo.ci" {
        return Err(format!(
            "`job_type` must be patchset.ci or repo.ci for test shard planning, got {job_type}."
        ));
    }

    let repo_name = required_text(payload, "repo_name")?;
    let repo_segment = safe_path_segment(&repo_name)?;
    let seed_root = optional_text(request, "main_seed_root")
        .unwrap_or_else(|| path_string(&default_main_seed_root()));
    let ram_shard_root = optional_text(request, "ram_shard_root")
        .unwrap_or_else(|| path_string(&default_ram_shard_root()));
    let seed_root_path = PathBuf::from(&seed_root);
    let ram_shard_root_path = PathBuf::from(&ram_shard_root);
    let seed_path = optional_text(request, "main_seed_path")
        .map(PathBuf::from)
        .unwrap_or_else(|| seed_root_path.join(&repo_segment).join(MAIN_SEED_DIR_NAME));
    let target = target_json(&job_type, payload);
    let suite_ids = suite_ids(&job_type, payload);
    let full_test = suite_ids
        .iter()
        .any(|suite_id| is_full_test_suite(suite_id));
    let seed_available = seed_path.is_dir();
    let admitted_cpu_tokens = positive_i64(request, "admitted_cpu_tokens")?
        .unwrap_or_else(|| default_admitted_cpu_tokens(request, &suite_ids));
    let shard_ids = shard_ids(request, admitted_cpu_tokens)?;
    let main_seed_max_idle_seconds = positive_i64(request, "main_seed_max_idle_seconds")?
        .unwrap_or(DEFAULT_MAIN_SEED_MAX_IDLE_SECONDS);
    let shard_max_idle_seconds =
        positive_i64(request, "shard_max_idle_seconds")?.unwrap_or(DEFAULT_SHARD_MAX_IDLE_SECONDS);
    let pool_id = optional_text(request, "pool_id")
        .unwrap_or_else(|| default_pool_id(&job_type, payload, &suite_ids));
    let pool_segment = safe_path_segment(&pool_id)?;
    let shard_root = optional_text(request, "shard_root")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            ram_shard_root_path
                .join(&repo_segment)
                .join("thread-pool-shards")
                .join(&pool_segment)
        });
    let pool_key = format!("repo:{repo_name}:ci-test-shard-pool:{pool_segment}");
    let materialization_source = if seed_available {
        "immutable_main_seed"
    } else {
        "bootstrap_fixed_main_seed_then_writable_shards"
    };
    let materialization_method = optional_text(request, "materialization_method")
        .unwrap_or_else(|| "platform_adaptive_overlay_or_copy_up".to_string());
    let assignments = shard_assignments(request, shard_ids.len())?;

    let shards = shard_ids
        .iter()
        .zip(assignments.iter())
        .map(|(shard_id, assignment)| {
            let shard_segment = safe_path_segment(shard_id)?;
            let path = shard_root.join(&shard_segment);
            Ok(json!({
                "shard_id": shard_id,
                "core_token": shard_id,
                "path": path_string(&path),
                "input_dir": path_string(&path.join("input")),
                "output_dir": path_string(&path.join("output")),
                "storage_class": "AIT_RAM",
                "lease": {
                    "scope": "exclusive_core_shard",
                    "write_key": format!("{pool_key}:shard:{shard_segment}"),
                    "held_for": "one_scheduler_cpu_token"
                },
                "materialization": {
                    "source": materialization_source,
                    "method": materialization_method,
                    "base_seed_path": path_string(&seed_path),
                    "create_shard_dir_if_missing": true,
                    "bootstrap_main_seed_if_missing": !seed_available,
                    "shares_prewarmed_environment": true,
                    "copy_entire_main_seed": false,
                    "seed_is_read_only": true,
                    "whole_seed_directory_symlink": false,
                    "writable_layer_required": true,
                    "linux_preferred_method": "overlayfs",
                    "macos_preferred_method": "apfs_clone_or_sparse_copy_up",
                    "fallback_method": "sparse_copy_up_selective_links"
                },
                "input": assignment,
                "output": {
                    "dir": path_string(&path.join("output")),
                    "partitioned_by_shard": true,
                    "merge_after_all_shards_complete": true
                },
                "cleanup": {
                    "strategy": "single_final_dirty_cleanup",
                    "when": "core_token_reclaimed",
                    "reset_tracked_files": true,
                    "remove_untracked_files": true,
                    "remove_generated_artifacts": true,
                    "release_shard_lease": true
                },
                "lifecycle": {
                    "max_idle_seconds": shard_max_idle_seconds,
                    "cleanup_when_idle": true,
                    "cleanup_on_core_reclaim": true
                }
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(json!({
        "contract": "ait.server.ci_test_shards.v1",
        "job_type": job_type,
        "job_id": optional_text(request, "job_id"),
        "repo_name": repo_name,
        "suite_ids": suite_ids,
        "full_test": full_test,
        "target": target,
        "main_seed": {
            "repo_name": repo_name,
            "path": path_string(&seed_path),
            "root": path_string(&seed_root_path),
            "storage_class": "lyravo_ssd",
            "available": seed_available,
            "per_repo": true,
            "prewarmed_environment": true,
            "action": if seed_available { "reuse_main_seed" } else { "bootstrap_fixed_main_seed" },
            "lifecycle": {
                "max_idle_seconds": main_seed_max_idle_seconds,
                "refresh_last_used_on_use": true,
                "cleanup_when_idle": true
            }
        },
        "thread_pool_shards": {
            "key": pool_key,
            "root": path_string(&shard_root),
            "storage_class": "AIT_RAM",
            "repo_segment": repo_segment,
            "pool_id": pool_id,
            "shard_count": shards.len(),
            "admitted_cpu_tokens": admitted_cpu_tokens,
            "one_shard_dir_per_cpu_token": true,
            "shards": shards,
            "lease": {
                "scope": "exclusive_shard_pool",
                "write_key": pool_key,
                "held_for": "entire_ci_job",
                "shard_write_keys_required": true
            }
        },
        "materialization": {
            "source": materialization_source,
            "method": materialization_method,
            "main_seed_path": path_string(&seed_path),
            "main_seed_available": seed_available,
            "copy_entire_main_seed": false,
            "seed_is_read_only": true,
            "whole_seed_directory_symlink": false,
            "writable_layer_required": true,
            "linux_preferred_method": "overlayfs",
            "macos_preferred_method": "apfs_clone_or_sparse_copy_up",
            "fallback_method": "sparse_copy_up_selective_links",
            "rule": if seed_available {
                "derive per-core shard directories from the immutable prewarmed main-seed using platform-adaptive writable overlay or copy-up materialization"
            } else {
                "bootstrap one fixed prewarmed main-seed, then derive per-core writable shard directories from it"
            }
        },
        "execution": {
            "runner_parallelism_source": "scheduler_admitted_cpu_tokens",
            "runner_parallelism": admitted_cpu_tokens,
            "one_shard_dir_per_cpu_token": true,
            "input_output_partitioned_by_shard": true,
            "run_all_assigned_tests_before_cleanup": true,
            "applies_to_patchset_ci": job_type == "patchset.ci",
            "applies_to_repo_ci": job_type == "repo.ci"
        },
        "cleanup": {
            "strategy": "single_final_dirty_cleanup",
            "when": "after all assigned tests complete or when the scheduler core lease is reclaimed",
            "reset_tracked_files": true,
            "remove_untracked_files": true,
            "remove_generated_artifacts": true,
            "release_shard_leases": true
        }
    }))
}

fn default_main_seed_root() -> PathBuf {
    env_path("AIT_NATIVE_SERVER_MAIN_SEED_ROOT")
        .or_else(|| env_path("AIT_MAIN_SEED_ROOT"))
        .or_else(|| env_path("AIT_NATIVE_SERVER_DATA").map(|root| root.join("main-seeds")))
        .unwrap_or_else(|| env::temp_dir().join("ait-server").join("main-seeds"))
}

fn default_ram_shard_root() -> PathBuf {
    env_path("AIT_NATIVE_SERVER_RAM_SHARD_ROOT")
        .or_else(|| env_path("AIT_RAM_SHARD_ROOT"))
        .or_else(|| env_path("AIT_NATIVE_SERVER_CI_TMP_ROOT").map(|root| root.join("test-shards")))
        .or_else(|| {
            env_path("AIT_NATIVE_SERVER_DATA").map(|root| root.join("tmp").join("test-shards"))
        })
        .unwrap_or_else(|| env::temp_dir().join("ait-server").join("test-shards"))
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn target_json(job_type: &str, payload: &JsonMap<String, JsonValue>) -> JsonValue {
    match job_type {
        "patchset.ci" => json!({
            "patchset_id": optional_text(payload, "patchset_id"),
            "snapshot_id": optional_text(payload, "revision_snapshot_id")
                .or_else(|| optional_text(payload, "snapshot_id"))
                .unwrap_or_else(|| "unknown-snapshot".to_string()),
        }),
        "repo.ci" => json!({
            "plane": optional_text(payload, "plane").unwrap_or_else(|| "default".to_string()),
            "target_line": optional_text(payload, "target_line").unwrap_or_else(|| "main".to_string()),
            "snapshot_id": optional_text(payload, "snapshot_id").unwrap_or_else(|| "unknown-snapshot".to_string()),
        }),
        _ => JsonValue::Null,
    }
}

fn default_pool_id(
    job_type: &str,
    payload: &JsonMap<String, JsonValue>,
    suite_ids: &[String],
) -> String {
    let suite_key = if suite_ids.is_empty() {
        "default".to_string()
    } else {
        suite_ids.join("+")
    };
    match job_type {
        "patchset.ci" => format!("patchset-ci-{suite_key}"),
        "repo.ci" => format!(
            "repo-ci-{}-{}-{}",
            optional_text(payload, "plane").unwrap_or_else(|| "default".to_string()),
            optional_text(payload, "target_line").unwrap_or_else(|| "main".to_string()),
            suite_key,
        ),
        _ => "unknown".to_string(),
    }
}

fn default_admitted_cpu_tokens(request: &JsonMap<String, JsonValue>, suite_ids: &[String]) -> i64 {
    let policy = scheduler_policy_from_request(request);
    let requested = if suite_ids
        .iter()
        .any(|suite_id| is_tg1_required_suite(suite_id))
    {
        TG1_REQUIRED_CPU_TOKENS
    } else {
        policy.full_test_job_cpu_tokens
    };
    requested
        .max(1)
        .min(policy.global_cpu_tokens.max(1))
        .min(policy.ci_full_shared_cpu_tokens.max(1)) as i64
}

fn scheduler_policy_from_request(request: &JsonMap<String, JsonValue>) -> SchedulerPolicy {
    let posture = optional_text(request, "scheduler_posture")
        .and_then(|value| SchedulerDeploymentPosture::parse(&value))
        .unwrap_or_else(SchedulerDeploymentPosture::from_environment);
    match positive_i64(request, "host_cpu_cores") {
        Ok(Some(value)) => SchedulerPolicy::for_host_cpu_cores(value as usize, posture),
        _ => SchedulerPolicy::for_detected_host(posture),
    }
}

fn suite_ids(job_type: &str, payload: &JsonMap<String, JsonValue>) -> Vec<String> {
    if job_type == "patchset.ci" {
        return Vec::from([
            optional_text(payload, "suite_id").unwrap_or_else(|| "default".to_string())
        ]);
    }
    if let Some(values) = payload.get("suite_ids").and_then(JsonValue::as_array) {
        return values
            .iter()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
    }
    optional_text(payload, "suite_id")
        .map(|suite_id| Vec::from([suite_id]))
        .unwrap_or_else(|| Vec::from(["default".to_string()]))
}

fn shard_ids(
    request: &JsonMap<String, JsonValue>,
    admitted_cpu_tokens: i64,
) -> Result<Vec<String>, String> {
    if let Some(values) = request.get("shard_ids").and_then(JsonValue::as_array) {
        let mut shards = Vec::new();
        for value in values {
            let shard_id = value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Field `shard_ids` must contain non-empty strings.".to_string())?;
            shards.push(shard_id.to_string());
        }
        if shards.is_empty() {
            return Err("Field `shard_ids` must not be empty.".to_string());
        }
        return Ok(shards);
    }

    Ok((0..admitted_cpu_tokens)
        .map(|index| format!("shard-{index}"))
        .collect())
}

fn shard_assignments(
    request: &JsonMap<String, JsonValue>,
    shard_count: usize,
) -> Result<Vec<JsonValue>, String> {
    if shard_count == 0 {
        return Err("At least one shard is required.".to_string());
    }
    if let Some(items) = optional_string_array(request, "test_items")? {
        return Ok(contiguous_item_shards(&items, shard_count));
    }
    if let Some(test_count) = positive_i64(request, "test_count")? {
        return Ok(contiguous_index_shards(test_count as usize, shard_count));
    }

    Ok((0..shard_count)
        .map(|index| {
            json!({
                "shard_index": index,
                "selection": "runner_partitioned_by_shard_index",
                "test_count": JsonValue::Null
            })
        })
        .collect())
}

fn contiguous_item_shards(items: &[String], shard_count: usize) -> Vec<JsonValue> {
    let ranges = contiguous_ranges(items.len(), shard_count);
    ranges
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| {
            json!({
                "shard_index": index,
                "test_count": end.saturating_sub(start),
                "test_items": items[start..end].to_vec()
            })
        })
        .collect()
}

fn contiguous_index_shards(test_count: usize, shard_count: usize) -> Vec<JsonValue> {
    let ranges = contiguous_ranges(test_count, shard_count);
    ranges
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| {
            json!({
                "shard_index": index,
                "test_count": end.saturating_sub(start),
                "test_index_range": {
                    "start": start,
                    "end_exclusive": end
                }
            })
        })
        .collect()
}

fn contiguous_ranges(total: usize, shard_count: usize) -> Vec<(usize, usize)> {
    let base = total / shard_count;
    let extra = total % shard_count;
    let mut start = 0;
    (0..shard_count)
        .map(|index| {
            let len = base + usize::from(index < extra);
            let end = start + len;
            let range = (start, end);
            start = end;
            range
        })
        .collect()
}

fn is_full_test_suite(suite_id: &str) -> bool {
    matches!(
        suite_id.trim().to_ascii_lowercase().as_str(),
        "full" | "full-test" | "full_test" | "full-repo" | "full_repo" | "all"
    )
}

fn is_tg1_required_suite(suite_id: &str) -> bool {
    matches!(
        suite_id.trim().to_ascii_lowercase().as_str(),
        "tg1" | "tg-1" | "tg1_required" | "tg-1-required"
    )
}

fn required_object<'a>(
    value: &'a JsonMap<String, JsonValue>,
    key: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .get(key)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Field `{key}` must be a JSON object."))
}

fn required_text(value: &JsonMap<String, JsonValue>, key: &str) -> Result<String, String> {
    optional_text(value, key).ok_or_else(|| format!("Field `{key}` must be a non-empty string."))
}

fn optional_text(value: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_string_array(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(values) = value.get(key) else {
        return Ok(None);
    };
    let values = values
        .as_array()
        .ok_or_else(|| format!("Field `{key}` must be an array of non-empty strings."))?;
    let mut parsed = Vec::new();
    for value in values {
        let item = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Field `{key}` must contain non-empty strings."))?;
        parsed.push(item.to_string());
    }
    Ok(Some(parsed))
}

fn positive_i64(value: &JsonMap<String, JsonValue>, key: &str) -> Result<Option<i64>, String> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => {
            let value = number
                .as_i64()
                .ok_or_else(|| format!("Field `{key}` must be a positive integer."))?;
            if value < 1 {
                Err(format!("Field `{key}` must be a positive integer."))
            } else {
                Ok(Some(value))
            }
        }
        Some(_) => Err(format!("Field `{key}` must be a positive integer.")),
    }
}

fn safe_path_segment(value: &str) -> Result<String, String> {
    let segment = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if segment.is_empty() || segment == "." || segment == ".." {
        Err(format!(
            "Value `{value}` cannot be used as a shard path segment."
        ))
    } else {
        Ok(segment)
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
