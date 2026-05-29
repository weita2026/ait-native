from __future__ import annotations

from pathlib import Path

from ait import store, store_content_ops
from ait.cli.commands import queue as queue_command
from ait.cli.commands import queue_workflow_land
from ait_agent.discord import clients as discord_clients
from ait_agent.discord.app import AitApiClient, DiscordApiClient
from ait_server.app import create_app
from ait_web.rendering.theme import DEFAULT_CSS
from ait_web.rendering.theme_authority_views import AUTHORITY_VIEW_CSS
from tests.postgres_fake import fake_postgres_context, fake_postgres_dsn


def test_theme_authority_views_are_composed_into_default_css() -> None:
    assert AUTHORITY_VIEW_CSS in DEFAULT_CSS
    assert ".authority-tree-shell" in AUTHORITY_VIEW_CSS



def test_queue_workflow_land_helpers_stay_reexported() -> None:
    assert queue_command._workflow_land_payload is queue_workflow_land._workflow_land_payload
    assert queue_command._workflow_land_apply is queue_workflow_land._workflow_land_apply



def test_server_read_routes_remain_registered(tmp_path: Path, monkeypatch) -> None:
    data_dir = tmp_path / "server-data"
    fake_postgres_context(data_dir)
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_DSN", fake_postgres_dsn(data_dir))
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content")
    monkeypatch.setenv("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control")

    app = create_app()
    paths = {route.path for route in app.routes}
    assert "/v1/native/read/queue-summary" in paths
    assert "/v1/native/read/reviewer-inbox" in paths
    assert "/v1/native/read/task-dag-progress" in paths
    assert "/v1/native/read/stacks/{stack_id}" in paths



def test_discord_client_helpers_stay_reexported() -> None:
    assert DiscordApiClient is discord_clients.DiscordApiClient
    assert AitApiClient is discord_clients.AitApiClient



def test_store_content_ops_stay_reexported() -> None:
    assert store.optimize_content is store_content_ops.optimize_content
    assert store.import_snapshot_bundle is store_content_ops.import_snapshot_bundle
