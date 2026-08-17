use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{
    BenchmarkManifest, CampaignScope, CellSpec, FixtureDeclaration, FixtureScale, SampleClass,
    Temperature,
};
use crate::MANIFEST_CONTRACT;

const REQUIRED_FEATURES: &[&str] = &[
    "small_text",
    "large_binary",
    "deep_directories",
    "ignored_files",
    "renames",
    "branches",
    "merge_history",
];

const REQUIRED_CELLS: &[(&str, Temperature, SampleClass)] = &[
    (
        "init_import",
        Temperature::Cold,
        SampleClass::ProcessNetwork,
    ),
    ("status_clean", Temperature::Cold, SampleClass::Local),
    ("status_clean", Temperature::Warm, SampleClass::Local),
    ("status_small_edit", Temperature::Warm, SampleClass::Local),
    ("snapshot_first", Temperature::Cold, SampleClass::Local),
    ("snapshot_noop", Temperature::Warm, SampleClass::Local),
    (
        "snapshot_small_delta",
        Temperature::Warm,
        SampleClass::Local,
    ),
    (
        "snapshot_large_delta",
        Temperature::Warm,
        SampleClass::Local,
    ),
    ("push_empty", Temperature::Cold, SampleClass::ProcessNetwork),
    (
        "push_incremental",
        Temperature::Warm,
        SampleClass::ProcessNetwork,
    ),
    (
        "pull_fast_forward",
        Temperature::Warm,
        SampleClass::ProcessNetwork,
    ),
    (
        "pull_non_fast_forward",
        Temperature::Warm,
        SampleClass::ProcessNetwork,
    ),
    ("ancestry", Temperature::Warm, SampleClass::Local),
    ("merge_base", Temperature::Warm, SampleClass::Local),
    (
        "git_round_trip",
        Temperature::Cold,
        SampleClass::ProcessNetwork,
    ),
];

#[derive(Clone, Debug, Serialize)]
pub struct ValidationReport {
    pub contract: &'static str,
    pub benchmark_id: String,
    pub protocol_revision: String,
    pub campaign_scope: String,
    pub manifest_digest: String,
    pub fixture_count: usize,
    pub cell_count: usize,
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn load_manifest(path: &Path) -> Result<(BenchmarkManifest, String), String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "Failed to read benchmark manifest {}: {error}",
            path.display()
        )
    })?;
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    let manifest = serde_json::from_slice::<BenchmarkManifest>(&bytes).map_err(|error| {
        format!(
            "Failed to decode benchmark manifest {}: {error}",
            path.display()
        )
    })?;
    Ok((manifest, digest))
}

pub fn validate_manifest(manifest: &BenchmarkManifest, manifest_digest: &str) -> ValidationReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if manifest.contract != MANIFEST_CONTRACT {
        errors.push(format!(
            "contract must be {MANIFEST_CONTRACT}, got {}",
            manifest.contract
        ));
    }
    require_text("benchmark_id", &manifest.benchmark_id, &mut errors);
    require_text(
        "protocol_revision",
        &manifest.protocol_revision,
        &mut errors,
    );
    if manifest.sampling.warmup_iterations < 5 {
        errors.push("sampling.warmup_iterations must be at least 5".to_string());
    }
    if manifest.sampling.measured_local_iterations < 50 {
        errors.push("sampling.measured_local_iterations must be at least 50".to_string());
    }
    if manifest.sampling.measured_cold_iterations < 30 {
        errors.push("sampling.measured_cold_iterations must be at least 30".to_string());
    }
    if manifest.bootstrap_resamples < 1_000 {
        errors.push("bootstrap_resamples must be at least 1000".to_string());
    }

    validate_environment(manifest, &mut errors);
    let fixture_by_id = validate_fixtures(manifest, &mut errors, &mut warnings);
    validate_cells(manifest, &fixture_by_id, &mut errors);
    match manifest.campaign_scope {
        CampaignScope::FullMatrix => validate_matrix(manifest, &fixture_by_id, &mut errors),
        CampaignScope::FocusedSlice => {
            if manifest.cells.is_empty() {
                errors.push("focused_slice must declare at least one benchmark cell".to_string());
            }
            warnings.push(
                "focused_slice evidence is never claim eligible and cannot satisfy the release matrix"
                    .to_string(),
            );
        }
    }

    ValidationReport {
        contract: MANIFEST_CONTRACT,
        benchmark_id: manifest.benchmark_id.clone(),
        protocol_revision: manifest.protocol_revision.clone(),
        campaign_scope: manifest.campaign_scope.as_str().to_string(),
        manifest_digest: manifest_digest.to_string(),
        fixture_count: manifest.fixtures.len(),
        cell_count: manifest.cells.len(),
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

fn validate_environment(manifest: &BenchmarkManifest, errors: &mut Vec<String>) {
    let environment = &manifest.environment;
    for (field, value) in [
        ("captured_at", environment.captured_at.as_str()),
        ("os", environment.os.as_str()),
        ("architecture", environment.architecture.as_str()),
        ("filesystem", environment.filesystem.as_str()),
        ("storage_medium", environment.storage_medium.as_str()),
        ("cpu", environment.cpu.as_str()),
        ("rust_version", environment.rust_version.as_str()),
        ("git_version", environment.git_version.as_str()),
        ("ait_version", environment.ait_version.as_str()),
        (
            "repository_snapshot",
            environment.repository_snapshot.as_str(),
        ),
        ("server_revision", environment.server_revision.as_str()),
        ("network_profile", environment.network_profile.as_str()),
        ("cache_drop_method", environment.cache_drop_method.as_str()),
    ] {
        require_text(&format!("environment.{field}"), value, errors);
    }
    if environment.memory_bytes == 0 {
        errors.push("environment.memory_bytes must be greater than zero".to_string());
    }
}

fn validate_fixtures<'a>(
    manifest: &'a BenchmarkManifest,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> BTreeMap<&'a str, &'a FixtureDeclaration> {
    let mut by_id = BTreeMap::new();
    let mut kinds_by_scale = BTreeMap::<&str, BTreeSet<String>>::new();
    for fixture in &manifest.fixtures {
        if fixture.fixture_id.trim().is_empty() {
            errors.push("fixture_id must not be empty".to_string());
            continue;
        }
        if by_id.insert(fixture.fixture_id.as_str(), fixture).is_some() {
            errors.push(format!("duplicate fixture_id: {}", fixture.fixture_id));
        }
        require_text(
            &format!("fixture {} revision", fixture.fixture_id),
            &fixture.revision,
            errors,
        );
        require_text(
            &format!("fixture {} source", fixture.fixture_id),
            &fixture.source,
            errors,
        );
        require_text(
            &format!("fixture {} redistribution", fixture.fixture_id),
            &fixture.redistribution,
            errors,
        );
        if !is_sha256(&fixture.content_digest) {
            errors.push(format!(
                "fixture {} content_digest must be sha256:<64 lowercase hex>",
                fixture.fixture_id
            ));
        }
        validate_scale_bounds(fixture, errors);
        let features = fixture
            .features
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for required in REQUIRED_FEATURES {
            if !features.contains(required) {
                errors.push(format!(
                    "fixture {} is missing required feature {required}",
                    fixture.fixture_id
                ));
            }
        }
        let normalized_kind = fixture.kind.trim().to_ascii_lowercase();
        if !matches!(normalized_kind.as_str(), "synthetic" | "real") {
            errors.push(format!(
                "fixture {} kind must be synthetic or real",
                fixture.fixture_id
            ));
        }
        kinds_by_scale
            .entry(fixture.scale.as_str())
            .or_default()
            .insert(normalized_kind);
    }
    let required_scales = match manifest.campaign_scope {
        CampaignScope::FullMatrix => ["small", "medium", "large"]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>(),
        CampaignScope::FocusedSlice => manifest
            .cells
            .iter()
            .filter_map(|cell| by_id.get(cell.fixture_id.as_str()))
            .map(|fixture| fixture.scale.as_str().to_string())
            .collect::<BTreeSet<_>>(),
    };
    for scale in required_scales {
        let kinds = kinds_by_scale
            .get(scale.as_str())
            .cloned()
            .unwrap_or_default();
        for kind in ["synthetic", "real"] {
            if !kinds.contains(kind) {
                errors.push(format!(
                    "{scale} scale must declare at least one {kind} fixture"
                ));
            }
        }
    }
    if manifest.fixtures.iter().any(|fixture| {
        fixture.kind.eq_ignore_ascii_case("real")
            && fixture.redistribution.eq_ignore_ascii_case("unspecified")
    }) {
        warnings.push(
            "real fixture redistribution should state public, private, or restricted constraints"
                .to_string(),
        );
    }
    by_id
}

fn validate_scale_bounds(fixture: &FixtureDeclaration, errors: &mut Vec<String>) {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    let valid = match fixture.scale {
        FixtureScale::Small => {
            fixture.file_count <= 1_000
                && fixture.total_bytes <= 100 * MIB
                && fixture.history_nodes <= 500
        }
        FixtureScale::Medium => {
            (10_000..=50_000).contains(&fixture.file_count)
                && (GIB..=5 * GIB).contains(&fixture.total_bytes)
                && (10_000..=50_000).contains(&fixture.history_nodes)
        }
        FixtureScale::Large => {
            (fixture.file_count >= 100_000 || fixture.total_bytes >= 10 * GIB)
                && fixture.history_nodes >= 100_000
        }
    };
    if !valid {
        errors.push(format!(
            "fixture {} does not satisfy {} scale file/byte/history bounds",
            fixture.fixture_id,
            fixture.scale.as_str()
        ));
    }
}

fn validate_cells<'a>(
    manifest: &'a BenchmarkManifest,
    fixture_by_id: &BTreeMap<&'a str, &'a FixtureDeclaration>,
    errors: &mut Vec<String>,
) {
    let mut ids = BTreeSet::new();
    for cell in &manifest.cells {
        if !ids.insert(cell.cell_id.as_str()) {
            errors.push(format!("duplicate cell_id: {}", cell.cell_id));
        }
        if !fixture_by_id.contains_key(cell.fixture_id.as_str()) {
            errors.push(format!(
                "cell {} references unknown fixture {}",
                cell.cell_id, cell.fixture_id
            ));
        }
        let roles = cell
            .subjects
            .iter()
            .map(|subject| subject.role.trim().to_ascii_lowercase())
            .collect::<Vec<_>>();
        for required_role in ["ait", "git"] {
            if roles
                .iter()
                .filter(|role| role.as_str() == required_role)
                .count()
                != 1
            {
                errors.push(format!(
                    "cell {} must contain exactly one {required_role} subject",
                    cell.cell_id
                ));
            }
        }
        let mut subject_ids = BTreeSet::new();
        for subject in &cell.subjects {
            if !subject_ids.insert(subject.subject_id.as_str()) {
                errors.push(format!(
                    "cell {} has duplicate subject_id {}",
                    cell.cell_id, subject.subject_id
                ));
            }
            validate_command(
                &format!(
                    "cell {} subject {} command",
                    cell.cell_id, subject.subject_id
                ),
                &subject.command,
                errors,
            );
            validate_command(
                &format!(
                    "cell {} subject {} history_node_probe",
                    cell.cell_id, subject.subject_id
                ),
                &subject.history_node_probe,
                errors,
            );
            validate_command(
                &format!(
                    "cell {} subject {} outcome_probe",
                    cell.cell_id, subject.subject_id
                ),
                &subject.outcome_probe,
                errors,
            );
            for (index, command) in subject
                .reset_commands
                .iter()
                .chain(subject.prepare_commands.iter())
                .chain(subject.cleanup_commands.iter())
                .enumerate()
            {
                validate_command(
                    &format!(
                        "cell {} subject {} lifecycle command {}",
                        cell.cell_id, subject.subject_id, index
                    ),
                    command,
                    errors,
                );
            }
            if subject.metadata_excludes.is_empty() {
                errors.push(format!(
                    "cell {} subject {} must declare VCS metadata exclusions",
                    cell.cell_id, subject.subject_id
                ));
            }
        }
    }
}

fn validate_command(label: &str, command: &crate::model::CommandSpec, errors: &mut Vec<String>) {
    if command.program.trim().is_empty() {
        errors.push(format!("{label} program must not be empty"));
    }
    if command.expected_exit_codes.is_empty() {
        errors.push(format!("{label} expected_exit_codes must not be empty"));
    }
}

fn validate_matrix<'a>(
    manifest: &'a BenchmarkManifest,
    fixture_by_id: &BTreeMap<&'a str, &'a FixtureDeclaration>,
    errors: &mut Vec<String>,
) {
    for scale in ["small", "medium", "large"] {
        for (operation, temperature, sample_class) in REQUIRED_CELLS {
            let present = manifest.cells.iter().any(|cell| {
                fixture_by_id
                    .get(cell.fixture_id.as_str())
                    .is_some_and(|fixture| fixture.scale.as_str() == scale)
                    && cell.operation == *operation
                    && cell.temperature == *temperature
                    && cell.sample_class == *sample_class
            });
            if !present {
                errors.push(format!(
                    "matrix is missing {scale}/{operation}/{}/{}",
                    temperature.as_str(),
                    sample_class.as_str()
                ));
            }
        }
    }
}

fn require_text(field: &str, value: &str, errors: &mut Vec<String>) {
    if value.trim().is_empty() {
        errors.push(format!("{field} must not be empty"));
    }
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

pub(crate) fn fixture_for_cell<'a>(
    manifest: &'a BenchmarkManifest,
    cell: &CellSpec,
) -> Result<&'a FixtureDeclaration, String> {
    manifest
        .fixtures
        .iter()
        .find(|fixture| fixture.fixture_id == cell.fixture_id)
        .ok_or_else(|| {
            format!(
                "Cell {} references unknown fixture {}",
                cell.cell_id, cell.fixture_id
            )
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{
        CommandSpec, EnvironmentPin, FixtureDeclaration, SamplingPolicy, SubjectSpec,
    };

    use super::*;

    #[test]
    fn sha256_contract_is_lowercase_and_exact() {
        assert!(is_sha256(&format!("sha256:{}", "a".repeat(64))));
        assert!(!is_sha256(&format!("sha256:{}", "A".repeat(64))));
        assert!(!is_sha256("sha256:abc"));
    }

    #[test]
    fn complete_multiscale_matrix_is_valid_and_missing_cell_is_not() {
        let mut manifest = complete_manifest();
        let report = validate_manifest(&manifest, &format!("sha256:{}", "b".repeat(64)));
        assert!(report.valid, "{:?}", report.errors);
        assert_eq!(report.cell_count, REQUIRED_CELLS.len() * 3);

        manifest.cells.pop();
        manifest.sampling.measured_local_iterations = 49;
        let report = validate_manifest(&manifest, &format!("sha256:{}", "b".repeat(64)));
        assert!(!report.valid);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("measured_local_iterations")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("matrix is missing")));
    }

    #[test]
    fn focused_slice_validates_declared_scale_but_never_the_release_matrix() {
        let mut manifest = complete_manifest();
        manifest.campaign_scope = CampaignScope::FocusedSlice;
        manifest
            .fixtures
            .retain(|fixture| fixture.scale.as_str() == "small");
        manifest
            .cells
            .retain(|cell| cell.cell_id == "small-status_clean-warm-local");
        let report = validate_manifest(&manifest, &format!("sha256:{}", "b".repeat(64)));
        assert!(report.valid, "{:?}", report.errors);
        assert_eq!(report.campaign_scope, "focused_slice");
        assert_eq!(report.cell_count, 1);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("never claim eligible")));
        assert!(!report
            .errors
            .iter()
            .any(|error| error.contains("matrix is missing")));
    }

    #[test]
    fn compiled_protocol_asset_matches_implementation_axes() {
        let protocol: serde_json::Value = serde_json::from_str(crate::PROTOCOL_V1_JSON).unwrap();
        assert_eq!(
            protocol["contract"],
            serde_json::json!("ait-vcs-benchmark-protocol/v1")
        );
        assert_eq!(
            protocol["required_cells_per_scale"]
                .as_array()
                .unwrap()
                .len(),
            REQUIRED_CELLS.len()
        );
        assert_eq!(
            protocol["sampling"]["minimum_measured_local_iterations"],
            serde_json::json!(50)
        );
        assert_eq!(
            protocol["campaign_scopes"]["focused_slice"],
            serde_json::json!("One or more explicitly selected cells with full sample counts; useful for engineering evidence but never claim eligible.")
        );
    }

    fn complete_manifest() -> BenchmarkManifest {
        let mut fixtures = Vec::new();
        for (scale, file_count, total_bytes, history_nodes) in [
            (FixtureScale::Small, 512, 16 * 1024 * 1024, 256),
            (FixtureScale::Medium, 10_000, 1024 * 1024 * 1024, 10_000),
            (
                FixtureScale::Large,
                100_000,
                10 * 1024 * 1024 * 1024,
                100_000,
            ),
        ] {
            for kind in ["synthetic", "real"] {
                fixtures.push(FixtureDeclaration {
                    fixture_id: format!("{}-{kind}-v1", scale.as_str()),
                    revision: "1".to_string(),
                    scale,
                    kind: kind.to_string(),
                    source: format!("fixture://{}-{kind}", scale.as_str()),
                    redistribution: if kind == "real" {
                        "restricted; local measurement only".to_string()
                    } else {
                        "generated; redistributable".to_string()
                    },
                    content_digest: format!("sha256:{}", "a".repeat(64)),
                    file_count,
                    total_bytes,
                    history_nodes,
                    features: REQUIRED_FEATURES
                        .iter()
                        .map(|feature| (*feature).to_string())
                        .collect(),
                });
            }
        }
        let command = CommandSpec {
            program: "/usr/bin/true".to_string(),
            args: vec![],
            cwd: None,
            env: BTreeMap::new(),
            expected_exit_codes: vec![0],
        };
        let mut cells = Vec::new();
        for scale in ["small", "medium", "large"] {
            for (operation, temperature, sample_class) in REQUIRED_CELLS {
                cells.push(CellSpec {
                    cell_id: format!(
                        "{scale}-{operation}-{}-{}",
                        temperature.as_str(),
                        sample_class.as_str()
                    ),
                    fixture_id: format!("{scale}-synthetic-v1"),
                    operation: (*operation).to_string(),
                    temperature: *temperature,
                    sample_class: *sample_class,
                    subjects: ["ait", "git"]
                        .iter()
                        .map(|role| SubjectSpec {
                            subject_id: format!("{role}-{scale}"),
                            role: (*role).to_string(),
                            workspace_root: PathBuf::from(format!("/fixtures/{scale}/{role}")),
                            metadata_excludes: vec![".ait".to_string(), ".git".to_string()],
                            command: command.clone(),
                            reset_commands: vec![],
                            prepare_commands: vec![],
                            cleanup_commands: vec![],
                            history_node_probe: command.clone(),
                            outcome_probe: command.clone(),
                            metrics_json_path: None,
                        })
                        .collect(),
                });
            }
        }
        BenchmarkManifest {
            contract: MANIFEST_CONTRACT.to_string(),
            benchmark_id: "matrix-v1".to_string(),
            protocol_revision: "vcs-performance-2026-07-19.1".to_string(),
            campaign_scope: CampaignScope::FullMatrix,
            seed: 42,
            sampling: SamplingPolicy {
                warmup_iterations: 5,
                measured_local_iterations: 50,
                measured_cold_iterations: 30,
            },
            environment: EnvironmentPin {
                captured_at: "2026-07-19T00:00:00Z".to_string(),
                os: "macOS".to_string(),
                architecture: "arm64".to_string(),
                filesystem: "APFS".to_string(),
                storage_medium: "NVMe".to_string(),
                cpu: "test CPU".to_string(),
                memory_bytes: 16 * 1024 * 1024 * 1024,
                rust_version: "rustc test".to_string(),
                git_version: "git test".to_string(),
                ait_version: "ait fixture".to_string(),
                repository_snapshot: "SNP-TEST".to_string(),
                server_revision: "server-test".to_string(),
                network_profile: "loopback".to_string(),
                cache_drop_method: "declared reset commands".to_string(),
                command_options: BTreeMap::new(),
            },
            fixtures,
            cells,
            bootstrap_resamples: 1_000,
            limitations: vec!["test fixture declarations only".to_string()],
        }
    }
}
