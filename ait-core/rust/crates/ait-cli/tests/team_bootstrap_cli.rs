#![cfg(unix)]
#[path = "support/agent_teams_cli.rs"]
mod fixture;
use ait_cli::runtime::RepoRuntime;
use ait_core::agent_binary_db::single_team;
use ait_core::binary_db::BinaryDb;
use fixture::*;

fn retained_state(temp: &tempfile::TempDir) -> Vec<Option<Vec<u8>>> {
    [
        ".ait/binary-db/agents.bin",
        ".ait/binary-db/agents_payload.bin",
        ".ait/binary-db/task.bin",
        ".ait/binary-db/task_payload.bin",
        ".ait/binary-db/line.bin",
        ".ait/team-session-bootstrap/agent-0.activation",
        ".ait/team-session-bootstrap/agent-0.role-loaded",
        ".ait/unrelated-control",
    ]
    .map(|path| std::fs::read(temp.path().join(path)).ok())
    .into()
}

fn bind_developer(temp: &tempfile::TempDir) -> RepoRuntime {
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let db = repo.binary_db_stores::<1>().local_db();
    single_team::ensure_pool(&db).unwrap();
    let agents = single_team::read(&db).unwrap();
    single_team::assign(&db, 2, &agents[2], 1, "developer").unwrap();
    repo
}

fn preserve_from_developer(
    temp: &tempfile::TempDir,
    args: &[&str],
) -> ait_core::json_support::JsonValue {
    let output = command(temp)
        .env("CODEX_THREAD_ID", "developer")
        .env("CODEX_SESSION_ID", "developer")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = ait_core::json_support::JsonCodec::parse_slice_with_error_prefix(
        &output.stdout,
        "preserved CLI",
    )
    .unwrap();
    let team = if args[0] == "init" {
        &payload["agent_team_bootstrap"]
    } else {
        &payload["team_bootstrap"]
    };
    assert_eq!(team["status"], "preserved");
    assert_eq!(team["session_readiness"], "unchanged");
    assert_eq!(team["work_started"], false);
    payload
}

#[test]
fn init_and_config_preserve_owner_developer_and_workflow_without_host_access() {
    use ait_core::{line_store::LineStore, local_snapshot::LocalSnapshotWriteStore};
    let temp = fixture();
    let repo = bind_developer(&temp);
    ait_core::task_workflow_store::create_task_with_task_workflow_task_store(
        &repo.task_store().unwrap(),
        &repo.repo_name(),
        "Retained Task",
        "Preserve unrelated work",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let lines = repo.binary_db_stores::<1>().lines();
    if lines.line_by_name("main").unwrap().is_none() {
        lines
            .create_line("main", None, "2026-11-01T00:00:00Z")
            .unwrap();
    }
    repo.local_snapshot_operation_store::<1>(&repo.workspace_root())
        .unwrap()
        .create_snapshot(&repo.repo_name(), "main", Some("retained head"), false)
        .unwrap();
    assert!(lines
        .line_by_name("main")
        .unwrap()
        .unwrap()
        .head_snapshot_id
        .is_some());
    std::fs::write(
        temp.path().join(".ait/unrelated-control"),
        b"retained control",
    )
    .unwrap();
    let before = retained_state(&temp);
    assert!(before[2].is_some());
    let rpc = std::fs::read(temp.path().join("rpc.log")).unwrap();
    for state in ["healthy", "archived", "missing", "denied", "system-error"] {
        let marker = temp.path().join(format!("fake-1.{state}"));
        std::fs::write(&marker, "").unwrap();
        for args in [
            vec!["init", "--json"],
            vec!["config", "set", "--agent-teams", "enabled", "--json"],
        ] {
            preserve_from_developer(&temp, &args);
            assert_eq!(retained_state(&temp), before, "{state}");
            assert_eq!(
                std::fs::read(temp.path().join("rpc.log")).unwrap(),
                rpc,
                "host contacted for {state}"
            );
        }
        std::fs::remove_file(marker).unwrap();
    }
    for args in [
        vec!["init"],
        vec!["config", "set", "--agent-teams", "enabled"],
    ] {
        let output = command(&temp)
            .env("CODEX_THREAD_ID", "developer")
            .env("CODEX_SESSION_ID", "developer")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text
            .to_lowercase()
            .contains("host readiness was not checked"));
        assert!(!text.contains("Resume: codex"));
    }
    run(
        &temp,
        &["config", "set", "--agent-teams", "disabled", "--json"],
        true,
    );
    preserve_from_developer(
        &temp,
        &["config", "set", "--agent-teams", "enabled", "--json"],
    );
    assert_eq!(retained_state(&temp), before);
    assert_eq!(std::fs::read(temp.path().join("rpc.log")).unwrap(), rpc);
}

#[test]
fn unfinished_native_creation_is_preserved_until_explicit_ensure_recovery() {
    let temp = uninitialized_fixture();
    std::fs::write(temp.path().join("fail-prepare"), "").unwrap();
    let initial = run(&temp, &["init", "--json"], true);
    assert_eq!(initial["agent_team_bootstrap"]["status"], "pending");
    let operational = temp.path().join(".ait/team-session-bootstrap");
    let candidate = std::fs::read(operational.join("agent-0.pending")).unwrap();
    let assignment = std::fs::read(operational.join("agent-0.assignment")).unwrap();
    let rpc = std::fs::read(temp.path().join("rpc.log")).unwrap();
    for args in [
        vec!["init", "--json"],
        vec!["config", "set", "--agent-teams", "enabled", "--json"],
    ] {
        let result = run(&temp, &args, true);
        let team = if args[0] == "init" {
            &result["agent_team_bootstrap"]
        } else {
            &result["team_bootstrap"]
        };
        assert_eq!(team["status"], "preserved");
        assert_eq!(team["session_readiness"], "pending");
        assert_eq!(
            std::fs::read(operational.join("agent-0.pending")).unwrap(),
            candidate
        );
        assert_eq!(
            std::fs::read(operational.join("agent-0.assignment")).unwrap(),
            assignment
        );
        assert_eq!(std::fs::read(temp.path().join("rpc.log")).unwrap(), rpc);
    }
    // Only explicit recovery may replace the candidate after native absence proof.
    std::fs::write(temp.path().join("fake-1.missing"), "").unwrap();
    let recovered = run(
        &temp,
        &["team", "session", "ensure", "--agent", "0", "--json"],
        true,
    );
    assert_eq!(recovered["status"], "ready");
    assert_eq!(recovered["session_id"], "fake-2");
    assert_eq!(
        std::fs::read_to_string(temp.path().join("count"))
            .unwrap()
            .trim(),
        "2"
    );
}

#[test]
fn init_and_enable_preserve_a_developer_binding_without_an_owner() {
    let temp = uninitialized_fixture();
    ait_cli::init_surface::init_repo(&ait_cli::init_surface::InitRequest {
        root: temp.path().into(),
        name: None,
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    bind_developer(&temp);
    let before = retained_state(&temp);
    preserve_from_developer(&temp, &["init", "--json"]);
    preserve_from_developer(
        &temp,
        &["config", "set", "--agent-teams", "enabled", "--json"],
    );
    assert_eq!(retained_state(&temp), before);
    assert!(!temp.path().join("rpc.log").exists());
}

#[test]
fn new_init_four_seats_and_repeated_enable_preserve_newton() {
    let temp = fixture();
    let shared = std::fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
    let unrelated_guidance =
        "\n## Repository notes\nPreserve this repository's validation instructions.\n";
    std::fs::write(
        temp.path().join("AGENTS.md"),
        format!("{shared}{unrelated_guidance}"),
    )
    .unwrap();
    let assert_state = |teams_enabled: bool, effective_author_mode: &str| {
        let config = run(&temp, &["config", "show", "--json"], true);
        assert_eq!(config["agent_teams"], teams_enabled);
        assert_eq!(config["default_author_mode"], "ai_with_human_review");
        assert_eq!(config["effective_author_mode"], effective_author_mode);
        let guidance = std::fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(guidance.contains("scope=`local`"));
        assert!(guidance.contains(&format!("author-mode=`{effective_author_mode}`")));
        assert_eq!(guidance.contains("Fresh team candidates"), teams_enabled);
        for rule in [
            "With teams disabled, the current executor rebases",
            "With teams enabled, Developer does this work and Reviewer",
            "need no PM relay or approval.",
        ] {
            assert!(
                guidance.contains(rule),
                "missing shared rebase rule: {rule}"
            );
        }
        assert_eq!(
            guidance
                .matches("Rebase as needed when main or the target Line advances")
                .count(),
            1
        );
        assert!(!guidance.contains("Newton"));
        assert!(!guidance.contains("<scope-args>"));
        assert!(guidance.ends_with(unrelated_guidance));
    };
    assert_state(true, "ai_only_experimental");
    let contacts = run(&temp, &["team", "contacts", "--json"], true);
    assert_eq!(contacts["actual_member_count"], 4);
    assert_eq!(contacts["bound_session_count"], 1);
    let newton = contacts["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["agent_index"] == 0)
        .unwrap();
    assert_eq!(newton["name"], "Newton");
    assert_eq!(newton["session_id"], "fake-1");
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let db = repo.binary_db_stores::<1>().local_db();
    assert_eq!(single_team::read(&db).unwrap().len(), 50);
    for path in [
        "agent_roles.bin",
        "agent_teams.bin",
        "agent_team_membership.bin",
    ] {
        assert!(!db.authority_root().as_path().join(path).exists());
    }
    run(
        &temp,
        &["config", "set", "--agent-teams", "disabled", "--json"],
        true,
    );
    assert_state(false, "ai_with_human_review");
    let disabled_rpc = std::fs::read(temp.path().join("rpc.log")).unwrap();
    let existing = run(&temp, &["init", "--json"], true);
    assert_eq!(existing["agent_team_bootstrap"]["status"], "disabled");
    assert_eq!(
        existing["agent_team_bootstrap"]["synchronization"]["status"],
        "ready"
    );
    assert_state(false, "ai_with_human_review");
    assert_eq!(
        std::fs::read(temp.path().join("rpc.log")).unwrap(),
        disabled_rpc
    );
    for _ in 0..2 {
        let reenabled = run(
            &temp,
            &["config", "set", "--agent-teams", "enabled", "--json"],
            true,
        );
        assert_eq!(reenabled["team_bootstrap"]["status"], "preserved");
        assert_eq!(
            reenabled["team_bootstrap"]["session_id"],
            newton["session_id"]
        );
        assert_state(true, "ai_only_experimental");
        let contacts = run(&temp, &["team", "contacts", "--json"], true);
        assert_eq!(contacts["actual_member_count"], 4);
        assert_eq!(contacts["bound_session_count"], 1);
        let current_newton = contacts["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|member| member["agent_index"] == 0)
            .unwrap();
        for field in ["name", "role_index", "session_id"] {
            assert_eq!(current_newton[field], newton[field]);
        }
    }
    assert_eq!(
        std::fs::read_to_string(temp.path().join("count"))
            .unwrap()
            .trim(),
        "1"
    );
    for option in ["--workflow-mode", "--agent-team-size"] {
        assert!(!command(&temp)
            .args(["config", "set", option, "4"])
            .output()
            .unwrap()
            .status
            .success());
    }
}
