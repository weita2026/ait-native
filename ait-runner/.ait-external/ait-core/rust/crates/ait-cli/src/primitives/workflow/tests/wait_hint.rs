use super::*;

#[test]
fn workflow_wait_hint_history_accepts_change_remote_trait() {
    let mut remote = FakeChangeRemote {
        change_rows: vec![
            json!({
                "change_id": "RCC-1",
                "status": "landed",
                "current_patchset_number": 1,
                "selected_patchset_number": 1,
                "landed_at": "2026-01-01T00:02:00Z"
            }),
            json!({
                "change_id": "RCC-IGNORED",
                "status": "active",
                "current_patchset_number": 1,
                "selected_patchset_number": 1,
                "landed_at": "2026-01-01T00:03:00Z"
            }),
            json!({
                "change_id": "RCC-2",
                "status": "landed",
                "current_patchset_number": 1,
                "selected_patchset_number": 1,
                "landed_at": "2026-01-01T00:04:00Z"
            }),
        ],
        change_details: BTreeMap::from([
            (
                "RCC-1".to_string(),
                json!({
                    "selected_patchset": {
                        "created_at": "2026-01-01T00:00:00Z"
                    },
                    "patchset_ci_status": {
                        "ci_completed_at_s": 1_767_225_620_u64
                    },
                    "change": {
                        "landed_at": "2026-01-01T00:02:00Z"
                    }
                }),
            ),
            (
                "RCC-2".to_string(),
                json!({
                    "selected_patchset": {
                        "created_at": "2026-01-01T00:00:00Z"
                    },
                    "patchset_ci_status": {
                        "ci_completed_at_s": 1_767_225_700_u64
                    },
                    "change": {
                        "landed_at": "2026-01-01T00:04:00Z"
                    }
                }),
            ),
        ]),
        ..Default::default()
    };

    assert_eq!(
        workflow_bootstrap_wait_hint_seconds_from_history_with_task_remote(
            &mut remote,
            "fixture-ait",
            "ready",
        )
        .expect("wait hint history"),
        Some(60)
    );
}

#[test]
fn workflow_wait_hint_land_history_accepts_change_remote_trait() {
    let mut remote = FakeChangeRemote {
        change_rows: vec![
            json!({
                "change_id": "RCC-1",
                "status": "landed",
                "current_patchset_number": 1,
                "selected_patchset_number": 1,
                "landed_at": "2026-01-01T00:02:00Z"
            }),
            json!({
                "change_id": "RCC-2",
                "status": "landed",
                "current_patchset_number": 1,
                "selected_patchset_number": 1,
                "landed_at": "2026-01-01T00:04:00Z"
            }),
        ],
        change_details: BTreeMap::from([
            (
                "RCC-1".to_string(),
                json!({
                    "patchset_ci_status": {
                        "ci_completed_at_s": 1_767_225_620_u64
                    },
                    "change": {
                        "landed_at": "2026-01-01T00:02:00Z"
                    }
                }),
            ),
            (
                "RCC-2".to_string(),
                json!({
                    "patchset_ci_status": {
                        "ci_completed_at_s": 1_767_225_700_u64
                    },
                    "change": {
                        "landed_at": "2026-01-01T00:04:00Z"
                    }
                }),
            ),
        ]),
        ..Default::default()
    };

    assert_eq!(
        workflow_bootstrap_wait_hint_seconds_from_history_with_task_remote(
            &mut remote,
            "fixture-ait",
            "land",
        )
        .expect("land wait hint history"),
        Some(120)
    );
}

#[test]
fn workflow_wait_hint_history_reads_accept_change_remote_trait() {
    let mut remote = FakeChangeRemote {
        change_rows: vec![json!({
            "change_id": "RCC-1",
            "status": "landed"
        })],
        change_details: BTreeMap::from([(
            "RCC-1".to_string(),
            json!({
                "change": {
                    "change_id": "RCC-1"
                }
            }),
        )]),
        ..Default::default()
    };

    let rows = workflow_wait_hint_change_rows_with_task_remote(&mut remote, "fixture-ait")
        .expect("read wait hint change rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["change_id"], json!("RCC-1"));

    let detail =
        workflow_wait_hint_change_detail_with_task_remote(&mut remote, "fixture-ait", "RCC-1")
            .expect("read wait hint change detail");
    assert_eq!(detail["change"]["change_id"], json!("RCC-1"));
    assert_eq!(
        remote.change_detail_requests,
        vec![("RCC-1".to_string(), Some("fixture-ait".to_string()))]
    );

    let err = workflow_wait_hint_change_detail_with_task_remote(
        &mut remote,
        "fixture-ait",
        "RCC-MISSING",
    )
    .expect_err("missing detail should fail");
    assert!(err.contains("Unknown change"));
}
