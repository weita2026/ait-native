use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::SyntheticFixtureRecipe;
use crate::statistics::DeterministicRng;
use crate::FIXTURE_CONTRACT;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyntheticFixtureReceipt {
    pub contract: &'static str,
    pub fixture_id: String,
    pub revision: String,
    pub scale: String,
    pub root: String,
    pub content_digest: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub declared_history_nodes: u64,
    pub seed: u64,
    pub features: Vec<String>,
}

pub fn create_synthetic_fixture(
    recipe: &SyntheticFixtureRecipe,
    root: &Path,
) -> Result<SyntheticFixtureReceipt, String> {
    validate_recipe(recipe)?;
    if root.exists() {
        let mut entries = fs::read_dir(root).map_err(|error| {
            format!("Failed to inspect fixture root {}: {error}", root.display())
        })?;
        if entries.next().is_some() {
            return Err(format!(
                "Fixture root {} must be absent or empty; existing data is never overwritten",
                root.display()
            ));
        }
    } else {
        fs::create_dir_all(root).map_err(|error| {
            format!("Failed to create fixture root {}: {error}", root.display())
        })?;
    }

    let file_count = usize::try_from(recipe.file_count)
        .map_err(|_| "fixture file_count exceeds this platform's address space".to_string())?;
    let base_size = recipe.total_bytes / recipe.file_count;
    let remainder = recipe.total_bytes % recipe.file_count;
    let mut rng = DeterministicRng::new(recipe.seed);

    for index in 0..file_count {
        let ignored = index * 100 / file_count.max(1) < usize::from(recipe.ignored_percent);
        let binary = index * 100 / file_count.max(1) < usize::from(recipe.binary_percent);
        let relative = fixture_relative_path(index, recipe.max_depth, ignored, binary);
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create fixture directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let length = base_size + u64::from((index as u64) < remainder);
        write_fixture_file(&path, length, binary, index, &mut rng)?;
    }

    let excludes = BTreeSet::new();
    let content_digest = digest_workspace_with_set(root, &excludes)?;
    let (actual_files, actual_bytes) = workspace_profile_with_set(root, &excludes)?;
    if actual_files != recipe.file_count || actual_bytes != recipe.total_bytes {
        return Err(format!(
            "Fixture generation invariant failed: expected {} files/{} bytes, got {actual_files}/{actual_bytes}",
            recipe.file_count, recipe.total_bytes
        ));
    }
    Ok(SyntheticFixtureReceipt {
        contract: FIXTURE_CONTRACT,
        fixture_id: recipe.fixture_id.clone(),
        revision: recipe.revision.clone(),
        scale: recipe.scale.as_str().to_string(),
        root: root.display().to_string(),
        content_digest,
        file_count: actual_files,
        total_bytes: actual_bytes,
        declared_history_nodes: recipe.history_nodes,
        seed: recipe.seed,
        features: recipe.features.clone(),
    })
}

pub fn digest_workspace(root: &Path, excludes: &[String]) -> Result<String, String> {
    let excludes = excludes.iter().cloned().collect::<BTreeSet<_>>();
    digest_workspace_with_set(root, &excludes)
}

pub fn profile_workspace(root: &Path, excludes: &[String]) -> Result<(u64, u64), String> {
    let excludes = excludes.iter().cloned().collect::<BTreeSet<_>>();
    workspace_profile_with_set(root, &excludes)
}

fn digest_workspace_with_set(root: &Path, excludes: &BTreeSet<String>) -> Result<String, String> {
    if !root.is_dir() {
        return Err(format!(
            "Fixture workspace is not a directory: {}",
            root.display()
        ));
    }
    let mut entries = Vec::new();
    collect_entries(root, root, excludes, &mut entries)?;
    entries.sort();
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    for relative in entries {
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Failed to stat {}: {error}", path.display()))?;
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)
                .map_err(|error| format!("Failed to read symlink {}: {error}", path.display()))?;
            digest.update(b"link\0");
            digest.update(relative_text.as_bytes());
            digest.update(b"\0");
            digest.update(target.to_string_lossy().as_bytes());
            digest.update(b"\0");
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        digest.update(b"file\0");
        digest.update(relative_text.as_bytes());
        digest.update(b"\0");
        digest.update(metadata.len().to_le_bytes());
        let mut file = File::open(&path)
            .map_err(|error| format!("Failed to read fixture file {}: {error}", path.display()))?;
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| format!("Failed to hash {}: {error}", path.display()))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn workspace_profile_with_set(
    root: &Path,
    excludes: &BTreeSet<String>,
) -> Result<(u64, u64), String> {
    let mut entries = Vec::new();
    collect_entries(root, root, excludes, &mut entries)?;
    let mut files = 0_u64;
    let mut bytes = 0_u64;
    for relative in entries {
        let metadata = fs::symlink_metadata(root.join(relative))
            .map_err(|error| format!("Failed to profile fixture: {error}"))?;
        if metadata.is_file() {
            files += 1;
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    Ok((files, bytes))
}

fn collect_entries(
    root: &Path,
    directory: &Path,
    excludes: &BTreeSet<String>,
    entries: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut children = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "Failed to read fixture directory {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to enumerate {}: {error}", directory.display()))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let name = child.file_name().to_string_lossy().to_string();
        if excludes.contains(&name) {
            continue;
        }
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| format!("Fixture entry escaped root: {}", path.display()))?
            .to_path_buf();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Failed to stat {}: {error}", path.display()))?;
        if metadata.is_dir() {
            collect_entries(root, &path, excludes, entries)?;
        } else {
            entries.push(relative);
        }
    }
    Ok(())
}

fn validate_recipe(recipe: &SyntheticFixtureRecipe) -> Result<(), String> {
    if recipe.contract != FIXTURE_CONTRACT {
        return Err(format!(
            "fixture recipe contract must be {FIXTURE_CONTRACT}"
        ));
    }
    if recipe.fixture_id.trim().is_empty() || recipe.revision.trim().is_empty() {
        return Err("fixture_id and revision must not be empty".to_string());
    }
    if recipe.file_count == 0 || recipe.total_bytes < recipe.file_count {
        return Err(
            "fixture total_bytes must be at least file_count and both must be positive".to_string(),
        );
    }
    if recipe.max_depth == 0 || recipe.max_depth > 64 {
        return Err("fixture max_depth must be between 1 and 64".to_string());
    }
    if recipe.binary_percent > 100 || recipe.ignored_percent > 100 {
        return Err("fixture percentages must be between 0 and 100".to_string());
    }
    Ok(())
}

fn fixture_relative_path(index: usize, depth: usize, ignored: bool, binary: bool) -> PathBuf {
    let mut path = PathBuf::new();
    path.push(if ignored { "ignored" } else { "tracked" });
    for level in 0..depth {
        path.push(format!("d{:02x}", (index + level * 17) % 251));
    }
    path.push(format!(
        "file-{index:08}.{}",
        if binary { "bin" } else { "txt" }
    ));
    path
}

fn write_fixture_file(
    path: &Path,
    length: u64,
    binary: bool,
    index: usize,
    rng: &mut DeterministicRng,
) -> Result<(), String> {
    let mut file = File::create(path)
        .map_err(|error| format!("Failed to create fixture file {}: {error}", path.display()))?;
    let mut remaining = length;
    let mut buffer = vec![0_u8; usize::try_from(length.min(1024 * 1024)).unwrap_or(1024 * 1024)];
    while remaining > 0 {
        let count = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        if binary {
            for chunk in buffer[..count].chunks_mut(8) {
                let bytes = rng.next_u64().to_le_bytes();
                chunk.copy_from_slice(&bytes[..chunk.len()]);
            }
        } else {
            let pattern = format!("fixture={index:08} deterministic benchmark payload\n");
            for (offset, byte) in buffer[..count].iter_mut().enumerate() {
                *byte = pattern.as_bytes()[offset % pattern.len()];
            }
        }
        file.write_all(&buffer[..count])
            .map_err(|error| format!("Failed to write fixture file {}: {error}", path.display()))?;
        remaining -= count as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FixtureScale;

    #[test]
    fn fixture_generation_is_deterministic_and_refuses_overwrite() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let recipe = SyntheticFixtureRecipe {
            contract: FIXTURE_CONTRACT.to_string(),
            fixture_id: "tiny-v1".to_string(),
            revision: "1".to_string(),
            scale: FixtureScale::Small,
            seed: 7,
            file_count: 12,
            total_bytes: 1_234,
            history_nodes: 10,
            max_depth: 3,
            binary_percent: 25,
            ignored_percent: 20,
            features: vec!["small_text".to_string(), "large_binary".to_string()],
        };
        let receipt = create_synthetic_fixture(&recipe, first.path()).unwrap();
        let repeated = create_synthetic_fixture(&recipe, second.path()).unwrap();
        assert_eq!(receipt.content_digest, repeated.content_digest);
        assert_eq!(receipt.file_count, 12);
        assert_eq!(receipt.total_bytes, 1_234);
        assert!(create_synthetic_fixture(&recipe, first.path()).is_err());
        assert_eq!(
            digest_workspace(first.path(), &[]).unwrap(),
            receipt.content_digest
        );
    }

    #[test]
    fn metadata_exclusions_preserve_payload_equivalence() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("payload.txt"), "same").unwrap();
        fs::create_dir(root.path().join(".git")).unwrap();
        fs::write(root.path().join(".git/config"), "subject-specific").unwrap();
        let before = digest_workspace(root.path(), &[".git".to_string()]).unwrap();
        fs::write(root.path().join(".git/config"), "changed metadata").unwrap();
        let after = digest_workspace(root.path(), &[".git".to_string()]).unwrap();
        assert_eq!(before, after);
    }
}
