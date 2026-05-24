from __future__ import annotations

from pathlib import Path

import ait_server.app as server_app
import ait_server.repository_routes as repository_routes


def test_server_app_uses_repository_routes_registration_module() -> None:
    assert callable(repository_routes.register_repository_routes)
    text = Path(server_app.__file__).read_text(encoding="utf-8")

    assert "from .repository_routes import register_repository_routes" in text
    assert "register_repository_routes(" in text
