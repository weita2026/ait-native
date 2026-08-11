use super::*;
use ait_core::external::update::ExternalUpdateSelection;
use std::fs;
use tempfile::TempDir;

fn write_runtime_config(root: &std::path::Path, config_json: &str) {
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(root.join(".ait/config.json"), config_json).unwrap();
}

#[test]
fn contains_terms_rejects_empty_values() {
    assert!(parse_contains_terms(" , ").is_err());
}

#[test]
fn contains_terms_deduplicates_values() {
    let terms = parse_contains_terms("alpha, beta ,alpha").unwrap();
    assert_eq!(terms, vec!["alpha".to_string(), "beta".to_string()]);
}

#[test]
fn remote_add_text_explains_patch_ci_configuration() {
    let rendered = render_remote_add_text(&json!({
        "name": "origin",
        "url": "https://example.test",
        "repo_name": "demo",
        "is_default_push": 1,
        "is_default_pull": 1,
        "created_at": "2026-07-28T00:00:00Z",
        "discarded_export": true,
        "patch_ci": {
            "status": "ready",
            "required": true,
            "manifest_path": "ci/patch_ci.json",
            "blocking_suite_ids": ["python_unit"],
        },
    }))
    .unwrap();

    assert!(rendered.contains("Patchset CI"));
    assert!(rendered.contains("manifest: ci/patch_ci.json"));
    assert!(rendered.contains("blocking_suites: python_unit"));
    assert!(rendered.contains("suites[].runner.commands"));
    assert!(rendered.contains("default_blocking: true"));
    assert!(rendered.contains("create a new Snapshot"));
    assert!(rendered.contains("discarded_export: true"));
}

#[test]
fn workflow_reconcile_parser_freezes_read_only_inventory_contract() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "workflow",
        "reconcile",
        "--remote",
        "origin",
        "--task",
        "RCT-42",
        "--dry-run",
        "--safe-only",
        "--limit",
        "7",
        "--json",
    ])
    .expect("workflow reconcile inventory should parse");
    let Commands::Workflow {
        command: WorkflowCommand::Reconcile(args),
    } = parsed.command
    else {
        panic!("expected workflow reconcile command");
    };
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert_eq!(args.task.as_deref(), Some("RCT-42"));
    assert!(args.dry_run);
    assert!(!args.apply);
    assert!(args.safe_only);
    assert!(!args.scheduled);
    assert_eq!(args.limit, 7);
    assert!(args.json);

    let conflict =
        Cli::try_parse_from(["ait-cli", "workflow", "reconcile", "--dry-run", "--apply"])
            .err()
            .expect("dry-run and apply must conflict")
            .to_string();
    assert!(conflict.contains("cannot be used with"), "{conflict}");

    let scheduled = Cli::try_parse_from([
        "ait-cli",
        "workflow",
        "reconcile",
        "--apply",
        "--scheduled",
        "--remote",
        "origin",
        "--limit",
        "5",
        "--json",
    ])
    .expect("scheduled reconciliation worker pass should parse");
    let Commands::Workflow {
        command: WorkflowCommand::Reconcile(scheduled),
    } = scheduled.command
    else {
        panic!("expected scheduled workflow reconcile command");
    };
    assert!(scheduled.apply);
    assert!(scheduled.scheduled);
    assert_eq!(scheduled.limit, 5);

    let missing_apply = Cli::try_parse_from(["ait-cli", "workflow", "reconcile", "--scheduled"])
        .err()
        .expect("scheduled pass must require apply")
        .to_string();
    assert!(missing_apply.contains("required"), "{missing_apply}");
}

#[test]
fn workflow_reconcile_text_surfaces_disposition_and_next_action() {
    let rendered = render_workflow_reconcile_text(&json!({
        "status": "completed",
        "mode": "dry_run",
        "repo_name": "fixture",
        "remote_name": "origin",
        "summary": {"returned_finding_count": 1, "remaining_count": 0},
        "findings": [{
            "code": "worktree.materialization_missing",
            "disposition": "safe_metadata_repair",
            "identities": {"task_id": "RCT-1", "worktree_name": "rct-1"}
        }],
        "next_command": "ait workflow reconcile --remote origin --apply --safe-only"
    }))
    .unwrap();
    assert!(rendered.contains("safe_metadata_repair"));
    assert!(rendered.contains("worktree.materialization_missing"));
    assert!(rendered.contains("ait workflow reconcile --remote origin --apply --safe-only"));
}

#[test]
fn workflow_tier_parser_and_renderer_freeze_the_risk_contract() {
    let parsed = Cli::try_parse_from(["ait", "workflow", "tier", "--json"])
        .expect("workflow tier should parse");
    let Commands::Workflow {
        command: WorkflowCommand::Tier(args),
    } = parsed.command
    else {
        panic!("expected workflow tier command");
    };
    assert!(args.json);
    assert!(!args.verbose);

    let payload = json!({
        "recommended_tier": "quick_modification",
        "quick_allowed": true,
        "changed_path_count": 1,
        "changed_bytes": 12,
        "limits": {"max_files": 8, "max_bytes": 65536},
        "reasons": [{"code": "bounded", "detail": "low risk"}],
        "required_gates": ["fast_validation_evidence", "immutable_snapshot"],
        "ceremony": [
            {"tier": "quick_modification", "minimum_commands": 1, "records_created": 1, "human_decisions": 1, "recovery_steps": 1}
        ],
        "escalation_command": "ait snapshot create --profile quick"
    });
    let rendered = render_workflow_tier_text(&payload, true).expect("workflow tier text");
    assert!(rendered.contains("result: quick_modification"));
    assert!(rendered.contains("commands=1, records=1, decisions=1, recovery=1"));
    assert!(rendered.contains("fast_validation_evidence"));

    let compact = render_workflow_tier_text(&payload, false).expect("compact workflow tier text");
    assert!(compact.contains("result: quick_modification"));
    assert!(compact.contains("reason: bounded — low risk"));
    assert!(compact.contains("next: ait snapshot create --profile quick"));
    assert!(!compact.contains("Ceremony baseline"));
    assert!(!compact.contains("quick limits"));

    let mut worktree_payload = payload.clone();
    worktree_payload["facts"] = json!({
        "is_worktree": true,
        "bound_task_id": "LCT-48",
    });
    worktree_payload["reasons"] = json!([
        {"code": "worktree_scope", "detail": "continue existing lineage"},
        {"code": "file_limit_exceeded", "detail": "too many files"},
    ]);
    let worktree = render_workflow_tier_text(&worktree_payload, false).expect("worktree tier text");
    assert!(worktree.contains("task: LCT-48"));
    assert!(worktree.contains("reason: worktree_scope"));
    assert!(!worktree.contains("file_limit_exceeded"));
    assert!(!worktree.contains("next: ait task start"));

    let guide = workflow_guide_payload(Some("tiers")).expect("tiers guide");
    assert_eq!(guide["contract_version"], "ait.workflow-tier/v1");
    assert!(guide.to_string().contains("--profile quick"));
}

#[test]
fn agent_list_selection_prioritizes_local_drafts_filters_terminal_rows_and_bounds_output() {
    let mut rows = (1..=25)
        .map(|ordinal| {
            json!({
                "task_id": format!("LCT-{ordinal:04}"),
                "status": "active",
                "publication_state": "published",
                "updated_at": format!("2026-08-01T00:{ordinal:02}:00Z"),
            })
        })
        .collect::<Vec<_>>();
    rows.push(json!({
        "task_id": "LCT-9999",
        "status": "active",
        "publication_state": "local_draft",
        "updated_at": "2026-01-01T00:00:00Z",
    }));
    rows.push(json!({
        "task_id": "LCT-9998",
        "status": "completed",
        "publication_state": "local_draft",
        "updated_at": "2026-08-01T23:59:00Z",
    }));

    let (selected, matching, total) =
        select_agent_list_rows(&rows, false, &["completed", "abandoned", "canceled"]);
    assert_eq!(selected.len(), DEFAULT_AGENT_TEXT_LIST_LIMIT);
    assert_eq!(matching, 26);
    assert_eq!(total, 27);
    assert_eq!(selected[0]["task_id"], "LCT-9999");
    assert!(selected.iter().all(|row| row["status"] != "completed"));

    let (all, matching, total) =
        select_agent_list_rows(&rows, true, &["completed", "abandoned", "canceled"]);
    assert_eq!(all.len(), 27);
    assert_eq!(matching, 27);
    assert_eq!(total, 27);
}

#[test]
fn change_text_projection_uses_task_scoped_reference_without_nested_json() {
    let projected = project_change_text_rows(&[json!({
        "task_id": "LCT-48",
        "change_id": "C-01",
        "status": "draft",
    })]);
    assert_eq!(projected[0]["change"], "LCT-48/C-01");
    assert_eq!(projected[0]["status"], "draft");
}

#[test]
fn quick_snapshot_parser_requires_provenance_and_keeps_plain_snapshot_compatible() {
    let parsed = Cli::try_parse_from([
        "ait",
        "snapshot",
        "create",
        "--profile",
        "quick",
        "--intent",
        "Fix a typo",
        "--validation",
        "docs lint passed",
        "--message",
        "Fix typo",
        "--json",
    ])
    .expect("guarded quick Snapshot should parse");
    let Commands::Snapshot {
        command: SnapshotCommand::Create(args),
    } = parsed.command
    else {
        panic!("expected snapshot create command");
    };
    assert_eq!(args.profile, Some(SnapshotProfile::Quick));
    assert_eq!(args.intent.as_deref(), Some("Fix a typo"));
    assert_eq!(args.validation.as_deref(), Some("docs lint passed"));
    assert_eq!(args.message.as_deref(), Some("Fix typo"));
    assert!(args.json);

    let missing_validation = Cli::try_parse_from([
        "ait",
        "snapshot",
        "create",
        "--profile",
        "quick",
        "--intent",
        "Fix a typo",
        "--message",
        "Fix typo",
    ])
    .err()
    .expect("quick profile must require validation evidence")
    .to_string();
    assert!(missing_validation.contains("--validation"));

    let plain = Cli::try_parse_from([
        "ait",
        "snapshot",
        "create",
        "--message",
        "Normal task Snapshot",
    ])
    .expect("plain Snapshot should remain compatible");
    let Commands::Snapshot {
        command: SnapshotCommand::Create(plain),
    } = plain.command
    else {
        panic!("expected plain snapshot create command");
    };
    assert_eq!(plain.profile, None);
    assert_eq!(plain.intent, None);
    assert_eq!(plain.validation, None);
}

#[test]
fn plan_sync_renderer_accepts_success_without_summary() {
    render_sync_like(&json!({
        "target": "docs/sprints/card.md",
        "scope": "remote",
        "mode": "local_publish",
        "status": "ok",
        "results": [],
    }))
    .expect("summary is not part of the plan sync response contract");
}

#[test]
fn plan_sync_renderer_surfaces_structured_failure_without_summary() {
    let error = render_sync_like(&json!({
        "target": "docs/sprints/card.md",
        "status": "failed",
        "error": {"message": "fixture path rejected"},
        "results": [],
    }))
    .expect_err("failed plan sync must return an error");
    assert_eq!(error, "Plan sync failed: fixture path rejected");
}

#[test]
fn task_start_parser_keeps_manual_title_required_and_accepts_plan_source_mode() {
    let manual_error = match Cli::try_parse_from([
        "ait-cli",
        "task",
        "start",
        "--intent",
        "Preserve a meaningful intent",
    ]) {
        Ok(_) => panic!("manual task start must still require --title"),
        Err(error) => error,
    };
    assert!(manual_error.to_string().contains("--title"));

    let parsed = Cli::try_parse_from([
        "ait-cli",
        "task",
        "start",
        "--from",
        "docs/sprints/card.md#card/implement",
        "--intent",
        "Implement the exact synchronized Plan item",
        "--title-override",
        "Implement bounded Plan start",
        "--remote",
        "origin",
        "--json",
    ])
    .expect("Plan source mode should not require --title");
    let Commands::Task {
        command: TaskCommand::Start(args),
    } = parsed.command
    else {
        panic!("expected task start command");
    };
    assert_eq!(
        args.source.as_deref(),
        Some("docs/sprints/card.md#card/implement")
    );
    assert!(args.title.is_none());
    assert_eq!(
        args.title_override.as_deref(),
        Some("Implement bounded Plan start")
    );
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.json);
}

#[test]
fn task_start_parser_rejects_plan_source_conflicts() {
    for conflicting in [vec!["--title", "Duplicate title"], vec!["--task-only"]] {
        let mut args = vec![
            "ait-cli",
            "task",
            "start",
            "--from",
            "docs/sprints/card.md#card/implement",
            "--intent",
            "Implement the exact synchronized Plan item",
        ];
        args.extend(conflicting);
        let error = match Cli::try_parse_from(args) {
            Ok(_) => panic!("conflict should be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("cannot be used with"));
    }
}

#[test]
fn task_start_parser_rejects_retired_explicit_plan_binding_flags() {
    for retired in [
        vec!["--plan", "PR-1"],
        vec!["--revision", "plan-revision:1"],
        vec!["--plan-item-ref", "card/implement"],
    ] {
        let mut args = vec![
            "ait-cli",
            "task",
            "start",
            "--title",
            "Legacy binding",
            "--intent",
            "Prove copied Plan coordinates are no longer public input",
        ];
        args.extend(retired.clone());
        let error = match Cli::try_parse_from(args) {
            Ok(_) => panic!("retired flag must be rejected"),
            Err(error) => error,
        };
        let rendered = error.to_string();
        assert!(rendered.contains("unexpected argument"));
        assert!(rendered.contains(retired[0]));
    }
}

#[test]
fn task_start_progress_renderer_covers_plan_source_phases() {
    assert_eq!(
        task_start_progress_line(&json!({
            "phase": "plan_sync_started",
            "artifact_path": "docs/sprints/card.md",
            "scope": "remote"
        }))
        .as_deref(),
        Some("synchronizing Plan source: docs/sprints/card.md (remote)")
    );
    assert_eq!(
        task_start_progress_line(&json!({
            "phase": "plan_synced",
            "plan_id": "PR-1",
            "plan_revision_id": "plan-revision:2"
        }))
        .as_deref(),
        Some("Plan synchronized: PR-1")
    );
    assert_eq!(
        task_start_progress_line(&json!({
            "phase": "plan_item_validated",
            "plan_item_ref": "card/implement",
            "title_source": "plan_item"
        }))
        .as_deref(),
        Some("Plan item taskable: card/implement (title source: plan_item)")
    );
}

#[test]
fn remote_plan_query_carries_binary_storage_context_without_retired_backend_paths() {
    let temp = TempDir::new().unwrap();
    write_runtime_config(
        temp.path(),
        r#"{
  "repo_name": "fixture",
  "default_remote": "origin",
  "remotes": {
    "origin": {
      "url": "http://example.test/fixture",
      "repo_name": "fixture"
    }
  }
}"#,
    );
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let payload = parse_value_error_string(
        &build_query_request(
            &repo,
            &QueryScopeArgs {
                local: false,
                remote: Some("origin".to_string()),
                json: true,
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(payload["scope"], "remote");
    assert_eq!(payload["plan_storage"]["write_layout"], 1);
    assert_eq!(
        payload["plan_storage"]["activation_pointer"],
        repo.authoritative_repo_root()
            .join(".ait/binary-db")
            .to_string_lossy()
            .as_ref()
    );
    assert!(payload["plan_storage"].get("mode").is_none());
}

#[test]
fn local_task_land_render_compacts_successful_closeout_and_keeps_material_retention() {
    let rendered = render_workflow_land_local_text(&json!({
        "change_id": "LCC-1",
        "target_line": "main",
        "landed_snapshot_id": "SNP-1",
        "change_status": "landed",
        "task_status": "completed",
        "workspace_action": "materialized",
        "bound_line_closeout": {
            "status": "archived",
            "line_name": "feature/lct-1"
        },
        "plan_checklist_closeout": {
            "status": "synced",
            "reason": "checklist_closed",
            "retention": {
                "status": "pruned",
                "removed_count": 3,
                "retained_completed_count": 20
            }
        }
    }))
    .unwrap();

    assert!(rendered.contains("landed: LCC-1 -> main @ SNP-1"));
    assert!(rendered.contains("closed: task, line, sprint"));
    assert!(rendered.contains("retention: pruned (3 removed, 20 retained)"));
    assert!(!rendered.contains("checklist reason"));
}

#[test]
fn remote_task_land_render_surfaces_separate_plan_sync_action() {
    let rendered = render_task_land_text(&json!({
        "change": {
            "change_id": "RCT-1/C-01",
            "status": "landed",
            "base_line": "main"
        },
        "task": {
            "task_id": "RCT-1"
        },
        "apply_status": "done",
        "bound_line_closeout": {
            "status": "archived",
            "reason": "final_task_completed",
            "line_name": "feature/rct-1"
        },
        "plan_checklist_closeout": {
            "status": "deferred",
            "reason": "remote_plan_sync_is_separate_from_task_land",
            "detail": "Remote task land completed without reading or synchronizing Plan state.",
            "command": "ait plan sync <bound-sprint-card-path> --remote origin"
        }
    }))
    .unwrap();

    assert!(rendered.contains("ait task land"));
    assert!(rendered.contains("Feature Line closeout"));
    assert!(rendered.contains("- archived: feature/rct-1"));
    assert!(rendered.contains("Sprint checklist closeout"));
    assert!(rendered.contains("- deferred"));
    assert!(rendered
        .contains("Remote task land completed without reading or synchronizing Plan state."));
    assert!(rendered.contains("ait plan sync <bound-sprint-card-path> --remote origin"));
}

#[test]
fn task_land_render_surfaces_versioned_partial_recovery() {
    let rendered = render_task_land_text(&json!({
        "mode": "local",
        "apply_status": "done",
        "change_id": "LCC-9",
        "target_line": "main",
        "landed_snapshot_id": "SNP-9",
        "change_status": "landed",
        "task_status": "completed",
        "workspace_action": "unchanged",
        "closeout_status": "partial",
        "task_land_contract": {
            "version": "task-land-plan-closeout/v1",
            "scope": "local"
        },
        "closeout_recovery": {
            "detail": "Repair the Plan drift and rerun task land.",
            "command": "ait task land LCC-9 --local"
        },
        "plan_checklist_closeout": {
            "status": "skipped",
            "reason": "artifact_has_unsynced_drift"
        }
    }))
    .unwrap();

    assert!(rendered.contains("task-land contract: task-land-plan-closeout/v1"));
    assert!(rendered.contains("closeout: partial"));
    assert!(rendered.contains("Repair the Plan drift and rerun task land."));
    assert!(rendered.contains("ait task land LCC-9 --local"));
}

#[test]
fn workflow_preview_derives_clean_workspace_state_from_boolean_authority() {
    let rendered = render_workflow_phase_text(
        &json!({
            "change": {"change_id": "LCT-1/C-01", "status": "draft", "base_line": "main"},
            "task": {"task_id": "LCT-1"},
            "workspace": {
                "current_line": "feature/lct-1",
                "clean": true,
                "changed_count": 0
            },
            "apply_status": "preview"
        }),
        "land",
    )
    .unwrap();

    assert!(rendered.contains("- workspace: clean (0 changed)"));
    assert!(!rendered.contains("workspace: unknown"));
}

#[test]
fn task_audit_labels_expected_work_as_pending_not_blocked() {
    assert_eq!(
        task_audit_reason_label("continue_task_work"),
        Some("pending")
    );
    assert_eq!(task_audit_reason_label("create_change"), Some("pending"));
    assert_eq!(
        task_audit_reason_label("external_readiness_blocked"),
        Some("blocker")
    );
    assert_eq!(task_audit_reason_label("none"), None);
}

#[test]
fn compact_status_keeps_actionable_reconciliation_next_commands() {
    let reconciliation = json!({
        "next_command": "ait workflow reconcile --apply --safe-only"
    });
    assert_eq!(
        status_reconciliation_next(&reconciliation, 3, 2, 1),
        "ait workflow reconcile --apply --safe-only"
    );
    assert_eq!(
        status_reconciliation_next(&reconciliation, 0, 2, 1),
        "ait workflow reconcile --dry-run"
    );
    assert_eq!(
        status_reconciliation_next(&json!({}), 1, 0, 0),
        "ait workflow reconcile --dry-run"
    );
    assert_eq!(status_reconciliation_next(&reconciliation, 0, 0, 0), "");
}

#[test]
fn change_command_parser_accepts_standalone_create() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "change",
        "create",
        "RCT-1",
        "--title",
        "Recover task",
        "--base-line",
        "main",
        "--remote",
        "origin",
        "--json",
    ])
    .expect("change create should parse");
    let Commands::Change {
        command: ChangeCommand::Create(args),
    } = parsed.command
    else {
        panic!("expected change create command");
    };
    assert_eq!(args.task_id, "RCT-1");
    assert_eq!(args.title, "Recover task");
    assert_eq!(args.base_line, "main");
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.json);
}

#[test]
fn snapshot_ancestry_parser_freezes_direction_bounds_and_exit_help() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "snapshot",
        "ancestry",
        "SNP-MERGE",
        "--descendants",
        "--first-parent",
        "--max-depth",
        "42",
        "--limit",
        "7",
        "--all",
        "--json",
    ])
    .expect("bounded descendant query should parse");
    let Commands::Snapshot {
        command: SnapshotCommand::Ancestry(args),
    } = parsed.command
    else {
        panic!("expected snapshot ancestry command");
    };
    assert_eq!(args.snapshot_id, "SNP-MERGE");
    assert!(!args.ancestors);
    assert!(args.descendants);
    assert!(args.first_parent);
    assert_eq!(args.max_depth, 42);
    assert_eq!(args.limit, 7);
    assert!(args.all);
    assert!(args.json);

    let conflict = match Cli::try_parse_from([
        "ait-cli",
        "snapshot",
        "ancestry",
        "SNP-MERGE",
        "--ancestors",
        "--descendants",
    ]) {
        Ok(_) => panic!("ancestry directions must be mutually exclusive"),
        Err(error) => error.to_string(),
    };
    assert!(conflict.contains("cannot be used with"));

    let help = match Cli::try_parse_from(["ait-cli", "snapshot", "is-ancestor", "--help"]) {
        Ok(_) => panic!("--help should render clap help"),
        Err(error) => error.to_string(),
    };
    assert!(help.contains("exits 0 when true, 1 when false, and 2"));
    assert!(help.contains("<OLDER_SNAPSHOT_ID> <NEWER_SNAPSHOT_ID>"));
}

#[test]
fn decision_complete_detail_flags_parse_for_snapshot_plan_and_cleanup() {
    let snapshot = Cli::try_parse_from(["ait-cli", "snapshot", "show", "SNP-DETAIL", "--files"])
        .expect("snapshot full-tree flag should parse");
    let Commands::Snapshot {
        command: SnapshotCommand::Show(snapshot),
    } = snapshot.command
    else {
        panic!("expected snapshot show command");
    };
    assert_eq!(snapshot.snapshot_id, "SNP-DETAIL");
    assert!(snapshot.files);

    let plan = Cli::try_parse_from(["ait-cli", "plan", "revisions", "PR-42", "--local", "--all"])
        .expect("complete Plan revision history flag should parse");
    let Commands::Plan {
        command: PlanCommand::Revisions(plan),
    } = plan.command
    else {
        panic!("expected Plan revisions command");
    };
    assert_eq!(plan.plan_id, "PR-42");
    assert!(plan.scope.local);
    assert!(plan.all);

    let cleanup = Cli::try_parse_from([
        "ait-cli",
        "line",
        "cleanup-candidates",
        "--older-than",
        "30d",
        "--kind",
        "merged",
        "--include-protected",
        "--all",
    ])
    .expect("complete Line cleanup evidence flag should parse");
    let Commands::Line {
        command: LineCommand::CleanupCandidates(cleanup),
    } = cleanup.command
    else {
        panic!("expected Line cleanup-candidates command");
    };
    assert_eq!(cleanup.older_than, "30d");
    assert_eq!(cleanup.cleanup_kind.as_deref(), Some("merged"));
    assert!(cleanup.include_protected);
    assert!(cleanup.all);
}

#[test]
fn line_merge_parser_freezes_start_continue_abort_and_conflicts() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "line",
        "merge",
        "feature/source",
        "--into",
        "feature/target",
        "--message",
        "Merge the bounded source",
        "--json",
    ])
    .expect("line merge start should parse");
    let Commands::Line {
        command: LineCommand::Merge(args),
    } = parsed.command
    else {
        panic!("expected line merge command");
    };
    assert_eq!(args.source.as_deref(), Some("feature/source"));
    assert_eq!(args.target.as_deref(), Some("feature/target"));
    assert_eq!(args.message.as_deref(), Some("Merge the bounded source"));
    assert!(!args.continue_merge);
    assert!(!args.abort_merge);
    assert!(args.json);

    let continued = Cli::try_parse_from([
        "ait-cli",
        "line",
        "merge",
        "--continue",
        "--message",
        "Resolved merge",
    ])
    .expect("line merge --continue should accept an optional message");
    let Commands::Line {
        command: LineCommand::Merge(args),
    } = continued.command
    else {
        panic!("expected line merge --continue command");
    };
    assert!(args.source.is_none());
    assert!(args.continue_merge);
    assert_eq!(args.message.as_deref(), Some("Resolved merge"));

    let aborted = Cli::try_parse_from(["ait-cli", "line", "merge", "--abort", "--json"])
        .expect("line merge --abort should parse");
    let Commands::Line {
        command: LineCommand::Merge(args),
    } = aborted.command
    else {
        panic!("expected line merge --abort command");
    };
    assert!(args.abort_merge);
    assert!(args.json);

    for conflicting in [
        vec!["feature/source", "--continue"],
        vec!["--continue", "--abort"],
        vec!["--abort", "--message", "not allowed"],
        vec!["--abort", "--into", "feature/target"],
    ] {
        let result =
            Cli::try_parse_from(["ait-cli", "line", "merge"].into_iter().chain(conflicting));
        let error = match result {
            Ok(_) => panic!("line merge mode conflict should be rejected"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("cannot be used with"), "{error}");
    }

    let help = match Cli::try_parse_from(["ait-cli", "line", "merge", "--help"]) {
        Ok(_) => panic!("--help should render clap help"),
        Err(error) => error.to_string(),
    };
    assert!(help.contains("two-parent Snapshot"));
    assert!(help.contains("--continue"));
    assert!(help.contains("--abort"));
}

#[test]
fn line_lifecycle_parser_freezes_rename_and_confirmed_delete() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "line",
        "rename",
        "topic/old",
        "topic/new",
        "--remote",
        "origin",
        "--json",
    ])
    .expect("line rename should parse");
    let Commands::Line {
        command: LineCommand::Rename(args),
    } = parsed.command
    else {
        panic!("expected line rename command");
    };
    assert_eq!(args.old, "topic/old");
    assert_eq!(args.new, "topic/new");
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.json);

    let parsed =
        Cli::try_parse_from(["ait-cli", "line", "delete", "topic/dead", "--yes", "--json"])
            .expect("confirmed line delete should parse");
    let Commands::Line {
        command: LineCommand::Delete(args),
    } = parsed.command
    else {
        panic!("expected line delete command");
    };
    assert_eq!(args.name, "topic/dead");
    assert!(args.yes);
    assert!(args.json);

    let error = Cli::try_parse_from(["ait-cli", "line", "delete", "topic/dead"])
        .err()
        .expect("delete without --yes must fail")
        .to_string();
    assert!(error.contains("--yes"), "{error}");
}

#[test]
fn git_interop_parser_freezes_resumable_import_export_contract() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "git",
        "import",
        "../legacy.git",
        "--all-refs",
        "--resume",
        "--json",
    ])
    .expect("git import should parse");
    let Commands::Git {
        command: GitCommand::Import(args),
    } = parsed.command
    else {
        panic!("expected git import command");
    };
    assert_eq!(args.source, "../legacy.git");
    assert!(args.all_refs);
    assert!(args.resume);
    assert!(!args.dry_run);
    assert!(args.json);

    let parsed = Cli::try_parse_from([
        "ait-cli",
        "git",
        "export",
        "../escape.git",
        "--dry-run",
        "--json",
    ])
    .expect("git export should parse");
    let Commands::Git {
        command: GitCommand::Export(args),
    } = parsed.command
    else {
        panic!("expected git export command");
    };
    assert_eq!(args.target, "../escape.git");
    assert!(!args.all_refs);
    assert!(args.dry_run);
    assert!(!args.resume);
    assert!(args.json);

    for mode in ["import", "export"] {
        let error = Cli::try_parse_from([
            "ait-cli",
            "git",
            mode,
            "fixture.git",
            "--dry-run",
            "--resume",
        ])
        .err()
        .expect("dry-run and resume must conflict")
        .to_string();
        assert!(error.contains("cannot be used with"), "{error}");
    }

    let parsed = Cli::try_parse_from([
        "ait-cli",
        "git",
        "mirror",
        "../team.git",
        "--direction",
        "bidirectional",
        "--dry-run",
        "--once",
        "--json",
    ])
    .expect("git mirror should parse");
    let Commands::Git {
        command: GitCommand::Mirror(args),
    } = parsed.command
    else {
        panic!("expected git mirror command");
    };
    assert_eq!(args.endpoint, "../team.git");
    assert_eq!(args.direction, "bidirectional");
    assert!(args.dry_run);
    assert!(args.once);
    assert!(args.json);

    let error = Cli::try_parse_from([
        "ait-cli",
        "git",
        "mirror",
        "fixture.git",
        "--direction",
        "force",
    ])
    .err()
    .expect("unknown mirror direction must fail")
    .to_string();
    assert!(error.contains("possible values"), "{error}");
}

#[test]
fn pull_parser_freezes_explicit_divergence_strategy_and_force_conflict() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "pull",
        "--remote",
        "origin",
        "--line",
        "feature/demo",
        "--merge",
        "--restore",
        "--json",
    ])
    .expect("pull --merge should parse");
    let Commands::Pull(args) = parsed.command else {
        panic!("expected pull command");
    };
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert_eq!(args.line.as_deref(), Some("feature/demo"));
    assert!(args.merge);
    assert!(args.restore);
    assert!(!args.force);
    assert!(args.json);

    let error = match Cli::try_parse_from([
        "ait-cli",
        "pull",
        "--line",
        "feature/demo",
        "--merge",
        "--restore",
        "--force",
    ]) {
        Ok(_) => panic!("pull merge must reject forced workspace overwrite"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("cannot be used with"), "{error}");
}

#[test]
fn stash_command_parser_accepts_the_complete_native_surface() {
    let cases = [
        (
            vec![
                "stash",
                "save",
                "--message",
                "parked",
                "--keep-workspace",
                "--json",
            ],
            "save",
        ),
        (vec!["stash", "list", "--json"], "list"),
        (vec!["stash", "show", "STH-000001", "--json"], "show"),
        (
            vec!["stash", "apply", "STH-000001", "--force", "--json"],
            "apply",
        ),
        (
            vec!["stash", "pop", "STH-000001", "--force", "--json"],
            "pop",
        ),
        (vec!["stash", "drop", "STH-000001", "--json"], "drop"),
    ];

    for (args, expected) in cases {
        let parsed = Cli::try_parse_from(std::iter::once("ait-cli").chain(args)).unwrap();
        let Commands::Stash { command } = parsed.command else {
            panic!("expected native stash command");
        };
        let actual = match command {
            StashCommand::Save(args) => {
                assert_eq!(args.message.as_deref(), Some("parked"));
                assert!(args.keep_workspace);
                assert!(args.json);
                "save"
            }
            StashCommand::List(args) => {
                assert!(args.json);
                "list"
            }
            StashCommand::Show(args) => {
                assert_eq!(args.stash_id, "STH-000001");
                assert!(args.json);
                "show"
            }
            StashCommand::Apply(args) => {
                assert_eq!(args.stash_id, "STH-000001");
                assert!(args.force);
                assert!(args.json);
                "apply"
            }
            StashCommand::Pop(args) => {
                assert_eq!(args.stash_id, "STH-000001");
                assert!(args.force);
                assert!(args.json);
                "pop"
            }
            StashCommand::Drop(args) => {
                assert_eq!(args.stash_id, "STH-000001");
                assert!(args.json);
                "drop"
            }
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn worktree_recover_task_parser_accepts_exact_remote_task_and_change() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "worktree",
        "recover-task",
        "RSET-0500",
        "--change",
        "RSET-0500/C-01",
        "--remote",
        "origin",
        "--dry-run",
        "--json",
    ])
    .expect("worktree recover-task should parse");
    let Commands::Worktree {
        command: WorktreeCommand::RecoverTask(args),
    } = parsed.command
    else {
        panic!("expected worktree recover-task command");
    };
    assert_eq!(args.task_id, "RSET-0500");
    assert_eq!(args.change, "RSET-0500/C-01");
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.dry_run);
    assert!(args.json);
}

#[test]
fn patchset_publish_parser_rejects_allow_empty() {
    let err = match Cli::try_parse_from([
        "ait-cli",
        "patchset",
        "publish",
        "--change",
        "RCC-1",
        "--summary",
        "Blocked",
        "--allow-empty",
    ]) {
        Ok(_) => panic!("--allow-empty should be rejected"),
        Err(err) => err,
    };
    let rendered = err.to_string();
    assert!(
        rendered.contains("unexpected argument '--allow-empty'"),
        "expected parser error to reject --allow-empty, got: {rendered}"
    );
}

#[test]
fn patchset_ci_parser_accepts_native_release_artifact_smoke() {
    let command = Cli::try_parse_from([
        "ait-cli",
        "test",
        "patchset-ci",
        "release-artifact-smoke",
        "--json",
    ])
    .unwrap()
    .command;

    match command {
        Commands::Test {
            command:
                TestCommand::PatchsetCi {
                    command: PatchsetCiSmokeCommand::ReleaseArtifactSmoke(args),
                },
        } => assert!(args.json),
        _ => panic!("expected native release-artifact-smoke command"),
    }
}

#[test]
fn release_build_parser_accepts_explicit_native_matrix_directory() {
    let command = Cli::try_parse_from([
        "ait-cli",
        "release",
        "build",
        "REL-1",
        "--native-matrix-dir",
        "/tmp/ait-native-matrix",
        "--json",
    ])
    .unwrap()
    .command;

    match command {
        Commands::Release {
            command: ReleaseCommand::Build(args),
        } => {
            assert_eq!(args.release_id, "REL-1");
            assert!(args.receipts.is_none());
            assert_eq!(
                args.native_matrix_dir,
                Some(PathBuf::from("/tmp/ait-native-matrix"))
            );
            assert!(args.json);
        }
        _ => panic!("expected release build command"),
    }
}

#[test]
fn release_adapter_parser_accepts_snapshot_derived_check_and_build() {
    for subcommand in ["check", "build"] {
        let command = Cli::try_parse_from([
            "ait-cli",
            "release",
            "adapter",
            subcommand,
            "--version",
            "1.2.3",
            "--line",
            "main",
            "--json",
        ])
        .unwrap()
        .command;

        match command {
            Commands::Release {
                command: ReleaseCommand::Adapter { command },
            } => {
                let args = match command {
                    ReleaseAdapterCommand::Check(args) | ReleaseAdapterCommand::Build(args) => args,
                };
                assert_eq!(args.version, "1.2.3");
                assert_eq!(args.line_name, "main");
                assert!(args.target.is_none());
                assert!(args.json);
            }
            _ => panic!("expected release adapter {subcommand} command"),
        }
    }
}

#[test]
fn release_family_parser_accepts_public_rc_lifecycle_arguments() {
    let candidate = Cli::try_parse_from([
        "ait-cli",
        "release",
        "candidate",
        "create",
        "--version",
        "1.0.0-rc.1",
        "--channel",
        "rc",
        "--json",
    ])
    .unwrap()
    .command;
    match candidate {
        Commands::Release {
            command:
                ReleaseCommand::Candidate {
                    command: ReleaseCandidateCommand::Create(args),
                },
        } => {
            assert_eq!(args.version, "1.0.0-rc.1");
            assert_eq!(args.channel.as_deref(), Some("rc"));
            assert!(args.profile.is_none());
            assert!(args.json);
        }
        _ => panic!("expected family release candidate command"),
    }

    let checked = Cli::try_parse_from([
        "ait-cli",
        "release",
        "check",
        "REL-FAM-0123456789ABCDEF",
        "--receipts",
        "/tmp/receipts",
        "--public-source-root",
        "/tmp/public-source",
        "--json",
    ])
    .unwrap()
    .command;
    match checked {
        Commands::Release {
            command: ReleaseCommand::Check(args),
        } => {
            assert_eq!(args.receipts, Some(PathBuf::from("/tmp/receipts")));
            assert_eq!(
                args.public_source_root,
                Some(PathBuf::from("/tmp/public-source"))
            );
        }
        _ => panic!("expected family release check command"),
    }

    let built = Cli::try_parse_from([
        "ait-cli",
        "release",
        "build",
        "REL-FAM-0123456789ABCDEF",
        "--receipts",
        "/tmp/receipts",
        "--public-source-root",
        "/tmp/public-source",
        "--json",
    ])
    .unwrap()
    .command;
    match built {
        Commands::Release {
            command: ReleaseCommand::Build(args),
        } => {
            assert_eq!(args.receipts, Some(PathBuf::from("/tmp/receipts")));
            assert_eq!(
                args.public_source_root,
                Some(PathBuf::from("/tmp/public-source"))
            );
        }
        _ => panic!("expected family release build command"),
    }

    let promoted = Cli::try_parse_from([
        "ait-cli",
        "release",
        "promote",
        "REL-FAM-0123456789ABCDEF",
        "--channel",
        "rc",
        "--json",
    ])
    .unwrap()
    .command;
    match promoted {
        Commands::Release {
            command: ReleaseCommand::Promote(args),
        } => {
            assert_eq!(args.release_id, "REL-FAM-0123456789ABCDEF");
            assert_eq!(args.channel, "rc");
            assert!(args.json);
        }
        _ => panic!("expected family release promote command"),
    }
}

#[test]
fn release_native_source_parser_requires_explicit_target_and_runner_facts() {
    let command = Cli::try_parse_from([
        "ait-cli",
        "release",
        "native-source",
        "REL-1",
        "--target",
        "aarch64-apple-darwin",
        "--source-dir",
        "/tmp/ait-native-matrix/aarch64-apple-darwin/release",
        "--runner",
        "macos-15",
        "--runner-image",
        "macos-15-arm64",
        "--rust-toolchain",
        "1.96.0",
        "--rustc-path",
        "/tmp/rustc",
        "--json",
    ])
    .unwrap()
    .command;

    match command {
        Commands::Release {
            command: ReleaseCommand::NativeSource(args),
        } => {
            assert_eq!(args.release_id, "REL-1");
            assert_eq!(args.target, "aarch64-apple-darwin");
            assert_eq!(args.runner, "macos-15");
            assert_eq!(args.runner_image, "macos-15-arm64");
            assert_eq!(args.rust_toolchain, "1.96.0");
            assert!(args.json);
        }
        _ => panic!("expected release native-source command"),
    }
}

#[test]
fn release_native_bundle_parser_requires_explicit_matrix_directory() {
    let command = Cli::try_parse_from([
        "ait-cli",
        "release",
        "native-bundle",
        "REL-1",
        "--native-matrix-dir",
        "/tmp/ait-native-matrix",
        "--json",
    ])
    .unwrap()
    .command;

    match command {
        Commands::Release {
            command: ReleaseCommand::NativeBundle(args),
        } => {
            assert_eq!(args.release_id, "REL-1");
            assert_eq!(
                args.native_matrix_dir,
                PathBuf::from("/tmp/ait-native-matrix")
            );
            assert!(args.json);
        }
        _ => panic!("expected release native-bundle command"),
    }

    assert!(
        Cli::try_parse_from(["ait-cli", "release", "native-bundle", "REL-1", "--json",]).is_err()
    );
}

#[test]
fn review_team_command_rejects_non_team_remote_mode() {
    let temp = TempDir::new().unwrap();
    write_runtime_config(
        temp.path(),
        r#"{"repo_name":"fixture","workflow_mode":"solo_remote","workflow_default_scope":"remote","task_default_scope":"remote","change_default_scope":"remote"}"#,
    );
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let result = run_review(
        repo,
        ReviewCommand::Team {
            command: ReviewTeamCommand::Approve(ReviewApproveArgs {
                change_id: "RC-1".to_string(),
                reviewer: None,
                patchset_id: Some("RP-1".to_string()),
                message: None,
                remote: Some("origin".to_string()),
                json: false,
            }),
        },
    );
    assert!(result.is_err());
    assert!(result
        .err()
        .unwrap()
        .contains("only available when `workflow_mode=team_remote`"));
}

fn parse_external_command(args: &[&str]) -> ExternalCommand {
    match Cli::try_parse_from(args).unwrap().command {
        Commands::External { command } => command,
        _ => panic!("expected external command"),
    }
}

#[test]
fn repository_retirement_restore_and_discard_flags_are_explicit() {
    let retire = Cli::try_parse_from([
        "ait-cli",
        "repo",
        "retire",
        "--remote",
        "origin",
        "--replace-export",
        "--json",
    ])
    .expect("parse repo retire");
    let Commands::Repo {
        command: RepoCommand::Retire(args),
    } = retire.command
    else {
        panic!("expected repo retire");
    };
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.replace_export);
    assert!(args.json);
    let request =
        build_repo_command_request(RepoCommand::Retire(args)).expect("build repo retire request");
    assert_eq!(request.command, "retire");
    assert_eq!(request.args["abort"], json!(false));
    assert_eq!(request.args["replace_export"], json!(true));

    let abort = Cli::try_parse_from([
        "ait-cli", "repo", "retire", "--remote", "origin", "--abort", "--json",
    ])
    .expect("parse repo retirement abort");
    let Commands::Repo {
        command: RepoCommand::Retire(args),
    } = abort.command
    else {
        panic!("expected repo retirement abort");
    };
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.abort);
    assert!(!args.replace_export);
    let request = build_repo_command_request(RepoCommand::Retire(args))
        .expect("build repo retirement abort request");
    assert_eq!(request.args["abort"], json!(true));
    assert_eq!(request.args["replace_export"], json!(false));

    let conflict =
        match Cli::try_parse_from(["ait-cli", "repo", "retire", "--abort", "--replace-export"]) {
            Err(error) => error,
            Ok(_) => panic!("abort and replace-export must conflict"),
        };
    assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);

    let restore =
        Cli::try_parse_from(["ait-cli", "repo", "restore", "--remote", "origin", "--json"])
            .expect("parse repo restore");
    assert!(matches!(
        restore.command,
        Commands::Repo {
            command: RepoCommand::Restore(RemoteJsonArgs {
                remote: Some(_),
                json: true,
            }),
        }
    ));

    let discard = Cli::try_parse_from([
        "ait-cli",
        "remote",
        "add",
        "origin",
        "https://example.test",
        "--discard-export",
    ])
    .expect("parse remote add discard");
    assert!(matches!(
        discard.command,
        Commands::Remote {
            command: RemoteCommand::Add(RemoteAddArgs {
                discard_export: true,
                ..
            }),
        }
    ));
}

#[test]
fn external_command_parser_covers_status_doctor_update_link_and_unlink_modes() {
    match parse_external_command(&["ait-cli", "external", "status", "--json"]) {
        ExternalCommand::Status(args) => assert!(args.json),
        _ => panic!("expected status"),
    }
    match parse_external_command(&["ait-cli", "external", "doctor"]) {
        ExternalCommand::Doctor(args) => assert!(!args.json),
        _ => panic!("expected doctor"),
    }
    match parse_external_command(&["ait-cli", "external", "update", "--locked"]) {
        ExternalCommand::Update(args) => {
            let options = external_update_options_from_args(&args).unwrap();
            assert!(matches!(
                options.selection,
                ExternalUpdateSelection::ManifestPins
            ));
            assert!(options.locked);
        }
        _ => panic!("expected update"),
    }
    match parse_external_command(&[
        "ait-cli",
        "external",
        "update",
        "ait-db",
        "--to",
        "SNP-DB-NEW",
        "--validate",
        "--no-recursive",
        "--json",
    ]) {
        ExternalCommand::Update(args) => {
            assert!(args.json);
            let options = external_update_options_from_args(&args).unwrap();
            assert!(matches!(
                options.selection,
                ExternalUpdateSelection::Exact { .. }
            ));
            assert!(options.validate);
            assert!(options.no_recursive);
        }
        _ => panic!("expected update --to"),
    }
    match parse_external_command(&["ait-cli", "external", "update", "ait-db", "--latest"]) {
        ExternalCommand::Update(args) => {
            let options = external_update_options_from_args(&args).unwrap();
            assert!(matches!(
                options.selection,
                ExternalUpdateSelection::Latest { .. }
            ));
        }
        _ => panic!("expected update --latest"),
    }
    match parse_external_command(&["ait-cli", "external", "link", "ait-db", "../ait-db"]) {
        ExternalCommand::Link(args) => {
            assert_eq!(args.name, "ait-db");
            assert_eq!(args.path, "../ait-db");
        }
        _ => panic!("expected link"),
    }
    match parse_external_command(&["ait-cli", "external", "unlink", "ait-db", "--json"]) {
        ExternalCommand::Unlink(args) => {
            assert_eq!(args.name, "ait-db");
            assert!(args.json);
        }
        _ => panic!("expected unlink"),
    }
}

#[test]
fn external_command_parser_rejects_sync_and_pin_subcommands() {
    for forbidden in ["sync", "pin"] {
        let result = Cli::try_parse_from(["ait-cli", "external", forbidden]);
        assert!(result.is_err());
        let rendered = result.err().unwrap().to_string();
        assert!(
            rendered.contains(forbidden),
            "expected parser error to mention forbidden subcommand {forbidden:?}: {rendered}"
        );
    }
}

#[test]
fn external_update_option_builder_rejects_incomplete_modes() {
    let name_without_mode =
        match parse_external_command(&["ait-cli", "external", "update", "ait-db"]) {
            ExternalCommand::Update(args) => external_update_options_from_args(&args),
            _ => panic!("expected update"),
        };
    assert!(name_without_mode.unwrap_err().contains("requires"));

    let to_without_name =
        match parse_external_command(&["ait-cli", "external", "update", "--to", "SNP-DB-NEW"]) {
            ExternalCommand::Update(args) => external_update_options_from_args(&args),
            _ => panic!("expected update --to"),
        };
    assert!(to_without_name
        .unwrap_err()
        .contains("requires an external name"));

    let latest_without_name =
        match parse_external_command(&["ait-cli", "external", "update", "--latest"]) {
            ExternalCommand::Update(args) => external_update_options_from_args(&args),
            _ => panic!("expected update --latest"),
        };
    assert!(latest_without_name
        .unwrap_err()
        .contains("requires an external name"));

    let conflicting_modes = match parse_external_command(&[
        "ait-cli",
        "external",
        "update",
        "ait-db",
        "--to",
        "SNP-DB-NEW",
        "--latest",
    ]) {
        ExternalCommand::Update(args) => external_update_options_from_args(&args),
        _ => panic!("expected conflicting update"),
    };
    assert!(conflicting_modes.unwrap_err().contains("either `--to"));

    let locked_with_target = match parse_external_command(&[
        "ait-cli",
        "external",
        "update",
        "ait-db",
        "--to",
        "SNP-DB-NEW",
        "--locked",
    ]) {
        ExternalCommand::Update(args) => external_update_options_from_args(&args),
        _ => panic!("expected locked update target"),
    };
    assert!(locked_with_target.unwrap_err().contains("does not accept"));
}
