from __future__ import annotations

import base64
import hashlib
from pathlib import Path

from ait_protocol.common import (
    code_review_summary_requirement_text,
    is_structured_code_review_summary,
    render_code_review_summary_template,
)
from ait_server import server_store
from tests.postgres_fake import fake_postgres_context


def _snapshot_bundle(
    repo_name: str,
    snapshot_id: str,
    *,
    parent_snapshot_id: str | None,
    line_name: str,
    message: str,
    files: dict[str, bytes],
) -> dict:
    file_rows = []
    for path, data in files.items():
        blob_id = f"BLB-{snapshot_id}-{path.replace('/', '_')}"
        file_rows.append(
            {
                "path": path,
                "blob_id": blob_id,
                "size_bytes": len(data),
                "mode": "100644",
                "sha256": hashlib.sha256(data).hexdigest(),
                "content_b64": base64.b64encode(data).decode("ascii"),
            }
        )
    return {
        "snapshot_id": snapshot_id,
        "repo_name": repo_name,
        "parent_snapshot_id": parent_snapshot_id,
        "line_name": line_name,
        "message": message,
        "files": file_rows,
    }


def _publish_reviewable_patchset(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    repo_name = "review-lanes"
    server_store.ensure_repository(ctx, repo_name, "main", id_namespace_prefix="RLN")
    base_snapshot = server_store.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            "SNP-REVIEW-LANES-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"app.py": b"print('base')\n"},
        ),
    )
    revision_snapshot = server_store.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            "SNP-REVIEW-LANES-REV",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="revision",
            files={"app.py": b"print('base')\nprint('revision')\n"},
        ),
    )
    server_store.update_line(ctx, repo_name, "main", base_snapshot["snapshot_id"])
    task = server_store.create_task(ctx, repo_name, "Review lanes", "Keep review lanes distinct", "medium")
    change = server_store.create_change(ctx, repo_name, task["task_id"], "Review lane counts", "main", "medium")
    patchset = server_store.publish_patchset(
        ctx,
        change["change_id"],
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        "review lane patchset",
        "human_only",
    )
    return ctx, change, patchset


def test_task_and_team_review_lanes_do_not_overwrite_each_other_for_same_reviewer(tmp_path: Path):
    ctx, change, patchset = _publish_reviewable_patchset(tmp_path)

    server_store.record_review(
        ctx,
        change["change_id"],
        patchset["patchset_id"],
        "alice@example.com",
        "task_approve",
        "Task outcome approved.",
    )
    server_store.record_review(
        ctx,
        change["change_id"],
        patchset["patchset_id"],
        "alice@example.com",
        "approve",
        "Team patchset approved.",
    )

    summary = server_store.list_reviews(ctx, change["change_id"])

    assert summary["task_approvals"] == 1
    assert summary["team_approvals"] == 1
    assert summary["approvals"] == 1
    assert summary["human_approvals"] == 1


def test_code_review_summary_parser_accepts_numbered_markdown_packet():
    review_text = """
1. Files reviewed
README.md
2. Observations
No blocking findings.
3. Residual risks
Low.
4. Checks
pytest -q
5. Verdict
Safe to land.
""".strip()

    assert is_structured_code_review_summary(review_text) is True


def test_code_review_summary_requirement_text_reports_missing_sections_and_hint():
    message = "Reviewed files: app.py; Findings: no blocking findings."

    detail = code_review_summary_requirement_text(message)

    assert "Risks, Tests, Recommendation" in detail
    assert "ait review code template --style numbered" in detail


def test_render_code_review_summary_template_supports_numbered_style():
    template = render_code_review_summary_template("numbered")

    assert template.startswith("1. Reviewed files")
    assert "5. Recommendation" in template


def test_ai_code_policy_accepts_review_approval_from_code_review_summary_reviewer(tmp_path: Path):
    ctx, change, patchset = _publish_reviewable_patchset(tmp_path)

    server_store.upsert_attestation(
        ctx,
        patchset["patchset_id"],
        "ai_with_human_review",
        {"tests": "pass"},
        {},
    )
    server_store.record_review(
        ctx,
        change["change_id"],
        patchset["patchset_id"],
        "alice@example.com",
        "code_review_summary",
        "Reviewed files: README.md; Findings: no blocking findings; Risks: low; Tests: pytest -q; Recommendation: safe to land.",
    )
    server_store.record_review(
        ctx,
        change["change_id"],
        patchset["patchset_id"],
        "alice@example.com",
        "approve",
        "Human reviewer agrees with the agent summary.",
    )

    passing = server_store.evaluate_policy(ctx, patchset["patchset_id"])

    assert passing["decision"] == "pass"
    assert {check["name"]: check["status"] for check in passing["checks"]}["required_human_review"] == "pass"


def test_policy_eval_reuses_cached_decision_when_inputs_are_unchanged(tmp_path: Path):
    ctx, change, patchset = _publish_reviewable_patchset(tmp_path)

    server_store.upsert_attestation(
        ctx,
        patchset["patchset_id"],
        "human_only",
        {"tests": "pass"},
        {"policy_readable": True},
    )
    server_store.record_review(
        ctx,
        change["change_id"],
        patchset["patchset_id"],
        "alice@example.com",
        "approve",
        "Looks good.",
    )

    first = server_store.evaluate_policy(ctx, patchset["patchset_id"])

    with server_store.connect(ctx) as conn:
        count_after_first = conn.execute(
            "select count(*) as c from policy_decisions where patchset_id = ?",
            (patchset["patchset_id"],),
        ).fetchone()["c"]
        latest_after_first = server_store.latest_policy_status(conn, patchset["patchset_id"])

    second = server_store.evaluate_policy(ctx, patchset["patchset_id"])

    with server_store.connect(ctx) as conn:
        count_after_second = conn.execute(
            "select count(*) as c from policy_decisions where patchset_id = ?",
            (patchset["patchset_id"],),
        ).fetchone()["c"]

    assert first["decision"] == "pass"
    assert second["decision"] == "pass"
    assert second["checks"] == first["checks"]
    assert second["evaluated_at"] == first["evaluated_at"]
    assert count_after_first == 1
    assert count_after_second == 1
    assert latest_after_first is not None
    assert latest_after_first["input_fingerprint"]


def test_policy_eval_recomputes_when_repository_policy_changes(tmp_path: Path):
    ctx, change, patchset = _publish_reviewable_patchset(tmp_path)

    server_store.upsert_attestation(
        ctx,
        patchset["patchset_id"],
        "human_only",
        {"tests": "pass"},
        {"policy_readable": True},
    )
    server_store.record_review(
        ctx,
        change["change_id"],
        patchset["patchset_id"],
        "alice@example.com",
        "approve",
        "Looks good.",
    )

    first = server_store.evaluate_policy(ctx, patchset["patchset_id"])

    with server_store.connect(ctx) as conn:
        count_after_first = conn.execute(
            "select count(*) as c from policy_decisions where patchset_id = ?",
            (patchset["patchset_id"],),
        ).fetchone()["c"]

    server_store.ensure_repository(
        ctx,
        "review-lanes",
        "main",
        policy={
            "version": 1,
            "policy_id": "review-lanes-updated",
            "defaults": {
                "require_attestation": True,
                "require_tests": True,
                "require_lint": False,
                "require_security_scan": False,
                "require_license_scan": False,
                "require_ai_provenance": False,
                "require_code_review_summary": True,
            },
            "class_overrides": [],
        },
    )

    second = server_store.evaluate_policy(ctx, patchset["patchset_id"])

    with server_store.connect(ctx) as conn:
        count_after_second = conn.execute(
            "select count(*) as c from policy_decisions where patchset_id = ?",
            (patchset["patchset_id"],),
        ).fetchone()["c"]

    assert first["decision"] == "pass"
    assert second["decision"] == "pending"
    assert count_after_first == 1
    assert count_after_second == 2
    assert {check["name"]: check["status"] for check in second["checks"]}["code_review_summary"] == "pending"
