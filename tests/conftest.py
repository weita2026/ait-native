from __future__ import annotations

from contextlib import ExitStack
import importlib
import sys
from pathlib import Path

import pytest

from tests._ram_root import detect_host_ram_root, managed_host_ram_root
from tests.postgres_fake import restore_real_psycopg

ROOT = Path(__file__).resolve().parents[1]
SRC = str(ROOT / "src")

while SRC in sys.path:
    sys.path.remove(SRC)
sys.path.insert(0, SRC)


@pytest.fixture(autouse=True)
def restore_postgres_driver_after_test():
    yield
    restore_real_psycopg()


@pytest.fixture(autouse=True)
def disable_host_memory_root_detection_for_init(monkeypatch):
    layout = importlib.import_module("ait.task_worktree_layout")
    store_modules = [importlib.import_module("ait.store")]
    try:
        store_modules.append(importlib.import_module("ait_native.store"))
    except ModuleNotFoundError:
        pass

    original_detect = layout.detect_init_task_worktree_defaults
    original_macos = layout._macos_ram_volume_roots
    original_macos_specs = layout._macos_ram_volume_specs
    original_linux = layout._linux_detected_memory_roots
    original_windows = layout._windows_ram_disk_roots

    def isolated_defaults(ctx):
        with ExitStack() as stack:
            if layout._macos_ram_volume_roots is original_macos:
                stack.callback(setattr, layout, "_macos_ram_volume_roots", layout._macos_ram_volume_roots)
                layout._macos_ram_volume_roots = lambda: []
            if layout._macos_ram_volume_specs is original_macos_specs:
                stack.callback(setattr, layout, "_macos_ram_volume_specs", layout._macos_ram_volume_specs)
                layout._macos_ram_volume_specs = lambda: []
            if layout._linux_detected_memory_roots is original_linux:
                stack.callback(setattr, layout, "_linux_detected_memory_roots", layout._linux_detected_memory_roots)
                layout._linux_detected_memory_roots = lambda: []
            if layout._windows_ram_disk_roots is original_windows:
                stack.callback(setattr, layout, "_windows_ram_disk_roots", layout._windows_ram_disk_roots)
                layout._windows_ram_disk_roots = lambda: []
            return original_detect(ctx)

    for module in store_modules:
        monkeypatch.setattr(module, "detect_init_task_worktree_defaults", isolated_defaults)


@pytest.fixture
def host_ram_root() -> Path:
    root = detect_host_ram_root()
    if root is None:
        pytest.skip("No host memory-backed root is available on this machine.")
    with managed_host_ram_root(root):
        yield root
