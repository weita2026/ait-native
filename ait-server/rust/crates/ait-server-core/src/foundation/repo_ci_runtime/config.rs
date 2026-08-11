use super::*;

#[derive(Debug)]
pub(super) struct RepoCiRuntimeConfig {
    pub(super) repo_name: String,
    pub(super) repo_id: Option<String>,
    pub(super) snapshot_id: String,
    pub(super) target_line: String,
    pub(super) trigger: String,
    pub(super) plane: String,
    pub(super) suite_ids: Vec<String>,
    pub(super) workspace_path: PathBuf,
    pub(super) output_dir: PathBuf,
    pub(super) temp_dir: Option<PathBuf>,
    pub(super) shared_cargo_target_dir: Option<PathBuf>,
    pub(super) shared_cargo_build_dir: Option<PathBuf>,
    pub(super) suites: Vec<PatchsetSuiteManifest>,
    pub(super) suite_values: Vec<JsonValue>,
    pub(super) ci_config: JsonMap<String, JsonValue>,
    pub(super) materialized_files: Vec<MaterializedFile>,
    pub(super) prewarm_commands: Vec<String>,
    pub(super) env: JsonMap<String, JsonValue>,
    pub(super) cleanup_workspace: bool,
    pub(super) dependency_evidence: Vec<String>,
    pub(super) compliance_evidence: Vec<String>,
    pub(super) task_batch_inputs: JsonMap<String, JsonValue>,
    pub(super) admitted_cpu_tokens: Option<i64>,
    pub(super) host_cpu_cores: Option<i64>,
    pub(super) scheduler_posture: Option<String>,
    pub(super) main_seed_root: Option<PathBuf>,
    pub(super) main_seed_path: Option<PathBuf>,
    pub(super) ram_shard_root: Option<PathBuf>,
    pub(super) platform: Option<String>,
    pub(super) materialization_strategy: Option<String>,
}

impl RepoCiRuntimeConfig {
    pub(super) fn from_request(request: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let repo_name = required_text(request, "repo_name")?;
        let snapshot_id = required_text(request, "snapshot_id")?;
        let target_line =
            optional_text(request, "target_line").unwrap_or_else(|| "main".to_string());
        let plane = normalize_plane(optional_text(request, "plane"))?;
        let suites_raw = request
            .get("suites")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "Field `suites` must be an array of CI suite manifests.".to_string())?;
        let mut suites = Vec::with_capacity(suites_raw.len());
        let mut suite_values = Vec::with_capacity(suites_raw.len());
        for suite in suites_raw {
            suites.push(
                serde_json::from_value::<PatchsetSuiteManifest>(suite.clone())
                    .map_err(|exc| format!("repo CI suite manifest is invalid: {exc}"))?,
            );
            suite_values.push(suite.clone());
        }
        let runtime_paths = ci_runtime_paths_from_request(request, "repo-ci", &repo_name)?;
        let mut prewarm_commands = string_array(request, "prewarm_commands")?;
        if prewarm_commands.is_empty() {
            prewarm_commands = request
                .get("prewarm")
                .and_then(JsonValue::as_object)
                .map(|prewarm| string_array(prewarm, "commands"))
                .unwrap_or_else(|| Ok(Vec::new()))?;
        }
        Ok(Self {
            repo_name,
            repo_id: optional_text(request, "repo_id"),
            snapshot_id,
            target_line,
            trigger: optional_text(request, "trigger")
                .unwrap_or_else(|| "manual_rerun".to_string()),
            plane,
            suite_ids: string_array(request, "suite_ids")?,
            workspace_path: runtime_paths.workspace_path,
            output_dir: runtime_paths.output_dir,
            temp_dir: Some(runtime_paths.temp_dir),
            shared_cargo_target_dir: optional_path(request, "shared_cargo_target_dir")
                .or_else(|| optional_path(request, "cargo_target_dir")),
            shared_cargo_build_dir: optional_path(request, "shared_cargo_build_dir")
                .or_else(|| optional_path(request, "cargo_build_dir")),
            suites,
            suite_values,
            ci_config: request
                .get("ci_config")
                .and_then(JsonValue::as_object)
                .cloned()
                .unwrap_or_default(),
            materialized_files: materialized_files_from_request(request)?,
            prewarm_commands,
            env: request
                .get("env")
                .and_then(JsonValue::as_object)
                .cloned()
                .unwrap_or_default(),
            cleanup_workspace: optional_bool(request, "cleanup_workspace")?
                .unwrap_or(runtime_paths.rust_owned),
            dependency_evidence: string_array(request, "dependency_evidence")?,
            compliance_evidence: string_array(request, "compliance_evidence")?,
            task_batch_inputs: request
                .get("task_batch_inputs")
                .and_then(JsonValue::as_object)
                .cloned()
                .unwrap_or_default(),
            admitted_cpu_tokens: optional_i64(request, "admitted_cpu_tokens")?,
            host_cpu_cores: optional_i64(request, "host_cpu_cores")?,
            scheduler_posture: optional_text(request, "scheduler_posture"),
            main_seed_root: optional_path(request, "main_seed_root"),
            main_seed_path: optional_path(request, "main_seed_path"),
            ram_shard_root: optional_path(request, "ram_shard_root"),
            platform: optional_text(request, "platform"),
            materialization_strategy: optional_text(request, "materialization_strategy"),
        })
    }
}
