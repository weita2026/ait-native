from __future__ import annotations

from ait.cli.commands import bootstrap


def _capture_imports(monkeypatch):
    imported: list[str] = []

    def fake_import_module(name: str, package: str | None = None):
        imported.append(name.lstrip("."))
        return object()

    monkeypatch.setattr(bootstrap.importlib, "import_module", fake_import_module)
    monkeypatch.setattr(bootstrap, "_IMPORTED_MODULES", set())
    return imported


def test_bootstrap_cli_commands_imports_only_status_module(monkeypatch):
    imported = _capture_imports(monkeypatch)

    bootstrap.bootstrap_cli_commands(["status", "--json"])

    assert imported == ["config"]


def test_bootstrap_cli_commands_imports_plan_and_plan_session(monkeypatch):
    imported = _capture_imports(monkeypatch)

    bootstrap.bootstrap_cli_commands(["plan", "session", "create"])

    assert imported == ["plan", "plan_session"]


def test_bootstrap_cli_commands_imports_workflow_helpers(monkeypatch):
    imported = _capture_imports(monkeypatch)

    bootstrap.bootstrap_cli_commands(["workflow", "land", "LC-1"])

    assert imported == ["queue_workflow_land", "workflow"]


def test_bootstrap_cli_commands_falls_back_to_full_bootstrap_for_unknown_command(monkeypatch):
    imported = _capture_imports(monkeypatch)

    bootstrap.bootstrap_cli_commands(["unknown-command"])

    assert imported == list(bootstrap._COMMAND_MODULES)


def test_bootstrap_cli_commands_imports_only_new_modules_across_calls(monkeypatch):
    imported = _capture_imports(monkeypatch)

    bootstrap.bootstrap_cli_commands(["status"])
    bootstrap.bootstrap_cli_commands(["task", "show"])
    bootstrap.bootstrap_cli_commands(["status", "--json"])

    assert imported == ["config", "task"]
