use super::*;

#[derive(Debug, Clone)]
pub(super) struct PatchsetCiRuntimeConfig {
    pub(super) patchset_id: String,
    pub(super) change_id: String,
    pub(super) repo_name: String,
    pub(super) repo_id: Option<String>,
    pub(super) change_seq: Option<JsonValue>,
    pub(super) patchset_number: Option<JsonValue>,
    pub(super) base_snapshot_id: String,
    pub(super) revision_snapshot_id: String,
    pub(super) ci_run_seq: u32,
    pub(super) trigger: String,
    pub(super) execution_profile: String,
    pub(super) workspace_path: PathBuf,
    pub(super) runtime_cleanup_workspace_path: PathBuf,
    pub(super) output_dir: PathBuf,
    pub(super) temp_dir: Option<PathBuf>,
    pub(super) shared_cargo_target_dir: Option<PathBuf>,
    pub(super) shared_cargo_build_dir: Option<PathBuf>,
    pub(super) suites: Vec<PatchsetSuiteManifest>,
    pub(super) suite_values: Vec<JsonValue>,
    pub(super) materialized_files: Vec<MaterializedFile>,
    pub(super) prewarm_commands: Vec<String>,
    pub(super) main_seed_prewarm: Option<JsonValue>,
    pub(super) snapshot_materialization_result: Option<JsonValue>,
    pub(super) scheduler_admission: Option<JsonValue>,
    pub(super) suite_pool_tokens: i64,
    pub(super) flow: PatchsetCiFlowConfig,
    pub(super) env: JsonMap<String, JsonValue>,
    pub(super) policy_mode: String,
    pub(super) cleanup_workspace: bool,
    pub(super) tg1: JsonMap<String, JsonValue>,
}

impl PatchsetCiRuntimeConfig {
    pub(super) fn from_request(request: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let patchset = required_object(request, "patchset")?;
        let change = required_object(request, "change")?;
        let patchset_id = required_text(patchset, "patchset_id")?;
        let change_id = optional_text(change, "change_id")
            .unwrap_or_else(|| required_text(patchset, "change_id").unwrap_or_default());
        if change_id.is_empty() {
            return Err(
                "Patchset CI runtime requires change.change_id or patchset.change_id.".to_string(),
            );
        }
        let repo_name = required_text(change, "repo_name")?;
        let base_snapshot_id = required_text(patchset, "base_snapshot_id")?;
        let revision_snapshot_id = required_text(patchset, "revision_snapshot_id")
            .or_else(|_| required_text(patchset, "snapshot_id"))?;
        let ci_run_seq = patchset
            .get("ci_run_seq")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| "Patchset CI runtime requires patchset.ci_run_seq".to_string())?;
        let flow = PatchsetCiFlowConfig::from_request(request)?;
        let execution_profile = normalize_execution_profile(
            optional_text(request, "execution_profile").or_else(|| flow.default_profile()),
        )?;
        let runtime_paths = ci_runtime_paths_from_request(request, "patchset-ci", &patchset_id)?;
        let runtime_cleanup_workspace_path =
            optional_path(request, "runtime_cleanup_workspace_path")
                .unwrap_or_else(|| runtime_paths.workspace_path.clone());
        let suites_raw = request
            .get("suites")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "Field `suites` must be an array of CI suite manifests.".to_string())?;
        let mut suites = Vec::with_capacity(suites_raw.len());
        let mut suite_values = Vec::with_capacity(suites_raw.len());
        for suite in suites_raw {
            suites.push(
                serde_json::from_value::<PatchsetSuiteManifest>(suite.clone())
                    .map_err(|exc| format!("patchset CI suite manifest is invalid: {exc}"))?,
            );
            suite_values.push(suite.clone());
        }
        let materialized_files = materialized_files_from_request(request)?;
        let mut prewarm_commands = string_array(request, "prewarm_commands")?;
        if prewarm_commands.is_empty() {
            prewarm_commands = request
                .get("prewarm")
                .and_then(JsonValue::as_object)
                .map(|prewarm| string_array(prewarm, "commands"))
                .unwrap_or_else(|| Ok(Vec::new()))?;
        }
        Ok(Self {
            patchset_id,
            change_id,
            repo_name,
            repo_id: optional_text(change, "repo_id")
                .or_else(|| optional_text(patchset, "repo_id")),
            change_seq: change.get("change_seq").cloned(),
            patchset_number: patchset.get("patchset_number").cloned(),
            base_snapshot_id,
            revision_snapshot_id,
            ci_run_seq,
            trigger: optional_text(request, "trigger")
                .unwrap_or_else(|| "manual_rerun".to_string()),
            execution_profile,
            workspace_path: runtime_paths.workspace_path,
            runtime_cleanup_workspace_path,
            output_dir: runtime_paths.output_dir,
            temp_dir: Some(runtime_paths.temp_dir),
            shared_cargo_target_dir: optional_path(request, "shared_cargo_target_dir")
                .or_else(|| optional_path(request, "cargo_target_dir")),
            shared_cargo_build_dir: optional_path(request, "shared_cargo_build_dir")
                .or_else(|| optional_path(request, "cargo_build_dir")),
            suites,
            suite_values,
            materialized_files,
            prewarm_commands,
            main_seed_prewarm: request.get("main_seed_prewarm").cloned(),
            snapshot_materialization_result: request
                .get("snapshot_materialization_result")
                .cloned(),
            scheduler_admission: request.get("scheduler_admission").cloned().or_else(|| {
                request
                    .get("snapshot_materialization_result")
                    .and_then(|value| value.get("scheduler_admission"))
                    .cloned()
            }),
            suite_pool_tokens: flow.suite_pool_tokens(
                optional_i64(request, "suite_pool_tokens")
                    .or_else(|| optional_i64(request, "admitted_cpu_tokens")),
            )?,
            flow,
            env: request
                .get("env")
                .and_then(JsonValue::as_object)
                .cloned()
                .unwrap_or_default(),
            policy_mode: optional_text(request, "policy_mode")
                .unwrap_or_else(|| "inline".to_string()),
            cleanup_workspace: optional_bool(request, "cleanup_workspace")?
                .unwrap_or(runtime_paths.rust_owned),
            tg1: request
                .get("tg1")
                .and_then(JsonValue::as_object)
                .cloned()
                .unwrap_or_default(),
        })
    }

    pub(super) fn validate_flow(&self) -> Result<(), String> {
        if self.flow.shared_cargo_target_required && self.shared_cargo_target_dir.is_none() {
            return Err(
                "tg1_patchset_ci flow requires `shared_cargo_target_dir` so Cargo artifacts stay in the repository shared cache."
                    .to_string(),
            );
        }
        if self.flow.prewarm_required
            && self.prewarm_commands.is_empty()
            && self.main_seed_prewarm.is_none()
        {
            return Err(
                "tg1_patchset_ci flow requires prewarm evidence before suite execution."
                    .to_string(),
            );
        }
        Ok(())
    }
}
