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
    change_refs: Vec<String>,
    final_snapshot_id: String,
    task_start_ms: Vec<f64>,
    local_task_land_ms: Vec<f64>,
}

const LOCAL_HISTORY_LAND_COUNT: usize = 65;

pub(super) fn assert_final_snapshot_remote_promotion_contract() -> Result<(), String> {
    let mut remote = spawn_fake_remote();
    let temp = init_solo_local_fixture_repo_unactivated(&remote.base_url)?;
    let root = temp.path();
    let mut remote_state = remote
        .state
        .lock()
        .map_err(|_| "fake remote state lock poisoned".to_string())?;
    remote_state.remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    remote_state
        .ci_run_required_patchset_ids
        .insert("RP-2".to_string());
    drop(remote_state);
    let history = create_local_lands(root, LOCAL_HISTORY_LAND_COUNT)?;
    let final_change_ref = history
        .change_refs
        .last()
        .ok_or_else(|| "local history has no final Change".to_string())?;

    let ready = command_output(
        root,
        &[
            "workflow",
            "ready",
            final_change_ref,
            "--apply",
            "--remote",
            "origin",
        ],
    )?;
    if ready.status != 0 {
        return Err(format!(
            "final snapshot workflow ready did not complete: {}",
            combined_output(&ready)
        ));
    }
    let remote_task_land_started = Instant::now();
    let landed = json_output(
        root,
        &[
            "task",
            "land",
            final_change_ref,
            "--remote",
            "origin",
            "--json",
            "--full",
        ],
    )?;
    let remote_task_land_ms = remote_task_land_started.elapsed().as_secs_f64() * 1_000.0;
    if string_field(&landed, "apply_status").as_deref() != Some("done") {
        return Err(format!(
            "final snapshot task land did not complete: {}",
            encode_value_or(&landed, "{}")
        ));
    }
    let landed_history = landed
        .get("history_promotion")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            format!(
                "remote Task Land omitted history promotion manifest: {}",
                encode_value_or(&landed, "{}")
            )
        })?;
    if string_field(landed_history, "contract").as_deref() != Some("ait-history-promotion/v2")
        || landed_history
            .get("total_entry_count")
            .and_then(JsonValue::as_u64)
            != Some(LOCAL_HISTORY_LAND_COUNT as u64)
        || landed_history
            .get("entries")
            .and_then(JsonValue::as_array)
            .map(Vec::len)
            != Some(1)
    {
        return Err(format!(
            "remote Task Land did not return the bounded final stage for all {} history entries: {}",
            LOCAL_HISTORY_LAND_COUNT,
            encode_value_or(landed_history, "{}")
        ));
    }
    if landed_history
        .get("aggregate")
        .and_then(|aggregate| string_field(aggregate, "patchset_id"))
        .as_deref()
        != Some("RP-2")
    {
        return Err(format!(
            "remote Task Land did not preserve the sole aggregate Patchset: {}",
            encode_value_or(landed_history, "{}")
        ));
    }

    let logged = remote
        .log
        .lock()
        .map_err(|_| "fake remote log lock poisoned".to_string())?
        .clone();
    let history_posts = logged
        .iter()
        .filter(|row| row.method == "POST" && row.url.ends_with("/history-promotion:prepare"))
        .collect::<Vec<_>>();
    if history_posts.len() != 2 {
        return Err(format!(
            "local history promotion issued {} prepare requests instead of two bounded stages",
            history_posts.len()
        ));
    }
    let history_bodies = history_posts
        .iter()
        .map(|post| parse_value_error_string(&post.body))
        .collect::<Result<Vec<_>, _>>()?;
    let promotion_id = string_field(&history_bodies[0], "promotion_id")
        .ok_or_else(|| "first history stage omitted promotion_id".to_string())?;
    for (stage_ordinal, (body, expected_entry_count)) in
        history_bodies.iter().zip([64_usize, 1]).enumerate()
    {
        let entries = body
            .get("entries")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| format!("history stage {stage_ordinal} omitted entries"))?;
        if string_field(body, "contract").as_deref() != Some("history-promotion-prepare/v2")
            || body.get("stage_ordinal").and_then(JsonValue::as_u64) != Some(stage_ordinal as u64)
            || body.get("total_entry_count").and_then(JsonValue::as_u64)
                != Some(LOCAL_HISTORY_LAND_COUNT as u64)
            || body.get("final_stage").and_then(JsonValue::as_bool) != Some(stage_ordinal == 1)
            || entries.len() != expected_entry_count
            || string_field(body, "promotion_id").as_deref() != Some(promotion_id.as_str())
            || string_field(body, "base_snapshot_id").as_deref() != Some(FIXTURE_BASE_SNAPSHOT_ID)
            || string_field(body, "revision_snapshot_id").as_deref()
                != Some(history.final_snapshot_id.as_str())
        {
            return Err(format!(
                "history prepare stage {stage_ordinal} has invalid bounded authority: {}",
                history_posts[stage_ordinal].body
            ));
        }
    }
    if !history_bodies[0]
        .get("previous_stage_patchset_id")
        .is_some_and(JsonValue::is_null)
        || string_field(&history_bodies[1], "previous_stage_patchset_id").as_deref()
            != Some("RHP-STAGE-0")
        || string_field(&history_bodies[0], "stage_revision_snapshot_id")
            != string_field(&history_bodies[1], "stage_base_snapshot_id")
    {
        return Err("history prepare stages do not form one exact predecessor chain".to_string());
    }
    let history_entries = history_bodies
        .iter()
        .flat_map(|body| {
            body.get("entries")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
        })
        .collect::<Vec<_>>();
    if history_entries.len() != LOCAL_HISTORY_LAND_COUNT {
        return Err(format!(
            "history prepare stages carried {} entries instead of {}",
            history_entries.len(),
            LOCAL_HISTORY_LAND_COUNT
        ));
    }
    let mut previous_snapshot_id = FIXTURE_BASE_SNAPSHOT_ID.to_string();
    for (ordinal, entry) in history_entries.iter().enumerate() {
        if string_field(entry, "local_task_id").as_deref()
            != history.task_ids.get(ordinal).map(String::as_str)
            || string_field(entry, "local_change_ref").as_deref()
                != history.change_refs.get(ordinal).map(String::as_str)
        {
            return Err(format!(
                "history prepare reordered local identity at ordinal {}: {}",
                ordinal + 1,
                encode_value_or(entry, "{}")
            ));
        }
        if string_field(entry, "pre_land_target_snapshot_id").as_deref()
            != Some(previous_snapshot_id.as_str())
        {
            return Err(format!(
                "history prepare broke the Land chain at ordinal {}: {}",
                ordinal + 1,
                encode_value_or(entry, "{}")
            ));
        }
        if entry
            .get("snapshots")
            .and_then(JsonValue::as_array)
            .is_none_or(Vec::is_empty)
        {
            return Err(format!(
                "history prepare omitted the complete Snapshot difference at ordinal {}",
                ordinal + 1
            ));
        }
        previous_snapshot_id = required_string_field(entry, "landed_snapshot_id")?;
    }
    if previous_snapshot_id != history.final_snapshot_id {
        return Err("history prepare chain did not end at the final local Snapshot".to_string());
    }
    let patchset_posts = logged
        .iter()
        .filter(|row| {
            row.method == "POST"
                && row
                    .url
                    .starts_with("/v1/native/repository-authorities/7/changes/")
                && row.url.ends_with("/patchsets")
        })
        .collect::<Vec<_>>();
    if !patchset_posts.is_empty() {
        return Err(format!(
            "history promotion fell back to {} standalone Patchset POSTs",
            patchset_posts.len()
        ));
    }
    let ci_runs = logged
        .iter()
        .filter(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/patchsets/RP-2:runCi"
        })
        .count();
    if ci_runs != 1 {
        let requests = logged
            .iter()
            .map(|row| format!("{} {}", row.method, row.url))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "final snapshot promotion ran CI {ci_runs} times instead of once; ready output:\n{}\nrequests:\n{requests}",
            combined_output(&ready)
        ));
    }
    let land_submissions = logged
        .iter()
        .filter(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
        })
        .count();
    if land_submissions != 1 {
        return Err(format!(
            "final snapshot promotion submitted atomic Task Land {land_submissions} times instead of once"
        ));
    }
    if logged.iter().any(|row| {
        row.method == "POST"
            && row
                .url
                .starts_with("/v1/native/repository-authorities/7/changes/")
            && row.url.ends_with(":submit")
    }) {
        return Err("final snapshot promotion fell back to legacy Land submission".to_string());
    }
    let task_posts = logged
        .iter()
        .filter(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"
        })
        .count();
    let change_posts = logged
        .iter()
        .filter(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
        })
        .count();
    if task_posts != 0 || change_posts != 0 {
        return Err(format!(
            "history prepare used {task_posts} standalone Task and {change_posts} standalone Change POSTs"
        ));
    }
    let remote_state = remote
        .state
        .lock()
        .map_err(|_| "fake remote state lock poisoned".to_string())?;
    if history.task_ids.iter().enumerate().any(|(ordinal, _)| {
        !remote_state
            .closed_task_ids
            .contains(&format!("RT-{:04}", ordinal + 1))
    }) {
        return Err(format!(
            "atomic remote Land did not complete all {} promoted Tasks",
            LOCAL_HISTORY_LAND_COUNT
        ));
    }
    drop(remote_state);
    for forbidden in ["workflow_record.bin", "workflow_record_payload.bin"] {
        if root.join(".ait/binary-db").join(forbidden).exists() {
            return Err(format!(
                "remote task smoke materialized forbidden local workflow file {forbidden}"
            ));
        }
    }
    let mean_ms = |values: &[f64]| {
        if values.is_empty() {
            0.0
        } else {
            values.iter().sum::<f64>() / values.len() as f64
        }
    };
    eprintln!(
        "AIT_HISTORY_PROMOTION_TIMINGS {}",
        encode_value_or(
            &json!({
                "sample_count": history.task_ids.len(),
                "task_start_wall_ms": {
                    "mean": mean_ms(&history.task_start_ms),
                    "max": history.task_start_ms.iter().copied().fold(0.0_f64, f64::max),
                    "samples": history.task_start_ms,
                },
                "local_task_land_wall_ms": {
                    "mean": mean_ms(&history.local_task_land_ms),
                    "max": history.local_task_land_ms.iter().copied().fold(0.0_f64, f64::max),
                    "samples": history.local_task_land_ms,
                },
                "remote_task_land_wall_ms": remote_task_land_ms,
                "remote_task_land_phase_timings_ms": landed
                    .get("phase_timings_ms")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            }),
            "{}",
        )
    );
    remote.stop()?;
    Ok(())
}

fn create_local_lands(root: &Path, local_land_count: usize) -> Result<LocalLandHistory, String> {
    activate_fixture_binary_db_generation(root)?;
    let mut task_ids = Vec::new();
    let mut change_refs = Vec::new();
    let mut final_snapshot_id = None;
    let mut task_start_ms = Vec::new();
    let mut local_task_land_ms = Vec::new();
    for ordinal in 1..=local_land_count {
        let title = format!("Local history task {ordinal}");
        let intent = format!("Preserve local workflow history {ordinal}");
        let task_start_started = Instant::now();
        let started = json_output(
            root,
            &[
                "task", "start", "--title", &title, "--intent", &intent, "--local", "--json",
                "--full",
            ],
        )?;
        task_start_ms.push(task_start_started.elapsed().as_secs_f64() * 1_000.0);
        let task_id = string_field(&started, "task_id")
            .ok_or_else(|| format!("local task start {ordinal} did not return task_id"))?;
        let change = started
            .get("change")
            .ok_or_else(|| format!("local task start {ordinal} did not return change"))?;
        let change_id = string_field(change, "change_id")
            .ok_or_else(|| format!("local task start {ordinal} did not return change_id"))?;
        let change_ref =
            string_field(change, "change_ref").unwrap_or_else(|| format!("{task_id}/{change_id}"));
        let worktree = started
            .get("worktree")
            .ok_or_else(|| format!("local task start {ordinal} did not return worktree"))?;
        let worktree_path = string_field(worktree, "open_path")
            .or_else(|| string_field(worktree, "path"))
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
                "--message",
                &format!("local history snapshot {ordinal}"),
                "--json",
            ],
        )?;
        let snapshot_id = string_field(&snapshot, "snapshot_id")
            .ok_or_else(|| format!("local snapshot {ordinal} did not return snapshot_id"))?;
        let local_task_land_started = Instant::now();
        let local_land = json_output(
            &worktree_path,
            &["task", "land", &change_ref, "--local", "--json", "--full"],
        )?;
        local_task_land_ms.push(local_task_land_started.elapsed().as_secs_f64() * 1_000.0);
        if string_field(&local_land, "apply_status").as_deref() != Some("done") {
            return Err(format!(
                "local Task Land {ordinal} did not complete: {}",
                encode_value_or(&local_land, "{}")
            ));
        }
        task_ids.push(task_id);
        change_refs.push(change_ref);
        final_snapshot_id = Some(snapshot_id);
    }
    Ok(LocalLandHistory {
        task_ids,
        change_refs,
        final_snapshot_id: final_snapshot_id
            .ok_or_else(|| "local history did not create a final Snapshot".to_string())?,
        task_start_ms,
        local_task_land_ms,
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
            "workflow_mode": "team_remote",
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

fn init_solo_local_fixture_repo_unactivated(base_url: &str) -> Result<TempDir, String> {
    let temp = init_fixture_repo_unactivated(base_url)?;
    let config_path = temp.path().join(".ait/config.json");
    let config_text = fs::read_to_string(&config_path).map_err(|error| error.to_string())?;
    let mut config = parse_value_error_string(&config_text)?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| "fixture config must be an object".to_string())?;
    object.insert(
        "workflow_mode".to_string(),
        JsonValue::String("solo_local".to_string()),
    );
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
