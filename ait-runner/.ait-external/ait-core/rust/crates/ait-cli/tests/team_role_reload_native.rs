//! Explicit native verification uses an isolated home and never starts model work.
#![cfg(unix)]
#[path = "support/native_agent_teams.rs"]
mod native;
use native::Fixture;
use std::fs;
#[test]
#[ignore = "explicit isolated native role reload verification"]
fn same_role_reload_preserves_all_bound_session_ids_and_retries_same_revision() {
    let f = Fixture::new();
    let first = f.run(
        &["team", "session", "ensure", "--agent", "2", "--json"],
        true,
    );
    let second = f.run(
        &[
            "team", "session", "ensure", "--agent", "3", "--role", "1", "--json",
        ],
        true,
    );
    let file = f.repo.join("docs/roles/1_Developer.md");
    fs::write(
        &file,
        fs::read_to_string(&file).unwrap() + "\nNative role update verification.\n",
    )
    .unwrap();
    let sync = f.run(
        &[
            "plan",
            "sync",
            "docs/roles/1_Developer.md",
            "--local",
            "--json",
        ],
        true,
    );
    assert_eq!(sync["role_reload"]["status"], "ready");
    assert_eq!(sync["role_reload"]["roles"].as_array().unwrap().len(), 2);
    let again = f.run(
        &[
            "plan",
            "sync",
            "docs/roles/1_Developer.md",
            "--local",
            "--json",
        ],
        true,
    );
    assert_eq!(
        again["results"][0]["plan_revision_id"],
        sync["results"][0]["plan_revision_id"]
    );
    assert_eq!(
        f.run(
            &["team", "session", "ensure", "--agent", "2", "--json"],
            true
        )["session_id"],
        first["session_id"]
    );
    assert_eq!(
        f.run(
            &["team", "session", "ensure", "--agent", "3", "--json"],
            true
        )["session_id"],
        second["session_id"]
    );
}
