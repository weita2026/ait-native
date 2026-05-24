from __future__ import annotations

from pathlib import Path

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_12_server_store_facade_imports_lands_subdomain() -> None:
    server_store_text = _read_text(WORKSPACE_ROOT / "src/ait_server/server_store.py")
    lands_text = _read_text(WORKSPACE_ROOT / "src/ait_server/store/lands.py")

    assert "from .store.lands import (" in server_store_text
    assert "def create_land_request(" not in server_store_text
    assert "def _process_land(" not in server_store_text
    assert "def create_land_request(" in lands_text
    assert "def _process_land(" in lands_text


def test_wave_12_supporting_land_store_module_exists() -> None:
    assert (WORKSPACE_ROOT / "src/ait_server/store/lands.py").exists()
