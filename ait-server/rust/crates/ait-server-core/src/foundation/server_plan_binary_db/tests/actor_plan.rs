use super::*;
use crate::foundation::workflow_binary_v0_adapter::{
    BinaryDbServerWorkflowV0Store, UserActorIdentity,
};

#[test]
fn actor_plan_retries_and_noops_preserve_immutable_author() {
    let service = service("actor-plan-replay");
    let first = service
        .create_plan("repo-bin", &create_payload("Actor Plan", "actor/item"))
        .unwrap();
    assert_eq!(first["actor_key_index"], 0);
    assert_eq!(first["head_revision"]["actor_key_index"], 0);
    let actor = BinaryDbServerWorkflowV0Store::new_frozen(service.db().clone())
        .resolve_user_actor(&UserActorIdentity::new("Other", "other@example.test").unwrap())
        .unwrap();
    assert_eq!(actor, 1);
    let mut retry = create_payload("Actor Plan", "actor/item");
    retry["actor_key_index"] = json!(actor);
    let before = authority_data_files(&service);
    assert_eq!(
        service.create_plan("repo-bin", &retry).unwrap()["plan_id"],
        first["plan_id"]
    );
    assert_eq!(authority_data_files(&service), before);
    retry["title"] = json!("A real revision");
    retry["expected_head_revision_id"] = first["head_revision_id"].clone();
    let revised = service.revise_plan("PR-0", &retry).unwrap();
    assert_ne!(revised["head_revision_id"], first["head_revision_id"]);
    assert_eq!(revised["actor_key_index"], 1);
    assert_eq!(revised["head_revision"]["actor_key_index"], 1);
    retry["actor_key_index"] = json!(0);
    let before = authority_data_files(&service);
    assert_eq!(
        service.revise_plan("PR-0", &retry).unwrap()["head_revision_id"],
        revised["head_revision_id"]
    );
    assert_eq!(authority_data_files(&service), before);
    assert_eq!(
        service
            .get_plan_revision("PR-0", "plan-revision:0")
            .unwrap()["actor_key_index"],
        0
    );
    assert_eq!(
        service
            .update_plan_status("PR-0", &json!({"actor_key_index":0,"status":"draft"}))
            .unwrap()["actor_key_index"],
        1
    );
    assert_eq!(authority_data_files(&service), before);
    let archived = service
        .update_plan_status("PR-0", &json!({"actor_key_index":0,"status":"archived"}))
        .unwrap();
    assert_eq!(archived["actor_key_index"], 0);
    assert_eq!(archived["head_revision"]["actor_key_index"], 1);
    service.validate_metadata().unwrap();
}

#[test]
fn actor_plan_invalid_keys_and_stale_conflicting_replay_have_no_effects() {
    let service = service("actor-plan-invalid");
    let first = service
        .create_plan("repo-bin", &create_payload("Initial", "actor/item"))
        .unwrap();
    let mut revised = create_payload("Revised", "actor/item");
    revised["expected_head_revision_id"] = first["head_revision_id"].clone();
    service.revise_plan("PR-0", &revised).unwrap();
    let before = authority_data_files(&service);
    revised["title"] = json!("Conflicting retry");
    assert!(service
        .revise_plan("PR-0", &revised)
        .unwrap_err()
        .contains("head advanced"));
    for actor in [
        JsonValue::Null,
        json!(-1),
        json!("0"),
        json!(1),
        json!(u32::MAX),
    ] {
        let mut invalid = create_payload("Invalid", "actor/item");
        invalid["actor_key_index"] = actor;
        assert!(service.create_plan("repo-bin", &invalid).is_err());
        assert!(service.revise_plan("PR-0", &invalid).is_err());
        assert!(service.update_plan_status("PR-0", &invalid).is_err());
        assert!(service
            .put_plan_revision_artifacts("PR-0", "plan-revision:0", &invalid)
            .is_err());
        assert_eq!(authority_data_files(&service), before);
    }
}

#[test]
fn actor_plan_metadata_validation_rejects_unknown_author_reference() {
    let service = service("actor-plan-closure");
    service
        .create_plan("repo-bin", &create_payload("Actor Plan", "actor/item"))
        .unwrap();
    service.validate_metadata().unwrap();
    let mut record = service.store().read_plan_record(0).unwrap();
    record.actor_index_plus1 = 2;
    let mut tx = BinaryDbWriteTxn::begin_serving(
        service.db(),
        crate::foundation::remote_binary_db::BinaryDbCommandScope::ServerTaskStart,
    )
    .unwrap();
    tx.overwrite_record(
        plan_file(),
        0,
        &super::super::codec::ServerPlanCodec::<2>::encode_record(&record).unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(service
        .validate_metadata()
        .unwrap_err()
        .contains("unknown Actor"));
}
