use super::*;
use crate::foundation::workflow_binary_v0_adapter::UserActorIdentity;

#[test]
fn actor_registration_reuses_exact_identity_and_preserves_collisions() {
    let db = empty_initialized_db("actor-registration-exact");
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());
    let alice = UserActorIdentity::new(" Alice ", " alice@example.test ").unwrap();
    assert_eq!(store.resolve_user_actor(&alice).unwrap(), 0);
    assert_eq!(store.resolve_user_actor(&alice).unwrap(), 0);
    assert_eq!(
        store
            .resolve_user_actor(&UserActorIdentity::new("Alice", "other@example.test").unwrap())
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .resolve_user_actor(&UserActorIdentity::new("alice", "alice@example.test").unwrap())
            .unwrap(),
        2
    );
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in b"Collision candidate" {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    tx.append_index_candidate(
        WorkflowBinaryV0Codec::actor_lookup_index(),
        &hash.to_le_bytes(),
        0,
    )
    .unwrap();
    tx.commit().unwrap();
    let collision = UserActorIdentity::new("Collision candidate", "alice@example.test").unwrap();
    assert_eq!(store.resolve_user_actor(&collision).unwrap(), 3);
    assert_eq!(store.resolve_user_actor(&collision).unwrap(), 3);
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::actor_file())
            .unwrap(),
        4
    );
    let actor = WorkflowBinaryV0Codec::decode_actor(
        &read
            .read_record(WorkflowBinaryV0Codec::actor_file(), 0)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(actor.actor_meta & 7, 0);
    assert_eq!(actor.actor_meta & 0x38, 1 << 4);
    let payload = WorkflowBinaryV0Codec::decode_actor_payload(
        &read
            .read_payload(
                WorkflowBinaryV0Codec::actor_payload_file(),
                actor.payload_offset,
                u32::from(actor.payload_len),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload.user_name, "Alice");
    assert_eq!(payload.email, "alice@example.test");
    assert!(payload.user_id.is_empty() && payload.memo.is_empty());
}

#[test]
fn actor_registration_concurrent_exact_repeats_allocate_one_key() {
    let db = empty_initialized_db("actor-registration-concurrent");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let threads = (0..4)
        .map(|_| {
            let db = db.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = BinaryDbServerWorkflowV0Store::new_frozen(db);
                let identity = UserActorIdentity::new("Concurrent", "same@example.test").unwrap();
                barrier.wait();
                store.resolve_user_actor(&identity).unwrap()
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), 0);
    }
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::actor_file())
            .unwrap(),
        1
    );
    drop(read);
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn actor_registration_bounds_and_repository_local_request_validation() {
    let db = empty_initialized_db("actor-registration-bounds");
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());
    for (name, email) in [
        (" ".to_string(), "a".to_string()),
        ("a".to_string(), " ".to_string()),
        ("é".repeat(128), "a".to_string()),
        ("a".to_string(), "e".repeat(256)),
    ] {
        assert!(UserActorIdentity::new(&name, &email).is_err());
    }
    let identity = UserActorIdentity::new(&"n".repeat(255), &"e".repeat(255)).unwrap();
    assert_eq!(store.resolve_user_actor(&identity).unwrap(), 0);
    assert_eq!(
        store
            .request_actor(json!({"actor_key_index":0}).as_object().unwrap())
            .unwrap(),
        1
    );
    for value in [
        JsonValue::Null,
        json!("0"),
        json!(-1),
        json!(1),
        json!(u32::MAX),
        json!(u64::MAX),
    ] {
        assert!(store
            .request_actor(json!({"actor_key_index":value}).as_object().unwrap())
            .is_err());
    }
    assert!(store.request_actor(json!({}).as_object().unwrap()).is_err());
    let other =
        BinaryDbServerWorkflowV0Store::new_frozen(empty_initialized_db("actor-registration-other"));
    assert!(other
        .request_actor(json!({"actor_key_index":0}).as_object().unwrap())
        .is_err());
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    let mut actor = WorkflowBinaryV0Codec::decode_actor(
        &tx.read_record(WorkflowBinaryV0Codec::actor_file(), 0)
            .unwrap(),
    )
    .unwrap();
    actor.actor_meta |= 0x80;
    tx.overwrite_record(
        WorkflowBinaryV0Codec::actor_file(),
        0,
        &WorkflowBinaryV0Codec::encode_actor(actor).unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(store
        .request_actor(json!({"actor_key_index":0}).as_object().unwrap())
        .is_err());
    let tx = BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerTaskStart).unwrap();
    assert!(store
        .request_actor_in_write(&tx, json!({"actor_key_index":0}).as_object().unwrap())
        .is_err());
}

#[test]
fn actor_registration_resolves_live_identity_without_reusing_tombstones() {
    let db = empty_initialized_db("actor-registration-tombstone");
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());
    let identity = UserActorIdentity::new("Alice", "alice@example.test").unwrap();
    assert_eq!(store.resolve_user_actor(&identity).unwrap(), 0);
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    let mut record = WorkflowBinaryV0Codec::decode_actor(
        &tx.read_record(WorkflowBinaryV0Codec::actor_file(), 0)
            .unwrap(),
    )
    .unwrap();
    record.actor_meta |= 0x80;
    tx.overwrite_record(
        WorkflowBinaryV0Codec::actor_file(),
        0,
        &WorkflowBinaryV0Codec::encode_actor(record).unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(store.resolve_user_actor(&identity).unwrap(), 1);
    assert_eq!(store.resolve_user_actor(&identity).unwrap(), 1);
    assert!(store
        .request_actor(json!({"actor_key_index":0}).as_object().unwrap())
        .is_err());
    assert_eq!(
        store
            .request_actor(json!({"actor_key_index":1}).as_object().unwrap())
            .unwrap(),
        2
    );
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn actor_registration_preserves_name_only_reviewer_authority() {
    let db = empty_initialized_db("actor-registration-reviewer");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());
    ensure_actor_fixture(&db);
    seed_task_change_and_patchset(&store);
    store.record_review("T-0001/C-01",&json!({"patchset_id":"T-0001/C-01/P-01","reviewer":"Alice","reviewer_kind":"user","action":"approve"})).unwrap();
    assert_eq!(
        store
            .resolve_user_actor(&UserActorIdentity::new("Alice", "alice@example.test").unwrap())
            .unwrap(),
        2
    );
    let read = BinaryDbReadTxn::new(&db);
    let review = WorkflowBinaryV0Codec::decode_review(
        &read
            .read_record(WorkflowBinaryV0Codec::review_file(), 0)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(review.actor_index_plus1, 2);
    let old = WorkflowBinaryV0Codec::decode_actor(
        &read
            .read_record(WorkflowBinaryV0Codec::actor_file(), 1)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(old.actor_meta & 0x38, 0);
    drop(read);
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn actor_registration_preserves_unknown_historical_times_on_reuse() {
    let db = empty_initialized_db("actor-registration-unknown-time");
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());
    let identity = UserActorIdentity::new("Alice", "alice@example.test").unwrap();
    store.resolve_user_actor(&identity).unwrap();
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    let mut actor = WorkflowBinaryV0Codec::decode_actor(
        &tx.read_record(WorkflowBinaryV0Codec::actor_file(), 0)
            .unwrap(),
    )
    .unwrap();
    actor.created_at_s = 0;
    actor.last_seen_at_s = 0;
    let original = WorkflowBinaryV0Codec::encode_actor(actor).unwrap();
    tx.overwrite_record(WorkflowBinaryV0Codec::actor_file(), 0, &original)
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(store.resolve_user_actor(&identity).unwrap(), 0);
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.read_record(WorkflowBinaryV0Codec::actor_file(), 0)
            .unwrap(),
        original
    );
    drop(read);
    validate_frozen_server_workflow_v0(&db).unwrap();
}
