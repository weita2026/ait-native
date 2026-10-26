use super::*;
use crate::binary_runtime::RoutedBinaryWorkflowStore;
use ait_server_core::foundation::remote_binary_db::{
    BinaryDbCommandScope, BinaryDbReadTxn, BinaryDbWriteTxn,
};
use ait_server_core::foundation::server_content_binary_db::{
    server_snapshot_hash48_from_id, ServerBinaryDbLineStore, ServerBinaryDbSnapshotStore,
    ServerBinarySnapshotPayload, ServerBinarySnapshotRecord, SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use ait_server_core::foundation::workflow_binary_v0::{
    V0InventoryIndexRecord, V0OrdinalIndexRecord, V0SnapshotLinkRecord, WorkflowBinaryV0Codec,
    SNAPSHOT_LINK_HAS_CHANGE,
};

#[tokio::test]
async fn native_review_actor_identity_survives_http_and_stored_readback() {
    for has_change in [false, true] {
        let directory = TestDirectory::new(if has_change {
            "native-review-actor-change"
        } else {
            "native-review-actor-task"
        });
        let state = fresh_state(directory.path());
        let (_, db) = state.operational_binary.repository_db(1).unwrap();
        let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
        let existing = lines
            .line_by_name(&BinaryDbReadTxn::new(&db), "main")
            .unwrap();
        let line_index = existing
            .map(|(index, _)| index)
            .unwrap_or_else(|| lines.create_line("main", 0, 1).unwrap());
        let snapshots =
            ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
        let base = "SNP-0000000000A1";
        let revision = "SNP-0000000000A2";
        let append_snapshot = |id, parent, line_index, line_name: &str| {
            snapshots
                .append_snapshot(
                    id,
                    ServerBinarySnapshotRecord {
                        actor_index_plus1: 0,
                        snapshot_meta: 0,
                        history_flags: 0,
                        payload_len: 0,
                        payload_offset: 0,
                        snapshot_hash48: server_snapshot_hash48_from_id(id).unwrap(),
                        parent_snapshot_index_plus1: parent,
                        root_tree_pack_index_plus1: 0,
                        root_entry_ordinal: 0,
                        line_index_plus1: line_index + 1,
                        manifest_hash: [0; 32],
                        file_count: 0,
                        total_bytes: 0,
                        created_at_s: 1,
                    },
                    &ServerBinarySnapshotPayload {
                        line_name: line_name.to_string(),
                        message: Some("Reviewer fixture".to_string()),
                    },
                )
                .unwrap();
        };
        append_snapshot(base, 0, line_index, "main");
        lines.set_line_head("main", 1, 2).unwrap();
        let (repo_name, workflow) = state
            .workflow_service
            .as_any()
            .downcast_ref::<RoutedBinaryWorkflowStore>()
            .unwrap()
            .store_for_repo("1")
            .unwrap();
        ait_server_core::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store::new_frozen(db.clone())
            .resolve_user_actor(&ait_server_core::foundation::workflow_binary_v0_adapter::UserActorIdentity::new("fixture","fixture@example.test").unwrap()).unwrap();
        let task = workflow
            .create_task(
                &repo_name,
                &json!({"actor_key_index":0,"title": "Review identity", "intent": "Native bot Actor"}),
            )
            .unwrap();
        let change = has_change.then(|| {
            workflow
                .create_change(
                    &repo_name,
                    &json!({"actor_key_index":0,
                        "task_id": task["task_id"], "title": "Review identity", "base_line": "main"
                    }),
                )
                .unwrap()
        });
        let change_ref = change
            .as_ref()
            .and_then(|change| change["change_ref"].as_str());
        let task_id = task["task_id"].as_str().unwrap();
        let owner_ref = change_ref.unwrap_or(task_id);
        let worktree = task_id.to_ascii_lowercase();
        let feature_line = format!("feature/{worktree}");
        let feature_line_index = lines.create_line(&feature_line, 1, 1).unwrap();
        append_snapshot(revision, 1, feature_line_index, &feature_line);
        // Seed admitted capture ownership so the real remote Patchset guard remains active.
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerSnapshotCapture)
                .unwrap();
        let link = V0SnapshotLinkRecord {
            link_meta: if has_change {
                SNAPSHOT_LINK_HAS_CHANGE
            } else {
                0
            },
            snapshot_ordinal: 0,
            task_index: 0,
            change_index_plus1: u32::from(has_change),
            content_snapshot_index: 1,
            created_at_s: 1,
        };
        tx.append_record(
            WorkflowBinaryV0Codec::snapshot_link_file(),
            &WorkflowBinaryV0Codec::encode_snapshot_link(link).unwrap(),
        )
        .unwrap();
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
            0,
            &WorkflowBinaryV0Codec::encode_task_snapshot_index(V0OrdinalIndexRecord {
                latest_index_plus1: 1,
                count: 1,
                next_ordinal: 1,
                reserved0: 0,
            })
            .unwrap(),
        )
        .unwrap();
        if has_change {
            tx.overwrite_record(
                WorkflowBinaryV0Codec::chain_index_file("change_snapshot_index.bin"),
                0,
                &WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord {
                    latest_index_plus1: 1,
                    count: 1,
                    reserved0: 0,
                })
                .unwrap(),
            )
            .unwrap();
        }
        tx.commit().unwrap();
        drop(tx);
        let router = build_router_with_state(state);
        let owner_uri = format!(
            "/v1/native/repository-authorities/1/changes/{}",
            owner_ref.replace('/', "%2F")
        );
        let response = router
            .clone()
            .oneshot(post_json(
                &format!("{owner_uri}/patchsets"),
                json!({
                    "base_snapshot_id": base, "revision_snapshot_id": revision,
                    "summary": "Exact candidate review", "author_mode": "ai_with_human_review", "actor_key_index":0
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let patchset = response_json(response).await;
        assert_eq!(patchset["task_id"], task_id);
        assert_eq!(patchset["change_ref"].as_str(), change_ref);
        let missing = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/native/repository-authorities/1/patchsets/{}/attestation",
                        patchset["patchset_id"]
                            .as_str()
                            .unwrap()
                            .replace('/', "%2F")
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{owner_uri}/patchsets"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await[0]["patchset_id"],
            patchset["patchset_id"]
        );
        let response = router
            .clone()
            .oneshot(post_json(
                &format!("{owner_uri}:selectPatchset"),
                json!({"actor_key_index":0,"patchset_id":patchset["patchset_id"]}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["selection_changed"], false);
        let uri = format!("{owner_uri}/reviews");
        for kind in ["bot", "user"] {
            let response = router
                .clone()
                .oneshot(post_json(
                    &uri,
                    json!({
                        "patchset_id": patchset["patchset_id"], "reviewer": "Einstein",
                        "reviewer_kind": kind, "action": "approve"
                    }),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let review = response_json(response).await;
            assert_eq!(review["reviewer"], "Einstein");
            assert_eq!(review["reviewer_kind"], kind);
            assert_eq!(review["patchset_id"], patchset["patchset_id"]);
        }
        let response = router
            .clone()
            .oneshot(post_json(
                &uri,
                json!({
                    "patchset_id": patchset["patchset_id"], "reviewer": "Einstein",
                    "reviewer_kind": "human", "action": "approve"
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = router
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result = response_json(response).await;
        assert_eq!(result["reviews"].as_array().unwrap().len(), 2);
        assert_eq!(result["reviews"][0]["reviewer_kind"], "bot");
        assert_eq!(result["reviews"][1]["reviewer_kind"], "user");
        assert_eq!(result["summary"]["approval_count"], 2);
    }
}
