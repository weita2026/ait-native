use super::*;

pub(in crate::foundation::native_repositories) const REPOSITORY_METADATA_SCHEMA_SQL: &str = r#"
create table if not exists repositories (
    repo_name text primary key,
    repo_id text not null unique,
    default_line text not null,
    lifecycle_state text not null default 'active',
    id_namespace_prefix text not null default '',
    policy_json text not null default '{}',
    created_at timestamptz not null,
    updated_at timestamptz not null
);
"#;

pub(in crate::foundation::native_repositories) const CONTENT_SCHEMA_SQL: &str = r#"
create table if not exists repositories (
    repo_name text primary key,
    repo_id text not null unique,
    default_line text not null,
    lifecycle_state text not null default 'active',
    id_namespace_prefix text not null default '',
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

create table if not exists blobs (
    blob_id text primary key,
    sha256 text not null unique,
    storage_path text not null,
    size_bytes bigint not null,
    storage_kind text not null default 'pack_full',
    pack_id text,
    pack_entry_type text,
    pack_base_blob_id text,
    pack_chain_depth integer,
    pruned_at timestamptz,
    created_at timestamptz not null
);
create index if not exists idx_blobs_pack_id on blobs(pack_id);

create table if not exists blob_locators (
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    blob_id text not null references blobs(blob_id) on delete cascade,
    sha256 text not null,
    storage_path text not null,
    size_bytes bigint not null,
    storage_kind text not null default 'pack_full',
    pack_id text,
    pack_entry_type text,
    pack_base_blob_id text,
    pack_chain_depth integer,
    created_at timestamptz not null,
    primary key (repo_id, blob_id)
);
create index if not exists idx_blob_locators_repo_name on blob_locators(repo_name, blob_id);
create index if not exists idx_blob_locators_blob_id on blob_locators(blob_id);
create index if not exists idx_blob_locators_pack_id on blob_locators(pack_id);

create table if not exists blob_inline_contents (
    blob_id text primary key references blobs(blob_id) on delete cascade,
    sha256 text not null,
    size_bytes bigint not null,
    content bytea not null,
    created_at timestamptz not null
);
create index if not exists idx_blob_inline_contents_sha256 on blob_inline_contents(sha256);

create table if not exists snapshots (
    snapshot_id text primary key,
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    parent_snapshot_id text,
    root_tree_pack_id text,
    root_entry_ordinal bigint,
    manifest_hash text not null default '',
    message text,
    line_name text,
    file_count integer not null,
    total_bytes bigint not null,
    created_at timestamptz not null
);
create index if not exists idx_snapshots_repo_created on snapshots(repo_name, created_at desc);

create table if not exists trees (
    tree_id text primary key,
    entry_count integer not null,
    tree_pack_id text,
    tree_pack_checksum text,
    created_at timestamptz not null
);
create index if not exists idx_trees_tree_pack_id on trees(tree_pack_id);

create table if not exists packs (
    pack_id text primary key,
    repo_name text not null references repositories(repo_name) on delete cascade,
    repo_id text not null,
    status text not null,
    member_count integer not null,
    total_bytes bigint not null,
    pack_path text,
    pack_format text not null default 'ait-pack-v3-zstd-chunked',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at timestamptz not null
);
create index if not exists idx_packs_repo on packs(repo_name, created_at desc);

create table if not exists tree_packs (
    pack_id text primary key,
    repo_name text references repositories(repo_name) on delete cascade,
    repo_id text,
    status text not null,
    tree_count integer not null,
    total_bytes bigint not null,
    pack_path text,
    pack_format text not null default 'ait-tree-pack-v2-zstd-chunked',
    pack_index_entry_name text,
    pack_index_checksum text,
    created_at timestamptz not null
);
create index if not exists idx_tree_packs_repo on tree_packs(repo_id, pack_id);
create index if not exists idx_tree_packs_repo_name on tree_packs(repo_name, pack_id);
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PostgresRepositorySchemaMode {
    FullContent,
    RepositoryMetadataOnly,
}

impl PostgresRepositorySchemaMode {
    fn cache_key(self) -> &'static str {
        match self {
            Self::FullContent => "full-content",
            Self::RepositoryMetadataOnly => "repository-metadata-only",
        }
    }
}

#[derive(Clone)]
pub struct PostgresNativeRepositoryService {
    pub registry: Arc<PostgresConnectionPoolRegistry<NativePostgresDriver>>,
    pub backend: String,
    pub dsn: Option<String>,
    pub content_schema: String,
    pub control_schema: String,
    pub paths: ServerRuntimePaths,
    schema_mode: PostgresRepositorySchemaMode,
    schema_ready: Arc<Mutex<HashSet<String>>>,
}

impl PostgresNativeRepositoryService {
    pub fn new(
        registry: Arc<PostgresConnectionPoolRegistry<NativePostgresDriver>>,
        backend: impl Into<String>,
        dsn: Option<String>,
        content_schema: impl Into<String>,
        control_schema: impl Into<String>,
        paths: ServerRuntimePaths,
    ) -> Self {
        Self::new_with_schema_mode(
            registry,
            backend,
            dsn,
            content_schema,
            control_schema,
            paths,
            PostgresRepositorySchemaMode::FullContent,
        )
    }

    pub fn new_metadata_only(
        registry: Arc<PostgresConnectionPoolRegistry<NativePostgresDriver>>,
        backend: impl Into<String>,
        dsn: Option<String>,
        content_schema: impl Into<String>,
        control_schema: impl Into<String>,
        paths: ServerRuntimePaths,
    ) -> Self {
        Self::new_with_schema_mode(
            registry,
            backend,
            dsn,
            content_schema,
            control_schema,
            paths,
            PostgresRepositorySchemaMode::RepositoryMetadataOnly,
        )
    }

    fn new_with_schema_mode(
        registry: Arc<PostgresConnectionPoolRegistry<NativePostgresDriver>>,
        backend: impl Into<String>,
        dsn: Option<String>,
        content_schema: impl Into<String>,
        control_schema: impl Into<String>,
        paths: ServerRuntimePaths,
        schema_mode: PostgresRepositorySchemaMode,
    ) -> Self {
        Self {
            registry,
            backend: backend.into(),
            dsn,
            content_schema: content_schema.into(),
            control_schema: control_schema.into(),
            paths,
            schema_mode,
            schema_ready: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn schema_cache_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.backend,
            self.dsn.clone().unwrap_or_default(),
            self.content_schema,
            self.schema_mode.cache_key()
        )
    }

    fn ensure_schema(&self) -> Result<(), NativeRepositoryError> {
        let cache_key = self.schema_cache_key();
        if self
            .schema_ready
            .lock()
            .expect("schema cache mutex poisoned")
            .contains(&cache_key)
        {
            return Ok(());
        }
        write_server_plane(
            self.registry.as_ref(),
            &self.backend,
            self.dsn.as_deref(),
            &self.content_schema,
            &self.control_schema,
            "content",
            &PostgresTimeoutScope::default(),
            |conn| {
                match self.schema_mode {
                    PostgresRepositorySchemaMode::FullContent => {
                        conn.raw_mut()
                            .batch_execute(CONTENT_SCHEMA_SQL)
                            .map_err(db_internal)?;
                    }
                    PostgresRepositorySchemaMode::RepositoryMetadataOnly => {
                        conn.raw_mut()
                            .batch_execute(REPOSITORY_METADATA_SCHEMA_SQL)
                            .map_err(db_internal)?;
                    }
                }
                Ok::<(), NativeRepositoryError>(())
            },
        )
        .map_err(NativeRepositoryError::from_wrapped_string)?;
        self.schema_ready
            .lock()
            .expect("schema cache mutex poisoned")
            .insert(cache_key);
        Ok(())
    }

    pub(super) fn with_read<T, F>(&self, callback: F) -> Result<T, NativeRepositoryError>
    where
        F: FnOnce(&mut pg::Client) -> Result<T, NativeRepositoryError>,
    {
        self.ensure_schema()?;
        read_server_plane(
            self.registry.as_ref(),
            &self.backend,
            self.dsn.as_deref(),
            &self.content_schema,
            &self.control_schema,
            "content",
            &PostgresTimeoutScope::default(),
            |conn| callback(conn.raw_mut()),
        )
        .map_err(NativeRepositoryError::from_wrapped_string)
    }

    pub(super) fn with_write<T, F>(&self, callback: F) -> Result<T, NativeRepositoryError>
    where
        F: FnOnce(&mut pg::Client) -> Result<T, NativeRepositoryError>,
    {
        self.ensure_schema()?;
        write_server_plane(
            self.registry.as_ref(),
            &self.backend,
            self.dsn.as_deref(),
            &self.content_schema,
            &self.control_schema,
            "content",
            &PostgresTimeoutScope::default(),
            |conn| callback(conn.raw_mut()),
        )
        .map_err(NativeRepositoryError::from_wrapped_string)
    }

    pub(super) fn with_control_read<T, F>(&self, callback: F) -> Result<T, NativeRepositoryError>
    where
        F: FnOnce(&mut pg::Client) -> Result<T, NativeRepositoryError>,
    {
        self.ensure_schema()?;
        read_server_plane(
            self.registry.as_ref(),
            &self.backend,
            self.dsn.as_deref(),
            &self.content_schema,
            &self.control_schema,
            "control",
            &PostgresTimeoutScope::default(),
            |conn| callback(conn.raw_mut()),
        )
        .map_err(NativeRepositoryError::from_wrapped_string)
    }

    pub(super) fn with_control_write<T, F>(&self, callback: F) -> Result<T, NativeRepositoryError>
    where
        F: FnOnce(&mut pg::Client) -> Result<T, NativeRepositoryError>,
    {
        self.ensure_schema()?;
        write_server_plane(
            self.registry.as_ref(),
            &self.backend,
            self.dsn.as_deref(),
            &self.content_schema,
            &self.control_schema,
            "control",
            &PostgresTimeoutScope::default(),
            |conn| callback(conn.raw_mut()),
        )
        .map_err(NativeRepositoryError::from_wrapped_string)
    }
}

impl NativeRepositoryService for PostgresNativeRepositoryService {
    fn create_repository(
        &self,
        request: RepositoryCreateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.with_write(move |client| create_or_update_repository(client, request))
    }

    fn create_repository_metadata(
        &self,
        request: RepositoryCreateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.with_write(move |client| create_or_update_repository_metadata(client, request))
    }

    fn list_repositories(&self) -> Result<JsonValue, NativeRepositoryError> {
        match self.schema_mode {
            PostgresRepositorySchemaMode::FullContent => self.with_read(list_repositories_json),
            PostgresRepositorySchemaMode::RepositoryMetadataOnly => {
                self.with_read(list_repository_metadata_json)
            }
        }
    }

    fn get_repository(&self, repo_name: &str) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        match self.schema_mode {
            PostgresRepositorySchemaMode::FullContent => {
                self.with_read(move |client| get_repository_json(client, &repo_name))
            }
            PostgresRepositorySchemaMode::RepositoryMetadataOnly => {
                self.with_read(move |client| get_repository_metadata_json(client, &repo_name))
            }
        }
    }

    fn get_repository_by_id(&self, repo_id: &str) -> Result<JsonValue, NativeRepositoryError> {
        let repo_id = repo_id.to_string();
        match self.schema_mode {
            PostgresRepositorySchemaMode::FullContent => {
                self.with_read(move |client| get_repository_json_by_id(client, &repo_id))
            }
            PostgresRepositorySchemaMode::RepositoryMetadataOnly => {
                self.with_read(move |client| get_repository_metadata_json_by_id(client, &repo_id))
            }
        }
    }

    fn list_lines(&self, repo_name: &str) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        self.with_read(move |client| list_lines_json(client, &repo_name))
    }

    fn get_line(
        &self,
        repo_name: &str,
        line_name: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let line_name = line_name.to_string();
        self.with_read(move |client| get_line_json(client, &repo_name, &line_name))
    }

    fn update_line(
        &self,
        repo_name: &str,
        line_name: &str,
        request: LineUpdateRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let line_name = line_name.to_string();
        self.with_write(move |client| update_line_json(client, &repo_name, &line_name, request))
    }

    fn close_line(
        &self,
        repo_name: &str,
        line_name: &str,
        request: LineCloseRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let line_name = line_name.to_string();
        self.with_write(move |client| close_line_json(client, &repo_name, &line_name, request))
    }

    fn retire_repository(
        &self,
        repo_name: &str,
        request: RetireRepositoryRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        retire_repository_json(self, repo_name, request)
    }

    fn snapshot_existence(
        &self,
        repo_name: &str,
        request: SnapshotExistsRequest,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        self.with_read(move |client| snapshot_existence_json(client, &repo_name, request))
    }

    fn zstd_bulk_plan(
        &self,
        repo_name: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        self.with_read(move |client| zstd_bulk_plan_json(client, &repo_name, request))
    }

    fn put_zstd_bulk_object_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let pack_id = pack_id.to_string();
        let paths = self.paths.clone();
        self.with_write(move |client| {
            put_zstd_bulk_object_pack_bytes(client, &paths, &repo_name, &pack_id, &pack_bytes)
        })
    }

    fn get_zstd_bulk_object_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let pack_id = pack_id.to_string();
        let paths = self.paths.clone();
        self.with_read(move |client| {
            get_zstd_bulk_object_pack_bytes(client, &paths, &repo_name, &pack_id)
        })
    }

    fn put_zstd_bulk_tree_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let pack_id = pack_id.to_string();
        let paths = self.paths.clone();
        self.with_write(move |client| {
            put_zstd_bulk_tree_pack_bytes(client, &paths, &repo_name, &pack_id, &pack_bytes)
        })
    }

    fn get_zstd_bulk_tree_pack(
        &self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let pack_id = pack_id.to_string();
        let paths = self.paths.clone();
        self.with_read(move |client| {
            get_zstd_bulk_tree_pack_bytes(client, &paths, &repo_name, &pack_id)
        })
    }

    fn get_zstd_import_manifest(
        &self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let snapshot_id = snapshot_id.to_string();
        let paths = self.paths.clone();
        self.with_read(move |client| {
            get_zstd_import_manifest_json(client, &paths, &repo_name, &snapshot_id)
        })
    }

    fn get_zstd_pull_manifest(
        &self,
        repo_name: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let request = RemoteSyncPlanJson::stateless().zstd_pull_manifest_request(&request)?;
        let repo_name = repo_name.to_string();
        let paths = self.paths.clone();
        self.with_read(move |client| {
            get_zstd_pull_manifest_json(client, &paths, &repo_name, &request)
        })
    }

    fn commit_zstd_bulk(
        &self,
        repo_name: &str,
        request: JsonValue,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let paths = self.paths.clone();
        self.with_write(move |client| zstd_bulk_commit_json(client, &paths, &repo_name, request))
    }

    fn export_snapshot(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        query: SnapshotExportQuery,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let snapshot_id = snapshot_id.to_string();
        let paths = self.paths.clone();
        self.with_read(move |client| {
            export_snapshot_json(client, &paths, &repo_name, &snapshot_id, query)
        })
    }

    fn materialize_snapshot(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let snapshot_id = snapshot_id.to_string();
        let destination = destination.to_path_buf();
        let paths = self.paths.clone();
        self.with_read(move |client| {
            materialize_snapshot_json(client, &paths, &repo_name, &snapshot_id, &destination)
        })
    }

    fn materialize_snapshot_paths(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        relative_paths: &[PathBuf],
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let snapshot_id = snapshot_id.to_string();
        let destination = destination.to_path_buf();
        let relative_paths = relative_paths.to_vec();
        let paths = self.paths.clone();
        self.with_read(move |client| {
            materialize_snapshot_paths_json(
                client,
                &paths,
                &repo_name,
                &snapshot_id,
                &destination,
                &relative_paths,
            )
        })
    }

    fn materialize_snapshot_manifest_entries(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        entries: &[SnapshotManifestFileEntry],
    ) -> Result<JsonValue, NativeRepositoryError> {
        let repo_name = repo_name.to_string();
        let snapshot_id = snapshot_id.to_string();
        let destination = destination.to_path_buf();
        let entries = entries.to_vec();
        let paths = self.paths.clone();
        self.with_read(move |client| {
            materialize_snapshot_manifest_entries_json(
                client,
                &paths,
                &repo_name,
                &snapshot_id,
                &destination,
                &entries,
            )
        })
    }
}
