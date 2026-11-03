fn run_snapshot(repo: RepoRuntime, command: SnapshotCommand) -> Result<ExitCode, String> {
    match command {
        SnapshotCommand::Create(args) => {
            let _command_range = perfetto_range!("ait.cli.snapshot_create.command");
            let payload = {
                let _range = perfetto_range!("ait.cli.snapshot_create.lock_and_author");
                crate::binding_services::capture_snapshot(
                    &repo,
                    &crate::binding_services::SnapshotCaptureRequest {
                        reference: args.change_id.clone(),
                        message: args.message.clone(),
                        resume_snapshot_id: args.resume_snapshot_id.clone(),
                    },
                )?
            };
            {
                let _range = perfetto_range!("ait.cli.snapshot_create.render");
                let mut output = payload.clone();
                if args.json {
                    if let Some(object) = output.as_object_mut() {
                        object.remove("phase_timings_ms");
                    }
                }
                emit_snapshot_create_result(&output, args.json, args.full)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::List(args) => {
            let _command_range = perfetto_range!("ait.cli.snapshot_list.command");
            let payload = {
                let _range = perfetto_range!("ait.cli.snapshot_list.load");
                snapshot_list(&repo)?
            };
            {
                let _range = perfetto_range!("ait.cli.snapshot_list.render");
                if args.json {
                    let payload = bounded_ordered_json_payload(
                        &payload,
                        args.all,
                        DEFAULT_AGENT_TEXT_LIST_LIMIT,
                    );
                    print_json(&payload)?;
                } else if let Some(rows) = payload.as_array() {
                    print_bounded_evidence(
                        rows,
                        &["snapshot_id", "line_name", "message"],
                        args.all,
                        DEFAULT_AGENT_TEXT_LIST_LIMIT,
                        "ait snapshot list --all",
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Show(args) => {
            let inspection =
                crate::binding_services::inspect_snapshot(&repo, &args.snapshot_id, !args.json)?;
            if args.json {
                print_json(&inspection["payload"])?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_snapshot_show(&inspection, args.files)?
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Diff(args) => {
            let payload = crate::binding_services::diff_snapshot_selectors(
                &repo,
                &args.old_snapshot_id,
                &args.new_snapshot_id,
                args.include_text,
                args.max_bytes,
            )?;
            if args.json {
                print_json(&payload)?;
            } else if payload.get("files").is_some_and(JsonValue::is_array) {
                println!("{}", crate::binding_output::format_snapshot_diff(&payload)?);
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::RestoreLines(args) => {
            let payload = snapshot_restore_lines(
                &repo,
                &SnapshotRestoreLinesRequest {
                    snapshot_id: args.snapshot_id,
                    path: args.path,
                    line: args.line,
                    start_line: args.start_line,
                    end_line: args.end_line,
                    apply: args.yes,
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_snapshot_restore_lines(&payload)?
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Revert(args) => {
            let payload = crate::binding_services::revert_snapshot_selector(
                &repo,
                &args.snapshot_id,
                args.force,
                args.dry_run,
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_snapshot_revert(&payload)?
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Replay(args) => {
            let payload = crate::binding_services::replay_snapshot_selector(
                &repo,
                &args.snapshot_id,
                args.onto.as_deref(),
                args.force,
                args.dry_run,
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_snapshot_replay(&payload)?
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Ancestry(args) => {
            let result = crate::binding_services::query_snapshot_ancestry(
                &repo,
                &crate::binding_services::SnapshotAncestryRequest {
                    selector: args.snapshot_id,
                    descendants: args.descendants,
                    first_parent: args.first_parent,
                    max_depth: args.max_depth,
                    limit: args.limit,
                },
            );
            let payload = match snapshot_query_result(result) {
                Ok(payload) => payload,
                Err(code) => return Ok(code),
            };
            if args.json {
                print_json(&payload)?;
            } else {
                render_snapshot_ancestry(&payload, args.all)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::IsAncestor(args) => {
            let result = crate::binding_services::query_snapshot_is_ancestor(
                &repo,
                &args.older_snapshot_id,
                &args.newer_snapshot_id,
            );
            let result = match snapshot_query_result(result) {
                Ok(result) => result,
                Err(code) => return Ok(code),
            };
            let payload = result.payload;
            let is_ancestor = result.exit_code == 0;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    if is_ancestor {
                        "ancestor"
                    } else {
                        "not ancestor"
                    }
                );
            }
            Ok(if is_ancestor {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        SnapshotCommand::MergeBase(args) => {
            let result = crate::binding_services::query_snapshot_merge_base(
                &repo,
                &args.left_snapshot_id,
                &args.right_snapshot_id,
                args.all,
            );
            let result = match snapshot_query_result(result) {
                Ok(result) => result,
                Err(code) => return Ok(code),
            };
            let payload = result.payload;
            let found = result.exit_code == 0;
            if args.json {
                print_json(&payload)?;
            } else if let Some(snapshot_ids) = payload
                .get("merge_base_snapshot_ids")
                .and_then(JsonValue::as_array)
            {
                for snapshot_id in snapshot_ids.iter().filter_map(JsonValue::as_str) {
                    println!("{snapshot_id}");
                }
            }
            Ok(if found {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
    }
}

fn snapshot_parent_text(payload: &JsonValue) -> String {
    let parents = payload
        .get("parent_snapshot_ids")
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if parents.is_empty() {
        string_field(payload.get("parent_snapshot_id"))
    } else {
        parents
    }
}

fn snapshot_change_summary(delta: &JsonValue) -> String {
    let files = delta
        .get("files")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut added = 0usize;
    let mut modified = 0usize;
    let mut deleted = 0usize;
    let mut other = 0usize;
    for row in &files {
        match row.get("status").and_then(JsonValue::as_str).unwrap_or("") {
            "added" => added += 1,
            "modified" => modified += 1,
            "deleted" => deleted += 1,
            _ => other += 1,
        }
    }
    let mut parts = Vec::new();
    for (count, label) in [
        (added, "added"),
        (modified, "modified"),
        (deleted, "deleted"),
        (other, "other"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    if parts.is_empty() {
        "none relative to primary parent".to_string()
    } else {
        format!("{} files ({})", files.len(), parts.join(", "))
    }
}

pub(crate) fn format_snapshot_identity_text(
    payload: &JsonValue,
    delta: Option<&JsonValue>,
    include_evidence: bool,
) -> Result<String, String> {
    let mut blocks = Vec::new();
    let snapshot_id = string_field(payload.get("snapshot_id"));
    let parents = snapshot_parent_text(payload);
    let file_count = string_field(payload.get("file_count"));
    let change = delta
        .map(snapshot_change_summary)
        .unwrap_or_else(|| format!("root tree ({file_count} files)"));
    blocks.push(crate::render::key_values_text(
        "ait snapshot show",
        &[
            ("snapshot", snapshot_id.clone()),
            ("line", string_field(payload.get("line_name"))),
            ("parents", parents.clone()),
            ("kind", string_field(payload.get("snapshot_kind"))),
            ("files", file_count),
            ("change", change),
            ("message", string_field(payload.get("message"))),
        ],
    ));
    if !include_evidence {
        return Ok(blocks.join("\n"));
    }

    if let Some(delta) = delta {
        let delta_parent = delta
            .get("summary")
            .and_then(|summary| summary.get("old_snapshot_id"))
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        let mut rows = delta
            .get("files")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        rows.sort_by(|left, right| {
            snapshot_change_priority(left)
                .cmp(&snapshot_change_priority(right))
                .then_with(|| string_field(left.get("path")).cmp(&string_field(right.get("path"))))
        });
        if !rows.is_empty() {
            blocks.push(String::new());
            blocks.push("changed paths".to_string());
            blocks.push(bounded_evidence_text(
                &rows,
                &["status", "path"],
                false,
                DEFAULT_AGENT_TEXT_LIST_LIMIT,
                &format!("ait snapshot diff {delta_parent} {snapshot_id}"),
            ));
        }
        blocks.push(format!("tree: ait snapshot show {snapshot_id} --files"));
        return Ok(blocks.join("\n"));
    }

    let files = payload
        .get("files")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if !files.is_empty() {
        blocks.push(String::new());
        blocks.push("root tree sample".to_string());
        blocks.push(bounded_evidence_text(
            &files,
            &["path", "size_bytes", "mode"],
            false,
            DEFAULT_AGENT_TEXT_LIST_LIMIT,
            &format!("ait snapshot show {snapshot_id} --files"),
        ));
    }
    Ok(blocks.join("\n"))
}

fn snapshot_change_priority(row: &JsonValue) -> u8 {
    match row.get("status").and_then(JsonValue::as_str).unwrap_or("") {
        "deleted" => 0,
        "added" => 1,
        "modified" => 2,
        _ => 3,
    }
}

fn render_snapshot_ancestry(payload: &JsonValue, show_all: bool) -> Result<(), String> {
    println!("{}", format_snapshot_ancestry_text(payload, show_all)?);
    Ok(())
}

pub(crate) fn format_snapshot_ancestry_text(
    payload: &JsonValue,
    show_all: bool,
) -> Result<String, String> {
    let mut blocks = Vec::new();
    let query_snapshot_id = string_field(payload.get("query_snapshot_id"));
    let direction = string_field(payload.get("direction"));
    let parent_mode = string_field(payload.get("parent_mode"));
    let max_depth = payload
        .get("max_depth")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let limit = payload
        .get("limit")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let truncated = payload
        .get("truncated")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let rows = payload
        .get("snapshots")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    blocks.push(crate::render::key_values_text(
        "ait snapshot ancestry",
        &[
            ("query", query_snapshot_id.clone()),
            ("direction", direction.clone()),
            ("parent mode", parent_mode.clone()),
            ("results", rows.len().to_string()),
            (
                "query bound",
                if truncated {
                    format!("truncated at {limit}")
                } else {
                    "complete within requested limit".to_string()
                },
            ),
        ],
    ));

    let mut display_rows = rows.clone();
    let mut all_command = format!("ait snapshot ancestry {query_snapshot_id} --{direction}");
    if parent_mode == "first_parent" {
        all_command.push_str(" --first-parent");
    }
    if max_depth > 0 {
        all_command.push_str(&format!(" --max-depth {max_depth}"));
    }
    if limit > 0 {
        all_command.push_str(&format!(" --limit {limit}"));
    }
    all_command.push_str(" --all");
    if !show_all {
        display_rows.sort_by(|left, right| {
            left.get("depth")
                .and_then(JsonValue::as_u64)
                .unwrap_or(u64::MAX)
                .cmp(
                    &right
                        .get("depth")
                        .and_then(JsonValue::as_u64)
                        .unwrap_or(u64::MAX),
                )
                .then_with(|| {
                    string_field(left.get("snapshot_id"))
                        .cmp(&string_field(right.get("snapshot_id")))
                })
        });
    }
    if !display_rows.is_empty() {
        blocks.push(String::new());
        blocks.push(
            if show_all {
                "history"
            } else {
                "nearest history"
            }
            .to_string(),
        );
        blocks.push(bounded_evidence_text(
            &display_rows,
            &["depth", "snapshot_id", "parent_snapshot_ids"],
            show_all,
            DEFAULT_AGENT_TEXT_LIST_LIMIT,
            &all_command,
        ));
    }
    if truncated {
        let next_limit = limit.saturating_mul(2).max(limit.saturating_add(1));
        let mut command = format!("ait snapshot ancestry {query_snapshot_id} --{direction}");
        if parent_mode == "first_parent" {
            command.push_str(" --first-parent");
        }
        if max_depth > 0 {
            command.push_str(&format!(" --max-depth {max_depth}"));
        }
        command.push_str(&format!(" --limit {next_limit}"));
        if show_all {
            command.push_str(" --all");
        }
        blocks.push(format!("continue: {command}"));
    }
    Ok(blocks.join("\n"))
}

fn snapshot_query_result<T>(result: Result<T, String>) -> Result<T, ExitCode> {
    result.map_err(|error| {
        eprintln!(
            "{}",
            crate::binding_output::format_snapshot_query_error(&error)
        );
        ExitCode::from(crate::binding_output::SNAPSHOT_QUERY_ERROR_EXIT_CODE)
    })
}
