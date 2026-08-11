use ait_core::json_support::{JsonCodec, JsonValue};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[path = "../../../test_support.rs"]
mod workspace_test_support;

fn ait_cli() -> assert_cmd::Command {
    assert_cmd::Command::new(workspace_test_support::cargo_binary(
        "ait-cli",
        option_env!("CARGO_BIN_EXE_ait-cli"),
    ))
}

fn run_json(root: &Path, args: &[&str]) -> JsonValue {
    let mut cmd = ait_cli();
    let output = cmd
        .current_dir(root)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    JsonCodec::parse_slice_with_error_prefix(&output, "Invalid CLI JSON").unwrap()
}

fn write_base(path: &Path) {
    let text = (0..20)
        .map(|idx| format!("line {idx:02} keep same text for compression\n"))
        .collect::<String>();
    fs::write(path, text).unwrap();
}

fn write_update(path: &Path) {
    let mut lines = (0..20)
        .map(|idx| format!("line {idx:02} keep same text for compression\n"))
        .collect::<Vec<_>>();
    lines[10] = "line 10 changed text for compression\n".to_string();
    lines.push("line 20 keep same text for compression\n".to_string());
    fs::write(path, lines.concat()).unwrap();
}

#[test]
fn gc_binary_runtime_exposes_only_supported_maintenance_operations() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let app = root.join("app.txt");
    write_base(&app);

    run_json(root, &["init", "--name", "gc-cli", "--json"]);
    run_json(root, &["snapshot", "create", "--message", "base", "--json"]);

    let validate = run_json(root, &["gc", "validate", "--json"]);
    assert_eq!(validate["state"], "packed_full_only");
    assert_eq!(validate["recommended_action"], "none");
    assert_eq!(validate["next_actions"].as_array().unwrap().len(), 0);

    write_update(&app);
    run_json(
        root,
        &["snapshot", "create", "--message", "update", "--json"],
    );

    let stats = run_json(root, &["gc", "stats", "--json"]);
    assert!(stats["snapshot_count"].as_i64().unwrap() >= 2);
    assert_eq!(stats["inventory_included"], false);
    assert_eq!(stats["reachability_summary"]["computed"], false);
    assert!(stats["reachable_blob_count"].is_null());
    assert_eq!(
        stats["validation_summary"]["state"],
        "reachability_not_computed"
    );
    assert!(stats.get("packs").is_none());
    assert!(stats.get("tree_packs").is_none());

    let inventory = run_json(root, &["gc", "stats", "--include-inventory", "--json"]);
    assert_eq!(inventory["inventory_included"], true);
    assert_eq!(inventory["reachability_summary"]["computed"], true);
    assert!(inventory["validation_summary"].is_object());
    assert!(inventory["packs"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty()));
    assert!(inventory["tree_packs"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty()));

    let prune = run_json(root, &["gc", "prune", "--json"]);
    assert!(prune["removed_orphan_pack_count"].as_i64().is_some());
    assert!(prune.get("prune_unreferenced").is_none());

    for removed in ["pack", "optimize"] {
        ait_cli()
            .current_dir(root)
            .args(["gc", removed])
            .assert()
            .failure()
            .stderr(predicates::str::contains("unrecognized subcommand"));
    }
    ait_cli()
        .current_dir(root)
        .args(["gc", "prune", "--prune-unreferenced"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unexpected argument"));
}
