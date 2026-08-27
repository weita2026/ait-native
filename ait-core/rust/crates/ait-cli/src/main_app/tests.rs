use super::*;
use ait_core::external::update::ExternalUpdateSelection;
use clap::CommandFactory;
use std::fs;
use tempfile::TempDir;

#[test]
fn embedded_entry_returns_help_and_parse_status_without_exiting() {
    assert_eq!(entry_with_args(Vec::new()), 0);
    assert_eq!(entry_with_args(vec!["ait".into()]), 0);
    assert_eq!(entry_with_args(vec!["ait".into(), "--help".into()]), 0);
    assert_eq!(
        entry_with_args(vec!["ait".into(), "not-a-command".into()]),
        2
    );
}

#[test]
fn root_command_inventory_is_frozen() {
    let command = Cli::command();
    let visible = command
        .get_subcommands()
        .filter(|subcommand| !subcommand.is_hide_set())
        .map(|subcommand| subcommand.get_name())
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        [
            "init", "line", "git", "blame", "doctor", "queue", "remote", "release", "repo",
            "config", "external", "status", "diff", "pull", "push", "gc", "stash", "plan", "task",
            "change", "snapshot", "commit", "tag", "patchset", "review", "attest", "policy",
            "worktree", "workflow",
        ]
    );
    assert_eq!(visible.len(), 29);
    assert!(!visible.contains(&"agent"));
    assert!(!visible.contains(&"branch"));

    let root_help = Cli::try_parse_from(["ait-cli", "--help"])
        .err()
        .expect("root --help must render Clap help")
        .to_string();
    assert!(root_help.contains("line"), "{root_help}");
    assert!(root_help.contains("[alias: branch]"), "{root_help}");
    assert!(root_help.contains("commit"), "{root_help}");

    let hidden = command
        .get_subcommands()
        .filter(|subcommand| subcommand.is_hide_set())
        .map(|subcommand| subcommand.get_name())
        .collect::<Vec<_>>();
    assert_eq!(hidden, ["binary-db", "current-source-cache", "auth"]);
}

#[test]
fn root_help_hides_auth_while_exact_dormant_invocation_remains_parseable() {
    let root_help = Cli::try_parse_from(["ait-cli", "--help"])
        .err()
        .expect("root --help must render Clap help")
        .to_string();
    assert!(
        !root_help
            .lines()
            .any(|line| line.trim_start().starts_with("auth")),
        "{root_help}"
    );

    let auth_help = Cli::try_parse_from(["ait-cli", "auth", "--help"])
        .err()
        .expect("exact dormant auth invocation must remain parseable")
        .to_string();
    for command in ["whoami", "grant", "bindings"] {
        assert!(auth_help.contains(command), "{auth_help}");
    }
}

#[test]
fn tag_parser_exposes_only_immutable_local_tag_contract() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "tag",
        "create",
        "stable/v1",
        "--snapshot",
        "SNP-123",
        "--message",
        "stable release",
        "--json",
    ])
    .expect("tag create should accept the complete immutable Tag contract");
    let Commands::Tag {
        command: TagCommand::Create(args),
    } = parsed.command
    else {
        panic!("expected tag create command");
    };
    assert_eq!(args.name, "stable/v1");
    assert_eq!(args.snapshot.as_deref(), Some("SNP-123"));
    assert_eq!(args.message, "stable release");
    assert!(args.json);

    let defaulted = Cli::try_parse_from([
        "ait-cli",
        "tag",
        "create",
        "stable/current",
        "--message",
        "current line head",
    ])
    .expect("tag create should default to the current Line head");
    let Commands::Tag {
        command: TagCommand::Create(args),
    } = defaulted.command
    else {
        panic!("expected tag create command");
    };
    assert_eq!(args.snapshot, None);
    assert!(!args.json);

    let root_help = Cli::try_parse_from(["ait-cli", "tag", "--help"])
        .err()
        .expect("tag --help must render Clap help")
        .to_string();
    for text in ["local-only AIT Tags", "create", "list", "show", "delete"] {
        assert!(root_help.contains(text), "{root_help}");
    }

    let create_help = Cli::try_parse_from(["ait-cli", "tag", "create", "--help"])
        .err()
        .expect("tag create --help must render Clap help")
        .to_string();
    for text in [
        "existing Tag name is always rejected",
        "current Line head",
        "<NAME>",
        "--snapshot <SNAPSHOT_ID>",
        "--message <MESSAGE>",
        "--json",
    ] {
        assert!(create_help.contains(text), "{create_help}");
    }
    assert!(!create_help.contains("--force"), "{create_help}");

    let list_help = Cli::try_parse_from(["ait-cli", "tag", "list", "--help"])
        .err()
        .expect("tag list --help must render Clap help")
        .to_string();
    assert!(list_help.contains("without changing"), "{list_help}");
    assert!(list_help.contains("--json"), "{list_help}");

    let show_help = Cli::try_parse_from(["ait-cli", "tag", "show", "--help"])
        .err()
        .expect("tag show --help must render Clap help")
        .to_string();
    assert!(show_help.contains("<NAME>"), "{show_help}");
    assert!(show_help.contains("--json"), "{show_help}");

    let delete_help = Cli::try_parse_from(["ait-cli", "tag", "delete", "--help"])
        .err()
        .expect("tag delete --help must render Clap help")
        .to_string();
    assert!(delete_help.contains("referenced Snapshot"), "{delete_help}");
    assert!(delete_help.contains("--json"), "{delete_help}");

    let removed = Cli::try_parse_from([
        "ait-cli",
        "tag",
        "create",
        "stable/v1",
        "--message",
        "replacement",
        "--force",
    ])
    .err()
    .expect("retired tag create --force must be rejected");
    assert_eq!(removed.kind(), clap::error::ErrorKind::UnknownArgument);
}

#[test]
fn status_parser_exposes_compact_json_and_explicit_full_projection() {
    let parsed = Cli::try_parse_from(["ait-cli", "status", "--json"])
        .expect("status should accept compact JSON output");
    let Commands::Status(args) = parsed.command else {
        panic!("expected status command");
    };
    assert!(args.json);
    assert!(!args.full);

    let parsed = Cli::try_parse_from(["ait-cli", "status", "--json", "--full"])
        .expect("status should accept explicit full JSON output");
    let Commands::Status(args) = parsed.command else {
        panic!("expected status command");
    };
    assert!(args.json);
    assert!(args.full);

    let help = Cli::try_parse_from(["ait-cli", "status", "--help"])
        .err()
        .expect("status --help must render Clap help")
        .to_string();
    assert!(help.contains("without changing repository data"), "{help}");
    assert!(help.contains("compact versioned"), "{help}");
    assert!(help.contains("--json"), "{help}");
    assert!(help.contains("--full"), "{help}");
    assert!(!help.contains("--verbose"), "{help}");

    let error = Cli::try_parse_from(["ait-cli", "status", "--verbose"])
        .err()
        .expect("retired status --verbose must be rejected")
        .to_string();
    assert!(error.contains("unexpected argument '--verbose'"), "{error}");
}

#[test]
fn common_public_help_uses_user_actions_instead_of_internal_vocabulary() {
    const COMMAND_PATHS: &[&[&str]] = &[
        &["--help"],
        &["status", "--help"],
        &["diff", "--help"],
        &["line", "--help"],
        &["git", "--help"],
        &["queue", "--help"],
        &["repo", "--help"],
        &["config", "--help"],
        &["external", "--help"],
        &["plan", "--help"],
        &["task", "--help"],
        &["change", "--help"],
        &["snapshot", "--help"],
        &["patchset", "--help"],
        &["attest", "--help"],
        &["policy", "--help"],
        &["worktree", "--help"],
        &["workflow", "--help"],
    ];
    const INTERNAL_TERMS: &[&str] = &[
        "authority",
        "lineage",
        "materializ",
        "mutation",
        "projection",
        " scope",
        "admitted",
    ];

    for command_path in COMMAND_PATHS {
        let help =
            Cli::try_parse_from(std::iter::once("ait-cli").chain(command_path.iter().copied()))
                .err()
                .unwrap_or_else(|| panic!("{} must render help", command_path.join(" ")))
                .to_string();
        let normalized = help.to_ascii_lowercase();
        for term in INTERNAL_TERMS {
            assert!(
                !normalized.contains(term),
                "public `{}` help exposes internal term {term:?}:\n{help}",
                command_path.join(" ")
            );
        }
    }
}

#[test]
fn every_visible_public_help_field_uses_user_language() {
    const INTERNAL_TERMS: &[&str] = &[
        "authority",
        "lineage",
        "materializ",
        "mutation",
        "projection",
        " scope",
        "admitted",
    ];

    fn inspect(command: &clap::Command, path: &str) {
        let mut help_fields = Vec::new();
        if let Some(value) = command.get_about() {
            help_fields.push(value.to_string());
        }
        if let Some(value) = command.get_long_about() {
            help_fields.push(value.to_string());
        }
        for argument in command
            .get_arguments()
            .filter(|argument| !argument.is_hide_set())
        {
            if let Some(value) = argument.get_help() {
                help_fields.push(value.to_string());
            }
            if let Some(value) = argument.get_long_help() {
                help_fields.push(value.to_string());
            }
        }

        let help = help_fields.join("\n");
        let normalized = help.to_ascii_lowercase();
        for term in INTERNAL_TERMS {
            assert!(
                !normalized.contains(term),
                "public `{path}` help exposes internal term {term:?}:\n{help}"
            );
        }

        for subcommand in command
            .get_subcommands()
            .filter(|subcommand| !subcommand.is_hide_set())
        {
            inspect(subcommand, &format!("{path} {}", subcommand.get_name()));
        }
    }

    inspect(&Cli::command(), "ait");
}

#[test]
fn agent_action_full_projection_requires_json_on_each_supported_command() {
    let invocations = [
        vec!["ait-cli", "status"],
        vec![
            "ait-cli", "task", "start", "--title", "Task", "--intent", "Intent",
        ],
        vec!["ait-cli", "snapshot", "create"],
        vec!["ait-cli", "task", "finish", "LCT-1"],
    ];

    for invocation in invocations {
        let mut compact = invocation.clone();
        compact.push("--json");
        Cli::try_parse_from(compact).expect("compact JSON invocation must parse");

        let mut full = invocation.clone();
        full.extend(["--json", "--full"]);
        Cli::try_parse_from(full).expect("full JSON invocation must parse");

        let mut missing_json = invocation;
        missing_json.push("--full");
        let error = Cli::try_parse_from(missing_json)
            .err()
            .expect("--full without --json must fail");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
}

#[test]
fn compact_agent_action_projections_keep_only_next_step_evidence() {
    let status = compact_status_payload(&json!({
        "repo_name": "fixture",
        "current_line": "main",
        "head_snapshot_id": "SNP-AAAA1111",
        "workspace_status": "clean",
        "workspace_dirty": false,
        "workspace_changed_count": 0,
        "workspace_modified_count": 0,
        "workspace_missing_count": 0,
        "workspace_untracked_count": 0,
        "is_worktree": false,
        "worktree_name": null,
        "remote_count": 7,
        "reconciliation": {
            "safe_finding_count": 1,
            "manual_resolution_count": 0,
            "protected_count": 0,
            "next_command": "ait workflow reconcile --local --apply --safe-only"
        }
    }));
    assert_eq!(status["contract"], AGENT_ACTION_JSON_CONTRACT);
    assert_eq!(status["command"], "status");
    assert_eq!(status["line_name"], "main");
    assert_eq!(status["workspace"]["changed_count"], 0);
    assert_eq!(status["next_action"]["code"], "reconcile");
    assert!(status.get("remote_count").is_none());
    assert!(status.get("reconciliation").is_none());

    let dirty_status = compact_status_payload(&json!({
        "workspace_changed_count": 2,
        "workspace_modified_count": 1,
        "workspace_missing_count": 0,
        "workspace_untracked_count": 1,
        "workspace_dirty": true,
        "workspace_status": "dirty",
        "is_worktree": true,
        "worktree_name": "lct-1"
    }));
    assert_eq!(dirty_status["next_action"]["command"], "ait diff");
    assert_eq!(dirty_status["worktree"]["name"], "lct-1");

    let started = compact_task_start_payload(&json!({
        "task_id": "LCT-1",
        "change": {"task_id": "LCT-1", "change_id": "C-01"},
        "cd_command": "cd /alias/lct-1",
        "worktree": {
            "name": "lct-1",
            "path": "/physical roots/lct-1",
            "open_path": "/alias/lct-1",
            "current_line": "feature/lct-1",
            "head_snapshot_id": "SNP-AAAA1111"
        },
        "automatic_reconciliation": {"findings": [1, 2, 3]}
    }));
    assert_eq!(started["change_ref"], "LCT-1/C-01");
    assert_eq!(started["edit_root"], "/physical roots/lct-1");
    assert_eq!(
        started["next_action"]["command"],
        "cd '/physical roots/lct-1'"
    );
    assert!(started.get("worktree").is_none());
    assert!(started.get("automatic_reconciliation").is_none());

    let snapshot = compact_snapshot_create_payload(&json!({
        "snapshot_id": "SNP-BBBB2222",
        "line_name": "feature/lct-1",
        "parent_snapshot_id": "SNP-AAAA1111",
        "message": "Implement compact output",
        "files": [1, 2, 3],
        "phase_timings_ms": {"total": 10.0}
    }));
    assert_eq!(snapshot["command"], "snapshot.create");
    assert_eq!(snapshot["snapshot_id"], "SNP-BBBB2222");
    assert!(snapshot.get("files").is_none());
    assert!(snapshot.get("phase_timings_ms").is_none());

    let finished = compact_task_finish_payload(&json!({
        "mode": "local",
        "task_id": "LCT-1",
        "change_ref": "LCT-1/C-01",
        "target_line": "main",
        "landed_snapshot_id": "SNP-BBBB2222",
        "task_status": "completed",
        "change_status": "landed",
        "closeout_status": "partial",
        "bound_worktree_cleanup": {"status": "failed", "detail": "large"},
        "bound_line_closeout": {"status": "deferred"},
        "plan_checklist_closeout": {"status": "synced"},
        "closeout_recovery": {
            "code": "resume_task_land_closeout",
            "command": "ait task finish LCT-1/C-01 --local",
            "detail": "large recovery explanation"
        },
        "task": {"large": true},
        "change": {"large": true}
    }));
    assert_eq!(finished["command"], "task.finish");
    assert_eq!(finished["ok"], false);
    assert_eq!(finished["closeout"]["worktree_status"], "failed");
    assert_eq!(
        finished["next_action"]["command"],
        "ait task finish LCT-1/C-01 --local"
    );
    assert!(finished.get("task").is_none());
    assert!(finished.get("change").is_none());

    let remote_finished = compact_task_finish_payload(&json!({
        "task_land_contract": {"scope": "remote"},
        "closeout_status": "execution_complete_plan_separate"
    }));
    assert_eq!(remote_finished["mode"], "remote");
    assert_eq!(remote_finished["ok"], true);
}

#[test]
fn diff_parser_exposes_only_positional_paths_and_one_output_mode() {
    let parsed = Cli::try_parse_from(["ait-cli", "diff", "--json", "src/lib.rs", "src/new.rs"])
        .expect("diff should accept stable JSON and positional path filters");
    let Commands::Diff(args) = parsed.command else {
        panic!("expected diff command");
    };
    assert!(args.json);
    assert!(!args.stat);
    assert!(!args.name_only);
    assert_eq!(args.paths, ["src/lib.rs", "src/new.rs"]);

    let help = Cli::try_parse_from(["ait-cli", "diff", "--help"])
        .err()
        .expect("diff --help must render Clap help")
        .to_string();
    assert!(help.contains("current Line head"), "{help}");
    assert!(help.contains("without changing repository data"), "{help}");
    assert!(help.contains("[PATH]..."), "{help}");
    assert!(help.contains("--json"), "{help}");
    assert!(help.contains("--stat"), "{help}");
    assert!(help.contains("--name-only"), "{help}");
    assert!(help.contains("not supported"), "{help}");
    assert!(!help.contains("--path"), "{help}");
    assert!(!help.contains("--max-bytes"), "{help}");
    assert!(!help.contains("debug"), "{help}");

    for removed in [vec!["--path", "src"], vec!["--max-bytes", "1"]] {
        let error = Cli::try_parse_from(
            ["ait-cli", "diff"]
                .into_iter()
                .chain(removed.iter().copied()),
        )
        .err()
        .expect("retired diff option must be rejected");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    for conflicting in [
        ["--json", "--stat"],
        ["--json", "--name-only"],
        ["--stat", "--name-only"],
    ] {
        let error = Cli::try_parse_from(["ait-cli", "diff"].into_iter().chain(conflicting))
            .err()
            .expect("diff output modes must be mutually exclusive");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[test]
fn gc_parser_exposes_only_bounded_stats_exact_validation_and_explicit_apply() {
    let stats = Cli::try_parse_from(["ait-cli", "gc", "stats", "--json"])
        .expect("gc stats should accept stable JSON output");
    let Commands::Gc {
        command: GcCommand::Stats(stats),
    } = stats.command
    else {
        panic!("expected gc stats command");
    };
    assert!(stats.json);

    let validate = Cli::try_parse_from(["ait-cli", "gc", "validate", "--json"])
        .expect("gc validate should accept stable JSON output");
    let Commands::Gc {
        command: GcCommand::Validate(validate),
    } = validate.command
    else {
        panic!("expected gc validate command");
    };
    assert!(validate.json);

    let preview = Cli::try_parse_from(["ait-cli", "gc", "prune", "--json"])
        .expect("gc prune should preview without apply");
    let Commands::Gc {
        command: GcCommand::Prune(preview),
    } = preview.command
    else {
        panic!("expected gc prune preview command");
    };
    assert!(!preview.apply);
    assert!(preview.json);

    let apply = Cli::try_parse_from(["ait-cli", "gc", "prune", "--apply", "--json"])
        .expect("gc prune should accept explicit apply authority");
    let Commands::Gc {
        command: GcCommand::Prune(apply),
    } = apply.command
    else {
        panic!("expected gc prune apply command");
    };
    assert!(apply.apply);
    assert!(apply.json);

    let root_help = Cli::try_parse_from(["ait-cli", "gc", "--help"])
        .err()
        .expect("gc --help must render Clap help")
        .to_string();
    for command in ["stats", "validate", "prune"] {
        assert!(root_help.contains(command), "{root_help}");
    }
    assert!(root_help.contains("read-only unless"), "{root_help}");
    assert!(
        root_help.contains("never removes tree packs"),
        "{root_help}"
    );

    let stats_help = Cli::try_parse_from(["ait-cli", "gc", "stats", "--help"])
        .err()
        .expect("gc stats --help must render Clap help")
        .to_string();
    assert!(stats_help.contains("bounded"), "{stats_help}");
    assert!(stats_help.contains("gc validate"), "{stats_help}");
    assert!(stats_help.contains("--json"), "{stats_help}");
    assert!(!stats_help.contains("--deep"), "{stats_help}");
    assert!(!stats_help.contains("--include-inventory"), "{stats_help}");

    let validate_help = Cli::try_parse_from(["ait-cli", "gc", "validate", "--help"])
        .err()
        .expect("gc validate --help must render Clap help")
        .to_string();
    assert!(validate_help.contains("exact read-only"), "{validate_help}");
    assert!(validate_help.contains("returns nonzero"), "{validate_help}");
    assert!(validate_help.contains("--json"), "{validate_help}");

    let prune_help = Cli::try_parse_from(["ait-cli", "gc", "prune", "--help"])
        .err()
        .expect("gc prune --help must render Clap help")
        .to_string();
    assert!(prune_help.contains("Preview"), "{prune_help}");
    assert!(prune_help.contains("--apply"), "{prune_help}");
    assert!(prune_help.contains("read-only"), "{prune_help}");
    assert!(
        prune_help.contains("never prunes tree packs"),
        "{prune_help}"
    );
    assert!(prune_help.contains("--json"), "{prune_help}");

    for removed in [
        ["stats", "--deep"],
        ["stats", "--include-inventory"],
        ["prune", "--dry-run"],
        ["prune", "--yes"],
        ["prune", "--force"],
    ] {
        let error =
            Cli::try_parse_from(["ait-cli", "gc"].into_iter().chain(removed.iter().copied()))
                .err()
                .expect("retired or unapproved gc option must be rejected");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }
}

#[test]
fn queue_summary_parser_exposes_only_remote_and_json_options() {
    let parsed = Cli::try_parse_from([
        "ait-cli", "queue", "summary", "--remote", "origin", "--json",
    ])
    .expect("queue summary should accept its supported options");
    let Commands::Queue {
        command: QueueCommand::Summary(args),
    } = parsed.command
    else {
        panic!("expected queue summary command");
    };
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.json);

    let help = match Cli::try_parse_from(["ait-cli", "queue", "summary", "--help"]) {
        Ok(_) => panic!("--help should render clap help"),
        Err(error) => error.to_string(),
    };
    assert!(help.contains("--remote"), "{help}");
    assert!(help.contains("--json"), "{help}");
    assert!(!help.contains("--status"), "{help}");
    assert!(!help.contains("--all-changes"), "{help}");

    for removed in [vec!["--status", "completed"], vec!["--all-changes"]] {
        let error = match Cli::try_parse_from(
            ["ait-cli", "queue", "summary"]
                .into_iter()
                .chain(removed.iter().copied()),
        ) {
            Ok(_) => panic!("retired queue summary option must be rejected"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("unexpected argument"),
            "{removed:?}: {error}"
        );
    }
}

#[test]
fn remote_parser_exposes_only_the_bounded_public_options() {
    let add = Cli::try_parse_from([
        "ait-cli",
        "remote",
        "add",
        "origin",
        "https://example.test",
        "--default",
        "--json",
    ])
    .expect("remote add should accept its bounded options");
    let Commands::Remote {
        command: RemoteCommand::Add(add),
    } = add.command
    else {
        panic!("expected remote add command");
    };
    assert_eq!(add.name, "origin");
    assert_eq!(add.url, "https://example.test");
    assert!(add.default);
    assert!(add.json);

    let add_help = Cli::try_parse_from(["ait-cli", "remote", "add", "--help"])
        .err()
        .expect("remote add help")
        .to_string();
    assert!(
        add_help.contains("canonical Repository directory name"),
        "{add_help}"
    );
    assert!(add_help.contains("--default"), "{add_help}");
    assert!(add_help.contains("--json"), "{add_help}");
    assert!(!add_help.contains("--repo-name"), "{add_help}");
    assert!(!add_help.contains("--discard-export"), "{add_help}");

    let list_help = Cli::try_parse_from(["ait-cli", "remote", "list", "--help"])
        .err()
        .expect("remote list help")
        .to_string();
    assert!(
        list_help.contains("without contacting their servers"),
        "{list_help}"
    );
    assert!(list_help.contains("--json"), "{list_help}");
    assert!(!list_help.contains("--remote"), "{list_help}");

    let recover = Cli::try_parse_from([
        "ait-cli",
        "remote",
        "recover-head",
        "--remote",
        "origin",
        "--jobs",
        "16",
        "--apply",
        "--json",
    ])
    .expect("remote recover-head should accept its bounded options");
    let Commands::Remote {
        command: RemoteCommand::RecoverHead(recover),
    } = recover.command
    else {
        panic!("expected remote recover-head command");
    };
    assert_eq!(recover.remote.as_deref(), Some("origin"));
    assert_eq!(recover.jobs, 16);
    assert!(recover.apply);
    assert!(recover.json);

    let recover_help = Cli::try_parse_from(["ait-cli", "remote", "recover-head", "--help"])
        .err()
        .expect("remote recover-head help")
        .to_string();
    assert!(
        recover_help.contains("exact remote main head"),
        "{recover_help}"
    );
    assert!(
        recover_help.contains("full Snapshot ancestry"),
        "{recover_help}"
    );
    assert!(recover_help.contains("1 through 64"), "{recover_help}");
    assert!(recover_help.contains("read-only preview"), "{recover_help}");
    assert!(!recover_help.contains("--line"), "{recover_help}");
    assert!(!recover_help.contains("--include-line"), "{recover_help}");

    for retired in [
        vec![
            "remote",
            "add",
            "origin",
            "https://example.test",
            "--repo-name",
            "other",
        ],
        vec![
            "remote",
            "add",
            "origin",
            "https://example.test",
            "--discard-export",
        ],
        vec!["remote", "recover-head", "--line", "feature/task"],
        vec!["remote", "recover-head", "--include-line", "release"],
    ] {
        let error = Cli::try_parse_from(std::iter::once("ait-cli").chain(retired.iter().copied()))
            .err()
            .expect("retired remote option must be rejected");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    for invalid_jobs in ["0", "65"] {
        let error =
            Cli::try_parse_from(["ait-cli", "remote", "recover-head", "--jobs", invalid_jobs])
                .err()
                .expect("invalid recover-head jobs must be rejected");
        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
    }
}

#[test]
fn config_parser_exposes_only_the_admitted_set_and_unset_surface() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "config",
        "set",
        "--workflow-mode",
        "solo_local",
        "--sprint",
        "off",
        "--default-author-mode",
        "human_only",
        "--default-model",
        "gpt-example",
        "--task-review",
        "required",
        "--task-worktree-alias-root",
        "task-links",
        "--task-worktree-main-seed-ram-max-bytes",
        "4096",
        "--id-namespace-prefix",
        "ZX",
        "--user-name",
        "Reviewer",
        "--user-email",
        "reviewer@example.test",
        "--json",
    ])
    .expect("config set should accept every admitted option");
    let Commands::Config {
        command: ConfigCommand::Set(args),
    } = parsed.command
    else {
        panic!("expected config set command");
    };
    assert_eq!(args.workflow_mode, Some(ConfigWorkflowModeArg::SoloLocal));
    assert_eq!(args.sprint, Some(ConfigToggleArg::Off));
    assert_eq!(
        args.default_author_mode,
        Some(ConfigAuthorModeArg::HumanOnly)
    );
    assert_eq!(args.default_model.as_deref(), Some("gpt-example"));
    assert_eq!(args.task_review, Some(ConfigTaskReviewArg::Required));
    assert_eq!(args.task_worktree_alias_root.as_deref(), Some("task-links"));
    assert_eq!(args.task_worktree_main_seed_ram_max_bytes, Some(4096));
    assert_eq!(args.id_namespace_prefix.as_deref(), Some("ZX"));
    assert_eq!(args.user_name.as_deref(), Some("Reviewer"));
    assert_eq!(args.user_email.as_deref(), Some("reviewer@example.test"));
    assert!(args.json);

    let request = ConfigSetRequest {
        workflow_mode: args.workflow_mode.map(|value| value.as_str().to_string()),
        sprint: args.sprint.map(|value| value.as_str().to_string()),
        default_author_mode: args
            .default_author_mode
            .map(|value| value.as_str().to_string()),
        default_model: args.default_model,
        task_review: args.task_review.map(|value| value.as_str().to_string()),
        task_worktree_alias_root: args.task_worktree_alias_root,
        task_worktree_main_seed_ram_max_bytes: args.task_worktree_main_seed_ram_max_bytes,
        id_namespace_prefix: args.id_namespace_prefix,
        user_name: args.user_name,
        user_email: args.user_email,
    };
    assert_eq!(
        request.updated_keys(),
        vec![
            "workflow-mode",
            "sprint",
            "default-author-mode",
            "default-model",
            "task-review",
            "task-worktree-alias-root",
            "task-worktree-main-seed-ram-max-bytes",
            "id-namespace-prefix",
            "user-name",
            "user-email",
        ]
    );
    assert_eq!(
        ConfigSetRequest {
            workflow_mode: Some("solo_local".to_string()),
            ..ConfigSetRequest::default()
        }
        .updated_keys(),
        vec!["workflow-mode", "sprint"]
    );

    for (raw, expected) in [
        ("default-author-mode", ConfigUnsetKey::DefaultAuthorMode),
        ("default-model", ConfigUnsetKey::DefaultModel),
        ("task-review", ConfigUnsetKey::TaskReview),
        (
            "task-worktree-alias-root",
            ConfigUnsetKey::TaskWorktreeAliasRoot,
        ),
        (
            "task-worktree-main-seed-ram-max-bytes",
            ConfigUnsetKey::TaskWorktreeMainSeedRamMaxBytes,
        ),
        ("id-namespace-prefix", ConfigUnsetKey::IdNamespacePrefix),
        ("user-name", ConfigUnsetKey::UserName),
        ("user-email", ConfigUnsetKey::UserEmail),
    ] {
        let parsed = Cli::try_parse_from(["ait-cli", "config", "unset", raw, "--json"])
            .unwrap_or_else(|error| panic!("{raw}: {error}"));
        let Commands::Config {
            command: ConfigCommand::Unset(args),
        } = parsed.command
        else {
            panic!("expected config unset command");
        };
        assert_eq!(args.key.into_config_key(), expected, "{raw}");
        assert!(args.json);
    }

    for invalid in [
        ["--workflow-mode", "SOLO_LOCAL"],
        ["--workflow-mode", "solo-local"],
        ["--sprint", "yes"],
        ["--default-author-mode", "HumanOnly"],
        ["--task-review", "on"],
        ["--task-review", "off"],
    ] {
        assert!(
            Cli::try_parse_from(["ait-cli", "config", "set"].into_iter().chain(invalid)).is_err(),
            "mistyped value must be rejected: {invalid:?}"
        );
    }

    for retired in [
        vec!["--repository-index", "7"],
        vec!["--clear-repository-index"],
        vec!["--clear-default-author-mode"],
        vec!["--clear-default-model"],
        vec!["--clear-task-review"],
        vec!["--clear-task-worktree-alias-root"],
        vec!["--clear-task-worktree-main-seed-ram-max-bytes"],
        vec!["--clear-id-namespace-prefix"],
        vec!["--clear-user-name"],
        vec!["--clear-user-email"],
        vec!["--task-tracking", "on"],
        vec!["--command-profiling", "on"],
        vec!["--task-auto-worktree", "on"],
        vec!["--clear-task-auto-worktree"],
        vec!["--workflow-default-scope", "local"],
        vec!["--clear-workflow-default-scope"],
        vec!["--task-default-scope", "local"],
        vec!["--clear-task-default-scope"],
        vec!["--change-default-scope", "local"],
        vec!["--clear-change-default-scope"],
        vec!["--plan-task-binding-mode", "required"],
        vec!["--clear-plan-task-binding"],
    ] {
        let error = Cli::try_parse_from(
            ["ait-cli", "config", "set"]
                .into_iter()
                .chain(retired.iter().copied()),
        )
        .err()
        .unwrap_or_else(|| panic!("retired config set option parsed: {retired:?}"));
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "{retired:?}: {error}"
        );
    }

    for forbidden_key in ["repository-index", "workflow-default-scope", "unknown-key"] {
        assert!(
            Cli::try_parse_from(["ait-cli", "config", "unset", forbidden_key]).is_err(),
            "forbidden unset key parsed: {forbidden_key}"
        );
    }
}

#[test]
fn config_help_documents_the_complete_public_contract() {
    let parent_help = Cli::try_parse_from(["ait-cli", "config", "--help"])
        .err()
        .expect("config help")
        .to_string();
    assert!(
        parent_help.contains("effective configuration"),
        "{parent_help}"
    );
    for command in ["show", "set", "unset"] {
        assert!(parent_help.contains(command), "{parent_help}");
    }

    let show_help = Cli::try_parse_from(["ait-cli", "config", "show", "--help"])
        .err()
        .expect("config show help")
        .to_string();
    assert!(
        show_help.contains("repository configuration"),
        "{show_help}"
    );
    assert!(show_help.contains("worktree settings"), "{show_help}");
    assert!(show_help.contains("--json"), "{show_help}");

    let set_help = Cli::try_parse_from(["ait-cli", "config", "set", "--help"])
        .err()
        .expect("config set help")
        .to_string();
    for option in [
        "--workflow-mode",
        "--sprint",
        "--default-author-mode",
        "--default-model",
        "--task-review",
        "--task-worktree-alias-root",
        "--task-worktree-main-seed-ram-max-bytes",
        "--id-namespace-prefix",
        "--user-name",
        "--user-email",
        "--json",
    ] {
        assert!(set_help.contains(option), "missing {option}: {set_help}");
    }
    for value in [
        "solo_local",
        "solo_remote",
        "team_remote",
        "human_only",
        "human_with_ai_assist",
        "ai_with_human_review",
        "ai_only_experimental",
        "required",
        "automatic",
    ] {
        assert!(set_help.contains(value), "missing {value}: {set_help}");
    }

    let unset_help = Cli::try_parse_from(["ait-cli", "config", "unset", "--help"])
        .err()
        .expect("config unset help")
        .to_string();
    assert!(unset_help.contains("Fallbacks are"), "{unset_help}");
    for fallback in [
        "ai_with_human_review",
        "default-model -> unset",
        "automatic",
        ".ait-worktree-links",
        "no configured budget",
        "actor detection remains available",
    ] {
        assert!(
            unset_help.contains(fallback),
            "missing {fallback}: {unset_help}"
        );
    }
    for key in [
        "default-author-mode",
        "default-model",
        "task-review",
        "task-worktree-alias-root",
        "task-worktree-main-seed-ram-max-bytes",
        "id-namespace-prefix",
        "user-name",
        "user-email",
    ] {
        assert!(unset_help.contains(key), "missing {key}: {unset_help}");
    }

    for removed in [
        "repository-index",
        "--clear-",
        "task-tracking",
        "command-profiling",
        "task-auto-worktree",
        "workflow-default-scope",
        "task-default-scope",
        "change-default-scope",
        "plan-task-binding-mode",
    ] {
        assert!(!set_help.contains(removed), "{removed}: {set_help}");
        assert!(!unset_help.contains(removed), "{removed}: {unset_help}");
    }
}

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
    assert!(!rendered.contains("discarded_export"));
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
fn workflow_tier_command_and_guide_topic_are_removed() {
    let parse_error = Cli::try_parse_from(["ait", "workflow", "tier", "--json"])
        .err()
        .expect("removed workflow tier command must be rejected")
        .to_string();
    assert!(parse_error.contains("tier"), "{parse_error}");
    assert!(
        parse_error.contains("unrecognized subcommand"),
        "{parse_error}"
    );

    let workflow_help = Cli::try_parse_from(["ait", "workflow", "--help"])
        .err()
        .expect("workflow help must render")
        .to_string();
    assert!(!workflow_help.contains("  tier"), "{workflow_help}");

    let default_guide = workflow_guide_payload(None).expect("default workflow guide");
    let topics = default_guide["topics"]
        .as_array()
        .expect("workflow guide topics");
    assert_eq!(topics.len(), 2);
    assert!(topics.iter().any(|topic| topic["topic"] == "inventory"));
    assert!(topics.iter().any(|topic| topic["topic"] == "land"));
    assert!(!topics.iter().any(|topic| topic["topic"] == "tiers"));

    let guide_error = workflow_guide_payload(Some("tiers"))
        .expect_err("removed tiers guide topic must be rejected");
    assert!(guide_error.contains("Available topics: inventory, land"));
}

#[test]
fn workflow_land_local_command_is_removed() {
    let parse_error = Cli::try_parse_from(["ait", "workflow", "land-local", "LCC-1"])
        .err()
        .expect("removed workflow finish-local command must be rejected")
        .to_string();
    assert!(parse_error.contains("land-local"), "{parse_error}");
    assert!(
        parse_error.contains("unrecognized subcommand"),
        "{parse_error}"
    );

    let workflow_help = Cli::try_parse_from(["ait", "workflow", "--help"])
        .err()
        .expect("workflow help must render")
        .to_string();
    assert!(!workflow_help.contains("  land-local"), "{workflow_help}");
}

#[test]
fn workflow_ready_parser_requires_apply_for_every_mutation_input() {
    for (option, value) in [
        ("--snapshot-message", "Snapshot"),
        ("--summary", "Summary"),
        ("--tests", "pass"),
        ("--lint", "pass"),
        ("--security", "pass"),
        ("--license", "pass"),
        ("--author-mode", "human_only"),
        ("--model", "gpt-5"),
    ] {
        let error = Cli::try_parse_from(["ait", "workflow", "ready", "RCC-1", option, value])
            .err()
            .unwrap_or_else(|| panic!("{option} parsed without --apply"))
            .to_string();
        assert!(error.contains("--apply"), "{option}: {error}");
    }

    let ready = Cli::try_parse_from([
        "ait",
        "workflow",
        "ready",
        "RCC-1",
        "--apply",
        "--snapshot-message",
        "Snapshot",
        "--summary",
        "Summary",
        "--tests",
        "pass",
        "--lint",
        "pass",
        "--security",
        "pass",
        "--license",
        "pass",
        "--author-mode",
        "human_with_ai_assist",
        "--model",
        "gpt-5",
        "--remote",
        "origin",
    ])
    .expect("complete Ready mutation surface should parse with --apply");
    let Commands::Workflow {
        command: WorkflowCommand::Ready(args),
    } = ready.command
    else {
        panic!("expected workflow ready command");
    };
    assert!(args.apply);
    assert_eq!(
        args.author_mode,
        Some(ConfigAuthorModeArg::HumanWithAiAssist)
    );
    assert_eq!(args.remote.as_deref(), Some("origin"));

    let invalid_author_mode = Cli::try_parse_from([
        "ait",
        "workflow",
        "ready",
        "RCC-1",
        "--apply",
        "--author-mode",
        "unreviewed_robot",
    ])
    .err()
    .expect("invalid author mode must fail during parsing")
    .to_string();
    assert!(
        invalid_author_mode.contains("invalid value"),
        "{invalid_author_mode}"
    );
}

#[test]
fn workflow_finish_replaces_land_and_exposes_only_the_remote_reviewer_contract() {
    let removed_land = Cli::try_parse_from(["ait", "workflow", "land", "RCC-1"])
        .err()
        .expect("workflow land must be removed without an alias")
        .to_string();
    assert!(
        removed_land.contains("unrecognized subcommand 'land'"),
        "{removed_land}"
    );

    let preview = Cli::try_parse_from(["ait", "workflow", "finish", "RCC-1"])
        .expect("remote reviewer preview should parse without an explicit remote");
    let Commands::Workflow {
        command: WorkflowCommand::Finish(args),
    } = preview.command
    else {
        panic!("expected workflow finish command");
    };
    assert_eq!(args.change_id, "RCC-1");
    assert!(!args.apply);
    assert!(args.review_message.is_none());
    assert!(args.remote.is_none());

    for (option, value) in [
        ("--snapshot-message", Some("Snapshot")),
        ("--summary", Some("Summary")),
        ("--tests", Some("pass")),
        ("--lint", Some("pass")),
        ("--security", Some("pass")),
        ("--license", Some("pass")),
        ("--author-mode", Some("human_only")),
        ("--model", Some("gpt-5")),
        ("--reviewer", Some("spoofed")),
        ("--target", Some("main")),
        ("--mode", Some("direct")),
        ("--local", None),
        ("--all-completed-local", None),
        ("--json", None),
    ] {
        let mut argv = vec!["ait", "workflow", "finish", "RCC-1", option];
        if let Some(value) = value {
            argv.push(value);
        }
        let error = Cli::try_parse_from(argv.clone())
            .err()
            .unwrap_or_else(|| panic!("removed Workflow finish option parsed: {argv:?}"))
            .to_string();
        assert!(error.contains("unexpected argument"), "{option}: {error}");
    }

    let missing_id = Cli::try_parse_from(["ait", "workflow", "finish"])
        .err()
        .expect("Workflow finish Change ID must be parser-required")
        .to_string();
    assert!(missing_id.contains("required"), "{missing_id}");

    let help = Cli::try_parse_from(["ait", "workflow", "finish", "--help"])
        .err()
        .expect("Workflow finish help must render")
        .to_string();
    assert!(help.contains("Usage: ait workflow finish"), "{help}");
    for retained in ["--apply", "--review-message", "--remote"] {
        assert!(help.contains(retained), "missing {retained}: {help}");
    }
    for removed in [
        "--snapshot-message",
        "--summary",
        "--tests",
        "--lint",
        "--security",
        "--license",
        "--author-mode",
        "--model",
        "--reviewer",
        "--target",
        "--mode",
        "--local",
        "--all-completed-local",
        "--json",
    ] {
        assert!(
            !help.contains(removed),
            "retired {removed} remains in help: {help}"
        );
    }
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
fn agent_list_json_payload_honors_bounded_and_complete_history_modes() {
    let mut rows = (1..=25)
        .map(|ordinal| {
            json!({
                "change_ref": format!("LCT-{ordinal:04}/C-01"),
                "status": "draft",
                "updated_at": format!("2026-08-01T00:{ordinal:02}:00Z"),
            })
        })
        .collect::<Vec<_>>();
    rows.push(json!({
        "change_ref": "LCT-9999/C-01",
        "status": "archived",
        "updated_at": "2026-08-01T23:59:00Z",
    }));
    let payload = JsonValue::Array(rows);

    let bounded = agent_list_json_payload(
        &payload,
        false,
        &["landed", "archived", "abandoned", "canceled"],
    );
    let bounded = bounded.as_array().expect("bounded JSON array");
    assert_eq!(bounded.len(), DEFAULT_AGENT_TEXT_LIST_LIMIT);
    assert!(bounded.iter().all(|row| row["status"] != "archived"));

    let complete = agent_list_json_payload(
        &payload,
        true,
        &["landed", "archived", "abandoned", "canceled"],
    );
    let complete = complete.as_array().expect("complete JSON array");
    assert_eq!(complete.len(), 26);
    assert!(complete.iter().any(|row| row["status"] == "archived"));
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
fn snapshot_create_rejects_removed_quick_options_and_accepts_plain_creation() {
    for option in ["--profile", "--intent", "--validation"] {
        let error = Cli::try_parse_from(["ait", "snapshot", "create", option, "removed-value"])
            .err()
            .expect("removed quick Snapshot option must be rejected")
            .to_string();
        assert!(error.contains(option), "{error}");
        assert!(error.contains("unexpected argument"), "{error}");
    }

    let plain = Cli::try_parse_from([
        "ait",
        "snapshot",
        "create",
        "--message",
        "Normal task Snapshot",
        "--json",
    ])
    .expect("plain Snapshot should remain compatible");
    let Commands::Snapshot {
        command: SnapshotCommand::Create(plain),
    } = plain.command
    else {
        panic!("expected plain snapshot create command");
    };
    assert_eq!(plain.message.as_deref(), Some("Normal task Snapshot"));
    assert!(plain.json);
}

#[test]
fn git_friendly_commit_and_branch_aliases_reuse_canonical_grammar() {
    let canonical = Cli::try_parse_from([
        "ait",
        "snapshot",
        "create",
        "-m",
        "Alias Snapshot",
        "--json",
        "--full",
    ])
    .expect("snapshot create should accept the shared -m spelling");
    let Commands::Snapshot {
        command: SnapshotCommand::Create(canonical),
    } = canonical.command
    else {
        panic!("expected snapshot create command");
    };

    let alias = Cli::try_parse_from(["ait", "commit", "-m", "Alias Snapshot", "--json", "--full"])
        .expect("commit should parse as the Snapshot creation alias");
    let Commands::Commit(alias) = alias.command else {
        panic!("expected commit command");
    };
    assert_eq!(alias.message, canonical.message);
    assert_eq!(alias.json, canonical.json);
    assert_eq!(alias.full, canonical.full);

    let branch = Cli::try_parse_from([
        "ait",
        "branch",
        "create",
        "feature/example",
        "--from-snapshot",
        "SNP-EXAMPLE",
        "--switch",
        "--json",
    ])
    .expect("branch should accept the complete Line subcommand grammar");
    let Commands::Line {
        command: LineCommand::Create(branch),
    } = branch.command
    else {
        panic!("expected branch alias to parse as line create");
    };
    assert_eq!(branch.name, "feature/example");
    assert_eq!(branch.from_snapshot.as_deref(), Some("SNP-EXAMPLE"));
    assert!(branch.switch);
    assert!(branch.json);

    let commit_help = Cli::try_parse_from(["ait", "commit", "--help"])
        .err()
        .expect("commit --help must render Clap help")
        .to_string();
    for text in [
        "Git-friendly alias for `ait snapshot create`",
        "Usage: ait commit [OPTIONS]",
        "-m, --message <MESSAGE>",
        "AIT Snapshot",
    ] {
        assert!(commit_help.contains(text), "{commit_help}");
    }

    let branch_help = Cli::try_parse_from(["ait", "branch", "--help"])
        .err()
        .expect("branch --help must render Clap help")
        .to_string();
    for text in [
        "`ait branch` is a Git-friendly alias",
        "Usage: ait line <COMMAND>",
        "create",
        "switch",
        "delete",
        "merge",
    ] {
        assert!(branch_help.contains(text), "{branch_help}");
    }

    for args in [
        vec!["ait", "commit", "--amend"],
        vec!["ait", "commit", "-a"],
        vec!["ait", "commit", "src/lib.rs"],
        vec!["ait", "branch", "feature/example"],
        vec!["ait", "branch", "-d", "feature/example"],
    ] {
        let error = Cli::try_parse_from(args.clone())
            .err()
            .unwrap_or_else(|| panic!("unsupported Git spelling must fail: {args:?}"))
            .to_string();
        assert!(
            error.contains("unexpected argument") || error.contains("unrecognized subcommand"),
            "{error}"
        );
    }
}

#[test]
fn snapshot_diff_parser_requires_text_for_positive_explicit_byte_bounds() {
    let defaulted = Cli::try_parse_from(["ait", "snapshot", "diff", "SNP-OLD", "SNP-NEW"])
        .expect("structural Snapshot diff should retain its internal text-size default");
    let Commands::Snapshot {
        command: SnapshotCommand::Diff(defaulted),
    } = defaulted.command
    else {
        panic!("expected snapshot diff command");
    };
    assert!(!defaulted.include_text);
    assert_eq!(defaulted.max_bytes, DEFAULT_SNAPSHOT_DIFF_MAX_BYTES);

    let bounded = Cli::try_parse_from([
        "ait",
        "snapshot",
        "diff",
        "SNP-OLD",
        "SNP-NEW",
        "--include-text",
        "--max-bytes",
        "4096",
    ])
    .expect("positive explicit text bound should parse");
    let Commands::Snapshot {
        command: SnapshotCommand::Diff(bounded),
    } = bounded.command
    else {
        panic!("expected snapshot diff command");
    };
    assert!(bounded.include_text);
    assert_eq!(bounded.max_bytes, 4096);

    let missing_text = Cli::try_parse_from([
        "ait",
        "snapshot",
        "diff",
        "SNP-OLD",
        "SNP-NEW",
        "--max-bytes",
        "4096",
    ])
    .err()
    .expect("explicit byte bound must require text diff generation");
    assert_eq!(
        missing_text.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert!(missing_text.to_string().contains("--include-text"));

    let zero = Cli::try_parse_from([
        "ait",
        "snapshot",
        "diff",
        "SNP-OLD",
        "SNP-NEW",
        "--include-text",
        "--max-bytes",
        "0",
    ])
    .err()
    .expect("zero text bound must fail during parsing");
    assert_eq!(zero.kind(), clap::error::ErrorKind::ValueValidation);
    assert!(zero.to_string().contains("value must be 1 or greater"));
}

#[test]
fn snapshot_replay_defaults_to_current_line_and_retains_hidden_onto_compatibility() {
    let parsed = Cli::try_parse_from(["ait-cli", "snapshot", "replay", "SNP-REVISION"])
        .expect("snapshot replay should default to the current Line");
    let Commands::Snapshot {
        command: SnapshotCommand::Replay(args),
    } = parsed.command
    else {
        panic!("expected snapshot replay command");
    };
    assert_eq!(args.onto, None);

    let compatible = Cli::try_parse_from([
        "ait-cli",
        "snapshot",
        "replay",
        "SNP-REVISION",
        "--onto",
        "main",
    ])
    .expect("legacy explicit current-Line assertion should remain parseable");
    let Commands::Snapshot {
        command: SnapshotCommand::Replay(args),
    } = compatible.command
    else {
        panic!("expected compatible snapshot replay command");
    };
    assert_eq!(args.onto.as_deref(), Some("main"));
}

#[test]
fn snapshot_help_explains_public_behavior_and_hides_compatibility_inputs() {
    fn help(command: Option<&str>) -> String {
        let mut args = vec!["ait-cli", "snapshot"];
        if let Some(command) = command {
            args.push(command);
        }
        args.push("--help");
        Cli::try_parse_from(args)
            .err()
            .expect("--help must render Clap help")
            .to_string()
    }

    let root = help(None);
    assert!(root.contains("immutable local Snapshots"), "{root}");
    assert!(
        root.contains("restore-lines, revert, and replay change only workspace files"),
        "{root}"
    );

    let create = help(Some("create"));
    for evidence in [
        "advance the current Line head",
        "--message <MESSAGE>",
        "machine-readable creation payload",
    ] {
        assert!(
            create.contains(evidence),
            "missing {evidence:?} in:\n{create}"
        );
    }
    for removed in ["--profile", "--intent", "--validation"] {
        assert!(
            !create.contains(removed),
            "unexpected {removed:?} in:\n{create}"
        );
    }

    let list = help(Some("list"));
    assert!(list.contains("bounded recent text or JSON view"), "{list}");

    let show = help(Some("show"));
    assert!(show.contains("<SNAPSHOT_OR_TAG>"), "{show}");
    assert!(show.contains("JSON is always complete"), "{show}");

    let diff = help(Some("diff"));
    assert!(diff.contains("<OLD_SNAPSHOT_OR_TAG>"), "{diff}");
    assert!(diff.contains("requires --include-text"), "{diff}");
    assert!(diff.contains("[default: 128000]"), "{diff}");

    let restore = help(Some("restore-lines"));
    assert!(restore.contains("exact immutable Snapshot ID"), "{restore}");
    assert!(restore.contains("Only --yes applies"), "{restore}");

    let revert = help(Some("revert"));
    assert!(revert.contains("<SNAPSHOT_OR_TAG>"), "{revert}");
    assert!(revert.contains("Overwrite unsaved changes"), "{revert}");
    assert!(revert.contains("does not create a Snapshot"), "{revert}");

    let replay = help(Some("replay"));
    assert!(replay.contains("current Line workspace"), "{replay}");
    assert!(replay.contains("<SNAPSHOT_OR_TAG>"), "{replay}");
    assert!(!replay.contains("--onto"), "{replay}");

    let ancestry = help(Some("ancestry"));
    assert!(ancestry.contains("default direction"), "{ancestry}");
    assert!(ancestry.contains("positive edge depth"), "{ancestry}");
    assert!(ancestry.contains("nearest 20"), "{ancestry}");

    let is_ancestor = help(Some("is-ancestor"));
    assert!(
        is_ancestor.contains("<OLDER_SNAPSHOT_OR_TAG> <NEWER_SNAPSHOT_OR_TAG>"),
        "{is_ancestor}"
    );
    assert!(is_ancestor.contains("exits 0 when true, 1 when false, and 2"));

    let merge_base = help(Some("merge-base"));
    assert!(
        merge_base.contains("<LEFT_SNAPSHOT_OR_TAG>"),
        "{merge_base}"
    );
    assert!(
        merge_base.contains("equally best common ancestor"),
        "{merge_base}"
    );
    assert!(
        merge_base.contains("exits 0 when a base exists"),
        "{merge_base}"
    );
}

#[test]
fn blame_parser_freezes_the_read_only_selector_contract() {
    let parsed = Cli::try_parse_from([
        "ait",
        "blame",
        "src/lib.rs",
        "--line",
        "7",
        "--snapshot",
        "SNP-MERGE",
        "--via-parent",
        "SNP-RIGHT",
        "--json",
    ])
    .expect("explicit Snapshot blame should parse");
    let Commands::Blame(args) = parsed.command else {
        panic!("expected blame command");
    };
    assert_eq!(args.path, "src/lib.rs");
    assert_eq!(args.line, Some(7));
    assert_eq!(args.snapshot_id.as_deref(), Some("SNP-MERGE"));
    assert_eq!(args.via_parent_snapshot_id.as_deref(), Some("SNP-RIGHT"));
    assert!(args.json);

    let patchset = Cli::try_parse_from([
        "ait",
        "blame",
        "src/lib.rs",
        "--start",
        "2",
        "--end",
        "9",
        "--patchset",
        "P-RCT-1000/C-01-1",
        "--remote",
        "origin",
    ])
    .expect("exact Patchset range blame should parse");
    let Commands::Blame(patchset) = patchset.command else {
        panic!("expected blame command");
    };
    assert_eq!(patchset.start_line, Some(2));
    assert_eq!(patchset.end_line, Some(9));
    assert_eq!(patchset.patchset_id.as_deref(), Some("P-RCT-1000/C-01-1"));
    assert_eq!(patchset.remote_name.as_deref(), Some("origin"));

    let help = match Cli::try_parse_from(["ait", "blame", "--help"]) {
        Ok(_) => panic!("--help should render Clap help"),
        Err(error) => error.to_string(),
    };
    assert!(help.contains("--via-parent"), "{help}");
    assert!(help.contains("without modifying workspace files"), "{help}");
    for removed in ["--restore", "--dry-run", "--parent", "--repo", "--change"] {
        assert!(
            !help.contains(removed),
            "removed option {removed} in:\n{help}"
        );
    }

    for invalid in [
        vec!["ait", "blame", "src/lib.rs", "--start", "2"],
        vec!["ait", "blame", "src/lib.rs", "--end", "2"],
        vec![
            "ait",
            "blame",
            "src/lib.rs",
            "--line",
            "1",
            "--start",
            "1",
            "--end",
            "2",
        ],
        vec!["ait", "blame", "src/lib.rs", "--line", "0"],
        vec!["ait", "blame", "src/lib.rs", "--patchset", "7"],
        vec!["ait", "blame", "src/lib.rs", "--remote", "origin"],
        vec!["ait", "blame", "src/lib.rs", "--via-parent", "SNP-PARENT"],
        vec![
            "ait",
            "blame",
            "src/lib.rs",
            "--snapshot",
            "SNP-A",
            "--patchset",
            "P-RCT-1/C-01-1",
        ],
        vec![
            "ait",
            "blame",
            "docs/plan.md",
            "--plan-id",
            "PLN-1",
            "--plan-ref",
            "root",
        ],
        vec![
            "ait",
            "blame",
            "docs/plan.md",
            "--plan-id",
            "PLN-1",
            "--snapshot",
            "SNP-A",
        ],
    ] {
        assert!(
            Cli::try_parse_from(invalid.clone()).is_err(),
            "invalid blame argv parsed: {invalid:?}"
        );
    }

    for removed in [
        vec!["ait", "blame", "src/lib.rs", "--restore"],
        vec!["ait", "blame", "src/lib.rs", "--dry-run"],
        vec!["ait", "blame", "src/lib.rs", "--parent", "SNP-PARENT"],
        vec!["ait", "blame", "src/lib.rs", "--repo", "fixture"],
        vec!["ait", "blame", "src/lib.rs", "--change", "C-01"],
    ] {
        let error = match Cli::try_parse_from(removed.clone()) {
            Ok(_) => panic!("removed blame option must be unrecognized"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("unexpected argument"),
            "{removed:?}: {error}"
        );
    }
}

#[test]
fn snapshot_restore_lines_parser_requires_one_complete_selection() {
    let parsed = Cli::try_parse_from([
        "ait",
        "snapshot",
        "restore-lines",
        "SNP-SOURCE",
        "src/lib.rs",
        "--line",
        "3",
        "--yes",
        "--json",
    ])
    .expect("single-line restore should parse");
    let Commands::Snapshot {
        command: SnapshotCommand::RestoreLines(args),
    } = parsed.command
    else {
        panic!("expected snapshot restore-lines command");
    };
    assert_eq!(args.snapshot_id, "SNP-SOURCE");
    assert_eq!(args.path, "src/lib.rs");
    assert_eq!(args.line, Some(3));
    assert!(args.yes);
    assert!(args.json);

    let range = Cli::try_parse_from([
        "ait",
        "snapshot",
        "restore-lines",
        "SNP-SOURCE",
        "src/lib.rs",
        "--start",
        "2",
        "--end",
        "4",
    ])
    .expect("range preview should parse");
    let Commands::Snapshot {
        command: SnapshotCommand::RestoreLines(range),
    } = range.command
    else {
        panic!("expected snapshot restore-lines command");
    };
    assert_eq!(range.start_line, Some(2));
    assert_eq!(range.end_line, Some(4));
    assert!(!range.yes);

    for invalid in [
        vec![
            "ait",
            "snapshot",
            "restore-lines",
            "SNP-SOURCE",
            "src/lib.rs",
        ],
        vec![
            "ait",
            "snapshot",
            "restore-lines",
            "SNP-SOURCE",
            "src/lib.rs",
            "--start",
            "2",
        ],
        vec![
            "ait",
            "snapshot",
            "restore-lines",
            "SNP-SOURCE",
            "src/lib.rs",
            "--line",
            "1",
            "--start",
            "1",
            "--end",
            "2",
        ],
        vec![
            "ait",
            "snapshot",
            "restore-lines",
            "SNP-SOURCE",
            "src/lib.rs",
            "--line",
            "0",
        ],
    ] {
        assert!(
            Cli::try_parse_from(invalid.clone()).is_err(),
            "invalid restore-lines argv parsed: {invalid:?}"
        );
    }

    let help = match Cli::try_parse_from(["ait", "snapshot", "restore-lines", "--help"]) {
        Ok(_) => panic!("--help should render Clap help"),
        Err(error) => error.to_string(),
    };
    assert!(help.contains("read-only preview"), "{help}");
    assert!(
        help.contains("(--line <N> | --start <N> --end <N>)"),
        "{help}"
    );
    assert!(help.contains("--yes"), "{help}");
    assert!(help.contains("--line"), "{help}");
    assert!(help.contains("--start"), "{help}");
    assert!(help.contains("--end"), "{help}");
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
fn plan_sync_terminal_errors_preserve_local_ahead_recovery_context() {
    let failed = plan_sync_terminal_error(&json!({
        "status": "failed",
        "results": [{"action": "created", "plan_id": "PR-1"}],
        "adoptions": [],
        "publish_results": [],
        "error": {"message": "remote publish rejected"},
    }))
    .expect("failed status must be terminal");
    assert!(failed.contains("saving local Plan history"), "{failed}");
    assert!(failed.contains("were not rolled back"), "{failed}");
    assert!(
        failed.contains("retry the same `ait plan sync`"),
        "{failed}"
    );

    let partial = plan_sync_terminal_error(&json!({
        "status": "partial_success",
        "results": [{"action": "updated", "plan_id": "PR-1"}],
        "publish_results": [{"plan_id": "PR-42"}],
        "error": {"message": "paired artifact upload failed"},
    }))
    .expect("partial success must be terminal");
    assert!(partial.contains("partially succeeded"), "{partial}");
    assert!(
        partial.contains("completed remote publications were kept"),
        "{partial}"
    );
    assert!(render_sync_like(&json!({
        "status": "partial_success",
        "results": [{"action": "updated", "plan_id": "PR-1"}],
        "publish_results": [{"plan_id": "PR-42"}],
        "error": {"message": "paired artifact upload failed"},
    }))
    .is_err());
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
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.json);
}

#[test]
fn task_start_parser_rejects_plan_source_conflicts() {
    let error = match Cli::try_parse_from([
        "ait-cli",
        "task",
        "start",
        "--from",
        "docs/sprints/card.md#card/implement",
        "--title",
        "Duplicate title",
        "--intent",
        "Implement the exact synchronized Plan item",
    ]) {
        Ok(_) => panic!("--from and --title must be mutually exclusive"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("cannot be used with"));
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
fn task_parser_freezes_the_supported_command_surface() {
    let help = match Cli::try_parse_from(["ait-cli", "task", "--help"]) {
        Ok(_) => panic!("task help should exit through clap"),
        Err(error) => error,
    };
    let rendered = help.to_string();
    for command in ["start", "list", "show", "audit", "finish", "abandon"] {
        assert!(
            rendered.contains(&format!("\n  {command}")),
            "missing task command {command}"
        );
    }
    for removed in [
        "land", "tokens", "canceled", "complete", "restart", "publish",
    ] {
        assert!(
            !rendered.contains(&format!("\n  {removed}")),
            "retired command leaked: {removed}"
        );
    }

    for command in [
        "land", "tokens", "canceled", "complete", "publish", "restart",
    ] {
        let removed = match Cli::try_parse_from(["ait-cli", "task", command, "LCT-1"]) {
            Ok(_) => panic!("removed task command must not parse"),
            Err(error) => error,
        };
        assert!(removed.to_string().contains("unrecognized subcommand"));
    }
}

#[test]
fn task_scope_overrides_are_consistent_and_mutually_exclusive() {
    let invocations = [
        vec![
            "ait-cli", "task", "start", "--title", "Task", "--intent", "Intent",
        ],
        vec!["ait-cli", "task", "list"],
        vec!["ait-cli", "task", "show", "LCT-1"],
        vec!["ait-cli", "task", "audit", "LCT-1"],
        vec!["ait-cli", "task", "finish", "LCT-1"],
        vec!["ait-cli", "task", "abandon", "LCT-1"],
    ];

    for invocation in invocations {
        let mut local = invocation.clone();
        local.push("--local");
        Cli::try_parse_from(local).expect("explicit local compatibility scope must parse");

        let mut remote = invocation.clone();
        remote.extend(["--remote", "origin"]);
        Cli::try_parse_from(remote).expect("explicit remote compatibility scope must parse");

        let mut conflict = invocation;
        conflict.extend(["--local", "--remote", "origin"]);
        let error = match Cli::try_parse_from(conflict) {
            Ok(_) => panic!("local and remote compatibility scopes must conflict"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("cannot be used with"));
    }
}

#[test]
fn task_parser_rejects_every_retired_option_and_requires_finish_identity() {
    let retired_cases = [
        vec![
            "ait-cli",
            "task",
            "start",
            "--title",
            "Task",
            "--intent",
            "Intent",
            "--task-only",
        ],
        vec![
            "ait-cli",
            "task",
            "start",
            "--title",
            "Task",
            "--intent",
            "Intent",
            "--change-title",
            "Change",
        ],
        vec![
            "ait-cli",
            "task",
            "start",
            "--title",
            "Task",
            "--intent",
            "Intent",
            "--base-line",
            "main",
        ],
        vec![
            "ait-cli",
            "task",
            "start",
            "--title",
            "Task",
            "--intent",
            "Intent",
            "--verbose",
        ],
        vec![
            "ait-cli",
            "task",
            "start",
            "--from",
            "docs/sprints/card.md#card/item",
            "--intent",
            "Intent",
            "--title-override",
            "Override",
        ],
        vec!["ait-cli", "task", "show", "LCT-1", "--repo", "other"],
        vec![
            "ait-cli",
            "task",
            "audit",
            "LCT-1",
            "--target-line",
            "other",
        ],
        vec![
            "ait-cli",
            "task",
            "finish",
            "LCT-1",
            "--snapshot-message",
            "Snapshot",
        ],
        vec!["ait-cli", "task", "finish", "LCT-1", "--summary", "Summary"],
        vec!["ait-cli", "task", "finish", "LCT-1", "--tests", "Tests"],
        vec!["ait-cli", "task", "finish", "LCT-1", "--lint", "Lint"],
        vec![
            "ait-cli",
            "task",
            "finish",
            "LCT-1",
            "--security",
            "Security",
        ],
        vec!["ait-cli", "task", "finish", "LCT-1", "--license", "License"],
        vec![
            "ait-cli",
            "task",
            "finish",
            "LCT-1",
            "--author-mode",
            "human_only",
        ],
        vec!["ait-cli", "task", "finish", "LCT-1", "--model", "model"],
        vec![
            "ait-cli",
            "task",
            "finish",
            "LCT-1",
            "--reviewer",
            "reviewer",
        ],
        vec![
            "ait-cli",
            "task",
            "finish",
            "LCT-1",
            "--review-message",
            "Review",
        ],
        vec!["ait-cli", "task", "finish", "LCT-1", "--target", "other"],
        vec!["ait-cli", "task", "finish", "LCT-1", "--mode", "merge"],
        vec!["ait-cli", "task", "finish", "LCT-1", "--preview"],
        vec!["ait-cli", "task", "abandon", "LCT-1", "--abandoned"],
        vec![
            "ait-cli",
            "task",
            "abandon",
            "LCT-1",
            "--exclude-later-promotion",
        ],
    ];

    for argv in retired_cases {
        let error = match Cli::try_parse_from(argv.clone()) {
            Ok(_) => panic!("retired Task option must fail during parsing"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("unexpected argument"),
            "unexpected parser result for {argv:?}: {error}"
        );
    }

    let missing_id = match Cli::try_parse_from(["ait-cli", "task", "finish"]) {
        Ok(_) => panic!("Task finish identity must be parser-required"),
        Err(error) => error,
    };
    assert!(missing_id.to_string().contains("required"));

    for identity in ["LCT-1", "LCT-1/C-02"] {
        let parsed = Cli::try_parse_from([
            "ait-cli",
            "task",
            "finish",
            identity,
            "--message",
            "Final Snapshot",
            "--local",
        ])
        .expect("Task finish must accept Task or Change identity and a local Snapshot message");
        let Commands::Task {
            command: TaskCommand::Finish(args),
        } = parsed.command
        else {
            panic!("expected task finish command");
        };
        assert_eq!(args.task_or_change_id, identity);
        assert_eq!(args.message.as_deref(), Some("Final Snapshot"));
        assert!(args.local);
    }

    let retired_land = Cli::try_parse_from(["ait-cli", "task", "land", "LCT-1"])
        .err()
        .expect("task land must remain retired");
    assert_eq!(
        retired_land.kind(),
        clap::error::ErrorKind::InvalidSubcommand
    );

    let forbidden_change_land = Cli::try_parse_from(["ait-cli", "change", "land", "LCT-1/C-01"])
        .err()
        .expect("change land must not exist");
    assert_eq!(
        forbidden_change_land.kind(),
        clap::error::ErrorKind::InvalidSubcommand
    );
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

fn plan_scope_runtime(
    workflow_mode: &str,
    workflow_default_scope: &str,
    default_remote: bool,
) -> (TempDir, RepoRuntime) {
    let temp = TempDir::new().unwrap();
    let default_remote_entry = if default_remote {
        r#""default_remote": "origin","#
    } else {
        ""
    };
    write_runtime_config(
        temp.path(),
        &format!(
            r#"{{
  "repo_name": "fixture",
  "workflow_mode": "{workflow_mode}",
  "workflow_default_scope": "{workflow_default_scope}",
  "task_default_scope": "{workflow_default_scope}",
  "change_default_scope": "{workflow_default_scope}",
  {default_remote_entry}
  "remotes": {{
    "origin": {{
      "url": "http://example.test/fixture",
      "repo_name": "fixture"
    }}
  }}
}}"#
        ),
    );
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    (temp, repo)
}

fn plan_query_payload(
    repo: &RepoRuntime,
    local: bool,
    remote: Option<&str>,
) -> Result<JsonValue, String> {
    parse_value_error_string(&build_query_request(
        repo,
        &QueryScopeArgs {
            local,
            remote: remote.map(str::to_string),
            json: true,
        },
    )?)
}

fn plan_sync_test_args(local: bool, remote: Option<&str>) -> SyncArgs {
    SyncArgs {
        target: PathBuf::from("docs/sprints/card.md"),
        plan_ref: None,
        prune: false,
        local,
        remote: remote.map(str::to_string),
        rebase: false,
        reconcile: false,
        json: true,
    }
}

#[test]
fn plan_scope_builders_follow_workflow_defaults_and_explicit_overrides() {
    let (_local_temp, local_repo) = plan_scope_runtime("solo_local", "local", true);
    let local_default = plan_query_payload(&local_repo, false, None).unwrap();
    assert_eq!(local_default["scope"], "local");
    assert!(local_default.get("base_url").is_none());

    let local_remote_override = plan_query_payload(&local_repo, false, Some("origin")).unwrap();
    assert_eq!(local_remote_override["scope"], "remote");
    assert_eq!(local_remote_override["remote"], "origin");

    let (_remote_temp, remote_repo) = plan_scope_runtime("solo_remote", "remote", true);
    let remote_default = plan_query_payload(&remote_repo, false, None).unwrap();
    assert_eq!(remote_default["scope"], "remote");
    assert_eq!(remote_default["remote"], "origin");
    assert_eq!(remote_default["base_url"], "http://example.test/fixture");

    let remote_local_override = plan_query_payload(&remote_repo, true, None).unwrap();
    assert_eq!(remote_local_override["scope"], "local");
    assert!(remote_local_override.get("base_url").is_none());

    let candidates = parse_value_error_string(
        &build_candidates_request(
            &remote_repo,
            &CandidatesArgs {
                local: false,
                remote: None,
                include_all: true,
                contains: Some("alpha,beta".to_string()),
                json: true,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(candidates["scope"], "remote");
    assert_eq!(candidates["remote"], "origin");
    assert_eq!(candidates["contains_terms"], json!(["alpha", "beta"]));

    let remote_sync = parse_value_error_string(
        &build_sync_request(&remote_repo, &plan_sync_test_args(false, None)).unwrap(),
    )
    .unwrap();
    assert_eq!(remote_sync["local"], false);
    assert_eq!(remote_sync["remote_name"], "origin");
    assert_eq!(remote_sync["base_url"], "http://example.test/fixture");

    let local_sync = parse_value_error_string(
        &build_sync_request(&remote_repo, &plan_sync_test_args(true, None)).unwrap(),
    )
    .unwrap();
    assert_eq!(local_sync["local"], true);
    assert!(local_sync["remote_name"].is_null());
    assert!(local_sync["base_url"].is_null());
}

#[test]
fn plan_sync_from_a_worktree_uses_the_authoritative_repository_root_for_all_scopes() {
    let temp = TempDir::new().unwrap();
    let authoritative_root = temp.path().join("canonical");
    let worktree_root = temp.path().join("worktree");
    fs::create_dir_all(&authoritative_root).unwrap();
    write_runtime_config(
        &worktree_root,
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
    fs::write(
        worktree_root.join(".ait-worktree.json"),
        serde_json::to_vec(&json!({
            "repo_root": authoritative_root,
            "workspace_root": worktree_root,
            "worktree_name": "fixture-task",
            "current_line": "feature/fixture-task"
        }))
        .unwrap(),
    )
    .unwrap();

    let repo = RepoRuntime::discover_from_path(&worktree_root).unwrap();
    assert!(repo.is_worktree());
    assert_ne!(repo.workspace_root(), repo.authoritative_repo_root());

    let local = parse_value_error_string(
        &build_sync_request(&repo, &plan_sync_test_args(true, None)).unwrap(),
    )
    .unwrap();
    let remote = parse_value_error_string(
        &build_sync_request(&repo, &plan_sync_test_args(false, Some("origin"))).unwrap(),
    )
    .unwrap();
    let expected_root = repo.authoritative_repo_root().to_string_lossy().to_string();

    for payload in [&local, &remote] {
        assert_eq!(payload["root_path"], expected_root);
        assert_eq!(payload["plan_storage"]["repo_root"], expected_root);
        assert_eq!(
            payload["target"],
            JsonValue::String("docs/sprints/card.md".to_string())
        );
    }
    assert_eq!(local["local"], true);
    assert!(local["base_url"].is_null());
    assert_eq!(remote["local"], false);
    assert_eq!(remote["base_url"], "http://example.test/fixture");
}

#[test]
fn remote_default_plan_scope_without_a_default_remote_fails_closed() {
    let (_temp, repo) = plan_scope_runtime("solo_remote", "remote", false);
    let query_error = plan_query_payload(&repo, false, None).unwrap_err();
    assert!(
        query_error.contains("No remote configured"),
        "{query_error}"
    );

    let candidates_error = build_candidates_request(
        &repo,
        &CandidatesArgs {
            local: false,
            remote: None,
            include_all: false,
            contains: None,
            json: true,
        },
    )
    .unwrap_err();
    assert!(
        candidates_error.contains("No remote configured"),
        "{candidates_error}"
    );

    let sync_error = build_sync_request(&repo, &plan_sync_test_args(false, None)).unwrap_err();
    assert!(sync_error.contains("No remote configured"), "{sync_error}");

    let empty_remote = plan_query_payload(&repo, false, Some("  ")).unwrap_err();
    assert!(
        empty_remote.contains("requires a non-empty remote name"),
        "{empty_remote}"
    );
}

#[test]
fn plan_parser_rejects_scope_retry_and_human_expansion_conflicts() {
    for argv in [
        vec!["ait", "plan", "list", "--local", "--remote", "origin"],
        vec![
            "ait", "plan", "show", "PR-1", "--local", "--remote", "origin",
        ],
        vec!["ait", "plan", "candidates", "--local", "--remote", "origin"],
        vec![
            "ait",
            "plan",
            "sync",
            "docs/plan.md",
            "--local",
            "--remote",
            "origin",
        ],
        vec![
            "ait",
            "plan",
            "sync",
            "docs/plan.md",
            "--rebase",
            "--reconcile",
        ],
        vec!["ait", "plan", "list", "--all", "--json"],
        vec!["ait", "plan", "revisions", "PR-1", "--all", "--json"],
    ] {
        let error = Cli::try_parse_from(argv.clone())
            .err()
            .expect("options must conflict");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "argv: {argv:?}\n{error}"
        );
    }

    Cli::try_parse_from(["ait", "plan", "sync", "docs/plan.md", "--rebase"])
        .expect("effective solo_remote scope can satisfy --rebase at runtime");
}

#[test]
fn plan_help_documents_every_public_subcommand_and_option_contract() {
    let top_help = Cli::try_parse_from(["ait", "plan", "--help"])
        .err()
        .expect("help should be rendered")
        .to_string();
    for fragment in [
        "solo_local",
        "solo_remote",
        "list",
        "show",
        "revisions",
        "items",
        "candidates",
        "inspect",
        "sync",
    ] {
        assert!(
            top_help.contains(fragment),
            "missing {fragment}: {top_help}"
        );
    }

    for (subcommand, expected) in [
        (
            "list",
            vec!["--local", "--remote", "--json", "--all", "archived"],
        ),
        (
            "show",
            vec![
                "<PLAN>",
                "--revision",
                "plan-revision:<index>",
                "--local",
                "--remote",
                "--json",
            ],
        ),
        (
            "revisions",
            vec!["<PLAN>", "published-plan:<index>", "--all", "--json"],
        ),
        (
            "items",
            vec!["<PLAN>", "--revision", "--local", "--remote", "--json"],
        ),
        (
            "candidates",
            vec![
                "--local",
                "--remote",
                "--all",
                "zero taskable",
                "--contains",
                "case-insensitive",
                "--json",
            ],
        ),
        (
            "inspect",
            vec!["<PLAN>", "--revision", "Task-readiness", "--json"],
        ),
        (
            "sync",
            vec![
                "<TARGET>",
                "--plan-ref",
                "--prune",
                "--local",
                "--remote",
                "--rebase",
                "--reconcile",
                "--json",
                "solo_remote",
                "Snapshot",
                "Line",
            ],
        ),
    ] {
        let help = Cli::try_parse_from(["ait", "plan", subcommand, "--help"])
            .err()
            .expect("help should be rendered")
            .to_string();
        for fragment in expected {
            assert!(
                help.contains(fragment),
                "{subcommand} help missing {fragment}: {help}"
            );
        }
    }
}

#[test]
fn local_task_finish_render_compacts_successful_closeout_and_keeps_material_retention() {
    let rendered = render_local_task_land_text(&json!({
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

    assert!(rendered.contains("finished: LCC-1 -> main @ SNP-1"));
    assert!(rendered.contains("closed: task, line, sprint"));
    assert!(rendered.contains("retention: pruned (3 removed, 20 retained)"));
    assert!(!rendered.contains("checklist reason"));
}

#[test]
fn remote_task_finish_render_surfaces_separate_plan_sync_action() {
    let rendered = render_task_finish_text(&json!({
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
            "detail": "Remote task finish completed without reading or synchronizing Plan state.",
            "command": "ait plan sync <bound-sprint-card-path> --remote origin"
        }
    }))
    .unwrap();

    assert!(rendered.contains("ait task finish"));
    assert!(rendered.contains("Feature Line closeout"));
    assert!(rendered.contains("- archived: feature/rct-1"));
    assert!(rendered.contains("Sprint checklist closeout"));
    assert!(rendered.contains("- deferred"));
    assert!(rendered
        .contains("Remote task finish completed without reading or synchronizing Plan state."));
    assert!(rendered.contains("ait plan sync <bound-sprint-card-path> --remote origin"));
}

#[test]
fn task_finish_render_surfaces_versioned_partial_recovery() {
    let rendered = render_task_finish_text(&json!({
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
            "detail": "Repair the Plan drift and rerun task finish.",
            "command": "ait task finish LCC-9 --local"
        },
        "plan_checklist_closeout": {
            "status": "skipped",
            "reason": "artifact_has_unsynced_drift"
        }
    }))
    .unwrap();

    assert!(rendered.contains("task-finish contract: task-land-plan-closeout/v1"));
    assert!(rendered.contains("closeout: partial"));
    assert!(rendered.contains("Repair the Plan drift and rerun task finish."));
    assert!(rendered.contains("ait task finish LCC-9 --local"));
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
fn task_audit_change_text_projection_accepts_flat_remote_rows() {
    let rows = vec![json!({
        "change_id": "C-01",
        "change_ref": "RWCT-0008/C-01",
        "task_id": "RWCT-0008",
        "status": "landed"
    })];

    let (projected, has_target_state) = project_task_audit_change_text_rows(&rows);

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0]["change"], "RWCT-0008/C-01");
    assert_eq!(projected[0]["status"], "landed");
    assert_eq!(projected[0]["target_state"], "");
    assert!(!has_target_state);
}

#[test]
fn task_audit_change_text_projection_preserves_nested_local_target_state() {
    let rows = vec![json!({
        "change": {
            "change_id": "C-01",
            "change_ref": "LCT-0767/C-01",
            "task_id": "LCT-0767",
            "status": "draft"
        },
        "target_state": "local_change_not_landed"
    })];

    let (projected, has_target_state) = project_task_audit_change_text_rows(&rows);

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0]["change"], "LCT-0767/C-01");
    assert_eq!(projected[0]["status"], "draft");
    assert_eq!(projected[0]["target_state"], "local_change_not_landed");
    assert!(has_target_state);
}

#[test]
fn task_audit_change_text_projection_omits_empty_inventory() {
    let (projected, has_target_state) = project_task_audit_change_text_rows(&[]);

    assert!(projected.is_empty());
    assert!(!has_target_state);
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
    assert_eq!(args.base_line.as_deref(), Some("main"));
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.json);

    let defaulted = Cli::try_parse_from([
        "ait-cli",
        "change",
        "create",
        "LCT-1",
        "--title",
        "Continue local task",
        "--local",
    ])
    .expect("change create should derive its base Line when omitted");
    let Commands::Change {
        command: ChangeCommand::Create(args),
    } = defaulted.command
    else {
        panic!("expected defaulted change create command");
    };
    assert_eq!(args.base_line, None);
    assert!(args.local);
}

#[test]
fn change_parser_rejects_repo_overrides_and_conflicting_scope_flags() {
    for args in [
        vec!["change", "show", "LCT-1/C-01", "--repo", "other"],
        vec!["change", "revert", "LCT-1/C-01", "--repo", "other"],
        vec!["change", "replay", "LCT-1/C-01", "--repo", "other"],
    ] {
        let error = Cli::try_parse_from(std::iter::once("ait-cli").chain(args))
            .err()
            .expect("Change --repo must be removed");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    for args in [
        vec![
            "change", "create", "LCT-1", "--title", "Change", "--local", "--remote", "origin",
        ],
        vec!["change", "list", "--local", "--remote", "origin"],
        vec![
            "change",
            "show",
            "LCT-1/C-01",
            "--local",
            "--remote",
            "origin",
        ],
        vec![
            "change",
            "revert",
            "LCT-1/C-01",
            "--local",
            "--remote",
            "origin",
        ],
        vec![
            "change",
            "replay",
            "LCT-1/C-01",
            "--local",
            "--remote",
            "origin",
        ],
        vec![
            "change",
            "close",
            "LCT-1/C-01",
            "--local",
            "--remote",
            "origin",
        ],
    ] {
        let error = Cli::try_parse_from(std::iter::once("ait-cli").chain(args))
            .err()
            .expect("Change scope flags must conflict during parsing");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[test]
fn change_replay_defaults_to_current_line_and_retains_hidden_onto_compatibility() {
    let parsed = Cli::try_parse_from(["ait-cli", "change", "replay", "LCT-1/C-01"])
        .expect("change replay should default to the current Line");
    let Commands::Change {
        command: ChangeCommand::Replay(args),
    } = parsed.command
    else {
        panic!("expected change replay command");
    };
    assert_eq!(args.onto, None);

    let compatible = Cli::try_parse_from([
        "ait-cli",
        "change",
        "replay",
        "LCT-1/C-01",
        "--onto",
        "main",
    ])
    .expect("legacy explicit current-Line assertion should remain parseable");
    let Commands::Change {
        command: ChangeCommand::Replay(args),
    } = compatible.command
    else {
        panic!("expected compatible change replay command");
    };
    assert_eq!(args.onto.as_deref(), Some("main"));
}

#[test]
fn change_help_explains_retained_behavior_and_hides_compatibility_inputs() {
    fn help(command: Option<&str>) -> String {
        let mut args = vec!["ait-cli", "change"];
        if let Some(command) = command {
            args.push(command);
        }
        args.push("--help");
        Cli::try_parse_from(args)
            .err()
            .expect("--help must render Clap help")
            .to_string()
    }

    let parent = help(None);
    for description in [
        "additional Change",
        "open Changes or complete Change history",
        "Archive one Change",
        "local draft Change record",
    ] {
        assert!(parent.contains(description), "{parent}");
    }

    let create = help(Some("create"));
    assert!(
        create.contains("defaults to the bound worktree target Line"),
        "{create}"
    );
    assert!(create.contains("--base-line <LINE>"), "{create}");

    let list = help(Some("list"));
    assert!(
        list.contains("bounded view still applies unless --all"),
        "{list}"
    );

    for command in ["show", "revert", "replay"] {
        let rendered = help(Some(command));
        assert!(!rendered.contains("--repo"), "{rendered}");
    }

    let revert = help(Some("revert"));
    assert!(revert.contains("does not create a Snapshot"), "{revert}");
    assert!(
        revert.contains("only local workspace files are changed"),
        "{revert}"
    );

    let replay = help(Some("replay"));
    assert!(replay.contains("does not create a Snapshot"), "{replay}");
    assert!(
        replay.contains("only local workspace files are changed"),
        "{replay}"
    );
    assert!(!replay.contains("--onto"), "{replay}");

    let close = help(Some("close"));
    assert!(
        close.contains("Archive one local or remote Change"),
        "{close}"
    );
    assert!(close.contains("without landing code"), "{close}");

    let publish = help(Some("publish"));
    assert!(publish.contains("does not publish a Patchset"), "{publish}");
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

    for args in [
        vec![
            "ait-cli",
            "snapshot",
            "ancestry",
            "SNP-MERGE",
            "--max-depth",
            "0",
        ],
        vec![
            "ait-cli",
            "snapshot",
            "ancestry",
            "SNP-MERGE",
            "--limit",
            "0",
        ],
    ] {
        let error = Cli::try_parse_from(args)
            .err()
            .expect("non-positive ancestry bounds must fail during parsing");
        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
        assert!(error.to_string().contains("value must be 1 or greater"));
    }

    let help = match Cli::try_parse_from(["ait-cli", "snapshot", "is-ancestor", "--help"]) {
        Ok(_) => panic!("--help should render clap help"),
        Err(error) => error.to_string(),
    };
    assert!(help.contains("exits 0 when true, 1 when false, and 2"));
    assert!(help.contains("<OLDER_SNAPSHOT_OR_TAG> <NEWER_SNAPSHOT_OR_TAG>"));
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
        "cleanup",
        "--idle-for",
        "30d",
        "--kind",
        "review",
        "--limit",
        "5",
        "--include-protected",
        "--all",
    ])
    .expect("complete Line cleanup evidence flag should parse");
    let Commands::Line {
        command: LineCommand::Cleanup(cleanup),
    } = cleanup.command
    else {
        panic!("expected Line cleanup command");
    };
    assert_eq!(cleanup.idle_for, "30d");
    assert_eq!(cleanup.cleanup_kind.as_deref(), Some("review"));
    assert_eq!(cleanup.limit, Some(5));
    assert!(cleanup.include_protected);
    assert!(cleanup.all);
    assert!(!cleanup.yes);
}

#[test]
fn line_merge_parser_freezes_start_continue_abort_and_conflicts() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "line",
        "merge",
        "feature/source",
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
    assert!(!help.contains("--into"));

    let removed = Cli::try_parse_from([
        "ait-cli",
        "line",
        "merge",
        "feature/source",
        "--into",
        "feature/target",
    ])
    .err()
    .expect("removed line merge --into must fail")
    .to_string();
    assert!(
        removed.contains("unexpected argument '--into'"),
        "{removed}"
    );
}

#[test]
fn line_create_keeps_selection_but_routes_workspace_restore_through_switch() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "line",
        "create",
        "feature/example",
        "--from-snapshot",
        "SNP-EXAMPLE",
        "--switch",
        "--json",
    ])
    .expect("line create selection should parse");
    let Commands::Line {
        command: LineCommand::Create(args),
    } = parsed.command
    else {
        panic!("expected line create command");
    };
    assert_eq!(args.name, "feature/example");
    assert_eq!(args.from_snapshot.as_deref(), Some("SNP-EXAMPLE"));
    assert!(args.switch);
    assert!(args.json);

    for removed_option in ["--restore", "--force"] {
        let error = Cli::try_parse_from([
            "ait-cli",
            "line",
            "create",
            "feature/example",
            removed_option,
        ])
        .err()
        .expect("removed line create workspace option must fail")
        .to_string();
        assert!(error.contains("unexpected argument"), "{error}");
    }

    let force_without_restore =
        Cli::try_parse_from(["ait-cli", "line", "switch", "main", "--force"])
            .err()
            .expect("line switch --force must require --restore")
            .to_string();
    assert!(
        force_without_restore.contains("--restore"),
        "{force_without_restore}"
    );
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
fn git_interop_parser_exposes_only_exact_user_choices() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "git",
        "import",
        "../legacy.git",
        "--all-branches-and-tags",
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
    assert!(args.all_branches_and_tags);
    assert!(!args.dry_run);
    assert!(args.json);

    let parsed = Cli::try_parse_from([
        "ait-cli",
        "git",
        "export",
        "../escape.git",
        "--all-lines-and-tags",
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
    assert!(args.all_lines_and_tags);
    assert!(args.dry_run);
    assert!(args.json);

    let parsed = Cli::try_parse_from([
        "ait-cli",
        "git",
        "mirror",
        "../team.git",
        "--direction",
        "bidirectional",
        "--dry-run",
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
    assert!(args.json);

    for removed in [
        &["ait-cli", "git", "import", "fixture.git", "--all-refs"][..],
        &["ait-cli", "git", "export", "fixture.git", "--all-refs"][..],
        &["ait-cli", "git", "import", "fixture.git", "--resume"][..],
        &["ait-cli", "git", "export", "fixture.git", "--resume"][..],
        &[
            "ait-cli",
            "git",
            "mirror",
            "fixture.git",
            "--direction",
            "inbound",
            "--once",
        ][..],
        &[
            "ait-cli",
            "git",
            "import",
            "fixture.git",
            "--all-lines-and-tags",
        ][..],
        &[
            "ait-cli",
            "git",
            "export",
            "fixture.git",
            "--all-branches-and-tags",
        ][..],
    ] {
        let error = Cli::try_parse_from(removed)
            .err()
            .expect("removed or foreign Git option must fail")
            .to_string();
        assert!(error.contains("unexpected argument"), "{error}");
    }

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
fn git_interop_help_explains_scope_mutation_and_endpoint_constraints() {
    let import_help = Cli::try_parse_from(["ait-cli", "git", "import", "--help"])
        .err()
        .expect("import help must stop parsing")
        .to_string();
    assert!(import_help.contains("local repository path or a Git remote URL"));
    assert!(import_help.contains("--all-branches-and-tags"));
    assert!(import_help.contains("only the source HEAD branch"));
    assert!(import_help.contains("without writing AIT data"));

    let export_help = Cli::try_parse_from(["ait-cli", "git", "export", "--help"])
        .err()
        .expect("export help must stop parsing")
        .to_string();
    assert!(export_help.contains("Local Git repository path"));
    assert!(export_help.contains("--all-lines-and-tags"));
    assert!(export_help.contains("only the current Line"));
    assert!(export_help.contains("without writing the target or AIT data"));

    let mirror_help = Cli::try_parse_from(["ait-cli", "git", "mirror", "--help"])
        .err()
        .expect("mirror help must stop parsing")
        .to_string();
    assert!(mirror_help.contains("local Git path for outbound or bidirectional mode"));
    assert!(mirror_help.contains("--direction <DIRECTION>"));
    assert!(mirror_help.contains("complete branch and tag set"));
    assert!(!mirror_help.contains("--once"));
}

#[test]
fn pull_parser_enforces_explicit_workspace_mutation_contract() {
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

    let merge_without_restore =
        Cli::try_parse_from(["ait-cli", "pull", "--line", "feature/demo", "--merge"])
            .err()
            .expect("pull merge must require explicit restore intent");
    assert_eq!(
        merge_without_restore.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    let merge_without_restore = merge_without_restore.to_string();
    assert!(
        merge_without_restore.contains("--restore"),
        "{merge_without_restore}"
    );

    let force_without_restore = Cli::try_parse_from(["ait-cli", "pull", "--force"])
        .err()
        .expect("pull force must require explicit restore intent");
    assert_eq!(
        force_without_restore.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    let force_without_restore = force_without_restore.to_string();
    assert!(
        force_without_restore.contains("--restore"),
        "{force_without_restore}"
    );

    let conflict = Cli::try_parse_from([
        "ait-cli",
        "pull",
        "--line",
        "feature/demo",
        "--merge",
        "--restore",
        "--force",
    ])
    .err()
    .expect("pull merge must reject forced workspace overwrite");
    assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);
    let conflict = conflict.to_string();
    assert!(conflict.contains("cannot be used with"), "{conflict}");
}

#[test]
fn pull_help_explains_defaults_requirements_and_workspace_effects() {
    let help = Cli::try_parse_from(["ait-cli", "pull", "--help"])
        .err()
        .expect("pull help must stop parsing")
        .to_string();

    for expected in [
        "repository's default remote",
        "current local Line",
        "requires --restore and a clean workspace",
        "select that Line",
        "rejected when the local Line is ahead",
        "cannot be used with --merge",
        "machine-readable JSON",
    ] {
        assert!(help.contains(expected), "missing {expected:?} in:\n{help}");
    }
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
fn stash_help_explains_workspace_line_and_output_contracts() {
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["stash", "--help"],
            &[
                "temporary local-only workspace Snapshots",
                "only while its source Line is current",
            ],
        ),
        (
            &["stash", "save", "--help"],
            &[
                "without advancing the current Line head",
                "By default, restore the current Line head",
                "optional human-readable message",
                "workspace remains dirty relative to it",
                "machine-readable JSON",
            ],
        ),
        (
            &["stash", "list", "--help"],
            &["active local-only stash metadata", "machine-readable JSON"],
        ),
        (
            &["stash", "show", "--help"],
            &[
                "does not display a content diff",
                "Exact active stash ID",
                "machine-readable JSON",
            ],
        ),
        (
            &["stash", "apply", "--help"],
            &[
                "entire managed workspace",
                "current Line must be the stash's source Line",
                "rather than applying a patch or three-way merge",
                "does not permit restoring a stash from another Line",
                "machine-readable JSON",
            ],
        ),
        (
            &["stash", "pop", "--help"],
            &[
                "drop its stash record only after a successful restore",
                "current Line must be the stash's source Line",
                "rather than applying a patch or three-way merge",
                "does not permit restoring a stash from another Line",
                "machine-readable JSON",
            ],
        ),
        (
            &["stash", "drop", "--help"],
            &[
                "without changing workspace content",
                "Exact active stash ID",
                "machine-readable JSON",
            ],
        ),
    ];

    for (args, expected_fragments) in cases {
        let help = Cli::try_parse_from(std::iter::once("ait-cli").chain(args.iter().copied()))
            .err()
            .expect("stash help must stop parsing")
            .to_string();
        for expected in *expected_fragments {
            assert!(
                help.contains(expected),
                "missing {expected:?} from `{}` help:\n{help}",
                args.join(" ")
            );
        }
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
fn worktree_help_documents_every_public_command_and_option_contract() {
    let parent_help = Cli::try_parse_from(["ait-cli", "worktree", "--help"])
        .err()
        .expect("worktree help")
        .to_string();
    for expected in [
        "isolated worktrees",
        "Normal Task worktrees are created by task start",
        "Cleanup, prune-stale, and remove require --yes",
        "status",
        "restore",
        "show",
        "path",
        "doctor",
        "cleanup-candidates",
        "cleanup",
        "prune-stale",
        "list",
        "sync",
        "recreate",
        "recover-task",
        "restore-owned-head",
        "rebase",
        "remove",
    ] {
        assert!(
            parent_help.contains(expected),
            "missing {expected:?}:\n{parent_help}"
        );
    }

    let cases: &[(&[&str], &[&str])] = &[
        (
            &["status"],
            &[
                "current checkout",
                "--snapshot <SNAPSHOT_ID>",
                "--line <LINE_NAME>",
                "--verbose",
                "--json",
            ],
        ),
        (
            &["restore"],
            &[
                "current checkout",
                "--path <PATH>",
                "Unsaved changes require --force",
                "--dry-run",
                "--json",
            ],
        ),
        (
            &["show"],
            &[
                "Task/Change binding",
                "current runtime worktree binding",
                "--json",
            ],
        ),
        (
            &["path"],
            &[
                "print-only behavior",
                "--shell",
                "managed Cargo paths",
                "--json",
            ],
        ),
        (
            &["doctor"],
            &["metadata and cached status", "--refresh", "--json"],
        ),
        (
            &["cleanup-candidates"],
            &[
                "without removing paths or registrations",
                "--older-than <DURATION>",
                "manual_only, after_remote_land, after_task_complete, after_idle, or never",
                "--allow-manual-only",
                "--include-protected",
                "--json",
            ],
        ),
        (
            &["cleanup"],
            &[
                "Applied cleanup deletes",
                "--older-than <DURATION>",
                "--policy <POLICY>",
                "--limit <COUNT>",
                "--dry-run",
                "--yes",
                "--json",
            ],
        ),
        (
            &["prune-stale"],
            &[
                "without deleting surviving checkout content",
                "--dry-run",
                "--yes",
                "--json",
            ],
        ),
        (
            &["list"],
            &["cached workspace status", "--refresh", "--json"],
        ),
        (
            &["sync"],
            &[
                "--all",
                "cannot be combined with NAME or --line",
                "--force",
                "--dry-run",
                "--json",
            ],
        ),
        (
            &["recreate"],
            &[
                "Task-bound worktree",
                "selected remote Patchset revision",
                "--dry-run",
                "--json",
            ],
        ),
        (
            &["recover-task"],
            &[
                "main repository root",
                "<TASK_ID>",
                "--change <CHANGE>",
                "--remote <REMOTE>",
                "--dry-run",
                "--json",
            ],
        ),
        (
            &["restore-owned-head"],
            &[
                "first foreign Snapshot",
                "Task-bound worktree name",
                "--dry-run",
                "--json",
            ],
        ),
        (
            &["rebase"],
            &[
                "--onto <LINE_NAME>",
                "--continue",
                "--abort",
                "--dry-run",
                "--json",
            ],
        ),
        (
            &["remove"],
            &[
                "leaving ordinary checkout content unless --delete-path",
                "--all-stale",
                "--delete-path",
                "--force",
                "--dry-run",
                "--yes",
                "--json",
            ],
        ),
    ];

    for (command, expected_fragments) in cases {
        let help = Cli::try_parse_from(
            ["ait-cli", "worktree"]
                .into_iter()
                .chain(command.iter().copied())
                .chain(["--help"]),
        )
        .err()
        .unwrap_or_else(|| panic!("{} help should stop parsing", command.join(" ")))
        .to_string();
        for expected in *expected_fragments {
            assert!(
                help.contains(expected),
                "missing {expected:?} from `worktree {}` help:\n{help}",
                command.join(" ")
            );
        }
    }
}

#[test]
fn worktree_destructive_confirmation_flags_parse_and_share_one_dispatch_contract() {
    let cleanup = Cli::try_parse_from(["ait-cli", "worktree", "cleanup", "--dry-run", "--yes"])
        .expect("cleanup confirmation should parse");
    let Commands::Worktree {
        command: WorktreeCommand::Cleanup(cleanup),
    } = cleanup.command
    else {
        panic!("expected worktree cleanup command");
    };
    assert!(cleanup.dry_run);
    assert!(cleanup.yes);

    let prune = Cli::try_parse_from(["ait-cli", "worktree", "prune-stale", "--dry-run", "--yes"])
        .expect("prune confirmation should parse");
    let Commands::Worktree {
        command: WorktreeCommand::PruneStale(prune),
    } = prune.command
    else {
        panic!("expected worktree prune-stale command");
    };
    assert!(prune.dry_run);
    assert!(prune.yes);

    let remove = Cli::try_parse_from([
        "ait-cli",
        "worktree",
        "remove",
        "rt-1",
        "--delete-path",
        "--force",
        "--yes",
    ])
    .expect("remove confirmation should parse");
    let Commands::Worktree {
        command: WorktreeCommand::Remove(remove),
    } = remove.command
    else {
        panic!("expected worktree remove command");
    };
    assert_eq!(remove.names, vec!["rt-1"]);
    assert!(remove.delete_path);
    assert!(remove.force);
    assert!(remove.yes);

    let error = require_worktree_destructive_confirmation(false, false)
        .expect_err("applied removal must require confirmation");
    assert_eq!(error, WORKTREE_DESTRUCTIVE_CONFIRMATION_ERROR);
    assert!(require_worktree_destructive_confirmation(true, false).is_ok());
    assert!(require_worktree_destructive_confirmation(true, true).is_ok());
    assert!(require_worktree_destructive_confirmation(false, true).is_ok());
}

#[test]
fn patchset_publish_parser_rejects_allow_empty() {
    let err = match Cli::try_parse_from([
        "ait-cli",
        "patchset",
        "publish",
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
fn patchset_parser_exposes_only_the_remote_authority_contract() {
    let published = Cli::try_parse_from([
        "ait-cli",
        "patchset",
        "publish",
        "RCT-1/C-01",
        "--summary",
        "Ready for review",
        "--author-mode",
        "human_with_ai_assist",
        "--remote",
        "mirror",
        "--json",
    ])
    .expect("patchset publish should accept the final contract");
    let Commands::Patchset {
        command: PatchsetCommand::Publish(args),
    } = published.command
    else {
        panic!("expected patchset publish command");
    };
    assert_eq!(args.change, "RCT-1/C-01");
    assert_eq!(args.summary, "Ready for review");
    assert_eq!(
        args.author_mode,
        Some(ConfigAuthorModeArg::HumanWithAiAssist)
    );
    assert_eq!(args.remote.as_deref(), Some("mirror"));
    assert!(args.json);

    let listed = Cli::try_parse_from([
        "ait-cli",
        "patchset",
        "list",
        "RCT-1/C-01",
        "--remote",
        "mirror",
        "--json",
    ])
    .expect("patchset list should accept one positional Change ID");
    let Commands::Patchset {
        command: PatchsetCommand::List(args),
    } = listed.command
    else {
        panic!("expected patchset list command");
    };
    assert_eq!(args.change, "RCT-1/C-01");
    assert_eq!(args.remote.as_deref(), Some("mirror"));
    assert!(args.json);

    for subcommand in ["show", "select", "ci-status", "rerun-ci"] {
        Cli::try_parse_from([
            "ait-cli",
            "patchset",
            subcommand,
            "RCT-1/C-01/P-01",
            "--remote",
            "mirror",
            "--json",
        ])
        .unwrap_or_else(|error| panic!("patchset {subcommand} should parse: {error}"));
    }

    let root_help = Cli::try_parse_from(["ait-cli", "patchset", "--help"])
        .err()
        .expect("patchset --help must render Clap help")
        .to_string();
    for text in [
        "Published Patchsets exist only on remotes",
        "publish",
        "list",
        "show",
        "select",
        "ci-status",
        "rerun-ci",
    ] {
        assert!(root_help.contains(text), "{root_help}");
    }
    assert!(!root_help.contains("ci-smoke"), "{root_help}");

    let publish_help = Cli::try_parse_from(["ait-cli", "patchset", "publish", "--help"])
        .err()
        .expect("patchset publish --help must render Clap help")
        .to_string();
    for text in [
        "<CHANGE_ID>",
        "--summary <SUMMARY>",
        "--author-mode <MODE>",
        "human_only",
        "human_with_ai_assist",
        "ai_with_human_review",
        "ai_only_experimental",
        "--remote <REMOTE>",
        "--json",
    ] {
        assert!(publish_help.contains(text), "{publish_help}");
    }
    assert!(!publish_help.contains("--change"), "{publish_help}");

    let help_cases = [
        ("list", "<CHANGE_ID>", "without modifying"),
        ("show", "<PATCHSET_ID>", "without modifying"),
        ("select", "<PATCHSET_ID>", "owning Change"),
        ("ci-status", "<PATCHSET_ID>", "10 most recent"),
        ("rerun-ci", "<PATCHSET_ID>", "manual_rerun"),
    ];
    for (subcommand, positional, behavior) in help_cases {
        let help = Cli::try_parse_from(["ait-cli", "patchset", subcommand, "--help"])
            .err()
            .expect("Patchset subcommand help must stop parsing")
            .to_string();
        for text in [positional, behavior, "--remote <REMOTE>", "--json"] {
            assert!(help.contains(text), "patchset {subcommand} help:\n{help}");
        }
    }
}

#[test]
fn patchset_parser_rejects_removed_controls_and_ambiguous_ids() {
    let removed_cases: &[&[&str]] = &[
        &[
            "publish",
            "RCT-1/C-01",
            "--summary",
            "x",
            "--change",
            "C-01",
        ],
        &["list", "RCT-1/C-01", "--change", "C-01"],
        &["list", "RCT-1/C-01", "--repo", "other"],
        &["show", "RCT-1/C-01/P-01", "--repo", "other"],
        &["show", "RCT-1/C-01/P-01", "--change", "C-01"],
        &["select", "RCT-1/C-01/P-01", "--change", "C-01"],
        &["ci-status", "RCT-1/C-01/P-01", "--recent-limit", "5"],
        &["rerun-ci", "RCT-1/C-01/P-01", "--trigger", "scheduled"],
        &["show", "RCT-1/C-01/P-01", "--local"],
    ];
    for args in removed_cases {
        let error = Cli::try_parse_from(
            std::iter::once("ait-cli")
                .chain(std::iter::once("patchset"))
                .chain(args.iter().copied()),
        )
        .err()
        .unwrap_or_else(|| panic!("retired Patchset input unexpectedly parsed: {args:?}"));
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "args: {args:?}\n{error}"
        );
    }

    let hidden_command = Cli::try_parse_from(["ait-cli", "patchset", "ci-smoke"])
        .err()
        .expect("retired patchset ci-smoke must be rejected");
    assert_eq!(
        hidden_command.kind(),
        clap::error::ErrorKind::InvalidSubcommand
    );

    for subcommand in ["show", "select", "ci-status", "rerun-ci"] {
        let error = Cli::try_parse_from(["ait-cli", "patchset", subcommand, "1"])
            .err()
            .unwrap_or_else(|| panic!("numeric Patchset ref parsed for {subcommand}"));
        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
    }

    let author_mode = Cli::try_parse_from([
        "ait-cli",
        "patchset",
        "publish",
        "RCT-1/C-01",
        "--summary",
        "x",
        "--author-mode",
        "invented",
    ])
    .err()
    .expect("unknown Patchset author mode must be rejected");
    assert_eq!(author_mode.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn release_build_parser_accepts_explicit_native_input_directories() {
    let command = Cli::try_parse_from([
        "ait-cli",
        "release",
        "build",
        "REL-1",
        "--native-matrix-dir",
        "/tmp/ait-native-matrix",
        "--native-command-dir",
        "/tmp/ait-native-commands",
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
            assert_eq!(
                args.native_command_dir,
                Some(PathBuf::from("/tmp/ait-native-commands"))
            );
            assert!(args.json);
        }
        _ => panic!("expected release build command"),
    }
}

#[test]
fn release_formula_parser_uses_typed_python_formula_argument() {
    let default_command = Cli::try_parse_from([
        "ait-cli",
        "release",
        "formula",
        "REL-1",
        "--name",
        "ait-native",
    ])
    .unwrap()
    .command;
    match default_command {
        Commands::Release {
            command: ReleaseCommand::Formula(args),
        } => assert_eq!(args.python_formula, "python@3"),
        _ => panic!("expected release formula command"),
    }

    let explicit_command = Cli::try_parse_from([
        "ait-cli",
        "release",
        "formula",
        "REL-1",
        "--name",
        "ait-native",
        "--python-formula",
        "python@3.14",
    ])
    .unwrap()
    .command;
    match explicit_command {
        Commands::Release {
            command: ReleaseCommand::Formula(args),
        } => assert_eq!(args.python_formula, "python@3.14"),
        _ => panic!("expected release formula command"),
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
        "--public-source-root",
        "/tmp/public-source",
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
            assert_eq!(
                args.public_source_root,
                Some(PathBuf::from("/tmp/public-source"))
            );
            assert!(args.json);
        }
        _ => panic!("expected family release promote command"),
    }

    let shown = Cli::try_parse_from([
        "ait-cli",
        "release",
        "show",
        "REL-FAM-0123456789ABCDEF",
        "--public-source-root",
        "/tmp/public-source",
        "--json",
    ])
    .unwrap()
    .command;
    match shown {
        Commands::Release {
            command: ReleaseCommand::Show(args),
        } => {
            assert_eq!(args.release_id, "REL-FAM-0123456789ABCDEF");
            assert_eq!(
                args.public_source_root,
                Some(PathBuf::from("/tmp/public-source"))
            );
            assert!(args.remote.is_none());
            assert!(args.json);
        }
        _ => panic!("expected family release show command"),
    }

    let packaged = Cli::try_parse_from([
        "ait-cli",
        "release",
        "package",
        "REL-FAM-0123456789ABCDEF",
        "--channel",
        "npm",
        "--public-source-root",
        "/tmp/public-source",
        "--json",
    ])
    .unwrap()
    .command;
    match packaged {
        Commands::Release {
            command: ReleaseCommand::Package(args),
        } => {
            assert_eq!(args.release_id, "REL-FAM-0123456789ABCDEF");
            assert_eq!(args.channel, "npm");
            assert_eq!(
                args.public_source_root,
                Some(PathBuf::from("/tmp/public-source"))
            );
            assert!(args.json);
        }
        _ => panic!("expected family release package command"),
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

#[test]
fn review_parser_enforces_exact_ai_and_task_review_identity_contracts() {
    let parsed = Cli::try_parse_from([
        "ait-cli",
        "review",
        "code",
        "submit",
        "RCC-1",
        "--patchset",
        "RCP-1",
        "--message",
        "Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: cargo test; Recommendation: pass",
        "--remote",
        "origin",
        "--json",
    ])
    .expect("exact AI review command");
    let Commands::Review {
        command:
            ReviewCommand::Code {
                command: ReviewCodeCommand::Submit(args),
            },
    } = parsed.command
    else {
        panic!("expected review code submit");
    };
    assert_eq!(args.change_id, "RCC-1");
    assert_eq!(args.patchset_id, "RCP-1");
    assert_eq!(args.remote.as_deref(), Some("origin"));
    assert!(args.json);

    let parsed = Cli::try_parse_from([
        "ait-cli",
        "review",
        "task",
        "approve",
        "RCC-1",
        "--patchset",
        "RCP-1",
        "--message",
        "Validated the requested behavior.",
    ])
    .expect("exact human Task review command");
    let Commands::Review {
        command:
            ReviewCommand::Task {
                command: ReviewTaskCommand::Approve(args),
            },
    } = parsed.command
    else {
        panic!("expected review task approve");
    };
    assert_eq!(args.patchset_id, "RCP-1");
    assert_eq!(args.message, "Validated the requested behavior.");

    for argv in [
        vec![
            "ait-cli",
            "review",
            "code",
            "submit",
            "RCC-1",
            "--message",
            "summary",
        ],
        vec![
            "ait-cli",
            "review",
            "code",
            "submit",
            "RCC-1",
            "--patchset",
            "1",
            "--message",
            "summary",
        ],
        vec![
            "ait-cli",
            "review",
            "code",
            "submit",
            "RCC-1",
            "--patchset",
            "RCP-1",
            "--message",
            "summary",
            "--verdict",
            "pass",
        ],
        vec![
            "ait-cli",
            "review",
            "code",
            "submit",
            "RCC-1",
            "--patchset",
            "RCP-1",
            "--message",
            "summary",
            "--reviewer",
            "spoofed-app",
        ],
        vec![
            "ait-cli",
            "review",
            "task",
            "approve",
            "RCC-1",
            "--patchset",
            "1",
            "--message",
            "validated",
        ],
        vec![
            "ait-cli",
            "review",
            "task",
            "approve",
            "RCC-1",
            "--patchset",
            "RCP-1",
        ],
        vec![
            "ait-cli",
            "review",
            "task",
            "approve",
            "RCC-1",
            "--patchset",
            "RCP-1",
            "--message",
            "validated",
            "--reviewer",
            "spoofed-human",
        ],
    ] {
        assert!(
            Cli::try_parse_from(argv.clone()).is_err(),
            "invalid Review surface parsed: {argv:?}"
        );
    }

    for alias in [
        "request",
        "approve",
        "request-changes",
        "comment",
        "defer",
        "code-summary",
    ] {
        assert!(
            Cli::try_parse_from(["ait-cli", "review", alias]).is_err(),
            "removed root Review alias parsed: {alias}"
        );
    }

    assert!(
        Cli::try_parse_from([
            "ait-cli",
            "workflow",
            "finish",
            "RCC-1",
            "--reviewer",
            "spoofed"
        ])
        .is_err(),
        "removed workflow-land reviewer override parsed"
    );

    let workflow_finish = Cli::try_parse_from([
        "ait-cli",
        "workflow",
        "finish",
        "RCC-1",
        "--apply",
        "--review-message",
        "structured exact-Patchset review",
        "--remote",
        "origin",
    ])
    .expect("reviewer-owned workflow finish summary should parse");
    let Commands::Workflow {
        command: WorkflowCommand::Finish(args),
    } = workflow_finish.command
    else {
        panic!("expected workflow finish command");
    };
    assert_eq!(
        args.review_message.as_deref(),
        Some("structured exact-Patchset review")
    );
    assert!(args.apply);
    assert_eq!(args.remote.as_deref(), Some("origin"));

    assert!(
        Cli::try_parse_from([
            "ait-cli",
            "workflow",
            "finish",
            "RCC-1",
            "--review-message",
            "structured exact-Patchset review"
        ])
        .is_err(),
        "workflow-land review mutation parsed without --apply"
    );

    let template =
        Cli::try_parse_from(["ait-cli", "review", "code", "template", "--style", "inline"])
            .expect("typed inline template style");
    let Commands::Review {
        command:
            ReviewCommand::Code {
                command: ReviewCodeCommand::Template(args),
            },
    } = template.command
    else {
        panic!("expected review code template");
    };
    assert_eq!(args.style, ReviewCodeTemplateStyleArg::Inline);
    assert!(
        Cli::try_parse_from(["ait-cli", "review", "code", "template", "--style", "compact",])
            .is_err()
    );
}

fn parse_external_command(args: &[&str]) -> ExternalCommand {
    match Cli::try_parse_from(args).unwrap().command {
        Commands::External { command } => command,
        _ => panic!("expected external command"),
    }
}

#[test]
fn repository_retirement_restore_flags_are_bounded_and_discard_is_retired() {
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
    let request = build_repo_command_request(RepoCommand::Retire(args))
        .expect("build repo retirement abort request");
    assert_eq!(request.args["abort"], json!(true));
    assert_eq!(request.args.len(), 1);

    let retired = Cli::try_parse_from(["ait-cli", "repo", "retire", "--replace-export"])
        .err()
        .expect("retired archive replacement option must be rejected");
    assert_eq!(retired.kind(), clap::error::ErrorKind::UnknownArgument);

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
    .err()
    .expect("retired remote discard option must be rejected");
    assert_eq!(discard.kind(), clap::error::ErrorKind::UnknownArgument);
}

#[test]
fn repo_jobs_parser_separates_exact_lookup_from_bounded_named_filters() {
    let exact = Cli::try_parse_from([
        "ait-cli",
        "repo",
        "jobs",
        "--remote",
        "origin",
        "--worker-job-index",
        "7",
        "--json",
    ])
    .expect("parse exact Worker Job lookup");
    let Commands::Repo {
        command: RepoCommand::Jobs(args),
    } = exact.command
    else {
        panic!("expected repo jobs");
    };
    assert_eq!(args.worker_job_index, Some(7));
    let request =
        build_repo_command_request(RepoCommand::Jobs(args)).expect("build exact Job request");
    assert_eq!(request.args["worker_job_index"], json!(7));
    assert_eq!(request.args.len(), 1);

    let list = Cli::try_parse_from([
        "ait-cli", "repo", "jobs", "--state", "failed", "--limit", "1000",
    ])
    .expect("parse filtered Worker Job list");
    let Commands::Repo {
        command: RepoCommand::Jobs(args),
    } = list.command
    else {
        panic!("expected repo jobs");
    };
    assert_eq!(args.state.as_deref(), Some("failed"));
    assert_eq!(args.limit, 1000);

    for conflicting in [
        vec!["--worker-job-index", "7", "--state", "failed"],
        vec!["--worker-job-index", "7", "--limit", "10"],
    ] {
        let error = Cli::try_parse_from(
            ["ait-cli", "repo", "jobs"]
                .into_iter()
                .chain(conflicting.iter().copied()),
        )
        .err()
        .expect("exact lookup and list filters must conflict");
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    for state in ["1", "2", "3", "4", "canceled"] {
        assert!(
            Cli::try_parse_from(["ait-cli", "repo", "jobs", "--state", state]).is_err(),
            "public state {state:?} must be rejected"
        );
    }
    for limit in ["0", "1001"] {
        assert!(
            Cli::try_parse_from(["ait-cli", "repo", "jobs", "--limit", limit]).is_err(),
            "out-of-range limit {limit:?} must be rejected"
        );
    }
}

#[test]
fn repo_help_explains_every_retained_command_and_option() {
    fn help(command: Option<&str>) -> String {
        let mut args = vec!["ait-cli", "repo"];
        if let Some(command) = command {
            args.push(command);
        }
        args.push("--help");
        Cli::try_parse_from(args)
            .err()
            .expect("--help must render Clap help")
            .to_string()
    }

    let parent = help(None);
    assert!(
        parent.contains("Inspect and manage the configured Repository on its remote server"),
        "{parent}"
    );
    for command in ["show", "retire", "restore", "jobs", "ci-capabilities"] {
        assert!(parent.contains(command), "{parent}");
    }

    let show = help(Some("show"));
    assert!(show.contains("Repository default remote"), "{show}");
    assert!(show.contains("machine-readable JSON"), "{show}");

    let retire = help(Some("retire"));
    assert!(
        retire.contains("download and verify its complete archive"),
        "{retire}"
    );
    assert!(
        retire.contains("ait repo restore --remote <NAME>"),
        "{retire}"
    );
    assert!(
        retire.contains("preserve any complete local archive"),
        "{retire}"
    );
    assert!(!retire.contains("replace-export"), "{retire}");

    let restore = help(Some("restore"));
    assert!(
        restore.contains("create a new remote Repository index"),
        "{restore}"
    );
    assert!(
        restore.contains("there is no name, index, or force override"),
        "{restore}"
    );

    let jobs = help(Some("jobs"));
    assert!(
        jobs.contains("cannot be combined with list filters"),
        "{jobs}"
    );
    assert!(
        jobs.contains("queued, running, succeeded, or failed"),
        "{jobs}"
    );
    assert!(jobs.contains("1 through 1000; default 50"), "{jobs}");

    let capabilities = help(Some("ci-capabilities"));
    assert!(capabilities.contains("native runner"), "{capabilities}");
    assert!(
        capabilities.contains("remote-sync prerequisites"),
        "{capabilities}"
    );
}

#[test]
fn external_command_parser_covers_status_doctor_update_link_and_unlink_modes() {
    match parse_external_command(&["ait-cli", "external", "status", "--json"]) {
        ExternalCommand::Status(args) => assert!(args.json),
        _ => panic!("expected status"),
    }
    match parse_external_command(&["ait-cli", "external", "doctor"]) {
        ExternalCommand::Doctor(args) => {
            assert!(!args.json);
            assert!(!args.fail_on_blocking);
        }
        _ => panic!("expected doctor"),
    }
    match parse_external_command(&[
        "ait-cli",
        "external",
        "doctor",
        "--fail-on-blocking",
        "--json",
    ]) {
        ExternalCommand::Doctor(args) => {
            assert!(args.json);
            assert!(args.fail_on_blocking);
        }
        _ => panic!("expected strict doctor"),
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
fn external_update_parser_rejects_incomplete_and_conflicting_modes() {
    for args in [
        vec!["ait-cli", "external", "update", "ait-db"],
        vec!["ait-cli", "external", "update", "--to", "SNP-DB-NEW"],
        vec!["ait-cli", "external", "update", "--latest"],
        vec![
            "ait-cli",
            "external",
            "update",
            "ait-db",
            "--to",
            "SNP-DB-NEW",
            "--latest",
        ],
        vec![
            "ait-cli",
            "external",
            "update",
            "ait-db",
            "--to",
            "SNP-DB-NEW",
            "--locked",
        ],
        vec!["ait-cli", "external", "update", "ait-db", "--locked"],
    ] {
        let error = Cli::try_parse_from(&args)
            .err()
            .unwrap_or_else(|| panic!("invalid external update must fail in Clap: {args:?}"));
        assert!(
            matches!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument
                    | clap::error::ErrorKind::ArgumentConflict
            ),
            "{args:?}: {error}"
        );
    }
}

#[test]
fn external_help_explains_modes_safety_and_machine_output() {
    fn help(command: Option<&str>) -> String {
        let mut args = vec!["ait-cli", "external"];
        if let Some(command) = command {
            args.push(command);
        }
        args.push("--help");
        Cli::try_parse_from(args)
            .err()
            .expect("--help must render Clap help")
            .to_string()
    }

    let parent = help(None);
    assert!(
        parent.contains("Inspect, diagnose, resolve, pin, restore, and locally link"),
        "{parent}"
    );
    for command in ["update", "status", "doctor", "link", "unlink"] {
        assert!(parent.contains(command), "{parent}");
    }

    let update = help(Some("update"));
    for description in [
        "Unique direct external name",
        "exact immutable Snapshot",
        "declared remote and line head",
        "drift-free ait-external.lock",
        "Prepare the selected files",
        "lockfile still records the complete dependency graph",
        "machine-readable update report",
    ] {
        assert!(update.contains(description), "{description:?}: {update}");
    }

    let doctor = help(Some("doctor"));
    assert!(doctor.contains("--fail-on-blocking"), "{doctor}");
    assert!(doctor.contains("exit code 2"), "{doctor}");

    let link = help(Some("link"));
    assert!(link.contains("Unique direct external name"), "{link}");
    assert!(link.contains("existing directory"), "{link}");

    let unlink = help(Some("unlink"));
    assert!(unlink.contains("restore"), "{unlink}");
}
