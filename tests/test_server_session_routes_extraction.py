from __future__ import annotations

from pathlib import Path

import ait_server.app as server_app
import ait_server.session_routes as session_routes


def test_server_app_uses_session_routes_registration_module() -> None:
    assert callable(session_routes.register_session_routes)
    text = Path(server_app.__file__).read_text(encoding="utf-8")

    assert "from .session_routes import register_session_routes" in text
    assert "register_session_routes(" in text
