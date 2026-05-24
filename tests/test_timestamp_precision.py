from __future__ import annotations

import importlib
import sys
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ait import store
from ait_protocol.common import utc_now

cli_module = importlib.import_module("ait.cli.app")
local_content_module = importlib.import_module("ait.local_content")
server_content_module = importlib.import_module("ait_server.server_content")


def test_utc_now_uses_second_precision() -> None:
    stamp = utc_now()

    assert "." not in stamp
    parsed = datetime.fromisoformat(stamp)
    assert parsed.microsecond == 0
    assert parsed.tzinfo == timezone.utc


def test_task_dag_graph_run_id_stays_unique_with_fixed_second(monkeypatch) -> None:
    counter = iter((1, 2))
    monkeypatch.setattr(cli_module, "utc_now", lambda: "2026-05-12T00:00:00+00:00")
    monkeypatch.setattr(cli_module.time, "time_ns", lambda: next(counter))

    first = cli_module._task_dag_graph_run_id("PL-demo", "graph-demo")
    second = cli_module._task_dag_graph_run_id("PL-demo", "graph-demo")

    assert first.startswith("graph-run-")
    assert second.startswith("graph-run-")
    assert first != second


def test_repository_group_ids_stay_unique_with_fixed_second(monkeypatch) -> None:
    values = iter(("first", "second"))
    monkeypatch.setattr(server_content_module, "utc_now", lambda: "2026-05-12T00:00:00+00:00")
    monkeypatch.setattr(server_content_module.uuid, "uuid4", lambda: SimpleNamespace(hex=next(values)))

    first = server_content_module._new_repository_group_id("group")
    second = server_content_module._new_repository_group_id("group")

    assert first.startswith("RPG-")
    assert second.startswith("RPG-")
    assert first != second


def test_local_repack_ids_stay_unique_with_fixed_second(monkeypatch, tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    ctx = store.init_repo(repo, "repo", "main")
    (repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    local_content_module.create_snapshot(ctx, "repo", "main", "seed")

    values = iter(("first", "second"))
    monkeypatch.setattr(local_content_module, "utc_now", lambda: "2026-05-12T00:00:00+00:00")
    monkeypatch.setattr(local_content_module.secrets, "token_hex", lambda _n=8: next(values))

    first = local_content_module.create_pack(ctx, repack=True)
    second = local_content_module.create_pack(ctx, repack=True)

    assert first["created"] is True
    assert second["created"] is True
    assert first["pack_id"].startswith("PCK-")
    assert second["pack_id"].startswith("PCK-")
    assert first["pack_id"] != second["pack_id"]
