begin;
create schema if not exists "ait_native_content";
set search_path to "ait_native_content", public;

create table if not exists schema_versions (
    plane text primary key,
    version integer not null,
    description text not null,
    applied_at timestamptz not null,
    checked_at timestamptz not null
);

create table if not exists repositories (
    repo_name text primary key,
    repo_id text not null unique,
    default_line text not null,
    id_namespace_prefix text not null default 'AIT',
    policy_json text not null default '{}',
    created_at timestamptz not null,
    updated_at timestamptz not null
);

create table if not exists lines (
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    line_name text not null,
    head_snapshot_id text,
    status text not null default 'active',
    archived_at timestamptz,
    created_at timestamptz not null,
    updated_at timestamptz not null,
    primary key (repo_id, line_name)
);
create index if not exists idx_lines_repo on lines(repo_name, line_name);
create index if not exists idx_lines_repo_id on lines(repo_id, line_name);
create index if not exists idx_lines_repo_id_head_snapshot on lines(repo_id, head_snapshot_id);

create table if not exists blobs (
    blob_id text primary key,
    sha256 text not null unique,
    storage_path text not null,
    size_bytes bigint not null,
    storage_kind text not null default 'loose',
    pack_id text,
    pack_entry_name text,
    pack_entry_type text,
    pack_base_blob_id text,
    pack_chain_depth integer,
    packed_at timestamptz,
    pruned_at timestamptz,
    created_at timestamptz not null
);
create index if not exists idx_blobs_pack_id on blobs(pack_id);

create table if not exists snapshots (
    snapshot_id text primary key,
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    parent_snapshot_id text,
    root_tree_id text,
    manifest_hash text not null default '',
    manifest_path text not null default '',
    message text,
    line_name text,
    file_count integer not null,
    total_bytes bigint not null,
    created_at timestamptz not null
);
create index if not exists idx_snapshots_repo_created on snapshots(repo_name, created_at desc);
create index if not exists idx_snapshots_repo_id_created on snapshots(repo_id, created_at desc);

create table if not exists trees (
    tree_id text primary key,
    entry_count integer not null,
    tree_pack_id text,
    tree_pack_entry_name text,
    tree_pack_checksum text,
    tree_packed_at timestamptz,
    created_at timestamptz not null
);
create index if not exists idx_trees_tree_pack_id on trees(tree_pack_id);

create table if not exists tree_entries (
    tree_id text not null references trees(tree_id) on delete cascade,
    entry_name text not null,
    entry_type text not null,
    target_id text not null,
    size_bytes bigint,
    mode text not null,
    primary key (tree_id, entry_name)
);
create index if not exists idx_tree_entries_target on tree_entries(target_id);

create or replace view snapshot_files as
with recursive snapshot_walk(snapshot_id, prefix, entry_name, entry_type, target_id, size_bytes, mode) as (
    select
        s.snapshot_id,
        '' as prefix,
        te.entry_name,
        te.entry_type,
        te.target_id,
        te.size_bytes,
        te.mode
    from snapshots s
    join tree_entries te on te.tree_id = s.root_tree_id
  union all
    select
        sw.snapshot_id,
        sw.prefix || sw.entry_name || '/',
        te.entry_name,
        te.entry_type,
        te.target_id,
        te.size_bytes,
        te.mode
    from snapshot_walk sw
    join tree_entries te on te.tree_id = sw.target_id
    where sw.entry_type = 'tree'
)
select
    snapshot_id,
    prefix || entry_name as path,
    target_id as blob_id,
    size_bytes,
    mode
from snapshot_walk
where entry_type = 'blob';

create table if not exists packs (
    pack_id text primary key,
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    status text not null,
    member_count integer not null,
    total_bytes bigint not null,
    pack_path text,
    pack_format text not null default 'ait-pack-v1',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at timestamptz not null
);
create index if not exists idx_packs_repo on packs(repo_name, created_at desc);
create index if not exists idx_packs_repo_id on packs(repo_id, created_at desc);

create table if not exists tree_packs (
    pack_id text primary key,
    status text not null,
    tree_count integer not null,
    total_bytes bigint not null,
    pack_path text,
    pack_format text not null default 'ait-tree-pack-v1',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at timestamptz not null
);

create table if not exists repository_groups (
    group_id text primary key,
    title text not null,
    sort_index integer not null,
    system_slug text unique,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_repository_groups_sort on repository_groups(sort_index, group_id);

create table if not exists repository_group_memberships (
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text primary key,
    group_id text not null references repository_groups(group_id) on delete cascade,
    sort_index integer not null,
    created_at timestamptz not null,
    updated_at timestamptz not null
);
create index if not exists idx_repository_group_memberships_group on repository_group_memberships(group_id, sort_index, repo_name);
create index if not exists idx_repository_group_memberships_group_repo_id on repository_group_memberships(group_id, sort_index, repo_id);

commit;
