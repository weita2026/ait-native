import json
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CI_CONTRACT_PATH = REPO_ROOT / "ci" / "config.contract.json"


def _load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def test_ci_config_references_existing_suite_manifests():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    checked_in_contract = _load_json(CI_CONTRACT_PATH)
    ci = config["ci"]
    assert checked_in_contract["schema_version"] == 1
    assert checked_in_contract["ci"] == ci
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    manifests = {
        path.stem: _load_json(path)
        for path in sorted(suite_dir.glob("*.json"))
    }

    expected_suite_ids = {
        "preflight",
        "stable_smoke",
        "package_smoke",
        "tg1_required",
        "recent_regression",
        "task_batch",
        "task_linked_case_matrix",
        "full_repo",
        "postgres_preview",
        "release_readiness",
        "release_readiness_xdist_preview",
        "safe_parallel_xdist_preview",
        "release_artifact_smoke",
    }

    assert set(manifests) == expected_suite_ids
    assert set(ci["required_patchset_suites"]) == {
        "preflight",
        "stable_smoke",
        "package_smoke",
        "tg1_required",
    }
    assert set(ci["informational_patchset_suites"]) == {
        "recent_regression",
        "task_batch",
    }
    assert set(ci["nightly_suites"]) == {
        "full_repo",
        "postgres_preview",
        "release_readiness",
    }
    assert set(ci["release_suites"]) == {"release_artifact_smoke"}

    all_configured = (
        set(ci["required_patchset_suites"])
        | set(ci["informational_patchset_suites"])
        | set(ci["nightly_suites"])
        | set(ci["release_suites"])
    )
    assert all_configured == {
        "preflight",
        "stable_smoke",
        "package_smoke",
        "tg1_required",
        "recent_regression",
        "task_batch",
        "full_repo",
        "postgres_preview",
        "release_readiness",
        "release_artifact_smoke",
    }
    assert all_configured < expected_suite_ids

    for suite_id, manifest in manifests.items():
        assert manifest["schema_version"] == 1
        assert manifest["suite_id"] == suite_id
        assert manifest["purpose"]
        assert manifest["runner"]["kind"]
        assert manifest["mode"] in {"gate", "diagnostic"}


def test_patchset_lane_semantics_are_config_readable():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    ci = config["ci"]
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    manifests = {
        path.stem: _load_json(path)
        for path in sorted(suite_dir.glob("*.json"))
    }

    for suite_id in ci["required_patchset_suites"]:
        manifest = manifests[suite_id]
        assert manifest["plane"] == "patchset"
        assert manifest["default_blocking"] is True
        assert manifest["mode"] == "gate"

    for suite_id in ci["informational_patchset_suites"]:
        manifest = manifests[suite_id]
        assert manifest["default_blocking"] is False


def test_package_smoke_manifest_blocks_plan_cli_contract_regressions():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    ci = config["ci"]
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    package_smoke = _load_json(suite_dir / "package_smoke.json")

    assert package_smoke["runner"] == {
        "kind": "command_bundle",
        "commands": [
            "PYTHONPATH=src python3 - <<'PY'\nfrom ait.cli.app import app\nfrom typer.main import get_command\nget_command(app).main(args=['--help'], prog_name='ait', standalone_mode=False)\nPY",
            "PYTHONPATH=src python3 scripts/check_plan_cli_contracts.py",
        ],
    }


def test_tg1_required_manifest_blocks_live_membership_floor_regressions():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    ci = config["ci"]
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    tg1_required = _load_json(suite_dir / "tg1_required.json")

    assert tg1_required == {
        "schema_version": 1,
        "suite_id": "tg1_required",
        "display_name": "TG-1 Required",
        "plane": "patchset",
        "default_blocking": True,
        "mode": "gate",
        "purpose": "Resolve TG-1 membership from the PostgreSQL catalog via checked-in SQL and run the sibling ait_test live pytest membership as a blocking patchset gate.",
        "runner": {
            "kind": "command_bundle",
            "commands": [
                "PYTHONPATH=src python3 scripts/check_tg1_required_cases.py --repo-name ait_test --pytest-workers 6 --pytest-dist loadfile --output .ait/generated/ci/tg1_required.json --markdown .ait/generated/ci/tg1_required.md --junit-xml .ait/generated/ci/tg1_required.junit.xml"
            ],
        },
        "artifacts": {
            "summary_json": ".ait/generated/ci/tg1_required.json",
            "summary_markdown": ".ait/generated/ci/tg1_required.md",
            "junit_xml": ".ait/generated/ci/tg1_required.junit.xml",
            "log_path": ".ait/generated/ci/tg1_required.log",
        },
    }


def test_full_repo_and_task_batch_capture_initial_diagnostic_contract():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    ci = config["ci"]
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    full_repo = _load_json(suite_dir / "full_repo.json")
    task_batch = _load_json(suite_dir / "task_batch.json")
    release_readiness_xdist_preview = _load_json(suite_dir / "release_readiness_xdist_preview.json")
    safe_parallel_xdist_preview = _load_json(suite_dir / "safe_parallel_xdist_preview.json")

    assert full_repo["plane"] == "nightly"
    assert full_repo["mode"] == "diagnostic"
    assert full_repo["runner"]["allow_fail_fast"] is False
    assert full_repo["runner"]["collect_complete_failure_set"] is True
    assert full_repo["triage"]["suspect_task_selector"] == "recent_remote_landed"
    assert full_repo["triage"]["ownership_rules"] == [
        {"test_path_prefix": "tests/cli/", "owner": "workflow-core"},
        {"test_path_prefix": "tests/ait_web/", "owner": "web"},
        {"test_glob": "tests/test_native_*bot.py", "owner": "transport-runtime"},
    ]

    assert task_batch["runner"]["supported_selectors"] == [
        "recent_remote_landed",
        "recent_remote_landed_high_risk",
        "explicit_task_ids",
        "curated_corpus",
    ]
    assert task_batch["runner"]["audit_first"] is True
    assert task_batch["runner"]["behavior_suite_ids"] == ["full_repo"]
    assert task_batch["runner"]["curated_corpus_dir"] == "ci/task_corpora"
    assert task_batch["defaults"] == {
        "selector": "recent_remote_landed",
        "count": 0,
        "window_days": 7,
        "remote": "origin",
        "target_line": "main",
        "require_land_status": "succeeded",
        "blocking": False,
        "max_parallel": 4,
        "audit_first": True,
        "include_lineage_representatives": True,
    }
    assert release_readiness_xdist_preview["plane"] == "post_land_regression"
    assert release_readiness_xdist_preview["default_blocking"] is False
    assert release_readiness_xdist_preview["runner"] == {
        "kind": "pytest",
        "args": [
            "tests/cli/test_land_workflow.py",
            "tests/cli/test_task_change.py",
            "tests/cli/test_patchset_publish_contract.py",
            "-q",
            "-n",
            "2",
            "--dist",
            "loadfile",
        ],
    }
    assert safe_parallel_xdist_preview["plane"] == "post_land_regression"
    assert safe_parallel_xdist_preview["default_blocking"] is False
    assert safe_parallel_xdist_preview["runner"] == {
        "kind": "pytest",
        "args": [
            "tests/cli/test_auth_remote_ref.py",
            "tests/cli/test_benchmark_help_contracts.py",
            "tests/cli/test_cli_bootstrap.py",
            "tests/cli/test_config.py",
            "tests/cli/test_extra_help_contracts.py",
            "tests/cli/test_help_contracts.py",
            "tests/cli/test_land_workflow.py",
            "tests/cli/test_line_worktree.py",
            "tests/cli/test_patchset_publish_contract.py",
            "tests/cli/test_queue_repo_context.py",
            "tests/cli/test_release_help_contracts.py",
            "tests/cli/test_task_change.py",
            "tests/test_authority_store.py",
            "tests/test_ci_suite_contract.py",
            "tests/test_plan_schema_contract.py",
            "tests/test_planning_compiler.py",
            "tests/test_snapshot_diff.py",
            "tests/ait_web/test_compat_shim.py",
            "tests/ait_web/test_import_boundaries.py",
            "-q",
            "-n",
            "8",
            "--dist",
            "loadfile",
        ],
    }
    assert safe_parallel_xdist_preview["selection"] == {
        "strategy": "explicit_file_whitelist",
        "whitelist_files": [
            "tests/cli/test_auth_remote_ref.py",
            "tests/cli/test_benchmark_help_contracts.py",
            "tests/cli/test_cli_bootstrap.py",
            "tests/cli/test_config.py",
            "tests/cli/test_extra_help_contracts.py",
            "tests/cli/test_help_contracts.py",
            "tests/cli/test_land_workflow.py",
            "tests/cli/test_line_worktree.py",
            "tests/cli/test_patchset_publish_contract.py",
            "tests/cli/test_queue_repo_context.py",
            "tests/cli/test_release_help_contracts.py",
            "tests/cli/test_task_change.py",
            "tests/test_authority_store.py",
            "tests/test_ci_suite_contract.py",
            "tests/test_plan_schema_contract.py",
            "tests/test_planning_compiler.py",
            "tests/test_snapshot_diff.py",
            "tests/ait_web/test_compat_shim.py",
            "tests/ait_web/test_import_boundaries.py",
        ],
        "excluded_high_coupling_files": [
            "tests/test_native_worker_queue.py",
            "tests/test_native_postgres_runtime.py",
            "tests/test_native_pack_gc.py",
            "tests/test_native_storage_split.py",
        ],
        "exclusion_signals": [
            "repo-root chdir",
            "shared runtime/server fixtures",
            "process-global env mutation",
            "generated-state coupling",
            "fixed localhost port reuse",
        ],
    }
    assert "release_readiness_xdist_preview" not in ci["nightly_suites"]
    assert "safe_parallel_xdist_preview" not in ci["nightly_suites"]
    assert ci["task_batch"] == {
        "selector": "recent_remote_landed",
        "count": 0,
        "window_days": 7,
        "remote": "origin",
        "target_line": "main",
        "require_land_status": "succeeded",
        "blocking": False,
        "max_parallel": 4,
        "audit_first": True,
        "include_lineage_representatives": True,
    }


def test_task_linked_case_matrix_manifest_stays_explicit_opt_in():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    ci = config["ci"]
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    suite = _load_json(suite_dir / "task_linked_case_matrix.json")

    assert suite == {
        "schema_version": 1,
        "suite_id": "task_linked_case_matrix",
        "display_name": "Task Linked Case Matrix",
        "plane": "post_land_regression",
        "default_blocking": False,
        "mode": "diagnostic",
        "purpose": "Run each task's linked pytest node inventory from PostgreSQL task_test_case_links as a per-task CI matrix.",
        "runner": {
            "kind": "command_bundle",
            "commands": [
                "PYTHONPATH=src python3 scripts/task_linked_case_matrix.py --output .ait/generated/ci/task_linked_case_matrix.json --markdown .ait/generated/ci/task_linked_case_matrix.md"
            ],
        },
        "artifacts": {
            "summary_json": ".ait/generated/ci/task_linked_case_matrix.json",
            "summary_markdown": ".ait/generated/ci/task_linked_case_matrix.md",
            "log_path": ".ait/generated/ci/task_linked_case_matrix.log",
        },
    }
    assert "task_linked_case_matrix" not in ci["nightly_suites"]
    assert "task_linked_case_matrix" not in ci["release_suites"]
    assert "task_linked_case_matrix" not in ci["rollout"]["phase0"]["informational_repo_suites"]


def test_rollout_config_models_phase0_visibility_and_future_promotion():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    ci = config["ci"]
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    release_smoke = _load_json(suite_dir / "release_artifact_smoke.json")

    assert ci["rollout"] == {
        "phase": 0,
        "phase0": {
            "blocking_patchset_suites": ["preflight", "stable_smoke", "package_smoke", "tg1_required"],
            "informational_patchset_suites": ["recent_regression"],
            "informational_repo_suites": [
                "task_batch",
                "full_repo",
                "postgres_preview",
                "release_readiness",
                "release_artifact_smoke",
            ],
        },
        "promotion_candidates": {
            "phase1": ["recent_regression", "task_batch"],
            "phase2": ["full_repo"],
        },
        "release_evidence": {
            "required_before_distribution": False,
            "dependency_keys": ["dependency_report", "dependency_review"],
            "compliance_keys": ["compliance_report", "license_exception_review"],
        },
    }
    assert release_smoke["release_gate_evidence"] == ci["rollout"]["release_evidence"]


def test_xdist_preview_manifest_stays_explicit_opt_in_until_promoted():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    ci = config["ci"]
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    preview = _load_json(suite_dir / "release_readiness_xdist_preview.json")

    assert preview["suite_id"] == "release_readiness_xdist_preview"
    assert preview["plane"] == "post_land_regression"
    assert preview["mode"] == "diagnostic"
    assert preview["default_blocking"] is False
    assert "release_readiness_xdist_preview" not in ci["nightly_suites"]
    assert "release_readiness_xdist_preview" not in ci["release_suites"]
    assert "release_readiness_xdist_preview" not in ci["rollout"]["phase0"]["informational_repo_suites"]


def test_safe_parallel_xdist_preview_manifest_stays_explicit_opt_in_until_promoted():
    config = _load_json(REPO_ROOT / ".ait" / "config.json")
    ci = config["ci"]
    suite_dir = REPO_ROOT / ci["suite_manifest_dir"]
    preview = _load_json(suite_dir / "safe_parallel_xdist_preview.json")

    assert preview["suite_id"] == "safe_parallel_xdist_preview"
    assert preview["plane"] == "post_land_regression"
    assert preview["mode"] == "diagnostic"
    assert preview["default_blocking"] is False
    assert "safe_parallel_xdist_preview" not in ci["nightly_suites"]
    assert "safe_parallel_xdist_preview" not in ci["release_suites"]
    assert "safe_parallel_xdist_preview" not in ci["rollout"]["phase0"]["informational_repo_suites"]
