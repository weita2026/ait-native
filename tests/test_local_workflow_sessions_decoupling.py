from __future__ import annotations

from pathlib import Path

from ait import local_control
from ait import local_workflow_sessions


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

LOCAL_WORKFLOW_SESSION_EXPORTS = (
    'create_workflow_session',
    'list_workflow_sessions',
    'get_workflow_session',
    'append_workflow_session_event',
    'list_workflow_session_events',
    'create_workflow_checkpoint',
    'list_workflow_checkpoints',
    'get_workflow_checkpoint',
    'record_workflow_snapshot_provenance',
    'get_workflow_snapshot_provenance',
    'list_workflow_snapshot_provenance',
    'list_workflow_snapshot_provenance_for_change',
    'resume_workflow_session',
    'close_workflow_session',
)


def test_local_workflow_session_helpers_match_local_control_facade() -> None:
    for name in LOCAL_WORKFLOW_SESSION_EXPORTS:
        assert getattr(local_workflow_sessions, name) is getattr(local_control, name), name


def test_local_workflow_session_domain_is_extracted_from_local_control_facade() -> None:
    local_control_text = (WORKSPACE_ROOT / 'src/ait/local_control.py').read_text(encoding='utf-8')
    session_text = (WORKSPACE_ROOT / 'src/ait/local_workflow_sessions.py').read_text(encoding='utf-8')

    assert 'from .local_workflow_sessions import (' in local_control_text
    assert 'def create_workflow_session(' not in local_control_text
    assert 'def append_workflow_session_event(' not in local_control_text
    assert 'def create_workflow_checkpoint(' not in local_control_text
    assert 'def record_workflow_snapshot_provenance(' not in local_control_text
    assert 'def resume_workflow_session(' not in local_control_text
    assert 'def close_workflow_session(' not in local_control_text
    assert 'from .local_control import (' not in session_text
