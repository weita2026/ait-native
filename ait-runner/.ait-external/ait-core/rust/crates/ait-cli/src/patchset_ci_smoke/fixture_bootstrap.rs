use super::*;
use crate::runtime::RepoBinaryDbStoreFactory;
use ait_core::binary_db::{AuthorityId, BinaryDbCommandScope, LocalStateScope};
use ait_core::binary_db_generation::{
    activate_binary_db_generation, capture_binary_db_generation,
    BinaryDbGenerationActivationOptions, CaptureBinaryDbGenerationOptions,
};
use ait_core::content_binary_db::{BinaryDbContentWriteCoordinator, BinaryDbSnapshotWriteInput};
use ait_core::line_store::LineStore;

struct LocalLandHistory {
    task_ids: Vec<String>,
    final_snapshot_id: String,
}

// Exercise a full publication package; Repository Task capacity is separate.
const PACKAGE_SOURCE_TASK_COUNT: usize = 64;

pub(super) fn assert_final_snapshot_remote_promotion_contract() -> Result<(), String> {
    assert_local_package_contract(1, true)?;
    assert_local_package_contract(PACKAGE_SOURCE_TASK_COUNT, false)
}

fn assert_local_package_contract(count: usize, empty_remote: bool) -> Result<(), String> {
    let mut remote = spawn_fake_remote();
    let temp = init_local_fixture_repo_unactivated(&remote.base_url)?;
    let root = temp.path();
    if !empty_remote {
        remote.state.lock().unwrap().remote_head_snapshot_id =
            Some(FIXTURE_BASE_SNAPSHOT_ID.into());
    }
    let history = create_local_lands(root, count)?;
    let task = history.task_ids.last().ok_or("history has no final Task")?;
    let ready = json_output(
        root,
        &[
            "workflow", "ready", task, "--apply", "--remote", "origin", "--json", "--full",
        ],
    )?;
    if ready["next_action"]["code"] != "done" {
        return Err(format!("package ready did not pass: {ready}"));
    }
    let identity = ready["expected_identity"].clone();
    ait_core::workflow_expected_identity::PackageWorkflowExpectedIdentity::from_value(&identity)?;
    {
        let state = remote.state.lock().unwrap();
        let manifest = state
            .package_manifest
            .as_ref()
            .ok_or("package upload was not decoded")?;
        let entries = manifest["history_entries"]
            .as_array()
            .ok_or("package history missing")?;
        if entries.len() != count
            || manifest["base_snapshot_id"] != FIXTURE_BASE_SNAPSHOT_ID
            || manifest["revision_snapshot_id"] != history.final_snapshot_id
            || state.remote_head_snapshot_id.as_deref() != Some(FIXTURE_BASE_SNAPSHOT_ID)
            || !state.created_task_ids.is_empty()
            || !state.created_change_refs.is_empty()
            || state.package_completion.is_some()
        {
            return Err("ready changed publication authority or source history".into());
        }
        for (entry, task) in entries.iter().zip(&history.task_ids) {
            if entry["local_task_id"] != *task
                || !entry["local_change_ref"].is_null()
                || !entry["local_change_id"].is_null()
            {
                return Err("package changed Task-only source ownership".into());
            }
        }
    }
    let inputs = TempDir::new().map_err(|error| error.to_string())?;
    let expected = inputs.path().join("expected-package.json");
    fs::write(&expected, identity.to_string()).map_err(|error| error.to_string())?;
    let message = "1. Reviewed files\nNative smoke source.\n2. Findings\nNone.\n3. Risks\nDisposable mock only.\n4. Tests\nExact fixture package CI passed.\n5. Recommendation\nPass.";
    let finished = json_output(
        root,
        &[
            "workflow",
            "finish",
            task,
            "--apply",
            "--remote",
            "origin",
            "--expected-identity",
            expected.to_str().ok_or("identity path must be UTF-8")?,
            "--reviewer",
            "Fixture Bot",
            "--review-message",
            message,
            "--json",
            "--full",
        ],
    )?;
    if finished["next_action"]["code"] != "done"
        || finished["completion"]["entries"].as_array().map(Vec::len) != Some(count)
    {
        return Err(format!(
            "guarded package finish did not map the whole source: {finished}"
        ));
    }
    remote.stop()?;
    let log = remote.log.lock().unwrap();
    let package_posts = log
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.content_type.as_deref()
                == Some(ait_core::workflow_package::WORKFLOW_PACKAGE_MEDIA_TYPE)
        })
        .collect::<Vec<_>>();
    if package_posts.len() != 1 {
        return Err("package bytes must be uploaded once".into());
    }
    if empty_remote {
        let seed = log
            .iter()
            .enumerate()
            .find(|(_, row)| {
                if row.method != "POST"
                    || !row.url.ends_with("/remote-sync/zstd-bulk/commit")
                    || row.content_type.as_deref()
                        == Some(ait_core::workflow_package::WORKFLOW_PACKAGE_MEDIA_TYPE)
                {
                    return false;
                }
                let Some(body) = parse_value_option(&row.body) else {
                    return false;
                };
                body["line_update"]["line_name"] == "main"
                    && body["line_update"].get("expected_head_snapshot_id")
                        == Some(&JsonValue::Null)
                    && body["line_update"]["head_snapshot_id"] == FIXTURE_BASE_SNAPSHOT_ID
                    && body["actor_key_index"].as_u64() == Some(0)
            })
            .ok_or("null Remote main did not receive its earlier-boundary Actor-aware CAS")?;
        if seed.0 >= package_posts[0].0 {
            return Err("package upload preceded null-head initialization".into());
        }
    }
    if log.iter().any(|row| {
        row.url.ends_with("/history-promotion:prepare")
            || row.url.ends_with(":submit")
            || (row.method == "POST"
                && matches!(
                    row.url.as_str(),
                    "/v1/native/repository-authorities/7/tasks"
                        | "/v1/native/repository-authorities/7/changes"
                ))
    }) {
        return Err(
            "package path used legacy preparation or standalone workflow publication".into(),
        );
    }
    if log
        .iter()
        .filter(|row| row.method == "POST" && row.url.ends_with("/task-land"))
        .count()
        != 1
    {
        return Err("package finish must commit once".into());
    }
    let state = remote.state.lock().unwrap();
    if state.created_task_ids.len() != count
        || !state.created_change_refs.is_empty()
        || state.remote_head_snapshot_id.as_deref() != Some(history.final_snapshot_id.as_str())
    {
        return Err("package finish changed source ownership or final target".into());
    }
    for forbidden in ["workflow_record.bin", "workflow_record_payload.bin"] {
        if root.join(".ait/binary-db").join(forbidden).exists() {
            return Err(format!("forbidden local workflow authority {forbidden}"));
        }
    }
    Ok(())
}

fn create_local_lands(root: &Path, local_land_count: usize) -> Result<LocalLandHistory, String> {
    activate_fixture_binary_db_generation(root)?;
    let mut task_ids = Vec::new();
    let mut final_snapshot_id = None;
    for ordinal in 1..=local_land_count {
        let title = format!("Local history task {ordinal}");
        let intent = format!("Preserve local workflow history {ordinal}");
        let started = json_output(
            root,
            &[
                "task", "start", "--title", &title, "--intent", &intent, "--local", "--json",
                "--full",
            ],
        )?;
        let task_id = string_field(&started, "task_id")
            .ok_or_else(|| format!("local task start {ordinal} did not return task_id"))?;
        if !started["change"].is_null() {
            return Err(format!(
                "initial Task {task_id} unexpectedly created a Change"
            ));
        }
        let worktree_path = string_field(&started["worktree"], "open_path")
            .or_else(|| string_field(&started["worktree"], "path"))
            .map(PathBuf::from)
            .ok_or_else(|| format!("local task start {ordinal} returned no worktree path"))?;
        write_file(
            &worktree_path.join("src/lib.rs"),
            &format!("pub fn example() -> &'static str {{ \"local-{ordinal}\" }}\n"),
        )?;
        let snapshot = json_output(
            &worktree_path,
            &[
                "snapshot",
                "create",
                &task_id,
                "--message",
                &format!("local history snapshot {ordinal}"),
                "--json",
            ],
        )?;
        let snapshot_id = string_field(&snapshot, "snapshot_id")
            .ok_or_else(|| format!("local snapshot {ordinal} did not return snapshot_id"))?;
        let local_land = json_output(
            &worktree_path,
            &["task", "finish", &task_id, "--local", "--json", "--full"],
        )?;
        if string_field(&local_land, "apply_status").as_deref() != Some("done") {
            return Err(format!(
                "local Task Land {ordinal} did not complete: {}",
                encode_value_or(&local_land, "{}")
            ));
        }
        task_ids.push(task_id);
        final_snapshot_id = Some(snapshot_id);
    }
    Ok(LocalLandHistory {
        task_ids,
        final_snapshot_id: final_snapshot_id
            .ok_or_else(|| "local history did not create a final Snapshot".to_string())?,
    })
}

pub(super) fn string_field(value: &JsonValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(ToString::to_string)
}

pub(super) fn init_fixture_repo(base_url: &str) -> Result<TempDir, String> {
    let temp = init_fixture_repo_unactivated(base_url)?;
    activate_fixture_binary_db_generation(temp.path())?;
    Ok(temp)
}

fn init_fixture_repo_unactivated(base_url: &str) -> Result<TempDir, String> {
    let temp = TempDir::new().map_err(|err| err.to_string())?;
    let root = temp.path();
    fs::create_dir_all(root.join(".ait/binary-db")).map_err(|err| err.to_string())?;
    fs::create_dir_all(root.join(".ait/objects")).map_err(|err| err.to_string())?;
    write_file(
        &root.join(".ait/config.json"),
        &json!({
            "repo_name": "fixture-ait",
            "repository_index": 7,
            "default_line": "main",
            "default_remote": "origin",
            "agent_teams": true,
            "workflow_default_scope": "remote",
            "task_default_scope": "remote",
            "sprint": "off",
            "plan_task_binding": {"mode": "off"},
            "user_name": "Fixture User",
            "user_email": "fixture@example.com",
            "remotes": {
                "origin": {
                    "remote_id": 1,
                    "url": base_url,
                    "repo_name": "fixture-ait",
                    "created_at": "2026-06-08T00:00:00Z"
                }
            }
        })
        .to_string(),
    )?;
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"ok\" }\n",
    )?;
    write_file(
        &root.join("ci/patch_ci.json"),
        r#"{"schema_version":1,"suites":[{"runner":{"kind":"test_discovery_sharded","build_args":["test","--test","patchset_ci_runner","--no-run"]}}]}"#,
    )?;
    binary_stores(root)
        .lines()
        .create_line("main", None, "2026-06-08T00:00:00Z")?;
    seed_snapshot_head(root, "main", FIXTURE_BASE_SNAPSHOT_ID)?;
    Ok(temp)
}

fn init_local_fixture_repo_unactivated(base_url: &str) -> Result<TempDir, String> {
    let temp = init_fixture_repo_unactivated(base_url)?;
    let config_path = temp.path().join(".ait/config.json");
    let config_text = fs::read_to_string(&config_path).map_err(|error| error.to_string())?;
    let mut config = parse_value_error_string(&config_text)?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| "fixture config must be an object".to_string())?;
    object.insert("agent_teams".to_string(), json!(false));
    object.insert(
        "workflow_default_scope".to_string(),
        JsonValue::String("local".to_string()),
    );
    object.insert(
        "task_default_scope".to_string(),
        JsonValue::String("local".to_string()),
    );
    write_file(&config_path, &encode_value_or(&config, "{}"))?;
    Ok(temp)
}

fn activate_fixture_binary_db_generation(root: &Path) -> Result<(), String> {
    let generation_root = root.join(".ait/fixture-binary-db-generation");
    capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
        repo_root: root.to_path_buf(),
        output_root: generation_root.clone(),
        jobs: 1,
    })?;
    activate_binary_db_generation(BinaryDbGenerationActivationOptions {
        repo_root: root.to_path_buf(),
        generation_root: generation_root.clone(),
        expected_current_authority_fingerprint: None,
    })?;
    fs::remove_dir_all(&generation_root).map_err(|error| {
        format!(
            "failed to remove activated fixture generation {}: {error}",
            generation_root.display()
        )
    })
}

pub(super) fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(path, content).map_err(|err| err.to_string())
}

pub(super) fn seed_snapshot_head(
    root: &Path,
    line_name: &str,
    snapshot_id: &str,
) -> Result<(), String> {
    let stores = binary_stores(root);
    let lines = stores.lines();
    let parent_snapshot_id = lines
        .line_by_name(line_name)?
        .and_then(|line| line.head_snapshot_id);
    let content = stores.content();
    let seeded = content.create_snapshot_content(
        "fixture-ait",
        line_name,
        parent_snapshot_id.as_deref(),
        Some("seed snapshot"),
        false,
    )?;
    let coordinator = BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    );
    let inserted = coordinator
        .record_snapshot(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: snapshot_id.to_string(),
                parent_snapshot_ids: parent_snapshot_id.into_iter().collect(),
                root_tree_pack_id: required_string_field(&seeded, "root_tree_pack_id")?,
                root_entry_ordinal: required_i64_field(&seeded, "root_entry_ordinal")?,
                manifest_hash: required_string_field(&seeded, "manifest_hash")?,
                message: string_field(&seeded, "message"),
                line_name: line_name.to_string(),
                snapshot_kind: "line".to_string(),
                file_count: required_i64_field(&seeded, "file_count")?,
                total_bytes: required_i64_field(&seeded, "total_bytes")?,
                created_at: required_string_field(&seeded, "created_at")?,
            },
        )
        .map_err(|err| err.to_string())?;
    if !inserted {
        return Err(format!("fixture snapshot already exists: {snapshot_id}"));
    }
    lines.set_line_head(line_name, Some(snapshot_id), "2026-06-08T00:00:00Z")?;
    Ok(())
}

fn binary_stores(root: &Path) -> RepoBinaryDbStoreFactory<1> {
    RepoBinaryDbStoreFactory::new(
        root,
        root.join(".ait/binary-db"),
        AuthorityId::new("local:fixture-ait"),
        LocalStateScope::Repository,
    )
}

fn required_string_field(value: &JsonValue, key: &str) -> Result<String, String> {
    string_field(value, key).ok_or_else(|| format!("fixture snapshot missing {key}"))
}

fn required_i64_field(value: &JsonValue, key: &str) -> Result<i64, String> {
    value
        .get(key)
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| format!("fixture snapshot missing integer {key}"))
}
