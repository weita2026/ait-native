use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::fs;
use std::path::PathBuf;

use super::helpers::{optional_text, path_string, write_json_file};
use super::overlayfs::prepare_overlayfs;
use super::seed_paths::{
    ensure_reusable_repo_dir, prune_repo_paths_missing_from_seed, reset_prepare_runtime_dirs,
    reset_stale_shard_before_prepare, shard_path_is_reusable,
};
use super::sparse_copy_up::prepare_sparse_copy_up;
use super::RUNTIME_MANIFEST_FILE;

#[derive(Clone, Copy)]
pub(super) enum ShardMaterializationStrategy {
    LinuxOverlayFs,
    MacosApfsCloneOrSparseCopyUp,
    SparseCopyUp,
}

impl ShardMaterializationStrategy {
    pub(super) fn from_request(
        request: &JsonMap<String, JsonValue>,
        platform: &str,
    ) -> Result<Self, String> {
        match optional_text(request, "materialization_strategy").as_deref() {
            Some("overlayfs") => Ok(Self::LinuxOverlayFs),
            Some("apfs_clone_or_sparse_copy_up") => Ok(Self::MacosApfsCloneOrSparseCopyUp),
            Some("sparse_copy_up") => Ok(Self::SparseCopyUp),
            Some(value) => Err(format!(
                "`materialization_strategy` must be overlayfs, apfs_clone_or_sparse_copy_up, or sparse_copy_up; got {value}."
            )),
            None => match platform {
                "linux" => Ok(Self::LinuxOverlayFs),
                "macos" => Ok(Self::MacosApfsCloneOrSparseCopyUp),
                _ => Ok(Self::SparseCopyUp),
            },
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::LinuxOverlayFs => "overlayfs",
            Self::MacosApfsCloneOrSparseCopyUp => "apfs_clone_or_sparse_copy_up",
            Self::SparseCopyUp => "sparse_copy_up",
        }
    }
}

pub(super) struct PrepareOneShardInput<'a> {
    pub(super) shard: &'a JsonValue,
    pub(super) shard_path: PathBuf,
    pub(super) seed_path: PathBuf,
    pub(super) platform: &'a str,
    pub(super) strategy: ShardMaterializationStrategy,
    pub(super) dry_run: bool,
    pub(super) execute_overlay_mount: bool,
    pub(super) prefer_apfs_clone: bool,
    pub(super) preserve_repo_dir: bool,
    pub(super) copy_up_paths: &'a [PathBuf],
    pub(super) immutable_link_paths: &'a [PathBuf],
    pub(super) implicit_immutable_seed_links: bool,
}

pub(super) fn prepare_one_shard(input: PrepareOneShardInput<'_>) -> Result<JsonValue, String> {
    let shard_id = input.shard["shard_id"].clone();
    let input_dir = input.shard_path.join("input");
    let output_dir = input.shard_path.join("output");
    let upper_dir = input.shard_path.join("upper");
    let work_dir = input.shard_path.join("work");
    let repo_dir = input.shard_path.join("repo");
    let mut preserved_repo_dir_between_runs = false;
    let stale_shard_removed_before_prepare = if input.dry_run {
        false
    } else if input.preserve_repo_dir && shard_path_is_reusable(&input.shard_path)? {
        preserved_repo_dir_between_runs = repo_dir.exists();
        reset_prepare_runtime_dirs(&[&input_dir, &output_dir, &upper_dir, &work_dir])?;
        ensure_reusable_repo_dir(&repo_dir)?;
        let _ =
            prune_repo_paths_missing_from_seed(&repo_dir, &input.seed_path, input.copy_up_paths)?;
        false
    } else {
        reset_stale_shard_before_prepare(&input.shard_path)?
    };
    if !input.dry_run {
        fs::create_dir_all(&input_dir).map_err(|exc| {
            format!(
                "Failed to create shard input dir `{}`: {exc}",
                path_string(&input_dir)
            )
        })?;
        fs::create_dir_all(&output_dir).map_err(|exc| {
            format!(
                "Failed to create shard output dir `{}`: {exc}",
                path_string(&output_dir)
            )
        })?;
        fs::create_dir_all(&upper_dir).map_err(|exc| {
            format!(
                "Failed to create shard upper dir `{}`: {exc}",
                path_string(&upper_dir)
            )
        })?;
        fs::create_dir_all(&work_dir).map_err(|exc| {
            format!(
                "Failed to create shard work dir `{}`: {exc}",
                path_string(&work_dir)
            )
        })?;
        fs::create_dir_all(&repo_dir).map_err(|exc| {
            format!(
                "Failed to create shard repo dir `{}`: {exc}",
                path_string(&repo_dir)
            )
        })?;
        write_json_file(&input_dir.join("assignment.json"), &input.shard["input"])?;
    }

    let materialization = match input.strategy {
        ShardMaterializationStrategy::LinuxOverlayFs => prepare_overlayfs(&input)?,
        ShardMaterializationStrategy::MacosApfsCloneOrSparseCopyUp
        | ShardMaterializationStrategy::SparseCopyUp => prepare_sparse_copy_up(&input)?,
    };

    let manifest = json!({
        "contract": "ait.server.ci_test_shard_runtime_manifest.v1",
        "shard_id": shard_id,
        "platform": input.platform,
        "strategy": input.strategy.name(),
        "main_seed_path": path_string(&input.seed_path),
        "seed_immutable": true,
        "copy_entire_main_seed": false,
        "whole_seed_directory_symlink": false,
        "shard_path": path_string(&input.shard_path),
        "repo_dir": path_string(&repo_dir),
        "upper_dir": path_string(&upper_dir),
        "work_dir": path_string(&work_dir),
        "input_dir": path_string(&input_dir),
        "output_dir": path_string(&output_dir),
        "assignment": input.shard["input"].clone(),
        "cleanup_when": "after_all_assigned_tests_complete_or_core_token_reclaimed"
    });
    if !input.dry_run {
        write_json_file(&input.shard_path.join(RUNTIME_MANIFEST_FILE), &manifest)?;
    }

    Ok(json!({
        "shard_id": input.shard["shard_id"].clone(),
        "core_token": input.shard["core_token"].clone(),
        "path": path_string(&input.shard_path),
        "repo_dir": path_string(&repo_dir),
        "upper_dir": path_string(&upper_dir),
        "work_dir": path_string(&work_dir),
        "input_dir": path_string(&input_dir),
        "output_dir": path_string(&output_dir),
        "assignment": input.shard["input"].clone(),
        "materialization": materialization,
        "manifest_path": path_string(&input.shard_path.join(RUNTIME_MANIFEST_FILE)),
        "prepare_reset": {
            "stale_shard_removed_before_prepare": stale_shard_removed_before_prepare,
            "idempotent_retry_prepare": true,
            "preserved_repo_dir_between_runs": preserved_repo_dir_between_runs
        },
        "dirty_cleanup": {
            "when": "after_all_assigned_tests_complete_or_core_token_reclaimed",
            "single_final_cleanup": true,
            "remove_shard_dir": !input.preserve_repo_dir,
            "write_main_seed": false
        }
    }))
}
