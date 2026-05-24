from __future__ import annotations

import importlib

import pytest

from ait.cli import remote_repository_defaults
from ait.remote_client import RemoteError

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_remote_repository_defaults_helpers() -> None:
    helper_names = [
        "_remote_tuple",
        "_remote_error_status_code",
        "_verify_remote_repository",
        "_sync_remote_repository_defaults",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(remote_repository_defaults, name)


def test_remote_tuple_prefers_remote_repo_name_and_falls_back_to_config(monkeypatch) -> None:
    ctx = object()

    monkeypatch.setattr(
        remote_repository_defaults,
        "get_remote",
        lambda _ctx, remote_name: {"name": remote_name, "url": "http://example.test", "repo_name": "repo-remote"},
    )
    monkeypatch.setattr(remote_repository_defaults, "load_config", lambda _ctx: {"repo_name": "repo-config"})

    remote_row, repo_name = remote_repository_defaults._remote_tuple(ctx, "origin")
    assert remote_row["name"] == "origin"
    assert repo_name == "repo-remote"

    monkeypatch.setattr(
        remote_repository_defaults,
        "get_remote",
        lambda _ctx, remote_name: {"name": remote_name, "url": "http://example.test", "repo_name": ""},
    )
    remote_row, repo_name = remote_repository_defaults._remote_tuple(ctx, "origin")
    assert remote_row["name"] == "origin"
    assert repo_name == "repo-config"


def test_remote_error_status_code_parses_http_status() -> None:
    assert remote_repository_defaults._remote_error_status_code(RemoteError("GET /healthz failed: 404 Not Found")) == 404
    assert remote_repository_defaults._remote_error_status_code(RemoteError("GET /healthz failed: 405 Method Not Allowed")) == 405
    assert remote_repository_defaults._remote_error_status_code(RemoteError("not an http status payload")) is None


def test_verify_remote_repository_checks_repo_name_and_default_line(monkeypatch) -> None:
    monkeypatch.setattr(
        cli_app_module,
        "remote_get_repository",
        lambda base_url, repo_name: {"repo_name": repo_name, "default_line": "main", "url": base_url},
        raising=False,
    )

    payload = remote_repository_defaults._verify_remote_repository("http://example.test", "repo-a", "main")
    assert payload["repo_name"] == "repo-a"

    monkeypatch.setattr(
        cli_app_module,
        "remote_get_repository",
        lambda base_url, repo_name: {"repo_name": "repo-b", "default_line": "main", "url": base_url},
        raising=False,
    )
    with pytest.raises(RemoteError, match="unexpected repository"):
        remote_repository_defaults._verify_remote_repository("http://example.test", "repo-a", "main")

    monkeypatch.setattr(
        cli_app_module,
        "remote_get_repository",
        lambda base_url, repo_name: {"repo_name": repo_name, "default_line": "dev", "url": base_url},
        raising=False,
    )
    with pytest.raises(RemoteError, match="default line mismatch"):
        remote_repository_defaults._verify_remote_repository("http://example.test", "repo-a", "main")


def test_sync_remote_repository_defaults_creates_missing_remote_repo(monkeypatch) -> None:
    calls: list[tuple[str, object]] = []
    ctx = object()

    monkeypatch.setattr(
        remote_repository_defaults,
        "_remote_tuple",
        lambda _ctx, remote_name: ({"name": remote_name, "url": "http://example.test"}, "repo-a"),
    )
    monkeypatch.setattr(remote_repository_defaults, "load_config", lambda _ctx: {"default_line": "main"})
    monkeypatch.setattr(remote_repository_defaults, "repo_id_namespace_prefix", lambda _ctx: "RT")
    monkeypatch.setattr(remote_repository_defaults, "load_policy", lambda _ctx: {"required": True})

    def fake_verify(base_url: str, repo_name: str, default_line: str) -> dict[str, object]:
        calls.append(("verify", base_url, repo_name, default_line))
        if len(calls) == 1:
            raise RemoteError("GET /v1/native/repositories/repo-a failed: 404 Not Found")
        return {"repo_name": repo_name, "default_line": default_line, "id_namespace_prefix": "RT", "policy": {"required": True}}

    monkeypatch.setattr(remote_repository_defaults, "_verify_remote_repository", fake_verify)
    monkeypatch.setattr(
        cli_app_module,
        "ensure_repository",
        lambda base_url, repo_name, default_line, **kwargs: calls.append(
            ("ensure", base_url, repo_name, default_line, kwargs["id_namespace_prefix"], kwargs["policy"])
        ),
        raising=False,
    )

    remote_row, repo_name = remote_repository_defaults._sync_remote_repository_defaults(ctx, "origin")

    assert remote_row["name"] == "origin"
    assert repo_name == "repo-a"
    assert calls == [
        ("verify", "http://example.test", "repo-a", "main"),
        ("ensure", "http://example.test", "repo-a", "main", "RT", {"required": True}),
        ("verify", "http://example.test", "repo-a", "main"),
    ]


def test_sync_remote_repository_defaults_reconciles_prefix_or_policy_drift(monkeypatch) -> None:
    calls: list[tuple[str, object]] = []
    ctx = object()

    monkeypatch.setattr(
        remote_repository_defaults,
        "_remote_tuple",
        lambda _ctx, remote_name: ({"name": remote_name, "url": "http://example.test"}, "repo-a"),
    )
    monkeypatch.setattr(remote_repository_defaults, "load_config", lambda _ctx: {"default_line": "main"})
    monkeypatch.setattr(remote_repository_defaults, "repo_id_namespace_prefix", lambda _ctx: "RT")
    monkeypatch.setattr(remote_repository_defaults, "load_policy", lambda _ctx: {"required": True})

    verify_payloads = [
        {"repo_name": "repo-a", "default_line": "main", "id_namespace_prefix": "OLD", "policy": {"required": False}},
        {"repo_name": "repo-a", "default_line": "main", "id_namespace_prefix": "RT", "policy": {"required": True}},
    ]

    def fake_verify(base_url: str, repo_name: str, default_line: str) -> dict[str, object]:
        calls.append(("verify", base_url, repo_name, default_line))
        return verify_payloads.pop(0)

    monkeypatch.setattr(remote_repository_defaults, "_verify_remote_repository", fake_verify)
    monkeypatch.setattr(
        cli_app_module,
        "ensure_repository",
        lambda base_url, repo_name, default_line, **kwargs: calls.append(
            ("ensure", base_url, repo_name, default_line, kwargs["id_namespace_prefix"], kwargs["policy"])
        ),
        raising=False,
    )

    remote_row, repo_name = remote_repository_defaults._sync_remote_repository_defaults(ctx, "origin")

    assert remote_row["name"] == "origin"
    assert repo_name == "repo-a"
    assert calls == [
        ("verify", "http://example.test", "repo-a", "main"),
        ("ensure", "http://example.test", "repo-a", "main", "RT", {"required": True}),
        ("verify", "http://example.test", "repo-a", "main"),
    ]
