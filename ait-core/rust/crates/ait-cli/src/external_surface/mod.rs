mod render;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use ait_core::external::bindings::{
    CommandExternalBindingToolProbe, ExternalBindingValidationRequest, ExternalBindingValidator,
    FilesystemExternalBindingValidator,
};
use ait_core::external::doctor::{
    build_external_doctor_report, ExternalDoctorOptions, ExternalDoctorSeverity,
};
use ait_core::external::link::{
    remove_external_local_link_override, upsert_external_local_link_override, ExternalLinkStore,
    FsExternalLinkStore, EXTERNAL_LINKS_FILE,
};
use ait_core::external::lockfile::{ExternalLockCodec, ExternalLockfile, TomlExternalLockCodec};
use ait_core::external::manifest::{
    ExternalDeclaration, ExternalManifest, ExternalManifestCodec, TomlExternalManifestCodec,
};
use ait_core::external::materializer::{
    ExternalContentSource, ExternalMaterializationOptions, ExternalMaterializer,
    FilesystemExternalMaterializer,
};
use ait_core::external::resolver::ExternalSnapshotResolver;
use ait_core::external::status::inspect_external_filesystem_status_report;
use ait_core::external::update::{
    run_external_update, ExternalUpdateOptions, ExternalUpdateSelection, ExternalUpdateStore,
    FilesystemExternalUpdateStore,
};
use ait_core::external::{ExternalError, ExternalResult};
use ait_core::json_support::{json, JsonValue};
use ait_core::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotReadStore, LocalSnapshotTreeReadStore,
};
use ait_core::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use ait_core::server_operational::RepositoryIndex;
use ait_core::snapshot_store::SnapshotStore;

use crate::primitives::{
    hydrate_remote_snapshot_boundary_for_repo, remote_sync_snapshot_content_complete_for_repo,
};
use crate::runtime::{
    RepoLocalSnapshotOperationStore, RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT,
};

pub use render::{
    render_external_doctor_text, render_external_link_text, render_external_status_text,
    render_external_text, render_external_unlink_text, render_external_update_text,
};

pub fn external_status(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let report = inspect_external_filesystem_status_report(repo.workspace_root(), repo.repo_name())
        .map_err(|err| err.to_string())?;
    Ok(report.report.to_json_value())
}

pub fn external_doctor(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let status = inspect_external_filesystem_status_report(repo.workspace_root(), repo.repo_name())
        .map_err(|err| err.to_string())?;
    let doctor = build_external_doctor_report(&status.report, &ExternalDoctorOptions::default());
    Ok(doctor.to_json_value())
}

pub fn external_update(
    repo: &RepoRuntime,
    mut options: ExternalUpdateOptions,
) -> Result<JsonValue, String> {
    for link in read_external_local_links(repo)? {
        options = options.with_local_link_override(link.name, link.path);
    }
    hydrate_external_update_snapshots(repo, &options)?;
    let repo_root = repo.workspace_root();
    let store =
        FilesystemExternalUpdateStore::for_repo_root(&repo_root).map_err(|err| err.to_string())?;
    let resolver = SelectedExternalSnapshotResolver::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>::new(repo)?;
    let resolver = RemoteAwareExternalSnapshotResolver::new(
        resolver,
        RepoRuntimeExternalRemoteLineHeadSource::new(repo),
    );
    let content_source =
        SelectedExternalContentSource::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>::new(repo)?;
    let materializer = FilesystemExternalMaterializer::new(&repo_root, content_source)
        .map_err(|err| err.to_string())?;
    let report = run_external_update(&store, &resolver, &materializer, &options)
        .map_err(|err| err.to_string())?;
    let mut payload = report.to_json_value();
    if options.validate {
        let lockfile = store
            .read_lockfile()
            .map_err(|err| err.to_string())?
            .ok_or_else(|| {
                "`ait external update --validate` requires ait-external.lock after update"
                    .to_string()
            })?;
        let validation = validate_external_update_bindings(repo, &lockfile)?;
        if let Some(object) = payload.as_object_mut() {
            object.insert("validation".to_string(), validation);
        }
    }
    Ok(payload)
}

pub(crate) fn materialize_locked_external_release_sources(
    repo: &RepoRuntime,
    lockfile_bytes: &[u8],
    destination_root: &Path,
) -> Result<JsonValue, String> {
    let lockfile = TomlExternalLockCodec
        .parse_lockfile(lockfile_bytes)
        .map_err(|err| err.to_string())?;
    let content_source =
        SelectedExternalContentSource::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>::new(repo)?;
    let materializer = FilesystemExternalMaterializer::new(destination_root, content_source)
        .map_err(|err| err.to_string())?;
    let report = materializer
        .materialize_lockfile(
            &lockfile,
            &ExternalMaterializationOptions::recursive()
                .with_locked(true)
                .with_release_ready(true),
        )
        .map_err(|err| err.to_string())?;
    let mut payload = report.to_json_value();
    if let Some(object) = payload.as_object_mut() {
        object.insert("authority".to_string(), json!("ait-external.lock"));
        object.insert(
            "content_source".to_string(),
            json!("selected_snapshot_store"),
        );
        object.insert("recursive".to_string(), json!(true));
        object.insert("locked".to_string(), json!(true));
        object.insert("release_ready".to_string(), json!(true));
    }
    Ok(payload)
}

trait ExternalUpdateHydrationPorts {
    fn snapshot_content_complete(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> Result<bool, String>;
    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> Result<Option<String>, String>;
    fn snapshot_manifest(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> Result<Option<ExternalManifest>, String>;
    fn import_snapshot(
        &mut self,
        repository_index: u32,
        remote: &str,
        repo_name: &str,
        snapshot: &str,
    ) -> Result<(), String>;
}

struct RepoRuntimeExternalUpdateHydrationPorts<'a> {
    repo: &'a RepoRuntime,
}

impl<'a> RepoRuntimeExternalUpdateHydrationPorts<'a> {
    fn new(repo: &'a RepoRuntime) -> Self {
        Self { repo }
    }

    fn local_resolver(
        &self,
    ) -> Result<SelectedExternalSnapshotResolver<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>, String> {
        SelectedExternalSnapshotResolver::new(self.repo)
    }

    fn remote_aware_resolver(
        &self,
    ) -> Result<
        RemoteAwareExternalSnapshotResolver<
            SelectedExternalSnapshotResolver<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>,
            RepoRuntimeExternalRemoteLineHeadSource<'a>,
        >,
        String,
    > {
        Ok(RemoteAwareExternalSnapshotResolver::new(
            self.local_resolver()?,
            RepoRuntimeExternalRemoteLineHeadSource::new(self.repo),
        ))
    }
}

struct SelectedExternalSnapshotResolver<const WRITE_LAYOUT: u32> {
    current_repo_name: String,
    current_repository_index: Option<u32>,
    store: RepoLocalSnapshotOperationStore<WRITE_LAYOUT>,
    manifest_codec: TomlExternalManifestCodec,
}

impl<const WRITE_LAYOUT: u32> SelectedExternalSnapshotResolver<WRITE_LAYOUT> {
    fn new(repo: &RepoRuntime) -> Result<Self, String> {
        Ok(Self {
            current_repo_name: repo.repo_name(),
            current_repository_index: repo.repository_index().map(|value| value.get()),
            store: selected_external_snapshot_store::<WRITE_LAYOUT>(repo)?,
            manifest_codec: TomlExternalManifestCodec,
        })
    }
}

impl<const WRITE_LAYOUT: u32> ExternalSnapshotResolver
    for SelectedExternalSnapshotResolver<WRITE_LAYOUT>
{
    fn snapshot_exists(
        &self,
        _repository_index: u32,
        _repo_name: &str,
        snapshot: &str,
    ) -> ExternalResult<bool> {
        self.store
            .snapshot_exists(snapshot)
            .map_err(selected_external_store_error)
    }

    fn snapshot_available_from_remote(
        &self,
        _repository_index: u32,
        _repo_name: &str,
        _remote: &str,
        _snapshot: &str,
    ) -> ExternalResult<bool> {
        Ok(false)
    }

    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        _remote: &str,
        line: &str,
    ) -> ExternalResult<Option<String>> {
        if repo_name != self.current_repo_name
            || self.current_repository_index != Some(repository_index)
        {
            return Ok(None);
        }
        match self.store.get_line(line) {
            Ok(line_payload) => Ok(json_string_field(
                line_payload
                    .as_object()
                    .and_then(|object| object.get("head_snapshot_id")),
            )),
            Err(message) if message.contains("Unknown line:") => Ok(None),
            Err(message) => Err(selected_external_store_error(message)),
        }
    }

    fn snapshot_manifest(
        &self,
        _repository_index: u32,
        _repo_name: &str,
        snapshot: &str,
    ) -> ExternalResult<Option<ExternalManifest>> {
        let rows = self
            .store
            .snapshot_tree_file_rows(Some(snapshot))
            .map_err(selected_external_store_error)?;
        let Some(row) = rows.into_iter().find(|row| row.path == "ait-external.toml") else {
            return Ok(None);
        };
        let bytes = self
            .store
            .read_blob_bytes(&row.blob_id)
            .map_err(selected_external_store_error)?;
        self.manifest_codec.parse_manifest(&bytes).map(Some)
    }
}

struct SelectedExternalContentSource<const WRITE_LAYOUT: u32> {
    store: RepoLocalSnapshotOperationStore<WRITE_LAYOUT>,
}

impl<const WRITE_LAYOUT: u32> SelectedExternalContentSource<WRITE_LAYOUT> {
    fn new(repo: &RepoRuntime) -> Result<Self, String> {
        Ok(Self {
            store: selected_external_snapshot_store::<WRITE_LAYOUT>(repo)?,
        })
    }
}

impl<const WRITE_LAYOUT: u32> ExternalContentSource
    for SelectedExternalContentSource<WRITE_LAYOUT>
{
    fn materialize_content(
        &self,
        node: &ait_core::external::lockfile::ExternalLockNode,
        destination: &Path,
    ) -> ExternalResult<()> {
        let rows = self
            .store
            .snapshot_tree_file_rows(Some(&node.snapshot))
            .map_err(selected_external_store_error)?;
        for row in rows {
            let file_path = safe_external_destination_file(destination, &row.path)?;
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    ExternalError::with_code(
                        "external_materializer_create_dir",
                        format!(
                            "failed to create external content directory {:?}: {err}",
                            parent
                        ),
                    )
                })?;
            }
            let bytes = self
                .store
                .read_blob_bytes(&row.blob_id)
                .map_err(selected_external_store_error)?;
            fs::write(&file_path, bytes).map_err(|err| {
                ExternalError::with_code(
                    "external_materializer_write",
                    format!(
                        "failed to write external content file {:?}: {err}",
                        file_path
                    ),
                )
            })?;
            set_external_file_mode(&file_path, &row.mode)?;
        }
        Ok(())
    }
}

fn selected_external_snapshot_store<const WRITE_LAYOUT: u32>(
    repo: &RepoRuntime,
) -> Result<RepoLocalSnapshotOperationStore<WRITE_LAYOUT>, String> {
    let workspace_root = repo.authoritative_repo_root();
    repo.local_snapshot_operation_store::<WRITE_LAYOUT>(&workspace_root)
}

fn selected_external_store_error(message: String) -> ExternalError {
    ExternalError::with_code("external_selected_snapshot_store", message)
}

fn safe_external_destination_file(
    destination: &Path,
    relative_path: &str,
) -> ExternalResult<PathBuf> {
    let relative_path = relative_path.trim();
    if relative_path.is_empty() {
        return Err(ExternalError::with_code(
            "external_materializer_snapshot_path",
            "snapshot file path must not be empty",
        ));
    }
    let parsed = Path::new(relative_path);
    if parsed.is_absolute() {
        return Err(ExternalError::with_code(
            "external_materializer_snapshot_path",
            format!("snapshot file path must be relative, got {relative_path:?}"),
        ));
    }
    let mut output = destination.to_path_buf();
    for component in parsed.components() {
        match component {
            Component::Normal(part) => output.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExternalError::with_code(
                    "external_materializer_snapshot_path",
                    format!(
                        "snapshot file path must stay inside destination, got {relative_path:?}"
                    ),
                ));
            }
        }
    }
    if output == destination {
        return Err(ExternalError::with_code(
            "external_materializer_snapshot_path",
            "snapshot file path must name a file",
        ));
    }
    Ok(output)
}

#[cfg(unix)]
fn set_external_file_mode(path: &Path, mode: &str) -> ExternalResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let trimmed = mode.trim();
    let octal = trimmed.strip_prefix("0o").unwrap_or(trimmed);
    let bits = u32::from_str_radix(octal, 8).unwrap_or(0o644) & 0o777;
    let permissions = fs::Permissions::from_mode(bits);
    fs::set_permissions(path, permissions).map_err(|err| {
        ExternalError::with_code(
            "external_materializer_permissions",
            format!(
                "failed to set external content permissions {:?}: {err}",
                path
            ),
        )
    })
}

#[cfg(not(unix))]
fn set_external_file_mode(_path: &Path, _mode: &str) -> ExternalResult<()> {
    Ok(())
}

impl ExternalUpdateHydrationPorts for RepoRuntimeExternalUpdateHydrationPorts<'_> {
    fn snapshot_content_complete(
        &self,
        _repository_index: u32,
        _repo_name: &str,
        snapshot: &str,
    ) -> Result<bool, String> {
        remote_sync_snapshot_content_complete_for_repo(self.repo, snapshot)
    }

    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> Result<Option<String>, String> {
        self.remote_aware_resolver()?
            .line_head_snapshot(repository_index, repo_name, remote, line)
            .map_err(|err| err.to_string())
    }

    fn snapshot_manifest(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> Result<Option<ExternalManifest>, String> {
        self.local_resolver()?
            .snapshot_manifest(repository_index, repo_name, snapshot)
            .map_err(|err| err.to_string())
    }

    fn import_snapshot(
        &mut self,
        repository_index: u32,
        remote: &str,
        repo_name: &str,
        snapshot: &str,
    ) -> Result<(), String> {
        hydrate_remote_snapshot_boundary_for_repo(
            self.repo,
            remote,
            RepositoryIndex::new(repository_index),
            repo_name,
            snapshot,
        )?;
        Ok(())
    }
}

fn hydrate_external_update_snapshots(
    repo: &RepoRuntime,
    options: &ExternalUpdateOptions,
) -> Result<(), String> {
    if options.locked {
        return Ok(());
    }
    let store = FilesystemExternalUpdateStore::for_repo_root(repo.workspace_root())
        .map_err(|err| err.to_string())?;
    let manifest = store.read_manifest().map_err(|err| err.to_string())?;
    let mut ports = RepoRuntimeExternalUpdateHydrationPorts::new(repo);
    hydrate_external_update_selection_with_ports(&mut ports, &manifest, &options.selection)
}

fn hydrate_external_update_selection_with_ports<P>(
    ports: &mut P,
    manifest: &ExternalManifest,
    selection: &ExternalUpdateSelection,
) -> Result<(), String>
where
    P: ExternalUpdateHydrationPorts + ?Sized,
{
    let declarations = hydration_root_declarations(ports, manifest, selection)?;
    let mut visited = BTreeSet::new();
    for declaration in &declarations {
        hydrate_external_declaration_tree(ports, declaration, &mut visited)?;
    }
    Ok(())
}

fn hydration_root_declarations<P>(
    ports: &P,
    manifest: &ExternalManifest,
    selection: &ExternalUpdateSelection,
) -> Result<Vec<ExternalDeclaration>, String>
where
    P: ExternalUpdateHydrationPorts + ?Sized,
{
    let mut declarations = manifest.externals.clone();
    match selection {
        ExternalUpdateSelection::ManifestPins => {}
        ExternalUpdateSelection::Exact { name, snapshot } => {
            let declaration = resolve_unique_root_declaration_mut(&mut declarations, name)?;
            declaration.snapshot = snapshot.clone();
        }
        ExternalUpdateSelection::Latest { name } => {
            let declaration = resolve_unique_root_declaration_mut(&mut declarations, name)?;
            declaration.snapshot = ports
                .line_head_snapshot(
                    declaration.repository_index,
                    &declaration.repo_name,
                    &declaration.remote,
                    &declaration.line,
                )?
                .ok_or_else(|| {
                    format!(
                        "external {:?} line {:?} on remote {:?} has no head snapshot",
                        declaration.name, declaration.line, declaration.remote
                    )
                })?;
        }
    }
    Ok(declarations)
}

fn resolve_unique_root_declaration_mut<'a>(
    declarations: &'a mut [ExternalDeclaration],
    name: &str,
) -> Result<&'a mut ExternalDeclaration, String> {
    let matching_indexes = declarations
        .iter()
        .enumerate()
        .filter_map(|(index, declaration)| (declaration.name == name).then_some(index))
        .collect::<Vec<_>>();
    match matching_indexes.as_slice() {
        [] => Err(format!(
            "external {name:?} is not declared in the root manifest"
        )),
        [index] => Ok(&mut declarations[*index]),
        _ => Err(format!(
            "external {name:?} appears more than once in the root manifest"
        )),
    }
}

fn hydrate_external_declaration_tree<P>(
    ports: &mut P,
    declaration: &ExternalDeclaration,
    visited: &mut BTreeSet<String>,
) -> Result<(), String>
where
    P: ExternalUpdateHydrationPorts + ?Sized,
{
    let key = format!(
        "{}|{}|{}|{}",
        declaration.repository_index,
        declaration.repo_name,
        declaration.remote,
        declaration.snapshot
    );
    if !visited.insert(key.clone()) {
        return Ok(());
    }

    if !ports.snapshot_content_complete(
        declaration.repository_index,
        &declaration.repo_name,
        &declaration.snapshot,
    )? {
        ports.import_snapshot(
            declaration.repository_index,
            &declaration.remote,
            &declaration.repo_name,
            &declaration.snapshot,
        )?;
    }

    if !ports.snapshot_content_complete(
        declaration.repository_index,
        &declaration.repo_name,
        &declaration.snapshot,
    )? {
        return Err(format!(
            "external {:?} snapshot {:?} content is not complete locally for repo {:?}",
            declaration.name, declaration.snapshot, declaration.repo_name
        ));
    }

    if let Some(nested_manifest) = ports.snapshot_manifest(
        declaration.repository_index,
        &declaration.repo_name,
        &declaration.snapshot,
    )? {
        nested_manifest.validate().map_err(|err| {
            format!(
                "external {:?} snapshot {:?} contains an invalid nested manifest: {}",
                declaration.name,
                declaration.snapshot,
                err.message()
            )
        })?;
        for nested_declaration in &nested_manifest.externals {
            hydrate_external_declaration_tree(ports, nested_declaration, visited)?;
        }
    }

    visited.remove(&key);
    Ok(())
}

struct RemoteAwareExternalSnapshotResolver<L, R> {
    local: L,
    remote_line_heads: R,
}

impl<L, R> RemoteAwareExternalSnapshotResolver<L, R> {
    fn new(local: L, remote_line_heads: R) -> Self {
        Self {
            local,
            remote_line_heads,
        }
    }
}

impl<L, R> ExternalSnapshotResolver for RemoteAwareExternalSnapshotResolver<L, R>
where
    L: ExternalSnapshotResolver,
    R: ExternalRemoteLineHeadSource,
{
    fn snapshot_exists(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> ait_core::external::ExternalResult<bool> {
        self.local
            .snapshot_exists(repository_index, repo_name, snapshot)
    }

    fn snapshot_available_from_remote(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        snapshot: &str,
    ) -> ait_core::external::ExternalResult<bool> {
        self.local
            .snapshot_available_from_remote(repository_index, repo_name, remote, snapshot)
    }

    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> ait_core::external::ExternalResult<Option<String>> {
        if let Some(snapshot) =
            self.local
                .line_head_snapshot(repository_index, repo_name, remote, line)?
        {
            return Ok(Some(snapshot));
        }
        self.remote_line_heads
            .line_head_snapshot(repository_index, repo_name, remote, line)
            .map_err(|message| {
                ait_core::external::ExternalError::with_code("external_remote_line_head", message)
            })
    }

    fn snapshot_manifest(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> ait_core::external::ExternalResult<Option<ExternalManifest>> {
        self.local
            .snapshot_manifest(repository_index, repo_name, snapshot)
    }
}

trait ExternalRemoteLineHeadSource {
    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> Result<Option<String>, String>;
}

struct RepoRuntimeExternalRemoteLineHeadSource<'a> {
    repo: &'a RepoRuntime,
    clients: RefCell<BTreeMap<(String, u32), PlanHttpClientManager>>,
}

impl<'a> RepoRuntimeExternalRemoteLineHeadSource<'a> {
    fn new(repo: &'a RepoRuntime) -> Self {
        Self {
            repo,
            clients: RefCell::new(BTreeMap::new()),
        }
    }
}

impl ExternalRemoteLineHeadSource for RepoRuntimeExternalRemoteLineHeadSource<'_> {
    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> Result<Option<String>, String> {
        let remote_row = self.repo.remote_row(Some(remote))?;
        let mut clients = self.clients.borrow_mut();
        let client_key = (remote.to_string(), repository_index);
        if !clients.contains_key(&client_key) {
            let client = PlanHttpClientManager::new(PlanHttpClientConfig {
                base_url: remote_row.url,
                repository_index: Some(RepositoryIndex::new(repository_index)),
                headers: self.repo.auth_headers(),
                ..PlanHttpClientConfig::default()
            })
            .map_err(|err| err.to_string())?;
            clients.insert(client_key.clone(), client);
        }
        let rows = clients
            .get_mut(&client_key)
            .ok_or_else(|| {
                format!(
                    "remote client {remote:?} for source repository_index {repository_index} was not initialized"
                )
            })?
            .list_lines(repo_name)
            .map_err(|err| err.to_string())?;
        Ok(line_head_from_remote_rows(&rows, line))
    }
}

fn line_head_from_remote_rows(rows: &[JsonValue], line: &str) -> Option<String> {
    rows.iter().find_map(|row| {
        let object = row.as_object()?;
        let row_line = json_string_field(object.get("line_name"))
            .or_else(|| json_string_field(object.get("name")))?;
        if row_line != line {
            return None;
        }
        json_string_field(object.get("head_snapshot_id"))
    })
}

fn validate_external_update_bindings(
    repo: &RepoRuntime,
    lockfile: &ExternalLockfile,
) -> Result<JsonValue, String> {
    let repo_root = repo.workspace_root();
    let validator =
        FilesystemExternalBindingValidator::new(CommandExternalBindingToolProbe::default());
    let findings = validator
        .validate_bindings(ExternalBindingValidationRequest::toolchain_probes(
            &repo_root,
            &lockfile.nodes,
        ))
        .map_err(|err| err.to_string())?;
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == ExternalDoctorSeverity::Warning)
        .count();
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == ExternalDoctorSeverity::Error)
        .count();
    if errors > 0 {
        let details = findings
            .iter()
            .filter(|finding| finding.severity == ExternalDoctorSeverity::Error)
            .map(|finding| format!("{}: {}", finding.code, finding.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!(
            "external binding validation failed with {errors} error(s): {details}"
        ));
    }
    Ok(json!({
        "mode": "toolchain_probes",
        "findings": findings.iter().map(|finding| finding.to_json_value()).collect::<Vec<_>>(),
        "summary": {
            "findings": findings.len(),
            "warnings": warnings,
            "errors": errors,
            "passed": errors == 0,
        },
    }))
}

pub fn external_link(repo: &RepoRuntime, name: &str, path: &str) -> Result<JsonValue, String> {
    validate_external_link_target(repo, path)?;
    let store = FsExternalLinkStore::for_repo_root(repo.authoritative_repo_root());
    let links = store.load_links().map_err(|err| err.to_string())?;
    let mutation =
        upsert_external_local_link_override(&links, name, path).map_err(|err| err.to_string())?;
    store
        .save_links(&mutation.links)
        .map_err(|err| err.to_string())?;
    Ok(json!({
        "command": "external link",
        "repo_name": repo.repo_name(),
        "name": name.trim(),
        "path": path.trim(),
        "links_path": EXTERNAL_LINKS_FILE,
        "changed": mutation.changed,
        "state": "linked",
    }))
}

pub fn external_unlink(repo: &RepoRuntime, name: &str) -> Result<JsonValue, String> {
    let store = FsExternalLinkStore::for_repo_root(repo.authoritative_repo_root());
    let links = store.load_links().map_err(|err| err.to_string())?;
    let mutation =
        remove_external_local_link_override(&links, name).map_err(|err| err.to_string())?;
    store
        .save_links(&mutation.links)
        .map_err(|err| err.to_string())?;
    let restore = if mutation.changed {
        restore_unlinked_external(repo, name.trim())?
    } else {
        ExternalUnlinkRestore::unchanged()
    };
    Ok(json!({
        "command": "external unlink",
        "repo_name": repo.repo_name(),
        "name": name.trim(),
        "links_path": EXTERNAL_LINKS_FILE,
        "changed": mutation.changed,
        "restored": restore.restored,
        "restore_state": restore.state,
        "materialization": restore.materialization,
        "state": "unlinked",
    }))
}

struct ExternalUnlinkRestore {
    restored: bool,
    state: &'static str,
    materialization: JsonValue,
}

impl ExternalUnlinkRestore {
    fn unchanged() -> Self {
        Self {
            restored: false,
            state: "unchanged",
            materialization: JsonValue::Null,
        }
    }

    fn skipped_no_lockfile() -> Self {
        Self {
            restored: false,
            state: "skipped_no_lockfile",
            materialization: JsonValue::Null,
        }
    }

    fn restored(materialization: JsonValue) -> Self {
        Self {
            restored: true,
            state: "restored",
            materialization,
        }
    }
}

fn restore_unlinked_external(
    repo: &RepoRuntime,
    name: &str,
) -> Result<ExternalUnlinkRestore, String> {
    let Some(lockfile) = read_external_lockfile(repo)? else {
        return Ok(ExternalUnlinkRestore::skipped_no_lockfile());
    };
    let subset = lockfile_subtree_for_external(&lockfile, name)?;
    let repo_root = repo.authoritative_repo_root();
    let content_source =
        SelectedExternalContentSource::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>::new(repo)?;
    let materializer = FilesystemExternalMaterializer::new(&repo_root, content_source)
        .map_err(|err| err.to_string())?;
    let report = materializer
        .materialize_lockfile(&subset, &ExternalMaterializationOptions::recursive())
        .map_err(|err| err.to_string())?;
    Ok(ExternalUnlinkRestore::restored(report.to_json_value()))
}

fn lockfile_subtree_for_external(
    lockfile: &ExternalLockfile,
    name: &str,
) -> Result<ExternalLockfile, String> {
    let root = lockfile
        .nodes
        .iter()
        .find(|node| node.parent_path.is_empty() && node.name == name)
        .ok_or_else(|| format!("ait-external.lock does not contain direct external {name:?}"))?;
    let root_path = root.materialize_to.clone();
    let descendant_prefix = format!("{root_path}/");
    let nodes = lockfile
        .nodes
        .iter()
        .filter(|node| {
            (node.parent_path.is_empty() && node.name == name)
                || node.parent_path == root_path
                || node.parent_path.starts_with(&descendant_prefix)
        })
        .cloned()
        .collect::<Vec<_>>();
    let subset = ExternalLockfile::new(nodes).normalized();
    subset.validate().map_err(|err| err.to_string())?;
    Ok(subset)
}

fn read_external_lockfile(repo: &RepoRuntime) -> Result<Option<ExternalLockfile>, String> {
    let store = FilesystemExternalUpdateStore::for_repo_root(repo.authoritative_repo_root())
        .map_err(|err| err.to_string())?;
    store.read_lockfile().map_err(|err| err.to_string())
}

fn read_external_local_links(
    repo: &RepoRuntime,
) -> Result<Vec<ait_core::external::materializer::ExternalLocalLinkOverride>, String> {
    FsExternalLinkStore::for_repo_root(repo.authoritative_repo_root())
        .load_links()
        .map_err(|err| err.to_string())
}

fn json_string_field(value: Option<&JsonValue>) -> Option<String> {
    match value? {
        JsonValue::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    }
}

fn validate_external_link_target(repo: &RepoRuntime, path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("`ait external link` requires a non-empty path.".to_string());
    }
    let raw = Path::new(trimmed);
    let repo_root = repo.authoritative_repo_root();
    let target = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        repo_root.join(raw)
    };
    let metadata = fs::metadata(&target).map_err(|err| {
        format!(
            "external link target {} is not available: {err}",
            target.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "external link target {} must be a directory",
            target.display()
        ));
    }
    reject_silent_repo_relative_symlink_escape(&repo_root, raw, &target)?;
    reject_same_repo_external_link_target(&repo_root, &target)?;
    Ok(target)
}

fn reject_silent_repo_relative_symlink_escape(
    repo_root: &Path,
    raw: &Path,
    target: &Path,
) -> Result<(), String> {
    let repo_root = repo_root.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize repository root {} before validating external link: {err}",
            repo_root.display()
        )
    })?;
    let target = target.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize external link target {}: {err}",
            target.display()
        )
    })?;
    let visibly_outside_repo = raw.is_absolute()
        || raw
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir));
    if !visibly_outside_repo && !target.starts_with(&repo_root) {
        return Err(format!(
            "external link target {} resolves outside repository root {} through a repository-relative path; use an explicit absolute or parent-relative path instead",
            target.display(),
            repo_root.display()
        ));
    }
    Ok(())
}

fn reject_same_repo_external_link_target(repo_root: &Path, target: &Path) -> Result<(), String> {
    let repo_root = repo_root.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize repository root {} before validating external link: {err}",
            repo_root.display()
        )
    })?;
    let target_repo = match RepoRuntime::discover_from_path(target) {
        Ok(repo) => repo,
        Err(_) => return Ok(()),
    };
    let target_root = target_repo.authoritative_repo_root();
    let target_root = target_root.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize external link repository root {}: {err}",
            target_root.display()
        )
    })?;
    if target_root == repo_root {
        return Err(format!(
            "external link target {} resolves to this repository {}; local external links must point to another repository checkout",
            target.display(),
            repo_root.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
