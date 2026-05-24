from __future__ import annotations

from pathlib import Path

import ait_server.app as server_app
import ait_server.stack_routes as stack_routes


def test_server_app_uses_stack_routes_registration_module() -> None:
    assert callable(stack_routes.register_stack_routes)
    text = Path(server_app.__file__).read_text(encoding="utf-8")

    assert "from .stack_routes import register_stack_routes" in text
    assert "register_stack_routes(" in text
