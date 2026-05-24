from __future__ import annotations

import pytest

from ait.workflow_conversation import (
    WORKFLOW_BOUNDARY_CLOSE_AFTER,
    WORKFLOW_SEGMENT_CLASS_CHANGE_LAND,
    WORKFLOW_SEGMENT_CLASS_PLANNING,
    WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION,
    infer_workflow_context,
    resolve_workflow_segment_attachment,
    summarize_workflow_segments,
)


@pytest.mark.parametrize(
    (
        "text",
        "session",
        "expected_class",
        "expected_classification_source",
        "expected_signal_kinds",
        "expected_boundary_behaviors",
        "expected_hints",
    ),
    [
        (
            "Please remote sync docs/sprints/workflow_conversation_segmentation.md",
            {
                "metadata": {
                    "plan_id": "PL-PLAN",
                    "planning_session_id": "PS-PLAN",
                    "graph_id": "graph-plan",
                    "node_id": "A",
                }
            },
            WORKFLOW_SEGMENT_CLASS_PLANNING,
            "boundary_signals",
            ["plan_sync"],
            [WORKFLOW_BOUNDARY_CLOSE_AFTER],
            {
                "plan_id": "PL-PLAN",
                "planning_session_id": "PS-PLAN",
                "graph_id": "graph-plan",
                "node_id": "A",
            },
        ),
        (
            "Use ait task start and ait change create for the first slice.",
            {
                "task_id": "T-EXEC",
                "metadata": {
                    "plan_id": "PL-EXEC",
                    "graph_run_id": "GR-EXEC",
                    "task_graph_json": "docs/sprints/workflow_conversation_segmentation.task_graph.json",
                    "node_id": "C",
                },
            },
            WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION,
            "attachment_hints",
            [],
            [],
            {
                "plan_id": "PL-EXEC",
                "task_id": "T-EXEC",
                "graph_run_id": "GR-EXEC",
                "task_graph_json": "docs/sprints/workflow_conversation_segmentation.task_graph.json",
                "node_id": "C",
            },
        ),
        (
            "Please remote land AITC-0200 after policy clears.",
            {
                "task_id": "T-LAND",
                "change_id": "AITC-0200",
                "metadata": {
                    "graph_run_id": "GR-LAND",
                    "node_id": "L",
                },
            },
            WORKFLOW_SEGMENT_CLASS_CHANGE_LAND,
            "boundary_signals",
            ["land"],
            [WORKFLOW_BOUNDARY_CLOSE_AFTER],
            {
                "task_id": "T-LAND",
                "change_id": "AITC-0200",
                "graph_run_id": "GR-LAND",
                "node_id": "L",
            },
        ),
    ],
)
def test_infer_workflow_context_classifies_boundary_signals_and_attachment_hints(
    text: str,
    session: dict,
    expected_class: str,
    expected_classification_source: str,
    expected_signal_kinds: list[str],
    expected_boundary_behaviors: list[str],
    expected_hints: dict[str, str],
):
    context = infer_workflow_context(text=text, session=session)

    assert context["segment_class"] == expected_class
    assert context["classification_source"] == expected_classification_source
    assert [row["kind"] for row in context["signals"]] == expected_signal_kinds
    assert [row["boundary_behavior"] for row in context["signals"]] == expected_boundary_behaviors
    assert context["attachment_hints"] == expected_hints


def test_infer_workflow_context_prefers_land_signals_over_attachment_only_execution_context() -> None:
    context = infer_workflow_context(
        text="Use ait task start now, then remote land C-0661 once policy clears.",
        session={"task_id": "T-0741", "change_id": "C-0661"},
    )

    assert context["segment_class"] == WORKFLOW_SEGMENT_CLASS_CHANGE_LAND
    assert [row["kind"] for row in context["signals"]] == ["land"]
    assert [row["boundary_behavior"] for row in context["signals"]] == [WORKFLOW_BOUNDARY_CLOSE_AFTER]
    assert context["attachment_hints"] == {"task_id": "T-0741", "change_id": "C-0661"}


def test_summarize_workflow_segments_keeps_post_sync_execution_in_one_window_until_land() -> None:
    events = [
        {
            "sequence": 1,
            "event_type": "session.message",
            "payload": {
                "text": "Let's discuss the design before we remote sync the planning artifact.",
                "workflow_context": infer_workflow_context(
                    text="Let's discuss the design before we remote sync the planning artifact.",
                    session={"plan_id": "PL-SEG", "planning_session_id": "PS-SEG"},
                ),
            },
        },
        {
            "sequence": 2,
            "event_type": "assistant.reply",
            "payload": {"text": "Planning reply."},
        },
        {
            "sequence": 3,
            "event_type": "session.message",
            "payload": {
                "text": "Now use ait task start so execution leaves the planning window.",
                "workflow_context": infer_workflow_context(
                    text="Now use ait task start so execution leaves the planning window.",
                    session={"task_id": "T-SEG"},
                ),
            },
        },
        {
            "sequence": 4,
            "event_type": "assistant.reply",
            "payload": {"text": "Execution reply."},
        },
        {
            "sequence": 5,
            "event_type": "session.message",
            "payload": {
                "text": "Please remote land C-SEG once policy clears.",
                "workflow_context": infer_workflow_context(
                    text="Please remote land C-SEG once policy clears.",
                    session={"task_id": "T-SEG", "change_id": "C-SEG"},
                ),
            },
        },
    ]

    segments = summarize_workflow_segments(events)

    assert len(segments) == 2
    assert [(segment["sequence_start"], segment["sequence_end"]) for segment in segments] == [(1, 1), (3, 5)]
    assert [segment.get("segment_class") for segment in segments] == [
        WORKFLOW_SEGMENT_CLASS_PLANNING,
        WORKFLOW_SEGMENT_CLASS_CHANGE_LAND,
    ]
    assert segments[-1]["signal_kinds"] == ["land"]
    assert segments[-1]["status"] == "closed"


def test_infer_workflow_context_preserves_durable_objects_from_attachment_hints() -> None:
    context = infer_workflow_context(
        text=None,
        session={
            "plan_id": "PL-001",
            "planning_session_id": "PS-001",
            "task_id": "T-001",
            "change_id": "C-001",
            "metadata": {
                "selected_patchset_id": "P-001-2",
                "land_id": "LAND-C-001-0001",
            },
        },
    )

    assert context["segment_class"] == WORKFLOW_SEGMENT_CLASS_CHANGE_LAND
    assert context["classification_source"] == "attachment_hints"
    assert context["durable_objects"] == [
        {"object_type": "plan", "object_id": "PL-001"},
        {"object_type": "planning_session", "object_id": "PS-001"},
        {"object_type": "task", "object_id": "T-001"},
        {"object_type": "change", "object_id": "C-001"},
        {"object_type": "patchset", "object_id": "P-001-2"},
        {"object_type": "land", "object_id": "LAND-C-001-0001"},
    ]


def test_resolve_workflow_segment_attachment_uses_durable_hints_first() -> None:
    resolution = resolve_workflow_segment_attachment(
        {
            "segment_class": "change_land",
            "durable_objects": [
                {"object_type": "task", "object_id": "T-001"},
                {"object_type": "change", "object_id": "C-001"},
                {"object_type": "patchset", "object_id": "P-001-1"},
            ],
        },
        changes=[{"change_id": "C-002", "status": "draft"}],
    )

    assert resolution == {
        "status": "attached",
        "confidence": "high",
        "primary_target": {"object_type": "change", "object_id": "C-001"},
        "supporting_targets": [
            {"object_type": "task", "object_id": "T-001"},
            {"object_type": "patchset", "object_id": "P-001-1"},
        ],
        "reason": "durable object hints already identify the workflow attachment",
    }


def test_resolve_workflow_segment_attachment_asks_one_question_on_ambiguity() -> None:
    resolution = resolve_workflow_segment_attachment(
        {"segment_class": "task_execution", "durable_objects": []},
        tasks=[
            {"task_id": "T-001", "status": "active"},
            {"task_id": "T-002", "status": "planned"},
        ],
    )

    assert resolution["status"] == "ambiguous"
    assert resolution["clarifying_question"] == "這段要接續哪個 task?"
    assert resolution["candidate_targets"] == [
        {"object_type": "task", "object_id": "T-001"},
        {"object_type": "task", "object_id": "T-002"},
    ]
