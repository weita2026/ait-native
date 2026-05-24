from __future__ import annotations

from pathlib import Path

from ait import local_control
from ait import local_workflow_identity


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

LOCAL_WORKFLOW_IDENTITY_EXPORTS = (
    'workflow_sequence_from_id',
    'get_workflow_sequence_floor',
    'allocate_workflow_task_identity',
    'allocate_workflow_change_identity',
)


def test_local_workflow_identity_helpers_match_local_control_facade() -> None:
    for name in LOCAL_WORKFLOW_IDENTITY_EXPORTS:
        assert getattr(local_workflow_identity, name) is getattr(local_control, name), name


def test_local_workflow_identity_domain_is_extracted_from_local_control_facade() -> None:
    local_control_text = (WORKSPACE_ROOT / 'src/ait/local_control.py').read_text(encoding='utf-8')
    identity_text = (WORKSPACE_ROOT / 'src/ait/local_workflow_identity.py').read_text(encoding='utf-8')

    assert 'from .local_workflow_identity import (' in local_control_text
    assert 'def workflow_sequence_from_id(' not in local_control_text
    assert 'def get_workflow_sequence_floor(' not in local_control_text
    assert 'def allocate_workflow_task_identity(' not in local_control_text
    assert 'def allocate_workflow_change_identity(' not in local_control_text
    assert 'def _resolve_workflow_task_id(' not in local_control_text
    assert 'def _resolve_workflow_change_id(' not in local_control_text
    assert 'from .local_control import (' not in identity_text
