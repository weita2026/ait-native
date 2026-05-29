from __future__ import annotations

import importlib

from ait.cli import task_dag_app_exports

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_task_dag_app_exports() -> None:
    helper_names = [
        "build_task_dag_planning_compiler_surface",
        "build_task_graph_execution_strategy",
        "compute_task_graph_readiness",
        "remote_read_task_dag_progress",
        "remote_read_task_dag_readiness",
        "_remote_advance_task_dag_run",
        "_ci_route_mismatch_guidance",
        "_remote_read_task_dag_readiness",
        "_task_dag_graph_payload",
        "_task_dag_view_rows",
        "_task_dag_compact_packet_surface_payload",
        "_task_dag_readiness_payload",
        "_task_dag_start_auto_compact_worker",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(task_dag_app_exports, name)
