from __future__ import annotations

import importlib

from ait.cli import remote_ci_readiness_helpers
from ait.remote_client import RemoteError

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_remote_ci_readiness_helpers() -> None:
    helper_names = [
        "_runtime_ci_capability_payload",
        "_ci_route_mismatch_guidance",
        "_readiness_supports_repo_name",
        "_remote_read_task_dag_readiness",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(remote_ci_readiness_helpers, name)


def test_runtime_ci_capability_payload_prefers_app_healthz_override(monkeypatch) -> None:
    monkeypatch.setattr(
        cli_app_module,
        "remote_get_server_health",
        lambda base_url: {
            "ok": True,
            "runtime_root": "/srv/ait",
            "ci_capabilities": {"patchset_run_ci_route": False},
            "ci_readiness": {"runtime_generation": "native_ci_runtime_v1"},
        },
    )

    payload = remote_ci_readiness_helpers._runtime_ci_capability_payload("http://example.test")
    assert payload is not None
    assert payload["ci_capabilities"]["patchset_run_ci_route"] is False
    assert payload["ci_readiness"]["runtime_generation"] == "native_ci_runtime_v1"


def test_ci_route_mismatch_guidance_uses_healthz_payload(monkeypatch) -> None:
    monkeypatch.setattr(
        cli_app_module,
        "remote_get_server_health",
        lambda base_url: {
            "ok": True,
            "runtime_root": "/srv/ait",
            "ci_capabilities": {"repo_run_ci_route": False},
            "ci_readiness": {"runtime_generation": "native_ci_runtime_v1"},
        },
    )

    message = remote_ci_readiness_helpers._ci_route_mismatch_guidance(
        base_url="http://example.test",
        route_label="repo_run_ci_route",
        cli_hint="ait repo ci-capabilities --remote origin",
        exc=RemoteError("POST http://example.test/v1/native/admin/repositories/demo:runCi failed: 404 Not Found"),
    )

    lowered = message.lower()
    assert "repo_run_ci_route" in lowered
    assert "restart/update" in lowered
    assert "/srv/ait" in message


def test_remote_read_task_dag_readiness_supports_app_overrides(monkeypatch) -> None:
    calls: list[tuple] = []

    def fake_supports_repo_name(base_url, graph, *, repo_name=None, current_plan_revision_id=None):
        calls.append(("modern", repo_name, current_plan_revision_id))
        return {"ok": True}

    monkeypatch.setattr(cli_app_module, "remote_read_task_dag_readiness", fake_supports_repo_name)
    result = remote_ci_readiness_helpers._remote_read_task_dag_readiness(
        "http://example.test",
        {"nodes": []},
        repo_name="repo-alpha",
        current_plan_revision_id="PR-1",
    )
    assert result["ok"] is True
    assert ("modern", "repo-alpha", "PR-1") in calls

    calls.clear()

    def fake_without_repo_name(base_url, graph, current_plan_revision_id=None):
        calls.append(("legacy", current_plan_revision_id))
        return {"legacy": True}

    monkeypatch.setattr(cli_app_module, "remote_read_task_dag_readiness", fake_without_repo_name)
    result = remote_ci_readiness_helpers._remote_read_task_dag_readiness(
        "http://example.test",
        {"nodes": []},
        repo_name="repo-alpha",
        current_plan_revision_id="PR-1",
    )
    assert result["legacy"] is True
    assert ("legacy", "PR-1") in calls
