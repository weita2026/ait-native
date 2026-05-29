from __future__ import annotations

import json

from ait_protocol.common import connect_sqlite, normalize_plan_items, utc_now

from .repo_paths import RepoContext


def _connect_control(ctx: RepoContext):
    return connect_sqlite(ctx.control_db_path)


def create_workflow_plan(
    ctx: RepoContext,
    plan_id: str,
    plan_revision_id: str,
    repo_name: str,
    title: str,
    artifact_path: str,
    artifact_selector: str | None,
    artifact_heading: str,
    items: list[dict],
    *,
    artifact_blob_id: str | None = None,
    summary: str | None = None,
    status: str = "draft",
    source_kind: str = "manual_edit",
    source_session_id: str | None = None,
    created_by: str | None = None,
    actor_type: str | None = None,
    publication_state: str = "local_draft",
) -> dict:
    conn = _connect_control(ctx)
    now = utc_now()
    conn.execute(
        """
        insert into workflow_plans(
            plan_id, repo_name, title, status, head_revision_id, publication_state, published_remote_name,
            published_plan_id, published_head_revision_id, published_at, created_by, created_at, updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            plan_id,
            repo_name,
            title,
            status,
            plan_revision_id,
            publication_state,
            None,
            None,
            None,
            None,
            created_by,
            now,
            now,
        ),
    )
    conn.execute(
        """
        insert into workflow_plan_revisions(
            plan_revision_id, plan_id, revision_number, parent_plan_revision_id, title_snapshot, summary,
            artifact_path, artifact_selector, artifact_heading, artifact_blob_id, items_json,
            source_kind, source_session_id, created_by, actor_type, publication_state, published_plan_revision_id, published_at, created_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            plan_revision_id,
            plan_id,
            1,
            None,
            title,
            summary,
            artifact_path,
            artifact_selector,
            artifact_heading,
            artifact_blob_id,
            json.dumps(normalize_plan_items(items), sort_keys=True),
            source_kind,
            source_session_id,
            created_by,
            actor_type or "human",
            publication_state,
            None,
            None,
            now,
        ),
    )
    conn.commit()
    row = conn.execute("select * from workflow_plans where plan_id = ?", (plan_id,)).fetchone()
    conn.close()
    assert row is not None
    return dict(row)


def list_workflow_plans(ctx: RepoContext) -> list[dict]:
    conn = _connect_control(ctx)
    rows = [
        dict(r)
        for r in conn.execute(
            """
            select
                p.*,
                pr.revision_number as head_revision_number,
                pr.summary as head_revision_summary,
                pr.artifact_path as head_artifact_path,
                pr.artifact_selector as head_artifact_selector,
                pr.artifact_heading as head_artifact_heading,
                pr.artifact_blob_id as head_artifact_blob_id,
                pr.created_at as head_revision_created_at
            from workflow_plans p
            left join workflow_plan_revisions pr on pr.plan_revision_id = p.head_revision_id
            order by p.updated_at desc, p.created_at desc, p.plan_id desc
            """
        )
    ]
    conn.close()
    return rows


def get_workflow_plan(ctx: RepoContext, plan_id: str) -> dict:
    conn = _connect_control(ctx)
    row = conn.execute("select * from workflow_plans where plan_id = ?", (plan_id,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local plan: {plan_id}")
    return dict(row)


def list_workflow_plan_revisions(ctx: RepoContext, plan_id: str) -> list[dict]:
    conn = _connect_control(ctx)
    if conn.execute("select 1 from workflow_plans where plan_id = ?", (plan_id,)).fetchone() is None:
        conn.close()
        raise KeyError(f"Unknown local plan: {plan_id}")
    rows = [
        dict(r)
        for r in conn.execute(
            """
            select
                plan_revision_id, plan_id, revision_number, parent_plan_revision_id, title_snapshot, summary,
                artifact_path, artifact_selector, artifact_heading, artifact_blob_id, items_json, source_kind, source_session_id, created_by, actor_type,
                publication_state, published_plan_revision_id, published_at, created_at
            from workflow_plan_revisions
            where plan_id = ?
            order by revision_number desc
            """,
            (plan_id,),
        )
    ]
    conn.close()
    return rows


def get_workflow_plan_revision(ctx: RepoContext, plan_id: str, plan_revision_id: str) -> dict:
    conn = _connect_control(ctx)
    if conn.execute("select 1 from workflow_plans where plan_id = ?", (plan_id,)).fetchone() is None:
        conn.close()
        raise KeyError(f"Unknown local plan: {plan_id}")
    row = conn.execute(
        "select * from workflow_plan_revisions where plan_id = ? and plan_revision_id = ?",
        (plan_id, plan_revision_id),
    ).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local plan revision: {plan_revision_id}")
    return dict(row)


def get_workflow_plan_revision_by_id(ctx: RepoContext, plan_revision_id: str) -> dict:
    conn = _connect_control(ctx)
    row = conn.execute(
        "select * from workflow_plan_revisions where plan_revision_id = ?",
        (plan_revision_id,),
    ).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local plan revision: {plan_revision_id}")
    return dict(row)


def revise_workflow_plan(
    ctx: RepoContext,
    plan_id: str,
    plan_revision_id: str,
    artifact_path: str,
    artifact_selector: str | None,
    artifact_heading: str,
    items: list[dict],
    *,
    artifact_blob_id: str | None = None,
    title: str | None = None,
    summary: str | None = None,
    source_kind: str = "manual_edit",
    source_session_id: str | None = None,
    created_by: str | None = None,
    actor_type: str | None = None,
) -> dict:
    conn = _connect_control(ctx)
    plan_row = conn.execute("select * from workflow_plans where plan_id = ?", (plan_id,)).fetchone()
    if plan_row is None:
        conn.close()
        raise KeyError(f"Unknown local plan: {plan_id}")
    plan = dict(plan_row)
    head_revision_id = plan["head_revision_id"]
    next_revision_number = 1
    if head_revision_id:
        head_row = conn.execute(
            "select revision_number from workflow_plan_revisions where plan_revision_id = ?",
            (head_revision_id,),
        ).fetchone()
        if head_row is not None:
            next_revision_number = int(head_row["revision_number"]) + 1
    current_title = title or plan["title"]
    now = utc_now()
    conn.execute(
        """
        insert into workflow_plan_revisions(
            plan_revision_id, plan_id, revision_number, parent_plan_revision_id, title_snapshot, summary,
            artifact_path, artifact_selector, artifact_heading, artifact_blob_id, items_json,
            source_kind, source_session_id, created_by, actor_type, publication_state, published_plan_revision_id, published_at, created_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            plan_revision_id,
            plan_id,
            next_revision_number,
            head_revision_id,
            current_title,
            summary,
            artifact_path,
            artifact_selector,
            artifact_heading,
            artifact_blob_id,
            json.dumps(normalize_plan_items(items), sort_keys=True),
            source_kind,
            source_session_id,
            created_by,
            actor_type or "human",
            "local_draft",
            None,
            None,
            now,
        ),
    )
    conn.execute(
        "update workflow_plans set title = ?, head_revision_id = ?, updated_at = ? where plan_id = ?",
        (current_title, plan_revision_id, now, plan_id),
    )
    conn.commit()
    row = conn.execute("select * from workflow_plans where plan_id = ?", (plan_id,)).fetchone()
    conn.close()
    assert row is not None
    return dict(row)


def close_workflow_plan(ctx: RepoContext, plan_id: str, status: str) -> dict:
    if status not in {"archived", "superseded"}:
        raise ValueError(f"Unsupported historical local plan status: {status}")
    conn = _connect_control(ctx)
    row = conn.execute("select * from workflow_plans where plan_id = ?", (plan_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown local plan: {plan_id}")
    plan = dict(row)
    if plan["status"] == status:
        conn.close()
        return plan
    now = utc_now()
    conn.execute("update workflow_plans set status = ?, updated_at = ? where plan_id = ?", (status, now, plan_id))
    row = conn.execute("select * from workflow_plans where plan_id = ?", (plan_id,)).fetchone()
    conn.commit()
    conn.close()
    assert row is not None
    return dict(row)


def mark_workflow_plan_published(
    ctx: RepoContext,
    plan_id: str,
    *,
    remote_name: str | None,
    published_plan_id: str,
    published_head_revision_id: str | None,
    revision_mappings: list[tuple[str, str]],
) -> dict:
    conn = _connect_control(ctx)
    existing = conn.execute("select * from workflow_plans where plan_id = ?", (plan_id,)).fetchone()
    if existing is None:
        conn.close()
        raise KeyError(f"Unknown local plan: {plan_id}")
    now = utc_now()
    for local_revision_id, remote_revision_id in revision_mappings:
        conn.execute(
            """
            update workflow_plan_revisions
            set publication_state = 'published',
                published_plan_revision_id = ?,
                published_at = coalesce(published_at, ?)
            where plan_revision_id = ? and plan_id = ?
            """,
            (remote_revision_id, now, local_revision_id, plan_id),
        )
    conn.execute(
        """
        update workflow_plans
        set publication_state = 'published',
            published_remote_name = ?,
            published_plan_id = ?,
            published_head_revision_id = ?,
            published_at = coalesce(published_at, ?),
            updated_at = ?
        where plan_id = ?
        """,
        (remote_name, published_plan_id, published_head_revision_id, now, now, plan_id),
    )
    row = conn.execute("select * from workflow_plans where plan_id = ?", (plan_id,)).fetchone()
    conn.commit()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown local plan: {plan_id}")
    return dict(row)
