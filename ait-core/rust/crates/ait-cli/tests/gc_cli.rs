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

    run_json(root, &["init", "--json"]);
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
    assert!(stats.get("inventory_included").is_none());
    assert_eq!(stats["reachability_summary"]["computed"], false);
    assert!(stats["reachable_blob_count"].is_null());
    assert_eq!(
        stats["validation_summary"]["state"],
        "reachability_not_computed"
    );
    assert!(stats.get("packs").is_none());
    assert!(stats.get("tree_packs").is_none());

    let preview = run_json(root, &["gc", "prune", "--json"]);
    assert_eq!(preview["mode"], "preview");
    assert_eq!(preview["applied"], false);
    assert!(preview["candidate_orphan_pack_count"].as_i64().is_some());
    assert!(preview.get("removed_orphan_pack_count").is_none());
    ait_cli()
        .current_dir(root)
        .args(["gc", "prune"])
        .assert()
        .success()
        .stdout(predicates::str::contains("mode: preview"))
        .stdout(predicates::str::contains("applied: false"))
        .stdout(predicates::str::contains(
            "candidate_verified_fallback_blob_count:",
        ));

    let applied = run_json(root, &["gc", "prune", "--apply", "--json"]);
    assert_eq!(applied["mode"], "apply");
    assert_eq!(applied["applied"], true);
    assert!(applied["removed_orphan_pack_count"].as_i64().is_some());
    assert!(applied.get("prune_unreferenced").is_none());

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
    for removed in ["--deep", "--include-inventory"] {
        ait_cli()
            .current_dir(root)
            .args(["gc", "stats", removed])
            .assert()
            .failure()
            .stderr(predicates::str::contains("unexpected argument"));
    }
}

#[test]
fn gc_validate_emits_attention_result_before_returning_failure() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(root.join("app.txt"), "content requiring a pack\n").unwrap();
    run_json(root, &["init", "--json"]);
    run_json(root, &["snapshot", "create", "--message", "base", "--json"]);

    let pack_path = fs::read_dir(root.join(".ait/objects/packs"))
        .unwrap()
        .next()
        .expect("snapshot must create an object pack")
        .unwrap()
        .path();
    fs::write(pack_path, b"invalid object pack").unwrap();

    let output = ait_cli()
        .current_dir(root)
        .args(["gc", "validate", "--json"])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let payload =
        JsonCodec::parse_slice_with_error_prefix(&output, "Invalid validation JSON").unwrap();
    assert_eq!(payload["state"], "attention_required");
    assert_eq!(payload["needs_attention"], true);
    assert!(payload["issues"]
        .as_array()
        .is_some_and(|issues| !issues.is_empty()));
}
