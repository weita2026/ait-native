from __future__ import annotations

import importlib
from pathlib import Path
from types import SimpleNamespace

from ait.cli import task_dag_telegram_watch, task_tracking_bindings
from ait.repo_paths import RepoContext

from ._shared import app, runner

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_task_dag_telegram_watch_helpers() -> None:
    helper_names = [
        "_maybe_auto_register_task_dag_telegram_watch",
        "_trigger_task_dag_execute_run_telegram_notifications",
        "_task_dag_telegram_auto_watch_enabled",
        "_task_dag_telegram_watch_session_hint",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(task_dag_telegram_watch, name)


def test_task_dag_telegram_watch_helper_contract(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-dag-telegram-watch"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    ctx = RepoContext.discover()
    assert task_dag_telegram_watch._task_dag_telegram_auto_watch_enabled() is True
    assert task_dag_telegram_watch._task_dag_telegram_watch_session_hint(ctx) is None

    monkeypatch.setenv("AIT_TASK_DAG_AUTO_WATCH_TELEGRAM", "off")
    assert task_dag_telegram_watch._task_dag_telegram_auto_watch_enabled() is False
    monkeypatch.delenv("AIT_TASK_DAG_AUTO_WATCH_TELEGRAM", raising=False)

    task_tracking_bindings._set_tracked_session_binding(
        ctx,
        task_id="RT-REMOTE-1",
        session_id="S-TRACKED-1",
        scope="remote",
        remote_name="origin",
    )
    assert task_dag_telegram_watch._task_dag_telegram_watch_session_hint(ctx) == "S-TRACKED-1"

    monkeypatch.setenv("AIT_SESSION_ID", "S-EXPLICIT-1")
    assert task_dag_telegram_watch._task_dag_telegram_watch_session_hint(ctx) == "S-EXPLICIT-1"

    captured: dict[str, object] = {}

    import ait_agent.telegram.graph_watches as telegram_graph_watches
    import ait_agent.telegram.worker_config as telegram_worker_config_module

    monkeypatch.setattr(
        telegram_worker_config_module,
        "load_config_for_telegram_worker",
        lambda repo_root, name=None: SimpleNamespace(repo_name="housekeeper"),
    )
    def fake_auto_register_graph_watch(_config, **kwargs):
        captured["kwargs"] = kwargs
        return {"registered": True}

    monkeypatch.setattr(
        telegram_graph_watches,
        "auto_register_graph_watch",
        fake_auto_register_graph_watch,
    )
    monkeypatch.setattr(task_dag_telegram_watch, "_effective_workflow_mode", lambda _ctx: {"value": "solo_local"})
    monkeypatch.setattr(
        task_dag_telegram_watch,
        "_local_task_dag_progress_payload",
        lambda current_ctx, graph: {"progress": {"mode": "local", "graph_id": graph.get("graph_id")}},
    )

    payload = task_dag_telegram_watch._maybe_auto_register_task_dag_telegram_watch(
        ctx=ctx,
        remote_row=None,
        repo_name=None,
        plan_id="PL-demo",
        graph_artifact_path="docs/sprints/demo.task_graph.json",
    )
    assert payload["enabled"] is True
    assert payload["registered"] is True
    assert payload["progress_reader_mode"] == "local"
    assert payload["repo_name"] == "housekeeper"
    progress = captured["kwargs"]["progress_reader"]({"graph_id": "graph-local"})
    assert progress["progress"]["mode"] == "local"
    assert progress["progress"]["graph_id"] == "graph-local"


def test_task_dag_telegram_watch_helper_uses_remote_progress_for_solo_remote(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-dag-telegram-watch-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    ctx = RepoContext.discover()

    captured: dict[str, object] = {}

    import ait_agent.telegram.graph_watches as telegram_graph_watches
    import ait_agent.telegram.worker_config as telegram_worker_config_module

    monkeypatch.setattr(
        telegram_worker_config_module,
        "load_config_for_telegram_worker",
        lambda repo_root, name=None: SimpleNamespace(repo_name="housekeeper"),
    )
    def fake_auto_register_graph_watch(_config, **kwargs):
        captured["kwargs"] = kwargs
        return {"registered": True}

    monkeypatch.setattr(
        telegram_graph_watches,
        "auto_register_graph_watch",
        fake_auto_register_graph_watch,
    )
    monkeypatch.setattr(task_dag_telegram_watch, "_effective_workflow_mode", lambda _ctx: {"value": "solo_remote"})
    monkeypatch.setattr(
        task_dag_telegram_watch,
        "remote_read_task_dag_progress",
        lambda base_url, graph: {"progress": {"mode": "remote", "graph_id": graph.get("graph_id"), "base_url": base_url}},
    )

    payload = task_dag_telegram_watch._maybe_auto_register_task_dag_telegram_watch(
        ctx=ctx,
        remote_row={"url": "http://example.test"},
        repo_name="housekeeper",
        plan_id="PL-demo",
        graph_artifact_path="docs/sprints/demo.task_graph.json",
    )

    assert payload["enabled"] is True
    assert payload["registered"] is True
    assert payload["progress_reader_mode"] == "remote"
    progress = captured["kwargs"]["progress_reader"]({"graph_id": "graph-remote"})
    assert progress["progress"]["mode"] == "remote"
    assert progress["progress"]["base_url"] == "http://example.test"


def test_task_dag_telegram_watch_helper_falls_back_to_local_when_remote_plan_is_unknown(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-dag-telegram-watch-fallback"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    ctx = RepoContext.discover()

    captured: dict[str, object] = {}

    import ait_agent.telegram.graph_watches as telegram_graph_watches
    import ait_agent.telegram.worker_config as telegram_worker_config_module

    monkeypatch.setattr(
        telegram_worker_config_module,
        "load_config_for_telegram_worker",
        lambda repo_root, name=None: SimpleNamespace(repo_name="housekeeper"),
    )
    def fake_auto_register_graph_watch(_config, **kwargs):
        captured["kwargs"] = kwargs
        return {"registered": True}

    monkeypatch.setattr(
        telegram_graph_watches,
        "auto_register_graph_watch",
        fake_auto_register_graph_watch,
    )
    monkeypatch.setattr(task_dag_telegram_watch, "_effective_workflow_mode", lambda _ctx: {"value": "solo_remote"})
    monkeypatch.setattr(
        task_dag_telegram_watch,
        "_local_task_dag_progress_payload",
        lambda current_ctx, graph: {"progress": {"mode": "local", "graph_id": graph.get("graph_id")}},
    )

    def _raise_unknown_plan(_base_url: str, _graph: dict[str, object]) -> dict[str, object]:
        raise RuntimeError("404 Unknown plan: PL-DEMO")

    monkeypatch.setattr(task_dag_telegram_watch, "remote_read_task_dag_progress", _raise_unknown_plan)

    payload = task_dag_telegram_watch._maybe_auto_register_task_dag_telegram_watch(
        ctx=ctx,
        remote_row={"url": "http://example.test"},
        repo_name="housekeeper",
        plan_id="PL-demo",
        graph_artifact_path="docs/sprints/demo.task_graph.json",
    )

    assert payload["enabled"] is True
    assert payload["registered"] is True
    assert payload["progress_reader_mode"] == "remote"
    progress = captured["kwargs"]["progress_reader"]({"graph_id": "graph-fallback"})
    assert progress["progress"]["mode"] == "local"
    assert progress["progress"]["graph_id"] == "graph-fallback"


def test_trigger_task_dag_execute_run_telegram_notifications_uses_local_progress_for_solo_local(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-dag-telegram-trigger-local"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    ctx = RepoContext.discover()

    import ait_agent.telegram.graph_watches as telegram_graph_watches
    import ait_agent.telegram.worker_config as telegram_worker_config_module

    monkeypatch.setattr(
        telegram_worker_config_module,
        "load_config_for_telegram_worker",
        lambda repo_root, name=None: SimpleNamespace(repo_name="housekeeper"),
    )
    monkeypatch.setattr(task_dag_telegram_watch, "_effective_workflow_mode", lambda _ctx: {"value": "solo_local"})
    monkeypatch.setattr(
        task_dag_telegram_watch,
        "_local_task_dag_progress_payload",
        lambda current_ctx, graph: {"progress": {"mode": "local", "graph_id": graph.get("graph_id")}},
    )

    captured: dict[str, object] = {}

    def fake_trigger(_config, *, repo_name, plan_ids, progress_reader):
        captured["repo_name"] = repo_name
        captured["plan_ids"] = set(plan_ids or [])
        captured["payload"] = progress_reader({"graph_id": "graph-local"})
        return {"checked": 1, "sent": 1, "errors": 0}

    monkeypatch.setattr(telegram_graph_watches, "trigger_graph_watch_notifications", fake_trigger)

    payload = task_dag_telegram_watch._trigger_task_dag_execute_run_telegram_notifications(
        ctx,
        remote_row=None,
        repo_name="housekeeper",
        plan_id="PL-demo",
    )

    assert payload["enabled"] is True
    assert payload["progress_reader_mode"] == "local"
    assert payload["checked"] == 1
    assert payload["sent"] == 1
    assert payload["errors"] == 0
    assert captured["repo_name"] == "housekeeper"
    assert captured["plan_ids"] == {"PL-demo"}
    assert captured["payload"]["progress"]["mode"] == "local"


def test_trigger_task_dag_execute_run_telegram_notifications_uses_remote_progress_for_solo_remote(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-task-dag-telegram-trigger-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    ctx = RepoContext.discover()

    import ait_agent.telegram.graph_watches as telegram_graph_watches
    import ait_agent.telegram.worker_config as telegram_worker_config_module

    monkeypatch.setattr(
        telegram_worker_config_module,
        "load_config_for_telegram_worker",
        lambda repo_root, name=None: SimpleNamespace(repo_name="housekeeper"),
    )
    monkeypatch.setattr(task_dag_telegram_watch, "_effective_workflow_mode", lambda _ctx: {"value": "solo_remote"})
    monkeypatch.setattr(
        task_dag_telegram_watch,
        "remote_read_task_dag_progress",
        lambda base_url, graph: {
            "progress": {"mode": "remote", "base_url": base_url, "graph_id": graph.get("graph_id")}
        },
    )

    captured: dict[str, object] = {}

    def fake_trigger(_config, *, repo_name, plan_ids, progress_reader):
        captured["repo_name"] = repo_name
        captured["plan_ids"] = set(plan_ids or [])
        captured["payload"] = progress_reader({"graph_id": "graph-remote"})
        return {"checked": 1, "sent": 1, "errors": 0}

    monkeypatch.setattr(telegram_graph_watches, "trigger_graph_watch_notifications", fake_trigger)

    payload = task_dag_telegram_watch._trigger_task_dag_execute_run_telegram_notifications(
        ctx,
        remote_row={"url": "http://example.test"},
        repo_name="housekeeper",
        plan_id="PL-demo",
    )

    assert payload["enabled"] is True
    assert payload["progress_reader_mode"] == "remote"
    assert payload["checked"] == 1
    assert payload["sent"] == 1
    assert payload["errors"] == 0
    assert captured["repo_name"] == "housekeeper"
    assert captured["plan_ids"] == {"PL-demo"}
    assert captured["payload"]["progress"]["mode"] == "remote"
    assert captured["payload"]["progress"]["base_url"] == "http://example.test"
