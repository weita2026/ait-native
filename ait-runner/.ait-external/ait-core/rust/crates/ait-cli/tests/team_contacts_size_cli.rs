//! Planned seats plus committed extra bindings; no configurable team count.
#![cfg(unix)]
#[path = "support/agent_teams_cli.rs"]
mod fixture;
use fixture::*;
use std::fs;
#[test]
fn contacts_expand_only_after_committed_binding_and_show_pending_reviewer() {
    let t = fixture();
    let before = run(&t, &["team", "contacts", "--json"], true);
    assert_eq!(before["actual_member_count"], 4);
    let reviewer = before["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["agent_index"] == 1)
        .unwrap();
    assert_eq!(reviewer["role_state"], "pending_server_registration");
    run(
        &t,
        &[
            "team", "session", "ensure", "--agent", "3", "--role", "1", "--json",
        ],
        true,
    );
    assert_eq!(
        run(&t, &["team", "contacts", "--json"], true)["actual_member_count"],
        5
    );
    run(
        &t,
        &["team", "session", "ensure", "--agent", "1", "--json"],
        false,
    );
}
#[test]
fn disabled_contacts_do_not_inspect_native_host_or_agent_store() {
    let t = fixture();
    run(
        &t,
        &["config", "set", "--agent-teams", "disabled", "--json"],
        true,
    );
    let log = fs::read(t.path().join("rpc.log")).unwrap();
    fs::write(t.path().join(".ait/binary-db/agents.bin"), "corrupt").unwrap();
    let contacts = run(&t, &["team", "contacts", "--json"], true);
    assert_eq!(contacts["team_enabled"], false);
    assert!(contacts["members"].as_array().unwrap().is_empty());
    assert_eq!(fs::read(t.path().join("rpc.log")).unwrap(), log);
}
