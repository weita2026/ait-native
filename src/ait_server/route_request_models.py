from __future__ import annotations

from typing import Any

from pydantic import BaseModel, Field

from ait_protocol.common import AuthorMode


class RepositoryCreate(BaseModel):
    repo_name: str
    default_line: str = "main"
    policy: dict[str, Any] = Field(default_factory=dict)
    id_namespace_prefix: str | None = None


class LineUpdate(BaseModel):
    head_snapshot_id: str | None = None
    expected_head_snapshot_id: str | None = None


class LineCloseRequest(BaseModel):
    status: str = "archived"


class SnapshotExistsRequest(BaseModel):
    snapshot_ids: list[str] = Field(default_factory=list)


class TaskCreate(BaseModel):
    task_id: str | None = None
    title: str
    intent: str
    risk_tier: str
    plan_id: str | None = None
    origin_plan_revision_id: str | None = None
    plan_item_ref: str | None = None
    tracking_session: dict[str, Any] | None = None


class TaskTrackingBackfillRequest(BaseModel):
    task_id: str | None = None


class TaskTrackingEnsureRequest(BaseModel):
    tracking_session: dict[str, Any] | None = None


class TaskCloseRequest(BaseModel):
    status: str = "completed"


class ChangeCreate(BaseModel):
    change_id: str | None = None
    task_id: str
    title: str
    base_line: str
    fork_snapshot_id: str | None = None
    forked_from_line: str | None = None
    risk_tier: str


class ChangeCloseRequest(BaseModel):
    status: str = "archived"


class SessionCreate(BaseModel):
    session_id: str | None = None
    session_kind: str = "agent_run"
    task_id: str | None = None
    change_id: str | None = None
    title: str | None = None
    line_name: str | None = None
    worktree_name: str | None = None
    model_name: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)


class SessionCloseRequest(BaseModel):
    status: str = "paused"


class SessionEventAppend(BaseModel):
    event_type: str
    payload: dict[str, Any] = Field(default_factory=dict)


class SessionTurnRequest(BaseModel):
    text: str
    surface: str | None = None
    title: str | None = None
    actor_display_name: str | None = None
    transport_envelope: dict[str, Any] | None = None


class TelegramTurnRequest(BaseModel):
    text: str
    chat_id: str
    chat_title: str | None = None
    chat_type: str | None = None
    telegram_message_id: int | None = None
    telegram_message_ids: list[int] = Field(default_factory=list)
    transport_envelope: dict[str, Any] | None = None


class SessionCheckpointCreate(BaseModel):
    checkpoint_id: str | None = None
    summary: str
    snapshot_id: str | None = None
    resume_payload: dict[str, Any] = Field(default_factory=dict)
    based_on_sequence: int | None = None


class SessionResumeRequest(BaseModel):
    after_sequence: int | None = None
    limit: int = 200


class StackCreate(BaseModel):
    title: str
    change_ids: list[str] = Field(default_factory=list)
    landing_policy: str = "ordered"


class StackUpdate(BaseModel):
    title: str | None = None
    landing_policy: str | None = None
    status: str | None = None


class StackChangeOp(BaseModel):
    change_id: str
    position: int | None = None


class PatchsetPublish(BaseModel):
    base_snapshot_id: str
    revision_snapshot_id: str
    summary: str
    author_mode: AuthorMode = AuthorMode.AI_WITH_HUMAN_REVIEW


class ReleaseArtifactUpload(BaseModel):
    kind: str
    path: str
    sha256: str
    size_bytes: int | None = None
    content_b64: str


class ReleasePublishRequest(BaseModel):
    release_id: str
    version: str
    line: str
    snapshot_id: str
    manifest_hash: str
    profile: str
    package: dict[str, Any] = Field(default_factory=dict)
    checks: list[dict[str, Any]] = Field(default_factory=list)
    artifacts: list[ReleaseArtifactUpload] = Field(default_factory=list)
    formula: dict[str, Any] = Field(default_factory=dict)
    metadata: dict[str, Any] = Field(default_factory=dict)


class SelectPatchsetRequest(BaseModel):
    patchset_id: str


class RunPatchsetCiRequest(BaseModel):
    trigger: str = "manual_rerun"


class RunRepoCiRequest(BaseModel):
    suite_ids: list[str] = Field(default_factory=list)
    plane: str | None = None
    target_line: str = "main"
    trigger: str = "manual_rerun"
    selector: str | None = None
    task_ids: list[str] = Field(default_factory=list)
    curated_corpus: str | None = None
    count: int | None = None
    window_days: int | None = None
    dependency_evidence: list[str] = Field(default_factory=list)
    compliance_evidence: list[str] = Field(default_factory=list)


class RequestReviewRequest(BaseModel):
    patchset_id: str
    reviewer_groups: list[str] = Field(default_factory=list)
    note: str | None = None


class RecordReviewRequest(BaseModel):
    patchset_id: str
    reviewer: str
    action: str
    comment: str | None = None
    blocking: bool = False


class UpsertAttestationRequest(BaseModel):
    author_mode: AuthorMode = AuthorMode.AI_WITH_HUMAN_REVIEW
    evaluation_summary: dict[str, Any]
    provenance_summary: dict[str, Any] = Field(default_factory=dict)
    detail: dict[str, Any] = Field(default_factory=dict)


class CreateWaiverRequest(BaseModel):
    rule_name: str
    reason: str
    expires_at: str | None = None


class SubmitLandRequest(BaseModel):
    patchset_id: str | None = None
    target_line: str = "main"
    mode: str = "direct"


class RetryLandRequest(BaseModel):
    reason: str | None = None


class ReconcileRequest(BaseModel):
    repair: bool = False


class OptimizeRequest(BaseModel):
    repair: bool = True


class PackRequest(BaseModel):
    repack: bool = False
    max_members: int | None = None


class GcRequest(BaseModel):
    prune_unreferenced: bool = True
    prune_orphan_packs: bool = True


class RoleBindingGrant(BaseModel):
    actor_identity: str
    roles: list[str] = Field(default_factory=list)


__all__ = [
    "ChangeCloseRequest",
    "ChangeCreate",
    "CreateWaiverRequest",
    "GcRequest",
    "LineCloseRequest",
    "LineUpdate",
    "OptimizeRequest",
    "PackRequest",
    "PatchsetPublish",
    "RecordReviewRequest",
    "ReconcileRequest",
    "ReleaseArtifactUpload",
    "ReleasePublishRequest",
    "RepositoryCreate",
    "RequestReviewRequest",
    "RetryLandRequest",
    "RoleBindingGrant",
    "RunPatchsetCiRequest",
    "RunRepoCiRequest",
    "SelectPatchsetRequest",
    "SessionCheckpointCreate",
    "SessionCloseRequest",
    "SessionCreate",
    "SessionEventAppend",
    "SessionResumeRequest",
    "SessionTurnRequest",
    "SnapshotExistsRequest",
    "StackChangeOp",
    "StackCreate",
    "StackUpdate",
    "SubmitLandRequest",
    "TaskCloseRequest",
    "TaskCreate",
    "TaskTrackingBackfillRequest",
    "TaskTrackingEnsureRequest",
    "TelegramTurnRequest",
    "UpsertAttestationRequest",
]
