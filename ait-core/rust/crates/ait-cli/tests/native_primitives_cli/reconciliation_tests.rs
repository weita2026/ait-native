fn reconciliation_tree_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(base: &Path, current: &Path, output: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let relative = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.file_type().is_symlink() {
                output.insert(
                    relative,
                    fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .as_bytes()
                        .to_vec(),
                );
            } else if metadata.is_dir() {
                visit(base, &path, output);
            } else if metadata.is_file() {
                output.insert(relative, fs::read(&path).unwrap());
            }
        }
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

fn configure_local_reconciliation_repo(root: &Path) {
    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "workflow_default_scope": "local",
  "task_default_scope": "local",
  "sprint": "off",
  "plan_task_binding": {"mode": "off"},
  "user_name": "Fixture User",
  "user_email": "fixture@example.com"
}"#,
    );
}

fn reconciliation_receipt_paths(root: &Path) -> Vec<PathBuf> {
    let receipts = root.join(".ait/reconciliation/v1/receipts");
    if !receipts.is_dir() {
        return Vec::new();
    }
    let mut paths = fs::read_dir(receipts)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn rewrite_receipt_as_started(path: &Path) {
    let mut receipt = parse_json_file(path);
    let object = receipt.as_object_mut().expect("receipt object");
    object.insert("state".to_string(), json!("started"));
    object.insert("completed_at".to_string(), JsonValue::Null);
    object.insert("result".to_string(), JsonValue::Null);
    object.insert("remaining_findings".to_string(), JsonValue::Null);
    write_file(path, &format!("{}\n", encode_json_pretty(&receipt)));
}

#[test]
fn native_workflow_reconcile_inventory_is_read_only_and_stable() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "workflow_default_scope": "local",
  "task_default_scope": "local",
  "sprint": "off",
  "plan_task_binding": {"mode": "off"},
  "user_name": "Fixture User",
  "user_email": "fixture@example.com"
}"#,
    );
    let started = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Reconciliation fixture",
            "--intent",
            "prove inventory does not mutate repository state",
            "--json",
        ],
    );
    let worktree_path = PathBuf::from(
        started["worktree"]["open_path"]
            .as_str()
            .or_else(|| started["worktree"]["path"].as_str())
            .expect("task worktree path"),
    );
    let registered_path = PathBuf::from(
        parse_json_file(root.join(".ait/worktrees/lt-0001.json"))["path"]
            .as_str()
            .expect("registered task worktree path"),
    );
    for path in BTreeSet::from([worktree_path, registered_path]) {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            fs::remove_file(&path).unwrap();
        } else {
            fs::remove_dir_all(&path).unwrap();
        }
    }

    let before = reconciliation_tree_bytes(&root.join(".ait"));
    let first = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--dry-run",
            "--limit",
            "25",
            "--json",
        ],
    );
    let after = reconciliation_tree_bytes(&root.join(".ait"));

    assert_eq!(first["contract"], json!("workflow-reconciliation/v1"));
    assert_eq!(first["mode"], json!("dry_run"));
    assert_eq!(first["apply_available"], json!(true));
    assert_eq!(first["mutated"], json!(false));
    assert_eq!(first["receipts_created"], json!(0));
    assert_eq!(first["sources"]["remote"]["status"], json!("not_configured"));
    assert_eq!(first["sources"]["workspace_lock"]["active"], json!(false));
    assert_eq!(before, after, "dry-run changed .ait repository bytes");

    let missing = first["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["code"].as_str() == Some("worktree.materialization_missing")
        })
        .unwrap_or_else(|| panic!("missing worktree materialization finding: {first:?}"));
    assert_eq!(missing["disposition"], json!("manual_resolution"));

    let second = json_output(
        root,
        &["workflow", "reconcile", "--dry-run", "--limit", "25", "--json"],
    );
    let first_ids = first["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["finding_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    let second_ids = second["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["finding_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(first_ids, second_ids);

}

#[test]
fn native_workflow_reconcile_apply_is_receipted_idempotent_and_protects_dirty_work() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "workflow_default_scope": "local",
  "task_default_scope": "local",
  "sprint": "off",
  "plan_task_binding": {"mode": "off"},
  "user_name": "Fixture User",
  "user_email": "fixture@example.com"
}"#,
    );
    let missing_path = init_registered_worktree(
        root,
        "missing-safe",
        "feature/missing-safe",
        None,
        None,
        false,
        Some("after_idle"),
    );
    fs::remove_dir_all(&missing_path).unwrap();
    let dirty_path = init_registered_worktree(
        root,
        "dirty-protected",
        "feature/dirty-protected",
        None,
        None,
        false,
        Some("after_idle"),
    );
    write_file(&dirty_path.join("untracked.txt"), "must survive\n");
    let dirty_registry = root.join(".ait/worktrees/dirty-protected.json");

    let applied = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--limit",
            "25",
            "--json",
        ],
    );
    assert_eq!(applied["operation"], json!("apply"));
    assert_eq!(applied["mode"], json!("apply"));
    assert_eq!(applied["mutated"], json!(true), "{applied:#?}");
    assert!(applied["receipts_created"].as_u64().unwrap() >= 2);
    assert!(applied["apply_summary"]["refused_count"]
        .as_u64()
        .unwrap()
        >= 1);
    assert!(!root.join(".ait/worktrees/missing-safe.json").exists());
    assert!(dirty_registry.exists());
    assert_eq!(fs::read_to_string(dirty_path.join("untracked.txt")).unwrap(), "must survive\n");
    let receipt_count = fs::read_dir(root.join(".ait/reconciliation/v1/receipts"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .count();
    assert_eq!(receipt_count, applied["receipts_created"].as_u64().unwrap() as usize);

    let replay = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--limit",
            "25",
            "--json",
        ],
    );
    assert_eq!(replay["mutated"], json!(false));
    assert_eq!(replay["receipts_created"], json!(0));
    assert_eq!(replay["apply_summary"]["remaining_safe_count"], json!(0));
    assert!(dirty_registry.exists());
    assert_eq!(fs::read_to_string(dirty_path.join("untracked.txt")).unwrap(), "must survive\n");

    let before_status = reconciliation_tree_bytes(&root.join(".ait"));
    let status = json_output(root, &["status", "--json"]);
    let after_status = reconciliation_tree_bytes(&root.join(".ait"));
    assert_eq!(status["reconciliation"]["state"], json!("available"));
    assert_eq!(status["reconciliation"]["safe_finding_count"], json!(0));
    assert!(status["reconciliation"]["protected_count"]
        .as_u64()
        .unwrap_or(0)
        >= 1);
    assert_eq!(
        status["reconciliation"]["next_command"],
        json!("ait workflow reconcile --apply --safe-only")
    );
    assert_eq!(before_status, after_status, "status mutated reconciliation state");
}

#[test]
fn native_scheduled_reconciliation_is_remote_bounded_and_safe_only() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let scheduled = json_output(
        temp.path(),
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--scheduled",
            "--remote",
            "origin",
            "--limit",
            "3",
            "--json",
        ],
    );

    assert_eq!(scheduled["automatic"], json!(true));
    assert_eq!(scheduled["safe_only"], json!(true));
    assert_eq!(
        scheduled["automatic_trigger"]["contract"],
        json!("workflow-automatic-reconciliation/v1")
    );
    assert_eq!(
        scheduled["automatic_trigger"]["trigger"],
        json!("scheduled_remote")
    );
    assert_eq!(scheduled["automatic_trigger"]["action_limit"], json!(3));
    assert_eq!(scheduled["remote_name"], json!("origin"));
    assert!(scheduled["cooperative_budget"]["time_budget_ms"]
        .as_u64()
        .unwrap_or(0)
        > 0);

    drop(temp);
    handle.join().unwrap();
}

#[test]
fn native_task_terminal_hook_reconciles_clean_bound_worktree() {
    let (temp, worktree, started) = init_cli_local_draft_worktree_repo("https://example.test");
    let root = temp.path();
    let task_id = started["task_id"].as_str().unwrap();
    let worktree_name = started["worktree"]["name"].as_str().unwrap();
    let registry = root
        .join(".ait")
        .join("worktrees")
        .join(format!("{worktree_name}.json"));
    assert!(registry.exists());
    assert!(worktree.exists());

    let abandoned = json_output(
        root,
        &["task", "abandon", task_id, "--local", "--json"],
    );
    assert_eq!(abandoned["status"], json!("abandoned"));
    assert_eq!(
        abandoned["automatic_reconciliation"]["automatic_trigger"]["trigger"],
        json!("task_terminal")
    );
    assert_eq!(abandoned["automatic_reconciliation"]["safe_only"], json!(true));
    assert_eq!(abandoned["automatic_reconciliation"]["mutated"], json!(true));
    assert!(!registry.exists());
    assert!(!worktree.exists());
}

#[test]
fn native_reconcile_recovers_receipts_interrupted_before_and_after_mutation() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    configure_local_reconciliation_repo(root);
    let missing_path = init_registered_worktree(
        root,
        "interrupted-safe",
        "feature/interrupted-safe",
        None,
        None,
        false,
        Some("after_idle"),
    );
    let registry = root.join(".ait/worktrees/interrupted-safe.json");
    let registry_bytes = fs::read(&registry).unwrap();
    fs::remove_dir_all(&missing_path).unwrap();

    let initial = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--limit",
            "1",
            "--json",
        ],
    );
    assert_eq!(initial["mutated"], json!(true));
    let receipt_paths = reconciliation_receipt_paths(root);
    assert_eq!(receipt_paths.len(), 1);
    let receipt_path = &receipt_paths[0];

    // Simulate termination after the durable `started` receipt and before its mutation.
    fs::write(&registry, &registry_bytes).unwrap();
    rewrite_receipt_as_started(receipt_path);
    let retried = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--limit",
            "1",
            "--json",
        ],
    );
    assert_eq!(retried["mutated"], json!(true));
    assert_eq!(retried["receipt_updates"][0]["attempt"], json!(2));
    assert_eq!(retried["receipt_updates"][0]["state"], json!("completed"));
    assert!(!registry.exists());

    // Simulate termination after the mutation but before receipt completion. The absent
    // finding is authoritative proof that replay must recover without mutating again.
    rewrite_receipt_as_started(receipt_path);
    let recovered = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--limit",
            "1",
            "--json",
        ],
    );
    assert_eq!(recovered["mutated"], json!(false));
    assert_eq!(
        recovered["apply_summary"]["recovered_receipt_count"],
        json!(1)
    );
    let receipt = parse_json_file(receipt_path);
    assert_eq!(receipt["state"], json!("completed_recovered"));
    assert_eq!(
        receipt["result"]["operation"],
        json!("authoritative_recovery")
    );
    assert!(!registry.exists());
}

#[test]
fn native_reconcile_lease_excludes_concurrent_mutators() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    configure_local_reconciliation_repo(root);
    let missing_path = init_registered_worktree(
        root,
        "lease-safe",
        "feature/lease-safe",
        None,
        None,
        false,
        Some("after_idle"),
    );
    fs::remove_dir_all(&missing_path).unwrap();
    let registry = root.join(".ait/worktrees/lease-safe.json");
    let lease_path = root.join(".ait/reconciliation/v1/reconcile.lock");
    fs::create_dir_all(lease_path.parent().unwrap()).unwrap();
    let lease = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lease_path)
        .unwrap();
    fs2::FileExt::lock_exclusive(&lease).unwrap();

    let blocked = command_output_with_env(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--limit",
            "1",
            "--json",
        ],
        &[],
    );
    assert!(!blocked.status.success());
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(
        stderr.contains("Another reconciler holds the repository lease"),
        "stderr:\n{stderr}"
    );
    assert!(registry.exists());
    assert!(reconciliation_receipt_paths(root).is_empty());

    fs2::FileExt::unlock(&lease).unwrap();
    let applied = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--limit",
            "1",
            "--json",
        ],
    );
    assert_eq!(applied["mutated"], json!(true));
    assert!(!registry.exists());
    assert_eq!(reconciliation_receipt_paths(root).len(), 1);
}

#[test]
fn native_reconcile_bounded_continuations_converge_without_duplicate_receipts() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    configure_local_reconciliation_repo(root);
    for sequence in 1..=3 {
        let name = format!("bounded-{sequence}");
        let path = init_registered_worktree(
            root,
            &name,
            &format!("feature/{name}"),
            None,
            None,
            false,
            Some("after_idle"),
        );
        fs::remove_dir_all(path).unwrap();
    }

    for expected_remaining in [2_u64, 1, 0] {
        let pass = json_output(
            root,
            &[
                "workflow",
                "reconcile",
                "--apply",
                "--safe-only",
                "--limit",
                "1",
                "--json",
            ],
        );
        assert_eq!(pass["mutated"], json!(true));
        assert_eq!(
            pass["apply_summary"]["remaining_safe_count"],
            json!(expected_remaining)
        );
        assert_eq!(
            pass["status"],
            json!(if expected_remaining == 0 {
                "completed"
            } else {
                "continuation_required"
            })
        );
    }
    let replay = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--limit",
            "1",
            "--json",
        ],
    );
    assert_eq!(replay["mutated"], json!(false));
    assert_eq!(replay["receipts_created"], json!(0));
    assert_eq!(reconciliation_receipt_paths(root).len(), 3);
}

#[test]
fn native_task_change_worktree_line_cleanup_converges_across_bounded_passes() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    configure_local_reconciliation_repo(root);
    let started = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Terminal owner cascade",
            "--intent",
            "prove worktree cleanup exposes a subsequent orphan line action",
            "--json",
        ],
    );
    let task_id = started["task_id"].as_str().unwrap();
    let change_id = started["change"]["change_id"].as_str().unwrap();
    let line_name = format!("feature/{}", task_id.to_ascii_lowercase());
    let closed_change = json_output(
        root,
        &["change", "close", change_id, "--local", "--json"],
    );
    assert_eq!(closed_change["status"], json!("archived"));
    let abandoned = json_output(
        root,
        &["task", "abandon", task_id, "--local", "--json"],
    );
    assert_eq!(abandoned["automatic_reconciliation"]["mutated"], json!(true));
    assert_eq!(
        abandoned["automatic_reconciliation"]["apply_summary"]["remaining_safe_count"],
        json!(1)
    );

    let continued = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--task",
            task_id,
            "--limit",
            "1",
            "--json",
        ],
    );
    assert_eq!(continued["mutated"], json!(true));
    assert_eq!(continued["apply_summary"]["remaining_safe_count"], json!(0));
    let line = json_output(root, &["line", "show", &line_name, "--json"]);
    assert_eq!(line["status"], json!("archived"));
}

#[test]
fn native_reconcile_rebuilds_corrupt_cached_summary_while_status_stays_read_only() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    configure_local_reconciliation_repo(root);
    let summary_path = root.join(".ait/reconciliation/v1/summary.json");
    write_file(&summary_path, "{not-json\n");
    let invalid_bytes = fs::read(&summary_path).unwrap();

    let first_status = json_output(root, &["status", "--json"]);
    let second_status = json_output(root, &["status", "--json"]);
    assert_eq!(first_status["reconciliation"]["state"], json!("invalid"));
    assert_eq!(second_status["reconciliation"]["state"], json!("invalid"));
    assert_eq!(fs::read(&summary_path).unwrap(), invalid_bytes);

    let repaired = json_output(
        root,
        &[
            "workflow",
            "reconcile",
            "--apply",
            "--safe-only",
            "--limit",
            "1",
            "--json",
        ],
    );
    assert_eq!(repaired["summary_cache"]["status"], json!("updated"));
    let status = json_output(root, &["status", "--json"]);
    assert_eq!(status["reconciliation"]["state"], json!("available"));
    assert_ne!(fs::read(&summary_path).unwrap(), invalid_bytes);
}
