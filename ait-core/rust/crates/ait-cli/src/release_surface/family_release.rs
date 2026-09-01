use super::*;
use std::io::Read;

pub const FAMILY_RELEASE_PROFILE: &str = "family";
pub(super) const FAMILY_RELEASE_MANIFEST_PATH: &str = "ait-release-family.json";
pub(super) const FAMILY_RELEASE_MANIFEST_CONTRACT: &str = "ait.release.family/v3";
const PUBLISHED_LEGACY_NATIVE_BUNDLE_VERSION: &str = "1.1.0";
const PUBLISHED_LEGACY_NATIVE_BUNDLE_TAG: &str = "v1.1.0";
const PUBLISHED_LEGACY_NATIVE_BUNDLE_SNAPSHOT: &str = "SNP-1D024C5B512C";
const PUBLISHED_LEGACY_NATIVE_BUNDLE_MANIFEST_SHA256: &str =
    "e85722913ed6724eb8f9cbb56fc2fd4a84ebcaad9fa84acb2e2971b2cc6c87fd";
pub(super) const FAMILY_RELEASE_CANDIDATE_CONTRACT: &str = "ait.release.family.candidate/v1";
pub(super) const FAMILY_RELEASE_CHECK_CONTRACT: &str = "ait.release.family.check/v1";
pub(super) const FAMILY_RELEASE_BUILD_CONTRACT: &str = "ait.release.family.build/v1";
pub(super) const FAMILY_RELEASE_FROZEN_MANIFEST_CONTRACT: &str = "ait.release.family.frozen/v1";
pub(super) const FAMILY_RELEASE_PROMOTION_CONTRACT: &str = "ait.release.family.promotion/v1";

const FAMILY_CANDIDATE_FILENAME: &str = "ait-release.candidate.json";
const FAMILY_CHECK_FILENAME: &str = "ait-release.check.json";
const FAMILY_BUILD_FILENAME: &str = "ait-release.build.json";
const FAMILY_PROMOTION_FILENAME: &str = "ait-release.promotion.json";
const FAMILY_FROZEN_DIRNAME: &str = "frozen";
const FAMILY_FROZEN_MANIFEST_FILENAME: &str = "ait-release-family.manifest.json";
const FAMILY_CHECKSUM_FILENAME: &str = "SHA256SUMS";
const COMPONENT_RECEIPT_FILENAME: &str = "ait-release.receipt.json";
const PUBLIC_SOURCE_MAPPING_FILENAME: &str = "ait-monorepo-source.json";
const PUBLIC_SOURCE_MAPPING_CONTRACT: &str = "ait.release.monorepo-source/v1";
const PUBLIC_SOURCE_IDENTITY: &str = "weita2026/ait-native";

const MAX_FAMILY_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_DOSSIER_BYTES: usize = 4 * 1024 * 1024;
const MAX_COMPONENT_RECEIPT_BYTES: usize = 8 * 1024 * 1024;
const MAX_FAMILY_COMPONENTS: usize = 64;
const MAX_FAMILY_TARGETS: usize = 32;
const MAX_COMPONENT_ARTIFACT_KINDS: usize = 32;
const MAX_FAMILY_DISTRIBUTIONS: usize = 128;
const MAX_DISTRIBUTION_COMPONENTS: usize = 64;
const MAX_COMPATIBILITY_ROWS: usize = 64;
const MAX_RECEIPTS: usize = 64;
const MAX_RECEIPT_TREE_ENTRIES: usize = 4096;
const MAX_RECEIPT_TREE_DEPTH: usize = 16;
const MAX_FAMILY_TEXT_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FamilyIdentity {
    name: String,
    version: String,
    channel: String,
    tag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FamilyArtifactRequirement {
    kind: String,
    targets: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FamilyComponentRequirement {
    id: String,
    source_repository: String,
    source_snapshot: String,
    ecosystem: String,
    license: String,
    version_scheme: String,
    version: String,
    artifacts: Vec<FamilyArtifactRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FamilyDistributionRequirement {
    channel: String,
    role: String,
    identity: String,
    components: Vec<String>,
    targets: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FamilyReleaseManifest {
    family: FamilyIdentity,
    targets: Vec<String>,
    public_source: JsonValue,
    components: Vec<FamilyComponentRequirement>,
    distributions: Vec<FamilyDistributionRequirement>,
    compatibility: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct FamilyArtifactSource {
    component: String,
    ecosystem: String,
    kind: String,
    target: Option<String>,
    sha256: String,
    size_bytes: u64,
    receipt_relative_path: String,
    source_relative_path: String,
    source_path: PathBuf,
}

#[derive(Clone, Debug)]
struct FamilyLicenseMaterialSource {
    source_repository: String,
    source_snapshot: String,
    role: String,
    declared_path: String,
    sha256: String,
    size_bytes: u64,
    receipt_relative_paths: BTreeSet<String>,
    source_relative_path: String,
    source_path: PathBuf,
}

#[derive(Clone, Debug)]
struct FamilyAdmission {
    record: JsonValue,
    artifacts: Vec<FamilyArtifactSource>,
    license_material: Vec<FamilyLicenseMaterialSource>,
}

#[derive(Clone, Debug)]
struct PublicGitSubtreeAuthority {
    source_snapshot: String,
    source_manifest_hash: String,
    source_snapshot_created_at: String,
    path: String,
    exported_content_sha256: String,
}

#[derive(Clone, Debug)]
struct PublicGitSourceAuthority {
    root: PathBuf,
    mapping_sha256: String,
    content_sha256: String,
    coordinator_snapshot: String,
    coordinator_manifest_hash: String,
    coordinator_created_at: String,
    subtrees: BTreeMap<String, PublicGitSubtreeAuthority>,
}

impl FamilyIdentity {
    fn to_json(&self) -> JsonValue {
        json!({
            "name": self.name,
            "version": self.version,
            "channel": self.channel,
            "tag": self.tag,
        })
    }
}

impl FamilyArtifactRequirement {
    fn to_json(&self) -> JsonValue {
        json!({
            "kind": self.kind,
            "targets": self.targets,
        })
    }

    fn expected_keys(&self) -> Vec<(String, Option<String>)> {
        if self.targets.is_empty() {
            vec![(self.kind.clone(), None)]
        } else {
            self.targets
                .iter()
                .map(|target| (self.kind.clone(), Some(target.clone())))
                .collect()
        }
    }
}

impl FamilyComponentRequirement {
    fn to_json(&self) -> JsonValue {
        json!({
            "id": self.id,
            "source_repository": self.source_repository,
            "source_snapshot": self.source_snapshot,
            "ecosystem": self.ecosystem,
            "license": self.license,
            "version_scheme": self.version_scheme,
            "version": self.version,
            "artifacts": self.artifacts.iter().map(FamilyArtifactRequirement::to_json).collect::<Vec<_>>(),
        })
    }

    fn expected_artifact_keys(&self) -> BTreeSet<(String, Option<String>)> {
        self.artifacts
            .iter()
            .flat_map(FamilyArtifactRequirement::expected_keys)
            .collect()
    }

    fn supports_distribution_target(&self, target: &str) -> bool {
        self.artifacts.iter().any(|artifact| {
            artifact.targets.is_empty() || artifact.targets.iter().any(|row| row == target)
        })
    }
}

impl FamilyDistributionRequirement {
    fn to_json(&self) -> JsonValue {
        json!({
            "channel": self.channel,
            "role": self.role,
            "identity": self.identity,
            "components": self.components,
            "targets": self.targets,
        })
    }
}

impl FamilyReleaseManifest {
    fn to_json(&self) -> JsonValue {
        json!({
            "schema": FAMILY_RELEASE_MANIFEST_CONTRACT,
            "family": self.family.to_json(),
            "targets": self.targets,
            "public_source": self.public_source,
            "components": self.components.iter().map(FamilyComponentRequirement::to_json).collect::<Vec<_>>(),
            "distributions": self.distributions.iter().map(FamilyDistributionRequirement::to_json).collect::<Vec<_>>(),
            "compatibility": self.compatibility,
        })
    }

    fn expected_artifact_count(&self) -> usize {
        self.components
            .iter()
            .map(|component| component.expected_artifact_keys().len())
            .sum()
    }
}

fn family_object<'a>(
    value: &'a JsonValue,
    context: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{context} must be a JSON object."))
}

fn family_known_fields(
    object: &JsonMap<String, JsonValue>,
    allowed: &[&str],
    context: &str,
) -> Result<(), String> {
    let mut unknown = object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unknown.sort();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{context} contains unknown field(s): {}.",
            unknown.join(", ")
        ))
    }
}

fn family_text(value: Option<&JsonValue>, context: &str) -> Result<String, String> {
    let text = value
        .and_then(JsonValue::as_str)
        .ok_or_else(|| format!("{context} must be a string."))?;
    if text.is_empty()
        || text.trim() != text
        || text.len() > MAX_FAMILY_TEXT_BYTES
        || text.chars().any(char::is_control)
    {
        return Err(format!(
            "{context} must be a non-empty, bounded single-line string without surrounding whitespace."
        ));
    }
    Ok(text.to_string())
}

fn family_identifier(value: Option<&JsonValue>, context: &str) -> Result<String, String> {
    let text = family_text(value, context)?;
    if text.len() > 128
        || !text
            .chars()
            .next()
            .map(|character| character.is_ascii_alphanumeric())
            .unwrap_or(false)
        || !text.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(format!(
            "{context} must start with an ASCII letter or digit and contain only ASCII letters, digits, '.', '_', or '-'."
        ));
    }
    Ok(text)
}

fn family_spdx_expression(value: Option<&JsonValue>, context: &str) -> Result<String, String> {
    let text = family_text(value, context)?;
    if text.len() > 128
        || !text.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '-' | '+' | '(' | ')' | ' ')
        })
        || text.contains("  ")
    {
        return Err(format!(
            "{context} must be a bounded SPDX license expression using only ASCII license identifiers, spaces, '+', and parentheses."
        ));
    }
    Ok(text)
}

fn family_distribution_identity(
    value: Option<&JsonValue>,
    context: &str,
) -> Result<String, String> {
    let text = family_text(value, context)?;
    if text.len() > 256
        || text.starts_with('/')
        || text.ends_with('/')
        || text.contains("//")
        || !text.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | '-' | '+' | '/' | '@' | ':')
        })
    {
        return Err(format!(
            "{context} must be a bounded registry or package identity using only ASCII letters, digits, '.', '_', '-', '+', '/', '@', or ':'."
        ));
    }
    Ok(text)
}

fn family_snapshot_id(value: Option<&JsonValue>, context: &str) -> Result<String, String> {
    let text = family_text(value, context)?;
    let suffix = text.strip_prefix("SNP-").unwrap_or_default();
    if suffix.len() < 12
        || suffix.len() > 64
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "{context} must be an exact SNP- prefixed hexadecimal Snapshot identity."
        ));
    }
    Ok(text)
}

fn family_relative_path(value: &str, context: &str) -> Result<String, String> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || Path::new(value).is_absolute()
        || value.contains('\0')
        || value.contains('\\')
        || value.contains(':')
        || value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!(
            "{context} must be a normalized '/'-separated relative path without traversal, drive prefixes, or backslashes."
        ));
    }
    Ok(value.to_string())
}

fn family_string_array(
    value: Option<&JsonValue>,
    context: &str,
    max: usize,
    allow_empty: bool,
) -> Result<Vec<String>, String> {
    let rows = value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context} must be an array."))?;
    if (!allow_empty && rows.is_empty()) || rows.len() > max {
        return Err(format!(
            "{context} must contain {} to {max} values.",
            if allow_empty { 0 } else { 1 }
        ));
    }
    let mut result = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let value = family_identifier(Some(row), &format!("{context}[{index}]"))?;
        if !seen.insert(value.clone()) {
            return Err(format!("{context} contains duplicate value {value:?}."));
        }
        result.push(value);
    }
    Ok(result)
}

fn canonical_numeric_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

fn stable_version_parts(value: &str) -> Option<(&str, &str, &str)> {
    let mut parts = value.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next()?;
    if parts.next().is_some()
        || !canonical_numeric_identifier(major)
        || !canonical_numeric_identifier(minor)
        || !canonical_numeric_identifier(patch)
    {
        return None;
    }
    Some((major, minor, patch))
}

fn validate_family_version(version: &str, channel: &str) -> Result<(), String> {
    match channel {
        "stable" if stable_version_parts(version).is_some() => Ok(()),
        "stable" => Err(format!(
            "Stable family version {version:?} must use canonical MAJOR.MINOR.PATCH syntax."
        )),
        "rc" => {
            let Some((base, ordinal)) = version.rsplit_once("-rc.") else {
                return Err(format!(
                    "RC family version {version:?} must use canonical MAJOR.MINOR.PATCH-rc.N syntax."
                ));
            };
            if stable_version_parts(base).is_none() || !canonical_numeric_identifier(ordinal) {
                return Err(format!(
                    "RC family version {version:?} must use canonical MAJOR.MINOR.PATCH-rc.N syntax."
                ));
            }
            Ok(())
        }
        _ => Err(format!(
            "Release channel must be either \"rc\" or \"stable\", got {channel:?}."
        )),
    }
}

pub(super) fn native_runner_bundle_required(version: &str) -> Result<bool, String> {
    let base = version
        .rsplit_once("-rc.")
        .map(|(base, _)| base)
        .unwrap_or(version);
    let (major, minor, _) = stable_version_parts(base).ok_or_else(|| {
        format!("Family version {version:?} cannot select the native runner-bundle contract.")
    })?;
    let major = major
        .parse::<u64>()
        .map_err(|_| format!("Family version {version:?} has an out-of-range major component."))?;
    let minor = minor
        .parse::<u64>()
        .map_err(|_| format!("Family version {version:?} has an out-of-range minor component."))?;
    Ok(major > 1 || (major == 1 && minor >= 1))
}

pub(super) fn is_exact_published_legacy_native_bundle_source(
    version: &str,
    channel: &str,
    tag: &str,
    snapshot_id: &str,
    family_manifest_sha256: &str,
) -> bool {
    version == PUBLISHED_LEGACY_NATIVE_BUNDLE_VERSION
        && channel == "stable"
        && tag == PUBLISHED_LEGACY_NATIVE_BUNDLE_TAG
        && snapshot_id == PUBLISHED_LEGACY_NATIVE_BUNDLE_SNAPSHOT
        && family_manifest_sha256 == PUBLISHED_LEGACY_NATIVE_BUNDLE_MANIFEST_SHA256
}

fn validate_native_product_bundle_contract(
    family: &FamilyReleaseManifest,
    snapshot_id: &str,
    family_manifest_sha256: &str,
) -> Result<(), String> {
    let runner_required = native_runner_bundle_required(&family.family.version)?;
    let exact_published_exception = is_exact_published_legacy_native_bundle_source(
        &family.family.version,
        &family.family.channel,
        &family.family.tag,
        snapshot_id,
        family_manifest_sha256,
    );
    let legacy = ["ait", "ait-server"].into_iter().collect::<BTreeSet<_>>();
    let runner_bundle = ["ait", "ait-server", "ait-runner"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let native_products = family
        .distributions
        .iter()
        .filter(|distribution| {
            distribution.role == "product"
                && matches!(distribution.channel.as_str(), "homebrew" | "apt" | "winget")
        })
        .collect::<Vec<_>>();
    if runner_required && !exact_published_exception {
        let channels = native_products
            .iter()
            .map(|distribution| distribution.channel.as_str())
            .collect::<BTreeSet<_>>();
        let required_channels = ["homebrew", "apt", "winget"]
            .into_iter()
            .collect::<BTreeSet<_>>();
        if native_products.len() != 3 || channels != required_channels {
            return Err(format!(
                "Family version {:?} must declare exactly one Homebrew, apt, and WinGet native product distribution.",
                family.family.version
            ));
        }
    }

    for distribution in native_products {
        let components = distribution
            .components
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let legacy_components = components == legacy && distribution.components.len() == 2;
        let runner_components = components == runner_bundle && distribution.components.len() == 3;
        if runner_components
            || (legacy_components && (!runner_required || exact_published_exception))
        {
            continue;
        }
        if legacy_components && runner_required {
            return Err(format!(
                "{} product distribution {:?} must bundle ait, ait-server, and ait-runner for family version {:?}; the two-command layout is admitted only for 1.0.x and the exact immutable published 1.1.0 family.",
                distribution.channel, distribution.identity, family.family.version
            ));
        }
        return Err(format!(
            "{} product distribution {:?} has an invalid native command component set.",
            distribution.channel, distribution.identity
        ));
    }
    Ok(())
}

fn expected_pep440_version(family_version: &str, channel: &str) -> Result<String, String> {
    if channel == "stable" {
        return Ok(family_version.to_string());
    }
    let (base, ordinal) = family_version.rsplit_once("-rc.").ok_or_else(|| {
        format!("RC family version {family_version:?} cannot be mapped to a PEP 440 release.")
    })?;
    Ok(format!("{base}rc{ordinal}"))
}

fn parse_family_artifacts(
    value: Option<&JsonValue>,
    context: &str,
    family_targets: &BTreeSet<String>,
) -> Result<Vec<FamilyArtifactRequirement>, String> {
    let rows = value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context} must be an array."))?;
    if rows.is_empty() || rows.len() > MAX_COMPONENT_ARTIFACT_KINDS {
        return Err(format!(
            "{context} must contain between 1 and {MAX_COMPONENT_ARTIFACT_KINDS} artifact requirements."
        ));
    }
    let mut requirements = Vec::with_capacity(rows.len());
    let mut expected_keys = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let row_context = format!("{context}[{index}]");
        let object = family_object(row, &row_context)?;
        family_known_fields(object, &["kind", "targets"], &row_context)?;
        let kind = family_identifier(object.get("kind"), &format!("{row_context}.kind"))?;
        let targets = family_string_array(
            object.get("targets"),
            &format!("{row_context}.targets"),
            MAX_FAMILY_TARGETS,
            true,
        )?;
        for target in &targets {
            if !family_targets.contains(target) {
                return Err(format!(
                    "{row_context}.targets contains {target:?}, which is absent from the family target matrix."
                ));
            }
        }
        let requirement = FamilyArtifactRequirement { kind, targets };
        for key in requirement.expected_keys() {
            if !expected_keys.insert(key.clone()) {
                return Err(format!(
                    "{context} contains duplicate artifact requirement {key:?}."
                ));
            }
        }
        requirements.push(requirement);
    }
    Ok(requirements)
}

fn parse_family_distributions(
    value: Option<&JsonValue>,
    family_targets: &BTreeSet<String>,
    components: &[FamilyComponentRequirement],
) -> Result<Vec<FamilyDistributionRequirement>, String> {
    let context = "ait-release-family.json.distributions";
    let rows = value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context} must be an array."))?;
    if rows.is_empty() || rows.len() > MAX_FAMILY_DISTRIBUTIONS {
        return Err(format!(
            "{context} must contain between 1 and {MAX_FAMILY_DISTRIBUTIONS} distribution requirements."
        ));
    }

    let component_map = components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let mut distributions = Vec::with_capacity(rows.len());
    let mut identities = BTreeSet::new();
    let mut distributed_components = BTreeSet::new();

    for (index, row) in rows.iter().enumerate() {
        let row_context = format!("{context}[{index}]");
        let object = family_object(row, &row_context)?;
        family_known_fields(
            object,
            &["channel", "role", "identity", "components", "targets"],
            &row_context,
        )?;
        let channel = family_identifier(object.get("channel"), &format!("{row_context}.channel"))?;
        if !matches!(
            channel.as_str(),
            "github" | "pypi" | "npm" | "oci" | "homebrew" | "apt" | "winget"
        ) {
            return Err(format!(
                "{row_context}.channel must be one of github, pypi, npm, oci, homebrew, apt, or winget, got {channel:?}."
            ));
        }
        let role = family_identifier(object.get("role"), &format!("{row_context}.role"))?;
        if !matches!(role.as_str(), "product" | "standalone" | "implementation") {
            return Err(format!(
                "{row_context}.role must be product, standalone, or implementation, got {role:?}."
            ));
        }
        let identity = family_distribution_identity(
            object.get("identity"),
            &format!("{row_context}.identity"),
        )?;
        if !identities.insert((channel.clone(), identity.clone())) {
            return Err(format!(
                "{context} contains duplicate channel/identity pair ({channel:?}, {identity:?})."
            ));
        }
        let component_ids = family_string_array(
            object.get("components"),
            &format!("{row_context}.components"),
            MAX_DISTRIBUTION_COMPONENTS,
            false,
        )?;
        let targets = family_string_array(
            object.get("targets"),
            &format!("{row_context}.targets"),
            MAX_FAMILY_TARGETS,
            false,
        )?;
        for target in &targets {
            if !family_targets.contains(target) {
                return Err(format!(
                    "{row_context}.targets contains {target:?}, which is absent from the family target matrix."
                ));
            }
        }
        for component_id in &component_ids {
            let component = component_map.get(component_id.as_str()).ok_or_else(|| {
                format!("{row_context}.components contains undeclared component {component_id:?}.")
            })?;
            for target in &targets {
                if !component.supports_distribution_target(target) {
                    return Err(format!(
                        "{row_context} selects target {target:?} for component {component_id:?}, but that component has no matching or portable artifact requirement."
                    ));
                }
            }
            distributed_components.insert(component_id.clone());
        }
        distributions.push(FamilyDistributionRequirement {
            channel,
            role,
            identity,
            components: component_ids,
            targets,
        });
    }

    let missing = component_map
        .keys()
        .filter(|component| !distributed_components.contains(**component))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "{context} does not distribute declared component(s): {}.",
            missing.join(", ")
        ));
    }
    Ok(distributions)
}

fn family_transform_ids(value: Option<&JsonValue>, context: &str) -> Result<Vec<String>, String> {
    let rows = value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context} must be an array."))?;
    if rows.len() > 16 {
        return Err(format!("{context} exceeds 16 transform identifiers."));
    }
    let mut result = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let value = family_text(Some(row), &format!("{context}[{index}]"))?;
        if value.len() > 128
            || !value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '/')
            })
            || !value.ends_with("/v1")
        {
            return Err(format!(
                "{context}[{index}] must be a bounded ASCII transform identifier ending in /v1."
            ));
        }
        if !seen.insert(value.clone()) {
            return Err(format!("{context} contains duplicate transform {value:?}."));
        }
        result.push(value);
    }
    Ok(result)
}

fn parse_family_public_source(
    value: Option<&JsonValue>,
    components: &[FamilyComponentRequirement],
    targets: &[String],
    distributions: &[FamilyDistributionRequirement],
) -> Result<JsonValue, String> {
    let context = "ait-release-family.json.public_source";
    let value = value.ok_or_else(|| format!("{context} must be declared."))?;
    let root = family_object(value, context)?;
    family_known_fields(
        root,
        &[
            "model",
            "identity",
            "product_document",
            "family_manifest",
            "mapping_manifest",
            "build_entrypoints",
            "subtrees",
            "transforms",
        ],
        context,
    )?;
    let model = family_identifier(root.get("model"), &format!("{context}.model"))?;
    if model != "release-monorepo" {
        return Err(format!(
            "{context}.model must be exactly \"release-monorepo\"."
        ));
    }
    let identity =
        family_distribution_identity(root.get("identity"), &format!("{context}.identity"))?;
    if identity != "weita2026/ait-native" {
        return Err(format!(
            "{context}.identity must be exactly \"weita2026/ait-native\" for the 1.0 public source authority."
        ));
    }
    for (field, expected) in [
        ("product_document", "docs/distribution.md"),
        ("family_manifest", "ait-release-family.json"),
        ("mapping_manifest", "ait-monorepo-source.json"),
    ] {
        let path = family_text(root.get(field), &format!("{context}.{field}"))?;
        let path = family_relative_path(&path, &format!("{context}.{field}"))?;
        if path != expected {
            return Err(format!("{context}.{field} must be exactly {expected:?}."));
        }
    }

    let build_context = format!("{context}.build_entrypoints");
    let build = family_object(
        root.get("build_entrypoints")
            .ok_or_else(|| format!("{build_context} must be declared."))?,
        &build_context,
    )?;
    family_known_fields(
        build,
        &["unix", "windows", "implementation"],
        &build_context,
    )?;
    for (field, expected) in [
        ("unix", "build-release.sh"),
        ("windows", "build-release.ps1"),
        ("implementation", "build-release.mjs"),
    ] {
        let path = family_text(build.get(field), &format!("{build_context}.{field}"))?;
        let path = family_relative_path(&path, &format!("{build_context}.{field}"))?;
        if path != expected {
            return Err(format!(
                "{build_context}.{field} must be exactly {expected:?}."
            ));
        }
    }

    let repositories = components
        .iter()
        .map(|component| component.source_repository.clone())
        .collect::<BTreeSet<_>>();
    let admitted_repositories = BTreeSet::from([
        "ait-core".to_string(),
        "ait-server".to_string(),
        "ait-runner".to_string(),
        "ait-python".to_string(),
        "ait-node".to_string(),
    ]);
    if repositories.is_empty() || !repositories.is_subset(&admitted_repositories) {
        return Err(format!(
            "{context} components must use the fixed ait-core, ait-server, ait-runner, ait-python, or ait-node source repositories."
        ));
    }
    let subtree_rows = root
        .get("subtrees")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context}.subtrees must be an array."))?;
    if subtree_rows.len() != repositories.len() {
        return Err(format!(
            "{context}.subtrees must map each of the five internal source repositories exactly once."
        ));
    }
    let mut subtree_transforms = BTreeMap::<String, Vec<String>>::new();
    let mut subtree_paths = BTreeSet::new();
    for (index, row) in subtree_rows.iter().enumerate() {
        let row_context = format!("{context}.subtrees[{index}]");
        let object = family_object(row, &row_context)?;
        family_known_fields(
            object,
            &["source_repository", "path", "transforms"],
            &row_context,
        )?;
        let repository = family_identifier(
            object.get("source_repository"),
            &format!("{row_context}.source_repository"),
        )?;
        if !repositories.contains(&repository) {
            return Err(format!(
                "{row_context} references undeclared source repository {repository:?}."
            ));
        }
        let path = family_text(object.get("path"), &format!("{row_context}.path"))?;
        let path = family_relative_path(&path, &format!("{row_context}.path"))?;
        if path != repository {
            return Err(format!(
                "{row_context}.path must equal its fixed source repository directory {repository:?}."
            ));
        }
        if !subtree_paths.insert(path) || subtree_transforms.contains_key(&repository) {
            return Err(format!(
                "{context}.subtrees contains a duplicate repository or public path."
            ));
        }
        subtree_transforms.insert(
            repository,
            family_transform_ids(
                object.get("transforms"),
                &format!("{row_context}.transforms"),
            )?,
        );
    }
    if subtree_transforms.keys().cloned().collect::<BTreeSet<_>>() != repositories {
        return Err(format!(
            "{context}.subtrees does not cover the exact internal repository set."
        ));
    }

    let expected_transforms = BTreeMap::from([
        (
            "runner-core-path/v1",
            (
                "ait-runner",
                "Cargo.toml",
                ".ait-external/ait-core/rust/crates/ait-core",
                "../ait-core/rust/crates/ait-core",
            ),
        ),
        (
            "python-core-path/v1",
            (
                "ait-python",
                "pyproject.toml",
                ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml",
                "../ait-core/rust/crates/ait-py/Cargo.toml",
            ),
        ),
    ]);
    let transform_rows = root
        .get("transforms")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context}.transforms must be an array."))?;
    let required_transform_count = expected_transforms
        .values()
        .filter(|(repository, _, _, _)| repositories.contains(*repository))
        .count();
    if transform_rows.len() != required_transform_count {
        return Err(format!(
            "{context}.transforms must contain exactly the allowlisted sibling-core path rewrites required by its mapped repositories."
        ));
    }
    let mut actual_transform_owners = BTreeMap::<String, String>::new();
    for (index, row) in transform_rows.iter().enumerate() {
        let row_context = format!("{context}.transforms[{index}]");
        let object = family_object(row, &row_context)?;
        family_known_fields(
            object,
            &["id", "source_repository", "path", "from", "to"],
            &row_context,
        )?;
        let id = family_transform_ids(
            Some(&json!([family_text(
                object.get("id"),
                &format!("{row_context}.id")
            )?])),
            &format!("{row_context}.id"),
        )?
        .remove(0);
        let repository = family_identifier(
            object.get("source_repository"),
            &format!("{row_context}.source_repository"),
        )?;
        let path = family_text(object.get("path"), &format!("{row_context}.path"))?;
        let path = family_relative_path(&path, &format!("{row_context}.path"))?;
        let from = family_text(object.get("from"), &format!("{row_context}.from"))?;
        let to = family_text(object.get("to"), &format!("{row_context}.to"))?;
        let expected = expected_transforms.get(id.as_str()).ok_or_else(|| {
            format!("{row_context}.id {id:?} is not an allowlisted monorepo transform.")
        })?;
        if (&repository[..], &path[..], &from[..], &to[..]) != *expected {
            return Err(format!(
                "{row_context} does not match the exact allowlisted {id:?} transformation."
            ));
        }
        if actual_transform_owners
            .insert(id.clone(), repository)
            .is_some()
        {
            return Err(format!(
                "{context}.transforms contains duplicate id {id:?}."
            ));
        }
    }
    for repository in &repositories {
        let expected_ids = actual_transform_owners
            .iter()
            .filter_map(|(id, owner)| (owner == repository).then_some(id.clone()))
            .collect::<Vec<_>>();
        if subtree_transforms.get(repository) != Some(&expected_ids) {
            return Err(format!(
                "{context}.subtrees transform references differ from the declared transforms for {repository:?}."
            ));
        }
    }

    let github_rows = distributions
        .iter()
        .filter(|distribution| distribution.channel == "github")
        .collect::<Vec<_>>();
    if github_rows.len() != 1 {
        return Err(format!(
            "{context} requires exactly one GitHub distribution for the release monorepo."
        ));
    }
    let github = github_rows[0];
    let component_ids = components
        .iter()
        .map(|component| component.id.clone())
        .collect::<BTreeSet<_>>();
    if github.identity != identity
        || github.role != "product"
        || github.components.iter().cloned().collect::<BTreeSet<_>>() != component_ids
        || github.targets.iter().cloned().collect::<BTreeSet<_>>()
            != targets.iter().cloned().collect::<BTreeSet<_>>()
    {
        return Err(
            "The sole GitHub distribution must use the public monorepo identity and cover every family component and target."
                .to_string(),
        );
    }
    Ok(value.clone())
}

fn parse_family_release_manifest(value: &JsonValue) -> Result<FamilyReleaseManifest, String> {
    let root = family_object(value, FAMILY_RELEASE_MANIFEST_PATH)?;
    family_known_fields(
        root,
        &[
            "schema",
            "family",
            "targets",
            "public_source",
            "components",
            "distributions",
            "compatibility",
        ],
        FAMILY_RELEASE_MANIFEST_PATH,
    )?;
    let schema = family_text(root.get("schema"), "ait-release-family.json.schema")?;
    if schema != FAMILY_RELEASE_MANIFEST_CONTRACT {
        return Err(format!(
            "ait-release-family.json.schema must be {FAMILY_RELEASE_MANIFEST_CONTRACT:?}, got {schema:?}."
        ));
    }
    let family_object_value = root
        .get("family")
        .ok_or_else(|| "ait-release-family.json.family must be declared.".to_string())?;
    let family_root = family_object(family_object_value, "ait-release-family.json.family")?;
    family_known_fields(
        family_root,
        &["name", "version", "channel", "tag"],
        "ait-release-family.json.family",
    )?;
    let family = FamilyIdentity {
        name: family_identifier(
            family_root.get("name"),
            "ait-release-family.json.family.name",
        )?,
        version: family_text(
            family_root.get("version"),
            "ait-release-family.json.family.version",
        )?,
        channel: family_identifier(
            family_root.get("channel"),
            "ait-release-family.json.family.channel",
        )?,
        tag: family_text(family_root.get("tag"), "ait-release-family.json.family.tag")?,
    };
    validate_family_version(&family.version, &family.channel)?;
    if family.tag != format!("v{}", family.version) {
        return Err(format!(
            "ait-release-family.json.family.tag must be exactly {:?}.",
            format!("v{}", family.version)
        ));
    }
    let targets = family_string_array(
        root.get("targets"),
        "ait-release-family.json.targets",
        MAX_FAMILY_TARGETS,
        false,
    )?;
    let target_set = targets.iter().cloned().collect::<BTreeSet<_>>();

    let component_rows = root
        .get("components")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "ait-release-family.json.components must be an array.".to_string())?;
    if component_rows.is_empty() || component_rows.len() > MAX_FAMILY_COMPONENTS {
        return Err(format!(
            "ait-release-family.json.components must contain between 1 and {MAX_FAMILY_COMPONENTS} components."
        ));
    }
    let mut components = Vec::with_capacity(component_rows.len());
    let mut component_ids = BTreeSet::new();
    for (index, row) in component_rows.iter().enumerate() {
        let context = format!("ait-release-family.json.components[{index}]");
        let object = family_object(row, &context)?;
        family_known_fields(
            object,
            &[
                "id",
                "source_repository",
                "source_snapshot",
                "ecosystem",
                "license",
                "version_scheme",
                "version",
                "artifacts",
            ],
            &context,
        )?;
        let id = family_identifier(object.get("id"), &format!("{context}.id"))?;
        if !component_ids.insert(id.clone()) {
            return Err(format!(
                "ait-release-family.json contains duplicate component id {id:?}."
            ));
        }
        let version_scheme = family_identifier(
            object.get("version_scheme"),
            &format!("{context}.version_scheme"),
        )?;
        let version = family_text(object.get("version"), &format!("{context}.version"))?;
        let expected_version = match version_scheme.as_str() {
            "family" => family.version.clone(),
            "pep440" => expected_pep440_version(&family.version, &family.channel)?,
            _ => {
                return Err(format!(
                    "{context}.version_scheme must be either \"family\" or \"pep440\", got {version_scheme:?}."
                ))
            }
        };
        if version != expected_version {
            return Err(format!(
                "{context}.version {version:?} does not match the {version_scheme} mapping {expected_version:?} for family version {:?}.",
                family.version
            ));
        }
        components.push(FamilyComponentRequirement {
            id,
            source_repository: family_identifier(
                object.get("source_repository"),
                &format!("{context}.source_repository"),
            )?,
            source_snapshot: family_snapshot_id(
                object.get("source_snapshot"),
                &format!("{context}.source_snapshot"),
            )?,
            ecosystem: family_identifier(object.get("ecosystem"), &format!("{context}.ecosystem"))?,
            license: family_spdx_expression(object.get("license"), &format!("{context}.license"))?,
            version_scheme,
            version,
            artifacts: parse_family_artifacts(
                object.get("artifacts"),
                &format!("{context}.artifacts"),
                &target_set,
            )?,
        });
    }

    let distributions =
        parse_family_distributions(root.get("distributions"), &target_set, &components)?;
    let public_source = parse_family_public_source(
        root.get("public_source"),
        &components,
        &targets,
        &distributions,
    )?;

    let compatibility_root = family_object(
        root.get("compatibility")
            .ok_or_else(|| "ait-release-family.json.compatibility must be declared.".to_string())?,
        "ait-release-family.json.compatibility",
    )?;
    if compatibility_root.len() > MAX_COMPATIBILITY_ROWS {
        return Err(format!(
            "ait-release-family.json.compatibility exceeds {MAX_COMPATIBILITY_ROWS} entries."
        ));
    }
    let mut compatibility = BTreeMap::new();
    for (key, value) in compatibility_root {
        let key_value = json!(key);
        let normalized_key = family_identifier(
            Some(&key_value),
            "ait-release-family.json.compatibility key",
        )?;
        let normalized_value = family_text(
            Some(value),
            &format!("ait-release-family.json.compatibility.{key}"),
        )?;
        compatibility.insert(normalized_key, normalized_value);
    }

    Ok(FamilyReleaseManifest {
        family,
        targets,
        public_source,
        components,
        distributions,
        compatibility,
    })
}

fn family_manifest_from_bundle(bundle: &ReleaseBundle) -> Result<FamilyReleaseManifest, String> {
    let entry = bundle
        .files
        .get(FAMILY_RELEASE_MANIFEST_PATH)
        .ok_or_else(|| {
            format!(
                "Release source Snapshot is missing required file: {FAMILY_RELEASE_MANIFEST_PATH}"
            )
        })?;
    if entry.data.len() > MAX_FAMILY_MANIFEST_BYTES {
        return Err(format!(
            "{FAMILY_RELEASE_MANIFEST_PATH} exceeds the {MAX_FAMILY_MANIFEST_BYTES}-byte contract limit."
        ));
    }
    let value = parse_slice_value(
        &entry.data,
        "ait-release-family.json must contain valid UTF-8 JSON",
    )?;
    let manifest = parse_family_release_manifest(&value)?;
    let snapshot_id = required_string_field(&bundle.raw, "snapshot_id")?;
    validate_native_product_bundle_contract(&manifest, &snapshot_id, &sha256_hex(&entry.data))?;
    Ok(manifest)
}

fn family_manifest_sha256(bundle: &ReleaseBundle) -> Result<String, String> {
    bundle
        .files
        .get(FAMILY_RELEASE_MANIFEST_PATH)
        .map(|entry| sha256_hex(&entry.data))
        .ok_or_else(|| {
            format!(
                "Release source Snapshot is missing required file: {FAMILY_RELEASE_MANIFEST_PATH}"
            )
        })
}

fn family_line_bundle(repo: &RepoRuntime, line_name: &str) -> Result<ReleaseBundle, String> {
    let line = release_local_line_row(repo, line_name)?;
    let snapshot_id = string_field(&line, "head_snapshot_id")
        .ok_or_else(|| format!("Line {line_name} does not have a head Snapshot yet."))?;
    release_snapshot_bundle(repo, &snapshot_id)
}

pub fn family_manifest_exists(repo: &RepoRuntime, line_name: &str) -> Result<bool, String> {
    Ok(family_line_bundle(repo, line_name)?
        .files
        .contains_key(FAMILY_RELEASE_MANIFEST_PATH))
}

fn family_release_id(
    repo: &RepoRuntime,
    line_name: &str,
    snapshot_id: &str,
    manifest_sha256: &str,
    family: &FamilyIdentity,
) -> String {
    let identity = format!(
        "{}\0{}\0{}\0{}\0{}\0{}",
        repo.repo_name(),
        line_name,
        snapshot_id,
        manifest_sha256,
        family.version,
        family.channel
    );
    format!(
        "REL-FAM-{}",
        sha256_hex(identity.as_bytes())[..16].to_ascii_uppercase()
    )
}

fn family_release_relative_dir(release_id: &str) -> String {
    format!("dist/{release_id}")
}

fn family_candidate_record(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    requested_channel: Option<&str>,
) -> Result<JsonValue, String> {
    let bundle = family_line_bundle(repo, line_name)?;
    let manifest = family_manifest_from_bundle(&bundle)?;
    if version.trim() != manifest.family.version {
        return Err(format!(
            "Requested family release version {version:?} does not match {FAMILY_RELEASE_MANIFEST_PATH} version {:?}.",
            manifest.family.version
        ));
    }
    if let Some(channel) = normalized_text(requested_channel) {
        if channel != manifest.family.channel {
            return Err(format!(
                "Requested release channel {channel:?} does not match {FAMILY_RELEASE_MANIFEST_PATH} channel {:?}.",
                manifest.family.channel
            ));
        }
    }
    release_require_external_readiness(repo)?;
    let snapshot_id = required_string_field(&bundle.raw, "snapshot_id")?;
    let source_manifest_hash = required_string_field(&bundle.raw, "manifest_hash")?;
    let manifest_sha256 = family_manifest_sha256(&bundle)?;
    let release_id = family_release_id(
        repo,
        line_name.trim(),
        &snapshot_id,
        &manifest_sha256,
        &manifest.family,
    );
    let created_at = bundle
        .raw
        .get("created_at")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .unwrap_or_else(current_timestamp);
    let release_dir = family_release_relative_dir(&release_id);
    Ok(json!({
        "contract": FAMILY_RELEASE_CANDIDATE_CONTRACT,
        "command": "release candidate create",
        "release_id": release_id,
        "repo_name": repo.repo_name(),
        "version": manifest.family.version,
        "channel": manifest.family.channel,
        "tag": manifest.family.tag,
        "line": line_name.trim(),
        "line_name": line_name.trim(),
        "snapshot_id": snapshot_id,
        "manifest_hash": source_manifest_hash,
        "profile": FAMILY_RELEASE_PROFILE,
        "status": "candidate",
        "family_manifest_path": FAMILY_RELEASE_MANIFEST_PATH,
        "family_manifest_sha256": manifest_sha256,
        "family": manifest.to_json(),
        "component_count": manifest.components.len(),
        "target_count": manifest.targets.len(),
        "expected_artifact_count": manifest.expected_artifact_count(),
        "checks": [],
        "artifacts": [],
        "dossier_path": format!("{release_dir}/{FAMILY_CANDIDATE_FILENAME}"),
        "authority": {
            "source": "selected_snapshot",
            "persistence": "portable_dist_dossier",
            "local_release_authority": "not_activated",
            "remote_release_authority": "not_activated",
            "binary_db_layout_changed": false,
        },
        "created_at": created_at,
        "updated_at": created_at,
        "next_action": {
            "code": "check_family_receipts",
            "detail": format!("Run `ait release check {release_id} --receipts <dir>` with one admitted component receipt bundle per source repository."),
        },
    }))
}

fn family_candidate_record_from_public_source(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    requested_channel: Option<&str>,
    public_source_root: &Path,
) -> Result<JsonValue, String> {
    if repo.repo_name() != "ait-core" {
        return Err(
            "Public Git family coordination must run from the ait-core subtree.".to_string(),
        );
    }
    if line_name.trim() != "main" {
        return Err("Public Git family coordination requires exact Line \"main\".".to_string());
    }
    let (authority, manifest) = public_git_source_authority(repo, Some(public_source_root))?
        .ok_or_else(|| "Public Git source mapping is unavailable.".to_string())?;
    if version.trim() != manifest.family.version {
        return Err(format!(
            "Requested family release version {version:?} does not match {FAMILY_RELEASE_MANIFEST_PATH} version {:?}.",
            manifest.family.version
        ));
    }
    if let Some(channel) = normalized_text(requested_channel) {
        if channel != manifest.family.channel {
            return Err(format!(
                "Requested release channel {channel:?} does not match {FAMILY_RELEASE_MANIFEST_PATH} channel {:?}.",
                manifest.family.channel
            ));
        }
    }
    let family_path = authority.root.join(FAMILY_RELEASE_MANIFEST_PATH);
    let family_bytes = read_bounded_file(
        &family_path,
        MAX_FAMILY_MANIFEST_BYTES,
        "Public Git family manifest",
    )?;
    let manifest_sha256 = sha256_hex(&family_bytes);
    let release_id = family_release_id(
        repo,
        line_name.trim(),
        &authority.coordinator_snapshot,
        &manifest_sha256,
        &manifest.family,
    );
    let release_dir = family_release_relative_dir(&release_id);
    Ok(json!({
        "contract": FAMILY_RELEASE_CANDIDATE_CONTRACT,
        "command": "release candidate create",
        "release_id": release_id,
        "repo_name": repo.repo_name(),
        "version": manifest.family.version,
        "channel": manifest.family.channel,
        "tag": manifest.family.tag,
        "line": line_name.trim(),
        "line_name": line_name.trim(),
        "snapshot_id": authority.coordinator_snapshot,
        "manifest_hash": authority.coordinator_manifest_hash,
        "profile": FAMILY_RELEASE_PROFILE,
        "status": "candidate",
        "family_manifest_path": FAMILY_RELEASE_MANIFEST_PATH,
        "family_manifest_sha256": manifest_sha256,
        "family": manifest.to_json(),
        "component_count": manifest.components.len(),
        "target_count": manifest.targets.len(),
        "expected_artifact_count": manifest.expected_artifact_count(),
        "checks": [],
        "artifacts": [],
        "dossier_path": format!("{release_dir}/{FAMILY_CANDIDATE_FILENAME}"),
        "authority": {
            "source": "selected_snapshot",
            "persistence": "portable_dist_dossier",
            "local_release_authority": "not_activated",
            "remote_release_authority": "not_activated",
            "binary_db_layout_changed": false,
        },
        "created_at": authority.coordinator_created_at,
        "updated_at": authority.coordinator_created_at,
        "next_action": {
            "code": "check_family_receipts",
            "detail": format!("Run `ait release check {release_id} --receipts <dir>` with one admitted component receipt bundle per source repository."),
        },
    }))
}

fn validate_release_id(release_id: &str) -> Result<(), String> {
    let value = json!(release_id);
    let normalized = family_identifier(Some(&value), "release_id")?;
    let suffix = normalized.strip_prefix("REL-FAM-").unwrap_or_default();
    if suffix.len() != 16 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "Family release identity must use REL-FAM- followed by 16 hexadecimal characters, got {release_id:?}."
        ));
    }
    Ok(())
}

fn ensure_real_directory(path: &Path, context: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(format!(
            "{context} must be a real directory: {}.",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(io_error)?;
            let metadata = fs::symlink_metadata(path).map_err(io_error)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "{context} must be a real directory: {}.",
                    path.display()
                ));
            }
            Ok(())
        }
        Err(error) => Err(io_error(error)),
    }
}

fn family_release_dir(
    repo: &RepoRuntime,
    release_id: &str,
    create: bool,
) -> Result<PathBuf, String> {
    validate_release_id(release_id)?;
    let dist = repo.workspace_root().join("dist");
    if create {
        ensure_real_directory(&dist, "Family release dist root")?;
    } else {
        let metadata = fs::symlink_metadata(&dist)
            .map_err(|error| format!("Family release dist root is unavailable: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Family release dist root must be a real directory.".to_string());
        }
    }
    let release_dir = dist.join(release_id);
    if create {
        ensure_real_directory(&release_dir, "Family release projection")?;
    } else {
        let metadata = fs::symlink_metadata(&release_dir).map_err(|error| {
            format!("Family release projection {release_id} is unavailable: {error}")
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "Family release projection {release_id} must be a real directory."
            ));
        }
    }
    let canonical_dist = dist.canonicalize().map_err(io_error)?;
    let canonical_release = release_dir.canonicalize().map_err(io_error)?;
    if !canonical_release.starts_with(&canonical_dist) {
        return Err(format!(
            "Family release projection {release_id} escapes the canonical dist root."
        ));
    }
    Ok(release_dir)
}

fn read_bounded_file(path: &Path, max_bytes: usize, context: &str) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("{context} is unavailable at {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{context} must be a real regular file: {}.",
            path.display()
        ));
    }
    if metadata.len() > max_bytes as u64 {
        return Err(format!(
            "{context} exceeds the {max_bytes}-byte limit: {}.",
            path.display()
        ));
    }
    let mut file = File::open(path).map_err(io_error)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes).map_err(io_error)?;
    Ok(bytes)
}

fn hash_regular_file(path: &Path, context: &str) -> Result<(u64, String), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("{context} is unavailable at {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{context} must be a real regular file: {}.",
            path.display()
        ));
    }
    let mut file = File::open(path).map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| format!("{context} byte count overflowed."))?;
        digest.update(&buffer[..read]);
    }
    if total != metadata.len() {
        return Err(format!(
            "{context} changed size while it was being verified: {}.",
            path.display()
        ));
    }
    Ok((total, format!("{:x}", digest.finalize())))
}

fn public_git_source_authority(
    repo: &RepoRuntime,
    requested_root: Option<&Path>,
) -> Result<Option<(PublicGitSourceAuthority, FamilyReleaseManifest)>, String> {
    let explicit_root = requested_root.is_some();
    let candidate_root = match requested_root {
        Some(root) => root.to_path_buf(),
        None => {
            let workspace_root = repo.workspace_root();
            let Some(parent) = workspace_root.parent() else {
                return Ok(None);
            };
            let parent = parent.to_path_buf();
            if !parent.join(PUBLIC_SOURCE_MAPPING_FILENAME).is_file() {
                return Ok(None);
            }
            parent
        }
    };
    let root_metadata = fs::symlink_metadata(&candidate_root).map_err(|error| {
        format!(
            "Public Git source root is unavailable at {}: {error}",
            candidate_root.display()
        )
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(format!(
            "Public Git source root must be a real directory: {}.",
            candidate_root.display()
        ));
    }
    let root = candidate_root.canonicalize().map_err(io_error)?;
    let workspace = repo.workspace_root().canonicalize().map_err(io_error)?;
    let public_core = root.join("ait-core");
    let public_core_metadata = fs::symlink_metadata(&public_core).map_err(|error| {
        format!(
            "Public Git source root does not contain ait-core at {}: {error}",
            public_core.display()
        )
    })?;
    if public_core_metadata.file_type().is_symlink() || !public_core_metadata.is_dir() {
        return Err(format!(
            "Public Git source ait-core subtree must be a real directory: {}.",
            public_core.display()
        ));
    }
    let expected_workspace = public_core.canonicalize().map_err(io_error)?;
    if !explicit_root && workspace != expected_workspace {
        return Err(format!(
            "Public Git source root must contain the active ait-core repository at {}.",
            expected_workspace.display()
        ));
    }

    let mapping_path = root.join(PUBLIC_SOURCE_MAPPING_FILENAME);
    let mapping_bytes = read_bounded_file(
        &mapping_path,
        MAX_DOSSIER_BYTES,
        "Public Git source mapping",
    )?;
    let mapping = parse_slice_value(
        &mapping_bytes,
        "Public Git source mapping must contain valid JSON",
    )?;
    let mapping_object = family_object(&mapping, "ait-monorepo-source.json")?;
    family_known_fields(
        mapping_object,
        &[
            "schema",
            "public_source_identity",
            "coordinator_snapshot",
            "coordinator_manifest_hash",
            "coordinator_created_at",
            "family_version",
            "family_tag",
            "family_manifest_sha256",
            "product_document_sha256",
            "content_digest_contract",
            "content_sha256",
            "subtrees",
            "excluded_operational_roots",
            "git_commit_created",
            "public_publish",
        ],
        "ait-monorepo-source.json",
    )?;
    if string_field(&mapping, "schema").as_deref() != Some(PUBLIC_SOURCE_MAPPING_CONTRACT)
        || string_field(&mapping, "public_source_identity").as_deref()
            != Some(PUBLIC_SOURCE_IDENTITY)
        || mapping
            .get("git_commit_created")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || mapping.get("public_publish").and_then(JsonValue::as_bool) != Some(false)
        || string_field(&mapping, "content_digest_contract").as_deref()
            != Some("size-sha256-path/v1; excludes ait-monorepo-source.json")
    {
        return Err(
            "Public Git source mapping has an invalid identity or publication boundary."
                .to_string(),
        );
    }
    let content_sha256 = required_string_field(&mapping, "content_sha256")?;
    let family_manifest_sha256 = required_string_field(&mapping, "family_manifest_sha256")?;
    let product_document_sha256 = required_string_field(&mapping, "product_document_sha256")?;
    if !valid_sha256_lower(&content_sha256)
        || !valid_sha256_lower(&family_manifest_sha256)
        || !valid_sha256_lower(&product_document_sha256)
    {
        return Err("Public Git source mapping contains an invalid lowercase SHA-256.".to_string());
    }

    let family_path = root.join(FAMILY_RELEASE_MANIFEST_PATH);
    let family_bytes = read_bounded_file(
        &family_path,
        MAX_FAMILY_MANIFEST_BYTES,
        "Public Git family manifest",
    )?;
    if sha256_hex(&family_bytes) != family_manifest_sha256 {
        return Err(
            "Public Git family manifest differs from ait-monorepo-source.json.".to_string(),
        );
    }
    let family_value = parse_slice_value(
        &family_bytes,
        "Public Git family manifest must contain valid JSON",
    )?;
    let family = parse_family_release_manifest(&family_value)?;
    validate_native_product_bundle_contract(
        &family,
        &required_string_field(&mapping, "coordinator_snapshot")?,
        &family_manifest_sha256,
    )?;
    if string_field(&mapping, "family_version").as_deref() != Some(family.family.version.as_str())
        || string_field(&mapping, "family_tag").as_deref() != Some(family.family.tag.as_str())
    {
        return Err(
            "Public Git source mapping version or tag differs from the family manifest."
                .to_string(),
        );
    }

    let expected_by_repository = family.components.iter().fold(
        BTreeMap::<String, (String, String, BTreeSet<String>)>::new(),
        |mut rows, component| {
            let entry = rows
                .entry(component.source_repository.clone())
                .or_insert_with(|| {
                    (
                        component.source_snapshot.clone(),
                        component.license.clone(),
                        BTreeSet::new(),
                    )
                });
            entry.2.insert(component.id.clone());
            rows
        },
    );
    let declared_transforms = family
        .public_source
        .get("subtrees")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some((
                required_string_field(row, "source_repository").ok()?,
                row.get("transforms")
                    .and_then(JsonValue::as_array)?
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let subtree_rows = mapping
        .get("subtrees")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Public Git source mapping subtrees must be an array.".to_string())?;
    if subtree_rows.len() != expected_by_repository.len() {
        return Err(
            "Public Git source mapping does not cover the exact family repository set.".to_string(),
        );
    }
    let mut subtrees = BTreeMap::new();
    for (index, row) in subtree_rows.iter().enumerate() {
        let context = format!("ait-monorepo-source.json.subtrees[{index}]");
        let object = family_object(row, &context)?;
        family_known_fields(
            object,
            &[
                "source_repository",
                "source_snapshot",
                "source_manifest_hash",
                "source_snapshot_created_at",
                "path",
                "license",
                "components",
                "transforms",
                "source_cache_evidence_sha256",
                "source_content_sha256",
                "exported_content_sha256",
            ],
            &context,
        )?;
        let repository = family_identifier(
            row.get("source_repository"),
            &format!("{context}.source_repository"),
        )?;
        let (expected_snapshot, expected_license, expected_components) = expected_by_repository
            .get(&repository)
            .ok_or_else(|| format!("{context} names undeclared repository {repository:?}."))?;
        let source_snapshot = family_snapshot_id(
            row.get("source_snapshot"),
            &format!("{context}.source_snapshot"),
        )?;
        let source_manifest_hash = required_string_field(row, "source_manifest_hash")?;
        let source_snapshot_created_at = required_string_field(row, "source_snapshot_created_at")?;
        let path = family_relative_path(
            &required_string_field(row, "path")?,
            &format!("{context}.path"),
        )?;
        let license = family_identifier(row.get("license"), &format!("{context}.license"))?;
        let components = family_string_array(
            row.get("components"),
            &format!("{context}.components"),
            MAX_FAMILY_COMPONENTS,
            false,
        )?
        .into_iter()
        .collect::<BTreeSet<_>>();
        let transforms =
            family_transform_ids(row.get("transforms"), &format!("{context}.transforms"))?
                .into_iter()
                .collect::<BTreeSet<_>>();
        let source_cache_evidence_sha256 =
            required_string_field(row, "source_cache_evidence_sha256")?;
        let source_content_sha256 = required_string_field(row, "source_content_sha256")?;
        let exported_content_sha256 = required_string_field(row, "exported_content_sha256")?;
        if path != repository
            || &source_snapshot != expected_snapshot
            || &license != expected_license
            || &components != expected_components
            || declared_transforms.get(&repository) != Some(&transforms)
            || !valid_sha256_lower(&source_manifest_hash)
            || !valid_u64_decimal(&source_snapshot_created_at)
            || !valid_sha256_lower(&source_cache_evidence_sha256)
            || !valid_sha256_lower(&source_content_sha256)
            || !valid_sha256_lower(&exported_content_sha256)
        {
            return Err(format!(
                "{context} differs from its family or digest authority."
            ));
        }
        if subtrees
            .insert(
                repository,
                PublicGitSubtreeAuthority {
                    source_snapshot,
                    source_manifest_hash,
                    source_snapshot_created_at,
                    path,
                    exported_content_sha256,
                },
            )
            .is_some()
        {
            return Err("Public Git source mapping contains a duplicate subtree.".to_string());
        }
    }
    if subtrees.keys().collect::<BTreeSet<_>>()
        != expected_by_repository.keys().collect::<BTreeSet<_>>()
    {
        return Err(
            "Public Git source mapping does not cover the exact family repository set.".to_string(),
        );
    }

    let coordinator_snapshot = family_snapshot_id(
        mapping.get("coordinator_snapshot"),
        "ait-monorepo-source.json.coordinator_snapshot",
    )?;
    let coordinator_manifest_hash = required_string_field(&mapping, "coordinator_manifest_hash")?;
    let coordinator_created_at = required_string_field(&mapping, "coordinator_created_at")?;
    let _core = subtrees.get("ait-core").ok_or_else(|| {
        "Public Git source mapping must contain the ait-core coordinator subtree.".to_string()
    })?;
    if !valid_sha256_lower(&coordinator_manifest_hash)
        || !valid_u64_decimal(&coordinator_created_at)
    {
        return Err("Public Git coordinator evidence is invalid.".to_string());
    }

    Ok(Some((
        PublicGitSourceAuthority {
            root,
            mapping_sha256: sha256_hex(&mapping_bytes),
            content_sha256,
            coordinator_snapshot,
            coordinator_manifest_hash,
            coordinator_created_at,
            subtrees,
        },
        family,
    )))
}

fn write_json_once(path: &Path, payload: &JsonValue, context: &str) -> Result<(), String> {
    let bytes = encode_value_pretty_with_newline_error_string(payload)?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let existing = read_bounded_file(path, MAX_DOSSIER_BYTES, context)?;
            if existing.as_slice() == bytes.as_bytes() {
                return Ok(());
            }
            return Err(format!(
                "{context} already exists with different bytes: {}.",
                path.display()
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{context} path has no parent directory."))?;
    ensure_real_directory(parent, context)?;
    let mut staged = NamedTempFile::new_in(parent).map_err(io_error)?;
    staged.write_all(bytes.as_bytes()).map_err(io_error)?;
    staged.flush().map_err(io_error)?;
    staged.as_file().sync_all().map_err(io_error)?;
    match staged.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = read_bounded_file(path, MAX_DOSSIER_BYTES, context)?;
            if existing.as_slice() == bytes.as_bytes() {
                Ok(())
            } else {
                Err(format!(
                    "{context} was concurrently created with different bytes: {}.",
                    path.display()
                ))
            }
        }
        Err(error) => Err(io_error(error.error)),
    }
}

fn candidate_path(repo: &RepoRuntime, release_id: &str, create: bool) -> Result<PathBuf, String> {
    Ok(family_release_dir(repo, release_id, create)?.join(FAMILY_CANDIDATE_FILENAME))
}

pub fn family_candidate_exists(repo: &RepoRuntime, release_id: &str) -> bool {
    candidate_path(repo, release_id, false)
        .ok()
        .and_then(|path| fs::symlink_metadata(path).ok())
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn load_family_candidate(
    repo: &RepoRuntime,
    release_id: &str,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    let path = candidate_path(repo, release_id, false)?;
    let bytes = read_bounded_file(&path, MAX_DOSSIER_BYTES, "Family release candidate dossier")?;
    let candidate = parse_slice_value(
        &bytes,
        "Family release candidate dossier must be valid JSON",
    )?;
    if string_field(&candidate, "contract").as_deref() != Some(FAMILY_RELEASE_CANDIDATE_CONTRACT)
        || string_field(&candidate, "release_id").as_deref() != Some(release_id)
        || string_field(&candidate, "profile").as_deref() != Some(FAMILY_RELEASE_PROFILE)
    {
        return Err(format!(
            "Family release candidate dossier {release_id} has an invalid contract or identity."
        ));
    }
    let version = required_string_field(&candidate, "version")?;
    let line = required_string_field(&candidate, "line")?;
    let channel = required_string_field(&candidate, "channel")?;
    let public_source = public_git_source_authority(repo, public_source_root)?;
    let expected = if let Some((authority, _)) = public_source {
        family_candidate_record_from_public_source(
            repo,
            &version,
            &line,
            Some(&channel),
            &authority.root,
        )?
    } else {
        family_candidate_record(repo, &version, &line, Some(&channel))?
    };
    if candidate != expected {
        return Err(format!(
            "Family release candidate dossier {release_id} does not match its immutable Snapshot manifest."
        ));
    }
    Ok(candidate)
}

pub fn family_release_candidate_create(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    channel: Option<&str>,
) -> Result<JsonValue, String> {
    let candidate = family_candidate_record(repo, version, line_name, channel)?;
    let release_id = required_string_field(&candidate, "release_id")?;
    let path = candidate_path(repo, &release_id, true)?;
    write_json_once(&path, &candidate, "Family release candidate dossier")?;
    Ok(candidate)
}

pub fn family_release_candidate_create_from_public_source(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    channel: Option<&str>,
    public_source_root: &Path,
) -> Result<JsonValue, String> {
    let candidate = family_candidate_record_from_public_source(
        repo,
        version,
        line_name,
        channel,
        public_source_root,
    )?;
    let release_id = required_string_field(&candidate, "release_id")?;
    let path = candidate_path(repo, &release_id, true)?;
    write_json_once(&path, &candidate, "Family release candidate dossier")?;
    Ok(candidate)
}

fn valid_sha256_lower(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_u64_decimal(value: &str) -> bool {
    !value.is_empty()
        && (value == "0"
            || (!value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit())))
        && value.parse::<u64>().is_ok()
}

fn walk_receipt_tree(
    directory: &Path,
    depth: usize,
    entry_count: &mut usize,
    receipts: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if depth > MAX_RECEIPT_TREE_DEPTH {
        return Err(format!(
            "Component receipt tree exceeds maximum depth {MAX_RECEIPT_TREE_DEPTH}: {}.",
            directory.display()
        ));
    }
    let mut entries = fs::read_dir(directory)
        .map_err(io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        *entry_count += 1;
        if *entry_count > MAX_RECEIPT_TREE_ENTRIES {
            return Err(format!(
                "Component receipt tree exceeds {MAX_RECEIPT_TREE_ENTRIES} entries."
            ));
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Component receipt tree contains a symbolic link: {}.",
                path.display()
            ));
        }
        if metadata.is_dir() {
            walk_receipt_tree(&path, depth + 1, entry_count, receipts)?;
        } else if metadata.is_file()
            && path.file_name().and_then(OsStr::to_str) == Some(COMPONENT_RECEIPT_FILENAME)
        {
            receipts.push(path);
            if receipts.len() > MAX_RECEIPTS {
                return Err(format!(
                    "Component receipt tree exceeds {MAX_RECEIPTS} receipt files."
                ));
            }
        }
    }
    Ok(())
}

fn component_receipt_paths(root: &Path) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("Component receipt directory is unavailable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Component receipt path must be a real directory.".to_string());
    }
    let canonical_root = root.canonicalize().map_err(io_error)?;
    let mut receipts = Vec::new();
    let mut entry_count = 0;
    walk_receipt_tree(&canonical_root, 0, &mut entry_count, &mut receipts)?;
    receipts.sort();
    if receipts.is_empty() {
        return Err(format!(
            "Component receipt directory contains no {COMPONENT_RECEIPT_FILENAME} files."
        ));
    }
    Ok((canonical_root, receipts))
}

fn receipt_relative_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        format!(
            "Component receipt path escapes the canonical receipt root: {}.",
            path.display()
        )
    })?;
    let text = relative.to_str().ok_or_else(|| {
        format!(
            "Component receipt path is not portable UTF-8: {}.",
            path.display()
        )
    })?;
    family_relative_path(&text.replace('\\', "/"), "Component receipt path")
}

fn component_definition_ecosystem(receipt: &JsonValue, component_id: &str) -> Option<String> {
    receipt
        .get("metadata")
        .and_then(|metadata| metadata.get("release_adapter"))
        .and_then(|adapter| adapter.get("definition"))
        .and_then(|definition| definition.get("components"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .find(|component| string_field(component, "id").as_deref() == Some(component_id))
        .and_then(|component| string_field(component, "ecosystem"))
}

fn component_definition_artifact_keys(
    receipt: &JsonValue,
    component_id: &str,
) -> Result<BTreeSet<(String, Option<String>)>, String> {
    let component = receipt
        .get("metadata")
        .and_then(|metadata| metadata.get("release_adapter"))
        .and_then(|adapter| adapter.get("definition"))
        .and_then(|definition| definition.get("components"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .find(|component| string_field(component, "id").as_deref() == Some(component_id))
        .ok_or_else(|| {
            format!(
                "Component {component_id} is missing from its receipt-bound adapter definition."
            )
        })?;
    let artifacts = component
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            format!(
                "Component {component_id} receipt-bound adapter definition is missing artifacts."
            )
        })?;
    let mut keys = BTreeSet::new();
    for (index, artifact) in artifacts.iter().enumerate() {
        let kind = family_identifier(
            artifact.get("kind"),
            &format!("Component {component_id} definition artifact {index} kind"),
        )?;
        let target = match artifact.get("target") {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(family_identifier(
                Some(value),
                &format!("Component {component_id} definition artifact {index} target"),
            )?),
        };
        if !keys.insert((kind, target)) {
            return Err(format!(
                "Component {component_id} receipt-bound adapter definition contains duplicate artifact keys."
            ));
        }
    }
    Ok(keys)
}

fn valid_git_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_public_git_receipt_authority(
    receipt: &JsonValue,
    receipt_repo: &str,
    authority: &PublicGitSourceAuthority,
) -> Result<String, String> {
    let source = receipt
        .get("authority")
        .ok_or_else(|| "Public Git receipt is missing its authority object.".to_string())?;
    let object = family_object(source, "Public Git receipt authority")?;
    family_known_fields(
        object,
        &[
            "source",
            "public_source_identity",
            "git_commit",
            "coordinator_snapshot",
            "source_snapshot",
            "source_manifest_hash",
            "source_mapping_path",
            "source_mapping_sha256",
            "source_content_sha256",
            "subtree_path",
            "subtree_exported_content_sha256",
            "persistence",
            "local_release_authority",
            "remote_publish_supported",
        ],
        "Public Git receipt authority",
    )?;
    let subtree = authority
        .subtrees
        .get(receipt_repo)
        .ok_or_else(|| format!("Public Git receipt names unmapped repository {receipt_repo:?}."))?;
    let git_commit = required_string_field(source, "git_commit")?;
    if !valid_git_commit(&git_commit)
        || string_field(source, "source").as_deref() != Some("public_git_commit")
        || string_field(source, "public_source_identity").as_deref() != Some(PUBLIC_SOURCE_IDENTITY)
        || string_field(source, "coordinator_snapshot").as_deref()
            != Some(authority.coordinator_snapshot.as_str())
        || string_field(source, "source_snapshot").as_deref()
            != Some(subtree.source_snapshot.as_str())
        || string_field(source, "source_manifest_hash").as_deref()
            != Some(subtree.source_manifest_hash.as_str())
        || string_field(receipt, "manifest_hash").as_deref()
            != Some(subtree.source_manifest_hash.as_str())
        || string_field(receipt, "created_at").as_deref()
            != Some(subtree.source_snapshot_created_at.as_str())
        || string_field(receipt, "updated_at").as_deref()
            != Some(subtree.source_snapshot_created_at.as_str())
        || receipt
            .get("metadata")
            .and_then(|metadata| string_field(metadata, "source_snapshot_created_at"))
            .as_deref()
            != Some(subtree.source_snapshot_created_at.as_str())
        || receipt
            .get("metadata")
            .and_then(|metadata| metadata.get("build"))
            .and_then(|build| string_field(build, "built_at"))
            .as_deref()
            != Some(subtree.source_snapshot_created_at.as_str())
        || receipt
            .get("metadata")
            .and_then(|metadata| metadata.get("build"))
            .and_then(|build| string_field(build, "source_date_epoch"))
            .as_deref()
            != Some(subtree.source_snapshot_created_at.as_str())
        || string_field(source, "source_mapping_path").as_deref()
            != Some(PUBLIC_SOURCE_MAPPING_FILENAME)
        || string_field(source, "source_mapping_sha256").as_deref()
            != Some(authority.mapping_sha256.as_str())
        || string_field(source, "source_content_sha256").as_deref()
            != Some(authority.content_sha256.as_str())
        || string_field(source, "subtree_path").as_deref() != Some(subtree.path.as_str())
        || string_field(source, "subtree_exported_content_sha256").as_deref()
            != Some(subtree.exported_content_sha256.as_str())
        || string_field(source, "persistence").as_deref() != Some("ci_artifact_bundle")
        || string_field(source, "local_release_authority").as_deref() != Some("not_activated")
        || source
            .get("remote_publish_supported")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || receipt.get("public_publish").and_then(JsonValue::as_bool) != Some(false)
        || receipt.get("publishable").and_then(JsonValue::as_bool) != Some(false)
    {
        return Err(format!(
            "Public Git receipt authority for {receipt_repo:?} differs from ait-monorepo-source.json."
        ));
    }
    Ok(git_commit)
}

fn inspect_family_receipts(
    repo: &RepoRuntime,
    candidate: &JsonValue,
    receipts_root: &Path,
    public_source_root: Option<&Path>,
) -> Result<FamilyAdmission, String> {
    let family_definition = candidate
        .get("family")
        .ok_or_else(|| "Family candidate is missing its manifest definition.".to_string())?;
    let manifest = parse_family_release_manifest(family_definition)?;
    let public_authority =
        public_git_source_authority(repo, public_source_root)?.map(|(authority, _)| authority);
    let expected_components = manifest
        .components
        .iter()
        .map(|component| (component.id.clone(), component))
        .collect::<BTreeMap<_, _>>();
    let mut expected_repositories = BTreeMap::<String, String>::new();
    for component in &manifest.components {
        match expected_repositories.get(&component.source_repository) {
            Some(snapshot) if snapshot != &component.source_snapshot => {
                return Err(format!(
                    "Family source repository {:?} is bound to conflicting Snapshots {:?} and {:?}; repository license material requires one exact source identity.",
                    component.source_repository, snapshot, component.source_snapshot
                ));
            }
            Some(_) => {}
            None => {
                expected_repositories.insert(
                    component.source_repository.clone(),
                    component.source_snapshot.clone(),
                );
            }
        }
    }
    let (canonical_root, receipt_paths) = component_receipt_paths(receipts_root)?;
    let mut actual_keys = BTreeMap::<String, BTreeSet<(String, Option<String>)>>::new();
    let mut component_receipts = BTreeMap::<String, BTreeSet<String>>::new();
    let mut sources = Vec::new();
    let mut license_sources = BTreeMap::<(String, String), FamilyLicenseMaterialSource>::new();
    let mut receipt_evidence = Vec::new();
    let mut public_git_commit: Option<String> = None;
    let mut receipt_contract: Option<String> = None;

    for receipt_path in receipt_paths {
        let receipt_bytes = read_bounded_file(
            &receipt_path,
            MAX_COMPONENT_RECEIPT_BYTES,
            "Component release receipt",
        )?;
        let receipt = parse_slice_value(
            &receipt_bytes,
            "Component release receipt must contain valid JSON",
        )?;
        let contract = required_string_field(&receipt, "contract")?;
        if !matches!(
            contract.as_str(),
            GENERIC_RELEASE_RECEIPT_CONTRACT | PUBLIC_GIT_RELEASE_RECEIPT_CONTRACT
        ) {
            return Err(format!(
                "Component receipt {} uses unsupported contract {contract:?}.",
                receipt_path.display()
            ));
        }
        if let Some(expected_contract) = &receipt_contract {
            if expected_contract != &contract {
                return Err(
                    "Component receipt directory mixes selected-Snapshot and public-Git authority contracts."
                        .to_string(),
                );
            }
        } else {
            receipt_contract = Some(contract.clone());
        }
        if string_field(&receipt, "status").as_deref() != Some("built")
            || receipt
                .get("check_summary")
                .and_then(|summary| summary.get("decision"))
                .and_then(JsonValue::as_str)
                != Some("pass")
        {
            return Err(format!(
                "Component receipt {} is not a passing built receipt.",
                receipt_path.display()
            ));
        }
        let receipt_relative = receipt_relative_path(&canonical_root, &receipt_path)?;
        let receipt_repo = required_string_field(&receipt, "repo_name")?;
        let receipt_snapshot = required_string_field(&receipt, "snapshot_id")?;
        let receipt_version = required_string_field(&receipt, "version")?;
        let receipt_git_commit = match (&public_authority, contract.as_str()) {
            (Some(authority), PUBLIC_GIT_RELEASE_RECEIPT_CONTRACT) => {
                let commit =
                    validate_public_git_receipt_authority(&receipt, &receipt_repo, authority)?;
                if let Some(expected) = &public_git_commit {
                    if expected != &commit {
                        return Err(
                            "Public Git component receipts were built from different commits."
                                .to_string(),
                        );
                    }
                } else {
                    public_git_commit = Some(commit.clone());
                }
                Some(commit)
            }
            (Some(_), GENERIC_RELEASE_RECEIPT_CONTRACT) => {
                return Err(
                    "Public monorepo family admission requires public-Git component receipts."
                        .to_string(),
                )
            }
            (None, PUBLIC_GIT_RELEASE_RECEIPT_CONTRACT) => {
                return Err(
                    "Public-Git component receipts require an adjacent ait-monorepo-source.json authority."
                        .to_string(),
                )
            }
            (None, GENERIC_RELEASE_RECEIPT_CONTRACT) => None,
            _ => unreachable!(),
        };
        let expected_repository_snapshot = expected_repositories.get(&receipt_repo).ok_or_else(|| {
            format!(
                "Component receipt {receipt_relative} names undeclared family source repository {receipt_repo:?}."
            )
        })?;
        if &receipt_snapshot != expected_repository_snapshot {
            return Err(format!(
                "Component receipt {receipt_relative} source Snapshot {receipt_snapshot:?} does not match repository {receipt_repo:?} family authority {expected_repository_snapshot:?}."
            ));
        }
        let receipt_target = match receipt.get("target") {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(family_identifier(
                Some(value),
                &format!("Component receipt {receipt_relative} target"),
            )?),
        };
        let receipt_parent = receipt_path
            .parent()
            .ok_or_else(|| "Component receipt path has no parent directory.".to_string())?;
        let canonical_parent = receipt_parent.canonicalize().map_err(io_error)?;
        let mut receipt_component_ids = BTreeSet::new();
        let artifacts = receipt
            .get("artifacts")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| format!("Component receipt {receipt_relative} is missing artifacts."))?;
        assert_generic_release_artifacts_complete(&receipt, artifacts).map_err(|error| {
            format!(
                "Component receipt {receipt_relative} fails its bound adapter evidence contract: {error}"
            )
        })?;
        for artifact in artifacts {
            let artifact_role = string_field(artifact, "role").unwrap_or_default();
            if artifact_role == "license-material" {
                let material_role = family_identifier(
                    artifact.get("material_role"),
                    &format!("Component receipt {receipt_relative} license material role"),
                )?;
                if !matches!(material_role.as_str(), "license" | "notice") {
                    return Err(format!(
                        "Component receipt {receipt_relative} has unsupported license material role {material_role:?}."
                    ));
                }
                let declared_path = family_relative_path(
                    &required_string_field(artifact, "declared_path")?,
                    &format!("Component receipt {receipt_relative} license declared path"),
                )?;
                let required_path = if material_role == "license" {
                    "LICENSE"
                } else {
                    "NOTICE"
                };
                if declared_path != required_path {
                    return Err(format!(
                        "Family source repository {receipt_repo:?} {material_role} material must use exact Snapshot path {required_path:?}, got {declared_path:?}."
                    ));
                }
                let source_relative = family_relative_path(
                    &required_string_field(artifact, "path")?,
                    &format!("Component receipt {receipt_relative} license material path"),
                )?;
                let source_path = receipt_parent.join(&source_relative);
                let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
                    format!(
                        "Source repository {receipt_repo:?} {material_role} material is unavailable at {}: {error}",
                        source_path.display()
                    )
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(format!(
                        "Source repository {receipt_repo:?} {material_role} material must be a real regular file: {}.",
                        source_path.display()
                    ));
                }
                let canonical_source = source_path.canonicalize().map_err(io_error)?;
                if !canonical_source.starts_with(&canonical_parent) {
                    return Err(format!(
                        "Source repository {receipt_repo:?} {material_role} material escapes its receipt root: {}.",
                        source_path.display()
                    ));
                }
                let size_bytes = artifact
                    .get("size_bytes")
                    .and_then(JsonValue::as_u64)
                    .ok_or_else(|| {
                        format!(
                            "Source repository {receipt_repo:?} {material_role} material is missing size_bytes."
                        )
                    })?;
                let digest = required_string_field(artifact, "sha256")?;
                if !valid_sha256_lower(&digest) {
                    return Err(format!(
                        "Source repository {receipt_repo:?} {material_role} material has an invalid lowercase SHA-256."
                    ));
                }
                let (verified_size, verified_digest) = hash_regular_file(
                    &source_path,
                    &format!("Source repository {receipt_repo:?} {material_role} material"),
                )?;
                if metadata.len() != size_bytes
                    || verified_size != size_bytes
                    || verified_digest != digest
                {
                    return Err(format!(
                        "Source repository {receipt_repo:?} {material_role} material differs from its receipt evidence: {}.",
                        source_path.display()
                    ));
                }
                let key = (receipt_repo.clone(), material_role.clone());
                if let Some(existing) = license_sources.get_mut(&key) {
                    if existing.source_snapshot != receipt_snapshot
                        || existing.declared_path != declared_path
                        || existing.sha256 != digest
                        || existing.size_bytes != size_bytes
                    {
                        return Err(format!(
                            "Source repository {receipt_repo:?} {material_role} material conflicts across target receipts."
                        ));
                    }
                    existing
                        .receipt_relative_paths
                        .insert(receipt_relative.clone());
                } else {
                    license_sources.insert(
                        key,
                        FamilyLicenseMaterialSource {
                            source_repository: receipt_repo.clone(),
                            source_snapshot: receipt_snapshot.clone(),
                            role: material_role,
                            declared_path,
                            sha256: digest,
                            size_bytes,
                            receipt_relative_paths: BTreeSet::from([receipt_relative.clone()]),
                            source_relative_path: source_relative,
                            source_path,
                        },
                    );
                }
                continue;
            }
            if artifact_role != "component-artifact" {
                continue;
            }
            let component_id = required_string_field(artifact, "component")?;
            let expected = expected_components.get(&component_id).ok_or_else(|| {
                format!(
                    "Component receipt {receipt_relative} contains undeclared family component {component_id:?}."
                )
            })?;
            receipt_component_ids.insert(component_id.clone());
            if receipt_repo != expected.source_repository
                || receipt_snapshot != expected.source_snapshot
                || receipt_version != expected.version
            {
                return Err(format!(
                    "Component {component_id} receipt source/version identity does not match the family manifest."
                ));
            }
            let ecosystem = required_string_field(artifact, "ecosystem")?;
            if ecosystem != expected.ecosystem
                || component_definition_ecosystem(&receipt, &component_id).as_deref()
                    != Some(expected.ecosystem.as_str())
            {
                return Err(format!(
                    "Component {component_id} receipt ecosystem does not match {:?}.",
                    expected.ecosystem
                ));
            }
            let kind = required_string_field(artifact, "kind")?;
            let target = match artifact.get("target") {
                None | Some(JsonValue::Null) => None,
                Some(value) => Some(family_identifier(
                    Some(value),
                    &format!("Component {component_id} artifact target"),
                )?),
            };
            let key = (kind.clone(), target.clone());
            if receipt_target.is_some() && receipt_target != target {
                return Err(format!(
                    "Component {component_id} artifact target {target:?} differs from receipt selector {receipt_target:?}."
                ));
            }
            if !component_definition_artifact_keys(&receipt, &component_id)?.contains(&key) {
                return Err(format!(
                    "Component {component_id} artifact key {key:?} is absent from its receipt-bound adapter definition."
                ));
            }
            if !actual_keys
                .entry(component_id.clone())
                .or_default()
                .insert(key.clone())
            {
                return Err(format!(
                    "Component {component_id} contains duplicate artifact requirement {key:?}."
                ));
            }
            let artifact_path_text = required_string_field(artifact, "path")?;
            let source_relative = family_relative_path(
                &artifact_path_text,
                &format!("Component {component_id} artifact path"),
            )?;
            let source_path = receipt_parent.join(&source_relative);
            let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
                format!(
                    "Component {component_id} artifact is unavailable at {}: {error}",
                    source_path.display()
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "Component {component_id} artifact must be a real regular file: {}.",
                    source_path.display()
                ));
            }
            let canonical_source = source_path.canonicalize().map_err(io_error)?;
            if !canonical_source.starts_with(&canonical_parent) {
                return Err(format!(
                    "Component {component_id} artifact escapes its receipt root: {}.",
                    source_path.display()
                ));
            }
            let size_bytes = artifact
                .get("size_bytes")
                .and_then(JsonValue::as_u64)
                .ok_or_else(|| {
                    format!("Component {component_id} artifact is missing size_bytes.")
                })?;
            if metadata.len() != size_bytes {
                return Err(format!(
                    "Component {component_id} artifact size differs from its receipt: {}.",
                    source_path.display()
                ));
            }
            let digest = required_string_field(artifact, "sha256")?;
            if !valid_sha256_lower(&digest) {
                return Err(format!(
                    "Component {component_id} artifact has an invalid lowercase SHA-256."
                ));
            }
            let (verified_size, verified_digest) =
                hash_regular_file(&source_path, &format!("Component {component_id} artifact"))?;
            if verified_size != size_bytes || verified_digest != digest {
                return Err(format!(
                    "Component {component_id} artifact SHA-256 differs from its receipt: {}.",
                    source_path.display()
                ));
            }
            sources.push(FamilyArtifactSource {
                component: component_id,
                ecosystem,
                kind,
                target,
                sha256: digest,
                size_bytes,
                receipt_relative_path: receipt_relative.clone(),
                source_relative_path: source_relative,
                source_path,
            });
        }
        if receipt_component_ids.is_empty() {
            return Err(format!(
                "Component receipt {receipt_relative} does not supply any declared family artifact."
            ));
        }
        let receipt_components = receipt_component_ids.iter().cloned().collect::<Vec<_>>();
        for component_id in receipt_component_ids {
            component_receipts
                .entry(component_id)
                .or_default()
                .insert(receipt_relative.clone());
        }
        receipt_evidence.push(json!({
            "path": receipt_relative,
            "sha256": sha256_hex(&receipt_bytes),
            "contract": contract,
            "repo_name": receipt_repo,
            "snapshot_id": receipt_snapshot,
            "version": receipt_version,
            "components": receipt_components,
            "git_commit": receipt_git_commit,
        }));
    }

    for component in &manifest.components {
        let actual = actual_keys.get(&component.id).cloned().unwrap_or_default();
        let expected = component.expected_artifact_keys();
        if actual != expected {
            let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
            let extra = actual.difference(&expected).cloned().collect::<Vec<_>>();
            return Err(format!(
                "Component {} artifact coverage differs from the family manifest (missing: {:?}; extra: {:?}).",
                component.id, missing, extra
            ));
        }
        if !component_receipts.contains_key(&component.id) {
            return Err(format!(
                "Family component {} has no admitted component receipt.",
                component.id
            ));
        }
    }

    let expected_license_keys = expected_repositories
        .keys()
        .flat_map(|repository| {
            ["license", "notice"]
                .into_iter()
                .map(|role| (repository.clone(), role.to_string()))
        })
        .collect::<BTreeSet<_>>();
    let actual_license_keys = license_sources.keys().cloned().collect::<BTreeSet<_>>();
    if actual_license_keys != expected_license_keys {
        let missing = expected_license_keys
            .difference(&actual_license_keys)
            .cloned()
            .collect::<Vec<_>>();
        let extra = actual_license_keys
            .difference(&expected_license_keys)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "Family repository license-material coverage is incomplete (missing: {missing:?}; extra: {extra:?})."
        ));
    }

    sources.sort_by_key(|artifact| {
        (
            artifact.component.clone(),
            artifact.kind.clone(),
            artifact.target.clone(),
        )
    });
    let license_material = license_sources.into_values().collect::<Vec<_>>();
    receipt_evidence.sort_by_key(|row| string_field(row, "path").unwrap_or_default());
    let artifact_rows = sources
        .iter()
        .map(|artifact| {
            json!({
                "role": "component-artifact",
                "component": artifact.component,
                "ecosystem": artifact.ecosystem,
                "kind": artifact.kind,
                "target": artifact.target,
                "sha256": artifact.sha256,
                "size_bytes": artifact.size_bytes,
                "source_receipt": artifact.receipt_relative_path,
                "source_path": artifact.source_relative_path,
            })
        })
        .collect::<Vec<_>>();
    let license_rows = license_material
        .iter()
        .map(|material| {
            json!({
                "role": "license-material",
                "source_repository": material.source_repository,
                "source_snapshot": material.source_snapshot,
                "material_role": material.role,
                "declared_path": material.declared_path,
                "sha256": material.sha256,
                "size_bytes": material.size_bytes,
                "source_receipts": material.receipt_relative_paths.iter().cloned().collect::<Vec<_>>(),
                "source_path": material.source_relative_path,
            })
        })
        .collect::<Vec<_>>();
    let created_at = required_string_field(candidate, "created_at")?;
    let release_id = required_string_field(candidate, "release_id")?;
    let mut checks = vec![
        check_result(
            "family_candidate_integrity",
            "Family candidate matches its immutable Snapshot manifest",
            "pass",
            format!(
                "Verified candidate {release_id} and family manifest SHA-256 {}.",
                required_string_field(candidate, "family_manifest_sha256")?
            ),
            false,
        ),
        check_result(
            "component_receipts",
            "Every declared component has one passing build receipt",
            "pass",
            format!(
                "Verified {} component(s) from {} receipt bundle(s).",
                manifest.components.len(),
                receipt_evidence.len()
            ),
            false,
        ),
        check_result(
            "ecosystem_versions",
            "Component ecosystem versions match the family mapping",
            "pass",
            format!(
                "Verified family version {} across family-exact and PEP 440 component mappings.",
                manifest.family.version
            ),
            false,
        ),
        check_result(
            "platform_coverage",
            "Component artifacts exactly cover the declared target matrix",
            "pass",
            format!(
                "Verified {} artifact row(s) across {} declared target(s).",
                sources.len(),
                manifest.targets.len()
            ),
            false,
        ),
        check_result(
            "artifact_integrity",
            "Every component artifact matches its size and SHA-256 receipt",
            "pass",
            "All component artifact bytes were read and matched their immutable receipt evidence.",
            false,
        ),
        check_result(
            "license_material",
            "Every source repository supplies exact Snapshot-bound LICENSE and NOTICE material",
            "pass",
            format!(
                "Verified and deduplicated {} license-material file(s) across {} source repository Snapshot(s).",
                license_material.len(),
                expected_repositories.len()
            ),
            false,
        ),
    ];
    if let (Some(authority), Some(git_commit)) = (&public_authority, &public_git_commit) {
        checks.push(check_result(
            "public_git_source_authority",
            "Every component receipt binds the same validated public Git source commit",
            "pass",
            format!(
                "Verified commit {git_commit}, mapping SHA-256 {}, and coordinator Snapshot {}.",
                authority.mapping_sha256, authority.coordinator_snapshot
            ),
            false,
        ));
    }
    let check_count = checks.len();
    let record = json!({
        "contract": FAMILY_RELEASE_CHECK_CONTRACT,
        "command": "release check",
        "release_id": release_id,
        "repo_name": required_string_field(candidate, "repo_name")?,
        "version": manifest.family.version,
        "channel": manifest.family.channel,
        "tag": manifest.family.tag,
        "line": required_string_field(candidate, "line")?,
        "snapshot_id": required_string_field(candidate, "snapshot_id")?,
        "manifest_hash": required_string_field(candidate, "manifest_hash")?,
        "profile": FAMILY_RELEASE_PROFILE,
        "status": "checked",
        "family_manifest_sha256": required_string_field(candidate, "family_manifest_sha256")?,
        "family": manifest.to_json(),
        "checks": checks,
        "check_summary": {
            "total": check_count,
            "passed": check_count,
            "failed": 0,
            "blocking": 0,
            "decision": "pass",
        },
        "component_receipts": receipt_evidence,
        "artifacts": artifact_rows,
        "license_material": license_rows,
        "authority": candidate.get("authority").cloned().unwrap_or_else(|| json!({})),
        "created_at": created_at,
        "updated_at": created_at,
        "next_action": {
            "code": "build_family",
            "detail": format!("Run `ait release build {release_id} --receipts <dir>` to freeze the exact admitted artifact bytes."),
        },
    });
    let _ = repo;
    Ok(FamilyAdmission {
        record,
        artifacts: sources,
        license_material,
    })
}

pub fn family_release_check(
    repo: &RepoRuntime,
    release_id: &str,
    receipts_root: &Path,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    let candidate = load_family_candidate(repo, release_id, public_source_root)?;
    let admission = inspect_family_receipts(repo, &candidate, receipts_root, public_source_root)?;
    let path = family_release_dir(repo, release_id, false)?.join(FAMILY_CHECK_FILENAME);
    write_json_once(&path, &admission.record, "Family release check receipt")?;
    Ok(admission.record)
}

fn safe_destination_segment(value: Option<&str>) -> String {
    value.unwrap_or("portable").to_string()
}

fn build_artifact_destination(
    staging_root: &Path,
    artifact: &FamilyArtifactSource,
) -> Result<PathBuf, String> {
    let filename = artifact
        .source_path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| {
            format!(
                "Component {} artifact has no portable filename.",
                artifact.component
            )
        })?;
    let filename = family_relative_path(filename, "Component artifact filename")?;
    Ok(staging_root
        .join("artifacts")
        .join(&artifact.component)
        .join(&artifact.kind)
        .join(safe_destination_segment(artifact.target.as_deref()))
        .join(filename))
}

fn build_license_material_destination(
    staging_root: &Path,
    material: &FamilyLicenseMaterialSource,
) -> Result<PathBuf, String> {
    let declared_path = family_relative_path(
        &material.declared_path,
        "Repository license material declared path",
    )?;
    Ok(staging_root
        .join("license-material")
        .join(&material.source_repository)
        .join(&material.role)
        .join(declared_path))
}

fn frozen_relative_path(path: &Path, frozen_root: &Path) -> Result<String, String> {
    path.strip_prefix(frozen_root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .map_err(|_| {
            format!(
                "Frozen family artifact escapes its release root: {}.",
                path.display()
            )
        })
}

fn family_promotion_routes(
    channel: &str,
    version: &str,
    tag: &str,
    distributions: &JsonValue,
) -> JsonValue {
    let registry_prerelease = channel == "rc";
    json!({
        "distributions": distributions,
        "github": {
            "tag": tag,
            "prerelease": registry_prerelease,
            "draft": false,
        },
        "npm": {
            "version": version,
            "dist_tag": if registry_prerelease { "rc" } else { "latest" },
        },
        "pypi": {
            "repository": "pypi",
            "prerelease": registry_prerelease,
        },
        "oci": {
            "version_tag": version,
            "moving_tag": if registry_prerelease { "rc" } else { "latest" },
        },
        "homebrew": {
            "channel": if registry_prerelease { "rc" } else { "stable" },
            "stable_formula_mutation": !registry_prerelease,
        },
        "apt": {
            "suite": if registry_prerelease { "testing" } else { "stable" },
        },
        "winget": {
            "route": if registry_prerelease { "validation" } else { "community" },
            "community_manifest_submission": !registry_prerelease,
        },
    })
}

fn candidate_distributions(candidate: &JsonValue) -> Result<&JsonValue, String> {
    candidate
        .get("family")
        .and_then(|family| family.get("distributions"))
        .filter(|value| value.is_array())
        .ok_or_else(|| "Family candidate is missing its distribution identity matrix.".to_string())
}

fn family_source_publication_requirements(candidate: &JsonValue) -> Result<JsonValue, String> {
    let family = candidate
        .get("family")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Family candidate is missing its bound family manifest.".to_string())?;
    let tag = required_string_field(candidate, "tag")?;
    let public_source = family
        .get("public_source")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "Family candidate is missing its public monorepo source contract.".to_string()
        })?;
    if public_source.get("model").and_then(JsonValue::as_str) != Some("release-monorepo") {
        return Err("Family candidate public source model must be release-monorepo.".to_string());
    }
    let github_identity = public_source
        .get("identity")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "Family public source contract is missing its GitHub identity.".to_string())?
        .to_string();
    let mapping_manifest = public_source
        .get("mapping_manifest")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            "Family public source contract is missing its mapping manifest path.".to_string()
        })?
        .to_string();
    let product_document = public_source
        .get("product_document")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            "Family public source contract is missing its product document path.".to_string()
        })?
        .to_string();
    let subtree_rows = public_source
        .get("subtrees")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Family public source contract is missing subtrees.".to_string())?;
    let mut subtree_by_repository = BTreeMap::<String, (String, JsonValue)>::new();
    for subtree in subtree_rows {
        let repository = required_string_field(subtree, "source_repository")?;
        let path = required_string_field(subtree, "path")?;
        let transforms = subtree
            .get("transforms")
            .cloned()
            .ok_or_else(|| "Family public source subtree is missing transforms.".to_string())?;
        if subtree_by_repository
            .insert(repository.clone(), (path, transforms))
            .is_some()
        {
            return Err(format!(
                "Family public source contract contains duplicate subtree {repository:?}."
            ));
        }
    }
    let components = family
        .get("components")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Family candidate is missing components.".to_string())?;
    let mut repositories = BTreeMap::<String, (String, String, BTreeSet<String>)>::new();
    for component in components {
        let component_id = required_string_field(component, "id")?;
        let repository = required_string_field(component, "source_repository")?;
        let snapshot = required_string_field(component, "source_snapshot")?;
        let license = required_string_field(component, "license")?;
        match repositories.get_mut(&repository) {
            Some((bound_snapshot, bound_license, component_ids)) => {
                if bound_snapshot != &snapshot || bound_license != &license {
                    return Err(format!(
                        "Source repository {repository:?} has conflicting public source authority."
                    ));
                }
                component_ids.insert(component_id);
            }
            None => {
                repositories.insert(
                    repository,
                    (snapshot, license, BTreeSet::from([component_id])),
                );
            }
        }
    }
    if repositories.is_empty() || subtree_by_repository.len() != repositories.len() {
        return Err(
            "Family public source authority must bind every component repository subtree."
                .to_string(),
        );
    }
    let subtrees = repositories
        .into_iter()
        .map(|(repository, (snapshot, license, components))| {
            let (path, transforms) = subtree_by_repository.remove(&repository).ok_or_else(|| {
                format!("Family public source contract is missing subtree {repository:?}.")
            })?;
            Ok(json!({
                "source_repository": repository,
                "source_snapshot": snapshot,
                "path": path,
                "license": license,
                "components": components,
                "transforms": transforms,
                "public_source_url": format!("https://github.com/{github_identity}/tree/{tag}/{path}"),
                "agpl_corresponding_source": license == "AGPL-3.0-only",
                "locked_external_source_closure_required": true,
                "status": "required_unverified",
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let public_source_url = format!("https://github.com/{github_identity}/tree/{tag}");
    Ok(json!({
        "status": "required_unverified",
        "publication_order": "all_source_before_any_binary_endpoint",
        "binary_publication_allowed": false,
        "source_requirement_count": 1,
        "subtree_count": subtrees.len(),
        "requirements": [{
            "model": "release-monorepo",
            "github_repository": github_identity,
            "tag": tag,
            "public_source_url": public_source_url,
            "mapping_manifest": mapping_manifest,
            "product_document": product_document,
            "subtrees": subtrees,
            "required_evidence": [
                "monorepo_export_manifest_sha256",
                "subtree_snapshot_mapping",
                "git_commit_identity",
                "monorepo_commit_tree_readback",
                "public_tag_readback",
                "locked_dependency_and_build_script_readback",
                "clean_clone_build"
            ],
            "status": "required_unverified",
        }],
    }))
}

fn reject_symlink_components(root: &Path, relative: &str, context: &str) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        let std::path::Component::Normal(component) = component else {
            return Err(format!("{context} contains a non-normal path component."));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            format!("{context} is unavailable at {}: {error}", current.display())
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "{context} traverses a symbolic link: {}.",
                current.display()
            ));
        }
    }
    Ok(())
}

fn validate_checksum_file(frozen_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let checksum_path = frozen_root.join(FAMILY_CHECKSUM_FILENAME);
    let bytes = read_bounded_file(
        &checksum_path,
        MAX_DOSSIER_BYTES,
        "Frozen family checksum manifest",
    )?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "Frozen family checksum manifest must be UTF-8.".to_string())?;
    let canonical_root = frozen_root.canonicalize().map_err(io_error)?;
    let mut checksums = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let Some((digest, relative)) = line.split_once("  ") else {
            return Err(format!(
                "Frozen family checksum line {} is malformed.",
                index + 1
            ));
        };
        if !valid_sha256_lower(digest) {
            return Err(format!(
                "Frozen family checksum line {} has an invalid SHA-256.",
                index + 1
            ));
        }
        let relative = family_relative_path(relative, "Frozen checksum path")?;
        if checksums
            .insert(relative.clone(), digest.to_string())
            .is_some()
        {
            return Err(format!(
                "Frozen family checksum contains duplicate path {relative:?}."
            ));
        }
        let path = frozen_root.join(&relative);
        reject_symlink_components(frozen_root, &relative, "Frozen checksum member")?;
        let canonical = path.canonicalize().map_err(|error| {
            format!("Frozen checksum member {relative:?} is unavailable: {error}")
        })?;
        if !canonical.starts_with(&canonical_root) {
            return Err(format!(
                "Frozen checksum member {relative:?} escapes the frozen release root."
            ));
        }
        let (_, member_digest) = hash_regular_file(&path, "Frozen checksum member")?;
        if member_digest != digest {
            return Err(format!(
                "Frozen checksum member {relative:?} does not match its SHA-256."
            ));
        }
    }
    if checksums.is_empty() {
        return Err("Frozen family checksum manifest is empty.".to_string());
    }
    Ok(checksums)
}

fn unpromoted_family_state(candidate: &JsonValue) -> Result<JsonValue, String> {
    Ok(json!({
        "authorized": false,
        "performed": false,
        "registry_write": false,
        "source_publication": family_source_publication_requirements(candidate)?,
        "routes": family_promotion_routes(
            &required_string_field(candidate, "channel")?,
            &required_string_field(candidate, "version")?,
            &required_string_field(candidate, "tag")?,
            candidate_distributions(candidate)?,
        ),
    }))
}

fn validate_frozen_family_manifest(
    candidate: &JsonValue,
    release_id: &str,
    frozen_root: &Path,
    manifest: &JsonValue,
    manifest_bytes: &[u8],
    checksums: &BTreeMap<String, String>,
) -> Result<(Vec<JsonValue>, Vec<JsonValue>), String> {
    if string_field(manifest, "contract").as_deref()
        != Some(FAMILY_RELEASE_FROZEN_MANIFEST_CONTRACT)
        || string_field(manifest, "release_id").as_deref() != Some(release_id)
        || string_field(manifest, "repo_name") != string_field(candidate, "repo_name")
        || string_field(manifest, "version") != string_field(candidate, "version")
        || string_field(manifest, "channel") != string_field(candidate, "channel")
        || string_field(manifest, "tag") != string_field(candidate, "tag")
        || string_field(manifest, "line") != string_field(candidate, "line")
        || string_field(manifest, "snapshot_id") != string_field(candidate, "snapshot_id")
        || string_field(manifest, "source_manifest_hash")
            != string_field(candidate, "manifest_hash")
        || string_field(manifest, "family_manifest_sha256")
            != string_field(candidate, "family_manifest_sha256")
        || manifest.get("family") != candidate.get("family")
        || string_field(manifest, "built_at") != string_field(candidate, "created_at")
        || manifest.get("promotion") != Some(&unpromoted_family_state(candidate)?)
    {
        return Err("Frozen family manifest does not match its immutable candidate.".to_string());
    }

    let definition = candidate
        .get("family")
        .ok_or_else(|| "Family candidate is missing its manifest definition.".to_string())?;
    let family = parse_family_release_manifest(definition)?;
    let expected_components = family
        .components
        .iter()
        .map(|component| (component.id.clone(), component))
        .collect::<BTreeMap<_, _>>();
    let artifacts = manifest
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Frozen family manifest is missing artifacts.".to_string())?;
    if artifacts.len() != family.expected_artifact_count() {
        return Err(format!(
            "Frozen family manifest contains {} artifacts; expected {}.",
            artifacts.len(),
            family.expected_artifact_count()
        ));
    }

    let mut actual = BTreeMap::<String, BTreeSet<(String, Option<String>)>>::new();
    let mut expected_checksum_paths = BTreeSet::new();
    for (index, artifact) in artifacts.iter().enumerate() {
        let context = format!("Frozen family artifact {index}");
        let object = family_object(artifact, &context)?;
        family_known_fields(
            object,
            &[
                "role",
                "component",
                "ecosystem",
                "kind",
                "target",
                "path",
                "sha256",
                "size_bytes",
                "source_receipt",
                "source_path",
            ],
            &context,
        )?;
        if string_field(artifact, "role").as_deref() != Some("component-artifact") {
            return Err(format!("{context} has an unsupported role."));
        }
        let component_id =
            family_identifier(artifact.get("component"), &format!("{context}.component"))?;
        let component = expected_components
            .get(&component_id)
            .ok_or_else(|| format!("{context} names undeclared component {component_id:?}."))?;
        let ecosystem =
            family_identifier(artifact.get("ecosystem"), &format!("{context}.ecosystem"))?;
        if ecosystem != component.ecosystem {
            return Err(format!(
                "{context} ecosystem does not match the family manifest."
            ));
        }
        let kind = family_identifier(artifact.get("kind"), &format!("{context}.kind"))?;
        let target = match artifact.get("target") {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(family_identifier(
                Some(value),
                &format!("{context}.target"),
            )?),
        };
        let key = (kind.clone(), target.clone());
        if !actual
            .entry(component_id.clone())
            .or_default()
            .insert(key.clone())
        {
            return Err(format!("{context} duplicates artifact key {key:?}."));
        }
        let _source_receipt = family_relative_path(
            &required_string_field(artifact, "source_receipt")?,
            &format!("{context}.source_receipt"),
        )?;
        let source_path = family_relative_path(
            &required_string_field(artifact, "source_path")?,
            &format!("{context}.source_path"),
        )?;
        let filename = Path::new(&source_path)
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("{context}.source_path has no portable filename."))?;
        let expected_path = format!(
            "artifacts/{component_id}/{kind}/{}/{}",
            safe_destination_segment(target.as_deref()),
            family_relative_path(filename, &format!("{context} filename"))?
        );
        let path = family_relative_path(
            &required_string_field(artifact, "path")?,
            &format!("{context}.path"),
        )?;
        if path != expected_path {
            return Err(format!(
                "{context} path differs from its deterministic frozen projection: expected {expected_path:?}."
            ));
        }
        let digest = required_string_field(artifact, "sha256")?;
        if !valid_sha256_lower(&digest) {
            return Err(format!("{context} has an invalid lowercase SHA-256."));
        }
        let size_bytes = artifact
            .get("size_bytes")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| format!("{context} is missing size_bytes."))?;
        if checksums.get(&path).map(String::as_str) != Some(digest.as_str()) {
            return Err(format!(
                "{context} is missing from the exact checksum manifest."
            ));
        }
        let (actual_size, actual_digest) = hash_regular_file(&frozen_root.join(&path), &context)?;
        if actual_size != size_bytes || actual_digest != digest {
            return Err(format!(
                "{context} bytes differ from its frozen manifest evidence."
            ));
        }
        expected_checksum_paths.insert(path);
    }
    for component in &family.components {
        let component_actual = actual.get(&component.id).cloned().unwrap_or_default();
        if component_actual != component.expected_artifact_keys() {
            return Err(format!(
                "Frozen family component {} does not exactly cover its declared artifact matrix.",
                component.id
            ));
        }
    }
    let mut expected_repositories = BTreeMap::<String, String>::new();
    for component in &family.components {
        match expected_repositories.get(&component.source_repository) {
            Some(snapshot) if snapshot != &component.source_snapshot => {
                return Err(format!(
                    "Frozen family source repository {:?} is bound to conflicting Snapshots.",
                    component.source_repository
                ));
            }
            Some(_) => {}
            None => {
                expected_repositories.insert(
                    component.source_repository.clone(),
                    component.source_snapshot.clone(),
                );
            }
        }
    }
    let license_material = manifest
        .get("license_material")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Frozen family manifest is missing license_material.".to_string())?;
    let expected_license_keys = expected_repositories
        .keys()
        .flat_map(|repository| {
            ["license", "notice"]
                .into_iter()
                .map(|role| (repository.clone(), role.to_string()))
        })
        .collect::<BTreeSet<_>>();
    if license_material.len() != expected_license_keys.len() {
        return Err(format!(
            "Frozen family manifest contains {} license-material rows; expected {}.",
            license_material.len(),
            expected_license_keys.len()
        ));
    }
    let mut actual_license_keys = BTreeSet::new();
    for (index, material) in license_material.iter().enumerate() {
        let context = format!("Frozen family license material {index}");
        let object = family_object(material, &context)?;
        family_known_fields(
            object,
            &[
                "role",
                "source_repository",
                "source_snapshot",
                "material_role",
                "declared_path",
                "path",
                "sha256",
                "size_bytes",
                "source_receipts",
                "source_path",
            ],
            &context,
        )?;
        if string_field(material, "role").as_deref() != Some("license-material") {
            return Err(format!("{context} has an unsupported role."));
        }
        let source_repository = family_identifier(
            material.get("source_repository"),
            &format!("{context}.source_repository"),
        )?;
        let expected_snapshot = expected_repositories
            .get(&source_repository)
            .ok_or_else(|| format!("{context} names an undeclared source repository."))?;
        let source_snapshot = family_snapshot_id(
            material.get("source_snapshot"),
            &format!("{context}.source_snapshot"),
        )?;
        if &source_snapshot != expected_snapshot {
            return Err(format!(
                "{context} source Snapshot differs from the family component authority."
            ));
        }
        let material_role = family_identifier(
            material.get("material_role"),
            &format!("{context}.material_role"),
        )?;
        if !matches!(material_role.as_str(), "license" | "notice") {
            return Err(format!("{context} has an unsupported material role."));
        }
        let key = (source_repository.clone(), material_role.clone());
        if !actual_license_keys.insert(key) {
            return Err(format!("{context} duplicates repository license material."));
        }
        let declared_path = family_relative_path(
            &required_string_field(material, "declared_path")?,
            &format!("{context}.declared_path"),
        )?;
        let required_path = if material_role == "license" {
            "LICENSE"
        } else {
            "NOTICE"
        };
        if declared_path != required_path {
            return Err(format!(
                "{context} must use exact declared path {required_path:?}."
            ));
        }
        let expected_path =
            format!("license-material/{source_repository}/{material_role}/{declared_path}");
        let path = family_relative_path(
            &required_string_field(material, "path")?,
            &format!("{context}.path"),
        )?;
        if path != expected_path {
            return Err(format!(
                "{context} path differs from its deterministic frozen projection: expected {expected_path:?}."
            ));
        }
        let source_path = family_relative_path(
            &required_string_field(material, "source_path")?,
            &format!("{context}.source_path"),
        )?;
        if !source_path.ends_with(&format!(
            "/license-material/{material_role}/{declared_path}"
        )) {
            return Err(format!(
                "{context}.source_path does not match the generic receipt projection."
            ));
        }
        let source_receipts = material
            .get("source_receipts")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| format!("{context}.source_receipts must be an array."))?;
        if source_receipts.is_empty() || source_receipts.len() > MAX_RECEIPTS {
            return Err(format!(
                "{context}.source_receipts must contain between 1 and {MAX_RECEIPTS} rows."
            ));
        }
        let mut receipt_paths = BTreeSet::new();
        for (receipt_index, receipt) in source_receipts.iter().enumerate() {
            let receipt = family_relative_path(
                receipt.as_str().ok_or_else(|| {
                    format!("{context}.source_receipts[{receipt_index}] must be a string.")
                })?,
                &format!("{context}.source_receipts[{receipt_index}]"),
            )?;
            if !receipt_paths.insert(receipt) {
                return Err(format!("{context}.source_receipts contains a duplicate."));
            }
        }
        if receipt_paths.iter().cloned().collect::<Vec<_>>()
            != source_receipts
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        {
            return Err(format!(
                "{context}.source_receipts must be sorted deterministically."
            ));
        }
        let digest = required_string_field(material, "sha256")?;
        if !valid_sha256_lower(&digest) {
            return Err(format!("{context} has an invalid lowercase SHA-256."));
        }
        let size_bytes = material
            .get("size_bytes")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| format!("{context} is missing size_bytes."))?;
        if checksums.get(&path).map(String::as_str) != Some(digest.as_str()) {
            return Err(format!(
                "{context} is missing from the exact checksum manifest."
            ));
        }
        let (actual_size, actual_digest) = hash_regular_file(&frozen_root.join(&path), &context)?;
        if actual_size != size_bytes || actual_digest != digest {
            return Err(format!(
                "{context} bytes differ from its frozen manifest evidence."
            ));
        }
        expected_checksum_paths.insert(path);
    }
    if actual_license_keys != expected_license_keys {
        return Err(
            "Frozen family license material does not exactly cover every source repository."
                .to_string(),
        );
    }
    expected_checksum_paths.insert(FAMILY_FROZEN_MANIFEST_FILENAME.to_string());
    let manifest_digest = sha256_hex(manifest_bytes);
    if checksums.keys().cloned().collect::<BTreeSet<_>>() != expected_checksum_paths
        || checksums
            .get(FAMILY_FROZEN_MANIFEST_FILENAME)
            .map(String::as_str)
            != Some(manifest_digest.as_str())
    {
        return Err(
            "Frozen family checksum inventory does not exactly match the manifest and component artifacts."
                .to_string(),
        );
    }
    Ok((artifacts.clone(), license_material.clone()))
}

fn validate_family_build(
    repo: &RepoRuntime,
    release_id: &str,
    admission: Option<&FamilyAdmission>,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    let candidate = load_family_candidate(repo, release_id, public_source_root)?;
    let release_dir = family_release_dir(repo, release_id, false)?;
    let frozen_root = release_dir.join(FAMILY_FROZEN_DIRNAME);
    let metadata = fs::symlink_metadata(&frozen_root)
        .map_err(|error| format!("Frozen family release is unavailable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Frozen family release must be a real directory.".to_string());
    }
    let checksums = validate_checksum_file(&frozen_root)?;
    let build_path = frozen_root.join(FAMILY_BUILD_FILENAME);
    let bytes = read_bounded_file(&build_path, MAX_DOSSIER_BYTES, "Family build receipt")?;
    let build = parse_slice_value(&bytes, "Family build receipt must contain valid JSON")?;
    let manifest_path = frozen_root.join(FAMILY_FROZEN_MANIFEST_FILENAME);
    let manifest_bytes =
        read_bounded_file(&manifest_path, MAX_DOSSIER_BYTES, "Frozen family manifest")?;
    let manifest = parse_slice_value(
        &manifest_bytes,
        "Frozen family manifest must contain valid JSON",
    )?;
    let (frozen_artifacts, frozen_license_material) = validate_frozen_family_manifest(
        &candidate,
        release_id,
        &frozen_root,
        &manifest,
        &manifest_bytes,
        &checksums,
    )?;
    let checksum_path = frozen_root.join(FAMILY_CHECKSUM_FILENAME);
    let checksum_bytes = read_bounded_file(
        &checksum_path,
        MAX_DOSSIER_BYTES,
        "Frozen family checksum manifest",
    )?;
    let final_prefix = format!(
        "{}/{FAMILY_FROZEN_DIRNAME}",
        family_release_relative_dir(release_id)
    );
    let mut expected_build_artifacts = frozen_artifacts
        .iter()
        .map(|artifact| {
            let mut row = artifact.clone();
            let path = required_string_field(artifact, "path")?;
            row.as_object_mut()
                .ok_or_else(|| "Frozen artifact row must be an object.".to_string())?
                .insert("path".to_string(), json!(format!("{final_prefix}/{path}")));
            Ok(row)
        })
        .collect::<Result<Vec<_>, String>>()?;
    expected_build_artifacts.extend(
        frozen_license_material
            .iter()
            .map(|material| {
                let mut row = material.clone();
                let path = required_string_field(material, "path")?;
                row.as_object_mut()
                    .ok_or_else(|| "Frozen license-material row must be an object.".to_string())?
                    .insert("path".to_string(), json!(format!("{final_prefix}/{path}")));
                Ok(row)
            })
            .collect::<Result<Vec<_>, String>>()?,
    );
    expected_build_artifacts.push(json!({
        "role": "family-manifest",
        "kind": "manifest",
        "path": format!("{final_prefix}/{FAMILY_FROZEN_MANIFEST_FILENAME}"),
        "sha256": sha256_hex(&manifest_bytes),
        "size_bytes": manifest_bytes.len(),
    }));
    expected_build_artifacts.push(json!({
        "role": "family-checksum",
        "kind": "checksum",
        "path": format!("{final_prefix}/{FAMILY_CHECKSUM_FILENAME}"),
        "sha256": sha256_hex(&checksum_bytes),
        "size_bytes": checksum_bytes.len(),
    }));
    expected_build_artifacts.sort_by_key(|artifact| {
        (
            string_field(artifact, "role").unwrap_or_default(),
            string_field(artifact, "component").unwrap_or_default(),
            string_field(artifact, "source_repository").unwrap_or_default(),
            string_field(artifact, "kind").unwrap_or_default(),
            string_field(artifact, "material_role").unwrap_or_default(),
            string_field(artifact, "target"),
        )
    });

    let check_path = release_dir.join(FAMILY_CHECK_FILENAME);
    let check_bytes = read_bounded_file(
        &check_path,
        MAX_DOSSIER_BYTES,
        "Family release check receipt",
    )?;
    let check = parse_slice_value(
        &check_bytes,
        "Family release check receipt must contain valid JSON",
    )?;
    if string_field(&check, "contract").as_deref() != Some(FAMILY_RELEASE_CHECK_CONTRACT)
        || string_field(&check, "release_id").as_deref() != Some(release_id)
        || check
            .get("check_summary")
            .and_then(|summary| summary.get("decision"))
            .and_then(JsonValue::as_str)
            != Some("pass")
    {
        return Err(
            "Family release check receipt is not passing or has an invalid identity.".to_string(),
        );
    }
    let expected_manifest_path = format!("{final_prefix}/{FAMILY_FROZEN_MANIFEST_FILENAME}");
    let expected_checksum_path = format!("{final_prefix}/{FAMILY_CHECKSUM_FILENAME}");
    if string_field(&build, "contract").as_deref() != Some(FAMILY_RELEASE_BUILD_CONTRACT)
        || string_field(&build, "command").as_deref() != Some("release build")
        || string_field(&build, "release_id").as_deref() != Some(release_id)
        || string_field(&build, "repo_name") != string_field(&candidate, "repo_name")
        || string_field(&build, "version") != string_field(&candidate, "version")
        || string_field(&build, "channel") != string_field(&candidate, "channel")
        || string_field(&build, "tag") != string_field(&candidate, "tag")
        || string_field(&build, "line") != string_field(&candidate, "line")
        || string_field(&build, "snapshot_id") != string_field(&candidate, "snapshot_id")
        || string_field(&build, "manifest_hash") != string_field(&candidate, "manifest_hash")
        || string_field(&build, "profile").as_deref() != Some(FAMILY_RELEASE_PROFILE)
        || string_field(&build, "status").as_deref() != Some("built")
        || string_field(&build, "family_manifest_sha256")
            != string_field(&candidate, "family_manifest_sha256")
        || build.get("family") != candidate.get("family")
        || build.get("checks") != check.get("checks")
        || build.get("check_summary") != check.get("check_summary")
        || build.get("component_receipts") != manifest.get("component_receipts")
        || build.get("component_receipts") != check.get("component_receipts")
        || build.get("artifacts") != Some(&JsonValue::Array(expected_build_artifacts))
        || string_field(&build, "frozen_manifest_path").as_deref()
            != Some(expected_manifest_path.as_str())
        || string_field(&build, "checksum_path").as_deref() != Some(expected_checksum_path.as_str())
        || build.get("promotion") != Some(&unpromoted_family_state(&candidate)?)
        || build.get("authority") != candidate.get("authority")
        || string_field(&build, "created_at") != string_field(&candidate, "created_at")
        || string_field(&build, "updated_at") != string_field(&candidate, "created_at")
    {
        return Err(format!(
            "Family build receipt {release_id} does not exactly match its candidate, check, and frozen artifact evidence."
        ));
    }
    if let Some(admission) = admission {
        let admitted = admission
            .artifacts
            .iter()
            .map(|artifact| {
                (
                    artifact.component.clone(),
                    artifact.kind.clone(),
                    artifact.target.clone(),
                    artifact.sha256.clone(),
                    artifact.size_bytes,
                )
            })
            .collect::<BTreeSet<_>>();
        let built = build
            .get("artifacts")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter(|artifact| {
                string_field(artifact, "role").as_deref() == Some("component-artifact")
            })
            .map(|artifact| {
                Ok((
                    required_string_field(artifact, "component")?,
                    required_string_field(artifact, "kind")?,
                    string_field(artifact, "target"),
                    required_string_field(artifact, "sha256")?,
                    artifact
                        .get("size_bytes")
                        .and_then(JsonValue::as_u64)
                        .ok_or_else(|| {
                            "Built family artifact is missing size_bytes.".to_string()
                        })?,
                ))
            })
            .collect::<Result<BTreeSet<_>, String>>()?;
        if admitted != built {
            return Err(
                "Existing frozen family build differs from the currently admitted component receipts."
                    .to_string(),
            );
        }
        let admitted_license_material = admission
            .license_material
            .iter()
            .map(|material| {
                (
                    material.source_repository.clone(),
                    material.source_snapshot.clone(),
                    material.role.clone(),
                    material.declared_path.clone(),
                    material.sha256.clone(),
                    material.size_bytes,
                )
            })
            .collect::<BTreeSet<_>>();
        let built_license_material = build
            .get("artifacts")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter(|artifact| {
                string_field(artifact, "role").as_deref() == Some("license-material")
            })
            .map(|material| {
                Ok((
                    required_string_field(material, "source_repository")?,
                    required_string_field(material, "source_snapshot")?,
                    required_string_field(material, "material_role")?,
                    required_string_field(material, "declared_path")?,
                    required_string_field(material, "sha256")?,
                    material
                        .get("size_bytes")
                        .and_then(JsonValue::as_u64)
                        .ok_or_else(|| {
                            "Built family license material is missing size_bytes.".to_string()
                        })?,
                ))
            })
            .collect::<Result<BTreeSet<_>, String>>()?;
        if admitted_license_material != built_license_material {
            return Err(
                "Existing frozen family build differs from the currently admitted repository license material."
                    .to_string(),
            );
        }
    }
    Ok(build)
}

pub(super) fn validate_existing_family_build(
    repo: &RepoRuntime,
    release_id: &str,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    validate_family_build(repo, release_id, None, public_source_root)
}

pub fn family_release_build(
    repo: &RepoRuntime,
    release_id: &str,
    receipts_root: &Path,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    let candidate = load_family_candidate(repo, release_id, public_source_root)?;
    let admission = inspect_family_receipts(repo, &candidate, receipts_root, public_source_root)?;
    let release_dir = family_release_dir(repo, release_id, false)?;
    let check_path = release_dir.join(FAMILY_CHECK_FILENAME);
    write_json_once(
        &check_path,
        &admission.record,
        "Family release check receipt",
    )?;
    let frozen_root = release_dir.join(FAMILY_FROZEN_DIRNAME);
    if fs::symlink_metadata(&frozen_root).is_ok() {
        return validate_family_build(repo, release_id, Some(&admission), public_source_root);
    }

    let staging = TempDirBuilder::new()
        .prefix(".ait-family-release-")
        .tempdir_in(&release_dir)
        .map_err(io_error)?;
    let staging_root = staging.path();
    let mut frozen_artifacts = Vec::new();
    for source in &admission.artifacts {
        let destination = build_artifact_destination(staging_root, source)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        if destination.exists() {
            return Err(format!(
                "Family build destination collides for component {} kind {} target {:?}.",
                source.component, source.kind, source.target
            ));
        }
        fs::copy(&source.source_path, &destination).map_err(io_error)?;
        let (size_bytes, digest) = hash_regular_file(&destination, "Frozen component artifact")?;
        if size_bytes != source.size_bytes || digest != source.sha256 {
            return Err(format!(
                "Frozen copy for component {} changed during assembly.",
                source.component
            ));
        }
        frozen_artifacts.push(json!({
            "role": "component-artifact",
            "component": source.component,
            "ecosystem": source.ecosystem,
            "kind": source.kind,
            "target": source.target,
            "path": frozen_relative_path(&destination, staging_root)?,
            "sha256": source.sha256,
            "size_bytes": source.size_bytes,
            "source_receipt": source.receipt_relative_path,
            "source_path": source.source_relative_path,
        }));
    }
    frozen_artifacts.sort_by_key(|artifact| {
        (
            string_field(artifact, "component").unwrap_or_default(),
            string_field(artifact, "kind").unwrap_or_default(),
            string_field(artifact, "target"),
        )
    });
    let mut frozen_license_material = Vec::new();
    for source in &admission.license_material {
        let destination = build_license_material_destination(staging_root, source)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        if destination.exists() {
            return Err(format!(
                "Family build license-material destination collides for repository {} role {}.",
                source.source_repository, source.role
            ));
        }
        fs::copy(&source.source_path, &destination).map_err(io_error)?;
        let (size_bytes, digest) =
            hash_regular_file(&destination, "Frozen repository license material")?;
        if size_bytes != source.size_bytes || digest != source.sha256 {
            return Err(format!(
                "Frozen {} material for repository {} changed during assembly.",
                source.role, source.source_repository
            ));
        }
        frozen_license_material.push(json!({
            "role": "license-material",
            "source_repository": source.source_repository,
            "source_snapshot": source.source_snapshot,
            "material_role": source.role,
            "declared_path": source.declared_path,
            "path": frozen_relative_path(&destination, staging_root)?,
            "sha256": source.sha256,
            "size_bytes": source.size_bytes,
            "source_receipts": source.receipt_relative_paths.iter().cloned().collect::<Vec<_>>(),
            "source_path": source.source_relative_path,
        }));
    }
    frozen_license_material.sort_by_key(|material| {
        (
            string_field(material, "source_repository").unwrap_or_default(),
            string_field(material, "material_role").unwrap_or_default(),
        )
    });

    let family = candidate
        .get("family")
        .cloned()
        .ok_or_else(|| "Family candidate is missing its manifest definition.".to_string())?;
    let frozen_manifest = json!({
        "contract": FAMILY_RELEASE_FROZEN_MANIFEST_CONTRACT,
        "release_id": release_id,
        "repo_name": required_string_field(&candidate, "repo_name")?,
        "version": required_string_field(&candidate, "version")?,
        "channel": required_string_field(&candidate, "channel")?,
        "tag": required_string_field(&candidate, "tag")?,
        "line": required_string_field(&candidate, "line")?,
        "snapshot_id": required_string_field(&candidate, "snapshot_id")?,
        "source_manifest_hash": required_string_field(&candidate, "manifest_hash")?,
        "family_manifest_sha256": required_string_field(&candidate, "family_manifest_sha256")?,
        "family": family,
        "component_receipts": admission.record.get("component_receipts").cloned().unwrap_or_else(|| json!([])),
        "artifacts": frozen_artifacts,
        "license_material": frozen_license_material,
        "promotion": unpromoted_family_state(&candidate)?,
        "built_at": required_string_field(&candidate, "created_at")?,
    });
    let manifest_path = staging_root.join(FAMILY_FROZEN_MANIFEST_FILENAME);
    fs::write(
        &manifest_path,
        encode_value_pretty_with_newline_error_string(&frozen_manifest)?,
    )
    .map_err(io_error)?;

    let mut checksum_members = frozen_artifacts
        .iter()
        .map(|artifact| {
            Ok((
                required_string_field(artifact, "path")?,
                required_string_field(artifact, "sha256")?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    checksum_members.extend(
        frozen_license_material
            .iter()
            .map(|material| {
                Ok((
                    required_string_field(material, "path")?,
                    required_string_field(material, "sha256")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?,
    );
    let manifest_bytes =
        read_bounded_file(&manifest_path, MAX_DOSSIER_BYTES, "Frozen family manifest")?;
    checksum_members.push((
        FAMILY_FROZEN_MANIFEST_FILENAME.to_string(),
        sha256_hex(&manifest_bytes),
    ));
    checksum_members.sort();
    let checksum_text = checksum_members
        .iter()
        .map(|(path, digest)| format!("{digest}  {path}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let checksum_path = staging_root.join(FAMILY_CHECKSUM_FILENAME);
    fs::write(&checksum_path, checksum_text).map_err(io_error)?;
    let checksum_bytes = read_bounded_file(
        &checksum_path,
        MAX_DOSSIER_BYTES,
        "Frozen family checksum manifest",
    )?;

    let final_prefix = format!(
        "{}/{FAMILY_FROZEN_DIRNAME}",
        family_release_relative_dir(release_id)
    );
    let mut build_artifacts = frozen_artifacts
        .iter()
        .map(|artifact| {
            let mut row = artifact.clone();
            let path = required_string_field(artifact, "path")?;
            row.as_object_mut()
                .ok_or_else(|| "Frozen artifact row must be an object.".to_string())?
                .insert("path".to_string(), json!(format!("{final_prefix}/{path}")));
            Ok(row)
        })
        .collect::<Result<Vec<_>, String>>()?;
    build_artifacts.extend(
        frozen_license_material
            .iter()
            .map(|material| {
                let mut row = material.clone();
                let path = required_string_field(material, "path")?;
                row.as_object_mut()
                    .ok_or_else(|| "Frozen license-material row must be an object.".to_string())?
                    .insert("path".to_string(), json!(format!("{final_prefix}/{path}")));
                Ok(row)
            })
            .collect::<Result<Vec<_>, String>>()?,
    );
    build_artifacts.push(json!({
        "role": "family-manifest",
        "kind": "manifest",
        "path": format!("{final_prefix}/{FAMILY_FROZEN_MANIFEST_FILENAME}"),
        "sha256": sha256_hex(&manifest_bytes),
        "size_bytes": manifest_bytes.len(),
    }));
    build_artifacts.push(json!({
        "role": "family-checksum",
        "kind": "checksum",
        "path": format!("{final_prefix}/{FAMILY_CHECKSUM_FILENAME}"),
        "sha256": sha256_hex(&checksum_bytes),
        "size_bytes": checksum_bytes.len(),
    }));
    build_artifacts.sort_by_key(|artifact| {
        (
            string_field(artifact, "role").unwrap_or_default(),
            string_field(artifact, "component").unwrap_or_default(),
            string_field(artifact, "source_repository").unwrap_or_default(),
            string_field(artifact, "kind").unwrap_or_default(),
            string_field(artifact, "material_role").unwrap_or_default(),
            string_field(artifact, "target"),
        )
    });
    let created_at = required_string_field(&candidate, "created_at")?;
    let build_record = json!({
        "contract": FAMILY_RELEASE_BUILD_CONTRACT,
        "command": "release build",
        "release_id": release_id,
        "repo_name": required_string_field(&candidate, "repo_name")?,
        "version": required_string_field(&candidate, "version")?,
        "channel": required_string_field(&candidate, "channel")?,
        "tag": required_string_field(&candidate, "tag")?,
        "line": required_string_field(&candidate, "line")?,
        "snapshot_id": required_string_field(&candidate, "snapshot_id")?,
        "manifest_hash": required_string_field(&candidate, "manifest_hash")?,
        "profile": FAMILY_RELEASE_PROFILE,
        "status": "built",
        "family_manifest_sha256": required_string_field(&candidate, "family_manifest_sha256")?,
        "family": candidate.get("family").cloned().unwrap_or_else(|| json!({})),
        "checks": admission.record.get("checks").cloned().unwrap_or_else(|| json!([])),
        "check_summary": admission.record.get("check_summary").cloned().unwrap_or_else(|| json!({})),
        "component_receipts": admission.record.get("component_receipts").cloned().unwrap_or_else(|| json!([])),
        "artifacts": build_artifacts,
        "frozen_manifest_path": format!("{final_prefix}/{FAMILY_FROZEN_MANIFEST_FILENAME}"),
        "checksum_path": format!("{final_prefix}/{FAMILY_CHECKSUM_FILENAME}"),
        "promotion": unpromoted_family_state(&candidate)?,
        "authority": candidate.get("authority").cloned().unwrap_or_else(|| json!({})),
        "created_at": created_at,
        "updated_at": created_at,
        "next_action": {
            "code": "prepare_protected_promotion",
            "detail": format!("Run `ait release promote {release_id} --channel {}` to verify and emit the protected CI promotion handoff.", required_string_field(&candidate, "channel")?),
        },
    });
    fs::write(
        staging_root.join(FAMILY_BUILD_FILENAME),
        encode_value_pretty_with_newline_error_string(&build_record)?,
    )
    .map_err(io_error)?;
    fs::rename(staging_root, &frozen_root).map_err(|error| {
        format!("Failed to atomically activate frozen family release {release_id}: {error}")
    })?;
    drop(staging);
    validate_family_build(repo, release_id, Some(&admission), public_source_root)
}

fn family_promotion_record(
    candidate: &JsonValue,
    build: &JsonValue,
    release_id: &str,
) -> Result<JsonValue, String> {
    let version = required_string_field(candidate, "version")?;
    let channel = required_string_field(candidate, "channel")?;
    let tag = required_string_field(candidate, "tag")?;
    validate_family_version(&version, &channel)?;
    let created_at = required_string_field(candidate, "created_at")?;
    Ok(json!({
        "contract": FAMILY_RELEASE_PROMOTION_CONTRACT,
        "command": "release promote",
        "release_id": release_id,
        "repo_name": required_string_field(candidate, "repo_name")?,
        "version": version,
        "channel": channel,
        "tag": tag,
        "line": required_string_field(candidate, "line")?,
        "snapshot_id": required_string_field(candidate, "snapshot_id")?,
        "manifest_hash": required_string_field(candidate, "manifest_hash")?,
        "profile": FAMILY_RELEASE_PROFILE,
        "status": "ready_for_protected_ci",
        "family_manifest_sha256": required_string_field(candidate, "family_manifest_sha256")?,
        "frozen_manifest_path": required_string_field(build, "frozen_manifest_path")?,
        "checksum_path": required_string_field(build, "checksum_path")?,
        "routes": family_promotion_routes(
            &required_string_field(candidate, "channel")?,
            &required_string_field(candidate, "version")?,
            &required_string_field(candidate, "tag")?,
            candidate_distributions(candidate)?,
        ),
        "authorization": {
            "required": true,
            "granted": false,
            "protected_environment_required": true,
            "public_source_readback_required": true,
            "snapshot_to_git_tree_equality_required": true,
            "binary_publication_before_source_allowed": false,
        },
        "source_publication": family_source_publication_requirements(candidate)?,
        "mutation": {
            "registry_write": false,
            "performed": false,
            "credentials_loaded": false,
            "rebuild_allowed": false,
        },
        "artifacts": build.get("artifacts").cloned().unwrap_or_else(|| json!([])),
        "created_at": created_at,
        "updated_at": created_at,
        "next_action": {
            "code": "approve_exact_frozen_digest",
            "detail": "Approve the exact frozen family manifest and checksum digest in protected CI, then promote the recorded bytes without rebuilding.",
        },
    }))
}

pub fn family_release_show(
    repo: &RepoRuntime,
    release_id: &str,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    let candidate = load_family_candidate(repo, release_id, public_source_root)?;
    let release_dir = family_release_dir(repo, release_id, false)?;
    let promotion_path = release_dir.join(FAMILY_PROMOTION_FILENAME);
    if promotion_path.exists() {
        let bytes = read_bounded_file(
            &promotion_path,
            MAX_DOSSIER_BYTES,
            "Family promotion handoff",
        )?;
        let promotion =
            parse_slice_value(&bytes, "Family promotion handoff must contain valid JSON")?;
        let build = validate_family_build(repo, release_id, None, public_source_root)?;
        if promotion != family_promotion_record(&candidate, &build, release_id)? {
            return Err(
                "Family promotion handoff does not match its verified frozen build.".to_string(),
            );
        }
        return Ok(promotion);
    }
    let frozen_root = release_dir.join(FAMILY_FROZEN_DIRNAME);
    if frozen_root.exists() {
        return validate_family_build(repo, release_id, None, public_source_root);
    }
    let check_path = release_dir.join(FAMILY_CHECK_FILENAME);
    if check_path.exists() {
        let bytes = read_bounded_file(
            &check_path,
            MAX_DOSSIER_BYTES,
            "Family release check receipt",
        )?;
        let check = parse_slice_value(
            &bytes,
            "Family release check receipt must contain valid JSON",
        )?;
        if string_field(&check, "contract").as_deref() != Some(FAMILY_RELEASE_CHECK_CONTRACT)
            || string_field(&check, "release_id").as_deref() != Some(release_id)
        {
            return Err("Family release check receipt has an invalid identity.".to_string());
        }
        return Ok(check);
    }
    Ok(candidate)
}

pub fn family_release_promote(
    repo: &RepoRuntime,
    release_id: &str,
    requested_channel: &str,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    let candidate = load_family_candidate(repo, release_id, public_source_root)?;
    let channel = required_string_field(&candidate, "channel")?;
    if requested_channel.trim() != channel {
        return Err(format!(
            "Requested promotion channel {:?} does not match candidate channel {channel:?}.",
            requested_channel.trim()
        ));
    }
    let build = validate_family_build(repo, release_id, None, public_source_root)?;
    let promotion = family_promotion_record(&candidate, &build, release_id)?;
    let path = family_release_dir(repo, release_id, false)?.join(FAMILY_PROMOTION_FILENAME);
    write_json_once(&path, &promotion, "Family promotion handoff")?;
    Ok(promotion)
}

pub fn family_release_publish_error(release_id: &str) -> String {
    format!(
        "Family release {release_id} intentionally has no persisted AIT Remote Release authority. `ait release publish` keeps its historical AIT Remote meaning; use `ait release promote {release_id} --channel <rc|stable>` to emit the protected public-registry handoff."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_public_source() -> JsonValue {
        json!({
            "model": "release-monorepo",
            "identity": "weita2026/ait-native",
            "product_document": "docs/distribution.md",
            "family_manifest": "ait-release-family.json",
            "mapping_manifest": "ait-monorepo-source.json",
            "build_entrypoints": {
                "unix": "build-release.sh",
                "windows": "build-release.ps1",
                "implementation": "build-release.mjs",
            },
            "subtrees": [
                {
                    "source_repository": "ait-core",
                    "path": "ait-core",
                    "transforms": [],
                },
                {
                    "source_repository": "ait-python",
                    "path": "ait-python",
                    "transforms": ["python-core-path/v1"],
                },
            ],
            "transforms": [{
                "id": "python-core-path/v1",
                "source_repository": "ait-python",
                "path": "pyproject.toml",
                "from": ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml",
                "to": "../ait-core/rust/crates/ait-py/Cargo.toml",
            }],
        })
    }

    fn manifest(version: &str, channel: &str, python_version: &str) -> JsonValue {
        json!({
            "schema": FAMILY_RELEASE_MANIFEST_CONTRACT,
            "family": {
                "name": "ait-native",
                "version": version,
                "channel": channel,
                "tag": format!("v{version}"),
            },
            "targets": [
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-gnu",
                "aarch64-pc-windows-msvc",
                "x86_64-pc-windows-msvc"
            ],
            "public_source": fixture_public_source(),
            "components": [
                {
                    "id": "ait",
                    "source_repository": "ait-core",
                    "source_snapshot": "SNP-0123456789AB",
                    "ecosystem": "native",
                    "license": "Apache-2.0",
                    "version_scheme": "family",
                    "version": version,
                    "artifacts": [{
                        "kind": "native-executable",
                        "targets": [
                            "aarch64-apple-darwin",
                            "x86_64-apple-darwin",
                            "aarch64-unknown-linux-gnu",
                            "x86_64-unknown-linux-gnu",
                            "aarch64-pc-windows-msvc",
                            "x86_64-pc-windows-msvc"
                        ]
                    }]
                },
                {
                    "id": "ait-python",
                    "source_repository": "ait-python",
                    "source_snapshot": "SNP-ABCDEF012345",
                    "ecosystem": "python",
                    "license": "Apache-2.0",
                    "version_scheme": "pep440",
                    "version": python_version,
                    "artifacts": [{
                        "kind": "python-wheel",
                        "targets": [
                            "aarch64-apple-darwin",
                            "x86_64-apple-darwin",
                            "aarch64-unknown-linux-gnu",
                            "x86_64-unknown-linux-gnu",
                            "aarch64-pc-windows-msvc",
                            "x86_64-pc-windows-msvc"
                        ]
                    }]
                }
            ],
            "distributions": [
                {
                    "channel": "pypi",
                    "role": "product",
                    "identity": "ait-native",
                    "components": ["ait", "ait-python"],
                    "targets": [
                        "aarch64-apple-darwin",
                        "x86_64-apple-darwin",
                        "aarch64-unknown-linux-gnu",
                        "x86_64-unknown-linux-gnu",
                        "aarch64-pc-windows-msvc",
                        "x86_64-pc-windows-msvc"
                    ]
                },
                {
                    "channel": "github",
                    "role": "product",
                    "identity": "weita2026/ait-native",
                    "components": ["ait", "ait-python"],
                    "targets": [
                        "aarch64-apple-darwin",
                        "x86_64-apple-darwin",
                        "aarch64-unknown-linux-gnu",
                        "x86_64-unknown-linux-gnu",
                        "aarch64-pc-windows-msvc",
                        "x86_64-pc-windows-msvc"
                    ]
                }
            ],
            "compatibility": {
                "native_protocol": "ait-native/v1",
                "python_abi": "abi3"
            }
        })
    }

    #[test]
    fn rc_family_manifest_admits_exact_pep440_mapping_and_six_targets() {
        let parsed = parse_family_release_manifest(&manifest("1.0.0-rc.1", "rc", "1.0.0rc1"))
            .expect("valid RC family manifest");
        assert_eq!(parsed.family.version, "1.0.0-rc.1");
        assert_eq!(parsed.family.channel, "rc");
        assert_eq!(parsed.targets.len(), 6);
        assert_eq!(parsed.expected_artifact_count(), 12);
        assert_eq!(parsed.components[1].license, "Apache-2.0");
        assert_eq!(parsed.distributions[0].identity, "ait-native");
    }

    #[test]
    fn stable_family_manifest_uses_exact_versions() {
        let parsed = parse_family_release_manifest(&manifest("1.0.0", "stable", "1.0.0"))
            .expect("valid stable family manifest");
        assert_eq!(parsed.components[1].version, "1.0.0");
    }

    #[test]
    fn native_runner_bundle_gate_preserves_only_the_exact_published_legacy_family() {
        let bytes = include_bytes!("../../../../../release/families/1.1.0/ait-release-family.json");
        assert_eq!(
            sha256_hex(bytes),
            PUBLISHED_LEGACY_NATIVE_BUNDLE_MANIFEST_SHA256
        );
        let value: JsonValue = serde_json::from_slice(bytes).unwrap();
        let family = parse_family_release_manifest(&value).unwrap();
        validate_native_product_bundle_contract(
            &family,
            PUBLISHED_LEGACY_NATIVE_BUNDLE_SNAPSHOT,
            PUBLISHED_LEGACY_NATIVE_BUNDLE_MANIFEST_SHA256,
        )
        .unwrap();

        assert!(validate_native_product_bundle_contract(
            &family,
            "SNP-FFFFFFFFFFFF",
            PUBLISHED_LEGACY_NATIVE_BUNDLE_MANIFEST_SHA256,
        )
        .unwrap_err()
        .contains("must bundle ait, ait-server, and ait-runner"));

        let mut future = family.clone();
        future.family.version = "1.1.1".to_string();
        future.family.tag = "v1.1.1".to_string();
        assert!(validate_native_product_bundle_contract(
            &future,
            "SNP-111111111111",
            &"1".repeat(64),
        )
        .is_err());

        for distribution in &mut future.distributions {
            if distribution.role == "product"
                && matches!(distribution.channel.as_str(), "homebrew" | "apt" | "winget")
            {
                distribution.components.push("ait-runner".to_string());
            }
        }
        validate_native_product_bundle_contract(&future, "SNP-111111111111", &"1".repeat(64))
            .unwrap();

        future
            .distributions
            .retain(|distribution| distribution.channel != "winget");
        assert!(validate_native_product_bundle_contract(
            &future,
            "SNP-111111111111",
            &"1".repeat(64),
        )
        .unwrap_err()
        .contains("exactly one Homebrew, apt, and WinGet"));
        assert!(native_runner_bundle_required("1.1.0-rc.1").unwrap());
        assert!(!native_runner_bundle_required("1.0.9").unwrap());
    }

    #[test]
    fn family_manifest_represents_portable_artifact_as_one_targetless_key() {
        let mut value = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        value["components"][0]["artifacts"][0]["targets"] = json!([]);
        let parsed = parse_family_release_manifest(&value).expect("portable artifact manifest");
        assert_eq!(parsed.components[0].expected_artifact_keys().len(), 1);
        assert_eq!(parsed.expected_artifact_count(), 7);
    }

    #[test]
    fn family_manifest_rejects_channel_version_and_python_mapping_drift() {
        let error = parse_family_release_manifest(&manifest("1.0.0", "rc", "1.0.0rc1"))
            .expect_err("RC channel must require RC SemVer");
        assert!(error.contains("MAJOR.MINOR.PATCH-rc.N"));

        let error = parse_family_release_manifest(&manifest("1.0.0-rc.1", "rc", "1.0.0-rc.1"))
            .expect_err("Python version must use exact PEP 440 mapping");
        assert!(error.contains("pep440"));
    }

    #[test]
    fn family_manifest_rejects_artifact_target_outside_matrix() {
        let mut value = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        value["components"][0]["artifacts"][0]["targets"] = json!(["x86_64-unknown-freebsd"]);
        let error = parse_family_release_manifest(&value).expect_err("unknown target must fail");
        assert!(error.contains("absent from the family target matrix"));
    }

    #[test]
    fn family_manifest_rejects_license_and_distribution_drift() {
        let mut missing_license = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        missing_license["components"][1]
            .as_object_mut()
            .unwrap()
            .remove("license");
        assert!(parse_family_release_manifest(&missing_license)
            .unwrap_err()
            .contains("license"));

        let mut bad_license = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        bad_license["components"][1]["license"] = json!("Apache-2.0?");
        assert!(parse_family_release_manifest(&bad_license)
            .unwrap_err()
            .contains("SPDX"));

        let mut undeclared_component = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        undeclared_component["distributions"][0]["components"] = json!(["ait-server"]);
        assert!(parse_family_release_manifest(&undeclared_component)
            .unwrap_err()
            .contains("undeclared component"));

        let mut unsupported_target = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        unsupported_target["components"][1]["artifacts"][0]["targets"] =
            json!(["aarch64-apple-darwin"]);
        assert!(parse_family_release_manifest(&unsupported_target)
            .unwrap_err()
            .contains("no matching or portable artifact"));

        let mut duplicate = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        let row = duplicate["distributions"][0].clone();
        duplicate["distributions"].as_array_mut().unwrap().push(row);
        assert!(parse_family_release_manifest(&duplicate)
            .unwrap_err()
            .contains("duplicate channel/identity"));

        let mut missing_component_coverage = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        missing_component_coverage["distributions"][0]["components"] = json!(["ait"]);
        missing_component_coverage["distributions"][1]["components"] = json!(["ait"]);
        assert!(parse_family_release_manifest(&missing_component_coverage)
            .unwrap_err()
            .contains("does not distribute declared component"));

        let mut unknown_channel = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        unknown_channel["distributions"][0]["channel"] = json!("custom");
        assert!(parse_family_release_manifest(&unknown_channel)
            .unwrap_err()
            .contains("must be one of"));
    }

    #[test]
    fn family_manifest_rejects_identity_duplicates_unknown_fields_and_tag_drift() {
        let mut duplicate_target = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        let first_target = duplicate_target["targets"][0].clone();
        duplicate_target["targets"][1] = first_target;
        assert!(parse_family_release_manifest(&duplicate_target)
            .unwrap_err()
            .contains("duplicate value"));

        let mut duplicate_component = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        duplicate_component["components"][1]["id"] = json!("ait");
        assert!(parse_family_release_manifest(&duplicate_component)
            .unwrap_err()
            .contains("duplicate component id"));

        let mut bad_snapshot = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        bad_snapshot["components"][0]["source_snapshot"] = json!("main");
        assert!(parse_family_release_manifest(&bad_snapshot)
            .unwrap_err()
            .contains("Snapshot identity"));

        let mut unknown = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        unknown["registry"] = json!("latest");
        assert!(parse_family_release_manifest(&unknown)
            .unwrap_err()
            .contains("unknown field"));

        let mut bad_tag = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        bad_tag["family"]["tag"] = json!("v1.0.0");
        assert!(parse_family_release_manifest(&bad_tag)
            .unwrap_err()
            .contains("must be exactly"));
    }

    #[test]
    fn family_manifest_rejects_legacy_or_ambiguous_public_source_contracts() {
        let mut legacy = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        legacy["schema"] = json!("ait.release.family/v2");
        assert!(parse_family_release_manifest(&legacy)
            .unwrap_err()
            .contains("ait.release.family/v3"));

        let mut missing_subtree = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        missing_subtree["public_source"]["subtrees"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        assert!(parse_family_release_manifest(&missing_subtree)
            .unwrap_err()
            .contains("map each"));

        let mut multiple_github = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        let mut second = multiple_github["distributions"][1].clone();
        second["identity"] = json!("weita2026/ait-core");
        multiple_github["distributions"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(parse_family_release_manifest(&multiple_github)
            .unwrap_err()
            .contains("exactly one GitHub"));

        let mut undeclared_transform = manifest("1.0.0-rc.1", "rc", "1.0.0rc1");
        undeclared_transform["public_source"]["transforms"][0]["to"] = json!("../../ait-core");
        assert!(parse_family_release_manifest(&undeclared_transform)
            .unwrap_err()
            .contains("does not match the exact allowlisted"));
    }

    #[test]
    fn rc_promotion_routes_keep_every_mutable_default_out_of_latest() {
        let definition = parse_family_release_manifest(&manifest("1.0.0-rc.1", "rc", "1.0.0rc1"))
            .unwrap()
            .to_json();
        let routes = family_promotion_routes(
            "rc",
            "1.0.0-rc.1",
            "v1.0.0-rc.1",
            &definition["distributions"],
        );
        assert_eq!(routes["github"]["prerelease"], json!(true));
        assert_eq!(routes["github"]["draft"], json!(false));
        assert_eq!(routes["npm"]["dist_tag"], json!("rc"));
        assert_eq!(routes["pypi"]["prerelease"], json!(true));
        assert_eq!(routes["oci"]["moving_tag"], json!("rc"));
        assert_eq!(routes["homebrew"]["channel"], json!("rc"));
        assert_eq!(routes["homebrew"]["stable_formula_mutation"], json!(false));
        assert_eq!(routes["apt"]["suite"], json!("testing"));
        assert_eq!(routes["winget"]["route"], json!("validation"));
        assert_eq!(
            routes["winget"]["community_manifest_submission"],
            json!(false)
        );
        assert_eq!(routes["distributions"][0]["identity"], json!("ait-native"));
    }

    #[test]
    fn stable_promotion_routes_select_only_stable_channels() {
        let definition = parse_family_release_manifest(&manifest("1.0.0", "stable", "1.0.0"))
            .unwrap()
            .to_json();
        let routes =
            family_promotion_routes("stable", "1.0.0", "v1.0.0", &definition["distributions"]);
        assert_eq!(routes["github"]["prerelease"], json!(false));
        assert_eq!(routes["npm"]["dist_tag"], json!("latest"));
        assert_eq!(routes["oci"]["moving_tag"], json!("latest"));
        assert_eq!(routes["homebrew"]["stable_formula_mutation"], json!(true));
        assert_eq!(routes["apt"]["suite"], json!("stable"));
        assert_eq!(routes["winget"]["route"], json!("community"));
        assert_eq!(
            routes["winget"]["community_manifest_submission"],
            json!(true)
        );
    }
}
