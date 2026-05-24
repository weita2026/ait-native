from __future__ import annotations

import importlib

from ait.cli import remote_session_wrappers

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_remote_session_wrappers() -> None:
    helper_names = [
        "_infer_remote_repo_name",
        "remote_get_session",
        "remote_append_session_event",
        "remote_create_session_turn",
        "remote_create_session_checkpoint",
        "remote_list_session_checkpoints",
        "remote_list_session_events",
        "remote_resume_session",
        "remote_close_session",
        "remote_advance_task_dag_run",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(remote_session_wrappers, name)


def test_infer_remote_repo_name_prefers_explicit_name_and_matching_remote(monkeypatch) -> None:
    assert remote_session_wrappers._infer_remote_repo_name("http://example.test", "repo-explicit") == "repo-explicit"
    assert remote_session_wrappers._infer_remote_repo_name("", None) is None

    monkeypatch.setattr(
        remote_session_wrappers.RepoContext,
        "discover",
        staticmethod(lambda: (_ for _ in ()).throw(FileNotFoundError("missing repo"))),
    )
    assert remote_session_wrappers._infer_remote_repo_name("http://example.test", None) is None

    monkeypatch.setattr(
        remote_session_wrappers.RepoContext,
        "discover",
        staticmethod(lambda: object()),
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "list_remotes",
        lambda ctx: [
            {"url": "http://example.test/", "repo_name": "repo-a"},
            {"url": "http://other.test", "repo_name": "repo-other"},
        ],
    )
    assert remote_session_wrappers._infer_remote_repo_name("http://example.test", None) == "repo-a"

    monkeypatch.setattr(
        remote_session_wrappers,
        "list_remotes",
        lambda ctx: [
            {"url": "http://example.test", "repo_name": "repo-a"},
            {"url": "http://example.test/", "repo_name": "repo-b"},
        ],
    )
    assert remote_session_wrappers._infer_remote_repo_name("http://example.test/", None) is None


def test_remote_session_wrappers_forward_inferred_repo_name(monkeypatch) -> None:
    monkeypatch.setattr(remote_session_wrappers, "_infer_remote_repo_name", lambda base_url, repo_name=None: "demo")
    calls: dict[str, tuple] = {}

    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_get_session",
        lambda base_url, session_id, repo_name=None: calls.setdefault("get", (base_url, session_id, repo_name))
        or {"session_id": session_id},
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_append_session_event",
        lambda base_url, session_id, event_type, payload=None, repo_name=None: calls.setdefault(
            "append",
            (base_url, session_id, event_type, payload, repo_name),
        )
        or {"event_type": event_type},
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_create_session_turn",
        lambda base_url, session_id, **kwargs: calls.setdefault("turn", (base_url, session_id, kwargs)) or {"ok": True},
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_create_session_checkpoint",
        lambda base_url, session_id, summary, **kwargs: calls.setdefault(
            "checkpoint",
            (base_url, session_id, summary, kwargs),
        )
        or {"ok": True},
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_list_session_checkpoints",
        lambda base_url, session_id, repo_name=None: calls.setdefault(
            "checkpoints",
            (base_url, session_id, repo_name),
        )
        or [],
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_list_session_events",
        lambda base_url, session_id, **kwargs: calls.setdefault("events", (base_url, session_id, kwargs)) or [],
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_resume_session",
        lambda base_url, session_id, **kwargs: calls.setdefault("resume", (base_url, session_id, kwargs)) or {"ok": True},
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_close_session",
        lambda base_url, session_id, **kwargs: calls.setdefault("close", (base_url, session_id, kwargs)) or {"ok": True},
    )
    monkeypatch.setattr(
        remote_session_wrappers,
        "_remote_advance_task_dag_run",
        lambda base_url, session_id, graph, **kwargs: calls.setdefault(
            "advance",
            (base_url, session_id, graph, kwargs),
        )
        or {"ok": True},
    )

    remote_session_wrappers.remote_get_session("http://example.test", "S-1")
    remote_session_wrappers.remote_append_session_event("http://example.test", "S-1", "event.demo", {"a": 1})
    remote_session_wrappers.remote_create_session_turn(
        "http://example.test",
        "S-1",
        text="hello",
        surface="telegram",
        title="Demo",
    )
    remote_session_wrappers.remote_create_session_checkpoint(
        "http://example.test",
        "S-1",
        "summary",
        snapshot_id="SNP-1",
        based_on_sequence=3,
    )
    remote_session_wrappers.remote_list_session_checkpoints("http://example.test", "S-1")
    remote_session_wrappers.remote_list_session_events("http://example.test", "S-1", after_sequence=4, limit=20)
    remote_session_wrappers.remote_resume_session("http://example.test", "S-1", after_sequence=2, limit=40)
    remote_session_wrappers.remote_close_session("http://example.test", "S-1", status="completed")
    remote_session_wrappers.remote_advance_task_dag_run(
        "http://example.test",
        "S-1",
        {"nodes": []},
        current_plan_revision_id="PR-1",
    )

    assert calls["get"] == ("http://example.test", "S-1", "demo")
    assert calls["append"] == ("http://example.test", "S-1", "event.demo", {"a": 1}, "demo")
    assert calls["turn"] == (
        "http://example.test",
        "S-1",
        {"text": "hello", "surface": "telegram", "title": "Demo", "repo_name": "demo"},
    )
    assert calls["checkpoint"] == (
        "http://example.test",
        "S-1",
        "summary",
        {"snapshot_id": "SNP-1", "resume_payload": None, "based_on_sequence": 3, "checkpoint_id": None, "repo_name": "demo"},
    )
    assert calls["checkpoints"] == ("http://example.test", "S-1", "demo")
    assert calls["events"] == (
        "http://example.test",
        "S-1",
        {"after_sequence": 4, "limit": 20, "repo_name": "demo"},
    )
    assert calls["resume"] == (
        "http://example.test",
        "S-1",
        {"after_sequence": 2, "limit": 40, "repo_name": "demo"},
    )
    assert calls["close"] == (
        "http://example.test",
        "S-1",
        {"status": "completed", "repo_name": "demo"},
    )
    assert calls["advance"] == (
        "http://example.test",
        "S-1",
        {"nodes": []},
        {"current_plan_revision_id": "PR-1", "repo_name": "demo"},
    )
