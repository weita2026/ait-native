use crate::generation_inventory::is_disposable_runtime_file;
use crate::u64_second_upgrade::{
    with_frozen_upgrade_source, U64_SECOND_UPGRADE_COMPLETION_SCHEMA,
    U64_SECOND_UPGRADE_REPORT_SCHEMA,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const COMPLETION_FILE: &str = "conversion-complete.json";
const ACTIVATION_SCHEMA: &str = "ait.server.binary_v0.activation.v1";
// Read-only compatibility for generations produced before the incident-only
// converter was retired. No command or writer in this crate can create this
// evidence pair.
const LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA: &str =
    "ait.server.binary_v0.plan_lineage_repair.complete.v1";
const LEGACY_PLAN_LINEAGE_REPORT_SCHEMA: &str =
    "ait.server.binary_v0.plan_lineage_repair.report.v1";
const LEGACY_PLAN_LINEAGE_RECEIPT_FILE: &str = "plan-lineage-repair-receipt.json";

#[derive(Clone, Debug)]
pub struct ActivateRequest {
    pub staged_generation: PathBuf,
    pub activation_pointer: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivateResult {
    pub staged_generation: PathBuf,
    pub activation_pointer: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct CompletionEvidence {
    schema: String,
    layout_id: u32,
    status: String,
    report_sha256: String,
}

#[derive(Debug, Deserialize)]
struct ReportEvidence {
    schema: String,
    layout_id: u32,
    status: String,
    #[serde(default)]
    authority_files: Vec<ReportFileEvidence>,
    #[serde(default)]
    sealed_files: Vec<ReportFileEvidence>,
    #[serde(default)]
    repository_indexes: Vec<u32>,
    source_generation: Option<String>,
    source_authority_fingerprint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReportFileEvidence {
    relative_path: String,
    byte_size: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct ActivationPointer<'a> {
    schema: &'static str,
    layout_id: u32,
    generation: &'a str,
    completion_sha256: String,
}

pub fn activate_generation(request: ActivateRequest) -> Result<ActivateResult, String> {
    let generation = canonical_real_directory(&request.staged_generation)?;
    let completion_path = generation.join(COMPLETION_FILE);
    let completion_bytes = read_regular_file(&completion_path)?;
    let completion: CompletionEvidence =
        serde_json::from_slice(&completion_bytes).map_err(|error| {
            format!(
                "failed to parse staged completion evidence {}: {error}",
                completion_path.display()
            )
        })?;
    if !matches!(
        completion.schema.as_str(),
        "ait.server.postgres_to_binary_v0.complete.v1"
            | U64_SECOND_UPGRADE_COMPLETION_SCHEMA
            | LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA
    ) || completion.layout_id != 1
        || completion.status != "validated_inactive"
        || !is_sha256(&completion.report_sha256)
    {
        return Err("staged generation completion evidence is invalid".to_string());
    }
    let report = validate_generation_evidence(&generation, &completion)?;
    let parent = request.activation_pointer.parent().ok_or_else(|| {
        format!(
            "activation pointer has no parent: {}",
            request.activation_pointer.display()
        )
    })?;
    let parent = canonical_real_directory(parent)?;
    let target_name = request
        .activation_pointer
        .file_name()
        .ok_or_else(|| "activation pointer has no file name".to_string())?;
    let activation_pointer = parent.join(target_name);
    reject_non_regular_if_present(&activation_pointer)?;

    let generation_text = generation
        .to_str()
        .ok_or_else(|| "staged generation path is not UTF-8".to_string())?;
    let pointer = ActivationPointer {
        schema: ACTIVATION_SCHEMA,
        layout_id: 1,
        generation: generation_text,
        completion_sha256: sha256(&completion_bytes),
    };
    let mut bytes = serde_json::to_vec_pretty(&pointer)
        .map_err(|error| format!("failed to encode activation pointer: {error}"))?;
    bytes.push(b'\n');
    if completion.schema == U64_SECOND_UPGRADE_COMPLETION_SCHEMA {
        let source_generation = report
            .source_generation
            .as_deref()
            .ok_or_else(|| "u64-second report lacks source generation".to_string())?;
        let source_fingerprint = report
            .source_authority_fingerprint
            .as_deref()
            .ok_or_else(|| "u64-second report lacks source fingerprint".to_string())?;
        with_frozen_upgrade_source(Path::new(source_generation), source_fingerprint, || {
            atomic_replace(&activation_pointer, &bytes)
        })?;
    } else {
        atomic_replace(&activation_pointer, &bytes)?;
    }

    Ok(ActivateResult {
        staged_generation: generation,
        activation_pointer,
    })
}

fn validate_generation_evidence(
    generation: &Path,
    completion: &CompletionEvidence,
) -> Result<ReportEvidence, String> {
    let report_path = generation.join("conversion-report.json");
    let report_bytes = read_regular_file(&report_path)?;
    if sha256(&report_bytes) != completion.report_sha256 {
        return Err("staged conversion report hash disagrees with completion evidence".to_string());
    }
    let report: ReportEvidence = serde_json::from_slice(&report_bytes).map_err(|error| {
        format!(
            "failed to parse staged conversion report {}: {error}",
            report_path.display()
        )
    })?;
    let expected_report_schema = match completion.schema.as_str() {
        U64_SECOND_UPGRADE_COMPLETION_SCHEMA => U64_SECOND_UPGRADE_REPORT_SCHEMA,
        LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA => LEGACY_PLAN_LINEAGE_REPORT_SCHEMA,
        _ => "ait.server.postgres_to_binary_v0.report.v1",
    };
    if report.schema != expected_report_schema
        || report.layout_id != 1
        || report.status != "validated_inactive"
    {
        return Err("staged conversion report envelope is invalid".to_string());
    }
    if completion.schema == LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA {
        validate_legacy_plan_lineage_seal(generation, &report)?;
        return Ok(report);
    }
    let mut expected_paths = BTreeSet::new();
    let mut previous: Option<&str> = None;
    for file in &report.authority_files {
        let relative = Path::new(&file.relative_path);
        if relative.is_absolute()
            || file.relative_path.is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            || previous.is_some_and(|previous| previous.as_bytes() >= file.relative_path.as_bytes())
            || !expected_paths.insert(file.relative_path.clone())
            || !is_sha256(&file.sha256)
        {
            return Err("staged conversion report file inventory is invalid".to_string());
        }
        previous = Some(&file.relative_path);
        let bytes = read_regular_file(&generation.join(relative))?;
        if bytes.len() as u64 != file.byte_size || sha256(&bytes) != file.sha256 {
            return Err(format!(
                "staged authority file changed after validation: {}",
                file.relative_path
            ));
        }
    }
    let actual_paths = inventory_generation_authority_paths(generation)?;
    if actual_paths != expected_paths {
        return Err(format!(
            "staged generation file closure changed after validation: expected={expected_paths:?}, actual={actual_paths:?}"
        ));
    }
    Ok(report)
}

fn validate_legacy_plan_lineage_seal(
    generation: &Path,
    report: &ReportEvidence,
) -> Result<(), String> {
    if report.repository_indexes != [0, 1]
        || !report.authority_files.is_empty()
        || report.sealed_files.is_empty()
    {
        return Err("legacy Plan-lineage report scope is invalid".to_string());
    }
    let expected_paths = expected_legacy_plan_lineage_sealed_paths();
    let mut actual_paths = BTreeSet::new();
    let mut previous: Option<&str> = None;
    for file in &report.sealed_files {
        let relative = Path::new(&file.relative_path);
        if relative.is_absolute()
            || file.relative_path.is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            || previous.is_some_and(|previous| previous.as_bytes() >= file.relative_path.as_bytes())
            || !actual_paths.insert(file.relative_path.clone())
            || !is_sha256(&file.sha256)
        {
            return Err("legacy Plan-lineage sealed file inventory is invalid".to_string());
        }
        previous = Some(&file.relative_path);
        let bytes = read_regular_file(&generation.join(relative))?;
        if bytes.len() as u64 != file.byte_size || sha256(&bytes) != file.sha256 {
            return Err(format!(
                "legacy Plan-lineage sealed file changed after validation: {}",
                file.relative_path
            ));
        }
    }
    if actual_paths != expected_paths {
        return Err(format!(
            "legacy Plan-lineage sealed scope differs: expected={expected_paths:?}, actual={actual_paths:?}"
        ));
    }
    Ok(())
}

fn expected_legacy_plan_lineage_sealed_paths() -> BTreeSet<String> {
    let mut paths = BTreeSet::from([
        "generation.json".to_string(),
        "global/repository.bin".to_string(),
        "global/repository_payload.bin".to_string(),
        LEGACY_PLAN_LINEAGE_RECEIPT_FILE.to_string(),
    ]);
    for repository_index in [0_u32, 1_u32] {
        for name in [
            "plan.bin",
            "plan_payload.bin",
            "plan_revision.bin",
            "plan_revision_payload.bin",
            "plan_item.bin",
            "plan_item_payload.bin",
            "task.bin",
        ] {
            paths.insert(format!("repositories/{repository_index}/{name}"));
        }
    }
    paths
}

fn inventory_generation_authority_paths(generation: &Path) -> Result<BTreeSet<String>, String> {
    fn visit(
        generation: &Path,
        current: &Path,
        paths: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(current)
            .map_err(|error| format!("failed to inventory {}: {error}", current.display()))?
        {
            let entry =
                entry.map_err(|error| format!("failed to read generation entry: {error}"))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "staged generation contains symlink {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                visit(generation, &path, paths)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(generation)
                    .map_err(|_| "staged path escaped generation".to_string())?
                    .to_str()
                    .ok_or_else(|| "staged generation path is not UTF-8".to_string())?;
                if matches!(
                    relative,
                    "conversion-report.json" | "conversion-complete.json"
                ) || is_disposable_runtime_file(Path::new(relative))
                {
                    continue;
                }
                paths.insert(relative.to_string());
            } else {
                return Err(format!(
                    "staged generation contains special path {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }
    let mut paths = BTreeSet::new();
    visit(generation, generation, &mut paths)?;
    Ok(paths)
}

pub(crate) fn write_completion(generation: &Path, report_sha256: String) -> Result<(), String> {
    write_completion_with_schema(
        generation,
        report_sha256,
        "ait.server.postgres_to_binary_v0.complete.v1",
    )
}

pub(crate) fn write_upgrade_completion(
    generation: &Path,
    report_sha256: String,
) -> Result<(), String> {
    write_completion_with_schema(
        generation,
        report_sha256,
        U64_SECOND_UPGRADE_COMPLETION_SCHEMA,
    )
}

fn write_completion_with_schema(
    generation: &Path,
    report_sha256: String,
    schema: &str,
) -> Result<(), String> {
    let evidence = CompletionEvidence {
        schema: schema.to_string(),
        layout_id: 1,
        status: "validated_inactive".to_string(),
        report_sha256,
    };
    let mut bytes = serde_json::to_vec_pretty(&evidence)
        .map_err(|error| format!("failed to encode completion evidence: {error}"))?;
    bytes.push(b'\n');
    write_new_sync(&generation.join(COMPLETION_FILE), &bytes)?;
    sync_directory(generation)
}

pub(crate) fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system time precedes Unix epoch".to_string())?
        .as_nanos();
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("activation"),
        nonce
    ));
    write_new_sync(&temporary, bytes)?;
    fs::rename(&temporary, path).map_err(|error| {
        format!(
            "failed to atomically replace {} from {}: {error}",
            path.display(),
            temporary.display()
        )
    })?;
    sync_directory(parent)
}

pub(crate) fn write_new_sync(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("failed to write and sync {}: {error}", path.display()))
}

pub(crate) fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "path is not a regular non-symlink file: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok(bytes)
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("failed to sync directory {}: {error}", path.display()))
}

pub(crate) fn canonical_real_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect directory {}: {error}", path.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "path is not a real non-symlink directory: {}",
            path.display()
        ));
    }
    fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))
}

pub(crate) fn reject_non_regular_if_present(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(())
        }
        Ok(_) => Err(format!(
            "existing path is not a regular non-symlink file: {}",
            path.display()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temporary_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ait-server-postgres-import-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn staged_fixture(root: &Path) -> PathBuf {
        let generation = root.join("generation-1");
        fs::create_dir(&generation).unwrap();
        let generation_bytes = b"{\"layout_id\":1}\n";
        write_new_sync(&generation.join("generation.json"), generation_bytes).unwrap();
        let report = json!({
            "schema": "ait.server.postgres_to_binary_v0.report.v1",
            "layout_id": 1,
            "status": "validated_inactive",
            "authority_files": [{
                "relative_path": "generation.json",
                "byte_size": generation_bytes.len(),
                "sha256": sha256(generation_bytes),
            }],
        });
        let mut report_bytes = serde_json::to_vec_pretty(&report).unwrap();
        report_bytes.push(b'\n');
        write_new_sync(&generation.join("conversion-report.json"), &report_bytes).unwrap();
        write_completion(&generation, sha256(&report_bytes)).unwrap();
        generation
    }

    #[test]
    fn activation_rehashes_and_atomically_replaces_pointer() {
        let root = temporary_root("activate");
        let generation = staged_fixture(&root);
        let pointer = root.join("current-generation.json");
        let result = activate_generation(ActivateRequest {
            staged_generation: generation.clone(),
            activation_pointer: pointer.clone(),
        })
        .unwrap();
        assert_eq!(
            result.staged_generation,
            fs::canonicalize(&generation).unwrap()
        );
        let first = read_regular_file(&pointer).unwrap();
        assert!(std::str::from_utf8(&first)
            .unwrap()
            .contains(ACTIVATION_SCHEMA));

        fs::write(generation.join("generation.json"), b"tampered\n").unwrap();
        let error = activate_generation(ActivateRequest {
            staged_generation: generation,
            activation_pointer: pointer.clone(),
        })
        .unwrap_err();
        assert!(error.contains("changed after validation"));
        assert_eq!(read_regular_file(&pointer).unwrap(), first);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn activation_ignores_only_exact_runtime_rebuild_temporaries() {
        let root = temporary_root("activate-runtime-rebuild");
        let generation = staged_fixture(&root);
        let repository = generation.join("repositories/2");
        fs::create_dir_all(&repository).unwrap();
        fs::write(
            repository.join(".worker_ready.idx.rebuild"),
            b"interrupted disposable rebuild",
        )
        .unwrap();
        let pointer = root.join("current-generation.json");

        activate_generation(ActivateRequest {
            staged_generation: generation.clone(),
            activation_pointer: pointer.clone(),
        })
        .unwrap();

        fs::write(
            repository.join(".worker_ready.idx.rebuild.extra"),
            b"unrecognized authority",
        )
        .unwrap();
        let error = activate_generation(ActivateRequest {
            staged_generation: generation,
            activation_pointer: pointer,
        })
        .unwrap_err();
        assert!(error.contains("file closure changed"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn activation_admits_legacy_plan_lineage_evidence_fail_closed() {
        let root = temporary_root("activate-legacy-plan-lineage");
        let generation = root.join("generation-1");
        fs::create_dir(&generation).unwrap();

        let sealed_files = expected_legacy_plan_lineage_sealed_paths()
            .into_iter()
            .map(|relative_path| {
                let path = generation.join(&relative_path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                let bytes = format!("legacy sealed evidence: {relative_path}\n").into_bytes();
                fs::write(&path, &bytes).unwrap();
                json!({
                    "relative_path": relative_path,
                    "byte_size": bytes.len(),
                    "sha256": sha256(&bytes),
                })
            })
            .collect::<Vec<_>>();
        let mut report_bytes = serde_json::to_vec_pretty(&json!({
            "schema": LEGACY_PLAN_LINEAGE_REPORT_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "authority_files": [],
            "sealed_files": sealed_files,
            "repository_indexes": [0, 1],
        }))
        .unwrap();
        report_bytes.push(b'\n');
        fs::write(generation.join("conversion-report.json"), &report_bytes).unwrap();
        let mut completion_bytes = serde_json::to_vec_pretty(&json!({
            "schema": LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "report_sha256": sha256(&report_bytes),
        }))
        .unwrap();
        completion_bytes.push(b'\n');
        fs::write(generation.join(COMPLETION_FILE), &completion_bytes).unwrap();

        let pointer = root.join("current-generation.json");
        activate_generation(ActivateRequest {
            staged_generation: generation.clone(),
            activation_pointer: pointer.clone(),
        })
        .expect("admit an already-produced legacy generation");

        fs::write(
            generation.join(LEGACY_PLAN_LINEAGE_RECEIPT_FILE),
            b"tampered legacy receipt\n",
        )
        .unwrap();
        let error = activate_generation(ActivateRequest {
            staged_generation: generation,
            activation_pointer: pointer,
        })
        .expect_err("legacy evidence must remain fail-closed");
        assert!(error.contains("changed after validation"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }
}
