from __future__ import annotations

from ait_web.rendering.theme import DEFAULT_CSS
from ait_web.rendering.theme_chat_shell import CHAT_SHELL_CSS
from ait_web.rendering.theme_authority_views import AUTHORITY_VIEW_CSS
from ait_web.rendering.theme_repo_catalog import REPO_CATALOG_CSS


def test_theme_chat_shell_css_is_composed_into_default_css() -> None:
    assert CHAT_SHELL_CSS in DEFAULT_CSS
    assert ".shared-chat-panel" in CHAT_SHELL_CSS
    assert ".role-lens-list" in CHAT_SHELL_CSS
    assert DEFAULT_CSS.index(CHAT_SHELL_CSS) < DEFAULT_CSS.index(REPO_CATALOG_CSS)


def test_theme_repo_catalog_css_is_composed_into_default_css() -> None:
    assert REPO_CATALOG_CSS in DEFAULT_CSS
    assert ".repo-catalog-shell" in REPO_CATALOG_CSS
    assert DEFAULT_CSS.index(REPO_CATALOG_CSS) < DEFAULT_CSS.index(AUTHORITY_VIEW_CSS)
