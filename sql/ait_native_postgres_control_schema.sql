begin;
create schema if not exists "ait_native_control";
set search_path to "ait_native_control", public;

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
    source_completion_mode text,
    source_local_task_id text,
    source_local_completed_at timestamptz,
    historical_publication_id text,
    status text not null,
    created_at timestamptz not null
);
create index if not exists idx_tasks_repo_created on tasks(repo_name, created_at desc);
create index if not exists idx_tasks_repo_id_created on tasks(repo_id, created_at desc);
create unique index if not exists uq_tasks_repo_id_task_seq on tasks(repo_id, task_seq);
alter table if exists tasks add column if not exists repo_id text;
alter table if exists tasks add column if not exists task_seq integer;
alter table if exists tasks add column if not exists plan_id text;
alter table if exists tasks add column if not exists origin_plan_revision_id text;
alter table if exists tasks add column if not exists plan_item_ref text;
alter table if exists tasks add column if not exists plan_section_ref text;
alter table if exists tasks add column if not exists plan_drift_state text;
alter table if exists tasks add column if not exists plan_linked_at timestamptz;
alter table if exists tasks add column if not exists planning_state text not null default 'unplanned';
alter table if exists tasks add column if not exists source_completion_mode text;
alter table if exists tasks add column if not exists source_local_task_id text;
alter table if exists tasks add column if not exists source_local_completed_at timestamptz;
alter table if exists tasks add column if not exists historical_publication_id text;

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
    source_kind text not null,
    source_session_id text,
    created_by text not null,
    actor_type text not null,
    created_at timestamptz not null,
    unique(plan_id, revision_number)
);
create index if not exists idx_plan_revisions_plan_created on plan_revisions(plan_id, created_at desc);

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
    source_completion_mode text,
    source_local_change_id text,
    source_local_status text,
    source_target_line text,
    source_landed_snapshot_id text,
    source_landed_at timestamptz,
    historical_publication_id text,
    status text not null,
    current_patchset_number integer not null default 0,
    selected_patchset_number integer,
    created_at timestamptz not null,
    updated_at timestamptz not null,
    landed_at timestamptz
);
create index if not exists idx_changes_repo_updated on changes(repo_name, updated_at desc);
alter table if exists changes add column if not exists repo_id text;
alter table if exists changes add column if not exists change_seq integer;
alter table if exists changes add column if not exists fork_snapshot_id text;
alter table if exists changes add column if not exists forked_from_line text;
alter table if exists changes add column if not exists source_completion_mode text;
alter table if exists changes add column if not exists source_local_change_id text;
alter table if exists changes add column if not exists source_local_status text;
alter table if exists changes add column if not exists source_target_line text;
alter table if exists changes add column if not exists source_landed_snapshot_id text;
alter table if exists changes add column if not exists source_landed_at timestamptz;
alter table if exists changes add column if not exists historical_publication_id text;

create table if not exists sessions (
    session_id text primary key,
    repo_name text not null,
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
    planning_session_id text not null,
    sequence integer not null,
    event_type text not null,
    payload_json text not null,
    actor_identity text not null,
    actor_type text not null,
    created_at timestamptz not null,
    primary key (planning_session_id, sequence)
);
create index if not exists idx_planning_session_events_session_sequence on planning_session_events(planning_session_id, sequence);

create table if not exists session_events (
    session_id text not null,
    sequence integer not null,
    event_type text not null,
    payload_json text not null,
    actor_identity text not null,
    actor_type text not null,
    created_at timestamptz not null,
    primary key (session_id, sequence)
);
create index if not exists idx_session_events_session_sequence on session_events(session_id, sequence);

create table if not exists session_checkpoints (
    checkpoint_id text primary key,
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
    change_id text not null,
    patchset_id text not null,
    reviewer_group text not null,
    note text,
    created_at timestamptz not null
);
create index if not exists idx_review_requests_change_patchset on review_requests(change_id, patchset_id, review_request_id);

create table if not exists reviews (
    review_id bigserial primary key,
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
    patchset_id text not null,
    lane text not null,
    decision text not null,
    checks_json text not null,
    created_at timestamptz not null
);
create index if not exists idx_policy_decisions_patchset on policy_decisions(patchset_id, policy_decision_id desc);

create table if not exists waivers (
    waiver_id text primary key,
    patchset_id text not null,
    rule_name text not null,
    reason text not null,
    expires_at timestamptz,
    created_at timestamptz not null
);
create index if not exists idx_waivers_patchset on waivers(patchset_id, created_at desc);

create table if not exists land_requests (
    submission_id text primary key,
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

create table if not exists historical_publications (
    publication_id text primary key,
    repo_name text not null,
    repo_id text,
    historical_publication_seq integer,
    source_workflow_mode text not null,
    publication_mode text not null,
    status text not null,
    target_line text,
    remote_case text,
    expected_remote_base_snapshot_id text,
    result_remote_snapshot_id text,
    idempotency_key text,
    created_at timestamptz not null,
    completed_at timestamptz
);
create index if not exists idx_historical_publications_repo_created on historical_publications(repo_name, created_at desc);
create unique index if not exists uq_historical_publications_repo_id_seq on historical_publications(repo_id, historical_publication_seq);
create unique index if not exists uq_historical_publications_repo_id_idempotency on historical_publications(repo_id, idempotency_key);

create table if not exists historical_publication_items (
    publication_id text not null,
    repo_id text,
    item_order integer not null,
    object_kind text not null,
    local_ref text not null,
    shared_ref text,
    action text not null,
    status text not null,
    blocker_code text,
    source_payload_json text not null default '{}',
    created_at timestamptz not null,
    primary key (publication_id, item_order)
);
create index if not exists idx_historical_publication_items_repo on historical_publication_items(repo_id, publication_id, item_order);

create table if not exists stacks (
    stack_id text primary key,
    repo_name text not null,
    title text not null,
    landing_policy text not null,
    status text not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_stacks_repo_updated on stacks(repo_name, updated_at desc);

create table if not exists stack_changes (
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

create table if not exists jobs (
    job_id bigserial primary key,
    repo_name text not null,
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
    created_at timestamptz not null,
    updated_at timestamptz not null
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
    created_at timestamptz not null,
    updated_at timestamptz not null,
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

create table if not exists test_case_inventory (
    repo_name text not null,
    repo_id text not null,
    test_case_id text not null,
    pytest_node_id text not null,
    test_file_path text not null,
    class_name text,
    function_name text not null,
    description text not null,
    source_line integer not null,
    discovered_at timestamptz not null,
    updated_at timestamptz not null,
    primary key (repo_id, test_case_id)
);

create table if not exists remote_task_inventory (
    repo_name text not null,
    repo_id text not null,
    task_id text not null,
    task_seq integer,
    title text not null,
    intent text not null,
    risk_tier text not null,
    status text not null,
    planning_state text,
    plan_id text,
    origin_plan_revision_id text,
    plan_item_ref text,
    created_at timestamptz not null,
    synced_at timestamptz not null,
    raw_json jsonb not null,
    primary key (repo_id, task_id)
);

create table if not exists remote_change_inventory (
    repo_name text not null,
    repo_id text not null,
    change_id text not null,
    change_seq integer,
    task_id text not null,
    title text not null,
    base_line text,
    lane text,
    risk_tier text,
    status text not null,
    fork_snapshot_id text,
    forked_from_line text,
    current_patchset_id text,
    current_patchset_number integer,
    selected_patchset_id text,
    selected_patchset_number integer,
    landed_at timestamptz,
    created_at timestamptz not null,
    updated_at timestamptz not null,
    synced_at timestamptz not null,
    raw_json jsonb not null,
    primary key (repo_id, change_id)
);

create table if not exists remote_patchset_inventory (
    repo_name text not null,
    repo_id text not null,
    patchset_id text not null,
    change_id text not null,
    patchset_number integer,
    publish_state text,
    evaluation_state text,
    author_mode text,
    summary text,
    base_snapshot_id text,
    revision_snapshot_id text,
    diff_stats_json jsonb not null,
    created_at timestamptz not null,
    synced_at timestamptz not null,
    raw_json jsonb not null,
    primary key (repo_id, patchset_id)
);

create table if not exists task_test_case_links (
    repo_name text not null,
    repo_id text not null,
    task_id text not null,
    test_case_id text not null,
    pytest_node_id text not null,
    test_file_path text not null,
    class_name text,
    function_name text not null,
    source_line integer not null,
    source_change_ids_json jsonb not null,
    source_patchset_ids_json jsonb not null,
    verification_mode text not null,
    linked_at timestamptz not null,
    primary key (repo_id, task_id, test_case_id)
);

create index if not exists idx_authority_maps_repo on authority_maps(repo_name);
create index if not exists idx_authority_maps_repo_id on authority_maps(repo_id, repo_name);
create index if not exists idx_authority_nodes_map_parent_sort on authority_nodes(authority_map_id, parent_node_id, sort_index);
create index if not exists idx_authority_nodes_document_path on authority_nodes(document_path);
create index if not exists idx_authority_mutations_map_created on authority_mutations(authority_map_id, created_at desc);
create index if not exists idx_authority_mutations_node on authority_mutations(authority_node_id);
create unique index if not exists uq_test_case_inventory_repo_id_test_case_id on test_case_inventory(repo_id, test_case_id);
create unique index if not exists uq_test_case_inventory_repo_id_node on test_case_inventory(repo_id, pytest_node_id);
create index if not exists idx_test_case_inventory_repo_id_path on test_case_inventory(repo_id, test_file_path, source_line);
create index if not exists idx_test_case_inventory_repo_name_path on test_case_inventory(repo_name, test_file_path, source_line);
create index if not exists idx_remote_task_inventory_repo_task_seq on remote_task_inventory(repo_id, task_seq, task_id);
create index if not exists idx_remote_change_inventory_repo_task on remote_change_inventory(repo_id, task_id, change_id);
create index if not exists idx_remote_change_inventory_repo_patchset on remote_change_inventory(repo_id, selected_patchset_id, current_patchset_id);
create index if not exists idx_remote_patchset_inventory_repo_change on remote_patchset_inventory(repo_id, change_id, patchset_number);
create index if not exists idx_task_test_case_links_repo_task on task_test_case_links(repo_id, task_id, test_case_id);
create index if not exists idx_task_test_case_links_repo_test_case on task_test_case_links(repo_id, test_case_id, task_id);

commit;
