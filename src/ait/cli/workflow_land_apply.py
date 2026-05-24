from __future__ import annotations

from typing import Any

from ait_protocol.common import AuthorMode, build_minimum_provenance, normalize_optional_text

from ..remote_client import (
    close_task as remote_close_task,
    evaluate_policy as remote_evaluate_policy,
    put_attestation as remote_put_attestation,
    record_review as remote_record_review,
    run_patchset_ci as remote_run_patchset_ci,
)
from ..store import RepoContext, create_snapshot
from .remote_repository_defaults import _remote_tuple
from .runtime_defaults import (
    _effective_author_mode,
    _effective_checkpoint_id,
    _effective_model_name,
    _effective_reviewer_identity,
    _effective_session_id,
)
from .task_worktree_runtime import (
    _maybe_auto_remove_bound_worktree_after_land,
    _maybe_auto_remove_bound_worktree_after_task_complete,
)
from .workflow_boundary_sessions import _submit_remote_land_with_boundary_event
from .workflow_land_publish import _workflow_refresh_patchset_for_land
from .workflow_land_state import _workflow_land_payload
from .workflow_land_sync import _attach_local_land_sync
from .workspace_command_locking import _run_locked_task_bound_authoring_command


def _workflow_land_apply(
    ctx: RepoContext,
    *,
    change_id: str | None,
    patchset_id: str | None,
    remote_name: str | None,
    snapshot_message: str | None,
    patchset_summary: str | None,
    tests: str | None,
    lint: str | None,
    security: str | None,
    license: str | None,
    author_mode: AuthorMode | None,
    model: str | None,
    session: str | None,
    checkpoint: str | None,
    reviewer: str | None,
    review_message: str | None,
    target: str | None,
    mode: str,
    ignore_workspace_authoring: bool = False,
    patchset_is_authoritative: bool = False,
) -> dict[str, Any]:
    applied_actions: list[dict[str, Any]] = []
    stopped_reason: str | None = None
    seen_signatures: set[tuple[str, str | None, str, str, int, int]] = set()
    max_actions = 8
    current_change_id = change_id
    current_patchset_id = patchset_id

    for _ in range(max_actions):
        state = _workflow_land_payload(
            ctx,
            change_id=current_change_id,
            patchset_id=current_patchset_id,
            remote_name=remote_name,
            ignore_workspace_authoring=ignore_workspace_authoring,
            patchset_is_authoritative=patchset_is_authoritative,
        )
        next_action = state.get("next_action") if isinstance(state.get("next_action"), dict) else {}
        code = str(next_action.get("code") or "").strip()
        if code in {"", "done"}:
            state["applied_actions"] = applied_actions
            state["apply_status"] = "done"
            return state

        change = state.get("change") if isinstance(state.get("change"), dict) else {}
        patchset = state.get("patchset") if isinstance(state.get("patchset"), dict) else {}
        current_change_id = str(change.get("change_id") or current_change_id or "").strip() or current_change_id
        current_patchset_id = str(patchset.get("patchset_id") or current_patchset_id or "").strip() or current_patchset_id

        signature = (
            code,
            str(patchset.get("patchset_id") or "").strip() or None,
            str((state.get("change") or {}).get("status") or "").strip(),
            str((state.get("policy") or {}).get("decision") or "").strip(),
            int((state.get("review") or {}).get("approvals") or 0),
            int((state.get("review") or {}).get("blocking") or 0),
        )
        if signature in seen_signatures:
            stopped_reason = f"Workflow land apply made no further progress at `{code}`."
            break
        seen_signatures.add(signature)

        remote_row, repo_name = _remote_tuple(ctx, remote_name)

        if code == "snapshot_create":
            result = _run_locked_task_bound_authoring_command(
                ctx,
                "workflow land snapshot",
                lambda: create_snapshot(ctx, snapshot_message or "reviewable checkpoint"),
            )
        elif code in {"publish_patchset", "refresh_patchset"}:
            resolved_change_id = str(change.get("change_id") or change_id or "").strip()
            if not resolved_change_id:
                raise KeyError("Workflow land apply could not resolve a change to publish.")

            result = _run_locked_task_bound_authoring_command(
                ctx,
                "workflow land patchset publish",
                lambda: _workflow_refresh_patchset_for_land(
                    ctx,
                    change_id=resolved_change_id,
                    summary=patchset_summary or "review summary",
                    remote_name=remote_name,
                    author_mode=author_mode,
                ),
            )
            current_patchset_id = str(result.get("patchset_id") or current_patchset_id or "").strip() or current_patchset_id
        elif code == "record_attestation":
            patchset_id_value = str(patchset.get("patchset_id") or "").strip()
            if not patchset_id_value:
                raise KeyError("Workflow land apply could not resolve a patchset for attestation.")
            evaluation: dict[str, Any] = {}
            if tests is not None:
                evaluation["tests"] = tests
            if lint is not None:
                evaluation["lint"] = lint
            if security is not None:
                evaluation["security_scan"] = security
            if license is not None:
                evaluation["license_scan"] = license
            resolved_author_mode = _effective_author_mode(ctx, author_mode)
            resolved_model = _effective_model_name(ctx, model)
            resolved_session = _effective_session_id(session)
            resolved_checkpoint = _effective_checkpoint_id(checkpoint)
            provenance, detail = build_minimum_provenance(
                resolved_author_mode,
                model_name=resolved_model,
                session_id=resolved_session,
                checkpoint_id=resolved_checkpoint,
            )
            result = remote_put_attestation(
                remote_row["url"],
                patchset_id_value,
                resolved_author_mode,
                evaluation,
                provenance,
                detail,
                repo_name=repo_name,
                exact_id=True,
            )
        elif code == "run_patchset_ci":
            patchset_id_value = str(patchset.get("patchset_id") or "").strip()
            if not patchset_id_value:
                raise KeyError("Workflow land apply could not resolve a patchset for CI.")
            result = remote_run_patchset_ci(
                remote_row["url"],
                patchset_id_value,
                trigger="workflow_land_apply",
                repo_name=repo_name,
                exact_id=True,
            )
        elif code == "record_review":
            resolved_change_id = str(change.get("change_id") or change_id or "").strip()
            patchset_id_value = str(patchset.get("patchset_id") or "").strip()
            resolved_reviewer = _effective_reviewer_identity(ctx, reviewer)
            if not resolved_reviewer:
                stopped_reason = "Workflow land apply needs a reviewer identity before it can record task approval."
                break
            if not resolved_change_id or not patchset_id_value:
                raise KeyError("Workflow land apply could not resolve the change or patchset for task approval.")
            result = remote_record_review(
                remote_row["url"],
                resolved_change_id,
                patchset_id_value,
                resolved_reviewer,
                "task_approve",
                None,
                False,
                repo_name=repo_name,
                exact_id=True,
            )
        elif code == "record_code_review_summary":
            resolved_change_id = str(change.get("change_id") or change_id or "").strip()
            patchset_id_value = str(patchset.get("patchset_id") or "").strip()
            resolved_reviewer = _effective_reviewer_identity(ctx, reviewer)
            if not resolved_reviewer:
                stopped_reason = "Workflow land apply needs a reviewer identity before it can record code review summary evidence."
                break
            if not review_message:
                stopped_reason = "Workflow land apply needs --review-message containing the code review summary before it can record code review evidence."
                break
            if not resolved_change_id or not patchset_id_value:
                raise KeyError("Workflow land apply could not resolve the change or patchset for code review summary.")
            result = remote_record_review(
                remote_row["url"],
                resolved_change_id,
                patchset_id_value,
                resolved_reviewer,
                "code_review_summary",
                review_message,
                False,
                repo_name=repo_name,
                exact_id=True,
            )
        elif code == "evaluate_policy":
            patchset_id_value = str(patchset.get("patchset_id") or "").strip()
            if not patchset_id_value:
                raise KeyError("Workflow land apply could not resolve a patchset for policy evaluation.")
            result = remote_evaluate_policy(
                remote_row["url"],
                patchset_id_value,
                repo_name=repo_name,
                exact_id=True,
            )
        elif code == "submit_land":
            resolved_change_id = str(change.get("change_id") or change_id or "").strip()
            patchset_id_value = str(patchset.get("patchset_id") or "").strip() or None
            resolved_target = normalize_optional_text(target) or str((state.get("base_line") or {}).get("line_name") or "main")
            if not resolved_change_id:
                raise KeyError("Workflow land apply could not resolve a change for landing.")
            result = _submit_remote_land_with_boundary_event(
                ctx,
                remote_name=remote_name,
                repo_name_override=repo_name,
                change_id=resolved_change_id,
                patchset_id=patchset_id_value,
                target_line=resolved_target,
                mode=mode,
                session_id=session,
            )
            result = _attach_local_land_sync(ctx, remote_name, result)
            result = _maybe_auto_remove_bound_worktree_after_land(
                ctx,
                remote_name=remote_name,
                change_id=resolved_change_id,
                land_result=result,
            )
        elif code == "complete_task":
            task = state.get("task") if isinstance(state.get("task"), dict) else {}
            task_id_value = str(task.get("task_id") or "").strip()
            if not task_id_value:
                raise KeyError("Workflow land apply could not resolve a task to complete.")
            result = remote_close_task(remote_row["url"], task_id_value, "completed", repo_name=repo_name)
            bound_worktree_cleanup = _maybe_auto_remove_bound_worktree_after_task_complete(
                ctx,
                task_id=task_id_value,
                task_status=str(result.get("status") or "completed"),
            )
            if bound_worktree_cleanup is not None:
                result = dict(result)
                result["bound_worktree_cleanup"] = bound_worktree_cleanup
        elif code == "address_blocking_review":
            stopped_reason = "Workflow land apply stopped because blocking review feedback still needs manual resolution."
            break
        else:
            stopped_reason = f"Workflow land apply does not support automatic `{code}`."
            break

        applied_actions.append({"code": code, "result": result})

    final_state = _workflow_land_payload(
        ctx,
        change_id=current_change_id,
        patchset_id=current_patchset_id,
        remote_name=remote_name,
        ignore_workspace_authoring=ignore_workspace_authoring,
        patchset_is_authoritative=patchset_is_authoritative,
    )
    final_state["applied_actions"] = applied_actions
    final_state["apply_status"] = "stopped" if stopped_reason else "incomplete"
    if stopped_reason:
        final_state["apply_stopped_reason"] = stopped_reason
    return final_state
