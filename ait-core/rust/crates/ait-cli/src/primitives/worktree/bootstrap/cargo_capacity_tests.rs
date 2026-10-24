use super::*;
use std::os::unix::fs::symlink;
use tempfile::{tempdir, TempDir};

fn fixture(mounted: bool) -> (TempDir, PathBuf, PathBuf) {
    let temp = tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let repo = base.join("repo");
    let ram = base.join("ram");
    fs::create_dir_all(repo.join(APP_DIR)).unwrap();
    fs::write(repo.join(".ait/config.json"), json!({"repo_name":"fixture", "task_worktree":{"memory_root":{"kind":"macos_ram_volume","root":ram}}}).to_string()).unwrap();
    if mounted {
        fs::create_dir(&ram).unwrap();
    }
    (temp, repo, ram)
}

#[test]
fn zero_gc_budget_keeps_default_ram_floor_and_positive_limits_only_raise_it() {
    for value in [None, Some("0"), Some("1"), Some("invalid")] {
        assert_eq!(cargo_ram_headroom(value), DEFAULT_CARGO_RAM_HEADROOM);
    }
    assert_eq!(
        cargo_ram_headroom(Some("8589934592")),
        8 * 1024 * 1024 * 1024
    );
}

#[test]
fn missing_root_uses_ram_only_with_available_headroom() {
    for capacity in [
        Some(DEFAULT_CARGO_RAM_HEADROOM),
        Some(DEFAULT_CARGO_RAM_HEADROOM - 1),
        None,
    ] {
        let (_temp, repo, ram) = fixture(true);
        let selection =
            cargo_build_location_with_space(&repo, DEFAULT_CARGO_RAM_HEADROOM, |probe| {
                assert_eq!(probe, ram);
                capacity
            });
        let selected = ensure_repository_cargo_build_selection(&repo, selection).unwrap();
        if capacity == Some(DEFAULT_CARGO_RAM_HEADROOM) {
            assert_eq!(selected, ram.join("ait-runtime/cargo-build/fixture"));
            assert!(repo.join(".ait/cargo-build").is_symlink());
        } else {
            assert_eq!(selected, repo.join(".ait/cargo-build-ssd"));
            assert!(!repo.join(".ait/cargo-build").exists());
            assert!(!ram.join("ait-runtime").exists());
        }
    }
}

#[test]
fn shared_link_capacity_is_measured_on_its_actual_target_filesystem() {
    let (temp, repo, _ram) = fixture(true);
    let target = temp.path().canonicalize().unwrap().join("previous-volume");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), "other build").unwrap();
    let shared = repo.join(".ait/cargo-build");
    symlink(&target, &shared).unwrap();
    let selected = cargo_build_location_with_space(&repo, DEFAULT_CARGO_RAM_HEADROOM, |probe| {
        assert_eq!(probe, target);
        Some(1)
    });
    ensure_repository_cargo_build_selection(&repo, selected).unwrap();
    assert_eq!(fs::read_link(&shared).unwrap(), target);
    assert!(target.join("keep").is_file());
}

#[test]
fn existing_ssd_root_survives_unavailable_ram_and_capacity_inspection() {
    let (_temp, repo, _ram) = fixture(false);
    let existing = repo.join(".ait/cargo-build");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), "existing SSD cache").unwrap();
    let selection = cargo_build_location_with_space(&repo, DEFAULT_CARGO_RAM_HEADROOM, |_| {
        panic!("SSD does not need RAM admission")
    });
    assert_eq!(
        ensure_repository_cargo_build_selection(&repo, selection).unwrap(),
        existing
    );
    assert!(existing.join("keep").is_file());
    assert!(!existing.is_symlink());
}

#[test]
fn pressure_and_missing_mount_preserve_the_shared_ram_link_and_other_builds() {
    for mounted in [false, true] {
        let (_temp, repo, ram) = fixture(mounted);
        let physical = ram.join("ait-runtime/cargo-build/fixture");
        if mounted {
            fs::create_dir_all(&physical).unwrap();
            fs::write(physical.join("other-build"), "keep").unwrap();
        }
        let shared = repo.join(".ait/cargo-build");
        symlink(&physical, &shared).unwrap();
        let selection =
            cargo_build_location_with_space(&repo, DEFAULT_CARGO_RAM_HEADROOM, |_| Some(1));
        assert_eq!(
            ensure_repository_cargo_build_selection(&repo, selection).unwrap(),
            repo.join(".ait/cargo-build-ssd")
        );
        assert_eq!(fs::read_link(&shared).unwrap(), physical);
        if mounted {
            assert_eq!(
                fs::read_to_string(physical.join("other-build")).unwrap(),
                "keep"
            );
        } else {
            assert!(!ram.exists());
        }
    }
}

#[test]
fn fallback_rejects_aliases_foreign_namespaces_and_configured_ram_containment() {
    for conflict in ["root_link", "namespace_link", "ram_containment"] {
        let (_temp, repo, _ram) = fixture(false);
        let foreign = repo.join("foreign");
        fs::create_dir(&foreign).unwrap();
        fs::write(foreign.join("keep"), "foreign").unwrap();
        let fallback = repo.join(".ait/cargo-build-ssd");
        match conflict {
            "root_link" => symlink(&foreign, &fallback).unwrap(),
            "namespace_link" => {
                fs::create_dir(&fallback).unwrap();
                symlink(&foreign, fallback.join("task-workspaces")).unwrap();
            }
            _ => fs::write(
                repo.join(".ait/config.json"),
                json!({"repo_name":"fixture","task_worktree":{"memory_root":{"root":repo}}})
                    .to_string(),
            )
            .unwrap(),
        }
        let selection =
            cargo_build_location_with_space(&repo, DEFAULT_CARGO_RAM_HEADROOM, |_| None);
        assert!(
            ensure_repository_cargo_build_selection(&repo, selection).is_err(),
            "{conflict}"
        );
        assert_eq!(fs::read_to_string(foreign.join("keep")).unwrap(), "foreign");
    }
}

#[test]
fn cleanup_does_not_adopt_an_unregistered_ram_directory_from_its_name() {
    let (_temp, repo, ram) = fixture(true);
    let name = "task-one";
    let unknown = ram
        .join("ait-runtime/cargo-build/fixture/task-workspaces")
        .join(name);
    fs::create_dir_all(&unknown).unwrap();
    fs::write(unknown.join("keep"), "unregistered cache").unwrap();
    let worktree = repo.join(name);
    fs::create_dir(&worktree).unwrap();
    symlink(repo.join(APP_DIR), worktree.join(APP_DIR)).unwrap();
    fs::write(
        worktree.join(WORKTREE_CONFIG_NAME),
        json!({"worktree_name":name}).to_string(),
    )
    .unwrap();
    let owned = repo.join(".ait/cargo-build-ssd/task-workspaces").join(name);
    fs::create_dir_all(&owned).unwrap();
    fs::write(owned.join("cache"), "owned fallback cache").unwrap();
    cleanup_registered_worktree_cargo_build_dir(&worktree, name).unwrap();
    assert!(!owned.exists());
    assert_eq!(
        fs::read_to_string(unknown.join("keep")).unwrap(),
        "unregistered cache"
    );
}

#[test]
fn canonical_and_task_fallback_projections_keep_the_final_ssd_targets() {
    let (_temp, repo, _ram) = fixture(false);
    let fallback = repo.join(".ait/cargo-build-ssd");
    assert_eq!(worktree_cargo_build_dir(&repo), fallback.join("canonical"));
    assert_eq!(
        worktree_cargo_target_dir(&repo),
        repo.join(".ait/cargo-target")
    );
    for name in ["task-one", "task-two"] {
        let worktree = repo.join(name);
        fs::create_dir(&worktree).unwrap();
        symlink(repo.join(APP_DIR), worktree.join(APP_DIR)).unwrap();
        fs::write(
            worktree.join(WORKTREE_CONFIG_NAME),
            json!({"worktree_name":name}).to_string(),
        )
        .unwrap();
        assert_eq!(
            worktree_cargo_build_dir(&worktree),
            fallback.join("task-workspaces").join(name)
        );
        let current = generated_worktree_cargo_config_text(&worktree);
        let old = current.replace(
            &fallback.to_string_lossy().to_string(),
            &primary_cargo_build_root(&worktree).to_string_lossy(),
        );
        assert_eq!(
            upgrade_generated_worktree_cargo_config_text(&worktree, &old).unwrap(),
            current
        );
        let previous_ssd = current.replace(
            &fallback.to_string_lossy().to_string(),
            &repo.join(".ait/cargo-build").to_string_lossy(),
        );
        assert_eq!(
            upgrade_generated_worktree_cargo_config_text(&worktree, &previous_ssd).unwrap(),
            current
        );
        assert_eq!(
            worktree_cargo_target_dir(&worktree),
            repo.join(".ait/cargo-target/task-workspaces").join(name)
        );
    }
}
