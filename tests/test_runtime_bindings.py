from __future__ import annotations

import importlib.util
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
RUNTIME_BINDINGS_PATH = REPO_ROOT / "src/ait_agent/runtime_bindings.py"
RUNTIME_BINDINGS_MODULE_NAME = "ait_agent_runtime_bindings_under_test"


def _runtime_bindings_module():
    spec = importlib.util.spec_from_file_location(
        RUNTIME_BINDINGS_MODULE_NAME,
        RUNTIME_BINDINGS_PATH,
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_runtime_binding_store_round_trip_tracks_canonical_and_branch_sessions(tmp_path: Path):
    runtime_bindings = _runtime_bindings_module()
    store = runtime_bindings.RuntimeSurfaceBindingStore(tmp_path / "telegram-sync.json")

    primary = store.upsert_binding(
        transport="telegram",
        surface_id="123",
        repo_name="ait",
        surface_title="Wei",
        surface_kind="private",
        session_id="AITS-0001",
        canonical_session_id="AITS-0001",
        binding_role="primary_shared",
        last_synced_sequence=4,
    )
    assert primary["canonical_session_id"] == "AITS-0001"
    assert primary["binding_role"] == "primary_shared"

    branch = store.upsert_binding(
        transport="telegram",
        surface_id="123",
        repo_name="ait",
        surface_title="Wei",
        surface_kind="private",
        session_id="AITS-0002",
        canonical_session_id="AITS-0001",
        branch_session_id="AITS-0002",
        binding_role="branch",
        updates={"branch_kind": "planning", "relink_reason": "planning_mode_trigger"},
    )
    assert branch["session_id"] == "AITS-0002"
    assert branch["canonical_session_id"] == "AITS-0001"
    assert branch["branch_session_id"] == "AITS-0002"
    assert branch["branch_kind"] == "planning"

    chat_link = store.load().chats["123"]
    assert chat_link["session_id"] == "AITS-0002"
    assert chat_link["canonical_session_id"] == "AITS-0001"
    assert chat_link["branch_session_id"] == "AITS-0002"


def test_runtime_binding_store_reports_ambiguous_repo_binding(tmp_path: Path):
    runtime_bindings = _runtime_bindings_module()
    store = runtime_bindings.RuntimeSurfaceBindingStore(tmp_path / "telegram-sync.json")
    store.upsert_binding(
        transport="telegram",
        surface_id="123",
        repo_name="ait",
        session_id="AITS-0001",
        canonical_session_id="AITS-0001",
    )
    store.upsert_binding(
        transport="slack",
        surface_id="C123",
        repo_name="ait",
        session_id="AITS-0002",
        canonical_session_id="AITS-0002",
    )

    resolved = store.resolve_repo_shared_binding("ait")
    assert resolved["status"] == "ambiguous"
    assert resolved["canonical_session_ids"] == ["AITS-0001", "AITS-0002"]


def test_runtime_binding_state_path_uses_runtime_root_without_server_backend_env(
    tmp_path: Path,
    monkeypatch,
):
    runtime_bindings = _runtime_bindings_module()
    runtime_root = tmp_path / "server-runtime"
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(runtime_root))
    monkeypatch.delenv("AIT_NATIVE_SERVER_DB_BACKEND", raising=False)
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_DSN", raising=False)

    assert runtime_bindings.resolve_runtime_binding_state_path() == runtime_root / "telegram-sync.json"


def test_runtime_binding_store_preserves_explicit_empty_graph_watches(tmp_path: Path):
    runtime_bindings = _runtime_bindings_module()
    store = runtime_bindings.RuntimeSurfaceBindingStore(tmp_path / "telegram-sync.json")

    seeded = store.upsert_binding(
        transport="telegram",
        surface_id="123",
        repo_name="ait",
        session_id="AITS-0001",
        canonical_session_id="AITS-0001",
        updates={
            "graph_watches": {
                "PL-TEST123": {
                    "plan_id": "PL-TEST123",
                    "graph_path": "docs/sprints/demo.task_graph.json",
                }
            }
        },
    )
    assert seeded["graph_watches"]["PL-TEST123"]["plan_id"] == "PL-TEST123"

    cleared = store.upsert_binding(
        transport="telegram",
        surface_id="123",
        repo_name="ait",
        session_id="AITS-0001",
        canonical_session_id="AITS-0001",
        updates={"graph_watches": {}},
    )

    assert "graph_watches" in cleared
    assert cleared["graph_watches"] == {}

    state = store.load()
    binding = state.surface_bindings["telegram:123"]
    chat_link = state.chats["123"]
    assert "graph_watches" in binding
    assert binding["graph_watches"] == {}
    assert "graph_watches" in chat_link
    assert chat_link["graph_watches"] == {}
