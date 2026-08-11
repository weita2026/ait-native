use ait_cli::doctor_surface::{doctor_postgres, postgres_schema_checks};
use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use zip::write::FileOptions;

fn cargo_bin() -> Command {
    Command::cargo_bin("ait-cli").unwrap()
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = fs::File::create(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}

fn output_json(command: &mut Command) -> JsonValue {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid CLI JSON").unwrap()
}

fn write_postgres_schema_files(schema_root: &Path) {
    write_file(
        &schema_root
            .join("sql")
            .join("ait_native_postgres_content_schema.sql"),
        "select 1;",
    );
    write_file(
        &schema_root
            .join("sql")
            .join("ait_native_postgres_control_schema.sql"),
        "select 1;",
    );
}

#[test]
fn doctor_plan_authority_runs_without_repo() {
    let temp = TempDir::new().unwrap();
    let payload = output_json(cargo_bin().current_dir(temp.path()).args([
        "doctor",
        "plan-authority",
        "--json",
    ]));
    assert_eq!(payload["selected_backend"], "rust");
    assert_eq!(payload["compatibility"], "compatible");
    assert_eq!(payload["rust_authority_ready"], true);
    assert_eq!(
        payload["extension_plan_contract_version"],
        "plan-foundation-v7"
    );
    assert_eq!(payload["missing_exports"].as_array().unwrap().len(), 0);
}

#[test]
fn doctor_memory_root_parses_environment_without_repo() {
    let temp = TempDir::new().unwrap();
    let output = cargo_bin()
        .current_dir(temp.path())
        .env("AIT_RAM_MOUNT_POINT", "relative/ram")
        .arg("doctor")
        .arg("memory-root")
        .arg("--json")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("AIT_RAM_MOUNT_POINT must be an absolute path"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Not an ait repository"));
}

#[test]
fn doctor_runtime_root_reports_inside_repo_as_snapshot_protected_warning() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"fixture-ait","default_line":"main"}"#,
    );
    write_file(&root.join(".aitignore"), "runtime-data\n");

    let runtime_root = root.join("runtime-data");
    let payload = output_json(cargo_bin().current_dir(root).args([
        "doctor",
        "runtime-root",
        "--server-data",
        runtime_root.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(payload["state"], "warn");
    assert_eq!(payload["inside_repo"], true);
    assert_eq!(payload["snapshot_ignored"], true);
    assert_eq!(payload["protected_from_snapshots"], true);
    assert_eq!(payload["runtime_root_relative_to_repo"], "runtime-data");
}

#[test]
fn doctor_postgres_fake_connect_reports_ready_schema_versions() {
    let temp = TempDir::new().unwrap();
    let schema_root = temp.path().join("schema-root");
    write_postgres_schema_files(&schema_root);
    let server_data = temp.path().join("server-data");
    let fake_pg = temp.path().join("fake-pg");
    let fake_dsn = format!("fake-postgres://{}", fake_pg.display());

    let payload = output_json(cargo_bin().current_dir(&schema_root).args([
        "doctor",
        "postgres",
        "--server-data",
        server_data.to_str().unwrap(),
        "--dsn",
        fake_dsn.as_str(),
        "--content-schema",
        "ait_native_content",
        "--control-schema",
        "ait_native_control",
        "--connect",
        "--json",
    ]));

    assert_eq!(payload["ready"], true);
    assert_eq!(payload["attempted_live_connect"], true);
    assert_eq!(payload["live_connection_ok"], true);
    assert_eq!(payload["schema_upgrade_checks"]["ok"], true);
    assert_eq!(
        payload["schema_upgrade_checks"]["checks"]["content"]["version"],
        5
    );
    assert_eq!(
        payload["schema_upgrade_checks"]["checks"]["control"]["version"],
        3
    );
}

#[test]
fn doctor_postgres_without_connect_reports_driver_status_and_no_schema_checks() {
    let temp = TempDir::new().unwrap();
    let schema_root = temp.path().join("schema-root");
    write_postgres_schema_files(&schema_root);
    let server_data = temp.path().join("server-data");
    let fake_pg = temp.path().join("fake-pg");
    let fake_dsn = format!("fake-postgres://{}", fake_pg.display());

    let payload = doctor_postgres(
        Some(&schema_root),
        Some(&server_data),
        Some("postgres"),
        Some(fake_dsn.as_str()),
        Some("ait_native_content"),
        Some("ait_native_control"),
        false,
    )
    .unwrap();

    assert_eq!(payload["attempted_live_connect"], false);
    assert!(payload["schema_upgrade_checks"].is_null());
    assert_eq!(payload["postgres_driver"], "rust-postgres");
    assert_eq!(payload["psycopg_installed"], true);
    assert_eq!(payload["postgres_driver_available"], true);
    assert_eq!(payload["postgres_driver_status"]["available"], true);
    assert_eq!(
        payload["postgres_driver_status"]["capability"],
        "ait-core-native-postgres"
    );
}

#[test]
fn doctor_postgres_missing_dsn_keeps_python_compatible_issue_text() {
    let temp = TempDir::new().unwrap();
    let schema_root = temp.path().join("schema-root");
    write_postgres_schema_files(&schema_root);
    let server_data = temp.path().join("server-data");

    let payload = doctor_postgres(
        Some(&schema_root),
        Some(&server_data),
        Some("postgres"),
        Some(""),
        Some("ait_native_content"),
        Some("ait_native_control"),
        false,
    )
    .unwrap();
    let issues = payload["issues"].as_array().unwrap();
    assert!(issues
        .iter()
        .any(|issue| issue.as_str() == Some("AIT_NATIVE_SERVER_POSTGRES_DSN is not configured.")));
}

#[test]
fn doctor_postgres_invalid_schema_reports_validation_issue() {
    let temp = TempDir::new().unwrap();
    let schema_root = temp.path().join("schema-root");
    write_postgres_schema_files(&schema_root);
    let server_data = temp.path().join("server-data");
    let fake_pg = temp.path().join("fake-pg");
    let fake_dsn = format!("fake-postgres://{}", fake_pg.display());

    let payload = doctor_postgres(
        Some(&schema_root),
        Some(&server_data),
        Some("postgres"),
        Some(fake_dsn.as_str()),
        Some("bad-name"),
        Some("ait_native_control"),
        false,
    )
    .unwrap();
    assert_eq!(payload["content_schema_valid"], false);
    let issues = payload["issues"].as_array().unwrap();
    assert!(issues.iter().any(|issue| {
        issue
            .as_str()
            .unwrap_or_default()
            .contains("Invalid schema name \"bad-name\"")
    }));
}

#[test]
fn postgres_schema_checks_fake_check_only_does_not_rewrite_timestamps() {
    let temp = TempDir::new().unwrap();
    let schema_root = temp.path().join("schema-root");
    write_postgres_schema_files(&schema_root);
    let server_data = temp.path().join("server-data");
    let fake_pg = temp.path().join("fake-pg");
    let fake_dsn = format!("fake-postgres://{}", fake_pg.display());

    let apply_payload = postgres_schema_checks(
        Some(&schema_root),
        Some(&server_data),
        Some("postgres"),
        Some(fake_dsn.as_str()),
        Some("ait_native_content"),
        Some("ait_native_control"),
        true,
    )
    .unwrap();
    assert_eq!(apply_payload["applied"], true);
    assert_eq!(apply_payload["ok"], true);

    std::thread::sleep(Duration::from_millis(1100));

    let check_payload = postgres_schema_checks(
        Some(&schema_root),
        Some(&server_data),
        Some("postgres"),
        Some(fake_dsn.as_str()),
        Some("ait_native_content"),
        Some("ait_native_control"),
        false,
    )
    .unwrap();

    assert_eq!(check_payload["applied"], false);
    assert_eq!(check_payload["ok"], true);
    assert_eq!(
        check_payload["checks"]["content"]["version"],
        apply_payload["checks"]["content"]["version"]
    );
    assert_eq!(
        check_payload["checks"]["content"]["applied_at"],
        apply_payload["checks"]["content"]["applied_at"]
    );
    assert_eq!(
        check_payload["checks"]["content"]["checked_at"],
        apply_payload["checks"]["content"]["checked_at"]
    );
    assert_eq!(
        check_payload["checks"]["control"]["checked_at"],
        apply_payload["checks"]["control"]["checked_at"]
    );
}

#[test]
fn doctor_plan_authority_wheel_inspects_wheel_tag() {
    let temp = TempDir::new().unwrap();
    let wheel = temp
        .path()
        .join("ait_py-0.1.0-cp311-cp311-manylinux_2_28_x86_64.whl");
    let file = fs::File::create(&wheel).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("ait_py-0.1.0.dist-info/WHEEL", FileOptions::default())
        .unwrap();
    zip.write_all(b"Wheel-Version: 1.0\nTag: cp311-cp311-manylinux_2_28_x86_64\n")
        .unwrap();
    zip.finish().unwrap();

    let payload = output_json(cargo_bin().current_dir(temp.path()).args([
        "doctor",
        "plan-authority-wheel",
        "--wheel",
        wheel.to_str().unwrap(),
        "--json",
    ]));
    assert_eq!(payload["wheel_tag"], "cp311-cp311-manylinux_2_28_x86_64");
    assert_eq!(payload["wheel_target"], "linux-x86_64");
    assert_eq!(payload["wheel_target_supported"], true);
    assert_eq!(payload["issues"].as_array().unwrap().len(), 0);
}
