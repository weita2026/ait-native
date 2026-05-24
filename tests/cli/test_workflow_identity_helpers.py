from __future__ import annotations

from importlib import import_module

import pytest

from ait import local_control


workflow_identity_helpers = import_module("ait.cli.workflow_identity_helpers")


def test_aligned_remote_publish_identity_request_reuses_explicit_non_sequence_identity_source():
    row = {
        "task_id": "remote-task-7",
        "identity_source": local_control.LOCAL_IDENTITY_SOURCE_LEGACY,
    }

    assert (
        workflow_identity_helpers._aligned_remote_publish_identity_request(
            "http://example.test",
            "repo-alpha",
            row,
            entity_type="task",
            namespace_prefix="AIT",
        )
        == "remote-task-7"
    )


def test_require_remote_workflow_identity_family_accepts_local_task_origin_ids():
    assert (
        workflow_identity_helpers._require_remote_workflow_identity_family(
            "task",
            {"task_id": "LAITT-0042"},
            namespace_prefix="AIT",
            requested_id="LAITT-0042",
        )
        == "LAITT-0042"
    )


def test_require_remote_workflow_identity_family_rejects_unexpected_prefix():
    with pytest.raises(ValueError, match="unexpected namespace prefix"):
        workflow_identity_helpers._require_remote_workflow_identity_family(
            "task",
            {"task_id": "ZZZT-9999"},
            namespace_prefix="AIT",
        )


def test_require_remote_identity_rejects_unexpected_remote_id():
    with pytest.raises(ValueError, match="RL-9999"):
        workflow_identity_helpers._require_remote_identity(
            "release",
            "RL-0001",
            {"release_id": "RL-9999"},
        )
