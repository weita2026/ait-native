from __future__ import annotations

import json
from typing import Any, Callable, Iterable, TypeVar

from ait_protocol.common import extract_plan_items, normalize_plan_items, utc_now
from .server_content_repo_lines import get_repository
from .server_db import (
    connect_server_plane,
    ensure_schema_version,
    postgres_advisory_lock,
    read_server_plane,
    write_server_plane,
)
from .server_paths import ServerContext

_T = TypeVar("_T")

SCHEMA_POSTGRES = """
create table if not exists schema_versions (
    plane text primary key,
    version integer not null,
    description text not null,
    applied_at timestamptz not null,
    checked_at timestamptz not null
);

create table if not exists tasks (
    task_id text primary key,
    repo_name text not null,
    repo_id text,
    task_seq integer,
    title text not null,
    intent text not null,
    risk_tier text not null,
    planning_state text not null default 'unplanned',
    plan_id text,
    origin_plan_revision_id text,
    plan_item_ref text,
    plan_section_ref text,
    plan_drift_state text,
    plan_linked_at timestamptz,
    status text not null,
    created_at timestamptz not null
);
create index if not exists idx_tasks_repo_created on tasks(repo_name, created_at desc);
alter table if exists tasks add column if not exists plan_id text;
alter table if exists tasks add column if not exists origin_plan_revision_id text;
alter table if exists tasks add column if not exists plan_item_ref text;
alter table if exists tasks add column if not exists plan_section_ref text;
alter table if exists tasks add column if not exists plan_drift_state text;
alter table if exists tasks add column if not exists plan_linked_at timestamptz;
alter table if exists tasks add column if not exists planning_state text not null default 'unplanned';

create table if not exists plans (
    plan_id text primary key,
    repo_name text not null,
    repo_id text,
    title text not null,
    status text not null,
    head_revision_id text,
    created_by text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_plans_repo_updated on plans(repo_name, updated_at desc);
alter table if exists plans add column if not exists repo_id text;
create index if not exists idx_plans_repo_id_updated on plans(repo_id, updated_at desc);

create table if not exists plan_revisions (
    plan_revision_id text primary key,
    plan_id text not null,
    revision_number integer not null,
    parent_plan_revision_id text,
    title_snapshot text not null,
    summary text,
    artifact_path text,
    artifact_selector text,
    artifact_heading text,
    items_json text not null,
    plan_links_surface_hash text,
    plan_links_changed_count_to_prev integer not null default 0,
    source_kind text not null,
    source_session_id text,
    created_by text not null,
    actor_type text not null,
    created_at timestamptz not null,
    unique(plan_id, revision_number)
);
create index if not exists idx_plan_revisions_plan_created on plan_revisions(plan_id, created_at desc);
alter table if exists plan_revisions add column if not exists artifact_path text;
alter table if exists plan_revisions add column if not exists artifact_selector text;
alter table if exists plan_revisions add column if not exists artifact_heading text;
alter table if exists plan_revisions add column if not exists items_json text not null default '[]';
alter table if exists plan_revisions add column if not exists plan_links_surface_hash text;
alter table if exists plan_revisions add column if not exists plan_links_changed_count_to_prev integer not null default 0;

create table if not exists plan_revision_blobs (
    plan_revision_id text primary key,
    repo_name text not null,
    repo_id text,
    blob_id text not null,
    media_type text not null default 'text/markdown',
    encoding text not null default 'utf-8',
    byte_count bigint,
    created_at timestamptz not null
);
create index if not exists idx_plan_revision_blobs_repo_blob on plan_revision_blobs(repo_name, blob_id);
alter table if exists plan_revision_blobs add column if not exists repo_id text;
create index if not exists idx_plan_revision_blobs_repo_id_blob on plan_revision_blobs(repo_id, blob_id);
alter table if exists plan_revision_blobs add column if not exists media_type text;
alter table if exists plan_revision_blobs add column if not exists encoding text;
alter table if exists plan_revision_blobs add column if not exists byte_count bigint;
alter table if exists plan_revision_blobs add column if not exists created_at timestamptz;

create table if not exists plan_revision_artifacts (
    plan_revision_id text not null,
    artifact_path text not null,
    repo_name text not null,
    repo_id text,
    role text not null,
    blob_id text not null,
    media_type text not null,
    encoding text,
    byte_count bigint not null,
    sha256 text not null,
    metadata_json text not null default '{}',
    created_at timestamptz not null,
    updated_at timestamptz not null,
    primary key (plan_revision_id, artifact_path)
);
create index if not exists idx_plan_revision_artifacts_repo on plan_revision_artifacts(repo_name, artifact_path);
alter table if exists plan_revision_artifacts add column if not exists repo_id text;
create index if not exists idx_plan_revision_artifacts_repo_id on plan_revision_artifacts(repo_id, artifact_path);
alter table if exists plan_revision_artifacts add column if not exists role text;
alter table if exists plan_revision_artifacts add column if not exists media_type text;
alter table if exists plan_revision_artifacts add column if not exists encoding text;
alter table if exists plan_revision_artifacts add column if not exists byte_count bigint;
alter table if exists plan_revision_artifacts add column if not exists sha256 text;
alter table if exists plan_revision_artifacts add column if not exists metadata_json text not null default '{}';
alter table if exists plan_revision_artifacts add column if not exists created_at timestamptz;
alter table if exists plan_revision_artifacts add column if not exists updated_at timestamptz;

create table if not exists changes (
    change_id text primary key,
    repo_name text not null,
    repo_id text,
    change_seq integer,
    task_id text not null,
    title text not null,
    base_line text not null,
    fork_snapshot_id text,
    forked_from_line text,
    risk_tier text not null,
    lane text not null default 'assisted',
    status text not null,
    current_patchset_number integer not null default 0,
    selected_patchset_number integer,
    created_at timestamptz not null,
    updated_at timestamptz not null,
    landed_at timestamptz
);
create index if not exists idx_changes_repo_updated on changes(repo_name, updated_at desc);

create table if not exists releases (
    release_id text primary key,
    repo_name text not null,
    repo_id text,
    version text not null,
    line_name text not null,
    snapshot_id text not null,
    manifest_hash text not null,
    profile text not null,
    package_name text,
    package_version text,
    package_requires_python text,
    status text not null,
    checks_json text not null default '[]',
    artifacts_json text not null default '[]',
    formula_json text not null default '{}',
    metadata_json text not null default '{}',
    created_by text not null,
    actor_type text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null,
    unique(repo_name, version)
);
create index if not exists idx_releases_repo_updated on releases(repo_name, updated_at desc);

create table if not exists sessions (
    session_id text primary key,
    repo_name text not null,
    repo_id text,
    session_local_id text,
    task_id text,
    change_id text,
    title text,
    session_kind text not null,
    status text not null,
    line_name text,
    worktree_name text,
    model_name text,
    actor_identity text,
    actor_type text,
    metadata_json text not null,
    last_event_sequence integer not null default 0,
    head_checkpoint_id text,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_sessions_repo_updated on sessions(repo_name, updated_at desc);
create index if not exists idx_sessions_repo_status on sessions(repo_name, status, updated_at desc);

create table if not exists planning_sessions (
    planning_session_id text primary key,
    repo_name text not null,
    repo_id text,
    planning_session_local_id text,
    plan_id text not null,
    title text,
    mode text not null,
    status text not null,
    preferred_agent text,
    artifact_status text not null,
    derived_task_id text,
    last_promoted_plan_revision_id text,
    last_event_sequence integer not null default 0,
    created_by text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
alter table if exists planning_sessions add column if not exists plan_id text;
alter table if exists planning_sessions add column if not exists derived_task_id text;
alter table if exists planning_sessions add column if not exists last_promoted_plan_revision_id text;
create index if not exists idx_planning_sessions_repo_plan on planning_sessions(repo_name, plan_id, status, updated_at desc);

create table if not exists planning_session_events (
    repo_id text,
    planning_session_id text not null,
    sequence integer not null,
    event_type text not null,
    payload_json text not null,
    actor_identity text not null,
    actor_type text not null,
    created_at timestamptz not null,
    primary key(planning_session_id, sequence)
);
create index if not exists idx_planning_session_events_session_sequence on planning_session_events(planning_session_id, sequence);

create table if not exists session_events (
    repo_id text,
    session_id text not null,
    sequence integer not null,
    event_type text not null,
    payload_json text not null,
    actor_identity text not null,
    actor_type text not null,
    created_at timestamptz not null,
    primary key(session_id, sequence)
);
create index if not exists idx_session_events_session_sequence on session_events(session_id, sequence);

create table if not exists session_checkpoints (
    checkpoint_id text primary key,
    repo_id text,
    checkpoint_local_id text,
    session_id text not null,
    based_on_sequence integer not null,
    summary text not null,
    snapshot_id text,
    resume_payload_json text not null,
    created_at timestamptz not null
);
create index if not exists idx_session_checkpoints_session_created on session_checkpoints(session_id, created_at desc);

create table if not exists patchsets (
    patchset_id text primary key,
    repo_id text,
    change_id text not null,
    patchset_number integer not null,
    base_snapshot_id text not null,
    revision_snapshot_id text not null,
    summary text not null,
    author_mode text not null,
    publish_state text not null,
    diff_stats_json text not null,
    evaluation_state text not null default 'pending',
    created_at timestamptz not null,
    unique(change_id, patchset_number)
);
create index if not exists idx_patchsets_change_num on patchsets(change_id, patchset_number desc);

create table if not exists review_requests (
    review_request_id bigserial primary key,
    repo_id text,
    change_id text not null,
    patchset_id text not null,
    reviewer_group text not null,
    note text,
    created_at timestamptz not null
);
create index if not exists idx_review_requests_change_patchset on review_requests(change_id, patchset_id, review_request_id);

create table if not exists reviews (
    review_id bigserial primary key,
    repo_id text,
    change_id text not null,
    patchset_id text not null,
    reviewer text not null,
    action text not null,
    comment text,
    blocking integer not null default 0,
    created_at timestamptz not null
);
create index if not exists idx_reviews_change_patchset on reviews(change_id, patchset_id, review_id);

create table if not exists attestations (
    attestation_id text primary key,
    repo_id text,
    patchset_id text not null unique,
    author_mode text not null,
    evaluation_summary_json text not null,
    provenance_summary_json text not null,
    detail_json text,
    created_at timestamptz not null,
    updated_at timestamptz not null
);

create table if not exists policy_decisions (
    policy_decision_id bigserial primary key,
    repo_id text,
    patchset_id text not null,
    lane text not null,
    decision text not null,
    checks_json text not null,
    input_fingerprint text,
    created_at timestamptz not null
);
create index if not exists idx_policy_decisions_patchset on policy_decisions(patchset_id, policy_decision_id desc);

create table if not exists waivers (
    waiver_id text primary key,
    repo_id text,
    patchset_id text not null,
    rule_name text not null,
    reason text not null,
    expires_at timestamptz,
    created_at timestamptz not null
);
create index if not exists idx_waivers_patchset on waivers(patchset_id, created_at desc);

create table if not exists land_requests (
    submission_id text primary key,
    repo_id text,
    land_seq integer,
    change_id text not null,
    patchset_id text not null,
    target_line text not null,
    mode text not null,
    status text not null,
    result_json text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_land_requests_change on land_requests(change_id, created_at desc);
create index if not exists idx_land_requests_target_fifo on land_requests(target_line, status, created_at asc, submission_id asc);

create table if not exists stacks (
    stack_id text primary key,
    repo_name text not null,
    repo_id text,
    stack_seq integer,
    title text not null,
    landing_policy text not null,
    status text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_stacks_repo_updated on stacks(repo_name, updated_at desc);

create table if not exists stack_changes (
    repo_id text,
    stack_id text not null,
    change_id text not null,
    position integer not null,
    primary key (stack_id, change_id),
    unique (stack_id, position)
);
create index if not exists idx_stack_changes_change on stack_changes(change_id);

create table if not exists role_bindings (
    binding_id bigserial primary key,
    repo_name text not null,
    repo_id text,
    actor_identity text not null,
    role text not null,
    created_at timestamptz not null,
    unique (repo_name, actor_identity, role)
);
create index if not exists idx_role_bindings_repo_actor on role_bindings(repo_name, actor_identity);
create index if not exists idx_role_bindings_repo_id_actor on role_bindings(repo_id, actor_identity);

create table if not exists community_accounts (
    account_id text primary key,
    email_normalized text not null,
    full_name text not null,
    display_name text,
    organization text,
    role_title text,
    status text not null,
    primary_auth_method text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);

create table if not exists community_password_credentials (
    account_id text primary key references community_accounts(account_id) on delete cascade,
    password_hash text not null,
    password_algo text not null,
    password_params_json text not null,
    password_updated_at timestamptz not null,
    must_rotate integer not null default 0
);

create table if not exists community_web_sessions (
    web_session_id text primary key,
    account_id text not null references community_accounts(account_id) on delete cascade,
    session_source text not null,
    created_at timestamptz not null,
    expires_at timestamptz not null,
    revoked_at timestamptz,
    last_seen_at timestamptz
);

create table if not exists community_external_identities (
    account_id text not null references community_accounts(account_id) on delete cascade,
    provider text not null,
    subject text not null,
    email_normalized text,
    linked_at timestamptz not null,
    primary key (account_id, provider),
    unique (provider, subject)
);

create table if not exists jobs (
    job_id bigserial primary key,
    repo_name text not null,
    repo_id text,
    job_type text not null,
    state text not null,
    payload_json text not null,
    result_json text not null default '{}',
    attempt_count integer not null default 0,
    max_attempts integer not null default 5,
    available_at timestamptz not null,
    locked_at timestamptz,
    locked_by text,
    last_error text,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_jobs_state_available on jobs(state, available_at, job_id);
create index if not exists idx_jobs_repo_state on jobs(repo_name, state, job_id);

create table if not exists repository_retirements (
    retirement_id text primary key,
    repo_name text not null,
    repo_id text not null,
    state text not null,
    actor_identity text not null,
    actor_type text not null,
    export_path text not null,
    manifest_path text not null,
    manifest_sha256 text not null,
    summary_json text not null default '{}',
    created_at timestamptz not null,
    exported_at timestamptz,
    verified_at timestamptz,
    purged_at timestamptz,
    updated_at timestamptz not null,
    last_error text
);
create index if not exists idx_repository_retirements_repo on repository_retirements(repo_id, created_at desc);

create table if not exists events (
    event_id bigserial primary key,
    event_type text not null,
    entity_type text not null,
    entity_id text not null,
    payload_json text not null,
    actor_identity text not null default 'system',
    actor_type text not null default 'system_worker',
    created_at timestamptz not null
);
create index if not exists idx_events_type_created on events(event_type, created_at desc);
create index if not exists idx_events_entity on events(entity_type, entity_id, created_at desc);

create table if not exists authority_maps (
    authority_map_id text primary key,
    repo_name text not null unique,
    repo_id text,
    root_document_path text not null,
    milestone_document_path text not null,
    schema_version integer not null default 1,
    created_at text not null,
    updated_at text not null
);

create table if not exists authority_nodes (
    authority_node_id text primary key,
    authority_map_id text not null references authority_maps(authority_map_id) on delete cascade,
    node_kind text not null,
    parent_node_id text references authority_nodes(authority_node_id) on delete cascade,
    document_path text not null,
    title text not null,
    slug text not null,
    sort_index integer not null,
    connection_mode text not null,
    created_at text not null,
    updated_at text not null,
    unique (authority_map_id, document_path)
);

create table if not exists authority_mutations (
    mutation_id text primary key,
    authority_map_id text not null references authority_maps(authority_map_id) on delete cascade,
    authority_node_id text references authority_nodes(authority_node_id) on delete cascade,
    mutation_kind text not null,
    payload_json text not null,
    actor_label text not null,
    created_at timestamptz not null
);

create index if not exists idx_authority_maps_repo on authority_maps(repo_name);
create index if not exists idx_authority_maps_repo_id on authority_maps(repo_id, repo_name);
create index if not exists idx_authority_nodes_map_parent_sort on authority_nodes(authority_map_id, parent_node_id, sort_index);
create index if not exists idx_authority_nodes_document_path on authority_nodes(document_path);
create index if not exists idx_authority_mutations_map_created on authority_mutations(authority_map_id, created_at desc);
create index if not exists idx_authority_mutations_node on authority_mutations(authority_node_id);
"""


def initialize(ctx: ServerContext) -> None:
    conn = connect(ctx)
    try:
        with postgres_advisory_lock(conn, scope=f"{ctx.control_schema}:server-control-initialize"):
            conn.executescript(SCHEMA_POSTGRES)
            _ensure_column(conn, "events", "actor_identity", "text not null default 'system'")
            _ensure_column(conn, "events", "actor_type", "text not null default 'system_worker'")
            _ensure_column(conn, "role_bindings", "repo_id", "text")
            _ensure_column(conn, "authority_maps", "repo_id", "text")
            _ensure_column(conn, "tasks", "repo_id", "text")
            _ensure_column(conn, "tasks", "task_seq", "integer")
            _ensure_column(conn, "tasks", "plan_id", "text")
            _ensure_column(conn, "tasks", "origin_plan_revision_id", "text")
            _ensure_column(conn, "tasks", "plan_item_ref", "text")
            _ensure_column(conn, "tasks", "plan_section_ref", "text")
            _ensure_column(conn, "tasks", "plan_drift_state", "text")
            _ensure_column(conn, "tasks", "plan_linked_at", "text")
            _ensure_column(conn, "tasks", "planning_state", "text not null default 'unplanned'")
            _ensure_column(conn, "plans", "repo_id", "text")
            _ensure_column(conn, "releases", "repo_id", "text")
            _ensure_column(conn, "releases", "checks_json", "text not null default '[]'")
            _ensure_column(conn, "releases", "artifacts_json", "text not null default '[]'")
            _ensure_column(conn, "releases", "formula_json", "text not null default '{}'")
            _ensure_column(conn, "releases", "metadata_json", "text not null default '{}'")
            _ensure_column(conn, "changes", "repo_id", "text")
            _ensure_column(conn, "changes", "change_seq", "integer")
            _ensure_column(conn, "changes", "fork_snapshot_id", "text")
            _ensure_column(conn, "changes", "forked_from_line", "text")
            _ensure_column(conn, "sessions", "repo_id", "text")
            _ensure_column(conn, "sessions", "session_local_id", "text")
            _ensure_column(conn, "planning_sessions", "plan_id", "text")
            _ensure_column(conn, "planning_sessions", "repo_id", "text")
            _ensure_column(conn, "planning_sessions", "planning_session_local_id", "text")
            _ensure_column(conn, "planning_sessions", "derived_task_id", "text")
            _ensure_column(conn, "planning_sessions", "last_promoted_plan_revision_id", "text")
            _ensure_column(conn, "planning_session_events", "repo_id", "text")
            _ensure_column(conn, "session_events", "repo_id", "text")
            _ensure_column(conn, "session_checkpoints", "repo_id", "text")
            _ensure_column(conn, "session_checkpoints", "checkpoint_local_id", "text")
            _ensure_column(conn, "patchsets", "repo_id", "text")
            _ensure_column(conn, "review_requests", "repo_id", "text")
            _ensure_column(conn, "reviews", "repo_id", "text")
            _ensure_column(conn, "attestations", "repo_id", "text")
            _ensure_column(conn, "policy_decisions", "repo_id", "text")
            _ensure_column(conn, "policy_decisions", "input_fingerprint", "text")
            _ensure_column(conn, "waivers", "repo_id", "text")
            _ensure_column(conn, "land_requests", "repo_id", "text")
            _ensure_column(conn, "land_requests", "land_seq", "integer")
            _ensure_column(conn, "stacks", "repo_id", "text")
            _ensure_column(conn, "stacks", "stack_seq", "integer")
            _ensure_column(conn, "stack_changes", "repo_id", "text")
            _ensure_column(conn, "jobs", "repo_id", "text")
            _ensure_column(conn, "jobs", "result_json", "text not null default '{}'")
            _ensure_column(conn, "jobs", "attempt_count", "integer not null default 0")
            _ensure_column(conn, "jobs", "max_attempts", "integer not null default 5")
            _ensure_column(conn, "jobs", "available_at", "text not null default ''")
            _ensure_column(conn, "jobs", "locked_at", "text")
            _ensure_column(conn, "jobs", "locked_by", "text")
            _ensure_column(conn, "jobs", "last_error", "text")
            _ensure_column(conn, "jobs", "updated_at", "text not null default ''")
            _ensure_column(conn, "plan_revisions", "artifact_path", "text")
            _ensure_column(conn, "plan_revisions", "artifact_selector", "text")
            _ensure_column(conn, "plan_revisions", "artifact_heading", "text")
            _ensure_column(conn, "plan_revisions", "items_json", "text not null default '[]'")
            _ensure_column(conn, "plan_revisions", "plan_links_surface_hash", "text")
            _ensure_column(conn, "plan_revisions", "plan_links_changed_count_to_prev", "integer not null default 0")
            _ensure_column(conn, "plan_revision_blobs", "repo_id", "text")
            _ensure_column(conn, "plan_revision_blobs", "media_type", "text")
            _ensure_column(conn, "plan_revision_blobs", "encoding", "text")
            _ensure_column(conn, "plan_revision_blobs", "byte_count", "integer")
            _ensure_column(conn, "plan_revision_blobs", "created_at", "text")
            _ensure_column(conn, "plan_revision_artifacts", "repo_id", "text")
            _ensure_column(conn, "plan_revision_artifacts", "role", "text")
            _ensure_column(conn, "plan_revision_artifacts", "media_type", "text")
            _ensure_column(conn, "plan_revision_artifacts", "encoding", "text")
            _ensure_column(conn, "plan_revision_artifacts", "byte_count", "integer")
            _ensure_column(conn, "plan_revision_artifacts", "sha256", "text")
            _ensure_column(conn, "plan_revision_artifacts", "metadata_json", "text not null default '{}'")
            _ensure_column(conn, "plan_revision_artifacts", "created_at", "text")
            _ensure_column(conn, "plan_revision_artifacts", "updated_at", "text")
            _migrate_plan_revisions(conn)
            _remove_imported_completion_source_lineage_columns(conn)
            _remove_historical_publication_storage(conn)
            repository_ids = _content_repository_ids(ctx)
            _migrate_role_bindings_repo_id(conn, repository_ids)
            _migrate_authority_maps_repo_id(conn, repository_ids)
            conn.execute("create index if not exists idx_jobs_state_available on jobs(state, available_at, job_id)")
            conn.execute("create index if not exists idx_jobs_repo_state on jobs(repo_name, state, job_id)")
            conn.execute("create index if not exists idx_tasks_repo_created on tasks(repo_name, created_at desc)")
            conn.execute("create index if not exists idx_tasks_repo_id_created on tasks(repo_id, created_at desc)")
            conn.execute("create unique index if not exists uq_tasks_repo_id_task_seq on tasks(repo_id, task_seq)")
            conn.execute("create index if not exists idx_plans_repo_id_updated on plans(repo_id, updated_at desc)")
            conn.execute("create index if not exists idx_releases_repo_id_updated on releases(repo_id, updated_at desc)")
            conn.execute("create unique index if not exists uq_releases_repo_id_version on releases(repo_id, version)")
            conn.execute("create index if not exists idx_changes_repo_id_updated on changes(repo_id, updated_at desc)")
            conn.execute("create unique index if not exists uq_changes_repo_id_change_seq on changes(repo_id, change_seq)")
            conn.execute("create index if not exists idx_sessions_repo_id_updated on sessions(repo_id, updated_at desc)")
            conn.execute("create index if not exists idx_sessions_repo_id_status on sessions(repo_id, status, updated_at desc)")
            conn.execute("create unique index if not exists uq_sessions_repo_id_local_id on sessions(repo_id, session_local_id)")
            conn.execute("create index if not exists idx_plans_repo_updated on plans(repo_name, updated_at desc)")
            conn.execute("create index if not exists idx_plan_revisions_plan_created on plan_revisions(plan_id, created_at desc)")
            conn.execute("create index if not exists idx_plan_revision_blobs_repo_blob on plan_revision_blobs(repo_name, blob_id)")
            conn.execute("create index if not exists idx_plan_revision_blobs_repo_id_blob on plan_revision_blobs(repo_id, blob_id)")
            conn.execute("create index if not exists idx_plan_revision_artifacts_repo on plan_revision_artifacts(repo_name, artifact_path)")
            conn.execute("create index if not exists idx_plan_revision_artifacts_repo_id on plan_revision_artifacts(repo_id, artifact_path)")
            conn.execute("create index if not exists idx_planning_sessions_repo_plan on planning_sessions(repo_name, plan_id, status, updated_at desc)")
            conn.execute("create index if not exists idx_planning_sessions_repo_id_plan on planning_sessions(repo_id, plan_id, status, updated_at desc)")
            conn.execute("create unique index if not exists uq_planning_sessions_repo_id_local_id on planning_sessions(repo_id, planning_session_local_id)")
            conn.execute("create index if not exists idx_planning_session_events_repo_id_session_sequence on planning_session_events(repo_id, planning_session_id, sequence)")
            conn.execute("create index if not exists idx_session_events_repo_id_session_sequence on session_events(repo_id, session_id, sequence)")
            conn.execute("create index if not exists idx_session_checkpoints_repo_id_session_created on session_checkpoints(repo_id, session_id, created_at desc)")
            conn.execute("create unique index if not exists uq_session_checkpoints_repo_id_local_id on session_checkpoints(repo_id, checkpoint_local_id)")
            conn.execute("create index if not exists idx_patchsets_repo_id_change_num on patchsets(repo_id, change_id, patchset_number desc)")
            conn.execute("create index if not exists idx_review_requests_repo_id_change_patchset on review_requests(repo_id, change_id, patchset_id, review_request_id)")
            conn.execute("create index if not exists idx_reviews_repo_id_change_patchset on reviews(repo_id, change_id, patchset_id, review_id)")
            conn.execute("create index if not exists idx_attestations_repo_id_patchset on attestations(repo_id, patchset_id)")
            conn.execute("create index if not exists idx_policy_decisions_repo_id_patchset on policy_decisions(repo_id, patchset_id, policy_decision_id desc)")
            conn.execute("create index if not exists idx_waivers_repo_id_patchset on waivers(repo_id, patchset_id, created_at desc)")
            conn.execute("create index if not exists idx_land_requests_repo_id_change on land_requests(repo_id, change_id, created_at desc)")
            conn.execute("create index if not exists idx_land_requests_repo_id_target_fifo on land_requests(repo_id, target_line, status, created_at asc, submission_id asc)")
            conn.execute("create unique index if not exists uq_land_requests_repo_id_land_seq on land_requests(repo_id, land_seq)")
            conn.execute("create index if not exists idx_stacks_repo_id_updated on stacks(repo_id, updated_at desc)")
            conn.execute("create unique index if not exists uq_stacks_repo_id_stack_seq on stacks(repo_id, stack_seq)")
            conn.execute("create index if not exists idx_stack_changes_repo_id_change on stack_changes(repo_id, change_id)")
            conn.execute("create unique index if not exists uq_community_accounts_email on community_accounts(email_normalized)")
            conn.execute("create index if not exists idx_community_accounts_status on community_accounts(status, created_at desc)")
            conn.execute("create index if not exists idx_community_web_sessions_account on community_web_sessions(account_id, expires_at desc)")
            conn.execute("create index if not exists idx_community_web_sessions_expires on community_web_sessions(expires_at)")
            conn.execute("create index if not exists idx_community_external_identities_email on community_external_identities(email_normalized)")
            conn.execute("create index if not exists idx_jobs_repo_id_state on jobs(repo_id, state, job_id)")
            conn.execute("create index if not exists idx_authority_maps_repo on authority_maps(repo_name)")
            conn.execute("create index if not exists idx_authority_maps_repo_id on authority_maps(repo_id, repo_name)")
            conn.execute("create index if not exists idx_role_bindings_repo_actor on role_bindings(repo_name, actor_identity)")
            conn.execute("create index if not exists idx_role_bindings_repo_id_actor on role_bindings(repo_id, actor_identity)")
            conn.execute("create index if not exists idx_authority_nodes_map_parent_sort on authority_nodes(authority_map_id, parent_node_id, sort_index)")
            conn.execute("create index if not exists idx_authority_nodes_document_path on authority_nodes(document_path)")
            conn.execute("create index if not exists idx_authority_mutations_map_created on authority_mutations(authority_map_id, created_at desc)")
            conn.execute("create index if not exists idx_authority_mutations_node on authority_mutations(authority_node_id)")
            conn.execute("drop index if exists idx_land_requests_target_queue")
            conn.execute("create index if not exists idx_land_requests_target_fifo on land_requests(target_line, status, created_at asc, submission_id asc)")
            ensure_schema_version(conn, plane="control")
            conn.commit()
    finally:
        conn.close()



def _ensure_column(conn, table_name: str, column_name: str, ddl: str) -> None:
    cols = _table_columns(conn, table_name)
    if column_name not in cols:
        conn.execute(f"alter table if exists {table_name} add column if not exists {column_name} {ddl}")


def _table_columns(conn, table_name: str) -> set[str]:
    rows = conn.execute(
        """
        select column_name
        from information_schema.columns
        where table_schema = current_schema() and table_name = %s
        """,
        (table_name,),
    ).fetchall()
    return {row["column_name"] for row in rows}


def _content_repository_ids(ctx: ServerContext) -> dict[str, str]:
    repo_ids: dict[str, str] = {}
    try:
        content_conn = connect_server_plane(ctx, "content")
    except Exception:
        return repo_ids
    with content_conn:
        try:
            rows = content_conn.execute("select repo_name, repo_id from repositories").fetchall()
        except Exception:
            return repo_ids
    for row in rows:
        repo_name = str(row["repo_name"] or "").strip()
        repo_id = str(row["repo_id"] or "").strip()
        if repo_name and repo_id:
            repo_ids[repo_name] = repo_id
    return repo_ids


def _repo_id_for_repo_name(ctx: ServerContext, repo_name: str) -> str | None:
    if repo_name == "*":
        return None
    try:
        repository = get_repository(ctx, repo_name)
    except KeyError:
        return None
    repo_id = str(repository.get("repo_id") or "").strip()
    return repo_id or None


def _migrate_role_bindings_repo_id(conn, repo_ids: dict[str, str]) -> None:
    columns = _table_columns(conn, "role_bindings")
    if "repo_id" not in columns:
        return
    for row in conn.execute("select binding_id, repo_name from role_bindings where repo_id is null").fetchall():
        repo_name = str(row["repo_name"] or "").strip()
        repo_id = repo_ids.get(repo_name, "")
        if not repo_id or repo_name == "*":
            continue
        conn.execute(
            "update role_bindings set repo_id = ? where binding_id = ?",
            (repo_id, row["binding_id"]),
        )


def _migrate_authority_maps_repo_id(conn, repo_ids: dict[str, str]) -> None:
    columns = _table_columns(conn, "authority_maps")
    if "repo_id" not in columns:
        return
    for row in conn.execute("select authority_map_id, repo_name from authority_maps where repo_id is null").fetchall():
        repo_name = str(row["repo_name"] or "").strip()
        repo_id = repo_ids.get(repo_name, "")
        if not repo_id:
            continue
        conn.execute(
            "update authority_maps set repo_id = ? where authority_map_id = ?",
            (repo_id, row["authority_map_id"]),
        )


def _drop_column_if_exists(conn, table_name: str, column_name: str) -> None:
    if column_name not in _table_columns(conn, table_name):
        return
    try:
        conn.execute(f"alter table if exists {table_name} drop column if exists {column_name}")
    except Exception:
        pass


def _remove_historical_publication_storage(conn) -> None:
    _drop_column_if_exists(conn, "tasks", "historical_publication_id")
    _drop_column_if_exists(conn, "changes", "historical_publication_id")
    conn.execute("drop table if exists historical_publication_items")
    conn.execute("drop table if exists historical_publications")
    conn.execute("drop index if exists idx_historical_publication_items_repo")
    conn.execute("drop index if exists idx_historical_publications_repo_created")
    conn.execute("drop index if exists uq_historical_publications_repo_id_seq")
    conn.execute("drop index if exists uq_historical_publications_repo_id_idempotency")


def _remove_imported_completion_source_lineage_columns(conn) -> None:
    for column_name in (
        "source_completion_mode",
        "source_local_task_id",
        "source_local_completed_at",
    ):
        _drop_column_if_exists(conn, "tasks", column_name)
    for column_name in (
        "source_completion_mode",
        "source_local_change_id",
        "source_local_status",
        "source_target_line",
        "source_landed_snapshot_id",
        "source_landed_at",
    ):
        _drop_column_if_exists(conn, "changes", column_name)


def _migrate_plan_revisions(conn) -> None:
    columns = _table_columns(conn, "plan_revisions")
    if "body_markdown" not in columns:
        return
    rows = conn.execute(
        """
        select plan_revision_id, title_snapshot, body_markdown, items_json, artifact_heading
        from plan_revisions
        """
    ).fetchall()
    for row in rows:
        items_json = row.get("items_json")
        if str(items_json or "").strip() not in {"", "[]"}:
            continue
        conn.execute(
            """
            update plan_revisions
            set items_json = ?,
                artifact_heading = coalesce(artifact_heading, ?)
            where plan_revision_id = ?
            """,
            (
                json.dumps(normalize_plan_items(extract_plan_items(row["body_markdown"])), sort_keys=True),
                row.get("artifact_heading") or row["title_snapshot"],
                row["plan_revision_id"],
            ),
        )
    _drop_column_if_exists(conn, "plan_revisions", "body_markdown")


def connect(ctx: ServerContext):
    return connect_server_plane(ctx, "control")


def read(
    ctx: ServerContext,
    callback: Callable[[Any], _T],
    *,
    lock_timeout_ms: int | None = None,
    statement_timeout_ms: int | None = None,
) -> _T:
    return read_server_plane(
        ctx,
        "control",
        callback,
        lock_timeout_ms=lock_timeout_ms,
        statement_timeout_ms=statement_timeout_ms,
    )


def write(
    ctx: ServerContext,
    callback: Callable[[Any], _T],
    *,
    lock_timeout_ms: int | None = None,
    statement_timeout_ms: int | None = None,
) -> _T:
    return write_server_plane(
        ctx,
        "control",
        callback,
        lock_timeout_ms=lock_timeout_ms,
        statement_timeout_ms=statement_timeout_ms,
    )



def record_event(
    conn,
    event_type: str,
    entity_type: str,
    entity_id: str,
    payload: dict,
    *,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> None:
    conn.execute(
        "insert into events(event_type, entity_type, entity_id, payload_json, actor_identity, actor_type, created_at) values (?, ?, ?, ?, ?, ?, ?)",
        (event_type, entity_type, entity_id, json.dumps(payload, sort_keys=True), actor_identity, actor_type, utc_now()),
    )



def latest_policy_status(conn, patchset_id: str) -> dict[str, Any] | None:
    row = conn.execute(
        "select lane, decision, checks_json, input_fingerprint, created_at from policy_decisions where patchset_id = ? order by policy_decision_id desc limit 1",
        (patchset_id,),
    ).fetchone()
    if row is None:
        return None
    return {
        "patchset_id": patchset_id,
        "lane": row["lane"],
        "decision": row["decision"],
        "checks": json.loads(row["checks_json"]),
        "input_fingerprint": row["input_fingerprint"],
        "evaluated_at": row["created_at"],
    }



def grant_role_bindings(ctx: ServerContext, repo_name: str, actor_identity: str, roles: Iterable[str]) -> list[dict[str, Any]]:
    repo_id = _repo_id_for_repo_name(ctx, repo_name)
    with connect(ctx) as conn:
        now = utc_now()
        for role in roles:
            conn.execute(
                "insert or ignore into role_bindings(repo_name, repo_id, actor_identity, role, created_at) values (?, ?, ?, ?, ?)",
                (repo_name, repo_id, actor_identity, role, now),
            )
        conn.commit()
        if repo_id is None:
            rows = [
                dict(r)
                for r in conn.execute(
                    "select repo_name, repo_id, actor_identity, role, created_at from role_bindings "
                    "where repo_name = ? and actor_identity = ? order by role",
                    (repo_name, actor_identity),
                )
            ]
        else:
            rows = [
                dict(r)
                for r in conn.execute(
                    "select repo_name, repo_id, actor_identity, role, created_at from role_bindings "
                    "where actor_identity = ? and (repo_id = ? or (repo_id is null and repo_name = ?)) order by role",
                    (actor_identity, repo_id, repo_name),
                )
            ]
    return rows



def list_role_bindings(ctx: ServerContext, repo_name: str) -> list[dict[str, Any]]:
    repo_id = _repo_id_for_repo_name(ctx, repo_name)
    with connect(ctx) as conn:
        if repo_id is None:
            rows = [
                dict(r)
                for r in conn.execute(
                    "select repo_name, repo_id, actor_identity, role, created_at "
                    "from role_bindings "
                    "where repo_name = ? or repo_name = '*' order by actor_identity, role",
                    (repo_name,),
                )
            ]
        else:
            rows = [
                dict(r)
                for r in conn.execute(
                    "select repo_name, repo_id, actor_identity, role, created_at from role_bindings "
                    "where repo_id = ? or (repo_id is null and repo_name = ?) or repo_name = '*' "
                    "order by actor_identity, role",
                    (repo_id, repo_name),
                )
            ]
    return rows


def resolve_bound_roles(
    conn,
    repo_name: str,
    actor_identity: str,
    repo_id: str | None = None,
) -> set[str]:
    if repo_id:
        rows = conn.execute(
            "select role from role_bindings "
            "where actor_identity = ? and (repo_id = ? or repo_name = ? or repo_name = '*')",
            (actor_identity, repo_id, repo_name),
        ).fetchall()
    else:
        rows = conn.execute(
            "select role from role_bindings where actor_identity = ? and (repo_name = ? or repo_name = '*')",
            (actor_identity, repo_name),
        ).fetchall()
    return {row["role"] for row in rows}
