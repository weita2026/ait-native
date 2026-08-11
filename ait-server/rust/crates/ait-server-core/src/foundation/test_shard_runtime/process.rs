use serde_json::{json, Value as JsonValue};
use std::fs;
use std::path::Path;
use std::process::Command;

use super::helpers::path_string;

pub(super) fn apfs_clone_or_copy(source: &Path, destination: &Path) -> Result<String, String> {
    let output = Command::new("/bin/cp")
        .arg("-c")
        .arg(source)
        .arg(destination)
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            return Ok("apfs_clonefile_cp_c".to_string());
        }
    }
    fs::copy(source, destination).map_err(|exc| {
        format!(
            "Failed to copy-up `{}` to `{}` after APFS clone fallback: {exc}",
            path_string(source),
            path_string(destination)
        )
    })?;
    Ok("std_copy_after_apfs_clone_fallback".to_string())
}

pub(super) fn run_overlay_mount(repo_dir: &Path, mount_options: &str) -> Result<JsonValue, String> {
    let output = Command::new("mount")
        .arg("-t")
        .arg("overlay")
        .arg("overlay")
        .arg("-o")
        .arg(mount_options)
        .arg(repo_dir)
        .output()
        .map_err(|exc| format!("Failed to execute overlay mount: {exc}"))?;
    if !output.status.success() {
        return Err(format!(
            "Overlay mount failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(json!({
        "status": "mounted",
        "stdout": String::from_utf8_lossy(&output.stdout).trim(),
        "stderr": String::from_utf8_lossy(&output.stderr).trim()
    }))
}

pub(super) fn run_unmount(repo_dir: &Path) -> Result<JsonValue, String> {
    let output = Command::new("umount")
        .arg(repo_dir)
        .output()
        .map_err(|exc| format!("Failed to execute overlay unmount: {exc}"))?;
    if !output.status.success() {
        return Err(format!(
            "Overlay unmount failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(json!({
        "status": "unmounted",
        "stdout": String::from_utf8_lossy(&output.stdout).trim(),
        "stderr": String::from_utf8_lossy(&output.stderr).trim()
    }))
}
