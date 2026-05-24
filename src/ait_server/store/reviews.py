from __future__ import annotations

from typing import Any

from ait_protocol.common import (
    code_review_summary_requirement_text,
    is_structured_code_review_summary,
    missing_code_review_summary_sections,
    utc_now,
)

from ..server_control import connect, record_event
from ..server_paths import ServerContext
from .plans import _normalize_optional_text
from .repo_ops import _repo_id
from .workflow_artifacts import (
    CODE_REVIEW_SUMMARY_ACTION,
    TASK_REVIEW_APPROVE_ACTION,
    TASK_REVIEW_COMMENT_ACTION,
    TASK_REVIEW_REQUEST_CHANGES_ACTION,
    TEAM_REVIEW_APPROVE_ACTION,
    _invalidate_patchset_policy,
    _review_decision_lane,
)


def _refresh_change_state(ctx: ServerContext, conn, change_id: str) -> str:
    from .. import server_store as legacy_server_store

    return legacy_server_store._refresh_change_state(ctx, conn, change_id)


def _ensure_change_mutable(change, action: str) -> None:
    status = change["status"]
    change_id = change["change_id"]
    if status == "archived":
        raise ValueError(f"Change {change_id} is archived and cannot {action}")
    if status == "landed":
        raise ValueError(f"Change {change_id} is landed and cannot {action}")


def _required_approvals(lane: str) -> int:
    if lane == "critical":
        return 2
    if lane == "assisted":
        return 1
    return 0


def _review_summary(conn, change_id: str, patchset_id: str) -> dict[str, Any]:
    reviews = conn.execute(
        "select reviewer, action, blocking, comment, created_at from reviews where change_id = ? and patchset_id = ? order by review_id asc",
        (change_id, patchset_id),
    ).fetchall()
    latest_decision_by_reviewer_lane: dict[tuple[str, str], Any] = {}
    blocking_count = 0
    comment_count = 0
    structured_code_review_summary_reviewers: set[str] = set()

    def _normalized_reviewer(value: Any | None) -> str | None:
        text = _normalize_optional_text(value)
        return text.casefold() if text else None

    for row in reviews:
        decision_lane = _review_decision_lane(str(row["action"] or ""))
        if decision_lane:
            latest_decision_by_reviewer_lane[(row["reviewer"], decision_lane)] = row
        if row["action"] in {"request_changes", TASK_REVIEW_REQUEST_CHANGES_ACTION} or row["blocking"]:
            blocking_count += 1
        if row["action"] in {"comment", TASK_REVIEW_COMMENT_ACTION}:
            comment_count += 1
        if row["action"] == CODE_REVIEW_SUMMARY_ACTION:
            comment_count += 1
            if is_structured_code_review_summary(row["comment"]):
                normalized_reviewer = _normalized_reviewer(row["reviewer"])
                if normalized_reviewer is not None:
                    structured_code_review_summary_reviewers.add(normalized_reviewer)
    task_approval_count = sum(
        1 for row in latest_decision_by_reviewer_lane.values() if row["action"] == TASK_REVIEW_APPROVE_ACTION
    )
    team_approval_count = sum(
        1 for row in latest_decision_by_reviewer_lane.values() if row["action"] == TEAM_REVIEW_APPROVE_ACTION
    )
    approval_count = len(
        {
            row["reviewer"]
            for row in latest_decision_by_reviewer_lane.values()
            if row["action"] in {TASK_REVIEW_APPROVE_ACTION, TEAM_REVIEW_APPROVE_ACTION}
        }
    )
    human_approval_reviewers = {
        normalized
        for row in latest_decision_by_reviewer_lane.values()
        if row["action"] in {TASK_REVIEW_APPROVE_ACTION, TEAM_REVIEW_APPROVE_ACTION}
        for normalized in [_normalized_reviewer(row["reviewer"])]
        if normalized not in {None, "anonymous"}
    }
    human_task_approval_reviewers = {
        normalized
        for row in latest_decision_by_reviewer_lane.values()
        if row["action"] == TASK_REVIEW_APPROVE_ACTION
        for normalized in [_normalized_reviewer(row["reviewer"])]
        if normalized not in {None, "anonymous"}
    }
    independent_human_approval_reviewers = human_approval_reviewers - structured_code_review_summary_reviewers
    independent_task_approval_reviewers = human_task_approval_reviewers - structured_code_review_summary_reviewers
    code_review_summary_count = sum(
        1
        for row in reviews
        if row["action"] == CODE_REVIEW_SUMMARY_ACTION and is_structured_code_review_summary(row["comment"])
    )
    return {
        "approval_count": approval_count,
        "task_approval_count": task_approval_count,
        "team_approval_count": team_approval_count,
        "human_approval_count": len(human_approval_reviewers),
        "independent_human_approval_count": len(independent_human_approval_reviewers),
        "human_task_approval_count": len(human_task_approval_reviewers),
        "independent_task_approval_count": len(independent_task_approval_reviewers),
        "code_review_summary_reviewer_count": len(structured_code_review_summary_reviewers),
        "blocking_count": blocking_count,
        "comment_count": comment_count,
        "code_review_summary_count": code_review_summary_count,
        "review_count": len(reviews),
    }


def request_review(
    ctx: ServerContext,
    change_id: str,
    patchset_id: str,
    reviewer_groups: list[str],
    note: str | None = None,
) -> dict:
    with connect(ctx) as conn:
        change = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if change is None:
            raise KeyError(f"Unknown change: {change_id}")
        _ensure_change_mutable(change, "request reviews")
        patchset = conn.execute(
            "select * from patchsets where patchset_id = ? and change_id = ?",
            (patchset_id, change_id),
        ).fetchone()
        if patchset is None:
            raise KeyError(f"Patchset {patchset_id} does not belong to change {change_id}")
        now = utc_now()
        repo_id = str(change["repo_id"] or "").strip() or _repo_id(ctx, change["repo_name"])
        for group in reviewer_groups:
            conn.execute(
                "insert into review_requests(repo_id, change_id, patchset_id, reviewer_group, note, created_at) values (?, ?, ?, ?, ?, ?)",
                (repo_id, change_id, patchset_id, group, note, now),
            )
        record_event(
            conn,
            "review.requested",
            "change",
            change_id,
            {"patchset_id": patchset_id, "reviewer_groups": reviewer_groups},
        )
        _refresh_change_state(ctx, conn, change_id)
        conn.commit()
    return {
        "change_id": change_id,
        "patchset_id": patchset_id,
        "requested_groups": reviewer_groups,
        "status": "requested",
    }


def record_review(
    ctx: ServerContext,
    change_id: str,
    patchset_id: str,
    reviewer: str,
    action: str,
    comment: str | None,
    blocking: bool = False,
) -> dict:
    with connect(ctx) as conn:
        change = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if change is None:
            raise KeyError(f"Unknown change: {change_id}")
        _ensure_change_mutable(change, "record reviews")
        patchset = conn.execute(
            "select * from patchsets where patchset_id = ? and change_id = ?",
            (patchset_id, change_id),
        ).fetchone()
        if patchset is None:
            raise KeyError(f"Patchset {patchset_id} does not belong to change {change_id}")
        if action == CODE_REVIEW_SUMMARY_ACTION and missing_code_review_summary_sections(comment):
            raise ValueError(code_review_summary_requirement_text(comment))
        now = utc_now()
        repo_id = str(change["repo_id"] or "").strip() or _repo_id(ctx, change["repo_name"])
        cur = conn.execute(
            "insert into reviews(repo_id, change_id, patchset_id, reviewer, action, comment, blocking, created_at) values (?, ?, ?, ?, ?, ?, ?, ?)",
            (repo_id, change_id, patchset_id, reviewer, action, comment, int(blocking), now),
        )
        review_id = cur.lastrowid
        if action in {TASK_REVIEW_APPROVE_ACTION, CODE_REVIEW_SUMMARY_ACTION, "approve"} or blocking:
            _invalidate_patchset_policy(conn, patchset_id)
        record_event(
            conn,
            "review.recorded",
            "change",
            change_id,
            {"patchset_id": patchset_id, "reviewer": reviewer, "action": action},
        )
        _refresh_change_state(ctx, conn, change_id)
        conn.commit()
        row = conn.execute("select * from reviews where review_id = ?", (review_id,)).fetchone()
    return dict(row)


def list_reviews(ctx: ServerContext, change_id: str) -> dict:
    with connect(ctx) as conn:
        change = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if change is None:
            raise KeyError(f"Unknown change: {change_id}")
        current = conn.execute(
            "select * from patchsets where change_id = ? order by patchset_number desc limit 1",
            (change_id,),
        ).fetchone()
        patchset_id = current["patchset_id"] if current is not None else None
        reviews = [
            dict(r)
            for r in conn.execute(
                "select review_id, change_id, patchset_id, reviewer, action, comment, blocking, created_at from reviews where change_id = ? order by review_id asc",
                (change_id,),
            )
        ]
        requests = [
            dict(r)
            for r in conn.execute(
                "select review_request_id, patchset_id, reviewer_group, note, created_at from review_requests where change_id = ? order by review_request_id asc",
                (change_id,),
            )
        ]
        summary = (
            _review_summary(conn, change_id, patchset_id)
            if patchset_id
            else {
                "approval_count": 0,
                "task_approval_count": 0,
                "team_approval_count": 0,
                "human_approval_count": 0,
                "independent_human_approval_count": 0,
                "human_task_approval_count": 0,
                "independent_task_approval_count": 0,
                "code_review_summary_reviewer_count": 0,
                "blocking_count": 0,
                "comment_count": 0,
                "code_review_summary_count": 0,
                "review_count": 0,
            }
        )
    return {
        "change_id": change_id,
        "current_patchset_id": patchset_id,
        "approvals": summary["approval_count"],
        "task_approvals": summary["task_approval_count"],
        "team_approvals": summary["team_approval_count"],
        "human_approvals": summary["human_approval_count"],
        "independent_human_approvals": summary["independent_human_approval_count"],
        "human_task_approvals": summary["human_task_approval_count"],
        "independent_task_approvals": summary["independent_task_approval_count"],
        "code_review_summary_reviewers": summary["code_review_summary_reviewer_count"],
        "blocking": summary["blocking_count"],
        "comments": summary["comment_count"],
        "code_review_summaries": summary["code_review_summary_count"],
        "reviews": reviews,
        "review_requests": requests,
    }
