use ait_server_core::foundation::remote_binary_db::{
    FilesystemServerRemoteBinaryDb, RepoId, RepoName, StoreGeneration, StorePath,
};
use ait_server_core::foundation::server_binary_db_schema_registry::{
    SERVER_BINARY_DB_BIN_SCHEMAS, SERVER_BINARY_DB_INDEX_SCHEMAS, SERVER_BINARY_DB_LAYOUT_ID,
};
use ait_server_core::foundation::server_binary_lifecycle::SERVER_FRESH_COMPLETION_FILE;
use ait_server_core::foundation::server_content_binary_db::{
    validate_server_snapshot_dag_v0, validate_server_tree_serving_authority_v0,
};
use ait_server_core::foundation::server_operational_job_domain::FrozenBinaryV0WorkerJobAuthority;
use ait_server_core::foundation::server_operational_repository_registry::{
    FreshRepositoryOptions, ServerOperationalRepositoryRegistry, FIXED_REPOSITORY_NAMES,
    PROTOTYPE_POLICY_DEFAULT_FLAGS,
};
use ait_server_core::foundation::server_operational_worker_jobs::{
    ServerOperationalWorkerJobStore, WorkerJobDomainAuthority,
};
use ait_server_core::foundation::workflow_binary_v0_adapter::validate_frozen_server_workflow_v0;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::sync::Arc;

const GENERATION_SCHEMA: &str = "ait.server.binary_v0.operational_generation.v1";
const FRESH_COMPLETION_SCHEMA: &str = "ait.server.binary_v0.fresh.complete.v1";
const FIXED_REPOSITORY_OPTIONS: [FreshRepositoryOptions; 4] = [
    FreshRepositoryOptions {
        namespace_ascii: [b'C', 0],
        policy_flags: PROTOTYPE_POLICY_DEFAULT_FLAGS,
    },
    FreshRepositoryOptions {
        namespace_ascii: *b"SE",
        policy_flags: PROTOTYPE_POLICY_DEFAULT_FLAGS,
    },
    FreshRepositoryOptions {
        namespace_ascii: [b'P', 0],
        policy_flags: PROTOTYPE_POLICY_DEFAULT_FLAGS,
    },
    FreshRepositoryOptions {
        namespace_ascii: [b'N', 0],
        policy_flags: PROTOTYPE_POLICY_DEFAULT_FLAGS,
    },
];

#[derive(Serialize)]
struct GenerationManifest {
    schema: &'static str,
    layout_id: u32,
    status: &'static str,
    global_registry: &'static str,
    repository_authorities: &'static str,
    repository_count: usize,
}

#[derive(Serialize)]
struct FreshCompletion {
    schema: &'static str,
    layout_id: u32,
    status: &'static str,
    generation_manifest_sha256: String,
    repository_count: usize,
}

pub(crate) fn initialize_fresh_generation(
    generation_root: &Path,
    created_at_s: u64,
) -> Result<(), String> {
    if created_at_s == 0 {
        return Err("fresh Binary generation requires non-zero creation time".to_string());
    }
    fs::create_dir(generation_root).map_err(|error| {
        format!(
            "failed to create fresh Binary generation {}: {error}",
            generation_root.display()
        )
    })?;
    let global_root = generation_root.join("global");
    let repositories_root = generation_root.join("repositories");
    fs::create_dir(&global_root)
        .and_then(|_| fs::create_dir(&repositories_root))
        .map_err(|error| format!("failed to create fresh Binary generation roots: {error}"))?;

    let registry = ServerOperationalRepositoryRegistry::new(&global_root, &repositories_root)
        .map_err(|error| format!("construct fresh Repository registry: {error}"))?;
    let entries = registry
        .initialize_fresh_with_options(created_at_s, FIXED_REPOSITORY_OPTIONS)
        .map_err(|error| format!("initialize fresh Repository registry: {error}"))?;
    if entries.len() != FIXED_REPOSITORY_NAMES.len() {
        return Err("fresh Repository registry did not create the four fixed slots".to_string());
    }

    for entry in &entries {
        let authority_root = registry
            .resolve_authority_directory(entry.repository_index)
            .map_err(|error| {
                format!(
                    "resolve fresh Repository {} authority: {error}",
                    entry.repository_index
                )
            })?;
        initialize_repository_authority(&authority_root, entry.repository_index, &entry.repo_name)?;
    }
    registry
        .validate()
        .map_err(|error| format!("validate fresh Repository registry: {error}"))?;

    let manifest = GenerationManifest {
        schema: GENERATION_SCHEMA,
        layout_id: SERVER_BINARY_DB_LAYOUT_ID,
        status: "validated_inactive",
        global_registry: "global",
        repository_authorities: "repositories",
        repository_count: entries.len(),
    };
    let manifest_bytes = pretty_json_line(&manifest)?;
    write_new_sync(&generation_root.join("generation.json"), &manifest_bytes)?;
    let completion = FreshCompletion {
        schema: FRESH_COMPLETION_SCHEMA,
        layout_id: SERVER_BINARY_DB_LAYOUT_ID,
        status: "validated_inactive",
        generation_manifest_sha256: sha256(&manifest_bytes),
        repository_count: entries.len(),
    };
    write_new_sync(
        &generation_root.join(SERVER_FRESH_COMPLETION_FILE),
        &pretty_json_line(&completion)?,
    )?;
    File::open(generation_root)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "failed to sync fresh Binary generation {}: {error}",
                generation_root.display()
            )
        })
}

pub(crate) fn initialize_repository_authority(
    authority_root: &Path,
    repository_index: u32,
    repository_name: &str,
) -> Result<(), String> {
    for relative in SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
    {
        let path = authority_root.join(relative);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => file
                .write_all(&SERVER_BINARY_DB_LAYOUT_ID.to_le_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|error| {
                    format!(
                        "failed to initialize Binary schema file {}: {error}",
                        path.display()
                    )
                })?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "failed to create Binary schema file {}: {error}",
                    path.display()
                ))
            }
        }
    }
    File::open(authority_root)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "failed to sync Binary authority directory {}: {error}",
                authority_root.display()
            )
        })?;

    let db = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new(repository_index.to_string()),
        RepoName::new(repository_name.to_string()),
        StorePath::new(authority_root.to_path_buf()),
        StoreGeneration::new(1),
    );
    validate_frozen_server_workflow_v0(&db)?;
    validate_server_snapshot_dag_v0(&db)
        .map_err(|error| format!("validate fresh Snapshot DAG: {error}"))?;
    validate_server_tree_serving_authority_v0(&db)
        .map_err(|error| format!("validate fresh Tree authority: {error}"))?;
    let authority = Arc::new(FrozenBinaryV0WorkerJobAuthority::new(db));
    let domain: Arc<dyn WorkerJobDomainAuthority> = authority;
    let jobs = ServerOperationalWorkerJobStore::new(
        repository_index,
        authority_root.to_path_buf(),
        domain,
    )
    .map_err(|error| format!("construct fresh Worker Job store: {error}"))?;
    jobs.initialize()
        .map_err(|error| format!("initialize fresh Worker Job store: {error}"))
}

fn pretty_json_line(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("encode JSON: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_new_sync(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
