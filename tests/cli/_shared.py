from __future__ import annotations

import json
import os
import re
import shlex
import shutil
import sqlite3
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from contextlib import contextmanager
from importlib import import_module
from pathlib import Path
from types import SimpleNamespace

import uvicorn
from typer.testing import CliRunner

from ait_native import cli as cli_module
import ait_native.remote_client as remote_client_module
import ait_native.server_db as server_db_module
import ait_server.app as server_app_module
import ait_native.server_store as server_store_module
from ait_native.cli import app
from ait_native.remote_client import RemoteError, update_remote_line
from ait_native.repo_paths import RepoContext
from ait_native.server import create_app
from ait_native.server_content import reset_postgres_schema_ready_cache
from ait_native.server_content import connect as connect_server_content
from ait_native.server_db import close_postgres_connection_pools
from ait_native.server_paths import ServerContext
from ait_native.store import export_snapshot_bundle, mark_local_change_published, mark_local_task_published
from ait_chat.session_reply import AiReplyResult
from ait_protocol.common import extract_plan_section, parse_policy_yaml, policy_to_yaml
from tests.postgres_fake import FakePsycopg


class _ModuleProxy:
    def __init__(self, primary, extras):
        object.__setattr__(self, "_primary", primary)
        object.__setattr__(self, "_extras", tuple(extras))

    def __getattr__(self, name):
        if hasattr(self._primary, name):
            return getattr(self._primary, name)
        for module in self._extras:
            if hasattr(module, name):
                return getattr(module, name)
        raise AttributeError(f"{self._primary!r} has no attribute {name!r}")

    def __setattr__(self, name, value):
        setattr(self._primary, name, value)
        for module in self._extras:
            if hasattr(module, name):
                setattr(module, name, value)


_CLI_EXTRA_MODULES = [
    import_module("ait.cli.app"),
    import_module("ait.cli.commands.blame"),
    import_module("ait.cli.commands.auth"),
    import_module("ait.cli.commands.change"),
    import_module("ait.cli.commands.config"),
    import_module("ait.cli.commands.land"),
    import_module("ait.cli.commands.line"),
    import_module("ait.cli.commands.patchset"),
    import_module("ait.cli.commands.plan"),
    import_module("ait.cli.commands.plan_session"),
    import_module("ait.cli.commands.policy"),
    import_module("ait.cli.commands.queue"),
    import_module("ait.cli.plan_publish_helpers"),
    import_module("ait.cli.plan_sync_adoption"),
    import_module("ait.cli.plan_sync_scope"),
    import_module("ait.cli.commands.ref"),
    import_module("ait.cli.commands.release"),
    import_module("ait.cli.commands.remote"),
    import_module("ait.cli.commands.repo"),
    import_module("ait.cli.commands.review"),
    import_module("ait.cli.commands.session"),
    import_module("ait.cli.commands.snapshot"),
    import_module("ait.cli.commands.stack"),
    import_module("ait.cli.commands.task"),
    import_module("ait.cli.commands.workflow"),
    import_module("ait.cli.commands.workspace"),
    import_module("ait.cli.commands.worktree"),
]
cli_module = _ModuleProxy(cli_module, _CLI_EXTRA_MODULES)

_SERVER_STORE_EXTRA_MODULES = [
    import_module("ait_server.server_store"),
    import_module("ait_server.store.repo_ops"),
    import_module("ait_server.store.plans"),
]
server_store_module = _ModuleProxy(server_store_module, _SERVER_STORE_EXTRA_MODULES)

runner = CliRunner()
_FAKE_POSTGRES_DRIVER_INSTALLED = False


def _set_solo_remote_advisory(*, namespace_prefix: str | None = None) -> None:
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
    command = ["config", "set", "--plan-task-binding-mode", "advisory"]
    if namespace_prefix is not None:
        command.extend(["--id-namespace-prefix", namespace_prefix])
    assert runner.invoke(app, command, catch_exceptions=False).exit_code == 0


def _set_plan_task_binding_advisory() -> None:
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"]).exit_code == 0


def _submit_passing_code_review_summary(
    change_id: str,
    patchset_id: str,
    *,
    reviewer: str = "codex",
    reviewed_files: str = "README.md",
) -> dict:
    result = runner.invoke(
        app,
        [
            "review",
            "code",
            "submit",
            change_id,
            "--patchset",
            patchset_id,
            "--verdict",
            "pass",
            "--reviewer",
            reviewer,
            "--message",
            f"Reviewed files: {reviewed_files}; Findings: no blocking findings; Risks: low; Tests: pytest focused suite passed; Recommendation: safe to land.",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert result.exit_code == 0, result.stdout
    return json.loads(result.stdout)


def _create_remote_session_id(
    *,
    title: str = "CLI test remote session",
    task_id: str | None = None,
    change_id: str | None = None,
) -> str:
    command = ["session", "create", "--remote", "origin", "--title", title, "--json"]
    if task_id is not None:
        command.extend(["--task", task_id])
    if change_id is not None:
        command.extend(["--change", change_id])
    result = runner.invoke(app, command, catch_exceptions=False)
    assert result.exit_code == 0, result.stdout
    return str(json.loads(result.stdout)["session_id"])


def _plan_sync_remote_args(
    *extra: str,
    title: str = "Plan sync remote session",
    task_id: str | None = None,
    change_id: str | None = None,
) -> list[str]:
    return [
        "--remote",
        "origin",
        "--source-session",
        _create_remote_session_id(title=title, task_id=task_id, change_id=change_id),
        *extra,
    ]


def _sync_plan_and_load(
    target: str,
    *,
    plan_ref: str | None = None,
    remote: bool = False,
    session_title: str = "Plan sync remote session",
    task_id: str | None = None,
    change_id: str | None = None,
) -> tuple[dict, dict]:
    command = ["plan", "sync", target]
    if plan_ref is not None:
        command.extend(["--plan-ref", plan_ref])
    if remote:
        command.extend(_plan_sync_remote_args("--json", title=session_title, task_id=task_id, change_id=change_id))
    else:
        command.append("--json")
    result = runner.invoke(app, command, catch_exceptions=False)
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    plan_id = str(payload["results"][0]["plan_id"])
    show_command = ["plan", "show", plan_id, "--json"]
    if remote:
        show_command = ["plan", "show", plan_id, "--remote", "origin", "--json"]
    show_out = runner.invoke(app, show_command, catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    return payload, json.loads(show_out.stdout)


def _bind_task_worktree(task_id: str, monkeypatch, *, name: str | None = None, line_name: str = "main", chdir: bool = True) -> Path:
    normalized = re.sub(r"[^a-z0-9]+", "-", task_id.lower()).strip("-") or "task"
    worktree_name = name or normalized
    repo_root = RepoContext.discover().repo_root
    if name is None:
        task_show = runner.invoke(app, ["task", "show", task_id, "--json"], catch_exceptions=False)
        if task_show.exit_code != 0:
            task_show = runner.invoke(app, ["task", "show", task_id, "--local", "--json"], catch_exceptions=False)
        assert task_show.exit_code == 0, task_show.stdout
        task_payload = json.loads(task_show.stdout)
        existing_worktree = task_payload.get("worktree") if isinstance(task_payload.get("worktree"), dict) else {}
        existing_path_value = existing_worktree.get("path")
        existing_name = str(existing_worktree.get("name") or "").strip()
        if existing_path_value and existing_name and Path(existing_path_value).exists():
            worktree_path = Path(existing_path_value)
            if chdir:
                monkeypatch.chdir(worktree_path)
            return worktree_path

    worktree_path = repo_root / ".ait" / "workspace" / worktree_name
    if not worktree_path.exists():
        add_out = _invoke_internal_worktree_add(["worktree", "add", worktree_name, "--line", line_name, "--json"])
        assert add_out.exit_code == 0, add_out.stdout
        worktree_path = Path(json.loads(add_out.stdout)["path"])
    bind_out = _invoke_internal_worktree_bind(
        ["worktree", "bind", worktree_name, "--task", task_id, "--json"]
    )
    assert bind_out.exit_code == 0, bind_out.stdout
    if chdir:
        monkeypatch.chdir(worktree_path)
    return worktree_path


def _invoke_internal_worktree_add(argv: list[str]):
    assert argv[:2] == ["worktree", "add"], argv
    if len(argv) < 3:
        raise AssertionError(f"worktree add argv missing name: {argv!r}")
    name = str(argv[2])
    line_name: str | None = None
    path: str | None = None
    creation_kind: str | None = None
    cleanup_policy: str | None = None
    json_output = False
    index = 3
    while index < len(argv):
        token = argv[index]
        if token == "--line":
            line_name = str(argv[index + 1])
            index += 2
            continue
        if token == "--path":
            path = str(argv[index + 1])
            index += 2
            continue
        if token == "--kind":
            creation_kind = str(argv[index + 1])
            index += 2
            continue
        if token == "--cleanup-policy":
            cleanup_policy = str(argv[index + 1])
            index += 2
            continue
        if token == "--json":
            json_output = True
            index += 1
            continue
        raise AssertionError(f"Unsupported internal worktree add argv: {argv!r}")

    app_module = import_module("ait.cli.app")
    ctx = RepoContext.discover()
    payload = app_module._run_locked_workspace_command(
        ctx,
        "worktree add",
        lambda: app_module.local_add_worktree(
            ctx,
            name,
            line_name=line_name,
            path=path,
            creation_kind=creation_kind,
            cleanup_policy=cleanup_policy,
        ),
    )
    stdout = json.dumps(payload) + "\n" if json_output else ""
    return SimpleNamespace(exit_code=0, stdout=stdout, stderr="", output=stdout)


def _invoke_internal_worktree_bind(argv: list[str]):
    assert argv[:2] == ["worktree", "bind"], argv
    name: str | None = None
    task_id: str | None = None
    change_id: str | None = None
    json_output = False
    index = 2
    while index < len(argv):
        token = argv[index]
        if not token.startswith("-") and name is None:
            name = str(token)
            index += 1
            continue
        if token == "--task":
            task_id = str(argv[index + 1])
            index += 2
            continue
        if token == "--change":
            change_id = str(argv[index + 1])
            index += 2
            continue
        if token == "--json":
            json_output = True
            index += 1
            continue
        raise AssertionError(f"Unsupported internal worktree bind argv: {argv!r}")
    if task_id is None and change_id is None:
        raise AssertionError(f"worktree bind argv missing task/change: {argv!r}")

    app_module = import_module("ait.cli.app")
    ctx = RepoContext.discover()
    payload = app_module._run_locked_workspace_command(
        ctx,
        "worktree bind",
        lambda: app_module.local_bind_worktree(
            ctx,
            name,
            task_id=task_id,
            change_id=change_id,
        ),
    )
    stdout = json.dumps(payload) + "\n" if json_output else ""
    return SimpleNamespace(exit_code=0, stdout=stdout, stderr="", output=stdout)


def _invoke_internal_worktree_promote(argv: list[str]):
    assert argv[:2] == ["worktree", "promote"], argv
    name: str | None = None
    line_name: str | None = None
    json_output = False
    index = 2
    while index < len(argv):
        token = argv[index]
        if not token.startswith("-") and name is None:
            name = str(token)
            index += 1
            continue
        if token == "--line":
            line_name = str(argv[index + 1])
            index += 2
            continue
        if token == "--json":
            json_output = True
            index += 1
            continue
        raise AssertionError(f"Unsupported internal worktree promote argv: {argv!r}")
    if line_name is None:
        raise AssertionError(f"worktree promote argv missing line: {argv!r}")

    app_module = import_module("ait.cli.app")
    ctx = RepoContext.discover()
    payload = app_module._run_locked_workspace_command(
        ctx,
        "worktree promote",
        lambda: app_module.local_promote_worktree(
            ctx,
            name,
            line_name=line_name,
        ),
    )
    stdout = json.dumps(payload) + "\n" if json_output else ""
    return SimpleNamespace(exit_code=0, stdout=stdout, stderr="", output=stdout)


def _fake_postgres_dsn(data_dir: Path) -> str:
    return f"fake-postgres:///{(data_dir / 'fake-postgres-runtime').resolve()}"


def _install_fake_postgres_driver() -> None:
    global _FAKE_POSTGRES_DRIVER_INSTALLED
    server_db_module._load_psycopg = lambda: FakePsycopg()
    server_db_module.postgres_support_installed = lambda: True
    _FAKE_POSTGRES_DRIVER_INSTALLED = True


def fake_postgres_context(data_dir: Path) -> ServerContext:
    close_postgres_connection_pools()
    reset_postgres_schema_ready_cache()
    _install_fake_postgres_driver()
    return ServerContext.create(data_dir, backend="postgres", postgres_dsn=_fake_postgres_dsn(data_dir))


def _write_plan_artifact(repo: Path, relative_path: str, markdown: str) -> str:
    target = repo / relative_path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(markdown, encoding="utf-8")
    return relative_path


def _plan_file_args(relative_path: str, plan_ref: str) -> list[str]:
    return ["--file", relative_path, "--plan-ref", plan_ref]


def _plan_artifact_payload(markdown: str, plan_ref: str, *, artifact_path: str = "docs/plans/direct.md") -> dict:
    section = extract_plan_section(markdown, plan_ref)
    assert section is not None
    return {
        "artifact_path": artifact_path,
        "artifact_selector": plan_ref,
        "artifact_heading": section["heading_title"],
        "items": section["items"],
    }


def _create_remote_plan_from_artifact(
    base_url: str,
    repo_name: str,
    markdown: str,
    plan_ref: str,
    *,
    artifact_path: str,
    title: str | None = None,
    summary: str | None = None,
    session_title: str = "Remote plan setup",
) -> dict:
    payload = _plan_artifact_payload(markdown, plan_ref, artifact_path=artifact_path)
    return remote_client_module.create_plan(
        base_url,
        repo_name,
        title or payload["artifact_heading"],
        payload["artifact_path"],
        payload["artifact_selector"],
        payload["artifact_heading"],
        payload["items"],
        summary=summary,
        source_session_id=_create_remote_session_id(title=session_title),
        artifact_body=markdown,
    )


def _revise_remote_plan_from_artifact(
    base_url: str,
    plan_id: str,
    markdown: str,
    plan_ref: str,
    *,
    artifact_path: str,
    title: str | None = None,
    summary: str | None = None,
    expected_head_revision_id: str | None = None,
    session_title: str = "Remote plan revise",
) -> dict:
    payload = _plan_artifact_payload(markdown, plan_ref, artifact_path=artifact_path)
    return remote_client_module.revise_plan(
        base_url,
        plan_id,
        payload["artifact_path"],
        payload["artifact_selector"],
        payload["artifact_heading"],
        payload["items"],
        title=title,
        summary=summary,
        source_session_id=_create_remote_session_id(title=session_title),
        artifact_body=markdown,
        expected_head_revision_id=expected_head_revision_id,
    )


def _task_graph_artifact_payload(
    *,
    repo_name: str,
    plan_id: str,
    plan_revision_id: str,
    plan_ref: str,
    plan_item_ref: str,
    artifact_path: str = "docs/plans/direct.task_graph.json",
    graph_id: str = "direct/task-graph",
    node_id: str = "A",
) -> dict:
    return {
        "artifact_path": artifact_path,
        "role": "task_graph_json",
        "media_type": "application/json",
        "body": json.dumps(
            {
                "schema_version": 1,
                "graph_id": graph_id,
                "repo_name": repo_name,
                "source_plan": {
                    "artifact_path": "docs/plans/direct.md",
                    "plan_id": plan_id,
                    "plan_ref": plan_ref,
                    "plan_revision_id": plan_revision_id,
                },
                "dispatch_artifacts": {
                    "source_markdown": "docs/plans/direct.md",
                    "parallel_execution_markdown": "docs/plans/direct.md",
                    "task_graph_json": artifact_path,
                },
                "execution_policy": {
                    "mode": "guarded_full_dag_convergence",
                    "validate_source_plan_revision": True,
                    "default_mode": "local_execution_dag_with_selective_promotion",
                    "dispatch_model": "compact_packet",
                    "worker_execution_mode": "worker_only_compact_packet",
                    "max_total_sessions": 1,
                    "max_worker_sessions": 1,
                    "max_batch_sessions": 1,
                },
                "nodes": [
                    {
                        "node_id": node_id,
                        "node_kind": "task",
                        "title": "Execute the DAG node",
                        "plan_item_ref": plan_item_ref,
                        "depends_on": [],
                        "progress_weight": 1,
                        "task_template": {
                            "title": "Execute the DAG node",
                            "change_title": "Land the DAG node",
                            "risk_tier": "medium",
                        },
                    }
                ],
                "edges": [],
            },
            sort_keys=True,
        ),
        "metadata": {
            "artifact_kind": "task_graph_json",
            "source_plan": {
                "plan_id": plan_id,
                "plan_revision_id": plan_revision_id,
                "plan_ref": plan_ref,
            },
        },
    }


@contextmanager
def running_server(data_dir: Path):
    old = os.environ.get("AIT_NATIVE_SERVER_DATA")
    old_backend = os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND")
    old_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
    old_content_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
    old_control_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
    close_postgres_connection_pools()
    reset_postgres_schema_ready_cache()
    _install_fake_postgres_driver()
    os.environ["AIT_NATIVE_SERVER_DATA"] = str(data_dir)
    os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = "postgres"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = _fake_postgres_dsn(data_dir)
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = "ait_native_content"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = "ait_native_control"
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    host, port = sock.getsockname()
    sock.close()
    app_obj = create_app()
    config = uvicorn.Config(app_obj, host=host, port=port, log_level="error")
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    base_url = f"http://{host}:{port}"
    deadline = time.time() + 10
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{base_url}/healthz", timeout=0.5) as resp:
                if resp.status == 200:
                    break
        except Exception:
            time.sleep(0.05)
    else:
        server.should_exit = True
        thread.join(timeout=5)
        raise RuntimeError("native test server did not start")
    try:
        yield base_url
    finally:
        server.should_exit = True
        thread.join(timeout=5)
        close_postgres_connection_pools()
        reset_postgres_schema_ready_cache()
        if old is None:
            os.environ.pop("AIT_NATIVE_SERVER_DATA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DATA"] = old
        if old_backend is None:
            os.environ.pop("AIT_NATIVE_SERVER_DB_BACKEND", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = old_backend
        if old_dsn is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_DSN", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = old_dsn
        if old_content_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = old_content_schema
        if old_control_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = old_control_schema


__all__ = [name for name in globals() if not name.startswith("__")]
