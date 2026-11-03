use super::*;
use std::cell::RefCell;

struct Fixture(RefCell<Result<CoordinatorContact, ContactUnavailable>>);
impl CoordinatorSource for Fixture {
    fn resolve(&self, query: &ContactQuery) -> Result<CoordinatorContact, ContactUnavailable> {
        assert_eq!(query.task_id, "T-1");
        assert_eq!(query.repository_root, Path::new("/repo"));
        self.0.borrow().clone()
    }
}
fn contact(session: &str) -> CoordinatorContact {
    CoordinatorContact {
        authority: "repo:7".into(),
        team_index: Some(0),
        agent_index: 0,
        name: "Newton".into(),
        session_id: session.into(),
    }
}
fn receipt() -> JsonValue {
    json!({"apply_status":"done", "closeout_status":"complete", "task_id":"T-1",
        "landed_snapshot_id":"SNP-1", "patchset_id":"T-1/P-01",
        "plan_checklist_closeout":{"plan_item_ref":"ship", "artifact_path":"docs/sprints/test.md"}})
}
#[test]
fn handoff_reads_current_binding_and_preserves_exact_receipts() {
    let source = Fixture(RefCell::new(Ok(contact("session-a"))));
    for (scope, session) in [
        (FinishScope::Local, "session-a"),
        (FinishScope::Remote, "session-b"),
    ] {
        *source.0.borrow_mut() = Ok(contact(session));
        let mut payload = receipt();
        let original = payload.clone();
        attach_finish_handoff(true, Path::new("/repo"), scope, &mut payload, &source);
        assert_eq!(
            payload["team_handoff"]["coordinator"]["session_id"],
            json!(session)
        );
        assert_eq!(payload["team_handoff"]["scope"], json!(scope.label()));
        assert_eq!(payload["team_handoff"]["plan_item_ref"], json!("ship"));
        assert_eq!(payload["team_handoff"]["notification_sent"], json!(false));
        let text = append_handoff_text("original receipt".into(), &payload);
        assert!(text.contains("Notify Coordinator: Newton"));
        assert!(text.contains(session));
        assert!(text.contains("Sprint ref: ship"));
        payload.as_object_mut().unwrap().remove("team_handoff");
        assert_eq!(payload, original);
    }
}
#[test]
fn handoff_partial_and_missing_contacts_never_claim_delivery() {
    for error in [
        ContactUnavailable::SourceUnavailable,
        ContactUnavailable::MissingBinding,
        ContactUnavailable::AmbiguousBinding,
    ] {
        let source = Fixture(RefCell::new(Err(error)));
        let mut payload = receipt();
        payload["closeout_status"] = json!("partial");
        payload["closeout_recovery"] = json!({"command":"retry original finish"});
        attach_finish_handoff(
            true,
            Path::new("/repo"),
            FinishScope::Remote,
            &mut payload,
            &source,
        );
        assert_eq!(payload["team_handoff"]["delivery_state"], json!("recovery"));
        assert_eq!(
            payload["team_handoff"]["recipient_resolution"],
            json!("blocked")
        );
        assert!(payload["team_handoff"].get("coordinator").is_none());
        assert_eq!(
            payload["closeout_recovery"]["command"],
            json!("retry original finish")
        );
        let text = append_handoff_text("partial".into(), &payload);
        assert!(text.contains("delivery is not complete"));
        assert!(!text.contains("Notify Coordinator:"));
    }
}
#[test]
fn handoff_never_resolves_for_solo_or_noncompleted_work() {
    struct MustNotRead;
    impl CoordinatorSource for MustNotRead {
        fn resolve(&self, _: &ContactQuery) -> Result<CoordinatorContact, ContactUnavailable> {
            panic!("unexpected lookup")
        }
    }
    let mut solo = receipt();
    let original = solo.clone();
    attach_finish_handoff(
        false,
        Path::new("/repo"),
        FinishScope::Local,
        &mut solo,
        &MustNotRead,
    );
    assert_eq!(solo, original);
    assert_eq!(append_handoff_text("solo".into(), &solo), "solo");
    for state in ["preview", "pending", "change_landed_task_active", "failed"] {
        let mut payload = receipt();
        payload["closeout_status"] = json!(state);
        attach_finish_handoff(
            true,
            Path::new("/repo"),
            FinishScope::Local,
            &mut payload,
            &MustNotRead,
        );
        assert!(payload.get("team_handoff").is_none());
    }
    let mut payload = receipt();
    payload["apply_status"] = json!("pending");
    attach_finish_handoff(
        true,
        Path::new("/repo"),
        FinishScope::Remote,
        &mut payload,
        &MustNotRead,
    );
    assert!(payload.get("team_handoff").is_none());
}
#[test]
fn handoff_rejects_empty_session_and_production_source_is_explicit() {
    for source in [
        Fixture(RefCell::new(Ok(contact("")))),
        Fixture(RefCell::new(Err(ContactUnavailable::SourceUnavailable))),
    ] {
        let mut payload = receipt();
        attach_finish_handoff(
            true,
            Path::new("/repo"),
            FinishScope::Local,
            &mut payload,
            &source,
        );
        assert_eq!(
            payload["team_handoff"]["recipient_resolution"],
            json!("blocked")
        );
        assert_eq!(payload["closeout_status"], json!("complete"));
    }
    let mut payload = receipt();
    attach_finish_handoff(
        true,
        Path::new("/repo"),
        FinishScope::Local,
        &mut payload,
        &UnavailableCoordinatorSource,
    );
    assert_eq!(
        payload["team_handoff"]["reason"],
        json!("source_unavailable")
    );
}
