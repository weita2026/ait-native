from __future__ import annotations

from pathlib import Path

import ait_server.app as server_app
import ait_server.task_routes as task_routes


def test_server_app_uses_task_routes_registration_module() -> None:
    assert callable(task_routes.register_task_routes)
    text = Path(server_app.__file__).read_text(encoding="utf-8")

    assert "from .task_routes import register_task_routes" in text
    assert "register_task_routes(" in text
