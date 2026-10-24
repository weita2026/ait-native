#![cfg(unix)]
#[path = "support/agent_teams_cli.rs"]
mod fixture;
use ait_cli::runtime::RepoRuntime;
use ait_core::agent_binary_db::single_team;
use ait_core::binary_db::BinaryDb;
use fixture::*;
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
        assert_eq!(reenabled["team_bootstrap"]["status"], "ready");
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
