use super::*;
use serde_json::{json, Map as JsonMap};
use std::env;
use std::fs;
use std::path::PathBuf;

#[test]
fn persistent_runtime_path_cannot_be_admitted_as_ci_ram() {
    let persistent = PathBuf::from("/persistent/ait-runtime");
    let candidate = persistent.join("ci-tmp");
    let error = validate_ci_ram_root_path_boundary(
        &candidate,
        "AIT_NATIVE_SERVER_RAM_SHARD_ROOT/parent",
        &[("AIT_RUNTIME_ROOT".to_string(), persistent)],
    )
    .expect_err("persistent runtime descendants must fail closed");
    assert!(error.contains("inside persistent authority AIT_RUNTIME_ROOT"));
}

#[cfg(unix)]
#[test]
fn system_filesystem_cannot_be_admitted_as_ci_ram() {
    let root = PathBuf::from("/");
    let error = validate_ci_ram_root_device_boundary(&root, "test", &[])
        .expect_err("the system filesystem must not masquerade as RAM");
    assert!(error.contains("persistent system filesystem"));
}

#[test]
fn default_reclaim_target_is_the_configured_hard_minimum() {
    let gib = 1024_u64.pow(3);

    assert_eq!(default_ci_ram_reclaim_target_bytes(gib), gib);
    assert_eq!(default_ci_ram_reclaim_target_bytes(u64::MAX), u64::MAX);
}

#[test]
fn explicit_managed_paths_preserve_rust_ownership_from_adjacent_manifest() {
    let root = env::temp_dir().join(format!(
        "ait-server-ci-runtime-owned-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let namespace = root.join("patchset-ci");
    let create_request = JsonMap::from_iter([
        ("ci_temp_root".to_string(), json!(path_string(&namespace))),
        ("runtime_scope_root".to_string(), json!(path_string(&root))),
    ]);
    let created = ci_runtime_paths_from_request(&create_request, "patchset-ci", "P-1")
        .expect("managed paths should be created");
    assert!(created.rust_owned);

    let explicit_request = JsonMap::from_iter([
        (
            "workspace_path".to_string(),
            json!(path_string(&created.workspace_path)),
        ),
        (
            "output_dir".to_string(),
            json!(path_string(&created.output_dir)),
        ),
        (
            "temp_dir".to_string(),
            json!(path_string(&created.temp_dir)),
        ),
    ]);
    let reparsed = ci_runtime_paths_from_request(&explicit_request, "patchset-ci", "P-1")
        .expect("explicit managed paths should parse");
    assert!(reparsed.rust_owned);

    let mut mismatched_request = explicit_request;
    mismatched_request.insert(
        "output_dir".to_string(),
        json!(path_string(&root.join("external-output"))),
    );
    let mismatched = ci_runtime_paths_from_request(&mismatched_request, "patchset-ci", "P-1")
        .expect("mismatched explicit paths should still parse as external");
    assert!(!mismatched.rust_owned);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn pruned_contract_shaped_paths_can_be_reinitialized_only_under_trusted_ram_root() {
    let root = env::temp_dir().join(format!(
        "ait-server-ci-runtime-readopt-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let namespace = root.join("ci-runs/patchset-ci");
    fs::create_dir_all(&namespace).unwrap();
    let base = namespace.join("ait-patchset-ci-p-1-123-pid456-seq7");
    let workspace = base.join("workspace");
    let output = base.join("output");
    let temp = workspace.join(".tmp");
    let request =
        JsonMap::from_iter([("runtime_scope_root".to_string(), json!(path_string(&root)))]);

    assert!(reinitialize_pruned_managed_runtime_paths(
        &request,
        &root,
        &workspace,
        &output,
        &temp,
        "patchset-ci",
        "P-1",
    )
    .unwrap());
    assert!(base.join("ci-runtime.json").is_file());
    assert!(temp.is_dir());

    let external_base = root.join("outside/ait-patchset-ci-p-1-123-pid456-seq7");
    assert!(!reinitialize_pruned_managed_runtime_paths(
        &request,
        &root,
        &external_base.join("workspace"),
        &external_base.join("output"),
        &external_base.join("workspace/.tmp"),
        "patchset-ci",
        "P-1",
    )
    .unwrap());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn pressure_reclamation_prunes_completed_managed_runs_before_warm_cargo_cache() {
    let root = env::temp_dir().join(format!(
        "ait-server-ci-pressure-order-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let namespace = root.join("ci-runs/patchset-ci");
    let request = JsonMap::from_iter([
        ("ci_temp_root".to_string(), json!(path_string(&namespace))),
        ("runtime_scope_root".to_string(), json!(path_string(&root))),
    ]);
    let paths = ci_runtime_paths_from_request(&request, "patchset-ci", "P-HEADROOM")
        .expect("managed CI run should be created");
    let run_base = paths.workspace_path.parent().unwrap().to_path_buf();
    fs::remove_dir_all(&paths.workspace_path).unwrap();
    fs::write(
        paths.output_dir.join("completed.log"),
        b"durable result stored",
    )
    .unwrap();
    let unmanaged_legacy = namespace.join("ait-patchset-ci-legacy-pid999999");
    fs::create_dir_all(&unmanaged_legacy).unwrap();
    fs::write(unmanaged_legacy.join("retained.log"), b"not manifest owned").unwrap();

    let incremental = root.join("cargo-build/ait-core/release/incremental");
    fs::create_dir_all(&incremental).unwrap();
    fs::write(incremental.join("warm.bin"), b"warm cache").unwrap();

    let evidence = reclaim_ci_ram_capacity_with_available(&root, 100, |_| {
        Ok(if run_base.exists() { 50 } else { 150 })
    })
    .unwrap();

    assert!(!run_base.exists());
    assert!(unmanaged_legacy.join("retained.log").is_file());
    assert!(incremental.join("warm.bin").is_file());
    assert_eq!(
        evidence["runtime_temp_pressure_prune"]["removed_run_base_count"],
        json!(1)
    );
    assert_eq!(evidence["removed_incremental_count"], json!(0));
    assert_eq!(
        evidence["reclamation_order"],
        json!([
            "completed_managed_ci_runs",
            "cargo_incremental",
            "idle_cargo_build_profiles"
        ])
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn low_capacity_reclaims_only_unlocked_cargo_incremental_state() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-reclaim-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let profile = root.join("cargo-target/ait-core/release");
    let incremental = profile.join("incremental");
    let deps = profile.join("deps");
    fs::create_dir_all(&incremental).unwrap();
    fs::create_dir_all(&deps).unwrap();
    fs::write(incremental.join("state.bin"), b"rebuildable").unwrap();
    fs::write(deps.join("libait_core.rlib"), b"preserve").unwrap();
    for name in CARGO_PROFILE_LOCK_NAMES {
        fs::write(profile.join(name), b"").unwrap();
    }

    let evidence = reclaim_cargo_incremental_cache_with_available(&root, 100, |_| {
        Ok(if incremental.exists() { 0 } else { 100 })
    })
    .unwrap();

    assert!(!incremental.exists());
    assert!(deps.join("libait_core.rlib").is_file());
    assert_eq!(evidence["removed_incremental_count"], json!(1));
    assert_eq!(evidence["available_after"], json!(100));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn low_capacity_reclaims_idle_build_profiles_without_touching_final_targets() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-build-reclaim-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let build_profile = root.join("cargo-build/ait-server/release");
    let deps = build_profile.join("deps");
    let final_binary = root.join("cargo-target/ait-server/release/ait-server");
    fs::create_dir_all(&deps).unwrap();
    fs::write(deps.join("libait_server.rlib"), b"rebuildable").unwrap();
    fs::create_dir_all(final_binary.parent().unwrap()).unwrap();
    fs::write(&final_binary, b"final").unwrap();
    for name in CARGO_PROFILE_LOCK_NAMES {
        fs::write(build_profile.join(name), b"").unwrap();
    }

    let evidence = reclaim_cargo_incremental_cache_with_available(&root, 100, |_| {
        Ok(if deps.exists() { 0 } else { 100 })
    })
    .unwrap();

    assert!(!deps.exists());
    assert!(final_binary.is_file());
    for name in CARGO_PROFILE_LOCK_NAMES {
        assert!(build_profile.join(name).is_file());
    }
    assert_eq!(evidence["removed_build_profile_count"], json!(1));
    assert_eq!(evidence["available_after"], json!(100));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn active_build_profile_lock_prevents_capacity_reclamation() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-build-reclaim-locked-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let build_profile = root.join("cargo-build/ait-server/release");
    let deps = build_profile.join("deps");
    fs::create_dir_all(&deps).unwrap();
    fs::write(deps.join("libait_server.rlib"), b"active").unwrap();
    for name in CARGO_PROFILE_LOCK_NAMES {
        fs::write(build_profile.join(name), b"").unwrap();
    }
    let active_locks = try_lock_cargo_profile(&build_profile)
        .unwrap()
        .expect("test should hold Cargo-compatible locks");

    let evidence = reclaim_cargo_incremental_cache_with_available(&root, 100, |_| Ok(0)).unwrap();

    assert!(deps.join("libait_server.rlib").is_file());
    assert_eq!(evidence["removed_build_profile_count"], json!(0));
    assert_eq!(
        evidence["skipped_locked"],
        json!([path_string(&build_profile)])
    );
    drop(active_locks);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn active_ci_build_lease_prevents_idle_profile_reclamation_between_cargo_commands() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-build-reclaim-leased-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let build_dir = root.join("cargo-build/ait-server/ci/workspaces/{workspace-path-hash}");
    let build_profile = root.join("cargo-build/ait-server/ci/workspaces/ab/hash/release");
    let deps = build_profile.join("deps");
    fs::create_dir_all(&deps).unwrap();
    fs::write(deps.join("ci-test-executable"), b"active between commands").unwrap();
    for name in CARGO_PROFILE_LOCK_NAMES {
        fs::write(build_profile.join(name), b"").unwrap();
    }
    let active_lease = acquire_cargo_build_dir_lease(&build_dir)
        .expect("the CI run should hold an exclusive build-dir lease");

    let evidence = reclaim_cargo_incremental_cache_with_available(&root, 100, |_| Ok(0)).unwrap();

    assert!(deps.join("ci-test-executable").is_file());
    assert_eq!(evidence["removed_build_profile_count"], json!(0));
    assert_eq!(
        evidence["skipped_leased"],
        json!([path_string(&build_profile)])
    );
    drop(active_lease);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn same_cargo_build_cache_users_serialize_on_exclusive_lease() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-build-serialize-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let build_dir = root.join("cargo-build/ait-server/ci/workspaces/{workspace-path-hash}");
    let first = acquire_cargo_build_dir_lease(&build_dir).expect("first exclusive lease");
    let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let second_build_dir = build_dir.clone();
    let waiter = std::thread::spawn(move || {
        attempted_tx.send(()).expect("signal second attempt");
        let result = acquire_cargo_build_dir_lease(&second_build_dir).map(drop);
        result_tx.send(result).expect("send second lease result");
    });

    attempted_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("second lease attempt started");
    assert!(matches!(
        result_rx.recv_timeout(std::time::Duration::from_millis(100)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    drop(first);
    result_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("second lease unblocked")
        .expect("second exclusive lease");
    waiter.join().expect("lease waiter");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn active_cargo_profile_lock_prevents_incremental_reclamation() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-reclaim-locked-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let profile = root.join("cargo-target/ait-server/release");
    let incremental = profile.join("incremental");
    fs::create_dir_all(&incremental).unwrap();
    fs::write(incremental.join("state.bin"), b"active").unwrap();
    for name in CARGO_PROFILE_LOCK_NAMES {
        fs::write(profile.join(name), b"").unwrap();
    }
    let active_locks = try_lock_cargo_profile(&profile)
        .unwrap()
        .expect("test should hold Cargo-compatible locks");

    let evidence = reclaim_cargo_incremental_cache_with_available(&root, 100, |_| Ok(0)).unwrap();

    assert!(incremental.is_dir());
    assert_eq!(evidence["removed_incremental_count"], json!(0));
    assert_eq!(
        evidence["skipped_locked"],
        json!([path_string(&incremental)])
    );
    drop(active_locks);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn terminal_prune_keeps_newest_incremental_generation_and_non_incremental_state() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-generation-prune-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let profile = root.join("cargo-target/ait-core/release");
    let crate_incremental = profile.join("incremental/ait_core-hash");
    let older = crate_incremental.join("s-0000000000-old-hash");
    let newest = crate_incremental.join("s-9999999999-new-hash");
    let deps = profile.join("deps");
    fs::create_dir_all(&older).unwrap();
    fs::write(older.join("dep-graph.bin"), b"old").unwrap();
    fs::write(crate_incremental.join("s-0000000000-old.lock"), b"").unwrap();
    fs::create_dir_all(&newest).unwrap();
    fs::write(newest.join("dep-graph.bin"), b"new").unwrap();
    fs::write(crate_incremental.join("s-9999999999-new.lock"), b"").unwrap();
    fs::create_dir_all(&deps).unwrap();
    fs::write(deps.join("libait_core.rlib"), b"preserve").unwrap();
    for name in CARGO_PROFILE_LOCK_NAMES {
        fs::write(profile.join(name), b"").unwrap();
    }

    let evidence = prune_obsolete_cargo_incremental_generations_in(&root).unwrap();

    assert!(!older.exists());
    assert!(!crate_incremental.join("s-0000000000-old.lock").exists());
    assert!(newest.join("dep-graph.bin").is_file());
    assert!(crate_incremental.join("s-9999999999-new.lock").is_file());
    assert!(deps.join("libait_core.rlib").is_file());
    assert_eq!(evidence["removed_generation_count"], json!(1));
    assert_eq!(evidence["preserved_generation_count"], json!(1));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn terminal_prune_also_covers_separated_cargo_build_root() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-build-generation-prune-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let profile = root.join("cargo-build/ait-core/release");
    let crate_incremental = profile.join("incremental/ait_core-hash");
    let older = crate_incremental.join("s-0000000000-old-hash");
    let newest = crate_incremental.join("s-9999999999-new-hash");
    fs::create_dir_all(&older).unwrap();
    fs::write(older.join("dep-graph.bin"), b"old").unwrap();
    fs::create_dir_all(&newest).unwrap();
    fs::write(newest.join("dep-graph.bin"), b"new").unwrap();
    for name in CARGO_PROFILE_LOCK_NAMES {
        fs::write(profile.join(name), b"").unwrap();
    }

    let evidence = prune_obsolete_cargo_incremental_generations_in(&root).unwrap();

    assert!(!older.exists());
    assert!(newest.join("dep-graph.bin").is_file());
    assert_eq!(evidence["removed_generation_count"], json!(1));
    assert_eq!(
        evidence["cargo_build_root"],
        json!(path_string(&root.join("cargo-build")))
    );

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn active_profile_lock_skips_terminal_incremental_generation_prune() {
    let root = env::temp_dir().join(format!(
        "ait-server-cargo-generation-prune-locked-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let profile = root.join("cargo-target/ait-server/release");
    let crate_incremental = profile.join("incremental/ait_server-hash");
    let older = crate_incremental.join("s-0000000000-old-hash");
    let newest = crate_incremental.join("s-9999999999-new-hash");
    fs::create_dir_all(&older).unwrap();
    fs::create_dir_all(&newest).unwrap();
    for name in CARGO_PROFILE_LOCK_NAMES {
        fs::write(profile.join(name), b"").unwrap();
    }
    let active_locks = try_lock_cargo_profile(&profile)
        .unwrap()
        .expect("test should hold Cargo-compatible locks");

    let evidence = prune_obsolete_cargo_incremental_generations_in(&root).unwrap();

    assert!(older.is_dir());
    assert!(newest.is_dir());
    assert_eq!(evidence["removed_generation_count"], json!(0));
    assert_eq!(
        evidence["skipped_locked"],
        json!([path_string(&profile.join("incremental"))])
    );

    drop(active_locks);
    let _ = fs::remove_dir_all(root);
}
