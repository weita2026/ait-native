use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::model::{BenchmarkManifest, CommandSpec};

pub const PORTABILITY_CONTRACT: &str = "ait-vcs-benchmark-portability/v1";
pub const NORMALIZATION_CONTRACT: &str = "ait-vcs-benchmark-normalization/v1";

#[derive(Clone, Debug, Default)]
pub struct RuntimeBindings {
    values: BTreeMap<String, String>,
}

impl RuntimeBindings {
    pub fn parse(entries: &[String]) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        let mut paths = BTreeMap::<String, String>::new();
        for entry in entries {
            let (name, value) = entry
                .split_once('=')
                .ok_or_else(|| "Runtime bindings must use NAME=ABSOLUTE_PATH syntax".to_string())?;
            let name = name.trim();
            if !valid_binding_name(name) {
                return Err(format!(
                    "Invalid runtime binding name {name:?}; use lowercase ASCII letters, digits, and hyphens"
                ));
            }
            if value.is_empty() {
                return Err(format!("Runtime binding {name} has an empty path"));
            }
            if !absolute_path_like(value) {
                return Err(format!(
                    "Runtime binding {name} must use an absolute Unix, UNC, or Windows drive path"
                ));
            }
            if filesystem_root(value) {
                return Err(format!(
                    "Runtime binding {name} must not target a filesystem root"
                ));
            }
            if value.contains("{binding") {
                return Err(format!(
                    "Runtime binding {name} must not contain another binding placeholder"
                ));
            }
            if values.insert(name.to_string(), value.to_string()).is_some() {
                return Err(format!("Duplicate runtime binding name: {name}"));
            }
            if let Some(existing) = paths.insert(value.to_string(), name.to_string()) {
                return Err(format!(
                    "Runtime bindings {existing} and {name} target the same path"
                ));
            }
        }
        Ok(Self { values })
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PortabilityReport {
    pub contract: &'static str,
    pub benchmark_id: String,
    pub portable: bool,
    pub required_bindings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct NormalizedManifest {
    pub manifest: BenchmarkManifest,
    pub replacement_count: usize,
    pub required_bindings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NormalizationReport {
    pub contract: &'static str,
    pub benchmark_id: String,
    pub source_manifest_digest: String,
    pub normalized_manifest_digest: String,
    pub output_path: String,
    pub replacement_count: usize,
    pub required_bindings: Vec<String>,
    pub portable: bool,
}

pub fn normalize_manifest(
    manifest: &BenchmarkManifest,
    bindings: &RuntimeBindings,
) -> Result<NormalizedManifest, String> {
    if bindings.is_empty() {
        return Err("Manifest normalization requires at least one --bind entry".to_string());
    }

    let mut value = serde_json::to_value(manifest)
        .map_err(|error| format!("Failed to encode benchmark manifest: {error}"))?;
    let mut ordered = bindings.values.iter().collect::<Vec<_>>();
    ordered.sort_by(|(left_name, left), (right_name, right)| {
        right
            .len()
            .cmp(&left.len())
            .then_with(|| left_name.cmp(right_name))
    });
    let mut replacement_counts = bindings
        .values
        .keys()
        .map(|name| (name.clone(), 0_usize))
        .collect::<BTreeMap<_, _>>();
    visit_strings_mut(&mut value, &mut |text| {
        for (name, path) in &ordered {
            let count = text.match_indices(path.as_str()).count();
            if count == 0 {
                continue;
            }
            *text = text.replace(path.as_str(), &binding_placeholder(name));
            *replacement_counts.get_mut(*name).expect("binding count") += count;
        }
    });

    let unused = replacement_counts
        .iter()
        .filter_map(|(name, count)| (*count == 0).then_some(name.as_str()))
        .collect::<Vec<_>>();
    if !unused.is_empty() {
        return Err(format!(
            "Normalization bindings were not present in the manifest: {}",
            unused.join(", ")
        ));
    }

    let normalized = serde_json::from_value::<BenchmarkManifest>(value)
        .map_err(|error| format!("Failed to decode normalized benchmark manifest: {error}"))?;
    let portability = validate_portable_manifest(&normalized);
    if !portability.portable {
        return Err(format!(
            "Normalized benchmark manifest is not portable: {}",
            portability.errors.join("; ")
        ));
    }
    Ok(NormalizedManifest {
        manifest: normalized,
        replacement_count: replacement_counts.values().sum(),
        required_bindings: portability.required_bindings,
    })
}

pub fn validate_portable_manifest(manifest: &BenchmarkManifest) -> PortabilityReport {
    let mut errors = Vec::new();
    let mut required_bindings = BTreeSet::new();
    match serde_json::to_value(manifest) {
        Ok(value) => visit_strings(&value, "$", &mut |path, text| {
            if contains_absolute_or_host_path(text) {
                errors.push(format!(
                    "{path} contains an absolute or host-specific path; use {{binding:<name>}}"
                ));
            }
            match binding_names(text, path) {
                Ok(names) => required_bindings.extend(names),
                Err(error) => errors.push(error),
            }
        }),
        Err(error) => errors.push(format!("Failed to inspect benchmark manifest: {error}")),
    }
    PortabilityReport {
        contract: PORTABILITY_CONTRACT,
        benchmark_id: manifest.benchmark_id.clone(),
        portable: errors.is_empty(),
        required_bindings: required_bindings.into_iter().collect(),
        errors,
    }
}

pub fn resolve_manifest_bindings(
    manifest: &BenchmarkManifest,
    bindings: &RuntimeBindings,
) -> Result<BenchmarkManifest, String> {
    let mut resolved = manifest.clone();
    for cell in &mut resolved.cells {
        for subject in &mut cell.subjects {
            let workspace_label = format!(
                "cell {} subject {} workspace_root",
                cell.cell_id, subject.subject_id
            );
            subject.workspace_root =
                resolve_path(&workspace_label, &subject.workspace_root, bindings)?;
            if !absolute_path_like(&subject.workspace_root.to_string_lossy()) {
                return Err(format!(
                    "{workspace_label} must resolve to an absolute runtime path"
                ));
            }
            resolve_command(
                &format!(
                    "cell {} subject {} command",
                    cell.cell_id, subject.subject_id
                ),
                &mut subject.command,
                bindings,
            )?;
            for (kind, commands) in [
                ("reset", &mut subject.reset_commands),
                ("prepare", &mut subject.prepare_commands),
                ("cleanup", &mut subject.cleanup_commands),
            ] {
                for (index, command) in commands.iter_mut().enumerate() {
                    resolve_command(
                        &format!(
                            "cell {} subject {} {kind} command {index}",
                            cell.cell_id, subject.subject_id
                        ),
                        command,
                        bindings,
                    )?;
                }
            }
            resolve_command(
                &format!(
                    "cell {} subject {} history_node_probe",
                    cell.cell_id, subject.subject_id
                ),
                &mut subject.history_node_probe,
                bindings,
            )?;
            resolve_command(
                &format!(
                    "cell {} subject {} outcome_probe",
                    cell.cell_id, subject.subject_id
                ),
                &mut subject.outcome_probe,
                bindings,
            )?;
            if let Some(path) = subject.metrics_json_path.as_mut() {
                *path = resolve_path(
                    &format!(
                        "cell {} subject {} metrics_json_path",
                        cell.cell_id, subject.subject_id
                    ),
                    path,
                    bindings,
                )?;
            }
        }
    }
    Ok(resolved)
}

pub fn encode_manifest(manifest: &BenchmarkManifest) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("Failed to encode benchmark manifest: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn resolve_command(
    label: &str,
    command: &mut CommandSpec,
    bindings: &RuntimeBindings,
) -> Result<(), String> {
    command.program = resolve_text(&format!("{label} program"), &command.program, bindings)?;
    for (index, arg) in command.args.iter_mut().enumerate() {
        *arg = resolve_text(&format!("{label} arg {index}"), arg, bindings)?;
    }
    if let Some(cwd) = command.cwd.as_mut() {
        *cwd = resolve_path(&format!("{label} cwd"), cwd, bindings)?;
    }
    for (name, value) in &mut command.env {
        *value = resolve_text(&format!("{label} env {name}"), value, bindings)?;
    }
    Ok(())
}

fn resolve_path(label: &str, path: &Path, bindings: &RuntimeBindings) -> Result<PathBuf, String> {
    resolve_text(label, &path.to_string_lossy(), bindings).map(PathBuf::from)
}

fn resolve_text(label: &str, value: &str, bindings: &RuntimeBindings) -> Result<String, String> {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find("{binding") {
        output.push_str(&remaining[..start]);
        let placeholder = &remaining[start..];
        if !placeholder.starts_with("{binding:") {
            return Err(format!("{label} contains a malformed runtime binding"));
        }
        let end = placeholder
            .find('}')
            .ok_or_else(|| format!("{label} contains an unterminated runtime binding"))?;
        let name = &placeholder["{binding:".len()..end];
        if !valid_binding_name(name) {
            return Err(format!("{label} contains invalid runtime binding {name:?}"));
        }
        let value = bindings
            .get(name)
            .ok_or_else(|| format!("{label} requires missing runtime binding {name}"))?;
        output.push_str(value);
        remaining = &placeholder[end + 1..];
    }
    output.push_str(remaining);
    Ok(output)
}

fn binding_names(value: &str, label: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut remaining = value;
    while let Some(start) = remaining.find("{binding") {
        let placeholder = &remaining[start..];
        if !placeholder.starts_with("{binding:") {
            return Err(format!("{label} contains a malformed runtime binding"));
        }
        let end = placeholder
            .find('}')
            .ok_or_else(|| format!("{label} contains an unterminated runtime binding"))?;
        let name = &placeholder["{binding:".len()..end];
        if !valid_binding_name(name) {
            return Err(format!("{label} contains invalid runtime binding {name:?}"));
        }
        names.push(name.to_string());
        remaining = &placeholder[end + 1..];
    }
    Ok(names)
}

fn binding_placeholder(name: &str) -> String {
    format!("{{binding:{name}}}")
}

fn valid_binding_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn filesystem_root(value: &str) -> bool {
    value == "/"
        || value == "\\\\"
        || (value.len() == 3
            && value.as_bytes()[0].is_ascii_alphabetic()
            && value.as_bytes()[1] == b':'
            && matches!(value.as_bytes()[2], b'/' | b'\\'))
}

fn contains_absolute_or_host_path(value: &str) -> bool {
    const HOST_MARKERS: &[&str] = &[
        "/Users/",
        "/home/",
        "/Volumes/",
        "/private/tmp/",
        "/tmp/",
        "\\Users\\",
        ".ait-worktree-links",
    ];
    if HOST_MARKERS.iter().any(|marker| value.contains(marker)) {
        return true;
    }
    value
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '=' | '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';'
                )
        })
        .filter(|token| !token.is_empty())
        .any(absolute_path_like)
}

fn absolute_path_like(value: &str) -> bool {
    Path::new(value).is_absolute()
        || value.starts_with("\\\\")
        || (value.len() >= 3
            && value.as_bytes()[0].is_ascii_alphabetic()
            && value.as_bytes()[1] == b':'
            && matches!(value.as_bytes()[2], b'/' | b'\\'))
}

fn visit_strings_mut(value: &mut Value, visitor: &mut impl FnMut(&mut String)) {
    match value {
        Value::String(text) => visitor(text),
        Value::Array(values) => {
            for value in values {
                visit_strings_mut(value, visitor);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                visit_strings_mut(value, visitor);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn visit_strings(value: &Value, path: &str, visitor: &mut impl FnMut(&str, &str)) {
    match value {
        Value::String(text) => visitor(path, text),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                visit_strings(value, &format!("{path}[{index}]"), visitor);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                visit_strings(value, &format!("{path}.{key}"), visitor);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufRead, BufReader};

    use crate::model::{
        CampaignScope, CellSpec, EnvironmentPin, FixtureDeclaration, FixtureScale, RawRecord,
        SampleClass, SamplingPolicy, SubjectSpec, Temperature,
    };
    use crate::{load_manifest, MANIFEST_CONTRACT};

    use super::*;

    #[test]
    fn binding_parser_is_cross_platform_and_fail_closed() {
        let bindings = RuntimeBindings::parse(&[
            "unix=/opt/ait/bin/ait".to_string(),
            "windows=C:\\ait\\bin\\ait.exe".to_string(),
            "unc=\\\\server\\share\\ait".to_string(),
        ])
        .unwrap();
        assert_eq!(bindings.names(), vec!["unc", "unix", "windows"]);
        assert!(RuntimeBindings::parse(&["UPPER=/opt/ait".to_string()]).is_err());
        assert!(RuntimeBindings::parse(&["relative=target/ait".to_string()]).is_err());
        assert!(RuntimeBindings::parse(&["root=/".to_string()]).is_err());
        assert!(
            RuntimeBindings::parse(&["one=/opt/ait".to_string(), "two=/opt/ait".to_string(),])
                .is_err()
        );
    }

    #[test]
    fn normalization_is_longest_first_portable_and_runtime_resolvable() {
        let manifest = manifest_fixture(
            "/opt/ait-campaign/workspaces/ait",
            "/opt/ait-campaign/bin/ait-cli",
        );
        let bindings = RuntimeBindings::parse(&[
            "campaign=/opt/ait-campaign".to_string(),
            "ait-cli=/opt/ait-campaign/bin/ait-cli".to_string(),
        ])
        .unwrap();
        let normalized = normalize_manifest(&manifest, &bindings).unwrap();
        assert_eq!(normalized.replacement_count, 5);
        assert_eq!(
            normalized.required_bindings,
            vec!["ait-cli".to_string(), "campaign".to_string()]
        );
        let encoded = String::from_utf8(encode_manifest(&normalized.manifest).unwrap()).unwrap();
        assert!(!encoded.contains("/opt/ait-campaign"));
        assert!(encoded.contains("{binding:ait-cli}"));
        assert!(encoded.contains("{binding:campaign}/workspaces/ait"));

        let runtime = RuntimeBindings::parse(&[
            "campaign=/srv/benchmark-run".to_string(),
            "ait-cli=/srv/ait/bin/ait-cli".to_string(),
        ])
        .unwrap();
        let resolved = resolve_manifest_bindings(&normalized.manifest, &runtime).unwrap();
        let subject = &resolved.cells[0].subjects[0];
        assert_eq!(
            subject.workspace_root,
            PathBuf::from("/srv/benchmark-run/workspaces/ait")
        );
        assert_eq!(subject.command.program, "/srv/ait/bin/ait-cli");
        assert_eq!(subject.history_node_probe.args[1], "/srv/ait/bin/ait-cli");
    }

    #[test]
    fn normalization_rejects_unused_or_incomplete_bindings() {
        let manifest = manifest_fixture("/opt/workspace", "/opt/bin/ait");
        let unused = RuntimeBindings::parse(&[
            "workspace=/opt/workspace".to_string(),
            "ait=/opt/bin/ait".to_string(),
            "unused=/srv/unused".to_string(),
        ])
        .unwrap();
        assert!(normalize_manifest(&manifest, &unused)
            .unwrap_err()
            .contains("unused"));

        let incomplete = RuntimeBindings::parse(&["workspace=/opt/workspace".to_string()]).unwrap();
        assert!(normalize_manifest(&manifest, &incomplete)
            .unwrap_err()
            .contains("not portable"));
    }

    #[test]
    fn portability_detects_unix_windows_and_worktree_paths() {
        let mut manifest = manifest_fixture("C:\\Users\\builder\\workspace", "/usr/local/bin/ait");
        manifest
            .limitations
            .push("built under .ait-worktree-links/private-task before publication".to_string());
        let report = validate_portable_manifest(&manifest);
        assert!(!report.portable);
        assert!(report.errors.len() >= 3, "{:?}", report.errors);
    }

    #[test]
    fn missing_runtime_binding_fails_before_process_launch() {
        let manifest = manifest_fixture("{binding:workspace}", "{binding:ait}");
        let bindings = RuntimeBindings::parse(&["workspace=/tmp/workspace".to_string()]).unwrap();
        let error = resolve_manifest_bindings(&manifest, &bindings).unwrap_err();
        assert!(error.contains("missing runtime binding ait"));
    }

    #[test]
    fn committed_evidence_is_portable_and_digest_linked() {
        let evidence_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("evidence")
            .join("stage4-2026-07-19");
        for entry in fs::read_dir(&evidence_root).unwrap() {
            let path = entry.unwrap().path();
            if !path.is_file() {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                !contains_absolute_or_host_path(&text),
                "committed evidence contains a host path: {}",
                path.display()
            );
        }

        for prefix in ["baseline", "candidate"] {
            let manifest_path = evidence_root.join(format!("{prefix}-manifest.json"));
            let raw_path = evidence_root.join(format!("{prefix}-raw.jsonl"));
            let report_path = evidence_root.join(format!("{prefix}-report.json"));
            let normalization_path = evidence_root.join(format!("{prefix}-normalization.json"));
            let (manifest, digest) = load_manifest(&manifest_path).unwrap();
            let portability = validate_portable_manifest(&manifest);
            assert!(portability.portable, "{:?}", portability.errors);

            let first_line = BufReader::new(fs::File::open(raw_path).unwrap())
                .lines()
                .next()
                .unwrap()
                .unwrap();
            let RawRecord::Header(header) = serde_json::from_str(&first_line).unwrap() else {
                panic!("raw evidence must begin with a header");
            };
            assert_eq!(header.manifest_digest, digest);
            let report: Value = serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap();
            assert_eq!(report["manifest_digest"], Value::String(digest.clone()));
            let normalization: Value =
                serde_json::from_slice(&fs::read(normalization_path).unwrap()).unwrap();
            assert_eq!(
                normalization["contract"],
                Value::String(NORMALIZATION_CONTRACT.to_string())
            );
            assert_eq!(
                normalization["normalized_manifest_digest"],
                Value::String(digest)
            );
            assert_eq!(normalization["portable"], Value::Bool(true));
        }
    }

    fn manifest_fixture(workspace: &str, program: &str) -> BenchmarkManifest {
        let command = CommandSpec {
            program: program.to_string(),
            args: vec!["status".to_string()],
            cwd: Some(PathBuf::from("{workspace}")),
            env: BTreeMap::new(),
            expected_exit_codes: vec![0],
        };
        let history_node_probe = CommandSpec {
            program: "/opt/ait-campaign/bin/helper".to_string(),
            args: vec!["--program".to_string(), program.to_string()],
            cwd: Some(PathBuf::from("{workspace}")),
            env: BTreeMap::new(),
            expected_exit_codes: vec![0],
        };
        BenchmarkManifest {
            contract: MANIFEST_CONTRACT.to_string(),
            benchmark_id: "portable-test".to_string(),
            protocol_revision: "vcs-performance-test".to_string(),
            campaign_scope: CampaignScope::FocusedSlice,
            seed: 1,
            sampling: SamplingPolicy {
                warmup_iterations: 5,
                measured_local_iterations: 50,
                measured_cold_iterations: 30,
            },
            environment: EnvironmentPin {
                captured_at: "2026-08-22T00:00:00Z".to_string(),
                os: "test".to_string(),
                architecture: "test".to_string(),
                filesystem: "test".to_string(),
                storage_medium: "test".to_string(),
                cpu: "test".to_string(),
                memory_bytes: 1,
                rust_version: "test".to_string(),
                git_version: "test".to_string(),
                ait_version: "test".to_string(),
                repository_snapshot: "test".to_string(),
                server_revision: "test".to_string(),
                network_profile: "test".to_string(),
                cache_drop_method: "test".to_string(),
                command_options: BTreeMap::new(),
            },
            fixtures: vec![FixtureDeclaration {
                fixture_id: "small-test".to_string(),
                revision: "1".to_string(),
                scale: FixtureScale::Small,
                kind: "synthetic".to_string(),
                source: "fixture://test".to_string(),
                redistribution: "generated".to_string(),
                content_digest: format!("sha256:{}", "a".repeat(64)),
                file_count: 1,
                total_bytes: 1,
                history_nodes: 1,
                features: vec![],
            }],
            cells: vec![CellSpec {
                cell_id: "small-status".to_string(),
                fixture_id: "small-test".to_string(),
                operation: "status_clean".to_string(),
                temperature: Temperature::Warm,
                sample_class: SampleClass::Local,
                subjects: vec![SubjectSpec {
                    subject_id: "ait".to_string(),
                    role: "ait".to_string(),
                    workspace_root: PathBuf::from(workspace),
                    metadata_excludes: vec![".ait".to_string()],
                    command: command.clone(),
                    reset_commands: vec![],
                    prepare_commands: vec![],
                    cleanup_commands: vec![],
                    history_node_probe,
                    outcome_probe: command,
                    metrics_json_path: None,
                }],
            }],
            bootstrap_resamples: 1_000,
            limitations: vec![],
        }
    }
}
