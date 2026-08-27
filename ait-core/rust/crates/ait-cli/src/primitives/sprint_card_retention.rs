use super::*;
use crate::primitives::plan_checklist_closeout::plan_sync_request;
use ait_core::plan_command_execution::execute_plan_list_command_request_json;
use ait_core::plan_items::extract_plan_items;
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
use ait_core::workflow_primitives::CheckboxState;

const PLAN_BINARY_DB_WRITE_LAYOUT: u32 = 1;
const SPRINT_CARD_DIRECTORY: &str = "docs/sprints";
const COMPLETED_SPRINT_CARD_RETENTION: usize = 20;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct SprintCardOrderKey {
    revision_created_at_s: u64,
    revision_ordinal: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SprintCardLineage {
    plan_ids: BTreeSet<String>,
    order_key: SprintCardOrderKey,
    eligible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompletedSprintCard {
    artifact_path: String,
    absolute_path: PathBuf,
    order_key: SprintCardOrderKey,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SprintCardRetentionSelection {
    tracked_card_count: usize,
    active_bound_card_count: usize,
    completed_card_count: usize,
    retained_completed_count: usize,
    prune_cards: Vec<CompletedSprintCard>,
}

pub(super) fn apply_completed_sprint_card_retention(
    repo: &RepoRuntime,
    current_artifact_path: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if !is_direct_sprint_card_path(current_artifact_path) {
        return Ok(json!({
            "status": "skipped",
            "reason": "bound_artifact_is_not_a_sprint_card",
            "retention_limit": COMPLETED_SPRINT_CARD_RETENTION,
        }));
    }

    let lineage = load_sprint_card_lineage(repo, remote_name.is_some())?;
    let active_plan_ids = load_active_task_plan_ids(repo, remote_name)?;
    let selection = select_sprint_card_retention(
        &repo.authoritative_repo_root(),
        &lineage,
        &active_plan_ids,
        current_artifact_path,
        COMPLETED_SPRINT_CARD_RETENTION,
    )?;
    let mut removed_artifact_paths = Vec::<String>::new();
    let mut syncs = Vec::new();

    for card in &selection.prune_cards {
        let sync = remove_sprint_card_and_sync(&card.absolute_path, &card.artifact_path, || {
            execute_plan_sync_command_request_json(
                &plan_sync_request(repo, &card.artifact_path, None, remote_name, true)?.to_string(),
            )
        })
        .map_err(|error| {
            let already_removed = if removed_artifact_paths.is_empty() {
                "none".to_string()
            } else {
                removed_artifact_paths.join(", ")
            };
            format!(
                "Sprint-card retention stopped at {} after successfully pruning [{}]: {error}",
                card.artifact_path, already_removed
            )
        })?;
        removed_artifact_paths.push(card.artifact_path.clone());
        syncs.push(sync);
    }

    Ok(json!({
        "status": if removed_artifact_paths.is_empty() { "unchanged" } else { "pruned" },
        "retention_limit": COMPLETED_SPRINT_CARD_RETENTION,
        "tracked_card_count": selection.tracked_card_count,
        "active_bound_card_count": selection.active_bound_card_count,
        "completed_card_count": selection.completed_card_count,
        "retained_completed_count": selection.retained_completed_count,
        "removed_count": removed_artifact_paths.len(),
        "removed_artifact_paths": removed_artifact_paths,
        "syncs": syncs,
    }))
}

fn load_sprint_card_lineage(
    repo: &RepoRuntime,
    require_published: bool,
) -> Result<BTreeMap<String, SprintCardLineage>, String> {
    let payload =
        execute_plan_list_command_request_json(&local_plan_list_request(repo)?.to_string())?;
    let rows = payload.as_array().ok_or_else(|| {
        "Local plan list did not return an array for sprint retention.".to_string()
    })?;
    let mut lineage = BTreeMap::<String, SprintCardLineage>::new();

    for row in rows {
        let Some(artifact_path) = string_field(row, "head_artifact_path") else {
            continue;
        };
        if !is_direct_sprint_card_path(&artifact_path)
            || is_historical_plan_status(string_field(row, "status").as_deref())
        {
            continue;
        }
        let Some(plan_id) = string_field(row, "plan_id") else {
            continue;
        };
        let row_is_eligible = !require_published
            || string_field(row, "publication_state").as_deref() == Some("published");
        let order_key = if row_is_eligible {
            Some(sprint_card_order_key(row).map_err(|error| {
                format!("Invalid sprint-card Plan history for {artifact_path}: {error}")
            })?)
        } else {
            None
        };
        let entry = lineage.entry(artifact_path).or_default();
        entry.plan_ids.insert(plan_id);
        let Some(order_key) = order_key else {
            continue;
        };
        entry.eligible = true;
        if order_key > entry.order_key {
            entry.order_key = order_key;
        }
    }
    Ok(lineage)
}

fn sprint_card_order_key(row: &JsonValue) -> Result<SprintCardOrderKey, String> {
    let revision_created_at = string_field(row, "head_revision_created_at")
        .ok_or_else(|| "missing `head_revision_created_at`".to_string())?;
    let revision_created_at_s = revision_created_at
        .parse::<u64>()
        .map_err(|_| format!("invalid `head_revision_created_at` value {revision_created_at:?}"))?;
    let revision_id = string_field(row, "head_revision_id")
        .ok_or_else(|| "missing `head_revision_id`".to_string())?;
    let revision_ordinal = revision_id
        .strip_prefix("plan-revision:")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| format!("invalid `head_revision_id` value {revision_id:?}"))?;
    Ok(SprintCardOrderKey {
        revision_created_at_s,
        revision_ordinal,
    })
}

fn load_active_task_plan_ids(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
) -> Result<BTreeSet<String>, String> {
    let mut active_plan_ids = BTreeSet::new();
    if let Some(remote_name) = remote_name {
        collect_active_task_plan_ids(
            &task_list(repo, false, Some(remote_name))?,
            &mut active_plan_ids,
        )?;
    }
    Ok(active_plan_ids)
}

fn collect_active_task_plan_ids(
    payload: &JsonValue,
    active_plan_ids: &mut BTreeSet<String>,
) -> Result<(), String> {
    let rows = payload
        .as_array()
        .ok_or_else(|| "Task list did not return an array for sprint retention.".to_string())?;
    for row in rows {
        if task_status_is_closed(string_field(row, "status").as_deref()) {
            continue;
        }
        if let Some(plan_id) = string_field(row, "plan_id") {
            active_plan_ids.insert(plan_id);
        }
    }
    Ok(())
}

fn select_sprint_card_retention(
    repo_root: &Path,
    lineage: &BTreeMap<String, SprintCardLineage>,
    active_plan_ids: &BTreeSet<String>,
    current_artifact_path: &str,
    retention_limit: usize,
) -> Result<SprintCardRetentionSelection, String> {
    let mut selection = SprintCardRetentionSelection {
        tracked_card_count: lineage.len(),
        ..Default::default()
    };
    let mut completed = Vec::new();

    for (artifact_path, card_lineage) in lineage {
        if !card_lineage.eligible {
            continue;
        }
        if card_lineage
            .plan_ids
            .iter()
            .any(|plan_id| active_plan_ids.contains(plan_id))
        {
            selection.active_bound_card_count += 1;
            continue;
        }
        let absolute_path = repo_root.join(artifact_path);
        match fs::symlink_metadata(&absolute_path) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to inspect sprint card {artifact_path}: {error}"
                ))
            }
        }
        let markdown = fs::read_to_string(&absolute_path)
            .map_err(|error| format!("Failed to read sprint card {artifact_path}: {error}"))?;
        if !sprint_card_is_completed(&markdown) {
            continue;
        }
        completed.push(CompletedSprintCard {
            artifact_path: artifact_path.clone(),
            absolute_path,
            order_key: card_lineage.order_key,
        });
    }

    completed.sort_by(|left, right| {
        let left_is_current = left.artifact_path == current_artifact_path;
        let right_is_current = right.artifact_path == current_artifact_path;
        right_is_current
            .cmp(&left_is_current)
            .then_with(|| right.order_key.cmp(&left.order_key))
            .then_with(|| left.artifact_path.cmp(&right.artifact_path))
    });
    selection.completed_card_count = completed.len();
    selection.retained_completed_count = completed.len().min(retention_limit);
    selection.prune_cards = completed.into_iter().skip(retention_limit).collect();
    Ok(selection)
}

fn sprint_card_is_completed(markdown: &str) -> bool {
    let items = extract_plan_items(Some(markdown));
    !items.is_empty()
        && items
            .iter()
            .all(|item| item.checkbox_state == CheckboxState::Done)
}

fn remove_sprint_card_and_sync<F>(
    absolute_path: &Path,
    artifact_path: &str,
    sync: F,
) -> Result<JsonValue, String>
where
    F: FnOnce() -> Result<JsonValue, String>,
{
    let original = fs::read(absolute_path)
        .map_err(|error| format!("Failed to read {artifact_path} before pruning: {error}"))?;
    fs::remove_file(absolute_path)
        .map_err(|error| format!("Failed to remove completed card {artifact_path}: {error}"))?;
    let sync_result = sync().and_then(|payload| validate_prune_sync(&payload, artifact_path));
    match sync_result {
        Ok(payload) => Ok(payload),
        Err(error) => {
            fs::write(absolute_path, original).map_err(|restore_error| {
                format!("{error}; additionally failed to restore {artifact_path}: {restore_error}")
            })?;
            Err(error)
        }
    }
}

fn validate_prune_sync(payload: &JsonValue, artifact_path: &str) -> Result<JsonValue, String> {
    if payload.get("status").and_then(JsonValue::as_str) != Some("ok") {
        let error = string_field(payload, "error")
            .unwrap_or_else(|| "plan sync returned a non-ok result".to_string());
        return Err(format!("Prune sync failed for {artifact_path}: {error}"));
    }
    let pruned = payload
        .get("results")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .any(|row| {
            string_field(row, "action").as_deref() == Some("pruned")
                && string_field(row, "artifact_path").as_deref() == Some(artifact_path)
        });
    if !pruned {
        return Err(format!(
            "Prune sync for {artifact_path} did not archive the tracked Plan history."
        ));
    }
    Ok(payload.clone())
}

fn local_plan_list_request(repo: &RepoRuntime) -> Result<JsonValue, String> {
    Ok(json!({
        "scope": "local",
        "repository_index": repo.repository_index(),
        "repo_name": repo.repo_name(),
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    }))
}

fn is_direct_sprint_card_path(artifact_path: &str) -> bool {
    let path = Path::new(artifact_path);
    path.parent() == Some(Path::new(SPRINT_CARD_DIRECTORY))
        && path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("md"))
}

fn is_historical_plan_status(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("archived" | "canceled" | "cancelled" | "abandoned" | "deleted")
    )
}

fn task_status_is_closed(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("completed" | "canceled" | "cancelled" | "archived" | "abandoned" | "failed")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    use tempfile::tempdir;

    fn completed_markdown(item_ref: &str) -> String {
        format!("# Card [plan-ref: card/root]\n\n- [x] Done [ref: {item_ref}]\n")
    }

    fn tracked(
        revision_created_at_s: u64,
        revision_ordinal: u64,
        plan_id: &str,
    ) -> SprintCardLineage {
        SprintCardLineage {
            plan_ids: BTreeSet::from([plan_id.to_string()]),
            order_key: SprintCardOrderKey {
                revision_created_at_s,
                revision_ordinal,
            },
            eligible: true,
        }
    }

    #[test]
    fn completion_requires_at_least_one_referenced_checkbox_and_no_open_items() {
        assert!(sprint_card_is_completed(
            "# Card [plan-ref: card/root]\n\n- [x] Done [ref: card/done]\n"
        ));
        assert!(!sprint_card_is_completed(
            "# Card [plan-ref: card/root]\n\n- [x] Done [ref: card/done]\n- [ ] Open [ref: card/open]\n"
        ));
        assert!(!sprint_card_is_completed(
            "# Card [plan-ref: card/root]\n\n- [x] Unreferenced\n"
        ));
    }

    #[test]
    fn order_key_uses_revision_time_and_numeric_ordinal() {
        let revision_98 = sprint_card_order_key(&json!({
            "head_revision_created_at": "1000",
            "head_revision_id": "plan-revision:98",
        }))
        .unwrap();
        let revision_2179 = sprint_card_order_key(&json!({
            "head_revision_created_at": "1000",
            "head_revision_id": "plan-revision:2179",
        }))
        .unwrap();
        let later_revision_7 = sprint_card_order_key(&json!({
            "head_revision_created_at": "1001",
            "head_revision_id": "plan-revision:7",
        }))
        .unwrap();

        assert!(revision_2179 > revision_98);
        assert!(later_revision_7 > revision_2179);
        assert!(sprint_card_order_key(&json!({
            "head_revision_created_at": "1002",
            "head_revision_id": "plan-revision:not-a-number",
        }))
        .is_err());
    }

    #[test]
    fn retention_prunes_the_oldest_card_across_revision_digit_boundaries() {
        let temp = tempdir().unwrap();
        let sprint_dir = temp.path().join(SPRINT_CARD_DIRECTORY);
        fs::create_dir_all(&sprint_dir).unwrap();
        let fixtures = [
            ("old-revision-98.md", 1_784_256_504, 98, "PL-OLD"),
            ("recent-revision-2179.md", 1_785_228_118, 2_179, "PL-RECENT"),
            (
                "current-revision-2181.md",
                1_785_232_118,
                2_181,
                "PL-CURRENT",
            ),
        ];
        let mut lineage = BTreeMap::new();
        for (file_name, created_at_s, revision_ordinal, plan_id) in fixtures {
            let artifact_path = format!("docs/sprints/{file_name}");
            fs::write(
                temp.path().join(&artifact_path),
                completed_markdown(plan_id),
            )
            .unwrap();
            lineage.insert(
                artifact_path,
                tracked(created_at_s, revision_ordinal, plan_id),
            );
        }

        let selection = select_sprint_card_retention(
            temp.path(),
            &lineage,
            &BTreeSet::new(),
            "docs/sprints/current-revision-2181.md",
            2,
        )
        .unwrap();

        assert_eq!(
            selection
                .prune_cards
                .iter()
                .map(|card| card.artifact_path.as_str())
                .collect::<Vec<_>>(),
            vec!["docs/sprints/old-revision-98.md"]
        );
    }

    #[test]
    fn retention_keeps_active_untracked_and_open_cards_and_prunes_beyond_twenty() {
        let temp = tempdir().unwrap();
        let sprint_dir = temp.path().join(SPRINT_CARD_DIRECTORY);
        fs::create_dir_all(&sprint_dir).unwrap();
        let mut lineage = BTreeMap::new();
        for index in 0..22 {
            let artifact_path = format!("docs/sprints/completed-{index:02}.md");
            fs::write(
                temp.path().join(&artifact_path),
                completed_markdown(&format!("card/{index:02}")),
            )
            .unwrap();
            lineage.insert(
                artifact_path,
                tracked(index, index, &format!("PL-{index:02}")),
            );
        }
        fs::write(
            sprint_dir.join("open.md"),
            "# Open [plan-ref: open/root]\n\n- [ ] Open [ref: open/item]\n",
        )
        .unwrap();
        lineage.insert(
            "docs/sprints/open.md".to_string(),
            tracked(99, 99, "PL-OPEN"),
        );
        fs::write(
            sprint_dir.join("active.md"),
            completed_markdown("active/item"),
        )
        .unwrap();
        lineage.insert(
            "docs/sprints/active.md".to_string(),
            tracked(98, 98, "PL-ACTIVE"),
        );
        fs::write(
            sprint_dir.join("untracked.md"),
            completed_markdown("untracked/item"),
        )
        .unwrap();

        let selection = select_sprint_card_retention(
            temp.path(),
            &lineage,
            &BTreeSet::from(["PL-ACTIVE".to_string()]),
            "docs/sprints/completed-21.md",
            20,
        )
        .unwrap();

        assert_eq!(selection.tracked_card_count, 24);
        assert_eq!(selection.active_bound_card_count, 1);
        assert_eq!(selection.completed_card_count, 22);
        assert_eq!(selection.retained_completed_count, 20);
        assert_eq!(
            selection
                .prune_cards
                .iter()
                .map(|card| card.artifact_path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "docs/sprints/completed-01.md",
                "docs/sprints/completed-00.md"
            ]
        );
    }

    #[test]
    fn current_completed_card_is_never_selected_for_pruning() {
        let temp = tempdir().unwrap();
        let sprint_dir = temp.path().join(SPRINT_CARD_DIRECTORY);
        fs::create_dir_all(&sprint_dir).unwrap();
        let mut lineage = BTreeMap::new();
        for index in 0..3 {
            let artifact_path = format!("docs/sprints/card-{index}.md");
            fs::write(
                temp.path().join(&artifact_path),
                completed_markdown(&format!("card/{index}")),
            )
            .unwrap();
            lineage.insert(artifact_path, tracked(index, index, &format!("PL-{index}")));
        }

        let selection = select_sprint_card_retention(
            temp.path(),
            &lineage,
            &BTreeSet::new(),
            "docs/sprints/card-0.md",
            1,
        )
        .unwrap();

        assert_eq!(selection.retained_completed_count, 1);
        assert!(selection
            .prune_cards
            .iter()
            .all(|card| card.artifact_path != "docs/sprints/card-0.md"));
    }

    #[test]
    fn prune_sync_failure_restores_the_card_exactly() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("completed.md");
        let body = completed_markdown("card/item");
        fs::write(&path, &body).unwrap();

        let error = remove_sprint_card_and_sync(&path, "docs/sprints/completed.md", || {
            Err("remote unavailable".to_string())
        })
        .unwrap_err();

        assert!(error.contains("remote unavailable"));
        assert_eq!(fs::read_to_string(path).unwrap(), body);
    }

    #[test]
    fn successful_exact_prune_sync_removes_the_card() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("completed.md");
        fs::write(&path, completed_markdown("card/item")).unwrap();

        let sync = remove_sprint_card_and_sync(&path, "docs/sprints/completed.md", || {
            Ok(json!({
                "status": "ok",
                "results": [{
                    "action": "pruned",
                    "artifact_path": "docs/sprints/completed.md"
                }]
            }))
        })
        .unwrap();

        assert_eq!(sync["status"], "ok");
        assert!(!path.exists());
    }

    #[test]
    fn local_retention_prunes_real_plan_lineage_beyond_twenty() {
        let temp = tempdir().unwrap();
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("demo".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        let sprint_dir = temp.path().join(SPRINT_CARD_DIRECTORY);
        fs::create_dir_all(&sprint_dir).unwrap();

        for index in 0..21 {
            let artifact_path = format!("docs/sprints/card-{index:02}.md");
            let plan_ref = format!("card-{index:02}/root");
            fs::write(
                temp.path().join(&artifact_path),
                format!("# Card [plan-ref: {plan_ref}]\n\n- [x] Done [ref: card/{index:02}]\n"),
            )
            .unwrap();
            let sync = execute_plan_sync_command_request_json(
                &plan_sync_request(&repo, &artifact_path, Some(&plan_ref), None, false)
                    .unwrap()
                    .to_string(),
            )
            .unwrap();
            assert_eq!(sync["status"], "ok", "{sync}");
        }

        let retention =
            apply_completed_sprint_card_retention(&repo, "docs/sprints/card-20.md", None).unwrap();

        assert_eq!(retention["status"], "pruned");
        assert_eq!(retention["completed_card_count"], 21);
        assert_eq!(retention["retained_completed_count"], 20);
        assert_eq!(retention["removed_count"], 1);
        assert!(sprint_dir.join("card-20.md").exists());
        assert_eq!(
            fs::read_dir(sprint_dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("md")
                )
                .count(),
            20
        );
    }
}
