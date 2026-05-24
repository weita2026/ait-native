import json
import importlib
import tempfile
from pathlib import Path
from types import SimpleNamespace

from ait_tk import launcher
from ait.repo_paths import RepoContext


def test_build_wish_command_default_order(tmp_path: Path):
    payload_path = tmp_path / "payload.json"
    script_path = tmp_path / "aitk.tcl"
    command = launcher.build_wish_command(payload_path, script_path, wish="wish-bin")
    assert command == ["wish-bin", str(script_path), str(payload_path)]


def test_write_payload_temp_writes_json_to_temp(tmp_path: Path):
    payload = {"hello": "world", "value": 7}
    payload_path = launcher.write_payload_temp(payload, directory=tmp_path, prefix="aitk-test-", suffix=".json")
    assert payload_path.exists()
    assert payload_path.parent == tmp_path
    assert payload_path.suffix == ".json"
    assert "aitk-test-" in payload_path.name
    assert json.loads(payload_path.read_text(encoding="utf-8")) == payload


def test_main_json_only_writes_payload_and_does_not_launch(tmp_path):
    payload = {"a": 1}
    captured: list[list[str]] = []

    def fake_builder() -> dict:
        return payload

    def fake_run(cmd: list[str]) -> int:
        captured.append(cmd)
        return 0

    script = tmp_path / "aitk.tcl"
    script.write_text("puts hi", encoding="utf-8")

    output = tmp_path / "payload.json"
    exit_code = launcher.main(
        ["--json-only", "--script", str(script), "--output", str(output)],
        payload_builder=fake_builder,
        run_command=fake_run,
    )
    assert exit_code == 0
    assert captured == []
    assert output.exists()
    assert json.loads(output.read_text(encoding="utf-8")) == payload


def test_main_no_open_writes_payload_and_skips_launch(tmp_path):
    payload = {"b": 2}
    launched: list[list[str]] = []

    def fake_builder() -> dict:
        return payload

    def fake_run(cmd: list[str]) -> int:
        launched.append(cmd)
        return 0

    script = tmp_path / "aitk.tcl"
    script.write_text("puts hi", encoding="utf-8")

    exit_code = launcher.main(["--no-open", "--script", str(script)], payload_builder=fake_builder, run_command=fake_run)
    assert exit_code == 0
    assert launched == []


def test_main_starts_wish_when_not_json_only(tmp_path, monkeypatch):
    payload = {"c": 3}
    recorded: dict[str, object] = {}
    output = tmp_path / "payload.json"
    script = tmp_path / "aitk.tcl"
    script.write_text("puts hi", encoding="utf-8")

    def fake_builder() -> dict:
        return payload

    def fake_write_payload(payload_data: dict, *, path=None, directory=None, prefix="aitk-", suffix=".json"):
        assert path == output
        assert directory is None
        assert prefix == "aitk-"
        assert suffix == ".json"
        output.write_text(json.dumps(payload_data, sort_keys=True), encoding="utf-8")
        return output

    def fake_run(cmd: list[str]) -> int:
        recorded["command"] = cmd
        return 0

    monkeypatch.setattr(launcher, "write_payload_temp", fake_write_payload)

    exit_code = launcher.main(
        ["--script", str(script), "--wish", "/usr/bin/wish", "--output", str(output)],
        payload_builder=fake_builder,
        run_command=fake_run,
    )
    assert exit_code == 0
    assert recorded["command"] == ["/usr/bin/wish", str(script), str(output)]
    assert output.exists()


def test_main_default_payload_builder_uses_fallback_when_module_missing(monkeypatch):
    original_import = importlib.import_module

    def fake_import(name: str, package: str | None = None):
        if name == "ait.aitk_export":
            raise ModuleNotFoundError("No module named 'ait.aitk_export'", name="ait.aitk_export")
        return original_import(name, package=package)

    recorded: list[Path] = []

    def fake_write(payload_data: dict, *, path=None, directory=None, prefix="aitk-", suffix=".json"):
        output = Path(path or (tmp_dir / "fallback.json"))
        output.write_text(json.dumps(payload_data), encoding="utf-8")
        recorded.append(output)
        return output

    def fake_run(cmd: list[str]) -> int:
        return 0

    monkeypatch.setattr(importlib, "import_module", fake_import)

    with tempfile.TemporaryDirectory() as work_dir:
        tmp_dir = Path(work_dir)
        monkeypatch.setattr(launcher, "write_payload_temp", fake_write)
        exit_code = launcher.main(["--script", str(tmp_dir / "aitk.tcl"), "--output", str(tmp_dir / "payload.json"), "--json-only"], run_command=fake_run)
        assert exit_code == 0
        assert len(recorded) == 1
        content = json.loads(recorded[0].read_text(encoding="utf-8"))
        assert content["status"] == "fallback"


def test_default_payload_builder_discovers_repo_context(monkeypatch):
    original_import = importlib.import_module
    fake_ctx = object()
    captured: dict[str, object] = {}

    def fake_builder(ctx):
        captured["ctx"] = ctx
        return {"ok": True}

    def fake_import(name: str, package: str | None = None):
        if name == "ait.aitk_export":
            return SimpleNamespace(build_aitk_history_payload=fake_builder)
        return original_import(name, package=package)

    monkeypatch.setattr(importlib, "import_module", fake_import)
    monkeypatch.setattr(RepoContext, "discover", staticmethod(lambda: fake_ctx))

    payload = launcher._build_payload()

    assert payload == {"ok": True}
    assert captured["ctx"] is fake_ctx


def test_default_payload_builder_uses_lazy_diffs_by_default(monkeypatch):
    original_import = importlib.import_module
    fake_ctx = object()
    captured: dict[str, object] = {}

    def fake_builder(
        ctx,
        *,
        include_snapshot_diffs=False,
        snapshot_diff_include_text=True,
        snapshot_diff_max_bytes=0,
        include_provenance=False,
    ):
        captured["ctx"] = ctx
        captured["include_snapshot_diffs"] = include_snapshot_diffs
        captured["snapshot_diff_include_text"] = snapshot_diff_include_text
        captured["snapshot_diff_max_bytes"] = snapshot_diff_max_bytes
        captured["include_provenance"] = include_provenance
        return {"ok": True}

    def fake_import(name: str, package: str | None = None):
        if name == "ait.aitk_export":
            return SimpleNamespace(build_aitk_history_payload=fake_builder)
        return original_import(name, package=package)

    monkeypatch.setattr(importlib, "import_module", fake_import)
    monkeypatch.setattr(RepoContext, "discover", staticmethod(lambda: fake_ctx))
    monkeypatch.setattr(launcher, "_ait_cli_path", lambda: "/tmp/ait")

    payload = launcher._build_payload()

    assert captured["ctx"] is fake_ctx
    assert captured["include_snapshot_diffs"] is False
    assert captured["snapshot_diff_include_text"] is True
    assert captured["snapshot_diff_max_bytes"] == launcher.DEFAULT_LAZY_DIFF_MAX_BYTES
    assert captured["include_provenance"] is True
    assert payload["diff_loader"] == {
        "kind": "ait_snapshot_diff",
        "enabled": True,
        "preloaded": False,
        "ait_cli_path": "/tmp/ait",
        "include_text": True,
        "max_bytes": launcher.DEFAULT_LAZY_DIFF_MAX_BYTES,
    }


def test_default_payload_builder_can_preload_diffs(monkeypatch):
    original_import = importlib.import_module
    fake_ctx = object()
    captured: dict[str, object] = {}

    def fake_builder(ctx, *, include_snapshot_diffs=False, snapshot_diff_include_text=True):
        captured["include_snapshot_diffs"] = include_snapshot_diffs
        captured["snapshot_diff_include_text"] = snapshot_diff_include_text
        return {"ok": True}

    def fake_import(name: str, package: str | None = None):
        if name == "ait.aitk_export":
            return SimpleNamespace(build_aitk_history_payload=fake_builder)
        return original_import(name, package=package)

    monkeypatch.setattr(importlib, "import_module", fake_import)
    monkeypatch.setattr(RepoContext, "discover", staticmethod(lambda: fake_ctx))
    monkeypatch.setattr(launcher, "_ait_cli_path", lambda: "/tmp/ait")

    payload = launcher._build_payload(preload_diffs=True, include_diff_text=False)

    assert captured["include_snapshot_diffs"] is True
    assert captured["snapshot_diff_include_text"] is False
    assert payload["diff_loader"]["enabled"] is False
    assert payload["diff_loader"]["preloaded"] is True
    assert payload["diff_loader"]["include_text"] is False
