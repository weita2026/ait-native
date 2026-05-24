from __future__ import annotations

import base64
import hashlib
from importlib import import_module

import ait_native.local_content as native_local_content
from ait_protocol.common import CODE_REVIEW_SUMMARY_TEMPLATE, CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND
from ait_protocol.common import utc_now
from ait.cli.commands.queue import _workflow_code_review_summary_count, _workflow_review_lane_counts
from ait.cli.workflow_land_views import (
    _workflow_land_applied_action_summary,
    _workflow_land_batch_item_status,
    _workflow_land_completed_local_route_metadata,
    _workflow_land_preview_item_status,
)
from ait.cli.workflow_land_text import _render_workflow_land_text
from ait_server.server_control import connect
from ait_native.store import set_line_head as local_set_line_head
import pytest

from ._shared import *  # noqa: F401,F403


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


def _publish_direct_patchset(ctx, repo_name: str, task_id: str, suffix: str) -> tuple[dict, dict]:
    change = server_store_module.create_change(ctx, repo_name, task_id, f"DAG change {suffix}", "main", "medium")
    base_snapshot = server_store_module.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    revision_snapshot = server_store_module.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-REV",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="revision",
            files={"README.md": f"base\n{suffix}\n".encode("utf-8")},
        ),
    )
    server_store_module.update_line(ctx, repo_name, "main", base_snapshot["snapshot_id"])
    patchset = server_store_module.publish_patchset(
        ctx,
        change["change_id"],
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        f"patchset {suffix}",
        "human_only",
    )
    return change, patchset


def _publish_direct_patchset_from_feature_line(
    ctx,
    repo_name: str,
    task_id: str,
    suffix: str,
    *,
    feature_line_name: str,
    files: dict[str, bytes],
) -> tuple[dict, dict, dict, dict]:
    change = server_store_module.create_change(ctx, repo_name, task_id, f"Feature change {suffix}", "main", "medium")
    base_snapshot = server_store_module.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    revision_snapshot = server_store_module.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-REV",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name=feature_line_name,
            message="revision",
            files=files,
        ),
    )
    server_store_module.update_line(ctx, repo_name, "main", base_snapshot["snapshot_id"])
    server_store_module.update_line(ctx, repo_name, feature_line_name, revision_snapshot["snapshot_id"])
    patchset = server_store_module.publish_patchset(
        ctx,
        change["change_id"],
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        f"patchset {suffix}",
        "human_only",
    )
    return change, patchset, base_snapshot, revision_snapshot


def test_server_publish_patchset_rejects_non_descendant_revision(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data-non-descendant-patchset")
    server_store_module.initialize(ctx)
    server_store_module.ensure_repository(ctx, "housekeeper", "main")
    task = server_store_module.create_task(
        ctx,
        "housekeeper",
        "Reject non-descendant patchset",
        "require ancestry validation before patchset publication",
        "medium",
    )
    change = server_store_module.create_change(ctx, "housekeeper", task["task_id"], "Reject non-descendant patchset", "main", "medium")
    base_snapshot = server_store_module.import_snapshot(
        ctx,
        "housekeeper",
        _snapshot_bundle(
            "housekeeper",
            "SNP-NONDESC-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    unrelated_parent = server_store_module.import_snapshot(
        ctx,
        "housekeeper",
        _snapshot_bundle(
            "housekeeper",
            "SNP-NONDESC-OTHER",
            parent_snapshot_id=None,
            line_name="feature/other",
            message="other root",
            files={"README.md": b"other\n"},
        ),
    )
    revision_snapshot = server_store_module.import_snapshot(
        ctx,
        "housekeeper",
        _snapshot_bundle(
            "housekeeper",
            "SNP-NONDESC-REV",
            parent_snapshot_id=unrelated_parent["snapshot_id"],
            line_name="feature/other",
            message="non descendant revision",
            files={"README.md": b"other\nrevision\n"},
        ),
    )

    with pytest.raises(ValueError, match="does not descend from base snapshot"):
        server_store_module.publish_patchset(
            ctx,
            change["change_id"],
            base_snapshot["snapshot_id"],
            revision_snapshot["snapshot_id"],
            "should fail",
            "human_only",
        )


def _set_solo_remote_advisory(*, namespace_prefix: str | None = None) -> None:
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
    command = ["config", "set", "--plan-task-binding-mode", "advisory"]
    if namespace_prefix is not None:
        command.extend(["--id-namespace-prefix", namespace_prefix])
    assert runner.invoke(app, command, catch_exceptions=False).exit_code == 0


def _bind_task_worktree(task_id: str, monkeypatch, *, name: str = "manual-scratch") -> Path:
    shared = import_module("tests.cli._shared")
    return shared._bind_task_worktree(task_id, monkeypatch, name=name)


def _write_patchset_ci_contract(repo: Path) -> None:
    suite_dir = repo / "ci" / "suites"
    suite_dir.mkdir(parents=True, exist_ok=True)
    suite_dir.joinpath("preflight.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "preflight",
                "display_name": "Preflight",
                "plane": "patchset",
                "default_blocking": True,
                "mode": "gate",
                "purpose": "minimal preflight",
                "runner": {"kind": "command_bundle", "commands": ["python3 -c \"print('preflight ok')\""]},
                "artifacts": {"log_path": ".ait/generated/ci/preflight.log"},
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("stable_smoke.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "stable_smoke",
                "display_name": "Stable Smoke",
                "plane": "patchset",
                "default_blocking": True,
                "mode": "gate",
                "purpose": "minimal smoke",
                "runner": {"kind": "pytest", "args": ["tests/test_patchset_ci_smoke.py", "-q"]},
                "artifacts": {"junit_xml": ".ait/generated/ci/stable_smoke.junit.xml"},
            }
        ),
        encoding="utf-8",
    )
    suite_dir.joinpath("package_smoke.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "suite_id": "package_smoke",
                "display_name": "Package Smoke",
                "plane": "patchset",
                "default_blocking": True,
                "mode": "gate",
                "purpose": "minimal package smoke",
                "runner": {"kind": "command_bundle", "commands": ["python3 -c \"print('package ok')\""]},
                "artifacts": {"log_path": ".ait/generated/ci/package_smoke.log"},
            }
        ),
        encoding="utf-8",
    )
    tests_dir = repo / "tests"
    tests_dir.mkdir(parents=True, exist_ok=True)
    tests_dir.joinpath("test_patchset_ci_smoke.py").write_text(
        "def test_patchset_ci_smoke():\n    assert True\n",
        encoding="utf-8",
    )


def _seed_dag_backed_change(
    ctx,
    *,
    repo_name: str,
    suffix: str,
    task_plan_item_ref: str | None,
    graph_run_evidence: bool,
    dag_shared_boundary_node: bool | None = None,
    single_path_dag: bool | None = None,
) -> dict:
    server_store_module.ensure_repository(ctx, repo_name, "main")
    plan_item_ref = "milestone-1/bootstrap-native-workflow"
    plan_ref = f"dag-land/{suffix}"
    artifact = _plan_artifact_payload(
        f"# DAG Land\n\n## Bootstrap Durable Plan Storage [plan-ref: {plan_ref}]\n\n- store plan records [ref: {plan_item_ref}]\n",
        plan_ref,
    )
    plan = server_store_module.create_plan(
        ctx,
        repo_name,
        "Bootstrap durable plan storage",
        artifact["artifact_path"],
        artifact["artifact_selector"],
        artifact["artifact_heading"],
        artifact["items"],
        summary="seed",
    )
    plan_revision_id = plan["head_revision"]["plan_revision_id"]
    server_store_module.put_plan_revision_artifacts(
        ctx,
        plan["plan_id"],
        plan_revision_id,
        [
            _task_graph_artifact_payload(
                repo_name=repo_name,
                plan_id=plan["plan_id"],
                plan_revision_id=plan_revision_id,
                plan_ref=plan_ref,
                plan_item_ref=plan_item_ref,
                artifact_path=f"docs/plans/{suffix}.task_graph.json",
                graph_id=f"dag-land/{suffix}",
            )
        ],
    )
    if task_plan_item_ref is None:
        task = server_store_module.create_task(
            ctx,
            repo_name,
            f"Legacy DAG task {suffix}",
            "Simulate a legacy DAG-backed task without plan_item_ref",
            "medium",
        )
        with connect(ctx) as conn:
            conn.execute(
                """
                update tasks
                set planning_state = 'planned',
                    plan_id = ?,
                    origin_plan_revision_id = ?,
                    plan_item_ref = null,
                    plan_linked_at = ?
                where task_id = ?
                """,
                (plan["plan_id"], plan_revision_id, utc_now(), task["task_id"]),
            )
            conn.commit()
        task = server_store_module.get_task(ctx, task["task_id"])
    else:
        task = server_store_module.create_task(
            ctx,
            repo_name,
            f"DAG task {suffix}",
            "Require graph-run evidence before land",
            "medium",
            plan_id=plan["plan_id"],
            plan_item_ref=task_plan_item_ref,
        )
    change, patchset = _publish_direct_patchset(ctx, repo_name, task["task_id"], suffix.upper())
    if graph_run_evidence:
        metadata = {
            "session_policy": "task_dag_node_bootstrap",
            "plan_id": plan["plan_id"],
            "plan_revision_id": plan_revision_id,
            "plan_item_ref": plan_item_ref,
            "node_id": "A",
            "graph_run_id": f"graph-run-{suffix}",
            "graph_run_session_id": f"S-RUN-{suffix}",
            "task_graph_json": f"docs/plans/{suffix}.task_graph.json",
        }
        if dag_shared_boundary_node is not None:
            metadata["dag_shared_boundary_node"] = dag_shared_boundary_node
        if single_path_dag is not None:
            metadata["single_path_dag"] = single_path_dag
        server_store_module.create_session(
            ctx,
            repo_name,
            "agent_run",
            task_id=task["task_id"],
            change_id=change["change_id"],
            title=f"DAG run {suffix}",
            metadata=metadata,
        )
    return {
        "plan": plan,
        "plan_revision_id": plan_revision_id,
        "task": task,
        "change": change,
        "patchset": patchset,
        "plan_item_ref": plan_item_ref,
    }


def test_workflow_code_review_summary_count_filters_legacy_reviews_by_patchset():
    review_summary = {
        "reviews": [
            {"patchset_id": "P-1", "action": "code_review_summary", "comment": "old summary"},
            {"patchset_id": "P-2", "action": "comment", "comment": "ordinary comment"},
            {"patchset_id": "P-2", "action": "code_review_summary", "comment": "Code review summary: current summary"},
            {"patchset_id": "P-2", "action": "code_review_summary", "comment": CODE_REVIEW_SUMMARY_TEMPLATE},
            {
                "patchset_id": "P-2",
                "action": "code_review_summary",
                "comment": "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: pytest; Recommendation: safe to land.",
            },
        ]
    }

    assert _workflow_code_review_summary_count(review_summary, "P-2") == 1
    assert _workflow_code_review_summary_count(review_summary, "P-3") == 0


def test_workflow_review_lane_counts_keep_task_and_team_lanes_separate_for_same_reviewer():
    review_summary = {
        "reviews": [
            {"patchset_id": "P-1", "reviewer": "alice@example.com", "action": "task_approve", "blocking": 0},
            {"patchset_id": "P-1", "reviewer": "alice@example.com", "action": "approve", "blocking": 0},
            {"patchset_id": "P-2", "reviewer": "bob@example.com", "action": "approve", "blocking": 0},
        ]
    }

    counts = _workflow_review_lane_counts(review_summary, "P-1")

    assert counts["task_approvals"] == 1
    assert counts["team_approvals"] == 1
    assert counts["eligible_human_approvals"] == 1
    assert counts["approvals"] == 1
    assert counts["blocking"] == 0


def test_workflow_review_lane_counts_keep_review_approval_available_when_code_review_summary_and_task_approval_share_reviewer():
    review_summary = {
        "reviews": [
            {
                "patchset_id": "P-1",
                "reviewer": "alice@example.com",
                "action": "code_review_summary",
                "comment": "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: pytest; Recommendation: safe to land.",
                "blocking": 0,
            },
            {"patchset_id": "P-1", "reviewer": "alice@example.com", "action": "approve", "blocking": 0},
            {"patchset_id": "P-1", "reviewer": "anonymous", "action": "task_approve", "blocking": 0},
        ]
    }

    counts = _workflow_review_lane_counts(review_summary, "P-1")

    assert counts["approvals"] == 2
    assert counts["eligible_human_approvals"] == 2


def test_workflow_guide_lists_topics_and_help_alias(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-guide"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-workflow-guide"]).exit_code == 0

    list_out = runner.invoke(app, ["workflow", "guide", "--json"])
    assert list_out.exit_code == 0, list_out.stdout
    listed = json.loads(list_out.stdout)
    assert [row["topic"] for row in listed["topics"]] == ["inventory", "land"]

    inventory_out = runner.invoke(app, ["workflow", "guide", "inventory", "--json"])
    assert inventory_out.exit_code == 0, inventory_out.stdout
    inventory = json.loads(inventory_out.stdout)
    assert inventory["topic"] == "inventory"
    assert inventory["commands"][0]["command"] == "ait queue summary --all-changes"

    help_out = runner.invoke(app, ["workflow", "help", "land", "--json"])
    assert help_out.exit_code == 0, help_out.stdout
    help_payload = json.loads(help_out.stdout)
    assert help_payload["topic"] == "land"
    assert help_payload["commands"][-1]["command"] == "ait task complete <task-id>"


def test_review_code_template_command_prints_numbered_scaffold():
    template_out = runner.invoke(app, ["review", "code", "template", "--json"], catch_exceptions=False)

    assert template_out.exit_code == 0, template_out.stdout
    payload = json.loads(template_out.stdout)
    assert payload["style"] == "numbered"
    assert payload["template"].startswith("1. Reviewed files")
    assert payload["hint_command"] == CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND


def test_workflow_help_alias_is_hidden_but_still_works():
    help_out = runner.invoke(app, ["workflow", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "guide" in help_out.stdout
    assert "│ help" not in help_out.stdout

    guide_out = runner.invoke(app, ["workflow", "guide", "land", "--json"])
    assert guide_out.exit_code == 0, guide_out.stdout
    alias_out = runner.invoke(app, ["workflow", "help", "land", "--json"])
    assert alias_out.exit_code == 0, alias_out.stdout
    assert json.loads(alias_out.stdout) == json.loads(guide_out.stdout)


def test_workflow_publish_alias_is_hidden_from_public_help():
    help_out = runner.invoke(app, ["workflow", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "│ publish" not in help_out.stdout


def test_land_show_help_describes_submission_inspection_role():
    help_out = runner.invoke(app, ["land", "show", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "Inspect one remote land submission" in help_out.stdout
    assert "status, result, and any" in help_out.stdout
    assert "blocker class." in help_out.stdout


def test_land_retry_help_describes_recovery_role():
    help_out = runner.invoke(app, ["land", "retry", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "Retry a previously blocked or failed remote land submission" in help_out.stdout
    assert "after its blocker" in help_out.stdout
    assert "has been cleared." in help_out.stdout


def test_land_queue_processes_older_same_target_request_before_later_job(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-queue"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-queue") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Serialize land queue", "--intent", "land same-target requests in queue order", "--risk", "medium", "--json"])
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        def ready_change(name: str, body: str) -> tuple[dict, dict]:
            assert runner.invoke(app, ["line", "switch", "main", "--restore", "--force"]).exit_code == 0
            assert runner.invoke(app, ["line", "create", f"feature/{name}", "--switch", "--restore"]).exit_code == 0
            (workspace / "app.py").write_text(body, encoding="utf-8")
            snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", f"{name} work", "--json"])
            assert snapshot_out.exit_code == 0, snapshot_out.stdout
            change_out = runner.invoke(
                app,
                ["change", "create", "--task", task["task_id"], "--title", f"{name} change", "--base-line", "main", "--risk", "medium", "--json"],
                catch_exceptions=False,
            )
            assert change_out.exit_code == 0, change_out.stdout
            change = json.loads(change_out.stdout)
            patchset_out = runner.invoke(app, ["patchset", "publish", "--change", change["change_id"], "--summary", f"{name} patchset", "--json"])
            assert patchset_out.exit_code == 0, patchset_out.stdout
            patchset = json.loads(patchset_out.stdout)
            assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0
            approve_out = runner.invoke(
                app,
                ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
                catch_exceptions=False,
            )
            assert approve_out.exit_code == 0, approve_out.stdout
            _submit_passing_code_review_summary(change["change_id"], patchset["patchset_id"])
            policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"])
            assert policy_out.exit_code == 0, policy_out.stdout
            assert json.loads(policy_out.stdout)["decision"] == "pass"
            return change, patchset

        first_change, first_patchset = ready_change("first", "print('first')\n")
        second_change, second_patchset = ready_change("second", "print('second')\n")

        ctx = ServerContext.from_env()
        first_land = server_store_module.submit_land(ctx, first_change["change_id"], first_patchset["patchset_id"], "main", "direct", inline=False)
        second_land = server_store_module.submit_land(ctx, second_change["change_id"], second_patchset["patchset_id"], "main", "direct", inline=False)

        processed_second = server_store_module._process_land(ctx, second_land["submission_id"])
        processed_first = server_store_module.get_land_request(ctx, first_land["submission_id"])

        assert processed_first["status"] == "succeeded"
        assert processed_first["result"]["landed_snapshot_id"] == first_patchset["revision_snapshot_id"]
        assert processed_second["status"] == "blocked"
        assert processed_second["result"]["blocker_class"] == "BASE_STALE"
        assert processed_second["result"]["target_line_head"] == first_patchset["revision_snapshot_id"]

        remote_main = json.loads(urllib.request.urlopen(f"{base_url}/v1/native/repositories/housekeeper/lines/main").read().decode("utf-8"))
        assert remote_main["head_snapshot_id"] == first_patchset["revision_snapshot_id"]


def test_remote_land_succeeds_when_target_line_is_already_aligned_to_equivalent_snapshot(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-equivalent-aligned"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-equivalent-aligned") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Land equivalent aligned content", "--intent", "allow already-aligned equivalent remote land", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="equivalent-aligned-land")

        assert runner.invoke(app, ["line", "switch", "main", "--restore", "--force"]).exit_code == 0
        assert runner.invoke(app, ["line", "create", "feature/equivalent-aligned-land", "--switch", "--restore"]).exit_code == 0
        (workspace / "app.py").write_text("print('equivalent aligned land')\n", encoding="utf-8")
        revision_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
        assert revision_out.exit_code == 0, revision_out.stdout
        revision_snapshot = json.loads(revision_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Equivalent aligned land", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "equivalent aligned land", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["revision_snapshot_id"] == revision_snapshot["snapshot_id"]

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
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low equivalent-line alignment coverage only; Tests: targeted remote land regression; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert code_summary_out.exit_code == 0, code_summary_out.stdout
        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0
        team_request_out = runner.invoke(
            app,
            ["review", "team", "request", change["change_id"], "--group", "team-housekeeper", "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert team_request_out.exit_code == 0, team_request_out.stdout
        approve_out = runner.invoke(
            app,
            ["review", "task", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout
        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"])
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        assert runner.invoke(app, ["line", "switch", "main", "--restore", "--force"]).exit_code == 0
        (workspace / "app.py").write_text("print('equivalent aligned land')\n", encoding="utf-8")
        aligned_out = runner.invoke(app, ["snapshot", "create", "--message", "main already aligned", "--json"])
        assert aligned_out.exit_code == 0, aligned_out.stdout
        aligned_snapshot = json.loads(aligned_out.stdout)
        assert aligned_snapshot["snapshot_id"] != patchset["revision_snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        ctx = ServerContext.from_env()
        monkeypatch.setattr(
            server_store_module,
            "export_content_snapshot",
            lambda *_args, **_kwargs: (_ for _ in ()).throw(
                AssertionError("remote land freshness/alignment should not export full snapshot content")
            ),
        )
        monkeypatch.setattr(
            server_store_module,
            "list_content_lines",
            lambda *_args, **_kwargs: (_ for _ in ()).throw(
                AssertionError("common already-aligned land should archive the known source line without a full line scan")
            ),
        )
        lands_module = import_module("ait_server.store.lands")
        monkeypatch.setattr(
            lands_module,
            "list_content_lines_by_head_snapshot_ids",
            lambda *_args, **_kwargs: (_ for _ in ()).throw(
                AssertionError("common already-aligned land should archive the known source line without broader fallback lookup")
            ),
        )
        land = server_store_module.submit_land(ctx, change["change_id"], patchset["patchset_id"], "main", "direct", inline=True)

        assert land["status"] == "succeeded"
        assert land["result"]["line_action"] == "already_aligned"
        assert land["result"]["snapshot_action"] == "reused_equivalent_existing_snapshot"
        assert land["result"]["selected_revision_snapshot_id"] == patchset["revision_snapshot_id"]
        assert land["result"]["landed_snapshot_id"] == aligned_snapshot["snapshot_id"]
        assert land["result"]["base_snapshot_id"] == patchset["base_snapshot_id"]
        assert land["result"]["archived_lines"] == ["feature/equivalent-aligned-land"]
        assert land["result"]["phase_timings_ms"]["create_land_request"]["total"] >= 0
        assert land["result"]["phase_timings_ms"]["policy_evaluation"] >= 0
        assert land["result"]["phase_timings_ms"]["archive_lines"]["strategy"] == "direct_source_line"
        assert land["result"]["phase_timings_ms"]["archive_lines"]["fallback_scan_used"] is False
        assert land["result"]["freshness_preflight"]["target_matches_revision_tree"] is True

        remote_main = json.loads(urllib.request.urlopen(f"{base_url}/v1/native/repositories/housekeeper/lines/main").read().decode("utf-8"))
        assert remote_main["head_snapshot_id"] == aligned_snapshot["snapshot_id"]

        landed_change = server_store_module.get_change(ctx, change["change_id"])
        assert landed_change["status"] == "landed"

        audit_out = runner.invoke(app, ["task", "audit", task["task_id"], "--json"])
        assert audit_out.exit_code == 0, audit_out.stdout
        audit = json.loads(audit_out.stdout)
        assert audit["summary"]["effective_on_target_change_count"] == 1
        assert audit["changes"][0]["target_state"] == "landed_on_target"
        assert "equivalent-tree land outcomes" in audit["changes"][0]["target_reason"]


def test_remote_land_moves_target_line_without_full_line_scan_when_source_line_is_known(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-direct-source-archive"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-direct-source-archive"):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

        ctx = ServerContext.from_env()
        server_store_module.ensure_repository(ctx, "housekeeper", "main")
        task = server_store_module.create_task(
            ctx,
            "housekeeper",
            "Land known source line",
            "archive the known feature line without scanning every remote line",
            "medium",
        )
        change, patchset, _base_snapshot, revision_snapshot = _publish_direct_patchset_from_feature_line(
            ctx,
            "housekeeper",
            task["task_id"],
            "DIRECT-SOURCE",
            feature_line_name="feature/direct-source-land",
            files={"README.md": b"base\ndirect source land\n"},
        )
        assert patchset["revision_snapshot_id"] == revision_snapshot["snapshot_id"]

        monkeypatch.setattr(server_store_module, "evaluate_policy", lambda *_args, **_kwargs: {"decision": "pass"})
        monkeypatch.setattr(
            server_store_module,
            "list_content_lines",
            lambda *_args, **_kwargs: (_ for _ in ()).throw(
                AssertionError("common moved land should archive the known source line without a full line scan")
            ),
        )
        lands_module = import_module("ait_server.store.lands")
        monkeypatch.setattr(
            lands_module,
            "list_content_lines_by_head_snapshot_ids",
            lambda *_args, **_kwargs: (_ for _ in ()).throw(
                AssertionError("common moved land should archive the known source line without broader fallback lookup")
            ),
        )
        land = server_store_module.submit_land(ctx, change["change_id"], patchset["patchset_id"], "main", "direct", inline=True)

        assert land["status"] == "succeeded"
        assert land["result"]["line_action"] == "moved"
        assert land["result"]["snapshot_action"] == "selected_patchset_revision"
        assert land["result"]["landed_snapshot_id"] == patchset["revision_snapshot_id"]
        assert land["result"]["archived_lines"] == ["feature/direct-source-land"]
        assert land["result"]["freshness_preflight"]["target_line_head"] == patchset["revision_snapshot_id"]
        assert land["result"]["freshness_preflight"]["target_matches_revision_tree"] is True
        assert land["result"]["phase_timings_ms"]["create_land_request"]["total"] >= 0
        assert land["result"]["phase_timings_ms"]["policy_evaluation"] >= 0
        assert land["result"]["phase_timings_ms"]["target_line_update"]["advisory_lock_wait"] >= 0
        assert land["result"]["phase_timings_ms"]["target_line_update"]["advisory_lock_hold"] >= 0
        assert land["result"]["phase_timings_ms"]["archive_lines"]["strategy"] == "direct_source_line"
        assert land["result"]["phase_timings_ms"]["archive_lines"]["fallback_scan_used"] is False
        assert land["result"]["phase_timings_ms"]["total_process_land"] >= 0

        main_line = server_store_module.get_line(ctx, "housekeeper", "main")
        assert main_line["head_snapshot_id"] == patchset["revision_snapshot_id"]


def test_remote_land_skips_broader_fallback_when_known_source_line_is_unavailable(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-known-source-unavailable"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-known-source-unavailable"):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

        ctx = ServerContext.from_env()
        server_store_module.ensure_repository(ctx, "housekeeper", "main")
        task = server_store_module.create_task(
            ctx,
            "housekeeper",
            "Land known unavailable source line",
            "skip broader fallback when the source line is already known but unavailable",
            "medium",
        )
        change, patchset, _base_snapshot, revision_snapshot = _publish_direct_patchset_from_feature_line(
            ctx,
            "housekeeper",
            task["task_id"],
            "KNOWN-SOURCE-UNAVAILABLE",
            feature_line_name="feature/known-source-unavailable",
            files={"README.md": b"base\nknown source unavailable\n"},
        )
        assert patchset["revision_snapshot_id"] == revision_snapshot["snapshot_id"]

        content_module = import_module("ait_server.server_content")
        with content_module.connect(ctx) as conn:
            conn.execute(
                "delete from lines where repo_name = ? and line_name = ?",
                ("housekeeper", "feature/known-source-unavailable"),
            )
            conn.commit()

        monkeypatch.setattr(server_store_module, "evaluate_policy", lambda *_args, **_kwargs: {"decision": "pass"})
        lands_module = import_module("ait_server.store.lands")
        monkeypatch.setattr(
            lands_module,
            "list_content_lines_by_head_snapshot_ids",
            lambda *_args, **_kwargs: (_ for _ in ()).throw(
                AssertionError("known unavailable source line should not trigger broader fallback lookup")
            ),
        )
        land = server_store_module.submit_land(ctx, change["change_id"], patchset["patchset_id"], "main", "direct", inline=True)

        assert land["status"] == "succeeded"
        assert land["result"]["line_action"] == "moved"
        assert "archived_lines" not in land["result"]
        assert land["result"]["phase_timings_ms"]["archive_lines"]["strategy"] == "known_source_unavailable"
        assert land["result"]["phase_timings_ms"]["archive_lines"]["source_line_state"] == "missing"
        assert land["result"]["phase_timings_ms"]["archive_lines"]["fallback_scan_used"] is False


def test_remote_land_uses_indexed_head_lookup_when_source_line_name_is_unknown(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-indexed-head-lookup"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-indexed-head-lookup"):
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

        ctx = ServerContext.from_env()
        server_store_module.ensure_repository(ctx, "housekeeper", "main")
        task = server_store_module.create_task(
            ctx,
            "housekeeper",
            "Land indexed head lookup",
            "use indexed line-head lookup when the source line metadata is unavailable",
            "medium",
        )
        change, patchset, _base_snapshot, revision_snapshot = _publish_direct_patchset_from_feature_line(
            ctx,
            "housekeeper",
            task["task_id"],
            "INDEXED-HEAD-LOOKUP",
            feature_line_name="feature/indexed-head-lookup",
            files={"README.md": b"base\nindexed head lookup\n"},
        )
        assert patchset["revision_snapshot_id"] == revision_snapshot["snapshot_id"]

        content_module = import_module("ait_server.server_content")
        with content_module.connect(ctx) as conn:
            conn.execute(
                "update snapshots set line_name = null where snapshot_id = ?",
                (revision_snapshot["snapshot_id"],),
            )
            conn.commit()

        monkeypatch.setattr(server_store_module, "evaluate_policy", lambda *_args, **_kwargs: {"decision": "pass"})
        lands_module = import_module("ait_server.store.lands")
        original_lookup = lands_module.list_content_lines_by_head_snapshot_ids
        lookup_calls: list[tuple[str, tuple[str, ...], tuple[str, ...]]] = []

        def _record_lookup(current_ctx, repo_name, snapshot_ids, *, exclude_line_names=None):
            lookup_calls.append(
                (
                    repo_name,
                    tuple(sorted(snapshot_ids)),
                    tuple(sorted(str(name) for name in (exclude_line_names or ()))),
                )
            )
            return original_lookup(
                current_ctx,
                repo_name,
                snapshot_ids,
                exclude_line_names=exclude_line_names,
            )

        monkeypatch.setattr(lands_module, "list_content_lines_by_head_snapshot_ids", _record_lookup)
        land = server_store_module.submit_land(ctx, change["change_id"], patchset["patchset_id"], "main", "direct", inline=True)

        assert land["status"] == "succeeded"
        assert land["result"]["archived_lines"] == ["feature/indexed-head-lookup"]
        assert land["result"]["phase_timings_ms"]["archive_lines"]["strategy"] == "indexed_head_lookup"
        assert land["result"]["phase_timings_ms"]["archive_lines"]["source_line_name"] is None
        assert land["result"]["phase_timings_ms"]["archive_lines"]["fallback_scan_used"] is True
        assert land["result"]["phase_timings_ms"]["archive_lines"]["fallback_candidate_count"] == 1
        assert lookup_calls == [
            (
                "housekeeper",
                tuple(sorted({patchset["revision_snapshot_id"]})),
                ("main",),
            )
        ]


def test_land_submission_ids_include_change_id_to_avoid_cross_repo_collisions(tmp_path: Path, monkeypatch):
    repo_a = tmp_path / "housekeeper-land-submit-id-a"
    repo_b = tmp_path / "housekeeper-land-submit-id-b"
    repo_a.mkdir()
    repo_b.mkdir()
    (repo_a / "app.py").write_text("print('base a')\n", encoding="utf-8")
    (repo_b / "app.py").write_text("print('base b')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-submit-cross-repo") as base_url:
        def ready_and_land(repo: Path, repo_name: str, feature_name: str, namespace_prefix: str) -> tuple[dict, dict]:
            monkeypatch.chdir(repo)
            assert runner.invoke(app, ["init", "--name", repo_name]).exit_code == 0
            assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"]).exit_code == 0
            _set_solo_remote_advisory(namespace_prefix=namespace_prefix)

            main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
            assert main_snap_out.exit_code == 0, main_snap_out.stdout
            assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

            task_out = runner.invoke(
                app,
                ["task", "start", "--task-only", "--title", f"Land from {repo_name}", "--intent", "prove cross-repo land submissions stay unique", "--risk", "medium", "--json"],
                catch_exceptions=False,
            )
            assert task_out.exit_code == 0, task_out.stdout
            task = json.loads(task_out.stdout)
            workspace = _bind_task_worktree(task["task_id"], monkeypatch)

            assert runner.invoke(app, ["line", "create", f"feature/{feature_name}", "--switch", "--restore"]).exit_code == 0
            (workspace / "app.py").write_text(f"print('{repo_name}')\n", encoding="utf-8")
            feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", f"{repo_name} feature", "--json"])
            assert feature_snap_out.exit_code == 0, feature_snap_out.stdout

            change_out = runner.invoke(
                app,
                ["change", "create", "--task", task["task_id"], "--title", f"{repo_name} change", "--base-line", "main", "--risk", "medium", "--json"],
                catch_exceptions=False,
            )
            assert change_out.exit_code == 0, change_out.stdout
            change = json.loads(change_out.stdout)

            patchset_out = runner.invoke(app, ["patchset", "publish", "--change", change["change_id"], "--summary", f"{repo_name} patchset", "--json"])
            assert patchset_out.exit_code == 0, patchset_out.stdout
            patchset = json.loads(patchset_out.stdout)
            assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0
            review_out = runner.invoke(
                app,
                ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
                catch_exceptions=False,
            )
            assert review_out.exit_code == 0, review_out.stdout
            _submit_passing_code_review_summary(change["change_id"], patchset["patchset_id"], reviewed_files="app.py")
            policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"])
            assert policy_out.exit_code == 0, policy_out.stdout
            assert json.loads(policy_out.stdout)["decision"] == "pass"

            land_out = runner.invoke(
                app,
                ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
                catch_exceptions=False,
            )
            assert land_out.exit_code == 0, land_out.stdout
            land = json.loads(land_out.stdout)
            assert land["status"] == "succeeded"
            return change, land

        first_change, first_land = ready_and_land(repo_a, "housekeeper-a", "cross-repo-a", "AIT")
        second_change, second_land = ready_and_land(repo_b, "housekeeper-b", "cross-repo-b", "ACC")

        assert first_land["submission_id"] != second_land["submission_id"]
        assert first_change["change_id"] in first_land["submission_id"]
        assert second_change["change_id"] in second_land["submission_id"]


def test_create_land_request_rejects_legacy_dag_task_without_plan_item_ref(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data-land-legacy-dag-null-ref")
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    seeded = _seed_dag_backed_change(
        ctx,
        repo_name="repo-a",
        suffix="legacy-null-ref",
        task_plan_item_ref=None,
        graph_run_evidence=False,
    )

    with pytest.raises(ValueError) as excinfo:
        server_store_module.create_land_request(
            ctx,
            seeded["change"]["change_id"],
            seeded["patchset"]["patchset_id"],
            "main",
            "direct",
        )

    message = str(excinfo.value)
    assert "without a plan_item_ref" in message
    assert "node-bound tasks" in message
    assert "--plan-item-ref" in message


def test_create_land_request_rejects_dag_change_without_graph_run_evidence(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data-land-dag-no-graph-run")
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    seeded = _seed_dag_backed_change(
        ctx,
        repo_name="repo-a",
        suffix="no-graph-run",
        task_plan_item_ref="milestone-1/bootstrap-native-workflow",
        graph_run_evidence=False,
    )

    with pytest.raises(ValueError) as excinfo:
        server_store_module.create_land_request(
            ctx,
            seeded["change"]["change_id"],
            seeded["patchset"]["patchset_id"],
            "main",
            "direct",
        )

    message = str(excinfo.value)
    assert "no graph-run session evidence" in message
    assert "ait plan execute" in message


def test_create_land_request_accepts_dag_change_with_graph_run_evidence(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data-land-dag-with-graph-run")
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    seeded = _seed_dag_backed_change(
        ctx,
        repo_name="repo-a",
        suffix="with-graph-run",
        task_plan_item_ref="milestone-1/bootstrap-native-workflow",
        graph_run_evidence=True,
        dag_shared_boundary_node=True,
        single_path_dag=True,
    )

    land = server_store_module.create_land_request(
        ctx,
        seeded["change"]["change_id"],
        seeded["patchset"]["patchset_id"],
        "main",
        "direct",
    )

    assert land["status"] == "queued"
    assert land["change_id"] == seeded["change"]["change_id"]
    assert land["patchset_id"] == seeded["patchset"]["patchset_id"]


def test_create_land_request_rejects_non_final_dag_change_even_with_graph_run_evidence(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data-land-dag-non-final")
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    seeded = _seed_dag_backed_change(
        ctx,
        repo_name="repo-a",
        suffix="non-final",
        task_plan_item_ref="milestone-1/bootstrap-native-workflow",
        graph_run_evidence=True,
        dag_shared_boundary_node=False,
        single_path_dag=True,
    )

    with pytest.raises(ValueError) as excinfo:
        server_store_module.create_land_request(
            ctx,
            seeded["change"]["change_id"],
            seeded["patchset"]["patchset_id"],
            "main",
            "direct",
        )

    assert "non-final DAG node" in str(excinfo.value)


def test_task_audit_reports_ready_to_complete_after_land(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-audit-landed"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-task-audit-landed") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        main_snapshot = json.loads(main_snap_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Audit landed task", "--intent", "confirm ready-to-complete tasks", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        assert runner.invoke(app, ["line", "create", "feature/task-audit-landed"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/task-audit-landed"]).exit_code == 0
        (workspace / "app.py").write_text("print('audit landed')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
        feature_snapshot = json.loads(feature_snap_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Landable audit change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "task audit patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["base_snapshot_id"] == main_snapshot["snapshot_id"]
        assert patchset["revision_snapshot_id"] == feature_snapshot["snapshot_id"]

        attest_out = runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"])
        assert attest_out.exit_code == 0, attest_out.stdout

        review_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        )
        assert review_out.exit_code == 0, review_out.stdout
        _submit_passing_code_review_summary(change["change_id"], patchset["patchset_id"], reviewer="codex")

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout

        audit_out = runner.invoke(app, ["task", "audit", task["task_id"], "--json"])
        assert audit_out.exit_code == 0, audit_out.stdout
        audit = json.loads(audit_out.stdout)
        assert audit["workflow"]["state"] == "ready_to_complete"
        assert audit["queue_workflow"]["state"] == "ready_to_complete"
        assert audit["next_action"]["code"] == "complete_task"
        assert audit["summary"]["ready_to_complete"] is True
        assert audit["summary"]["effectively_complete_on_target"] is True
        assert audit["summary"]["stale_workflow_records"] is False
        assert audit["summary"]["verdict"] == "ready_to_complete"
        assert audit["recommended_action"]["code"] == "complete_task"
        assert audit["summary"]["effective_on_target_change_count"] == 1
        assert audit["changes"][0]["target_state"] == "landed_on_target"

        queue_json = urllib.request.urlopen(f"{base_url}/v1/native/read/task-queue?repo_name=housekeeper").read().decode("utf-8")
        queue_payload = json.loads(queue_json)
        queue_item = next(item for item in queue_payload["items"] if item["task"]["task_id"] == task["task_id"])
        assert queue_item["workflow"]["state"] == "ready_to_complete"
        assert queue_item["next_action"]["code"] == "complete_task"
        assert queue_item["attention"]["stale_base"] == 0
        assert queue_payload["summary"]["attention_required"] == 0
        assert queue_payload["summary"]["ready_to_complete"] == 1


def test_workflow_land_reports_publish_patchset_next_action(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-land-publish"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-land-publish") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Workflow land helper", "--intent", "show next land action", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        assert runner.invoke(app, ["line", "create", "feature/workflow-land-publish"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/workflow-land-publish"]).exit_code == 0
        (workspace / "app.py").write_text("print('feature work')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
        feature_snapshot = json.loads(feature_snap_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Workflow land helper", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        workflow_out = runner.invoke(app, ["workflow", "land", change["change_id"], "--json"])
        assert workflow_out.exit_code == 0, workflow_out.stdout
        workflow = json.loads(workflow_out.stdout)

        assert workflow["workspace"]["head_snapshot_id"] == feature_snapshot["snapshot_id"]
        assert workflow["patchset"] is None
        assert workflow["next_action"]["code"] == "publish_patchset"
        assert workflow["next_action"]["command"] == f'ait patchset publish --change {change["change_id"]} --summary "review summary"'
        assert workflow["steps"][0]["status"] == "done"
        assert workflow["steps"][1]["status"] == "pending"


def test_workflow_land_reports_retarget_guidance_before_publish(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-land-retarget"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-land-retarget") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Workflow land stale rebase guidance",
                "--intent",
                "surface retarget guidance before publish",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        change = payload["change"]
        worktree = payload["worktree"]
        worktree_path = Path(worktree["path"])

        monkeypatch.chdir(worktree_path)
        (worktree_path / "feature.txt").write_text("feature only\n", encoding="utf-8")
        feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature side", "--json"], catch_exceptions=False)
        assert feature_out.exit_code == 0, feature_out.stdout

        monkeypatch.chdir(repo)
        (repo / "README.md").write_text("base\nmain advanced\n", encoding="utf-8")
        repo_ctx = RepoContext.discover(repo)
        native_local_content.create_snapshot(repo_ctx, "housekeeper", "main", "main advance")

        monkeypatch.chdir(worktree_path)
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        workflow_out = runner.invoke(app, ["workflow", "land", change["change_id"], "--json"], catch_exceptions=False)
        assert workflow_out.exit_code == 0, workflow_out.stdout
        workflow = json.loads(workflow_out.stdout)

        assert workflow["patchset"] is None
        assert workflow["next_action"]["code"] == "publish_patchset"
        assert workflow["next_action"]["command"] == "ait worktree rebase --onto main"
        patchset_step = next(step for step in workflow["steps"] if step["code"] == "patchset")
        assert patchset_step["status"] == "pending"
        assert patchset_step["command"] == "ait worktree rebase --onto main"
        assert workflow["suggested_commands"][0] == "ait worktree rebase --onto main"


def test_workflow_land_apply_prefers_patchset_ci_over_manual_attestation(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-land-inline-ci"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")
    _write_patchset_ci_contract(repo)

    monkeypatch.setenv("AIT_NATIVE_QUEUE_MODE", "inline")
    with running_server(tmp_path / "server-data-workflow-land-inline-ci") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on", "--json"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Workflow land inline ci", "--intent", "prefer patchset ci over manual attestation", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = Path(task["worktree"]["path"])
        monkeypatch.chdir(workspace)
        (workspace / "app.py").write_text("print('inline ci')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "inline ci"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Workflow land inline ci", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        apply_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                change["change_id"],
                "--apply",
                "--summary",
                "guided land patchset",
                "--reviewer",
                "reviewer@example.com",
                "--review-message",
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: patchset CI passed; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert apply_out.exit_code == 0, apply_out.stdout
        applied = json.loads(apply_out.stdout)

        assert applied["apply_status"] == "done"
        assert "record_attestation" not in [row["code"] for row in applied["applied_actions"]]
        assert [row["code"] for row in applied["applied_actions"]] == [
            "publish_patchset",
            "run_patchset_ci",
            "record_code_review_summary",
            "record_review",
            "evaluate_policy",
            "submit_land",
            "complete_task",
        ]
        assert applied["patchset"]["patchset_id"].startswith("RAITP-")
        assert applied["next_action"]["code"] == "done"
        assert applied["patchset"]["patchset_id"].startswith("RAITP-")


def test_workflow_land_reports_land_submit_then_task_complete(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-land-complete"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-land-complete") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on", "--json"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Workflow land ready", "--intent", "show ready-to-land and complete guidance", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        assert "worktree" in task
        workspace = Path(task["worktree"]["path"])
        monkeypatch.chdir(workspace)
        (workspace / "app.py").write_text("print('ready to land')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "ready to land"]).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Workflow land ready", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "ready patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--reviewer",
                "codex",
                "--message",
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: pytest; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"]).exit_code == 0

        ready_out = runner.invoke(app, ["workflow", "land", change["change_id"], "--json"])
        assert ready_out.exit_code == 0, ready_out.stdout
        ready = json.loads(ready_out.stdout)
        assert ready["next_action"]["code"] == "submit_land"
        assert ready["next_action"]["command"] == f"ait land submit {change['change_id']} --patchset {patchset['patchset_id']} --target main --mode direct"
        assert ready["steps"][-1]["status"] == "ready"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout

        complete_out = runner.invoke(app, ["workflow", "land", change["change_id"], "--json"])
        assert complete_out.exit_code == 0, complete_out.stdout
        completed = json.loads(complete_out.stdout)
        assert completed["change"]["status"] == "landed"
        assert completed["next_action"]["code"] == "complete_task"
        assert completed["next_action"]["command"] == f"ait task complete {task['task_id']}"


def test_workflow_land_views_report_batch_status_and_completed_local_route_metadata():
    assert _workflow_land_batch_item_status(
        {"change": {"status": "landed"}, "task": {"status": "completed"}}
    ) == "completed"
    assert _workflow_land_batch_item_status({"next_action": {"code": "done"}}) == "completed"
    pending_state = {"change": {"status": "draft"}, "task": {"status": "active"}, "next_action": {"code": "publish_patchset"}}
    assert _workflow_land_batch_item_status(pending_state) == "blocked"
    assert _workflow_land_preview_item_status(pending_state) == "ready"
    assert _workflow_land_preview_item_status({"next_action": {"code": "done"}}) == "completed"

    assert _workflow_land_completed_local_route_metadata(
        local_task={"task_id": "LT-1", "published_task_id": " RT-1 "},
        local_change={"change_id": "LC-1", "published_change_id": ""},
        remote_name="origin",
        target_line="main",
        remote_task_id="  RT-1 ",
        remote_change_id=" RC-1 ",
    ) == {
        "kind": "completed_local",
        "local_task_id": "LT-1",
        "local_change_id": "LC-1",
        "published_task_id": "RT-1",
        "published_change_id": None,
        "remote_task_id": "RT-1",
        "remote_change_id": "RC-1",
        "remote_name": "origin",
        "target_line": "main",
    }


def test_workflow_land_applied_action_summary_formats_auto_rebase_and_cleanup():
    assert _workflow_land_applied_action_summary(
        {
            "code": "publish_patchset",
            "result": {
                "patchset_id": "RP-1",
                "auto_rebase": {"rebase": {"status": "applied"}},
            },
        }
    ) == "published patchset `RP-1` after auto-rebase `applied`"
    assert _workflow_land_applied_action_summary(
        {
            "code": "submit_land",
            "result": {
                "submission_id": "LAND-1",
                "status": "succeeded",
                "bound_worktree_cleanup": {
                    "status": "removed",
                    "worktree": {"name": "rt-1"},
                },
            },
        }
    ) == "land request `LAND-1` is `succeeded` and removed bound worktree `rt-1`"


def test_workflow_land_text_renderer_formats_steps_actions_and_next_action():
    rendered = _render_workflow_land_text(
        {
            "change": {"change_id": "RC-1", "status": "draft", "base_line": "main"},
            "task": {"task_id": "RT-1"},
            "patchset": {"patchset_id": "RP-1"},
            "workspace": {"current_line": "feature/rt-1", "workspace_status": "dirty", "changed_count": 2},
            "steps": [
                {
                    "status": "ready",
                    "label": "Publish patchset",
                    "detail": "Need a new patchset before review.",
                    "command": "ait patchset publish RC-1",
                }
            ],
            "applied_actions": [
                {
                    "code": "publish_patchset",
                    "result": {"patchset_id": "RP-1", "auto_rebase": {"rebase": {"status": "applied"}}},
                }
            ],
            "next_action": {
                "summary": "Publish patchset",
                "detail": "Create the first review artifact.",
                "command": "ait patchset publish RC-1",
            },
            "apply_stopped_reason": "Waiting for review",
        }
    )

    assert "ait workflow land · RC-1" in rendered
    assert "- workspace: dirty (2 changed)" in rendered
    assert "Workflow steps" in rendered
    assert "- [ready] Publish patchset: Need a new patchset before review." in rendered
    assert "Applied actions" in rendered
    assert "- publish_patchset: published patchset `RP-1` after auto-rebase `applied`" in rendered
    assert "Next action" in rendered
    assert "- Publish patchset" in rendered
    assert "Apply status" in rendered
    assert "- Waiting for review" in rendered


def test_remote_land_does_not_materialize_non_sprint_root_markdown(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-land-root-markdown"
    repo.mkdir()
    app_file = repo / "app.py"
    readme_zh = repo / "README_ZH.md"
    app_file.write_text("print('base')\n", encoding="utf-8")
    readme_zh.write_text("base release docs\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-land-root-markdown") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on", "--json"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Exclude root markdown from remote land", "--intent", "keep non-sprint root markdown out of remote-landed snapshots", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        assert "worktree" in task
        workspace = Path(task["worktree"]["path"])
        monkeypatch.chdir(workspace)
        (workspace / "app.py").write_text("print('landed')\n", encoding="utf-8")
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "remote root markdown", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Land tracked code only", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "exclude root markdown patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--reviewer",
                "codex",
                "--message",
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: pytest; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"]).exit_code == 0

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        landed = json.loads(land_out.stdout)
        assert landed["status"] == "succeeded"

        monkeypatch.chdir(repo)
        assert app_file.read_text(encoding="utf-8") == "print('landed')\n"
        assert readme_zh.read_text(encoding="utf-8") == "base release docs\n"

        main_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_out.exit_code == 0, main_out.stdout
        main_snapshot_id = json.loads(main_out.stdout)["head_snapshot_id"]
        snapshot_show_out = runner.invoke(app, ["snapshot", "show", main_snapshot_id, "--json"])
        assert snapshot_show_out.exit_code == 0, snapshot_show_out.stdout
        snapshot_paths = [row["path"] for row in json.loads(snapshot_show_out.stdout)["files"]]
        assert "app.py" in snapshot_paths
        assert "README_ZH.md" not in snapshot_paths


def test_land_submit_records_a_boundary_event_on_the_task_session(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-submit-session-boundary"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-submit-session-boundary") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on", "--json"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Land boundary session", "--intent", "record remote land boundaries on the shared task session", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        tracking_session_id = task["tracking"]["session_id"]
        assert "worktree" in task
        workspace = Path(task["worktree"]["path"])
        monkeypatch.chdir(workspace)
        (workspace / "app.py").write_text("print('land boundary')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "land boundary work"]).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Land boundary change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "land boundary patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        _submit_passing_code_review_summary(change["change_id"], patchset["patchset_id"], reviewed_files="app.py")
        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"])
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout

        events = remote_client_module.list_session_events(base_url, tracking_session_id, repo_name="housekeeper")
        boundary_events = [row for row in events if row["event_type"] == "workflow.boundary"]
        assert len(boundary_events) == 1
        payload = boundary_events[0]["payload"]
        assert payload["boundary_kind"] == "land_submit"
        assert payload["workflow_context"]["signals"][0]["kind"] == "land"
        assert payload["workflow_context"]["attachment_hints"]["change_id"] == change["change_id"]
        assert payload["workflow_context"]["attachment_hints"]["patchset_id"] == patchset["patchset_id"]


def test_land_submit_creates_a_new_task_session_when_the_existing_one_is_completed(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-submit-completed-session-fallback"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-submit-completed-session-fallback") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on", "--json"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Land completed session fallback", "--intent", "remote land should not append to a completed task session", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        tracking_session_id = task["tracking"]["session_id"]
        assert "worktree" in task
        workspace = Path(task["worktree"]["path"])
        monkeypatch.chdir(workspace)
        (workspace / "app.py").write_text("print('land completed fallback')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "land completed fallback work"]).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Land completed fallback change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "land completed fallback patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        _submit_passing_code_review_summary(change["change_id"], patchset["patchset_id"], reviewed_files="app.py")
        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"])
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        close_out = remote_client_module.close_session(base_url, tracking_session_id, status="completed", repo_name="housekeeper")
        assert close_out["status"] == "completed"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout

        original_events = remote_client_module.list_session_events(base_url, tracking_session_id, repo_name="housekeeper")
        original_boundary_events = [row for row in original_events if row["event_type"] == "workflow.boundary"]
        assert original_boundary_events == []

        sessions = remote_client_module.list_sessions(base_url, "housekeeper")
        boundary_session_ids = []
        for session in sessions:
            if session["session_id"] == tracking_session_id:
                continue
            events = remote_client_module.list_session_events(base_url, session["session_id"], repo_name="housekeeper")
            if any(
                row["event_type"] == "workflow.boundary" and row["payload"].get("boundary_kind") == "land_submit"
                for row in events
            ):
                boundary_session_ids.append(session["session_id"])
        assert len(boundary_session_ids) == 1


def test_ai_code_patchset_requires_code_review_summary_before_remote_land(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-code-review-summary-gate"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-code-review-summary-gate") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Require code review summary", "--intent", "block code land without technical review summary", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        assert runner.invoke(app, ["line", "create", "feature/code-review-summary"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/code-review-summary"]).exit_code == 0
        (workspace / "app.py").write_text("print('reviewed')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "code review summary gate"]).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Require code review summary", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            [
                "patchset",
                "publish",
                "--change",
                change["change_id"],
                "--summary",
                "code review summary gate patchset",
                "--author-mode",
                "ai_with_human_review",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0

        workflow_out = runner.invoke(app, ["workflow", "land", change["change_id"], "--json"])
        assert workflow_out.exit_code == 0, workflow_out.stdout
        workflow = json.loads(workflow_out.stdout)
        assert workflow["next_action"]["code"] == "record_code_review_summary"
        assert workflow["next_action"]["command"] == (
            f"ait review code submit {change['change_id']} --patchset {patchset['patchset_id']} --verdict pass --message \"{CODE_REVIEW_SUMMARY_TEMPLATE}\""
        )
        assert CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND in workflow["next_action"]["detail"]
        assert CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND in workflow["suggested_commands"]
        review_step = next(step for step in workflow["steps"] if step["code"] == "review")
        assert review_step["status"] == "pending"
        assert "Code review: pending" in review_step["detail"]

        pending_policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"])
        assert pending_policy_out.exit_code == 0, pending_policy_out.stdout
        pending_policy = json.loads(pending_policy_out.stdout)
        checks = {check["name"]: check["status"] for check in pending_policy["checks"]}
        assert pending_policy["decision"] == "pending"
        assert pending_policy["effective_requirements"]["require_code_review_summary"] is True
        assert checks["code_review_summary"] == "pending"

        blocked_land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert blocked_land_out.exit_code == 0, blocked_land_out.stdout
        blocked_land = json.loads(blocked_land_out.stdout)
        assert blocked_land["status"] == "blocked"
        assert blocked_land["result"]["blocker_class"] == "POLICY_BLOCKED"

        invalid_summary_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--reviewer",
                "codex",
                "--message",
                "looks good to me",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert invalid_summary_out.exit_code != 0
        invalid_summary_text = invalid_summary_out.stdout + invalid_summary_out.stderr
        assert "Code review summary is missing sections" in invalid_summary_text
        assert "safe scaffold" in invalid_summary_text

        summary_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--reviewer",
                "codex",
                "--message",
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low, policy gate only; Tests: pytest focused suite passed; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert summary_out.exit_code == 0, summary_out.stdout
        summary = json.loads(summary_out.stdout)
        assert summary["action"] == "code_review_summary"

        still_pending_policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"])
        assert still_pending_policy_out.exit_code == 0, still_pending_policy_out.stdout
        still_pending_policy = json.loads(still_pending_policy_out.stdout)
        assert still_pending_policy["decision"] == "pending"
        assert {check["name"]: check["status"] for check in still_pending_policy["checks"]}["code_review_summary"] == "pass"
        assert {check["name"]: check["status"] for check in still_pending_policy["checks"]}["required_human_review"] == "pending"

        approve_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "codex", "--json"],
            catch_exceptions=False,
        )
        assert approve_out.exit_code == 0, approve_out.stdout

        passing_policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"])
        assert passing_policy_out.exit_code == 0, passing_policy_out.stdout
        passing_policy = json.loads(passing_policy_out.stdout)
        assert passing_policy["decision"] == "pass"
        assert {check["name"]: check["status"] for check in passing_policy["checks"]}["code_review_summary"] == "pass"
        assert {check["name"]: check["status"] for check in passing_policy["checks"]}["required_human_review"] == "pass"

        ready_out = runner.invoke(app, ["workflow", "land", change["change_id"], "--json"])
        assert ready_out.exit_code == 0, ready_out.stdout
        ready = json.loads(ready_out.stdout)
        assert ready["next_action"]["code"] == "submit_land"


def test_workflow_land_apply_drives_change_to_landed_and_completes_task(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-land-apply"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-land-apply") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Workflow land apply", "--intent", "land from one guided helper", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        assert runner.invoke(app, ["line", "create", "feature/workflow-land-apply"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/workflow-land-apply"]).exit_code == 0
        (workspace / "app.py").write_text("print('apply helper')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "apply helper"]).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Workflow land apply", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        apply_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                change["change_id"],
                "--apply",
                "--summary",
                "guided land patchset",
                "--tests",
                "pass",
                "--reviewer",
                "reviewer@example.com",
                "--review-message",
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: pytest focused suite passed; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert apply_out.exit_code == 0, apply_out.stdout
        applied = json.loads(apply_out.stdout)

        assert applied["apply_status"] == "done"
        assert [row["code"] for row in applied["applied_actions"]] == [
            "publish_patchset",
            "record_attestation",
            "record_code_review_summary",
            "record_review",
            "evaluate_policy",
            "submit_land",
            "complete_task",
        ]
        assert applied["next_action"]["code"] == "done"
        assert applied["change"]["status"] == "landed"
        assert applied["task"]["status"] == "completed"
        assert applied["patchset"]["patchset_id"].startswith("RAITP-")
        submit_result = next(row["result"] for row in applied["applied_actions"] if row["code"] == "submit_land")
        assert submit_result["local_sync"]["status"] == "synced"
        assert submit_result["local_sync"]["line"] == "main"
        assert submit_result["local_sync"]["head_snapshot_id"] == applied["patchset"]["revision_snapshot_id"]
        assert submit_result["local_sync"]["workspace_restore"]["status"] == "restored"

        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == applied["patchset"]["revision_snapshot_id"]


def test_workflow_land_apply_rejects_foreign_snapshot_lineage_before_publish(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-land-foreign-lineage"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-land-foreign-lineage") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Workflow land apply foreign lineage",
                "--intent",
                "block auto publish when the current line head comes from foreign lineage",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        change = payload["change"]
        worktree = payload["worktree"]
        feature_line_name = str(worktree["current_line"])
        worktree_path = Path(worktree["path"])

        monkeypatch.chdir(worktree_path)
        worktree_ctx = RepoContext.discover(worktree_path)
        (worktree_path / "app.py").write_text("print('foreign lineage')\n", encoding="utf-8")
        foreign_snapshot = native_local_content.create_snapshot(
            worktree_ctx,
            "housekeeper",
            feature_line_name,
            "foreign workflow land lineage",
            parent_snapshot_id=str(worktree["head_snapshot_id"]),
        )
        local_set_line_head(worktree_ctx, feature_line_name, str(foreign_snapshot["snapshot_id"]))

        apply_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                change["change_id"],
                "--apply",
                "--summary",
                "should fail before publish",
                "--tests",
                "pass",
                "--reviewer",
                "reviewer@example.com",
                "--review-message",
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: pytest focused suite passed; Recommendation: safe to land.",
            ],
            catch_exceptions=False,
        )
        assert apply_out.exit_code != 0
        output = apply_out.output or apply_out.stdout
        assert "not owned by bound task" in output
        assert "Restore or reopen the correct task worktree" in output
