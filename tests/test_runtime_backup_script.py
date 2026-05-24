from __future__ import annotations

import importlib.util
import json
import sys
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "runtime_backup.py"
SPEC = importlib.util.spec_from_file_location("runtime_backup", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
runtime_backup = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runtime_backup
SPEC.loader.exec_module(runtime_backup)


def _seed_runtime_root(path: Path) -> None:
    (path / "objects").mkdir(parents=True)
    (path / "refs").mkdir(parents=True)
    (path / "objects" / "blob.txt").write_text("blob-data\n", encoding="utf-8")
    (path / "refs" / "main").write_text("SNP-123\n", encoding="utf-8")
    (path / "telegram-sync.json").write_text('{"chat_id": 1}\n', encoding="utf-8")
    (path / "control.db").write_text("sqlite-control\n", encoding="utf-8")


def test_runtime_backup_script_creates_backup_sets_and_prunes_old_copies(tmp_path: Path, monkeypatch) -> None:
    runtime_root = tmp_path / "server-data"
    output_dir = tmp_path / "backups"
    runtime_root.mkdir()
    _seed_runtime_root(runtime_root)
    monkeypatch.delenv("AIT_NATIVE_SERVER_DB_BACKEND", raising=False)
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_DSN", raising=False)

    first = runtime_backup.create_backup_set(
        runtime_root,
        output_dir,
        keep=2,
        now=datetime(2026, 4, 27, 2, 0, tzinfo=timezone.utc),
    )
    second = runtime_backup.create_backup_set(
        runtime_root,
        output_dir,
        keep=2,
        now=datetime(2026, 4, 28, 2, 0, tzinfo=timezone.utc),
    )
    third = runtime_backup.create_backup_set(
        runtime_root,
        output_dir,
        keep=2,
        now=datetime(2026, 4, 29, 2, 0, tzinfo=timezone.utc),
    )

    backup_dirs = sorted(path.name for path in output_dir.iterdir() if path.is_dir())
    assert backup_dirs == [
        "ait-backup-20260428T020000Z",
        "ait-backup-20260429T020000Z",
    ]
    assert third["pruned_backup_paths"] == [str(output_dir / "ait-backup-20260427T020000Z")]

    manifest = json.loads(Path(second["manifest_path"]).read_text(encoding="utf-8"))
    assert manifest["backend"] == "sqlite"
    assert manifest["keep_count"] == 2
    assert manifest["runtime_copy_path"] == "runtime-root"

    copied_blob = Path(third["backup_path"]) / "runtime-root" / "objects" / "blob.txt"
    copied_ref = Path(third["backup_path"]) / "runtime-root" / "refs" / "main"
    copied_sync = Path(third["backup_path"]) / "runtime-root" / "telegram-sync.json"
    assert copied_blob.read_text(encoding="utf-8") == "blob-data\n"
    assert copied_ref.read_text(encoding="utf-8") == "SNP-123\n"
    assert copied_sync.read_text(encoding="utf-8") == '{"chat_id": 1}\n'
    assert first["pruned_backup_paths"] == []


def test_runtime_backup_script_dumps_postgres_schemas_when_requested(tmp_path: Path, monkeypatch) -> None:
    runtime_root = tmp_path / "server-data"
    output_dir = tmp_path / "backups"
    runtime_root.mkdir()
    _seed_runtime_root(runtime_root)
    calls: list[list[str]] = []

    def fake_run(command, *, check, capture_output, text):
        assert check is True
        assert capture_output is True
        assert text is True
        calls.append(list(command))
        file_path = Path(command[command.index("--file") + 1])
        schema = command[command.index("--schema") + 1]
        file_path.write_text(f"-- dump for {schema}\n", encoding="utf-8")
        return object()

    monkeypatch.setattr(runtime_backup.subprocess, "run", fake_run)

    payload = runtime_backup.create_backup_set(
        runtime_root,
        output_dir,
        backend="postgres",
        postgres_dsn="postgresql://ait:secret@db.example:5432/ait_native",
        content_schema="content_schema",
        control_schema="control_schema",
        keep=8,
        now=datetime(2026, 4, 27, 3, 0, tzinfo=timezone.utc),
    )

    assert len(calls) == 2
    assert [row["schema"] for row in payload["postgres_dumps"]] == ["content_schema", "control_schema"]
    for row in payload["postgres_dumps"]:
        dump_path = Path(row["path"])
        assert dump_path.exists()
        assert dump_path.read_text(encoding="utf-8").startswith("-- dump for ")
