#![cfg(unix)]
#[path = "support/agent_teams_cli.rs"]
mod fixture;
use ait_cli::{agent_teams::CoordinatorSource, runtime::RepoRuntime, team_remote::*};
use ait_core::{agent_binary_db::single_team, json_support::json};
use fixture::*;
#[test]
fn finish_notifications_share_fresh_local_coordinator_resolution() {
    let t = fixture();
    let repo = RepoRuntime::discover_from_path(t.path()).unwrap();
    let db = repo.binary_db_stores::<1>().local_db();
    let old = single_team::read(&db).unwrap();
    single_team::assign(&db, 0, &old[0], 0, "newest-session").unwrap();
    for scope in [FinishScope::Local, FinishScope::Remote] {
        let mut v =
            json!({"task_id":"T-1","apply_status":"done","closeout_status":"complete_unbound"});
        let original = v.clone();
        attach_finish_handoff(
            true,
            &repo.authoritative_repo_root(),
            scope,
            &mut v,
            &CoordinatorSource,
        );
        assert_eq!(
            v["team_handoff"]["coordinator"]["session_id"],
            "newest-session"
        );
        v.as_object_mut().unwrap().remove("team_handoff");
        assert_eq!(v, original);
    }
}
#[test]
fn disabled_teams_do_not_add_finish_handoff() {
    let t = fixture();
    run(
        &t,
        &["config", "set", "--agent-teams", "disabled", "--json"],
        true,
    );
    let repo = RepoRuntime::discover_from_path(t.path()).unwrap();
    let mut v = json!({"task_id":"T-1","apply_status":"done","closeout_status":"complete_unbound"});
    attach_finish_handoff(
        false,
        &repo.authoritative_repo_root(),
        FinishScope::Local,
        &mut v,
        &CoordinatorSource,
    );
    assert!(v.get("team_handoff").is_none());
}
