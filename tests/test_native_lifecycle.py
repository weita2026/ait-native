from __future__ import annotations

import json
import os
import socket
import threading
import time
import urllib.request
from contextlib import contextmanager
from pathlib import Path

import uvicorn
from typer.testing import CliRunner

from ait_native.cli import app
from ait_native.remote_client import get_remote_line, update_remote_line
from ait_native.server import create_app
from ait_protocol.common import parse_policy_yaml, policy_to_yaml
from tests.postgres_fake import fake_postgres_dsn, install_fake_psycopg_global, reset_fake_postgres_runtime

runner = CliRunner()
POLICY_CODE_REVIEW_SUMMARY = (
    "Reviewed files: policy_notes.txt; Findings: no blocking findings; "
    "Risks: low policy-gating regression risk; Tests: targeted native lifecycle pytest coverage; "
    "Recommendation: safe to continue once policy requirements pass."
)


@contextmanager
def running_server(data_dir: Path):
    old = os.environ.get("AIT_NATIVE_SERVER_DATA")
    old_backend = os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND")
    old_dsn = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN")
    old_content_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
    old_control_schema = os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
    install_fake_psycopg_global()
    reset_fake_postgres_runtime()
    os.environ["AIT_NATIVE_SERVER_DATA"] = str(data_dir)
    os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = "postgres"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = fake_postgres_dsn(data_dir)
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = "ait_native_content"
    os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = "ait_native_control"
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    host, port = sock.getsockname()
    sock.close()
    app_obj = create_app()
    config = uvicorn.Config(app_obj, host=host, port=port, log_level="error")
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    base_url = f"http://{host}:{port}"
    deadline = time.time() + 10
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{base_url}/healthz", timeout=0.5) as resp:
                if resp.status == 200:
                    break
        except Exception:
            time.sleep(0.05)
    else:
        server.should_exit = True
        thread.join(timeout=5)
        raise RuntimeError("native test server did not start")
    try:
        yield base_url
    finally:
        server.should_exit = True
        thread.join(timeout=5)
        reset_fake_postgres_runtime()
        if old is None:
            os.environ.pop("AIT_NATIVE_SERVER_DATA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DATA"] = old
        if old_backend is None:
            os.environ.pop("AIT_NATIVE_SERVER_DB_BACKEND", None)
        else:
            os.environ["AIT_NATIVE_SERVER_DB_BACKEND"] = old_backend
        if old_dsn is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_DSN", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_DSN"] = old_dsn
        if old_content_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA"] = old_content_schema
        if old_control_schema is None:
            os.environ.pop("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", None)
        else:
            os.environ["AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA"] = old_control_schema


def _bootstrap_repo(tmp_path: Path, monkeypatch, base_url: str, policy_profile: str = "prototype"):
    repo = tmp_path / "housekeeper"
    artifact_path = Path("docs/sprints/native_lifecycle.md")
    plan_ref = "native-lifecycle/bootstrap"
    plan_item_ref = f"{plan_ref}/task"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)
    assert runner.invoke(app, ["init", "--name", "housekeeper", "--policy-profile", policy_profile], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
    plan_file = repo / artifact_path
    plan_file.parent.mkdir(parents=True, exist_ok=True)
    plan_file.write_text(
        (
            "# Native Lifecycle\n\n"
            f"## Bootstrap Lifecycle [plan-ref: {plan_ref}]\n\n"
            f"- [ ] Bootstrap native lifecycle coverage [ref: {plan_item_ref}]\n"
        ),
        encoding="utf-8",
    )

    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    main_snapshot = json.loads(main_snap_out.stdout)
    assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
    sync_out = runner.invoke(
        app,
        ["plan", "sync", str(artifact_path), "--remote", "origin", "--json"],
        catch_exceptions=False,
    )
    assert sync_out.exit_code == 0, sync_out.stdout
    plan = json.loads(sync_out.stdout)["results"][0]
    pull_out = runner.invoke(app, ["pull", "--line", "main", "--json"], catch_exceptions=False)
    assert pull_out.exit_code == 0, pull_out.stdout

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--title",
            "Native lifecycle",
            "--intent",
            "bootstrap",
            "--risk",
            "medium",
            "--plan",
            plan["plan_id"],
            "--plan-item-ref",
            plan_item_ref,
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree_path = Path(task["worktree"]["open_path"])
    monkeypatch.chdir(worktree_path)

    (worktree_path / "housekeeping.txt").write_text("run native lifecycle\n", encoding="utf-8")
    feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
    assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
    feature_snapshot = json.loads(feature_snap_out.stdout)

    change_out = runner.invoke(
        app,
        ["change", "create", "--task", task["task_id"], "--title", "Implement native lifecycle", "--base-line", "main", "--risk", "medium", "--json"],
        catch_exceptions=False,
    )
    assert change_out.exit_code == 0, change_out.stdout
    change = json.loads(change_out.stdout)

    patchset_out = runner.invoke(
        app,
        ["patchset", "publish", "--change", change["change_id"], "--summary", "reviewable native patchset", "--json"],
        catch_exceptions=False,
    )
    assert patchset_out.exit_code == 0, patchset_out.stdout
    patchset = json.loads(patchset_out.stdout)
    code_summary_out = runner.invoke(
        app,
        [
            "review",
            "code",
            "submit",
            change["change_id"],
            "--patchset",
            patchset["patchset_id"],
            "--reviewer",
            "codex",
            "--verdict",
            "pass",
            "--message",
            "Reviewed files: feature.txt; Findings: no blocking findings; Risks: low lifecycle fixture coverage only; Tests: lifecycle fixture setup; Recommendation: safe for fixture.",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert code_summary_out.exit_code == 0, code_summary_out.stdout

    return repo, main_snapshot, feature_snapshot, task, change, patchset


def _create_plan_bound_task(repo: Path, *, title: str, intent: str, risk: str = "low", slug: str) -> dict:
    artifact_path = Path("docs/sprints") / f"{slug}.md"
    plan_ref = f"native-lifecycle/{slug}"
    plan_item_ref = f"{plan_ref}/task"
    plan_file = repo / artifact_path
    plan_file.parent.mkdir(parents=True, exist_ok=True)
    plan_file.write_text(
        (
            f"# {title}\n\n"
            f"## {title} [plan-ref: {plan_ref}]\n\n"
            f"- [ ] {title} [ref: {plan_item_ref}]\n"
        ),
        encoding="utf-8",
    )
    sync_out = runner.invoke(
        app,
        ["plan", "sync", str(artifact_path), "--remote", "origin", "--json"],
        catch_exceptions=False,
    )
    assert sync_out.exit_code == 0, sync_out.stdout
    plan = json.loads(sync_out.stdout)["results"][0]
    pull_out = runner.invoke(app, ["pull", "--line", "main", "--json"], catch_exceptions=False)
    assert pull_out.exit_code == 0, pull_out.stdout
    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--title",
            title,
            "--intent",
            intent,
            "--risk",
            risk,
            "--plan",
            plan["plan_id"],
            "--plan-item-ref",
            plan_item_ref,
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    return json.loads(task_out.stdout)


def test_native_review_policy_and_land_happy_path(tmp_path: Path, monkeypatch):
    with running_server(tmp_path / "server-data") as base_url:
        _, main_snapshot, feature_snapshot, _, change, patchset = _bootstrap_repo(tmp_path, monkeypatch, base_url)

        req_out = runner.invoke(
            app,
            ["review", "team", "request", change["change_id"], "--group", "team-housekeeper", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert req_out.exit_code == 0, req_out.stdout

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout

        approve_out = runner.invoke(
            app,
            ["review", "task", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout

        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        policy = json.loads(policy_out.stdout)
        assert policy["decision"] == "pass"
        assert policy["lane"] == "assisted"
        checks = {row["name"]: row["status"] for row in policy["checks"]}
        assert checks["tests"] == "pass"
        assert checks["lint"] == "not_required"
        assert checks["security_scan"] == "not_required"
        assert checks["license_scan"] == "not_required"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        land = json.loads(land_out.stdout)
        assert land["status"] == "succeeded"
        assert land["result"]["landed_snapshot_id"] == feature_snapshot["snapshot_id"]
        assert land["result"]["base_snapshot_id"] == main_snapshot["snapshot_id"]

        change_out = runner.invoke(app, ["change", "show", change["change_id"], "--json"], catch_exceptions=False)
        assert change_out.exit_code == 0, change_out.stdout
        changed = json.loads(change_out.stdout)
        assert changed["status"] == "landed"


def test_land_cleanup_preserves_default_line_when_target_is_review_base(tmp_path: Path, monkeypatch):
    with running_server(tmp_path / "server-data-default-line-cleanup") as base_url:
        _, main_snapshot, feature_snapshot, _, change, patchset = _bootstrap_repo(tmp_path, monkeypatch, base_url)

        update_remote_line(base_url, "housekeeper", "review-base/default-line-cleanup", main_snapshot["snapshot_id"])
        update_remote_line(base_url, "housekeeper", "main", feature_snapshot["snapshot_id"])

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout

        approve_out = runner.invoke(
            app,
            ["review", "task", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout

        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        land_out = runner.invoke(
            app,
            [
                "land",
                "submit",
                change["change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--target",
                "review-base/default-line-cleanup",
                "--mode",
                "direct",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        land = json.loads(land_out.stdout)
        assert land["status"] == "succeeded"
        assert land["result"]["landed_snapshot_id"] == feature_snapshot["snapshot_id"]
        assert "main" not in (land["result"].get("archived_lines") or [])

        main_line = get_remote_line(base_url, "housekeeper", "main")
        assert main_line["status"] == "active"
        assert main_line["head_snapshot_id"] == feature_snapshot["snapshot_id"]


def test_lineage_only_markdown_updates_can_sync_without_blocking_task_dispatch(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-docs-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-docs-only") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper", "--default-author-mode", "human_only"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

        plan_file = repo / "docs" / "sprints" / "docs_policy.md"
        plan_file.parent.mkdir(parents=True, exist_ok=True)
        plan_file.write_text(
            "# Docs Policy\n\n## Docs Policy [plan-ref: docs-policy/root]\n\n- [ ] initial docs [ref: docs-policy/item]\n",
            encoding="utf-8",
        )

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        plan_file.write_text(
            "# Docs Policy\n\n## Docs Policy [plan-ref: docs-policy/root]\n\n- [ ] updated docs [ref: docs-policy/item]\n",
            encoding="utf-8",
        )

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Docs policy", "--intent", "verify docs-only policy gating", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout

        sync_out = runner.invoke(
            app,
            ["plan", "sync", "docs/sprints/docs_policy.md", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["status"] == "ok"


def test_ai_related_patchset_requires_policy_readable_provenance(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-ai-provenance"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-ai-provenance") as base_url:
        monkeypatch.chdir(repo)
        monkeypatch.delenv("AIT_SESSION_ID", raising=False)
        monkeypatch.delenv("AIT_CHECKPOINT_ID", raising=False)
        assert runner.invoke(
            app,
            ["init", "--name", "housekeeper", "--default-author-mode", "human_with_ai_assist"],
            catch_exceptions=False,
        ).exit_code == 0
        policy_path = repo / ".ait" / "policy.yaml"
        policy = parse_policy_yaml(policy_path.read_text(encoding="utf-8"))
        policy["class_overrides"].append(
            {
                "when": {"author_class": "ai_related"},
                "set": {
                    "require_attestation": True,
                    "require_ai_provenance": True,
                },
            }
        )
        policy_path.write_text(policy_to_yaml(policy), encoding="utf-8")
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task = _create_plan_bound_task(
            repo,
            title="AI provenance",
            intent="verify ai provenance gating",
            risk="low",
            slug="ai-provenance",
        )
        worktree_path = Path(task["worktree"]["open_path"])
        monkeypatch.chdir(worktree_path)
        (worktree_path / "policy_notes.txt").write_text("ai-assisted policy notes\n", encoding="utf-8")
        feature_snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "ai provenance", "--json"], catch_exceptions=False)
        assert feature_snapshot_out.exit_code == 0, feature_snapshot_out.stdout
        feature_snapshot = json.loads(feature_snapshot_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "AI provenance gate", "--base-line", "main", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "ai provenance patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout
        code_review_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--reviewer",
                "codex@example.com",
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--message",
                POLICY_CODE_REVIEW_SUMMARY,
                "--json",
            ],
            catch_exceptions=False,
        )
        assert code_review_out.exit_code == 0, code_review_out.stdout

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--model", "gpt-5.4-codex", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout

        policy_pending_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_pending_out.exit_code == 0, policy_pending_out.stdout
        pending_policy = json.loads(policy_pending_out.stdout)
        assert pending_policy["decision"] == "pending"
        assert pending_policy["content_class"] == "code_change"
        assert pending_policy["author_class"] == "ai_related"
        assert pending_policy["effective_requirements"]["require_tests"] is True
        assert pending_policy["effective_requirements"]["require_ai_provenance"] is True
        pending_checks = {row["name"]: row["status"] for row in pending_policy["checks"]}
        assert pending_checks["require_attestation"] == "pass"
        assert pending_checks["tests"] == "pass"
        assert pending_checks["ai_provenance"] == "pending"

        session_out = runner.invoke(
            app,
            [
                "session",
                "create",
                "--title",
                "AI provenance test run",
                "--task",
                task["task_id"],
                "--change",
                change["change_id"],
                "--json",
            ],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)

        checkpoint_out = runner.invoke(
            app,
            [
                "session",
                "checkpoint",
                session["session_id"],
                "--summary",
                "Attach durable provenance for the AI-related docs patchset",
                "--snapshot",
                feature_snapshot["snapshot_id"],
                "--json",
            ],
            catch_exceptions=False,
        )
        assert checkpoint_out.exit_code == 0, checkpoint_out.stdout
        checkpoint = json.loads(checkpoint_out.stdout)

        attest_complete_out = runner.invoke(
            app,
            [
                "attest",
                "put",
                patchset["patchset_id"],
                "--tests",
                "pass",
                "--model",
                "gpt-5.4-codex",
                "--session",
                session["session_id"],
                "--checkpoint",
                checkpoint["checkpoint_id"],
                "--json",
            ],
            catch_exceptions=False,
        )
        assert attest_complete_out.exit_code == 0, attest_complete_out.stdout

        policy_pass_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_pass_out.exit_code == 0, policy_pass_out.stdout
        pass_policy = json.loads(policy_pass_out.stdout)
        assert pass_policy["decision"] == "pass"
        pass_checks = {row["name"]: row["status"] for row in pass_policy["checks"]}
        assert pass_checks["ai_provenance"] == "pass"
        assert pass_checks["tests"] == "pass"


def test_m6_provenance_policy_acceptance_surfaces_are_consistent(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-m6-provenance"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-m6-provenance") as base_url:
        monkeypatch.chdir(repo)
        monkeypatch.delenv("AIT_SESSION_ID", raising=False)
        monkeypatch.delenv("AIT_CHECKPOINT_ID", raising=False)
        assert runner.invoke(
            app,
            ["init", "--name", "housekeeper", "--default-author-mode", "human_with_ai_assist"],
            catch_exceptions=False,
        ).exit_code == 0
        policy_path = repo / ".ait" / "policy.yaml"
        policy = parse_policy_yaml(policy_path.read_text(encoding="utf-8"))
        policy["class_overrides"].append(
            {
                "when": {"author_class": "ai_related"},
                "set": {
                    "require_attestation": True,
                    "require_ai_provenance": True,
                },
            }
        )
        policy_path.write_text(policy_to_yaml(policy), encoding="utf-8")
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task = _create_plan_bound_task(
            repo,
            title="M6 acceptance",
            intent="verify provenance/policy acceptance surfaces",
            risk="low",
            slug="m6-provenance",
        )
        worktree_path = Path(task["worktree"]["open_path"])
        monkeypatch.chdir(worktree_path)
        (worktree_path / "policy_notes.txt").write_text("m6 provenance notes\n", encoding="utf-8")
        feature_snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "m6 provenance", "--json"], catch_exceptions=False)
        assert feature_snapshot_out.exit_code == 0, feature_snapshot_out.stdout
        feature_snapshot = json.loads(feature_snapshot_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "M6 provenance acceptance", "--base-line", "main", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "m6 provenance acceptance patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout
        code_review_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--reviewer",
                "codex@example.com",
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--message",
                POLICY_CODE_REVIEW_SUMMARY,
                "--json",
            ],
            catch_exceptions=False,
        )
        assert code_review_out.exit_code == 0, code_review_out.stdout

        attest_partial_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--model", "gpt-5.4-codex", "--json"],
            catch_exceptions=False,
        )
        assert attest_partial_out.exit_code == 0, attest_partial_out.stdout

        attestation_partial_out = runner.invoke(
            app,
            ["attest", "show", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert attestation_partial_out.exit_code == 0, attestation_partial_out.stdout
        attestation_partial = json.loads(attestation_partial_out.stdout)
        partial_summary = attestation_partial["provenance_summary"]
        assert partial_summary["evidence_readiness"] == "partial"
        assert partial_summary["missing_fields"] == ["session_id", "checkpoint_id"]
        assert partial_summary["policy_readable"] is False

        policy_pending_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_pending_out.exit_code == 0, policy_pending_out.stdout
        pending_policy = json.loads(policy_pending_out.stdout)
        assert pending_policy["decision"] == "pending"
        assert pending_policy["content_class"] == "code_change"
        assert pending_policy["author_class"] == "ai_related"
        assert pending_policy["effective_requirements"]["require_tests"] is True
        assert pending_policy["effective_requirements"]["require_ai_provenance"] is True
        assert [row["when"] for row in pending_policy["matched_overrides"]] == [{"author_class": "ai_related"}]
        pending_checks = {row["name"]: row["status"] for row in pending_policy["checks"]}
        assert pending_checks["require_attestation"] == "pass"
        assert pending_checks["tests"] == "pass"
        assert pending_checks["ai_provenance"] == "pending"

        policy_show_pending_out = runner.invoke(
            app,
            ["policy", "show", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert policy_show_pending_out.exit_code == 0, policy_show_pending_out.stdout
        shown_pending_policy = json.loads(policy_show_pending_out.stdout)
        assert shown_pending_policy["content_class"] == "code_change"
        assert shown_pending_policy["author_class"] == "ai_related"
        assert shown_pending_policy["effective_requirements"]["require_ai_provenance"] is True

        session_out = runner.invoke(
            app,
            [
                "session",
                "create",
                "--title",
                "M6 provenance acceptance run",
                "--task",
                task["task_id"],
                "--change",
                change["change_id"],
                "--json",
            ],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)

        checkpoint_out = runner.invoke(
            app,
            [
                "session",
                "checkpoint",
                session["session_id"],
                "--summary",
                "Attach complete provenance for the M6 acceptance patchset",
                "--snapshot",
                feature_snapshot["snapshot_id"],
                "--json",
            ],
            catch_exceptions=False,
        )
        assert checkpoint_out.exit_code == 0, checkpoint_out.stdout
        checkpoint = json.loads(checkpoint_out.stdout)

        attest_complete_out = runner.invoke(
            app,
            [
                "attest",
                "put",
                patchset["patchset_id"],
                "--tests",
                "pass",
                "--model",
                "gpt-5.4-codex",
                "--session",
                session["session_id"],
                "--checkpoint",
                checkpoint["checkpoint_id"],
                "--json",
            ],
            catch_exceptions=False,
        )
        assert attest_complete_out.exit_code == 0, attest_complete_out.stdout

        attestation_complete_out = runner.invoke(
            app,
            ["attest", "show", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert attestation_complete_out.exit_code == 0, attestation_complete_out.stdout
        attestation_complete = json.loads(attestation_complete_out.stdout)
        complete_summary = attestation_complete["provenance_summary"]
        assert complete_summary["evidence_readiness"] == "complete"
        assert complete_summary["missing_fields"] == []
        assert complete_summary["policy_readable"] is True
        assert complete_summary["session_id"] == session["session_id"]
        assert complete_summary["checkpoint_id"] == checkpoint["checkpoint_id"]

        policy_pass_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_pass_out.exit_code == 0, policy_pass_out.stdout
        pass_policy = json.loads(policy_pass_out.stdout)
        assert pass_policy["decision"] == "pass"
        pass_checks = {row["name"]: row["status"] for row in pass_policy["checks"]}
        assert pass_checks["ai_provenance"] == "pass"
        assert pass_checks["tests"] == "pass"

        policy_show_pass_out = runner.invoke(
            app,
            ["policy", "show", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert policy_show_pass_out.exit_code == 0, policy_show_pass_out.stdout
        shown_pass_policy = json.loads(policy_show_pass_out.stdout)
        assert shown_pass_policy["decision"] == "pass"
        assert shown_pass_policy["matched_overrides"][0]["when"] == {"author_class": "ai_related"}

        inbox_json = urllib.request.urlopen(
            f"{base_url}/v1/native/read/reviewer-inbox?repo_name=housekeeper&author_class=ai_related"
        ).read().decode("utf-8")
        inbox = json.loads(inbox_json)
        assert inbox["count"] >= 1
        item = next(row for row in inbox["items"] if row["change_id"] == change["change_id"])
        attestation_summary = item["attestation"]
        assert attestation_summary["author_mode"] == "human_with_ai_assist"
        assert attestation_summary["model_name"] == "gpt-5.4-codex"
        assert attestation_summary["session_id"] == session["session_id"]
        assert attestation_summary["checkpoint_id"] == checkpoint["checkpoint_id"]
        assert attestation_summary["evidence_readiness"] == "complete"


def test_native_land_blocked_then_retry_after_approval(tmp_path: Path, monkeypatch):
    with running_server(tmp_path / "server-data-2") as base_url:
        _, _, feature_snapshot, _, change, patchset = _bootstrap_repo(tmp_path, monkeypatch, base_url)

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--lint", "pass", "--security", "pass", "--license", "pass", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        blocked = json.loads(land_out.stdout)
        assert blocked["status"] == "blocked"
        assert blocked["result"]["blocker_class"] == "POLICY_BLOCKED"

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout

        retry_out = runner.invoke(app, ["land", "retry", blocked["submission_id"], "--json"], catch_exceptions=False)
        assert retry_out.exit_code == 0, retry_out.stdout
        retried = json.loads(retry_out.stdout)
        assert retried["status"] == "succeeded"
        assert retried["result"]["landed_snapshot_id"] == feature_snapshot["snapshot_id"]


def test_policy_status_returns_pending_after_review_or_attestation_invalidation(tmp_path: Path, monkeypatch):
    with running_server(tmp_path / "server-data-policy-invalidation") as base_url:
        _, _, _, _, change, patchset = _bootstrap_repo(tmp_path, monkeypatch, base_url)

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout

        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        second_review_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "bob@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert second_review_out.exit_code == 0, second_review_out.stdout

        policy_show_after_review = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_show_after_review.exit_code == 0, policy_show_after_review.stdout
        after_review = json.loads(policy_show_after_review.stdout)
        assert after_review["decision"] == "pending"
        assert after_review["checks"] == []

        policy_refresh_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_refresh_out.exit_code == 0, policy_refresh_out.stdout
        assert json.loads(policy_refresh_out.stdout)["decision"] == "pass"

        attest_refresh_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"],
            catch_exceptions=False,
        )
        assert attest_refresh_out.exit_code == 0, attest_refresh_out.stdout

        policy_show_after_attest = runner.invoke(app, ["policy", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_show_after_attest.exit_code == 0, policy_show_after_attest.stdout
        after_attest = json.loads(policy_show_after_attest.stdout)
        assert after_attest["decision"] == "pending"
        assert after_attest["checks"] == []


def test_native_team_profile_keeps_missing_lint_pending(tmp_path: Path, monkeypatch):
    with running_server(tmp_path / "server-data-team") as base_url:
        _, _, _, _, change, patchset = _bootstrap_repo(tmp_path, monkeypatch, base_url, policy_profile="team")

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout

        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        policy = json.loads(policy_out.stdout)
        assert policy["policy_id"] == "team"
        assert policy["decision"] == "pending"
        checks = {row["name"]: row["status"] for row in policy["checks"]}
        assert checks["tests"] == "pass"
        assert checks["lint"] == "pending"
        assert checks["security_scan"] == "not_required"
        assert checks["license_scan"] == "not_required"


def test_native_waiver_keeps_failed_security_rule_blocked_from_remote_land(tmp_path: Path, monkeypatch):
    with running_server(tmp_path / "server-data-3") as base_url:
        _, _, _, _, change, patchset = _bootstrap_repo(tmp_path, monkeypatch, base_url, policy_profile="release")

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--lint", "pass", "--security", "fail", "--license", "pass", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--reviewer", "alice@example.com", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout

        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        policy = json.loads(policy_out.stdout)
        assert policy["decision"] == "hard_fail"
        assert policy["policy_id"] == "release"

        waive_out = runner.invoke(
            app,
            ["policy", "waive", patchset["patchset_id"], "--rule", "security_scan", "--reason", "bounded pilot exception", "--json"],
            catch_exceptions=False,
        )
        assert waive_out.exit_code == 0, waive_out.stdout
        waived = json.loads(waive_out.stdout)
        assert waived["policy"]["decision"] == "waived"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        blocked = json.loads(land_out.stdout)
        assert blocked["status"] == "blocked"
        assert blocked["result"]["blocker_class"] == "POLICY_BLOCKED"
        assert blocked["result"]["policy"]["decision"] == "waived"
