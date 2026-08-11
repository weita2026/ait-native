use serde_json::{json, Value as JsonValue};

use super::helpers::path_string;
use super::materialization::PrepareOneShardInput;
use super::process::run_overlay_mount;

pub(super) fn prepare_overlayfs(input: &PrepareOneShardInput<'_>) -> Result<JsonValue, String> {
    let upper_dir = input.shard_path.join("upper");
    let work_dir = input.shard_path.join("work");
    let repo_dir = input.shard_path.join("repo");
    let mount_options = format!(
        "lowerdir={},upperdir={},workdir={}",
        path_string(&input.seed_path),
        path_string(&upper_dir),
        path_string(&work_dir)
    );
    let mount_command = vec![
        "mount".to_string(),
        "-t".to_string(),
        "overlay".to_string(),
        "overlay".to_string(),
        "-o".to_string(),
        mount_options.clone(),
        path_string(&repo_dir),
    ];
    let mount_result = if input.execute_overlay_mount && !input.dry_run {
        run_overlay_mount(&repo_dir, &mount_options)?
    } else {
        JsonValue::Null
    };
    Ok(json!({
        "strategy": "overlayfs",
        "platform": input.platform,
        "lowerdir": path_string(&input.seed_path),
        "upperdir": path_string(&upper_dir),
        "workdir": path_string(&work_dir),
        "merged_repo": path_string(&repo_dir),
        "copy_up_semantics": "kernel_overlayfs_copy_up",
        "patch_touched_files_are_shard_local": true,
        "whole_seed_directory_symlink": false,
        "copy_entire_main_seed": false,
        "execute_overlay_mount": input.execute_overlay_mount,
        "mount_command": mount_command,
        "mount_result": mount_result
    }))
}
