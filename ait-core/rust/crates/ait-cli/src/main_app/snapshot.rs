fn run_snapshot(repo: RepoRuntime, command: SnapshotCommand) -> Result<ExitCode, String> {
    match command {
        SnapshotCommand::Create(args) => {
            let _command_range = perfetto_range!("ait.cli.snapshot_create.command");
            let payload = {
                let _range = perfetto_range!("ait.cli.snapshot_create.lock_and_author");
                run_locked_workspace_command(&repo, "ait-cli snapshot create", || {
                    snapshot_create(&repo, args.message.as_deref())
                })?
            };
            {
                let _range = perfetto_range!("ait.cli.snapshot_create.render");
                let mut output = payload.clone();
                if args.json {
                    if let Some(object) = output.as_object_mut() {
                        object.remove("phase_timings_ms");
                    }
                }
                emit_result(
                    "ait-cli snapshot create",
                    &output,
                    args.json,
                    &[
                        "snapshot_id",
                        "line_name",
                        "parent_snapshot_id",
                        "message",
                    ],
                )?;
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
            let snapshot_id = resolve_snapshot_ref_cmd(&repo, &args.snapshot_id)?;
            let payload = snapshot_show(&repo, &snapshot_id)?;
            if args.json {
                print_json(&payload)?;
            } else {
                let parent_snapshot_id = string_field(payload.get("parent_snapshot_id"));
                let delta = if parent_snapshot_id.is_empty() {
                    None
                } else {
                    Some(snapshot_diff(
                        &repo,
                        &parent_snapshot_id,
                        &snapshot_id,
                        false,
                        DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
                    )?)
                };
                render_snapshot_identity(&payload, delta.as_ref(), !args.files)?;
                if args.files {
                    if let Some(files) = payload.get("files").and_then(JsonValue::as_array) {
                        println!();
                        println!("files");
                        print_list(files, &["path", "blob_id", "size_bytes", "mode"]);
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Diff(args) => {
            let old_snapshot_id = resolve_snapshot_ref_cmd(&repo, &args.old_snapshot_id)?;
            let new_snapshot_id = resolve_snapshot_ref_cmd(&repo, &args.new_snapshot_id)?;
            let payload = snapshot_diff(
                &repo,
                &old_snapshot_id,
                &new_snapshot_id,
                args.include_text,
                args.max_bytes,
            )?;
            if args.json {
                print_json(&payload)?;
            } else if let Some(rows) = payload.get("files").and_then(JsonValue::as_array) {
                print_list(rows, &["status", "path", "diff"]);
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
            emit_result(
                "ait-cli snapshot restore-lines",
                &payload,
                args.json,
                &[
                    "mode",
                    "snapshot_id",
                    "path",
                    "blob_id",
                    "selected_range",
                    "selected_line_count",
                    "source_line_count",
                    "workspace_line_count",
                    "changed_line_count",
                    "would_overwrite_selected_local_edits",
                    "unchanged_outside_selected_range",
                    "creates_snapshot",
                    "applied",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Revert(args) => {
            let snapshot_id = resolve_snapshot_ref_cmd(&repo, &args.snapshot_id)?;
            let payload = run_locked_workspace_command(&repo, "ait-cli snapshot revert", || {
                snapshot_revert(&repo, &snapshot_id, args.force, args.dry_run)
            })?;
            emit_result(
                "ait-cli snapshot revert",
                &payload,
                args.json,
                &[
                    "snapshot_id",
                    "parent_snapshot_id",
                    "current_line",
                    "applied",
                    "affected_path_count",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Replay(args) => {
            let snapshot_id = resolve_snapshot_ref_cmd(&repo, &args.snapshot_id)?;
            let payload = run_locked_workspace_command(&repo, "ait-cli snapshot replay", || {
                let onto_line = match args.onto.as_deref() {
                    Some(onto_line) => onto_line.to_string(),
                    None => repo.current_line_name()?,
                };
                snapshot_replay(&repo, &snapshot_id, &onto_line, args.force, args.dry_run)
            })?;
            emit_result(
                "ait-cli snapshot replay",
                &payload,
                args.json,
                &[
                    "snapshot_id",
                    "parent_snapshot_id",
                    "onto_line",
                    "applied",
                    "affected_path_count",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        SnapshotCommand::Ancestry(args) => {
            let result = (|| {
                let snapshot_id = resolve_snapshot_ref_cmd(&repo, &args.snapshot_id)?;
                let direction = if args.descendants {
                    SnapshotAncestryDirection::Descendants
                } else {
                    SnapshotAncestryDirection::Ancestors
                };
                snapshot_ancestry(
                    &repo,
                    &snapshot_id,
                    direction,
                    args.first_parent,
                    args.max_depth,
                    args.limit,
                )
            })();
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
            let result = (|| {
                let older_snapshot_id =
                    resolve_snapshot_ref_cmd(&repo, &args.older_snapshot_id)?;
                let newer_snapshot_id =
                    resolve_snapshot_ref_cmd(&repo, &args.newer_snapshot_id)?;
                snapshot_is_ancestor_query(&repo, &older_snapshot_id, &newer_snapshot_id)
            })();
            let (payload, is_ancestor) = match snapshot_query_result(result) {
                Ok(payload) => payload,
                Err(code) => return Ok(code),
            };
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
            let result = (|| {
                let left_snapshot_id = resolve_snapshot_ref_cmd(&repo, &args.left_snapshot_id)?;
                let right_snapshot_id =
                    resolve_snapshot_ref_cmd(&repo, &args.right_snapshot_id)?;
                snapshot_merge_base_query(
                    &repo,
                    &left_snapshot_id,
                    &right_snapshot_id,
                    args.all,
                )
            })();
            let (payload, found) = match snapshot_query_result(result) {
                Ok(payload) => payload,
                Err(code) => return Ok(code),
            };
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

fn render_snapshot_identity(
    payload: &JsonValue,
    delta: Option<&JsonValue>,
    include_evidence: bool,
) -> Result<(), String> {
    let snapshot_id = string_field(payload.get("snapshot_id"));
    let parents = snapshot_parent_text(payload);
    let file_count = string_field(payload.get("file_count"));
    let change = delta
        .map(snapshot_change_summary)
        .unwrap_or_else(|| format!("root tree ({file_count} files)"));
    print_key_values(
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
    );
    if !include_evidence {
        return Ok(());
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
            println!();
            println!("changed paths");
            print_bounded_evidence(
                &rows,
                &["status", "path"],
                false,
                DEFAULT_AGENT_TEXT_LIST_LIMIT,
                &format!("ait snapshot diff {delta_parent} {snapshot_id}"),
            );
        }
        println!("tree: ait snapshot show {snapshot_id} --files");
        return Ok(());
    }

    let files = payload
        .get("files")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if !files.is_empty() {
        println!();
        println!("root tree sample");
        print_bounded_evidence(
            &files,
            &["path", "size_bytes", "mode"],
            false,
            DEFAULT_AGENT_TEXT_LIST_LIMIT,
            &format!("ait snapshot show {snapshot_id} --files"),
        );
    }
    Ok(())
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
    let query_snapshot_id = string_field(payload.get("query_snapshot_id"));
    let direction = string_field(payload.get("direction"));
    let parent_mode = string_field(payload.get("parent_mode"));
    let max_depth = payload
        .get("max_depth")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let limit = payload.get("limit").and_then(JsonValue::as_u64).unwrap_or(0);
    let truncated = payload
        .get("truncated")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let rows = payload
        .get("snapshots")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    print_key_values(
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
    );

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
        println!();
        println!("{}", if show_all { "history" } else { "nearest history" });
        print_bounded_evidence(
            &display_rows,
            &["depth", "snapshot_id", "parent_snapshot_ids"],
            show_all,
            DEFAULT_AGENT_TEXT_LIST_LIMIT,
            &all_command,
        );
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
        println!("continue: {command}");
    }
    Ok(())
}

fn snapshot_query_result<T>(result: Result<T, String>) -> Result<T, ExitCode> {
    result.map_err(|error| {
        eprintln!("Error: {error}");
        ExitCode::from(2)
    })
}
