use super::*;

pub(super) const GENERIC_RELEASE_PROFILE: &str = "generic-command";
pub(super) const GENERIC_RELEASE_ADAPTER_CONTRACT: &str = "ait.release.adapter/v1";
pub(super) const GENERIC_RELEASE_RECEIPT_CONTRACT: &str = "ait.release.adapter.receipt/v1";
pub(super) const PUBLIC_GIT_RELEASE_RECEIPT_CONTRACT: &str = "ait.release.public-git.receipt/v1";
pub(super) const GENERIC_RELEASE_MANIFEST_PATH: &str = "ait-release.json";
pub(super) const GENERIC_RELEASE_BUILDER: &str = "ait_generic_command_adapter_v1";
pub(super) const PUBLIC_GIT_RELEASE_BUILDER: &str = "ait_public_git_adapter_v1";
const GENERIC_PORTABLE_SELECTION: &str = "portable";
const GENERIC_RELEASE_ARGUMENT_TOKENS: [&str; 6] = [
    "$AIT_RELEASE_ID",
    "$AIT_RELEASE_VERSION",
    "$AIT_RELEASE_COMPONENT",
    "$AIT_RELEASE_ECOSYSTEM",
    "$AIT_RELEASE_TARGET",
    "$SOURCE_DATE_EPOCH",
];

const MAX_COMPONENTS: usize = 64;
const MAX_DEPENDENCY_FILES: usize = 64;
const MAX_LICENSE_FILES: usize = 8;
const MAX_COMMANDS_PER_PHASE: usize = 16;
const MAX_ARGUMENTS_PER_COMMAND: usize = 64;
const MAX_ARTIFACTS_PER_COMPONENT: usize = 128;
const MAX_TEXT_BYTES: usize = 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_COMMAND_OUTPUT_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GenericReleasePackage {
    name: String,
    version: String,
    description: Option<String>,
    license_files: Vec<GenericReleaseLicenseFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GenericReleaseLicenseFile {
    path: String,
    role: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GenericReleaseArtifact {
    path: String,
    kind: String,
    target: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GenericReleaseComponent {
    id: String,
    ecosystem: String,
    working_directory: String,
    dependency_files: Vec<String>,
    prepare_commands: Vec<Vec<String>>,
    test_commands: Vec<Vec<String>>,
    build_commands: Vec<Vec<String>>,
    smoke_commands: Vec<Vec<String>>,
    artifacts: Vec<GenericReleaseArtifact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GenericReleaseAdapter {
    package: GenericReleasePackage,
    components: Vec<GenericReleaseComponent>,
}

#[derive(Debug)]
struct GenericCommandFailure {
    detail: String,
    evidence: Vec<JsonValue>,
}

impl GenericReleasePackage {
    fn to_manifest_json(&self) -> JsonValue {
        let mut value = json!({
            "name": self.name,
            "version": self.version,
            "description": self.description,
        });
        if !self.license_files.is_empty() {
            value
                .as_object_mut()
                .expect("generic package manifest must be an object")
                .insert(
                    "license_files".to_string(),
                    json!(self
                        .license_files
                        .iter()
                        .map(GenericReleaseLicenseFile::to_json)
                        .collect::<Vec<_>>()),
                );
        }
        value
    }

    fn to_package_json(&self) -> JsonValue {
        json!({
            "name": self.name,
            "version": self.version,
            "description": self.description,
            "requires_python": JsonValue::Null,
            "license_files": self.license_files.iter().map(GenericReleaseLicenseFile::to_json).collect::<Vec<_>>(),
            "adapter_contract": GENERIC_RELEASE_ADAPTER_CONTRACT,
        })
    }
}

impl GenericReleaseLicenseFile {
    fn to_json(&self) -> JsonValue {
        json!({
            "path": self.path,
            "role": self.role,
        })
    }
}

impl GenericReleaseArtifact {
    fn to_json(&self) -> JsonValue {
        let mut value = json!({
            "path": self.path,
            "kind": self.kind,
        });
        if let Some(target) = &self.target {
            if let Some(object) = value.as_object_mut() {
                object.insert("target".to_string(), json!(target));
            }
        }
        value
    }
}

impl GenericReleaseComponent {
    fn to_json(&self) -> JsonValue {
        json!({
            "id": self.id,
            "ecosystem": self.ecosystem,
            "working_directory": self.working_directory,
            "dependency_files": self.dependency_files,
            "commands": {
                "prepare": self.prepare_commands,
                "test": self.test_commands,
                "build": self.build_commands,
                "smoke": self.smoke_commands,
            },
            "artifacts": self.artifacts.iter().map(GenericReleaseArtifact::to_json).collect::<Vec<_>>(),
        })
    }
}

impl GenericReleaseAdapter {
    pub(super) fn to_json(&self) -> JsonValue {
        json!({
            "schema": GENERIC_RELEASE_ADAPTER_CONTRACT,
            "package": self.package.to_manifest_json(),
            "components": self.components.iter().map(GenericReleaseComponent::to_json).collect::<Vec<_>>(),
        })
    }

    fn declared_artifact_count(&self) -> usize {
        self.components
            .iter()
            .map(|component| component.artifacts.len())
            .sum()
    }

    fn selected_components(&self, target: Option<&str>) -> Vec<&GenericReleaseComponent> {
        self.components
            .iter()
            .filter(|component| {
                component
                    .artifacts
                    .iter()
                    .any(|artifact| generic_artifact_selected(artifact, target))
            })
            .collect()
    }

    fn selected_artifact_count(&self, target: Option<&str>) -> usize {
        self.components
            .iter()
            .flat_map(|component| &component.artifacts)
            .filter(|artifact| generic_artifact_selected(artifact, target))
            .count()
    }
}

fn generic_artifact_selected(artifact: &GenericReleaseArtifact, target: Option<&str>) -> bool {
    match target {
        Some(GENERIC_PORTABLE_SELECTION) => artifact.target.is_none(),
        Some(target) => artifact.target.as_deref() == Some(target),
        None => true,
    }
}

fn normalize_generic_target(target: Option<&str>) -> Result<Option<String>, String> {
    let Some(target) = normalized_text(target) else {
        return Ok(None);
    };
    let value = json!(target);
    require_identifier(Some(&value), "release adapter target").map(Some)
}

fn require_generic_target_selection(
    adapter: &GenericReleaseAdapter,
    target: Option<&str>,
) -> Result<(), String> {
    if let Some(target) = target {
        if adapter.selected_artifact_count(Some(target)) == 0 {
            return Err(format!(
                "Release adapter target {target:?} has no declared artifacts in {GENERIC_RELEASE_MANIFEST_PATH}."
            ));
        }
    }
    Ok(())
}

fn generic_record_selection(record: &JsonValue) -> Result<Option<String>, String> {
    let target = normalize_generic_target(string_field(record, "target").as_deref())?;
    let portable = match record.get("artifact_selection") {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::String(value)) if value == GENERIC_PORTABLE_SELECTION => true,
        Some(_) => {
            return Err(format!(
                "Generic release receipt artifact_selection must be {GENERIC_PORTABLE_SELECTION:?} when declared."
            ))
        }
    };
    if portable && target.is_some() {
        return Err(
            "Generic release receipt cannot combine portable and target artifact selectors."
                .to_string(),
        );
    }
    if !portable && target.as_deref() == Some(GENERIC_PORTABLE_SELECTION) {
        return Err(
            "Generic release receipt must encode portable selection through artifact_selection, not target."
                .to_string(),
        );
    }
    Ok(if portable {
        Some(GENERIC_PORTABLE_SELECTION.to_string())
    } else {
        target
    })
}

pub(super) fn is_generic_release_profile(profile: &str) -> bool {
    profile.trim().eq_ignore_ascii_case(GENERIC_RELEASE_PROFILE)
}

pub(super) fn is_generic_release_record(record: &JsonValue) -> bool {
    string_field(record, "profile")
        .map(|profile| is_generic_release_profile(&profile))
        .unwrap_or(false)
}

pub(super) fn generic_release_profile_settings() -> JsonValue {
    json!({
        "adapter_contract": GENERIC_RELEASE_ADAPTER_CONTRACT,
        "manifest_path": GENERIC_RELEASE_MANIFEST_PATH,
        "command_execution": "direct_argv_without_implicit_shell",
        "argument_resolution": "exact_whole_argument_tokens_only",
        "argument_tokens": GENERIC_RELEASE_ARGUMENT_TOKENS,
        "artifact_selection": "exact_declared_regular_files",
        "registry_publish": false,
    })
}

fn require_object<'a>(
    value: &'a JsonValue,
    context: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{context} must be a JSON object."))
}

fn require_known_fields(
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

fn require_text(value: Option<&JsonValue>, context: &str) -> Result<String, String> {
    let text = value
        .and_then(JsonValue::as_str)
        .ok_or_else(|| format!("{context} must be a string."))?;
    if text.is_empty()
        || text.trim() != text
        || text.len() > MAX_TEXT_BYTES
        || text.chars().any(char::is_control)
    {
        return Err(format!(
            "{context} must be a non-empty, bounded single-line string without surrounding whitespace."
        ));
    }
    Ok(text.to_string())
}

fn optional_text(value: Option<&JsonValue>, context: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => require_text(Some(value), context).map(Some),
    }
}

fn require_identifier(value: Option<&JsonValue>, context: &str) -> Result<String, String> {
    let text = require_text(value, context)?;
    if text.len() > 64
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

fn require_relative_path(
    value: Option<&JsonValue>,
    context: &str,
    allow_dot: bool,
) -> Result<String, String> {
    let text = require_text(value, context)?;
    if allow_dot && text == "." {
        return Ok(text);
    }
    if Path::new(&text).is_absolute()
        || text.contains('\\')
        || text.contains(':')
        || text
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!(
            "{context} must be a normalized '/'-separated relative path without traversal, drive prefixes, or backslashes."
        ));
    }
    Ok(text)
}

fn require_relative_path_list(
    value: Option<&JsonValue>,
    context: &str,
) -> Result<Vec<String>, String> {
    let rows = value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context} must be an array."))?;
    if rows.is_empty() || rows.len() > MAX_DEPENDENCY_FILES {
        return Err(format!(
            "{context} must contain between 1 and {MAX_DEPENDENCY_FILES} paths."
        ));
    }
    let mut values = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let path = require_relative_path(Some(row), &format!("{context}[{index}]"), false)?;
        if !seen.insert(path.clone()) {
            return Err(format!("{context} contains duplicate path {path:?}."));
        }
        values.push(path);
    }
    Ok(values)
}

fn parse_generic_license_files(
    value: Option<&JsonValue>,
    context: &str,
) -> Result<Vec<GenericReleaseLicenseFile>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{context} must be an array."))?;
    if rows.is_empty() || rows.len() > MAX_LICENSE_FILES {
        return Err(format!(
            "{context} must contain between 1 and {MAX_LICENSE_FILES} rows when declared."
        ));
    }
    let mut result = Vec::with_capacity(rows.len());
    let mut paths = BTreeSet::new();
    let mut roles = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let row_context = format!("{context}[{index}]");
        let object = require_object(row, &row_context)?;
        require_known_fields(object, &["path", "role"], &row_context)?;
        let path =
            require_relative_path(object.get("path"), &format!("{row_context}.path"), false)?;
        let role = require_identifier(object.get("role"), &format!("{row_context}.role"))?;
        if !matches!(role.as_str(), "license" | "notice") {
            return Err(format!(
                "{row_context}.role must be exactly \"license\" or \"notice\"."
            ));
        }
        if !paths.insert(path.clone()) {
            return Err(format!("{context} contains duplicate path {path:?}."));
        }
        if !roles.insert(role.clone()) {
            return Err(format!("{context} contains duplicate role {role:?}."));
        }
        result.push(GenericReleaseLicenseFile { path, role });
    }
    result.sort_by(|left, right| {
        left.role
            .cmp(&right.role)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(result)
}

fn command_phase(
    commands: &JsonMap<String, JsonValue>,
    phase: &str,
    context: &str,
    required: bool,
) -> Result<Vec<Vec<String>>, String> {
    let Some(value) = commands.get(phase) else {
        if required {
            return Err(format!("{context}.{phase} must be declared."));
        }
        return Ok(Vec::new());
    };
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{context}.{phase} must be an array of argv arrays."))?;
    if (required && rows.is_empty()) || rows.len() > MAX_COMMANDS_PER_PHASE {
        return Err(format!(
            "{context}.{phase} must contain {} to {MAX_COMMANDS_PER_PHASE} commands.",
            if required { 1 } else { 0 }
        ));
    }
    let mut result = Vec::with_capacity(rows.len());
    for (command_index, row) in rows.iter().enumerate() {
        let argv = row.as_array().ok_or_else(|| {
            format!("{context}.{phase}[{command_index}] must be an argv array, not a shell string.")
        })?;
        if argv.is_empty() || argv.len() > MAX_ARGUMENTS_PER_COMMAND {
            return Err(format!(
                "{context}.{phase}[{command_index}] must contain between 1 and {MAX_ARGUMENTS_PER_COMMAND} argv values."
            ));
        }
        let mut command = Vec::with_capacity(argv.len());
        for (argument_index, value) in argv.iter().enumerate() {
            let argument = value.as_str().ok_or_else(|| {
                format!("{context}.{phase}[{command_index}][{argument_index}] must be a string.")
            })?;
            if argument.len() > MAX_TEXT_BYTES
                || argument.contains('\0')
                || argument
                    .chars()
                    .any(|character| matches!(character, '\n' | '\r'))
                || (argument_index == 0 && argument.trim().is_empty())
            {
                return Err(format!(
                    "{context}.{phase}[{command_index}][{argument_index}] is not a bounded single-line argv value."
                ));
            }
            command.push(argument.to_string());
        }
        result.push(command);
    }
    Ok(result)
}

fn parse_generic_artifacts(
    value: Option<&JsonValue>,
    context: &str,
) -> Result<Vec<GenericReleaseArtifact>, String> {
    let rows = value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context} must be an array."))?;
    if rows.is_empty() || rows.len() > MAX_ARTIFACTS_PER_COMPONENT {
        return Err(format!(
            "{context} must contain between 1 and {MAX_ARTIFACTS_PER_COMPONENT} artifacts."
        ));
    }
    let mut artifacts = Vec::with_capacity(rows.len());
    let mut seen_paths = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let row_context = format!("{context}[{index}]");
        let object = require_object(row, &row_context)?;
        require_known_fields(object, &["path", "kind", "target"], &row_context)?;
        let path =
            require_relative_path(object.get("path"), &format!("{row_context}.path"), false)?;
        let kind = require_identifier(object.get("kind"), &format!("{row_context}.kind"))?;
        let target = match object.get("target") {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(require_identifier(
                Some(value),
                &format!("{row_context}.target"),
            )?),
        };
        if !seen_paths.insert(path.clone()) {
            return Err(format!(
                "{context} contains duplicate artifact path {path:?}."
            ));
        }
        artifacts.push(GenericReleaseArtifact { path, kind, target });
    }
    Ok(artifacts)
}

pub(super) fn parse_generic_release_adapter(
    value: &JsonValue,
) -> Result<GenericReleaseAdapter, String> {
    let root = require_object(value, GENERIC_RELEASE_MANIFEST_PATH)?;
    require_known_fields(
        root,
        &["schema", "package", "components"],
        GENERIC_RELEASE_MANIFEST_PATH,
    )?;
    let schema = require_text(root.get("schema"), "ait-release.json.schema")?;
    if schema != GENERIC_RELEASE_ADAPTER_CONTRACT {
        return Err(format!(
            "ait-release.json.schema must be {GENERIC_RELEASE_ADAPTER_CONTRACT:?}, got {schema:?}."
        ));
    }

    let package_object = require_object(
        root.get("package")
            .ok_or_else(|| "ait-release.json.package must be declared.".to_string())?,
        "ait-release.json.package",
    )?;
    require_known_fields(
        package_object,
        &["name", "version", "description", "license_files"],
        "ait-release.json.package",
    )?;
    let package = GenericReleasePackage {
        name: require_text(package_object.get("name"), "ait-release.json.package.name")?,
        version: require_text(
            package_object.get("version"),
            "ait-release.json.package.version",
        )?,
        description: optional_text(
            package_object.get("description"),
            "ait-release.json.package.description",
        )?,
        license_files: parse_generic_license_files(
            package_object.get("license_files"),
            "ait-release.json.package.license_files",
        )?,
    };

    let rows = root
        .get("components")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "ait-release.json.components must be an array.".to_string())?;
    if rows.is_empty() || rows.len() > MAX_COMPONENTS {
        return Err(format!(
            "ait-release.json.components must contain between 1 and {MAX_COMPONENTS} components."
        ));
    }
    let mut components = Vec::with_capacity(rows.len());
    let mut component_ids = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let context = format!("ait-release.json.components[{index}]");
        let object = require_object(row, &context)?;
        require_known_fields(
            object,
            &[
                "id",
                "ecosystem",
                "working_directory",
                "dependency_files",
                "commands",
                "artifacts",
            ],
            &context,
        )?;
        let id = require_identifier(object.get("id"), &format!("{context}.id"))?;
        if !component_ids.insert(id.clone()) {
            return Err(format!(
                "ait-release.json contains duplicate component id {id:?}."
            ));
        }
        let ecosystem =
            require_identifier(object.get("ecosystem"), &format!("{context}.ecosystem"))?;
        let working_directory = require_relative_path(
            object.get("working_directory"),
            &format!("{context}.working_directory"),
            true,
        )?;
        let dependency_files = require_relative_path_list(
            object.get("dependency_files"),
            &format!("{context}.dependency_files"),
        )?;
        let commands_context = format!("{context}.commands");
        let commands = require_object(
            object
                .get("commands")
                .ok_or_else(|| format!("{commands_context} must be declared."))?,
            &commands_context,
        )?;
        require_known_fields(
            commands,
            &["prepare", "test", "build", "smoke"],
            &commands_context,
        )?;
        let component = GenericReleaseComponent {
            id,
            ecosystem,
            working_directory,
            dependency_files,
            prepare_commands: command_phase(commands, "prepare", &commands_context, false)?,
            test_commands: command_phase(commands, "test", &commands_context, true)?,
            build_commands: command_phase(commands, "build", &commands_context, true)?,
            smoke_commands: command_phase(commands, "smoke", &commands_context, false)?,
            artifacts: parse_generic_artifacts(
                object.get("artifacts"),
                &format!("{context}.artifacts"),
            )?,
        };
        components.push(component);
    }
    Ok(GenericReleaseAdapter {
        package,
        components,
    })
}

pub(super) fn generic_release_adapter_from_bundle(
    bundle: &ReleaseBundle,
) -> Result<GenericReleaseAdapter, String> {
    let entry = bundle
        .files
        .get(GENERIC_RELEASE_MANIFEST_PATH)
        .ok_or_else(|| {
            format!(
                "Release source snapshot is missing required file: {GENERIC_RELEASE_MANIFEST_PATH}"
            )
        })?;
    if entry.data.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "{GENERIC_RELEASE_MANIFEST_PATH} exceeds the {MAX_MANIFEST_BYTES}-byte contract limit."
        ));
    }
    let value = parse_slice_value(
        &entry.data,
        "ait-release.json must contain valid UTF-8 JSON",
    )?;
    let adapter = parse_generic_release_adapter(&value)?;
    validate_generic_dependency_files(&adapter, bundle)?;
    validate_generic_license_files(&adapter, bundle)?;
    Ok(adapter)
}

fn component_relative_path(component: &GenericReleaseComponent, path: &str) -> String {
    if component.working_directory == "." {
        path.to_string()
    } else {
        format!("{}/{path}", component.working_directory)
    }
}

fn validate_generic_dependency_files(
    adapter: &GenericReleaseAdapter,
    bundle: &ReleaseBundle,
) -> Result<(), String> {
    let mut missing = Vec::new();
    for component in &adapter.components {
        for path in &component.dependency_files {
            let source_path = component_relative_path(component, path);
            if !bundle.files.contains_key(&source_path) {
                missing.push(format!("{}:{source_path}", component.id));
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Generic release dependency-authority files are missing from the selected Snapshot: {}.",
            missing.join(", ")
        ))
    }
}

fn validate_generic_license_files(
    adapter: &GenericReleaseAdapter,
    bundle: &ReleaseBundle,
) -> Result<(), String> {
    let missing = adapter
        .package
        .license_files
        .iter()
        .filter(|file| !bundle.files.contains_key(&file.path))
        .map(|file| format!("{}:{}", file.role, file.path))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Generic release license-material files are missing from the selected Snapshot: {}.",
            missing.join(", ")
        ))
    }
}

fn generic_manifest_sha256(bundle: &ReleaseBundle) -> Result<String, String> {
    bundle
        .files
        .get(GENERIC_RELEASE_MANIFEST_PATH)
        .map(|entry| sha256_hex(&entry.data))
        .ok_or_else(|| {
            format!(
                "Release source snapshot is missing required file: {GENERIC_RELEASE_MANIFEST_PATH}"
            )
        })
}

fn generic_release_adapter_metadata(
    adapter: &GenericReleaseAdapter,
    bundle: &ReleaseBundle,
) -> Result<JsonValue, String> {
    Ok(json!({
        "contract": GENERIC_RELEASE_ADAPTER_CONTRACT,
        "profile": GENERIC_RELEASE_PROFILE,
        "manifest_path": GENERIC_RELEASE_MANIFEST_PATH,
        "manifest_sha256": generic_manifest_sha256(bundle)?,
        "component_count": adapter.components.len(),
        "declared_artifact_count": adapter.declared_artifact_count(),
        "license_material_count": adapter.package.license_files.len(),
        "definition": adapter.to_json(),
    }))
}

fn validate_generic_release_identity(
    record: &JsonValue,
    adapter: &GenericReleaseAdapter,
    bundle: &ReleaseBundle,
) -> Result<(), String> {
    if !is_generic_release_record(record) {
        return Err("Release candidate does not use the generic-command profile.".to_string());
    }
    let version = required_string_field(record, "version")?;
    if version != adapter.package.version {
        return Err(format!(
            "Release version {version:?} does not match ait-release.json package version {:?}.",
            adapter.package.version
        ));
    }
    if let Some(value) = record.get("target") {
        if !value.is_null() && value.as_str().is_none() {
            return Err("Generic release receipt has an invalid target selector.".to_string());
        }
    }
    let target = generic_record_selection(record)?;
    require_generic_target_selection(adapter, target.as_deref())?;
    let metadata = record
        .get("metadata")
        .and_then(|value| value.get("release_adapter"))
        .ok_or_else(|| {
            "Generic release candidate is missing release_adapter metadata.".to_string()
        })?;
    if string_field(metadata, "contract").as_deref() != Some(GENERIC_RELEASE_ADAPTER_CONTRACT)
        || string_field(metadata, "manifest_path").as_deref() != Some(GENERIC_RELEASE_MANIFEST_PATH)
        || string_field(metadata, "manifest_sha256").as_deref()
            != Some(generic_manifest_sha256(bundle)?.as_str())
        || metadata.get("definition") != Some(&adapter.to_json())
    {
        return Err(
            "Generic release adapter identity no longer matches the candidate-bound Snapshot manifest."
                .to_string(),
        );
    }
    Ok(())
}

fn generic_release_adapter_record(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    requested_target: Option<&str>,
) -> Result<JsonValue, String> {
    let line = release_local_line_row(repo, line_name)?;
    let snapshot_id = string_field(&line, "head_snapshot_id")
        .ok_or_else(|| format!("Line {line_name} does not have a head snapshot yet."))?;
    let bundle = release_snapshot_bundle(repo, &snapshot_id)?;
    let adapter = generic_release_adapter_from_bundle(&bundle)?;
    if version.trim() != adapter.package.version {
        return Err(format!(
            "Requested release version {version:?} does not match ait-release.json package version {:?}.",
            adapter.package.version
        ));
    }
    let target = normalize_generic_target(requested_target)?;
    require_generic_target_selection(&adapter, target.as_deref())?;
    let manifest_hash = required_string_field(&bundle.raw, "manifest_hash")?;
    let manifest_sha256 = generic_manifest_sha256(&bundle)?;
    let legacy_release_identity = format!(
        "{}\0{}\0{}\0{}\0{}",
        repo.repo_name(),
        line_name.trim(),
        snapshot_id,
        version.trim(),
        manifest_sha256
    );
    let release_identity = target
        .as_deref()
        .map(|target| format!("{legacy_release_identity}\0target\0{target}"))
        .unwrap_or(legacy_release_identity);
    let release_id = format!(
        "REL-GEN-{}",
        sha256_hex(release_identity.as_bytes())[..16].to_ascii_uppercase()
    );
    let created_at = bundle
        .raw
        .get("created_at")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .unwrap_or_else(current_timestamp);
    let package = adapter.package.to_package_json();
    let mut metadata = json!({
        "package": package.clone(),
        "profile": GENERIC_RELEASE_PROFILE,
        "profile_settings": generic_release_profile_settings(),
        "source_snapshot_created_at": bundle.raw.get("created_at").cloned().unwrap_or(JsonValue::Null),
        "release_adapter": generic_release_adapter_metadata(&adapter, &bundle)?,
    });
    if let Some(target) = &target {
        let field = if target == GENERIC_PORTABLE_SELECTION {
            "artifact_selection"
        } else {
            "target"
        };
        metadata
            .as_object_mut()
            .ok_or_else(|| "release metadata must be a JSON object".to_string())?
            .insert(field.to_string(), json!(target));
    }
    if let Some(external_closure) = release_external_closure_from_bundle(&bundle)? {
        metadata
            .as_object_mut()
            .ok_or_else(|| "release metadata must be a JSON object".to_string())?
            .insert("external_closure".to_string(), external_closure);
    }
    let mut record = json!({
        "contract": GENERIC_RELEASE_RECEIPT_CONTRACT,
        "release_id": release_id,
        "repo_name": repo.repo_name(),
        "version": version.trim(),
        "line": line_name.trim(),
        "line_name": line_name.trim(),
        "snapshot_id": snapshot_id,
        "manifest_hash": manifest_hash,
        "profile": GENERIC_RELEASE_PROFILE,
        "package_name": adapter.package.name,
        "package_version": adapter.package.version,
        "package_requires_python": JsonValue::Null,
        "package": package,
        "status": "candidate",
        "checks": [],
        "artifacts": [],
        "formula": {},
        "metadata": metadata,
        "authority": {
            "source": "selected_snapshot",
            "persistence": "none",
            "local_release_authority": "not_activated",
            "remote_publish_supported": false,
        },
        "created_at": created_at,
        "updated_at": created_at,
    });
    if let Some(target) = target {
        let field = if target == GENERIC_PORTABLE_SELECTION {
            "artifact_selection"
        } else {
            "target"
        };
        record
            .as_object_mut()
            .ok_or_else(|| "Generic release receipt must be an object.".to_string())?
            .insert(field.to_string(), json!(target));
    }
    Ok(record)
}

fn spawn_bounded_command_stream<R>(
    mut reader: R,
) -> std::thread::JoinHandle<Result<(String, bool), String>>
where
    R: io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut retained = Vec::with_capacity(MAX_COMMAND_OUTPUT_BYTES);
        let mut buffer = [0_u8; 4096];
        let mut truncated = false;
        loop {
            let read = reader.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            let remaining = MAX_COMMAND_OUTPUT_BYTES.saturating_sub(retained.len());
            let keep = remaining.min(read);
            retained.extend_from_slice(&buffer[..keep]);
            truncated |= keep < read;
        }
        Ok((
            String::from_utf8_lossy(&retained).trim().to_string(),
            truncated,
        ))
    })
}

fn bounded_command_output(
    stdout: (String, bool),
    stderr: (String, bool),
    fallback: &str,
) -> String {
    let (stdout, stdout_truncated) = stdout;
    let (stderr, stderr_truncated) = stderr;
    let mut sections = Vec::new();
    if !stdout.is_empty() {
        sections.push(stdout);
    }
    if !stderr.is_empty() {
        sections.push(format!("stderr:\n{stderr}"));
    }
    let mut text = if sections.is_empty() {
        fallback.to_string()
    } else {
        sections.join("\n")
    };
    if stdout_truncated || stderr_truncated {
        text.push_str("\n...[output truncated by ait generic release adapter]");
    }
    text
}

fn generic_component_working_directory(
    source_dir: &Path,
    component: &GenericReleaseComponent,
) -> Result<PathBuf, String> {
    let working_directory = if component.working_directory == "." {
        source_dir.to_path_buf()
    } else {
        source_dir.join(&component.working_directory)
    };
    let metadata = fs::symlink_metadata(&working_directory).map_err(|error| {
        format!(
            "Component {} working directory is missing from the Snapshot export: {} ({error}).",
            component.id, component.working_directory
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "Component {} working directory must be a real directory inside the Snapshot export: {}.",
            component.id, component.working_directory
        ));
    }
    let canonical_source = source_dir.canonicalize().map_err(io_error)?;
    let canonical_working = working_directory.canonicalize().map_err(io_error)?;
    if !canonical_working.starts_with(canonical_source) {
        return Err(format!(
            "Component {} working directory escapes the isolated Snapshot export: {}.",
            component.id, component.working_directory
        ));
    }
    Ok(working_directory)
}

fn resolve_generic_command_argv(
    argv: &[String],
    component: &GenericReleaseComponent,
    record: &JsonValue,
    source_date_epoch: i64,
) -> Result<Vec<String>, String> {
    let release_id = required_string_field(record, "release_id")?;
    let version = required_string_field(record, "version")?;
    let target = generic_record_selection(record)?;
    argv.iter()
        .map(|argument| {
            Ok(match argument.as_str() {
                "$AIT_RELEASE_ID" => release_id.clone(),
                "$AIT_RELEASE_VERSION" => version.clone(),
                "$AIT_RELEASE_COMPONENT" => component.id.clone(),
                "$AIT_RELEASE_ECOSYSTEM" => component.ecosystem.clone(),
                "$AIT_RELEASE_TARGET" => target.clone().ok_or_else(|| {
                    "Direct release argv uses $AIT_RELEASE_TARGET without an exact target or portable selection."
                        .to_string()
                })?,
                "$SOURCE_DATE_EPOCH" => source_date_epoch.to_string(),
                _ => argument.clone(),
            })
        })
        .collect()
}

fn run_generic_commands(
    source_dir: &Path,
    component: &GenericReleaseComponent,
    record: &JsonValue,
    source_date_epoch: i64,
    phase: &str,
    commands: &[Vec<String>],
    mut evidence: Vec<JsonValue>,
) -> Result<Vec<JsonValue>, GenericCommandFailure> {
    let release_id = string_field(record, "release_id").unwrap_or_default();
    let version = string_field(record, "version").unwrap_or_default();
    for (index, argv) in commands.iter().enumerate() {
        let working_directory = generic_component_working_directory(source_dir, component)
            .map_err(|detail| GenericCommandFailure {
                detail,
                evidence: evidence.clone(),
            })?;
        let resolved_argv =
            resolve_generic_command_argv(argv, component, record, source_date_epoch).map_err(
                |detail| GenericCommandFailure {
                    detail: format!(
                        "Component {} {phase} command {index} has invalid direct argv: {detail}",
                        component.id
                    ),
                    evidence: evidence.clone(),
                },
            )?;
        let mut command = Command::new(&resolved_argv[0]);
        command
            .args(&resolved_argv[1..])
            .current_dir(&working_directory)
            .env("AIT_RELEASE_ID", &release_id)
            .env("AIT_RELEASE_VERSION", &version)
            .env("AIT_RELEASE_COMPONENT", &component.id)
            .env("AIT_RELEASE_ECOSYSTEM", &component.ecosystem)
            .env("SOURCE_DATE_EPOCH", source_date_epoch.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(target) =
            generic_record_selection(record).map_err(|detail| GenericCommandFailure {
                detail,
                evidence: evidence.clone(),
            })?
        {
            command.env("AIT_RELEASE_TARGET", target);
        }
        match command.spawn() {
            Ok(mut child) => {
                let stdout_reader = child
                    .stdout
                    .take()
                    .map(spawn_bounded_command_stream)
                    .ok_or_else(|| GenericCommandFailure {
                        detail: format!(
                            "Component {} {phase} command {index} did not expose stdout evidence.",
                            component.id
                        ),
                        evidence: evidence.clone(),
                    })?;
                let stderr_reader = child
                    .stderr
                    .take()
                    .map(spawn_bounded_command_stream)
                    .ok_or_else(|| GenericCommandFailure {
                        detail: format!(
                            "Component {} {phase} command {index} did not expose stderr evidence.",
                            component.id
                        ),
                        evidence: evidence.clone(),
                    })?;
                let status = child.wait().map_err(|error| GenericCommandFailure {
                    detail: format!(
                        "Component {} {phase} command {index} could not be reaped: {error}",
                        component.id
                    ),
                    evidence: evidence.clone(),
                })?;
                let stdout = stdout_reader
                    .join()
                    .map_err(|_| GenericCommandFailure {
                        detail: format!(
                            "Component {} {phase} command {index} stdout reader panicked.",
                            component.id
                        ),
                        evidence: evidence.clone(),
                    })?
                    .map_err(|detail| GenericCommandFailure {
                        detail: format!(
                            "Component {} {phase} command {index} stdout could not be read: {detail}",
                            component.id
                        ),
                        evidence: evidence.clone(),
                    })?;
                let stderr = stderr_reader
                    .join()
                    .map_err(|_| GenericCommandFailure {
                        detail: format!(
                            "Component {} {phase} command {index} stderr reader panicked.",
                            component.id
                        ),
                        evidence: evidence.clone(),
                    })?
                    .map_err(|detail| GenericCommandFailure {
                        detail: format!(
                            "Component {} {phase} command {index} stderr could not be read: {detail}",
                            component.id
                        ),
                        evidence: evidence.clone(),
                    })?;
                let passed = status.success();
                let detail = bounded_command_output(
                    stdout,
                    stderr,
                    if passed {
                        "Command completed without output."
                    } else {
                        "Command failed without output."
                    },
                );
                evidence.push(json!({
                    "phase": phase,
                    "command_index": index,
                    "declared_argv": argv,
                    "argv": &resolved_argv,
                    "status": if passed { "pass" } else { "fail" },
                    "exit_code": status.code(),
                    "output": detail,
                }));
                if !passed {
                    return Err(GenericCommandFailure {
                        detail: format!(
                            "Component {} {phase} command {} failed with exit code {:?}: {}",
                            component.id,
                            index,
                            status.code(),
                            resolved_argv.join(" ")
                        ),
                        evidence,
                    });
                }
            }
            Err(error) => {
                evidence.push(json!({
                    "phase": phase,
                    "command_index": index,
                    "declared_argv": argv,
                    "argv": &resolved_argv,
                    "status": "fail",
                    "exit_code": JsonValue::Null,
                    "output": error.to_string(),
                }));
                return Err(GenericCommandFailure {
                    detail: format!(
                        "Component {} {phase} command {} could not start: {error}",
                        component.id, index
                    ),
                    evidence,
                });
            }
        }
    }
    Ok(evidence)
}

fn add_component_check_evidence(
    mut check: JsonValue,
    component: &GenericReleaseComponent,
    evidence: Vec<JsonValue>,
) -> JsonValue {
    if let Some(object) = check.as_object_mut() {
        object.insert("component".to_string(), json!(component.id));
        object.insert("ecosystem".to_string(), json!(component.ecosystem));
        object.insert("commands".to_string(), JsonValue::Array(evidence));
    }
    check
}

fn generic_component_test_check(
    repo: &RepoRuntime,
    bundle: &ReleaseBundle,
    component: &GenericReleaseComponent,
    record: &JsonValue,
) -> JsonValue {
    let source_date_epoch = match release_epoch(&bundle.raw) {
        Ok(epoch) => epoch,
        Err(error) => {
            return add_component_check_evidence(
                check_result(
                    &format!("adapter_tests.{}", component.id),
                    &format!("Generic adapter tests pass for component {}", component.id),
                    "fail",
                    error,
                    true,
                ),
                component,
                Vec::new(),
            )
        }
    };
    let temp = match materialize_release_bundle_to_temp(repo, bundle, "ait-release-adapter-check-")
    {
        Ok(temp) => temp,
        Err(error) => {
            return add_component_check_evidence(
                check_result(
                    &format!("adapter_tests.{}", component.id),
                    &format!("Generic adapter tests pass for component {}", component.id),
                    "fail",
                    error,
                    true,
                ),
                component,
                Vec::new(),
            )
        }
    };
    let external_materialization = temp.external_materialization().cloned();
    let prepared = run_generic_commands(
        temp.source_dir(),
        component,
        record,
        source_date_epoch,
        "prepare",
        &component.prepare_commands,
        Vec::new(),
    );
    let result = prepared.and_then(|evidence| {
        run_generic_commands(
            temp.source_dir(),
            component,
            record,
            source_date_epoch,
            "test",
            &component.test_commands,
            evidence,
        )
    });
    let mut check = match result {
        Ok(evidence) => add_component_check_evidence(
            check_result(
                &format!("adapter_tests.{}", component.id),
                &format!("Generic adapter tests pass for component {}", component.id),
                "pass",
                format!(
                    "Executed {} direct prepare/test command(s) in an isolated Snapshot export.",
                    evidence.len()
                ),
                false,
            ),
            component,
            evidence,
        ),
        Err(failure) => add_component_check_evidence(
            check_result(
                &format!("adapter_tests.{}", component.id),
                &format!("Generic adapter tests pass for component {}", component.id),
                "fail",
                failure.detail,
                true,
            ),
            component,
            failure.evidence,
        ),
    };
    if let Some(external_materialization) = external_materialization {
        if let Some(object) = check.as_object_mut() {
            object.insert(
                "external_materialization".to_string(),
                external_materialization,
            );
        }
    }
    check
}

fn generic_release_check_record(
    repo: &RepoRuntime,
    record: &JsonValue,
) -> Result<JsonValue, String> {
    let snapshot_id = required_string_field(record, "snapshot_id")?;
    let line_name = required_string_field(record, "line")?;
    let bundle = release_snapshot_bundle(repo, &snapshot_id)?;
    let adapter = generic_release_adapter_from_bundle(&bundle)?;
    validate_generic_release_identity(record, &adapter, &bundle)?;
    let target = generic_record_selection(record)?;
    let selected_components = adapter.selected_components(target.as_deref());
    let selected_artifact_count = adapter.selected_artifact_count(target.as_deref());
    let mut checks = Vec::new();

    let workspace_matches = workspace_matches_release_source(repo, &line_name, &snapshot_id);
    checks.push(check_result(
        "workspace_clean",
        "Workspace is clean against the selected line head",
        if workspace_matches { "pass" } else { "fail" },
        if workspace_matches {
            format!("Workspace is clean on line {line_name} at snapshot {snapshot_id}.")
        } else {
            format!("Current workspace is not on line {line_name} at snapshot {snapshot_id}.")
        },
        !workspace_matches,
    ));
    checks.push(check_result(
        "adapter_contract",
        "Snapshot-owned generic release adapter contract is valid",
        "pass",
        format!(
            "Validated {} selected component(s), {} selected artifact(s), target {}, and manifest SHA-256 {}.",
            selected_components.len(),
            selected_artifact_count,
            target.as_deref().unwrap_or("all"),
            generic_manifest_sha256(&bundle)?
        ),
        false,
    ));
    let export = materialize_release_bundle_to_temp(repo, &bundle, "ait-release-adapter-export-");
    let export_ok = export
        .as_ref()
        .map(|temp| {
            temp.source_dir()
                .join(GENERIC_RELEASE_MANIFEST_PATH)
                .is_file()
        })
        .unwrap_or(false);
    let external_materialization = export
        .as_ref()
        .ok()
        .and_then(MaterializedBundleTemp::external_materialization)
        .cloned();
    let mut export_check = check_result(
        "snapshot_export",
        "Selected Snapshot exports into an isolated generic release source tree",
        if export_ok { "pass" } else { "fail" },
        match &export {
            Ok(_) if export_ok => {
                format!("Snapshot exported with {GENERIC_RELEASE_MANIFEST_PATH} present.")
            }
            Ok(_) => format!("Snapshot export did not include {GENERIC_RELEASE_MANIFEST_PATH}."),
            Err(error) => format!("Snapshot export failed: {error}"),
        },
        !export_ok,
    );
    if let Some(external_materialization) = external_materialization {
        if let Some(object) = export_check.as_object_mut() {
            object.insert(
                "external_materialization".to_string(),
                external_materialization,
            );
        }
    }
    checks.push(export_check);
    checks.push(check_result(
        "dependency_authority",
        "Declared dependency-authority files are present in the selected Snapshot",
        "pass",
        format!(
            "Validated {} dependency-authority file declaration(s).",
            selected_components
                .iter()
                .map(|component| component.dependency_files.len())
                .sum::<usize>()
        ),
        false,
    ));
    for component in selected_components {
        checks.push(generic_component_test_check(
            repo, &bundle, component, record,
        ));
    }
    if let Some(check) = release_external_readiness_check(repo)? {
        checks.push(check);
    }

    let failed = checks
        .iter()
        .filter(|row| string_field(row, "status").as_deref() == Some("fail"))
        .count();
    let blocking = checks
        .iter()
        .filter(|row| bool_field(row, "blocking"))
        .count();
    let decision = if failed == 0 { "pass" } else { "fail" };
    let mut metadata = record
        .get("metadata")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.insert("package".to_string(), adapter.package.to_package_json());
    let checked_at = current_timestamp();
    metadata.insert(
        "check_summary".to_string(),
        json!({
            "decision": decision,
            "failed": failed,
            "blocking": blocking,
            "checked_at": checked_at,
            "adapter_contract": GENERIC_RELEASE_ADAPTER_CONTRACT,
        }),
    );
    let passed = checks
        .iter()
        .filter(|row| string_field(row, "status").as_deref() == Some("pass"))
        .count();
    let mut result = record.clone();
    let object = result
        .as_object_mut()
        .ok_or_else(|| "Generic release receipt must be an object.".to_string())?;
    object.insert(
        "status".to_string(),
        json!(if decision == "pass" {
            "checked"
        } else {
            "blocked"
        }),
    );
    object.insert("checks".to_string(), JsonValue::Array(checks));
    object.insert("metadata".to_string(), JsonValue::Object(metadata));
    object.insert(
        "check_summary".to_string(),
        json!({
            "total": passed + failed,
            "passed": passed,
            "failed": failed,
            "blocking": blocking,
            "decision": decision,
        }),
    );
    object.insert(
        "next_action".to_string(),
        if decision == "pass" {
            json!({
                "code": "build_adapter",
                "detail": format!(
                    "Run `ait release adapter build --version {} --line {}{}` to build the exact checked Snapshot selection.",
                    required_string_field(record, "version")?,
                    required_string_field(record, "line")?,
                    target
                        .as_deref()
                        .map(|target| format!(" --target {target}"))
                        .unwrap_or_default()
                ),
            })
        } else {
            json!({
                "code": "resolve_checks",
                "detail": "Resolve blocking generic adapter checks, create a new Snapshot, and rerun the adapter check.",
            })
        },
    );
    object.insert("updated_at".to_string(), json!(checked_at));
    Ok(result)
}

fn remove_declared_generic_outputs(
    source_dir: &Path,
    adapter: &GenericReleaseAdapter,
) -> Result<(), String> {
    for component in &adapter.components {
        for artifact in &component.artifacts {
            let path = source_dir.join(component_relative_path(component, &artifact.path));
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() => {
                    return Err(format!(
                        "Declared generic release artifact path is a directory: {}.",
                        component_relative_path(component, &artifact.path)
                    ));
                }
                Ok(_) => fs::remove_file(&path).map_err(io_error)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
        }
    }
    Ok(())
}

fn prepare_generic_dist_dir(repo: &RepoRuntime, release_id: &str) -> Result<PathBuf, String> {
    let workspace_root = repo.workspace_root();
    let canonical_workspace = workspace_root.canonicalize().map_err(io_error)?;
    let dist_root = workspace_root.join("dist");
    match fs::symlink_metadata(&dist_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(
                "Generic release dist root must be a real directory inside the workspace."
                    .to_string(),
            )
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&dist_root).map_err(io_error)?;
        }
        Err(error) => return Err(io_error(error)),
    }
    let canonical_dist_root = dist_root.canonicalize().map_err(io_error)?;
    if canonical_dist_root.parent() != Some(canonical_workspace.as_path()) {
        return Err(
            "Generic release dist root escapes the canonical workspace boundary.".to_string(),
        );
    }

    let dist_dir = dist_root.join(release_id);
    match fs::symlink_metadata(&dist_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "Generic release projection dist/{release_id} must be a real directory."
            ))
        }
        Ok(_) => fs::remove_dir_all(&dist_dir).map_err(io_error)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    fs::create_dir(&dist_dir).map_err(io_error)?;
    let canonical_dist_dir = dist_dir.canonicalize().map_err(io_error)?;
    if canonical_dist_dir.parent() != Some(canonical_dist_root.as_path()) {
        return Err(format!(
            "Generic release projection dist/{release_id} escapes the canonical dist root."
        ));
    }
    Ok(dist_dir)
}

fn collect_generic_artifact(
    repo: &RepoRuntime,
    source_dir: &Path,
    dist_dir: &Path,
    component: &GenericReleaseComponent,
    artifact: &GenericReleaseArtifact,
) -> Result<JsonValue, String> {
    let relative_source = component_relative_path(component, &artifact.path);
    let source_path = source_dir.join(&relative_source);
    let symlink_metadata = fs::symlink_metadata(&source_path).map_err(|error| {
        format!(
            "Component {} did not produce declared artifact {} ({error}).",
            component.id, artifact.path
        )
    })?;
    if symlink_metadata.file_type().is_symlink() || !symlink_metadata.is_file() {
        return Err(format!(
            "Component {} declared artifact {} must be a regular file, not a symlink or directory.",
            component.id, artifact.path
        ));
    }
    let canonical_source_root = source_dir.canonicalize().map_err(io_error)?;
    let canonical_source = source_path.canonicalize().map_err(io_error)?;
    if !canonical_source.starts_with(&canonical_source_root) {
        return Err(format!(
            "Component {} declared artifact {} escapes the isolated Snapshot export.",
            component.id, artifact.path
        ));
    }
    let destination = dist_dir
        .join("components")
        .join(&component.id)
        .join(&artifact.path);
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            return Err(format!(
            "Component {} artifact destination already exists in the fresh release projection: {}.",
            component.id, artifact.path
        ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
        let canonical_dist_dir = dist_dir.canonicalize().map_err(io_error)?;
        let canonical_parent = parent.canonicalize().map_err(io_error)?;
        if !canonical_parent.starts_with(&canonical_dist_dir) {
            return Err(format!(
                "Component {} artifact destination escapes the canonical release projection: {}.",
                component.id, artifact.path
            ));
        }
    }
    fs::copy(&source_path, &destination).map_err(io_error)?;
    let destination_metadata = fs::symlink_metadata(&destination).map_err(io_error)?;
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_file() {
        return Err(format!(
            "Component {} artifact destination is not a regular file: {}.",
            component.id, artifact.path
        ));
    }
    let mut row = artifact_info(repo, &destination)?;
    let object = row
        .as_object_mut()
        .ok_or_else(|| "Generic release artifact projection must be an object.".to_string())?;
    object.insert("kind".to_string(), json!(artifact.kind));
    if let Some(target) = &artifact.target {
        object.insert("target".to_string(), json!(target));
    }
    object.insert("role".to_string(), json!("component-artifact"));
    object.insert("component".to_string(), json!(component.id));
    object.insert("ecosystem".to_string(), json!(component.ecosystem));
    object.insert("declared_path".to_string(), json!(artifact.path));
    object.insert("source_path".to_string(), json!(relative_source));
    Ok(row)
}

fn collect_generic_license_material(
    repo: &RepoRuntime,
    source_dir: &Path,
    dist_dir: &Path,
    license_file: &GenericReleaseLicenseFile,
) -> Result<JsonValue, String> {
    let source_path = source_dir.join(&license_file.path);
    let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
        format!(
            "Declared generic release {} material {} is unavailable ({error}).",
            license_file.role, license_file.path
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Declared generic release {} material {} must be a regular file, not a symlink or directory.",
            license_file.role, license_file.path
        ));
    }
    let canonical_source_root = source_dir.canonicalize().map_err(io_error)?;
    let canonical_source = source_path.canonicalize().map_err(io_error)?;
    if !canonical_source.starts_with(&canonical_source_root) {
        return Err(format!(
            "Declared generic release {} material {} escapes the isolated Snapshot export.",
            license_file.role, license_file.path
        ));
    }
    let destination = dist_dir
        .join("license-material")
        .join(&license_file.role)
        .join(&license_file.path);
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            return Err(format!(
                "Generic release {} material destination already exists in the fresh release projection: {}.",
                license_file.role, license_file.path
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
        let canonical_dist_dir = dist_dir.canonicalize().map_err(io_error)?;
        let canonical_parent = parent.canonicalize().map_err(io_error)?;
        if !canonical_parent.starts_with(&canonical_dist_dir) {
            return Err(format!(
                "Generic release {} material destination escapes the canonical release projection: {}.",
                license_file.role, license_file.path
            ));
        }
    }
    fs::copy(&source_path, &destination).map_err(io_error)?;
    let destination_metadata = fs::symlink_metadata(&destination).map_err(io_error)?;
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_file() {
        return Err(format!(
            "Generic release {} material destination is not a regular file: {}.",
            license_file.role, license_file.path
        ));
    }
    let mut row = artifact_info(repo, &destination)?;
    let object = row
        .as_object_mut()
        .ok_or_else(|| "Generic release license material must be an object.".to_string())?;
    object.insert("kind".to_string(), json!("license-material"));
    object.insert("role".to_string(), json!("license-material"));
    object.insert("material_role".to_string(), json!(license_file.role));
    object.insert("declared_path".to_string(), json!(license_file.path));
    object.insert("source_path".to_string(), json!(license_file.path));
    Ok(row)
}

fn mark_generated_artifact_role(mut artifact: JsonValue, role: &str) -> Result<JsonValue, String> {
    artifact
        .as_object_mut()
        .ok_or_else(|| "Generated generic release artifact must be an object.".to_string())?
        .insert("role".to_string(), json!(role));
    Ok(artifact)
}

fn generic_build_projection(
    repo: &RepoRuntime,
    record: &JsonValue,
    bundle: &ReleaseBundle,
    adapter: &GenericReleaseAdapter,
) -> Result<(Vec<JsonValue>, JsonValue), String> {
    let release_id = required_string_field(record, "release_id")?;
    let release_id_value = json!(release_id.clone());
    require_identifier(Some(&release_id_value), "release_id")?;
    let source_date_epoch = release_epoch(&bundle.raw)?;
    let temp = materialize_release_bundle_to_temp(repo, bundle, "ait-release-adapter-build-")?;
    let external_materialization = temp.external_materialization().cloned();
    let source_dir = temp.source_dir();
    let target = generic_record_selection(record)?;
    let selected_components = adapter.selected_components(target.as_deref());
    let selected_artifact_count = adapter.selected_artifact_count(target.as_deref());
    remove_declared_generic_outputs(source_dir, adapter)?;
    let dist_dir = prepare_generic_dist_dir(repo, &release_id)?;
    let mut artifacts = adapter
        .package
        .license_files
        .iter()
        .map(|file| collect_generic_license_material(repo, source_dir, &dist_dir, file))
        .collect::<Result<Vec<_>, String>>()?;
    let mut component_results = Vec::new();
    for component in selected_components {
        let evidence = run_generic_commands(
            source_dir,
            component,
            record,
            source_date_epoch,
            "prepare",
            &component.prepare_commands,
            Vec::new(),
        )
        .and_then(|evidence| {
            run_generic_commands(
                source_dir,
                component,
                record,
                source_date_epoch,
                "build",
                &component.build_commands,
                evidence,
            )
        })
        .map_err(|failure| failure.detail)?;
        let evidence = run_generic_commands(
            source_dir,
            component,
            record,
            source_date_epoch,
            "smoke",
            &component.smoke_commands,
            evidence,
        )
        .map_err(|failure| failure.detail)?;
        let mut component_artifacts = Vec::new();
        for artifact in component
            .artifacts
            .iter()
            .filter(|artifact| generic_artifact_selected(artifact, target.as_deref()))
        {
            component_artifacts.push(collect_generic_artifact(
                repo, source_dir, &dist_dir, component, artifact,
            )?);
        }
        artifacts.extend(component_artifacts);
        component_results.push(json!({
            "component": component.id,
            "ecosystem": component.ecosystem,
            "status": "pass",
            "command_count": evidence.len(),
            "commands": evidence,
            "artifact_count": component
                .artifacts
                .iter()
                .filter(|artifact| generic_artifact_selected(artifact, target.as_deref()))
                .count(),
        }));
    }
    artifacts.sort_by_key(|artifact| {
        (
            string_field(artifact, "component").unwrap_or_default(),
            string_field(artifact, "declared_path").unwrap_or_default(),
            string_field(artifact, "kind").unwrap_or_default(),
            string_field(artifact, "target"),
        )
    });
    let manifest_components = component_results
        .iter()
        .map(|component| {
            json!({
                "component": component.get("component").cloned().unwrap_or(JsonValue::Null),
                "ecosystem": component.get("ecosystem").cloned().unwrap_or(JsonValue::Null),
                "status": component.get("status").cloned().unwrap_or(JsonValue::Null),
                "command_count": component.get("command_count").cloned().unwrap_or(JsonValue::Null),
                "artifact_count": component.get("artifact_count").cloned().unwrap_or(JsonValue::Null),
            })
        })
        .collect::<Vec<_>>();
    let manifest_artifacts = artifacts
        .iter()
        .filter(|artifact| string_field(artifact, "role").as_deref() == Some("component-artifact"))
        .map(|artifact| {
            let mut row = json!({
                "role": artifact.get("role").cloned().unwrap_or(JsonValue::Null),
                "component": artifact.get("component").cloned().unwrap_or(JsonValue::Null),
                "ecosystem": artifact.get("ecosystem").cloned().unwrap_or(JsonValue::Null),
                "declared_path": artifact.get("declared_path").cloned().unwrap_or(JsonValue::Null),
                "kind": artifact.get("kind").cloned().unwrap_or(JsonValue::Null),
                "path": artifact.get("path").cloned().unwrap_or(JsonValue::Null),
                "size_bytes": artifact.get("size_bytes").cloned().unwrap_or(JsonValue::Null),
                "sha256": artifact.get("sha256").cloned().unwrap_or(JsonValue::Null),
            });
            if let Some(target) = artifact.get("target") {
                if let Some(object) = row.as_object_mut() {
                    object.insert("target".to_string(), target.clone());
                }
            }
            row
        })
        .collect::<Vec<_>>();
    let manifest_license_material = artifacts
        .iter()
        .filter(|artifact| string_field(artifact, "role").as_deref() == Some("license-material"))
        .map(|artifact| {
            json!({
                "role": "license-material",
                "material_role": artifact.get("material_role").cloned().unwrap_or(JsonValue::Null),
                "declared_path": artifact.get("declared_path").cloned().unwrap_or(JsonValue::Null),
                "path": artifact.get("path").cloned().unwrap_or(JsonValue::Null),
                "size_bytes": artifact.get("size_bytes").cloned().unwrap_or(JsonValue::Null),
                "sha256": artifact.get("sha256").cloned().unwrap_or(JsonValue::Null),
            })
        })
        .collect::<Vec<_>>();
    let built_at = required_string_field(&bundle.raw, "created_at")?;
    let manifest_path = dist_dir.join("ait-release.manifest.json");
    let mut manifest = json!({
        "contract": GENERIC_RELEASE_ADAPTER_CONTRACT,
        "builder": GENERIC_RELEASE_BUILDER,
        "release_id": release_id,
        "repo_name": required_string_field(record, "repo_name")?,
        "version": required_string_field(record, "version")?,
        "line": required_string_field(record, "line")?,
        "snapshot_id": required_string_field(record, "snapshot_id")?,
        "source_manifest_hash": required_string_field(record, "manifest_hash")?,
        "adapter_manifest_path": GENERIC_RELEASE_MANIFEST_PATH,
        "adapter_manifest_sha256": generic_manifest_sha256(bundle)?,
        "package": adapter.package.to_package_json(),
        "components": manifest_components,
        "artifacts": manifest_artifacts,
        "license_material": manifest_license_material,
        "built_at": built_at,
        "source_date_epoch": source_date_epoch,
    });
    if let Some(target) = &target {
        let field = if target == GENERIC_PORTABLE_SELECTION {
            "artifact_selection"
        } else {
            "target"
        };
        manifest
            .as_object_mut()
            .ok_or_else(|| "Generic build manifest must be an object.".to_string())?
            .insert(field.to_string(), json!(target));
    }
    if let Some(external_materialization) = &external_materialization {
        manifest
            .as_object_mut()
            .ok_or_else(|| "Generic build manifest must be an object.".to_string())?
            .insert(
                "external_materialization".to_string(),
                external_materialization.clone(),
            );
    }
    fs::write(
        &manifest_path,
        encode_value_pretty_with_newline_error_string(&manifest)?,
    )
    .map_err(io_error)?;
    let manifest_artifact =
        mark_generated_artifact_role(artifact_info(repo, &manifest_path)?, "release-manifest")?;
    artifacts.push(manifest_artifact);

    let checksum_path = dist_dir.join("ait-release.sha256");
    let checksum_text = artifacts
        .iter()
        .map(|artifact| {
            format!(
                "{}  {}",
                string_field(artifact, "sha256").unwrap_or_default(),
                string_field(artifact, "path").unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&checksum_path, checksum_text).map_err(io_error)?;
    artifacts.push(mark_generated_artifact_role(
        artifact_info(repo, &checksum_path)?,
        "release-checksum",
    )?);
    artifacts.sort_by_key(|artifact| {
        (
            string_field(artifact, "role").unwrap_or_default(),
            string_field(artifact, "component").unwrap_or_default(),
            string_field(artifact, "path").unwrap_or_default(),
        )
    });
    let mut build = json!({
        "builder": GENERIC_RELEASE_BUILDER,
        "adapter_contract": GENERIC_RELEASE_ADAPTER_CONTRACT,
        "adapter_manifest_sha256": generic_manifest_sha256(bundle)?,
        "dist_dir": relative_or_absolute(repo, &dist_dir),
        "manifest_path": relative_or_absolute(repo, &manifest_path),
        "checksum_path": relative_or_absolute(repo, &checksum_path),
        "built_at": built_at,
        "source_date_epoch": source_date_epoch,
        "component_count": component_results.len(),
        "declared_artifact_count": selected_artifact_count,
        "license_material_count": adapter.package.license_files.len(),
        "components": component_results,
        "command_execution": "direct_argv_without_implicit_shell",
        "registry_publish": false,
    });
    if let Some(external_materialization) = external_materialization {
        build
            .as_object_mut()
            .ok_or_else(|| "Generic build evidence must be an object.".to_string())?
            .insert(
                "external_materialization".to_string(),
                external_materialization,
            );
    }
    Ok((artifacts, build))
}

pub fn release_adapter_check(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
) -> Result<JsonValue, String> {
    release_adapter_check_for_target(repo, version, line_name, None)
}

pub fn release_adapter_check_for_target(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    target: Option<&str>,
) -> Result<JsonValue, String> {
    let record = generic_release_adapter_record(repo, version, line_name, target)?;
    let mut checked = generic_release_check_record(repo, &record)?;
    checked
        .as_object_mut()
        .ok_or_else(|| "Generic release receipt must be an object.".to_string())?
        .insert("command".to_string(), json!("release adapter check"));
    Ok(checked)
}

pub fn release_adapter_build(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
) -> Result<JsonValue, String> {
    release_adapter_build_for_target(repo, version, line_name, None)
}

pub fn release_adapter_build_for_target(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    target: Option<&str>,
) -> Result<JsonValue, String> {
    let mut record = release_adapter_check_for_target(repo, version, line_name, target)?;
    let decision = record
        .get("check_summary")
        .and_then(|summary| summary.get("decision"))
        .and_then(JsonValue::as_str)
        .unwrap_or("fail");
    if decision != "pass" {
        let blocking = record
            .get("check_summary")
            .and_then(|summary| summary.get("blocking"))
            .and_then(JsonValue::as_u64)
            .unwrap_or_default();
        return Err(format!(
            "Generic release adapter build is blocked by {blocking} check(s). Run `ait release adapter check --version {} --line {}{}` and resolve the recorded failures.",
            version.trim(),
            line_name.trim(),
            normalize_generic_target(target)?
                .map(|target| format!(" --target {target}"))
                .unwrap_or_default()
        ));
    }
    let snapshot_id = required_string_field(&record, "snapshot_id")?;
    let bundle = release_snapshot_bundle(repo, &snapshot_id)?;
    let adapter = generic_release_adapter_from_bundle(&bundle)?;
    validate_generic_release_identity(&record, &adapter, &bundle)?;
    let (artifacts, build) = generic_build_projection(repo, &record, &bundle, &adapter)?;
    assert_release_artifact_paths_are_publishable(
        &required_string_field(&record, "release_id")?,
        &artifacts,
    )?;
    let mut metadata = record
        .get("metadata")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.insert("package".to_string(), adapter.package.to_package_json());
    metadata.insert("build".to_string(), build);
    let built_at = current_timestamp();
    let object = record
        .as_object_mut()
        .ok_or_else(|| "Generic release receipt must be an object.".to_string())?;
    object.insert("command".to_string(), json!("release adapter build"));
    object.insert("status".to_string(), json!("built"));
    object.insert("artifacts".to_string(), JsonValue::Array(artifacts));
    object.insert("metadata".to_string(), JsonValue::Object(metadata));
    object.insert("updated_at".to_string(), json!(built_at));
    object.insert(
        "next_action".to_string(),
        json!({
            "code": "promote_with_ecosystem_adapter",
            "detail": "Use the recorded immutable artifacts with an ecosystem-specific signing, clean-install, and registry promotion adapter. AIT remote Release authority is not activated by this command.",
        }),
    );
    let recorded_artifacts = record
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    assert_generic_release_artifacts_complete(&record, &recorded_artifacts)?;
    Ok(record)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn assert_generic_release_artifacts_complete(
    record: &JsonValue,
    artifacts: &[JsonValue],
) -> Result<(), String> {
    let release_id = required_string_field(record, "release_id")?;
    let adapter_metadata = record
        .get("metadata")
        .and_then(|metadata| metadata.get("release_adapter"))
        .ok_or_else(|| {
            format!("Generic release {release_id} is missing release_adapter metadata.")
        })?;
    if string_field(adapter_metadata, "contract").as_deref()
        != Some(GENERIC_RELEASE_ADAPTER_CONTRACT)
        || string_field(adapter_metadata, "manifest_path").as_deref()
            != Some(GENERIC_RELEASE_MANIFEST_PATH)
    {
        return Err(format!(
            "Generic release {release_id} has an invalid adapter contract identity."
        ));
    }
    let definition = adapter_metadata.get("definition").ok_or_else(|| {
        format!("Generic release {release_id} is missing its adapter definition.")
    })?;
    let adapter = parse_generic_release_adapter(definition)?;
    if adapter.package.version != required_string_field(record, "version")? {
        return Err(format!(
            "Generic release {release_id} adapter version no longer matches the candidate."
        ));
    }
    let manifest_sha256 = required_string_field(adapter_metadata, "manifest_sha256")?;
    if !valid_sha256(&manifest_sha256) {
        return Err(format!(
            "Generic release {release_id} has an invalid adapter manifest SHA-256."
        ));
    }
    if adapter_metadata
        .get("component_count")
        .and_then(JsonValue::as_u64)
        != Some(adapter.components.len() as u64)
        || adapter_metadata
            .get("declared_artifact_count")
            .and_then(JsonValue::as_u64)
            != Some(adapter.declared_artifact_count() as u64)
    {
        return Err(format!(
            "Generic release {release_id} adapter inventory counts do not match its bound definition."
        ));
    }
    let adapter_license_count = adapter_metadata
        .get("license_material_count")
        .and_then(JsonValue::as_u64);
    if adapter_license_count
        .map(|count| count != adapter.package.license_files.len() as u64)
        .unwrap_or(!adapter.package.license_files.is_empty())
    {
        return Err(format!(
            "Generic release {release_id} adapter license-material count does not match its bound definition."
        ));
    }
    let target = generic_record_selection(record)?;
    require_generic_target_selection(&adapter, target.as_deref())?;
    let selected_components = adapter.selected_components(target.as_deref());
    let selected_artifact_count = adapter.selected_artifact_count(target.as_deref());
    let build = record
        .get("metadata")
        .and_then(|metadata| metadata.get("build"))
        .ok_or_else(|| format!("Generic release {release_id} has not been built."))?;
    let expected_builder = match string_field(record, "contract").as_deref() {
        Some(GENERIC_RELEASE_RECEIPT_CONTRACT) => GENERIC_RELEASE_BUILDER,
        Some(PUBLIC_GIT_RELEASE_RECEIPT_CONTRACT) => PUBLIC_GIT_RELEASE_BUILDER,
        Some(contract) => {
            return Err(format!(
                "Generic release {release_id} uses unsupported receipt contract {contract:?}."
            ))
        }
        None => {
            return Err(format!(
                "Generic release {release_id} is missing its receipt contract."
            ))
        }
    };
    if string_field(build, "builder").as_deref() != Some(expected_builder)
        || string_field(build, "adapter_contract").as_deref()
            != Some(GENERIC_RELEASE_ADAPTER_CONTRACT)
        || string_field(build, "adapter_manifest_sha256").as_deref()
            != Some(manifest_sha256.as_str())
    {
        return Err(format!(
            "Generic release {release_id} is missing the bound generic builder contract."
        ));
    }
    if build.get("component_count").and_then(JsonValue::as_u64)
        != Some(selected_components.len() as u64)
        || build
            .get("declared_artifact_count")
            .and_then(JsonValue::as_u64)
            != Some(selected_artifact_count as u64)
    {
        return Err(format!(
            "Generic release {release_id} build inventory counts do not match the adapter definition."
        ));
    }
    let build_license_count = build
        .get("license_material_count")
        .and_then(JsonValue::as_u64);
    if build_license_count
        .map(|count| count != adapter.package.license_files.len() as u64)
        .unwrap_or(!adapter.package.license_files.is_empty())
    {
        return Err(format!(
            "Generic release {release_id} build license-material count does not match the adapter definition."
        ));
    }

    let expected_components = selected_components
        .iter()
        .map(|component| (component.id.clone(), component.ecosystem.clone()))
        .collect::<BTreeSet<_>>();
    let mut built_components = BTreeSet::new();
    for component in build
        .get("components")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        if string_field(component, "status").as_deref() != Some("pass") {
            return Err(format!(
                "Generic release {release_id} contains a non-passing component build result."
            ));
        }
        let identity = (
            required_string_field(component, "component")?,
            required_string_field(component, "ecosystem")?,
        );
        if !built_components.insert(identity.clone()) {
            return Err(format!(
                "Generic release {release_id} contains duplicate component build result {identity:?}."
            ));
        }
    }
    if built_components != expected_components {
        return Err(format!(
            "Generic release {release_id} component build coverage does not match the adapter definition."
        ));
    }

    let expected = adapter
        .components
        .iter()
        .flat_map(|component| {
            component
                .artifacts
                .iter()
                .filter(|artifact| generic_artifact_selected(artifact, target.as_deref()))
                .map(|artifact| {
                    (
                        component.id.clone(),
                        component.ecosystem.clone(),
                        artifact.path.clone(),
                        artifact.kind.clone(),
                        artifact.target.clone(),
                    )
                })
        })
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    let expected_license_material = adapter
        .package
        .license_files
        .iter()
        .map(|file| (file.role.clone(), file.path.clone()))
        .collect::<BTreeSet<_>>();
    let mut actual_license_material = BTreeSet::new();
    let mut manifest_count = 0;
    let mut checksum_count = 0;
    for artifact in artifacts {
        let role = string_field(artifact, "role").unwrap_or_default();
        match role.as_str() {
            "component-artifact" => {
                let component = required_string_field(artifact, "component")?;
                let ecosystem = required_string_field(artifact, "ecosystem")?;
                let declared_path = required_string_field(artifact, "declared_path")?;
                let kind = required_string_field(artifact, "kind")?;
                let target = string_field(artifact, "target");
                let row = (
                    component.clone(),
                    ecosystem,
                    declared_path.clone(),
                    kind,
                    target,
                );
                if !actual.insert(row.clone()) {
                    return Err(format!(
                        "Generic release {release_id} contains duplicate component artifact {:?}.",
                        row
                    ));
                }
                let expected_path =
                    format!("dist/{release_id}/components/{component}/{declared_path}");
                if string_field(artifact, "path").as_deref() != Some(expected_path.as_str()) {
                    return Err(format!(
                        "Generic release {release_id} component artifact path does not match its release-specific projection: expected {expected_path}."
                    ));
                }
            }
            "license-material" => {
                if string_field(artifact, "kind").as_deref() != Some("license-material") {
                    return Err(format!(
                        "Generic release {release_id} license material has an invalid kind."
                    ));
                }
                let material_role = required_string_field(artifact, "material_role")?;
                if !matches!(material_role.as_str(), "license" | "notice") {
                    return Err(format!(
                        "Generic release {release_id} license material has unsupported role {material_role:?}."
                    ));
                }
                let declared_path = required_string_field(artifact, "declared_path")?;
                if string_field(artifact, "source_path").as_deref() != Some(declared_path.as_str())
                {
                    return Err(format!(
                        "Generic release {release_id} license material source path differs from its declaration."
                    ));
                }
                let row = (material_role.clone(), declared_path.clone());
                if !actual_license_material.insert(row.clone()) {
                    return Err(format!(
                        "Generic release {release_id} contains duplicate license material {row:?}."
                    ));
                }
                let expected_path =
                    format!("dist/{release_id}/license-material/{material_role}/{declared_path}");
                if string_field(artifact, "path").as_deref() != Some(expected_path.as_str()) {
                    return Err(format!(
                        "Generic release {release_id} license material path does not match its release-specific projection: expected {expected_path}."
                    ));
                }
            }
            "release-manifest" => {
                manifest_count += 1;
                let expected_path = format!("dist/{release_id}/ait-release.manifest.json");
                if string_field(artifact, "kind").as_deref() != Some("manifest")
                    || string_field(artifact, "path").as_deref() != Some(expected_path.as_str())
                {
                    return Err(format!(
                        "Generic release {release_id} generated manifest evidence has an invalid kind or path."
                    ));
                }
            }
            "release-checksum" => {
                checksum_count += 1;
                let expected_path = format!("dist/{release_id}/ait-release.sha256");
                if string_field(artifact, "kind").as_deref() != Some("checksum")
                    || string_field(artifact, "path").as_deref() != Some(expected_path.as_str())
                {
                    return Err(format!(
                        "Generic release {release_id} generated checksum evidence has an invalid kind or path."
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "Generic release {release_id} contains artifact with unsupported role {role:?}."
                ))
            }
        }
        let digest = required_string_field(artifact, "sha256")?;
        if !valid_sha256(&digest)
            || artifact
                .get("size_bytes")
                .and_then(JsonValue::as_u64)
                .is_none()
        {
            return Err(format!(
                "Generic release {release_id} artifact is missing valid digest or size evidence."
            ));
        }
    }
    if actual != expected
        || actual_license_material != expected_license_material
        || manifest_count != 1
        || checksum_count != 1
    {
        let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
        let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
        let missing_license_material = expected_license_material
            .difference(&actual_license_material)
            .cloned()
            .collect::<Vec<_>>();
        let unexpected_license_material = actual_license_material
            .difference(&expected_license_material)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "Generic release {release_id} artifact coverage is incomplete (missing: {missing:?}; unexpected: {unexpected:?}; missing license material: {missing_license_material:?}; unexpected license material: {unexpected_license_material:?}; release manifests: {manifest_count}; checksums: {checksum_count})."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_release_license_is_apache_only_and_locked_dependency_notice_stays_complete() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("ait-core repository root should resolve");
        let license = fs::read_to_string(repo_root.join("LICENSE"))
            .expect("root Apache license should be readable");
        assert!(license.contains("complete `ait-core`\nrepository"));
        assert!(license.contains("rust/**"));
        assert!(!license.contains("AGPL-3.0-only"));
        assert!(!license.contains("LicenseRef-"));
        assert!(license.contains("Apache License\n                           Version 2.0"));
        assert!(!repo_root.join("LICENSES/AGPL-3.0-only.txt").exists());
        assert!(!repo_root
            .join("LICENSES/LicenseRef-AIT-Commercial.txt")
            .exists());

        let workspace = fs::read_to_string(repo_root.join("rust/Cargo.toml"))
            .expect("workspace manifest should be readable");
        assert!(workspace.contains("license = \"Apache-2.0\""));
        for relative in [
            "rust/crates/ait-agent-core/Cargo.toml",
            "rust/crates/ait-agent-worker/Cargo.toml",
            "rust/crates/ait-benchmark/Cargo.toml",
            "rust/crates/ait-cli/Cargo.toml",
            "rust/crates/ait-core/Cargo.toml",
            "rust/crates/ait-py/Cargo.toml",
        ] {
            let manifest = fs::read_to_string(repo_root.join(relative))
                .unwrap_or_else(|error| panic!("{relative} should be readable: {error}"));
            assert!(
                manifest.contains("license.workspace = true"),
                "{relative} must inherit the Apache-2.0 workspace license"
            );
        }

        let notice =
            fs::read_to_string(repo_root.join("NOTICE")).expect("root NOTICE should be readable");
        assert_eq!(
            notice
                .matches("----- BEGIN GENERATED THIRD-PARTY NOTICES -----")
                .count(),
            1
        );
        assert!(!notice.contains("/.cargo/registry/"));
        assert!(!notice.contains("/Users/"));
        assert!(!notice.contains("/Volumes/"));
        let lock = fs::read_to_string(repo_root.join("rust/Cargo.lock"))
            .expect("Cargo.lock should be readable");
        let lock: toml::Value = toml::from_str(&lock).expect("Cargo.lock should parse");
        for package in lock
            .get("package")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|package| package.get("source").is_some())
        {
            let name = package
                .get("name")
                .and_then(toml::Value::as_str)
                .expect("locked package should have a name");
            let version = package
                .get("version")
                .and_then(toml::Value::as_str)
                .expect("locked package should have a version");
            let row_prefix = format!("{name}\t{version}\t");
            assert!(
                notice.lines().any(|line| line.starts_with(&row_prefix)),
                "NOTICE is missing locked package {name} {version}"
            );
        }
        let generator = fs::read_to_string(repo_root.join("ci/generate_rust_notice.sh"))
            .expect("notice generator should be readable");
        assert!(generator.contains("cargo metadata --manifest-path"));
        assert!(generator.contains("locked --format-version 1"));
        assert!(generator.contains("Complete deduplicated upstream legal texts"));
        assert!(generator.contains("cmp -s \"$generated\" \"$notice\""));
    }

    fn component_row(id: &str, ecosystem: &str, working_directory: &str) -> JsonValue {
        json!({
            "id": id,
            "ecosystem": ecosystem,
            "working_directory": working_directory,
            "dependency_files": ["dependency.lock"],
            "commands": {
                "prepare": [],
                "test": [["tool", "test"]],
                "build": [["tool", "build"]],
                "smoke": [],
            },
            "artifacts": [{"path": "dist/output.bin", "kind": "binary"}],
        })
    }

    fn manifest_with_components(components: Vec<JsonValue>) -> JsonValue {
        json!({
            "schema": GENERIC_RELEASE_ADAPTER_CONTRACT,
            "package": {
                "name": "polyglot-product",
                "version": "1.2.3",
                "description": "Cross-language fixture",
            },
            "components": components,
        })
    }

    fn test_repo(root: &Path) -> RepoRuntime {
        RepoRuntime {
            root: root.to_path_buf(),
            ait_dir: root.join(".ait"),
            config: JsonMap::from_iter([("repo_name".to_string(), json!("polyglot"))]),
            worktree_config_path: None,
        }
    }

    fn bundle_for_manifest(manifest: &JsonValue, extra_files: &[(&str, &[u8])]) -> ReleaseBundle {
        let mut files = BTreeMap::from([(
            GENERIC_RELEASE_MANIFEST_PATH.to_string(),
            BundleEntry {
                path: GENERIC_RELEASE_MANIFEST_PATH.to_string(),
                data: manifest.to_string().into_bytes(),
                mode: "0644".to_string(),
            },
        )]);
        for (path, data) in extra_files {
            files.insert(
                (*path).to_string(),
                BundleEntry {
                    path: (*path).to_string(),
                    data: data.to_vec(),
                    mode: "0644".to_string(),
                },
            );
        }
        ReleaseBundle {
            raw: json!({
                "created_at": "2026-01-01T00:00:00Z",
                "manifest_hash": "snapshot-manifest-hash",
            }),
            files,
        }
    }

    #[test]
    fn generic_adapter_parses_representative_cross_language_components() {
        let ecosystems = [
            ("native", "c-cpp"),
            ("web", "javascript"),
            ("node", "nodejs"),
            ("server", "java-jsp"),
            ("service", "dotnet"),
            ("package", "php"),
        ];
        let manifest = manifest_with_components(
            ecosystems
                .iter()
                .map(|(id, ecosystem)| component_row(id, ecosystem, id))
                .collect(),
        );

        let adapter = parse_generic_release_adapter(&manifest).unwrap();

        assert_eq!(adapter.components.len(), ecosystems.len());
        assert_eq!(
            adapter
                .components
                .iter()
                .map(|component| component.ecosystem.as_str())
                .collect::<Vec<_>>(),
            ecosystems
                .iter()
                .map(|(_, ecosystem)| *ecosystem)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            parse_generic_release_adapter(&adapter.to_json()).unwrap(),
            adapter
        );
    }

    #[test]
    fn generic_adapter_resolves_only_exact_release_argument_tokens() {
        let adapter =
            parse_generic_release_adapter(&manifest_with_components(vec![component_row(
                "app", "native", ".",
            )]))
            .unwrap();
        let record = json!({
            "release_id": "REL-GEN-EXACT",
            "version": "1.2.3",
            "target": "aarch64-apple-darwin",
        });
        let argv = [
            "$AIT_RELEASE_ID",
            "$AIT_RELEASE_VERSION",
            "$AIT_RELEASE_COMPONENT",
            "$AIT_RELEASE_ECOSYSTEM",
            "$AIT_RELEASE_TARGET",
            "$SOURCE_DATE_EPOCH",
            "prefix-$AIT_RELEASE_TARGET",
            "$(uname)",
        ]
        .map(str::to_string);

        let resolved =
            resolve_generic_command_argv(&argv, &adapter.components[0], &record, 1_767_225_600)
                .unwrap();

        assert_eq!(
            resolved,
            vec![
                "REL-GEN-EXACT",
                "1.2.3",
                "app",
                "native",
                "aarch64-apple-darwin",
                "1767225600",
                "prefix-$AIT_RELEASE_TARGET",
                "$(uname)",
            ]
        );

        let without_target = json!({
            "release_id": "REL-GEN-EXACT",
            "version": "1.2.3",
        });
        let error = resolve_generic_command_argv(
            &["$AIT_RELEASE_TARGET".to_string()],
            &adapter.components[0],
            &without_target,
            1_767_225_600,
        )
        .unwrap_err();
        assert!(error.contains("without an exact target or portable selection"));
    }

    #[test]
    fn generic_adapter_rejects_unknown_fields_shell_strings_and_unsafe_paths() {
        let mut unknown = manifest_with_components(vec![component_row("app", "nodejs", ".")]);
        unknown["surprise"] = json!(true);
        assert!(parse_generic_release_adapter(&unknown)
            .unwrap_err()
            .contains("unknown field(s): surprise"));

        let mut shell_string = manifest_with_components(vec![component_row("app", "nodejs", ".")]);
        shell_string["components"][0]["commands"]["build"] = json!(["npm run build"]);
        assert!(parse_generic_release_adapter(&shell_string)
            .unwrap_err()
            .contains("argv array, not a shell string"));

        let mut traversal =
            manifest_with_components(vec![component_row("app", "nodejs", "../outside")]);
        assert!(parse_generic_release_adapter(&traversal)
            .unwrap_err()
            .contains("without traversal"));
        traversal["components"][0]["working_directory"] = json!("C:\\outside");
        assert!(parse_generic_release_adapter(&traversal)
            .unwrap_err()
            .contains("drive prefixes"));

        let duplicate = manifest_with_components(vec![
            component_row("app", "nodejs", "."),
            component_row("app", "php", "."),
        ]);
        assert!(parse_generic_release_adapter(&duplicate)
            .unwrap_err()
            .contains("duplicate component id"));

        let mut license_traversal =
            manifest_with_components(vec![component_row("app", "nodejs", ".")]);
        license_traversal["package"]["license_files"] =
            json!([{"path": "../LICENSE", "role": "license"}]);
        assert!(parse_generic_release_adapter(&license_traversal)
            .unwrap_err()
            .contains("without traversal"));

        let mut duplicate_license_role =
            manifest_with_components(vec![component_row("app", "nodejs", ".")]);
        duplicate_license_role["package"]["license_files"] = json!([
            {"path": "LICENSE", "role": "license"},
            {"path": "COPYING", "role": "license"}
        ]);
        assert!(parse_generic_release_adapter(&duplicate_license_role)
            .unwrap_err()
            .contains("duplicate role"));
    }

    #[test]
    fn generic_adapter_requires_snapshot_owned_dependency_authority_files() {
        let manifest = manifest_with_components(vec![component_row("app", "nodejs", ".")]);
        let bundle = bundle_for_manifest(&manifest, &[]);

        let error = generic_release_adapter_from_bundle(&bundle).unwrap_err();

        assert!(error.contains("dependency-authority files are missing"));
        assert!(error.contains("app:dependency.lock"));
    }

    #[test]
    fn generic_adapter_requires_snapshot_owned_license_material() {
        let mut manifest = manifest_with_components(vec![component_row("app", "nodejs", ".")]);
        manifest["package"]["license_files"] = json!([
            {"path": "LICENSE", "role": "license"},
            {"path": "NOTICE", "role": "notice"}
        ]);
        let bundle = bundle_for_manifest(&manifest, &[("dependency.lock", b"locked\n")]);

        let error = generic_release_adapter_from_bundle(&bundle).unwrap_err();

        assert!(error.contains("license-material files are missing"));
        assert!(error.contains("license:LICENSE"));
        assert!(error.contains("notice:NOTICE"));
    }

    #[test]
    fn generic_adapter_bounds_snapshot_manifest_bytes() {
        let manifest = manifest_with_components(vec![component_row("app", "nodejs", ".")]);
        let mut bundle = bundle_for_manifest(&manifest, &[("dependency.lock", b"locked\n")]);
        bundle
            .files
            .get_mut(GENERIC_RELEASE_MANIFEST_PATH)
            .unwrap()
            .data = vec![b' '; MAX_MANIFEST_BYTES + 1];

        let error = generic_release_adapter_from_bundle(&bundle).unwrap_err();

        assert!(error.contains("exceeds the"));
        assert!(error.contains("contract limit"));
    }

    #[cfg(unix)]
    fn executable_manifest(artifact_command: &str) -> JsonValue {
        json!({
            "schema": GENERIC_RELEASE_ADAPTER_CONTRACT,
            "package": {"name": "fixture", "version": "1.2.3"},
            "components": [{
                "id": "app",
                "ecosystem": "nodejs",
                "working_directory": "app",
                "dependency_files": ["dependency.lock"],
                "commands": {
                    "prepare": [["sh", "-c", "test \"$SOURCE_DATE_EPOCH\" = 1767225600 && printf prepared > prepared.txt"]],
                    "test": [["sh", "-c", "test -f prepared.txt && test -f dependency.lock"]],
                    "build": [["sh", "-c", artifact_command]],
                    "smoke": [["sh", "-c", "test -s out/app.bin"]]
                },
                "artifacts": [{"path": "out/app.bin", "kind": "node-package"}]
            }]
        })
    }

    #[cfg(unix)]
    fn executable_record(adapter: &GenericReleaseAdapter, bundle: &ReleaseBundle) -> JsonValue {
        json!({
            "contract": GENERIC_RELEASE_RECEIPT_CONTRACT,
            "release_id": "REL-TEST",
            "repo_name": "polyglot",
            "version": "1.2.3",
            "line": "main",
            "snapshot_id": "SNP-TEST",
            "manifest_hash": "snapshot-manifest-hash",
            "profile": GENERIC_RELEASE_PROFILE,
            "status": "checked",
            "checks": [{"check_id": "fixture", "status": "pass", "blocking": false}],
            "artifacts": [],
            "metadata": {
                "release_adapter": generic_release_adapter_metadata(adapter, bundle).unwrap(),
            }
        })
    }

    #[cfg(unix)]
    #[test]
    fn generic_adapter_executes_isolated_phases_and_collects_exact_artifacts() {
        let temp = tempfile::TempDir::new().unwrap();
        let manifest = executable_manifest(
            "mkdir -p out && printf artifact > out/app.bin && yes x | head -c 9000",
        );
        let bundle = bundle_for_manifest(
            &manifest,
            &[("app/dependency.lock", b"locked dependency\n")],
        );
        let adapter = generic_release_adapter_from_bundle(&bundle).unwrap();
        let mut record = executable_record(&adapter, &bundle);
        let repo = test_repo(temp.path());
        let check = generic_component_test_check(&repo, &bundle, &adapter.components[0], &record);

        assert_eq!(check["status"], json!("pass"));
        assert_eq!(check["commands"].as_array().unwrap().len(), 2);

        let (artifacts, build) =
            generic_build_projection(&repo, &record, &bundle, &adapter).unwrap();
        let manifest_path = temp.path().join("dist/REL-TEST/ait-release.manifest.json");
        let first_manifest_bytes = fs::read(&manifest_path).unwrap();
        let first_manifest: JsonValue = serde_json::from_slice(&first_manifest_bytes).unwrap();
        assert_eq!(first_manifest["built_at"], json!("2026-01-01T00:00:00Z"));
        assert!(first_manifest["components"][0].get("commands").is_none());
        assert!(first_manifest["artifacts"][0]
            .get("absolute_path")
            .is_none());
        assert!(first_manifest["artifacts"][0].get("url").is_none());
        let (_, repeated_build) =
            generic_build_projection(&repo, &record, &bundle, &adapter).unwrap();
        assert_eq!(fs::read(&manifest_path).unwrap(), first_manifest_bytes);
        assert_eq!(repeated_build["built_at"], build["built_at"]);
        assert_eq!(
            artifacts
                .iter()
                .filter(|artifact| string_field(artifact, "role").as_deref()
                    == Some("component-artifact"))
                .count(),
            1
        );
        let component_artifact = artifacts
            .iter()
            .find(|artifact| {
                string_field(artifact, "role").as_deref() == Some("component-artifact")
            })
            .unwrap();
        assert_eq!(component_artifact["component"], json!("app"));
        assert_eq!(component_artifact["ecosystem"], json!("nodejs"));
        assert_eq!(component_artifact["declared_path"], json!("out/app.bin"));
        assert!(valid_sha256(component_artifact["sha256"].as_str().unwrap()));
        assert_eq!(build["builder"], json!(GENERIC_RELEASE_BUILDER));
        assert_eq!(build["components"][0]["command_count"], json!(3));
        let build_output = build["components"][0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["phase"] == "build")
            .and_then(|command| command["output"].as_str())
            .unwrap();
        assert!(build_output.contains("output truncated by ait generic release adapter"));

        record["artifacts"] = JsonValue::Array(artifacts);
        record["metadata"]["build"] = build;
        let recorded_artifacts = record["artifacts"].as_array().unwrap().clone();
        assert_generic_release_artifacts_complete(&record, &recorded_artifacts).unwrap();

        let mut tampered = record.clone();
        let component_index = tampered["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .position(|artifact| artifact["role"] == "component-artifact")
            .unwrap();
        tampered["artifacts"][component_index]["ecosystem"] = json!("php");
        let tampered_artifacts = tampered["artifacts"].as_array().unwrap().clone();
        assert!(
            assert_generic_release_artifacts_complete(&tampered, &tampered_artifacts)
                .unwrap_err()
                .contains("artifact coverage is incomplete")
        );

        let mut wrong_path = record.clone();
        wrong_path["artifacts"][component_index]["path"] = json!("dist/elsewhere/app.bin");
        let wrong_path_artifacts = wrong_path["artifacts"].as_array().unwrap().clone();
        assert!(
            assert_generic_release_artifacts_complete(&wrong_path, &wrong_path_artifacts)
                .unwrap_err()
                .contains("release-specific projection")
        );

        let built_path = temp.path().join("dist/REL-TEST/components/app/out/app.bin");
        assert_eq!(fs::read(built_path).unwrap(), b"artifact");
    }

    #[cfg(unix)]
    #[test]
    fn generic_adapter_blocks_commands_when_snapshot_epoch_is_invalid() {
        let temp = tempfile::TempDir::new().unwrap();
        let manifest = executable_manifest("exit 99");
        let mut bundle = bundle_for_manifest(
            &manifest,
            &[("app/dependency.lock", b"locked dependency\n")],
        );
        bundle.raw["created_at"] = json!("not-a-time");
        let adapter = generic_release_adapter_from_bundle(&bundle).unwrap();
        let record = executable_record(&adapter, &bundle);
        let repo = test_repo(temp.path());

        let check = generic_component_test_check(&repo, &bundle, &adapter.components[0], &record);

        assert_eq!(check["status"], json!("fail"));
        assert_eq!(check["blocking"], json!(true));
        assert!(check["details"]
            .as_str()
            .unwrap_or_default()
            .contains("created_at"));
        assert!(check["commands"].as_array().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn generic_adapter_rejects_symlink_artifacts() {
        let temp = tempfile::TempDir::new().unwrap();
        let manifest = executable_manifest("mkdir -p out && ln -s ../dependency.lock out/app.bin");
        let bundle = bundle_for_manifest(
            &manifest,
            &[("app/dependency.lock", b"locked dependency\n")],
        );
        let adapter = generic_release_adapter_from_bundle(&bundle).unwrap();
        let record = executable_record(&adapter, &bundle);
        let repo = test_repo(temp.path());

        let error = generic_build_projection(&repo, &record, &bundle, &adapter).unwrap_err();

        assert!(error.contains("must be a regular file, not a symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn generic_adapter_rejects_symlink_license_material() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("source");
        let dist_dir = temp.path().join("dist/REL-TEST");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&dist_dir).unwrap();
        fs::write(temp.path().join("outside-license"), b"outside\n").unwrap();
        symlink(
            temp.path().join("outside-license"),
            source_dir.join("LICENSE"),
        )
        .unwrap();
        let file = GenericReleaseLicenseFile {
            path: "LICENSE".to_string(),
            role: "license".to_string(),
        };
        let repo = test_repo(temp.path());

        let error =
            collect_generic_license_material(&repo, &source_dir, &dist_dir, &file).unwrap_err();

        assert!(error.contains("must be a regular file, not a symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn generic_adapter_rejects_symlink_dist_root() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        symlink(outside.path(), temp.path().join("dist")).unwrap();
        let manifest = executable_manifest("mkdir -p out && printf artifact > out/app.bin");
        let bundle = bundle_for_manifest(
            &manifest,
            &[("app/dependency.lock", b"locked dependency\n")],
        );
        let adapter = generic_release_adapter_from_bundle(&bundle).unwrap();
        let record = executable_record(&adapter, &bundle);
        let repo = test_repo(temp.path());

        let error = generic_build_projection(&repo, &record, &bundle, &adapter).unwrap_err();

        assert!(error.contains("dist root must be a real directory"));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn generic_artifact_completion_requires_adapter_build_evidence() {
        let manifest = manifest_with_components(vec![component_row("app", "nodejs", ".")]);
        let bundle = bundle_for_manifest(&manifest, &[("dependency.lock", b"locked\n")]);
        let adapter = generic_release_adapter_from_bundle(&bundle).unwrap();
        let record = json!({
            "release_id": "REL-TEST",
            "version": "1.2.3",
            "profile": GENERIC_RELEASE_PROFILE,
            "status": "checked",
            "checks": [{"check_id": "fixture", "status": "pass", "blocking": false}],
            "artifacts": [],
            "metadata": {
                "release_adapter": generic_release_adapter_metadata(&adapter, &bundle).unwrap(),
            }
        });

        assert!(assert_generic_release_artifacts_complete(&record, &[])
            .unwrap_err()
            .contains("has not been built"));
    }
}
