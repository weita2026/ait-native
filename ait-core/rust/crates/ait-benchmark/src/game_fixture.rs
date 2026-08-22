use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::digest_workspace;

pub const GAME_FIXTURE_MANIFEST_CONTRACT: &str = "ait-agent-token-game-fixture-manifest/v1";
pub const GAME_FIXTURE_TRANSFORM_CONTRACT: &str = "ait-agent-token-game-source-transform/v1";
pub const GAME_FIXTURE_RECEIPT_CONTRACT: &str = "ait-agent-token-game-fixture-receipt/v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameFixtureManifest {
    pub contract: String,
    pub fixture_id: String,
    pub revision: String,
    pub baseline_dir: PathBuf,
    pub evaluator_path: PathBuf,
    pub browser_evaluator_path: PathBuf,
    pub metadata_excludes: Vec<String>,
    pub workloads: Vec<GameWorkloadDeclaration>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameWorkloadDeclaration {
    pub workload_id: String,
    pub name: String,
    pub scale: String,
    pub overlay_dir: PathBuf,
    pub transform_path: PathBuf,
    pub acceptance_path: PathBuf,
    pub expected_content_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameSourceTransform {
    pub contract: String,
    pub workload_id: String,
    pub replacements: Vec<GameSourceReplacement>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameSourceReplacement {
    pub path: PathBuf,
    pub from: String,
    pub to: String,
    pub expected_matches: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct GameFixtureReceipt {
    pub contract: &'static str,
    pub fixture_id: String,
    pub fixture_revision: String,
    pub workload_id: String,
    pub workload_name: String,
    pub workload_scale: String,
    pub output_dir: PathBuf,
    pub content_digest: String,
    pub acceptance_path: PathBuf,
    pub evaluator_path: PathBuf,
    pub browser_evaluator_path: PathBuf,
}

pub fn load_game_fixture_manifest(path: &Path) -> Result<GameFixtureManifest, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "Failed to read game fixture manifest {}: {error}",
            path.display()
        )
    })?;
    let manifest = serde_json::from_slice::<GameFixtureManifest>(&bytes).map_err(|error| {
        format!(
            "Failed to decode game fixture manifest {}: {error}",
            path.display()
        )
    })?;
    validate_game_fixture_manifest(path, &manifest)?;
    Ok(manifest)
}

pub fn materialize_game_fixture(
    manifest_path: &Path,
    workload_id: &str,
    output_dir: &Path,
) -> Result<GameFixtureReceipt, String> {
    let manifest = load_game_fixture_manifest(manifest_path)?;
    let manifest_dir = manifest_path.parent().ok_or_else(|| {
        format!(
            "Game fixture manifest has no parent directory: {}",
            manifest_path.display()
        )
    })?;
    let workload = manifest
        .workloads
        .iter()
        .find(|workload| workload.workload_id == workload_id)
        .ok_or_else(|| format!("Unknown game benchmark workload: {workload_id}"))?;

    prepare_empty_output(output_dir)?;
    copy_tree(
        &resolve_declared_path(manifest_dir, &manifest.baseline_dir)?,
        output_dir,
        false,
    )?;
    copy_tree(
        &resolve_declared_path(manifest_dir, &workload.overlay_dir)?,
        output_dir,
        true,
    )?;

    let transform_path = resolve_declared_path(manifest_dir, &workload.transform_path)?;
    let transform = load_transform(&transform_path, workload_id)?;
    apply_transform(output_dir, &transform)?;

    let content_digest = digest_workspace(output_dir, &manifest.metadata_excludes)?;
    if content_digest != workload.expected_content_digest {
        return Err(format!(
            "Materialized workload {workload_id} digest mismatch: expected {}, got {content_digest}",
            workload.expected_content_digest
        ));
    }

    Ok(GameFixtureReceipt {
        contract: GAME_FIXTURE_RECEIPT_CONTRACT,
        fixture_id: manifest.fixture_id,
        fixture_revision: manifest.revision,
        workload_id: workload.workload_id.clone(),
        workload_name: workload.name.clone(),
        workload_scale: workload.scale.clone(),
        output_dir: output_dir.to_path_buf(),
        content_digest,
        acceptance_path: resolve_declared_path(manifest_dir, &workload.acceptance_path)?,
        evaluator_path: resolve_declared_path(manifest_dir, &manifest.evaluator_path)?,
        browser_evaluator_path: resolve_declared_path(
            manifest_dir,
            &manifest.browser_evaluator_path,
        )?,
    })
}

fn validate_game_fixture_manifest(
    manifest_path: &Path,
    manifest: &GameFixtureManifest,
) -> Result<(), String> {
    if manifest.contract != GAME_FIXTURE_MANIFEST_CONTRACT {
        return Err(format!(
            "Game fixture manifest contract must be {GAME_FIXTURE_MANIFEST_CONTRACT}, got {}",
            manifest.contract
        ));
    }
    if manifest.fixture_id.trim().is_empty() || manifest.revision.trim().is_empty() {
        return Err("Game fixture id and revision must not be empty".to_string());
    }
    if manifest.workloads.len() != 5 {
        return Err(format!(
            "Game fixture manifest must declare exactly five workloads, got {}",
            manifest.workloads.len()
        ));
    }
    let manifest_dir = manifest_path.parent().ok_or_else(|| {
        format!(
            "Game fixture manifest has no parent directory: {}",
            manifest_path.display()
        )
    })?;
    require_directory(
        &resolve_declared_path(manifest_dir, &manifest.baseline_dir)?,
        "baseline_dir",
    )?;
    require_file(
        &resolve_declared_path(manifest_dir, &manifest.evaluator_path)?,
        "evaluator_path",
    )?;
    require_file(
        &resolve_declared_path(manifest_dir, &manifest.browser_evaluator_path)?,
        "browser_evaluator_path",
    )?;

    let mut seen = std::collections::BTreeSet::new();
    for workload in &manifest.workloads {
        if !seen.insert(workload.workload_id.as_str()) {
            return Err(format!(
                "Duplicate game benchmark workload id: {}",
                workload.workload_id
            ));
        }
        if !matches!(
            workload.workload_id.as_str(),
            "GD-01" | "GD-02" | "GD-03" | "GD-04" | "GD-05"
        ) {
            return Err(format!(
                "Unsupported game benchmark workload id: {}",
                workload.workload_id
            ));
        }
        if workload.name.trim().is_empty() || workload.scale.trim().is_empty() {
            return Err(format!(
                "Game benchmark workload {} must declare name and scale",
                workload.workload_id
            ));
        }
        require_directory(
            &resolve_declared_path(manifest_dir, &workload.overlay_dir)?,
            "workload overlay_dir",
        )?;
        require_file(
            &resolve_declared_path(manifest_dir, &workload.transform_path)?,
            "workload transform_path",
        )?;
        require_file(
            &resolve_declared_path(manifest_dir, &workload.acceptance_path)?,
            "workload acceptance_path",
        )?;
        if !is_sha256(&workload.expected_content_digest) {
            return Err(format!(
                "Game benchmark workload {} expected_content_digest must be sha256:<64 lowercase hex>",
                workload.workload_id
            ));
        }
    }
    Ok(())
}

fn load_transform(path: &Path, workload_id: &str) -> Result<GameSourceTransform, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "Failed to read game source transform {}: {error}",
            path.display()
        )
    })?;
    let transform = serde_json::from_slice::<GameSourceTransform>(&bytes).map_err(|error| {
        format!(
            "Failed to decode game source transform {}: {error}",
            path.display()
        )
    })?;
    if transform.contract != GAME_FIXTURE_TRANSFORM_CONTRACT {
        return Err(format!(
            "Game source transform contract must be {GAME_FIXTURE_TRANSFORM_CONTRACT}, got {}",
            transform.contract
        ));
    }
    if transform.workload_id != workload_id {
        return Err(format!(
            "Game source transform workload mismatch: expected {workload_id}, got {}",
            transform.workload_id
        ));
    }
    Ok(transform)
}

fn apply_transform(root: &Path, transform: &GameSourceTransform) -> Result<(), String> {
    for replacement in &transform.replacements {
        if replacement.expected_matches == 0 {
            return Err(format!(
                "Transform {} replacement expected_matches must be greater than zero",
                transform.workload_id
            ));
        }
        let path = resolve_declared_path(root, &replacement.path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "Failed to inspect transform target {}: {error}",
                path.display()
            )
        })?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "Transform target must be one regular file: {}",
                path.display()
            ));
        }
        let source = fs::read_to_string(&path).map_err(|error| {
            format!(
                "Transform target must be UTF-8 text {}: {error}",
                path.display()
            )
        })?;
        let matches = source.matches(&replacement.from).count();
        if matches != replacement.expected_matches {
            return Err(format!(
                "Transform {} expected {} match(es) in {}, got {matches}",
                transform.workload_id,
                replacement.expected_matches,
                replacement.path.display()
            ));
        }
        fs::write(&path, source.replace(&replacement.from, &replacement.to)).map_err(|error| {
            format!(
                "Failed to write transform target {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn prepare_empty_output(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(format!(
                    "Game fixture output must be a regular directory: {}",
                    path.display()
                ));
            }
            if fs::read_dir(path)
                .map_err(|error| {
                    format!(
                        "Failed to inspect game fixture output {}: {error}",
                        path.display()
                    )
                })?
                .next()
                .is_some()
            {
                return Err(format!(
                    "Game fixture output must be absent or empty: {}",
                    path.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|create_error| {
                format!(
                    "Failed to create game fixture output {}: {create_error}",
                    path.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "Failed to inspect game fixture output {}: {error}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path, overwrite: bool) -> Result<(), String> {
    require_directory(source, "fixture source")?;
    copy_tree_inner(source, destination, overwrite)
}

fn copy_tree_inner(source: &Path, destination: &Path, overwrite: bool) -> Result<(), String> {
    let mut entries = fs::read_dir(source)
        .map_err(|error| {
            format!(
                "Failed to read fixture directory {}: {error}",
                source.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "Failed to read fixture directory {}: {error}",
                source.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
            format!(
                "Failed to inspect fixture entry {}: {error}",
                source_path.display()
            )
        })?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(format!(
                "Game fixture source must not contain symbolic links: {}",
                source_path.display()
            ));
        }
        if file_type.is_dir() {
            fs::create_dir_all(&destination_path).map_err(|error| {
                format!(
                    "Failed to create fixture directory {}: {error}",
                    destination_path.display()
                )
            })?;
            copy_tree_inner(&source_path, &destination_path, overwrite)?;
        } else if file_type.is_file() {
            if destination_path.exists() && !overwrite {
                return Err(format!(
                    "Fixture destination already exists: {}",
                    destination_path.display()
                ));
            }
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "Failed to create fixture parent {}: {error}",
                        parent.display()
                    )
                })?;
            }
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "Failed to copy fixture file {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        } else {
            return Err(format!(
                "Game fixture source must contain only directories and regular files: {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn resolve_declared_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(format!(
            "Fixture path must be one non-empty relative path: {}",
            relative.display()
        ));
    }
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!(
                "Fixture path must not contain traversal or platform prefixes: {}",
                relative.display()
            ));
        }
    }
    Ok(root.join(relative))
}

fn require_directory(path: &Path, field: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("{field} {} is unavailable: {error}", path.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "{field} must be a regular directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn require_file(path: &Path, field: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("{field} {} is unavailable: {error}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "{field} must be one regular file: {}",
            path.display()
        ));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed_manifest() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("agent-token-game-v1")
            .join("manifest.json")
    }

    #[test]
    fn committed_game_fixture_materializes_all_frozen_workloads() {
        let manifest_path = committed_manifest();
        let manifest = load_game_fixture_manifest(&manifest_path).unwrap();
        let temp = tempfile::tempdir().unwrap();
        for workload in &manifest.workloads {
            let output = temp.path().join(&workload.workload_id);
            let receipt =
                materialize_game_fixture(&manifest_path, &workload.workload_id, &output).unwrap();
            assert_eq!(receipt.workload_id, workload.workload_id);
            assert_eq!(receipt.content_digest, workload.expected_content_digest);
            assert!(output.join("TASK.txt").is_file());
            assert!(output.join(".benchmark/workload.json").is_file());
            assert!(output.join("src/game.js").is_file());
            assert!(!output.join("evaluator").exists());
        }
    }

    #[test]
    fn gd05_shared_task_avoids_projected_paths_and_exposes_acceptance_contract() {
        let manifest_path = committed_manifest();
        let manifest = load_game_fixture_manifest(&manifest_path).unwrap();
        let manifest_dir = manifest_path.parent().unwrap();
        let workload = manifest
            .workloads
            .iter()
            .find(|workload| workload.workload_id == "GD-05")
            .unwrap();

        let task =
            fs::read_to_string(manifest_dir.join(&workload.overlay_dir).join("TASK.txt")).unwrap();
        assert!(task.contains("repository-root RELEASE.txt"));
        assert!(!task.contains("docs/RELEASE.txt"));
        assert!(task.contains("schemaVersion equal"));
        assert!(task.contains("volume clamped to 0..1"));
        assert!(task.contains("invalid values defaulting to"));
        assert!(task.contains("returning boolean"));
        assert!(task.contains("missing inputs false"));

        let acceptance: serde_json::Value = serde_json::from_slice(
            &fs::read(manifest_dir.join(&workload.acceptance_path)).unwrap(),
        )
        .unwrap();
        let required_paths = acceptance["required_paths"].as_array().unwrap();
        assert!(required_paths.iter().any(|path| path == "RELEASE.txt"));
        assert!(!required_paths
            .iter()
            .any(|path| path.as_str().is_some_and(|path| path.starts_with("docs/"))));

        let evaluator = fs::read_to_string(manifest_dir.join(&manifest.evaluator_path)).unwrap();
        assert!(evaluator.contains("candidatePath(\"RELEASE.txt\")"));
        assert!(!evaluator.contains("candidatePath(\"docs/RELEASE.txt\")"));
        assert!(evaluator.contains(r"/(?:scripts\/release-check\.mjs|npm run release-check)/"));
    }

    #[test]
    fn gd05_browser_probe_measures_focus_loss_and_recovery_behavior() {
        let manifest_path = committed_manifest();
        let manifest = load_game_fixture_manifest(&manifest_path).unwrap();
        let manifest_dir = manifest_path.parent().unwrap();
        let evaluator =
            fs::read_to_string(manifest_dir.join(&manifest.browser_evaluator_path)).unwrap();
        let focus_state = evaluator
            .find("Object.defineProperty(document, \"hasFocus\"")
            .unwrap();
        let blur_event = evaluator
            .find("window.dispatchEvent(new Event(\"blur\"))")
            .unwrap();
        assert!(focus_state < blur_event);
        assert!(evaluator.contains("value: () => false"));
        assert!(evaluator.contains("duringFocusLossTime === beforeFocusLossTime"));
        assert!(evaluator.contains("focusRecoveryDeltaMs > 0"));
        assert!(evaluator.contains("focusRecoveryDeltaMs <= 200"));
        assert!(!evaluator.contains("focusLossPaused"));
        assert!(!evaluator.contains("state?.().paused === true"));
    }

    #[test]
    fn game_fixture_refuses_unknown_workload_and_nonempty_output() {
        let manifest_path = committed_manifest();
        let temp = tempfile::tempdir().unwrap();
        let unknown = temp.path().join("unknown");
        let error = materialize_game_fixture(&manifest_path, "GD-99", &unknown).unwrap_err();
        assert!(error.contains("Unknown game benchmark workload"));

        let occupied = temp.path().join("occupied");
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("keep.txt"), "keep").unwrap();
        let error = materialize_game_fixture(&manifest_path, "GD-01", &occupied).unwrap_err();
        assert!(error.contains("absent or empty"));
        assert_eq!(
            fs::read_to_string(occupied.join("keep.txt")).unwrap(),
            "keep"
        );
    }
}
