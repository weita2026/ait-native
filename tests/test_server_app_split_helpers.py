from __future__ import annotations

from types import SimpleNamespace
from typing import get_type_hints

import ait_server.app as server_app
import ait_server.admin_cache as admin_cache
import ait_server.route_request_models as route_request_models
import ait_server.workflow_async_jobs as workflow_async_jobs
from tests.postgres_fake import fake_postgres_context, fake_postgres_dsn


def test_admin_cache_returns_computed_then_cached_and_clear_resets(monkeypatch):
    monkeypatch.setenv("AIT_SERVER_PRESSURE_METRICS_CACHE_TTL_SECONDS", "5")
    admin_cache._clear_admin_response_cache()
    calls = {"count": 0}

    def compute():
        calls["count"] += 1
        return {"value": calls["count"]}

    first = admin_cache._cached_admin_payload("metrics", (1, 2), compute)
    second = admin_cache._cached_admin_payload("metrics", (1, 2), compute)

    assert first["cache_state"] == "computed"
    assert second["cache_state"] == "cached"
    assert first["value"] == 1
    assert second["value"] == 1
    assert calls["count"] == 1

    admin_cache._clear_admin_response_cache()
    third = admin_cache._cached_admin_payload("metrics", (1, 2), compute)
    assert third["cache_state"] == "computed"
    assert third["value"] == 2
    assert calls["count"] == 2


def test_queue_mode_normalizes_known_values(monkeypatch):
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "async")
    assert workflow_async_jobs._queue_mode() == "async"
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "INLINE")
    assert workflow_async_jobs._queue_mode() == "inline"
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "weird")
    assert workflow_async_jobs._queue_mode() == "inline"


def test_policy_ci_and_land_job_helpers_build_payloads_and_enqueue(monkeypatch):
    ctx = SimpleNamespace()
    monkeypatch.setattr(
        workflow_async_jobs,
        "get_patchset",
        lambda _ctx, patchset_id: {"patchset_id": patchset_id, "change_id": "AITC-1", "repo_id": "REPO-1", "patchset_number": 3},
    )
    monkeypatch.setattr(
        workflow_async_jobs,
        "get_land_request",
        lambda _ctx, submission_id: {"submission_id": submission_id, "change_id": "AITC-1", "repo_id": "REPO-1", "patchset_id": "AITP-1", "land_seq": 9},
    )
    monkeypatch.setattr(
        workflow_async_jobs,
        "get_change",
        lambda _ctx, change_id: {"change_id": change_id, "repo_name": "ait", "repo_id": "REPO-1", "change_seq": 7},
    )

    captured: list[tuple[str, str, dict[str, object]]] = []

    def fake_enqueue(_ctx, repo_name, job_type, payload, *, max_attempts, dedupe_active):
        captured.append((repo_name, job_type, dict(payload)))
        return {"job_type": job_type, "payload": dict(payload), "max_attempts": max_attempts, "dedupe_active": dedupe_active}

    monkeypatch.setattr(workflow_async_jobs, "enqueue_async_job", fake_enqueue)
    monkeypatch.setattr(workflow_async_jobs, "patchset_ci_contract_available", lambda _ctx, patchset_id: patchset_id == "AITP-1")
    pending_calls: list[dict[str, str]] = []
    monkeypatch.setattr(
        workflow_async_jobs,
        "mark_patchset_ci_pending",
        lambda _ctx, patchset_id, *, trigger, job_state: pending_calls.append(
            {"patchset_id": patchset_id, "trigger": trigger, "job_state": job_state}
        ),
    )

    policy_payload = workflow_async_jobs._policy_job_payload(ctx, "AITP-1")
    ci_payload = workflow_async_jobs._patchset_ci_job_payload(ctx, "AITP-1")
    land_payload = workflow_async_jobs._land_job_payload(ctx, "LAND-1")
    assert policy_payload["repo_name"] == "ait"
    assert policy_payload["patchset_number"] == 3
    assert ci_payload["repo_name"] == "ait"
    assert ci_payload["patchset_number"] == 3
    assert land_payload["repo_name"] == "ait"
    assert land_payload["land_seq"] == 9

    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "async")
    ci_job = workflow_async_jobs._maybe_enqueue_patchset_ci(ctx, "AITP-1")
    policy_job = workflow_async_jobs._maybe_enqueue_policy(ctx, "AITP-1")
    land_job = workflow_async_jobs._maybe_enqueue_land(ctx, "LAND-1")
    assert ci_job is not None
    assert policy_job is not None
    assert land_job is not None
    assert pending_calls == [{"patchset_id": "AITP-1", "trigger": "queued_rerun", "job_state": "queued"}]
    assert captured == [
        ("ait", "patchset.ci", ci_payload),
        ("ait", "policy.evaluate", policy_payload),
        ("ait", "land.process", land_payload),
    ]


def test_patchset_ci_helper_runs_inline_when_queue_is_inline(monkeypatch):
    ctx = SimpleNamespace()
    monkeypatch.setattr(workflow_async_jobs, "patchset_ci_contract_available", lambda _ctx, patchset_id: patchset_id == "AITP-1")
    monkeypatch.setattr(
        workflow_async_jobs,
        "run_patchset_ci",
        lambda _ctx, patchset_id, *, trigger: {"patchset_id": patchset_id, "trigger": trigger, "tests_status": "pass"},
    )

    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")
    result = workflow_async_jobs._maybe_start_patchset_ci(ctx, "AITP-1", trigger="patchset_select")

    assert result == {
        "ci_result": {
            "patchset_id": "AITP-1",
            "trigger": "patchset_select",
            "tests_status": "pass",
        }
    }


def test_patchset_publish_ci_followup_defers_inline_ci(monkeypatch):
    ctx = SimpleNamespace()
    monkeypatch.setattr(workflow_async_jobs, "patchset_ci_contract_available", lambda _ctx, patchset_id: patchset_id == "AITP-1")
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")

    result = workflow_async_jobs._maybe_follow_patchset_publish_with_ci(ctx, "AITP-1")

    assert result == {
        "ci_followup": {
            "state": "deferred",
            "trigger": "patchset_publish",
            "queue_mode": "inline",
            "reason": "Patchset publish keeps patchset CI off the inline request path; run patchset CI explicitly through the dedicated CI route.",
            "command": "ait patchset rerun-ci AITP-1",
        }
    }


def test_patchset_publish_policy_followup_defers_until_evidence_changes(monkeypatch):
    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "async")
    async_followup = workflow_async_jobs._patchset_publish_policy_followup("AITP-1")
    assert async_followup == {
        "policy_followup": {
            "state": "deferred",
            "queue_mode": "async",
            "reason": "Patchset publish keeps policy evaluation off the request path until patchset evidence changes through attestation, review, patchset selection, or waiver actions.",
            "activation_events": [
                "patchset.selected",
                "attestation.upserted",
                "review.recorded",
                "policy.waived",
            ],
        }
    }

    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")
    inline_followup = workflow_async_jobs._patchset_publish_policy_followup("AITP-1")
    assert inline_followup == {
        "policy_followup": {
            "state": "deferred",
            "queue_mode": "inline",
            "reason": "Patchset publish keeps policy evaluation off the request path until patchset evidence changes through attestation, review, patchset selection, or waiver actions.",
            "activation_events": [
                "patchset.selected",
                "attestation.upserted",
                "review.recorded",
                "policy.waived",
            ],
            "command": "ait policy eval AITP-1",
        }
    }


def test_patchset_publish_request_path_audit_marks_required_vs_deferred_work():
    result = {
        "ci_followup": {"state": "deferred"},
        "policy_followup": {"state": "deferred"},
        "notification_followup": {"delivery": "background"},
    }
    timings = {
        "publish_patchset_seconds": 2.4,
        "ci_followup_seconds": 1.2,
        "policy_followup_seconds": 0.001,
        "notification_followup_seconds": 0.0002,
    }

    audit = server_app._patchset_publish_request_path_audit(result, timings)

    assert audit == [
        {
            "phase": "publish_patchset",
            "state": "completed",
            "seconds": 2.4,
            "required_for_immediate_correctness": True,
            "deferred_safe": False,
            "reason": "Patchset identity, revision selection, and workflow-land patchset readiness depend on this persistence step.",
        },
        {
            "phase": "ci_followup",
            "state": "deferred",
            "seconds": 1.2,
            "required_for_immediate_correctness": False,
            "deferred_safe": True,
            "reason": "Patchset CI evidence is required later for land gates, but patchset publication stays correct when CI is queued or deferred off the request path.",
        },
        {
            "phase": "policy_followup",
            "state": "deferred",
            "seconds": 0.001,
            "required_for_immediate_correctness": False,
            "deferred_safe": True,
            "reason": "Policy may wait for attestation, review, selection, or waiver evidence without changing patchset identity or next-action correctness.",
        },
        {
            "phase": "notification_followup",
            "state": "background",
            "seconds": 0.0002,
            "required_for_immediate_correctness": False,
            "deferred_safe": True,
            "reason": "Notification scheduling is observability-only and does not affect patchset publication correctness.",
        },
    ]


def test_route_request_models_preserve_defaults_and_server_app_uses_them(tmp_path, monkeypatch):
    first_repo = route_request_models.RepositoryCreate(repo_name="ait")
    second_repo = route_request_models.RepositoryCreate(repo_name="ait")
    first_repo.policy["mode"] = "strict"
    assert second_repo.policy == {}

    first_ci = route_request_models.RunRepoCiRequest()
    second_ci = route_request_models.RunRepoCiRequest()
    first_ci.suite_ids.append("stable_smoke")
    assert second_ci.suite_ids == []

    release = route_request_models.ReleasePublishRequest(
        release_id="REL-1",
        version="1.0.0",
        line="main",
        snapshot_id="SNP-1",
        manifest_hash="sha256:abc",
        profile="prod",
    )
    assert release.artifacts == []
    assert release.metadata == {}

    data_dir = tmp_path / "server-data"
    fake_postgres_context(data_dir)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")

    app = server_app.create_app()
    route_map = {
        (route.path, tuple(sorted(route.methods or ()))): route.endpoint
        for route in app.routes
        if getattr(route, "path", None)
    }
    assert get_type_hints(route_map[("/v1/native/repositories", ("POST",))])["req"] is route_request_models.RepositoryCreate
    assert get_type_hints(route_map[("/v1/native/sessions/{session_id}:turn", ("POST",))])["req"] is route_request_models.SessionTurnRequest
    assert get_type_hints(route_map[("/v1/native/changes/{change_id}/patchsets", ("POST",))])["req"] is route_request_models.PatchsetPublish
