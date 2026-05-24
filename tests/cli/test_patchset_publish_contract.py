from __future__ import annotations

import ait.cli.commands.patchset as patchset_module

from ._shared import *  # noqa: F401,F403


def test_patchset_publish_help_keeps_change_first_snapshot_inference_contract():
    help_out = runner.invoke(app, ["patchset", "publish", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "--change" in help_out.stdout
    assert "--summary" in help_out.stdout
    assert "--author-mode" in help_out.stdout
    assert "--allow-empty" in help_out.stdout
    assert "--remote" in help_out.stdout
    assert "--json" in help_out.stdout
    assert "--base-snapshot" not in help_out.stdout
    assert "--revision-snapshot" not in help_out.stdout
    assert "--base" not in help_out.stdout
    assert "--revision" not in help_out.stdout


def test_patchset_publish_command_calls_current_line_publish_helper(monkeypatch):
    monkeypatch.setattr(patchset_module, "_ctx", lambda: object())
    monkeypatch.setattr(
        patchset_module,
        "_run_locked_task_bound_authoring_command",
        lambda ctx, command_name, operation: operation(),
    )
    monkeypatch.setattr(
        patchset_module,
        "_publish_patchset_from_current_line",
        lambda ctx, **kwargs: {
            "patchset_id": "P-1",
            "change_id": kwargs["change_id"],
            "summary": kwargs["summary"],
        },
    )
    monkeypatch.setattr(patchset_module, "_touch_worktree_usage_safely", lambda ctx: None)

    result = runner.invoke(
        app,
        ["patchset", "publish", "--change", "LC-1", "--summary", "review summary", "--json"],
        catch_exceptions=False,
    )

    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["patchset_id"] == "P-1"
    assert payload["change_id"] == "LC-1"
    assert payload["summary"] == "review summary"
