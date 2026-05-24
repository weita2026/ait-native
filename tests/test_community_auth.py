from __future__ import annotations

from pathlib import Path

import pytest

from ait_server.community_auth import (
    authenticate_community_account,
    create_community_account,
    resolve_community_web_session,
    revoke_community_web_session,
)
from ait_server.server_paths import ServerContext
from ait_server.server_store import initialize
from tests.ait_web.helpers import _configure_web_runtime


def _community_ctx(data_dir: Path) -> ServerContext:
    _configure_web_runtime(data_dir)
    ctx = ServerContext.from_env()
    initialize(ctx)
    return ctx


def test_create_community_account_is_casefold_unique(tmp_path: Path) -> None:
    ctx = _community_ctx(tmp_path / "community-auth-create")

    session = create_community_account(
        ctx,
        full_name="Community User",
        email="Community@Example.com",
        password="correct horse battery",
        organization="Open Community",
        role_title="tester",
    )
    assert session["account_id"].startswith("CA-")
    assert session["web_session_id"].startswith("CWS-")
    assert session["actor_identity"].startswith("community:CA-")
    assert session["email_normalized"] == "community@example.com"

    with pytest.raises(ValueError, match="already exists"):
        create_community_account(
            ctx,
            full_name="Second User",
            email="community@example.com",
            password="correct horse battery",
        )


def test_authenticate_and_revoke_community_session(tmp_path: Path) -> None:
    ctx = _community_ctx(tmp_path / "community-auth-session")

    created = create_community_account(
        ctx,
        full_name="Community User",
        email="community@example.com",
        password="correct horse battery",
    )
    assert resolve_community_web_session(ctx, created["web_session_id"]) is not None

    with pytest.raises(ValueError, match="Incorrect email or password"):
        authenticate_community_account(ctx, email="community@example.com", password="wrong wrong wrong")

    signed_in = authenticate_community_account(
        ctx,
        email="community@example.com",
        password="correct horse battery",
    )
    assert signed_in["account_id"] == created["account_id"]
    assert signed_in["web_session_id"] != created["web_session_id"]
    assert resolve_community_web_session(ctx, signed_in["web_session_id"]) is not None

    revoke_community_web_session(ctx, signed_in["web_session_id"])
    assert resolve_community_web_session(ctx, signed_in["web_session_id"]) is None
