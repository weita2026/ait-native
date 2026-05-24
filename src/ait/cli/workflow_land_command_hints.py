from __future__ import annotations

from pathlib import Path
from typing import Any

from ait_protocol.common import (
    CODE_REVIEW_SUMMARY_TEMPLATE,
    CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND,
    code_review_summary_requirement_text,
)

from ..store import RepoContext


def _local_patchset_ci_contract_exists(ctx: RepoContext) -> bool:
    suite_dir = Path(ctx.root) / "ci" / "suites"
    try:
        return suite_dir.is_dir() and any(path.suffix == ".json" for path in suite_dir.iterdir())
    except OSError:
        return False


def _workflow_land_publish_command(
    *,
    change_id: str,
    base_line_name: str,
    worktree_retarget: dict[str, Any] | None,
) -> str:
    publish_command = f'ait patchset publish --change {change_id} --summary "review summary"'
    if not isinstance(worktree_retarget, dict):
        return publish_command
    if str(worktree_retarget.get("rebase_state") or "idle") == "conflicted":
        return "ait worktree rebase --continue"
    if bool(worktree_retarget.get("needs_retarget")):
        return f"ait worktree rebase --onto {base_line_name}"
    return publish_command


def _workflow_land_command_hints(
    ctx: RepoContext,
    *,
    change_id: str,
    task_id: str,
    patchset: dict[str, Any] | None,
    base_line_name: str,
    target_line: str,
    worktree_retarget: dict[str, Any] | None,
    review_blocking: int,
    requires_code_review_summary: bool,
) -> dict[str, str | None]:
    patchset_id = str((patchset or {}).get("patchset_id") or "").strip() or None
    publish_command = _workflow_land_publish_command(
        change_id=change_id,
        base_line_name=base_line_name,
        worktree_retarget=worktree_retarget,
    )
    patchset_ci_command = (
        f"ait patchset rerun-ci {patchset_id}"
        if patchset_id is not None and _local_patchset_ci_contract_exists(ctx)
        else None
    )
    attest_command = f"ait attest put {patchset_id} --tests pass" if patchset_id is not None else None
    code_review_summary_command = (
        f'ait review code submit {change_id} --patchset {patchset_id} --verdict pass --message "{CODE_REVIEW_SUMMARY_TEMPLATE}"'
        if patchset_id is not None
        else None
    )
    review_command = (
        f"ait review show {change_id}"
        if review_blocking > 0
        else f"ait review task approve {change_id}" + (f" --patchset {patchset_id}" if patchset_id is not None else "")
    )
    land_command = f"ait land submit {change_id}" + (f" --patchset {patchset_id}" if patchset_id is not None else "") + f" --target {target_line} --mode direct"
    return {
        "publish_command": publish_command,
        "patchset_ci_command": patchset_ci_command,
        "attest_command": attest_command,
        "attestation_command": patchset_ci_command or attest_command,
        "code_review_summary_command": code_review_summary_command,
        "code_review_template_command": (
            CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND
            if patchset_id is not None and requires_code_review_summary
            else None
        ),
        "review_command": review_command,
        "policy_command": f"ait policy eval {patchset_id}" if patchset_id is not None else None,
        "land_command": land_command,
        "task_complete_command": f"ait task complete {task_id}",
    }


def _workflow_land_next_action(
    *,
    change: dict[str, Any],
    task: dict[str, Any],
    workspace: dict[str, Any],
    patchset: dict[str, Any] | None,
    base_is_fresh: bool,
    workspace_matches_patchset: bool | None,
    patchset_is_authoritative: bool,
    attestation: dict[str, Any] | None,
    tests_state: str,
    requires_code_review_summary: bool,
    code_review_summary_count: int,
    review_blocking: int,
    review_approvals: int,
    policy_decision: str,
    target_line: str,
    ignore_workspace_authoring: bool,
    commands: dict[str, str | None],
) -> dict[str, Any]:
    if str(change.get("status") or "") == "landed" and str(task.get("status") or "") != "completed":
        return {
            "code": "complete_task",
            "summary": "Complete the task now that the change is landed.",
            "detail": "The workflow record should match the landed reality.",
            "command": commands["task_complete_command"],
        }
    if str(change.get("status") or "") == "landed":
        return {
            "code": "done",
            "summary": "This change and task are already fully landed.",
            "detail": "No further land workflow action is required.",
            "command": None,
        }
    if not ignore_workspace_authoring and not bool(workspace.get("clean")):
        return {
            "code": "snapshot_create",
            "summary": "Capture a fresh snapshot before publishing or refreshing the patchset.",
            "detail": "The workspace still has unsaved changes.",
            "command": 'ait snapshot create --message "reviewable checkpoint"',
        }
    if patchset is None or not base_is_fresh or (workspace_matches_patchset is False and not patchset_is_authoritative):
        return {
            "code": "publish_patchset" if patchset is None else "refresh_patchset",
            "summary": "Publish the current line as the reviewable patchset.",
            "detail": "The land workflow still needs a fresh published patchset from the current line.",
            "command": commands["publish_command"],
        }
    if commands["patchset_ci_command"] and (attestation is None or (tests_state and tests_state not in {"pass", "not_required"})):
        return {
            "code": "run_patchset_ci",
            "summary": "Run patchset CI so attestation evidence is recorded automatically.",
            "detail": "Routine patchsets should rely on CI-backed attestation evidence instead of manual attestation entry.",
            "command": commands["patchset_ci_command"],
        }
    if attestation is None or (tests_state and tests_state not in {"pass", "not_required"}):
        return {
            "code": "record_attestation",
            "summary": "Record attestation for the selected patchset.",
            "detail": "Policy and landing should work from explicit attestation evidence.",
            "command": commands["attest_command"],
        }
    if requires_code_review_summary and code_review_summary_count <= 0:
        return {
            "code": "record_code_review_summary",
            "summary": "Record code review summary for the selected patchset.",
            "detail": (
                "AI-related code patchsets need agent-prepared code review summary evidence before review approval or land. "
                + code_review_summary_requirement_text()
            ),
            "command": commands["code_review_summary_command"],
        }
    if review_blocking > 0:
        return {
            "code": "address_blocking_review",
            "summary": "Resolve the blocking review feedback before land.",
            "detail": "A blocking review is already recorded on this change.",
            "command": commands["review_command"],
        }
    if review_approvals <= 0:
        return {
            "code": "record_review",
            "summary": "Record the required task review for this change.",
            "detail": "Land still needs task/outcome approval.",
            "command": commands["review_command"],
        }
    if policy_decision != "pass":
        return {
            "code": "evaluate_policy",
            "summary": "Evaluate policy for the selected patchset.",
            "detail": "Policy has not passed yet for the selected patchset.",
            "command": commands["policy_command"],
        }
    return {
        "code": "submit_land",
        "summary": "Submit the approved patchset for landing.",
        "detail": f"The selected patchset is ready to land onto `{target_line}`.",
        "command": commands["land_command"],
    }


def _workflow_land_suggested_commands(
    *,
    next_action: dict[str, Any],
    change: dict[str, Any],
    patchset: dict[str, Any] | None,
    base_is_fresh: bool,
    workspace_matches_patchset: bool | None,
    requires_code_review_summary: bool,
    commands: dict[str, str | None],
) -> list[str]:
    return [
        command
        for command in [
            str(next_action.get("command") or "").strip() or None,
            commands["publish_command"] if patchset is None or not base_is_fresh or workspace_matches_patchset is False else None,
            commands["attestation_command"] if patchset is not None else None,
            commands["code_review_template_command"],
            commands["code_review_summary_command"] if patchset is not None and requires_code_review_summary else None,
            commands["review_command"] if patchset is not None else None,
            commands["policy_command"] if patchset is not None else None,
            commands["land_command"] if patchset is not None else None,
            commands["task_complete_command"] if str(change.get("status") or "") == "landed" else None,
        ]
        if command
    ]
