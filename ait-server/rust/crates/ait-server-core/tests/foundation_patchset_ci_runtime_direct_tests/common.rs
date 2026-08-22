use ait_server_core::foundation::ci_runtime_json::PatchsetCiRunJson;
use ait_server_core::foundation::patchset_ci_runtime::patchset_ci_run_json;
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    env::temp_dir().join(format!(
        "ait-server-patchset-ci-runtime-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn run_patchset_ci(payload: JsonValue) -> JsonValue {
    patchset_ci_run_json(&payload).expect("patchset CI runtime should run")
}

fn runtime_base_from_log(value: &JsonValue) -> PathBuf {
    let log_path = PathBuf::from(
        value["suite_results"][0]["artifacts"]["log_path"]["path"]
            .as_str()
            .expect("suite log path should be text"),
    );
    log_path
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .expect("log path should live under base/output/suite")
        .to_path_buf()
}

fn shard_command_args() -> Vec<String> {
    vec![
        "-c".to_string(),
        r#"if [ "${AIT_SHARD_REPO_DIR}" = "" ]; then
  echo "AIT_SHARD_REPO_DIR missing" >&2
  exit 1
fi
echo "explicit TG1 shard runner pass"
"#
        .to_string(),
    ]
}

fn native_runner_env_command_args() -> Vec<String> {
    vec![
        "-c".to_string(),
        r#"if [ -n "${PYTHONPATH:-}" ]; then
  echo "TG1 native runner must not receive PYTHONPATH: ${PYTHONPATH}" >&2
  exit 1
fi
if [ "${AIT_TG1_NATIVE_RUNNER}" != "1" ]; then
  echo "TG1 native marker missing" >&2
  exit 2
fi
if [ "${AIT_TG1_RUNNER_AUTHORITY}" != "rust" ]; then
  echo "TG1 runner authority must be rust" >&2
  exit 3
fi
if [ ! -d "${AIT_TG1_CATALOG_ROOT}" ]; then
  echo "TG1 catalog root missing" >&2
  exit 4
fi
if [ ! -d "${AIT_TG1_TARGET_ROOT}" ]; then
  echo "TG1 target root missing" >&2
  exit 5
fi
if [ "${CARGO_BUILD_JOBS}" != "1" ]; then
  echo "TG1 shard must cap cargo jobs to one per scheduler token" >&2
  exit 6
fi
echo "explicit TG1 native env pass"
"#
        .to_string(),
    ]
}

fn write_static_ait_test_tg1_contract(root: &Path) {
    let descriptor_dir = root.join("descriptors/suites");
    let members_dir = root.join("crates/ait-test-contract/src");
    fs::create_dir_all(&descriptor_dir).expect("descriptor dir should be created");
    fs::create_dir_all(&members_dir).expect("members dir should be created");
    fs::write(
        descriptor_dir.join("ait.tg1.toml"),
        r#"target_repo = "ait"
suite_id = "tg1_required"

[test_group]
test_group_id = "TG-1"
membership_source = "compiled_rust"
formal_members_source = "crates/ait-test-contract/src/test_groups.rs"
minimum_count = 33
"#,
    )
    .expect("TG1 descriptor should be written");
    let members = (1..=33)
        .map(|index| {
            format!(
                "    TestGroupMember {{ index: {index}, local_node_id: \"test_{index}.rs::test_case\", corpus_node_id: \"corpora/ait/full_repo/tests/test_{index}.rs::test_case\" }},\n"
            )
        })
        .collect::<String>();
    fs::write(
        members_dir.join("test_groups.rs"),
        format!("pub const TG1_FORMAL_MEMBERS: &[TestGroupMember] = &[\n{members}];\n"),
    )
    .expect("TG1 formal member source should be written");
}

#[cfg(unix)]
fn make_executable(path: &PathBuf) {
    let mut permissions = fs::metadata(path)
        .expect("script metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("script should be executable");
}

#[cfg(not(unix))]
fn make_executable(_path: &PathBuf) {}

fn write_script(path: &PathBuf, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("script parent should be created");
    }
    fs::write(path, body).expect("script should be written");
    make_executable(path);
}

fn base_patchset() -> JsonValue {
    json!({
        "patchset_id": "RCP-1",
        "change_id": "RCC-1",
        "base_snapshot_id": "SNP-BASE",
        "revision_snapshot_id": "SNP-REV",
        "ci_run_seq": 1,
        "author_mode": "ai_with_human_review",
        "patchset_number": 1,
    })
}

fn base_change() -> JsonValue {
    json!({
        "repo_name": "ait",
        "change_id": "RCC-1",
        "change_seq": 1,
    })
}
