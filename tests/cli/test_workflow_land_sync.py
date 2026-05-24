from __future__ import annotations

import importlib

from ait.cli import workflow_land_sync

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_workflow_land_sync_helpers() -> None:
    helper_names = [
        "_attach_local_land_sync",
        "_restore_repo_root_after_land",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(workflow_land_sync, name)
