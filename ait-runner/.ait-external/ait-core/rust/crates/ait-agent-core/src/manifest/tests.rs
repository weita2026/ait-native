use super::*;

#[test]
fn counts_workers_by_transport_after_rust_normalization() {
    let payload = json!({
        "version": 1,
        "workers": {
            "telegram/main": {"token": "t"},
            "discord/ops": {"kind": "discord", "application_id": "app", "bot_token": "bot"},
            "slack/team": {"kind": "slack", "app_token": "x"},
            "line/team": {"kind": "line", "token": "x", "secret": "s"}
        }
    });

    let counts = count_manifest_workers(&payload);

    assert_eq!(
        counts
            .iter()
            .map(|count| (count.transport, count.configured_workers))
            .collect::<Vec<_>>(),
        vec![
            (TransportKind::Telegram, 1),
            (TransportKind::Discord, 1),
            (TransportKind::Slack, 1),
            (TransportKind::Line, 1),
        ]
    );
}

#[test]
fn lists_individual_workers_for_runtime_selection() {
    let payload = json!({
        "version": 1,
        "workers": {
            "telegram/main": {"token": "t"},
            "discord/ops": {"kind": "discord", "application_id": "app", "bot_token": "bot"}
        }
    });

    let workers = list_manifest_workers(&payload);

    assert_eq!(
        workers
            .iter()
            .map(|worker| (&worker.key, worker.transport, &worker.name))
            .collect::<Vec<_>>(),
        vec![
            (
                &"discord/ops".to_string(),
                TransportKind::Discord,
                &"ops".to_string()
            ),
            (
                &"telegram/main".to_string(),
                TransportKind::Telegram,
                &"main".to_string()
            ),
        ]
    );
}

#[test]
fn agent_worker_manifest_boundary_preserves_core_schema_and_fixture_shape() {
    let payload = json!({
        "version": 1,
        "workers": {
            "telegram/main": {"token": "t"},
            "discord/ops": {"kind": "discord", "application_id": "app", "bot_token": "bot"}
        }
    });

    assert_eq!(
        agent_worker_manifest_ir_version(),
        ait_core::worker_manifest::worker_manifest_ir_version()
    );
    assert_eq!(
        agent_worker_manifest_schema_json(),
        ait_core::worker_manifest::worker_manifest_schema_json()
    );
    assert_eq!(
        agent_default_worker_manifest_config_json(),
        ait_core::worker_manifest::default_worker_manifest_config_json()
    );
    assert_eq!(
        agent_normalize_worker_manifest_document_json(&payload, Some("/tmp/workers.json")),
        ait_core::worker_manifest::normalize_worker_manifest_document_json(
            &payload,
            Some("/tmp/workers.json")
        )
    );
    assert_eq!(
        agent_select_telegram_worker_json(&payload, Some("main")),
        ait_core::worker_manifest::select_telegram_worker_json(&payload, Some("main"))
    );
}
