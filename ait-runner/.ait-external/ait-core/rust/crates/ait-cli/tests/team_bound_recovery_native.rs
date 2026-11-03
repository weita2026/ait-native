//! Explicit native verification uses an isolated home, no model turns or user-session deletion.
#![cfg(unix)]
#[path = "support/native_agent_teams.rs"]
mod native;
use native::Fixture;
use std::fs;
#[test]
#[ignore = "explicit isolated native recovery verification"]
fn known_absent_candidate_recovers_once_and_valid_binding_is_protected() {
    let f = Fixture::new();
    let op = f.repo.join(".ait/team-session-bootstrap");
    let scope = fs::read(op.join("agent-0.host")).unwrap();
    fs::write(op.join("agent-3.host"), scope).unwrap();
    let absent = "00000000-0000-4000-8000-000000000000";
    fs::write(op.join("agent-3.pending"), absent).unwrap();
    let created = f.run(
        &[
            "team", "session", "ensure", "--agent", "3", "--role", "1", "--json",
        ],
        true,
    );
    let id = created["session_id"].as_str().unwrap();
    assert_ne!(id, absent);
    let repeat = f.run(
        &["team", "session", "ensure", "--agent", "3", "--json"],
        true,
    );
    assert_eq!(repeat["session_id"], id);
    assert_eq!(
        f.run(
            &["team", "session", "ensure", "--agent", "3", "--json"],
            true
        )["session_id"],
        id
    );
}

#[test]
#[ignore = "explicit isolated native recovery verification"]
fn bound_known_absent_session_recovers_with_plain_ensure() {
    let f = Fixture::new();
    let absent = "00000000-0000-4000-8000-000000000000";
    let repo = ait_cli::runtime::RepoRuntime::discover_from_path(&f.repo).unwrap();
    let db = repo.binary_db_stores::<1>().local_db();
    let agents = ait_core::agent_binary_db::single_team::read(&db).unwrap();
    ait_core::agent_binary_db::single_team::assign(&db, 3, &agents[3], 1, absent).unwrap();
    let op = f.repo.join(".ait/team-session-bootstrap");
    fs::copy(op.join("agent-0.host"), op.join("agent-3.host")).unwrap();
    let recovered = f.run(
        &["team", "session", "ensure", "--agent", "3", "--json"],
        true,
    );
    assert_eq!(recovered["action"], "recovered");
    let id = recovered["session_id"].as_str().unwrap();
    assert_ne!(id, absent);
    assert_eq!(
        f.run(
            &["team", "session", "ensure", "--agent", "3", "--json"],
            true
        )["session_id"],
        id
    );
    assert_eq!(
        ait_core::agent_binary_db::single_team::read(&db).unwrap()[3].session,
        id
    );
}
