use super::*;

#[derive(Debug, Clone)]
pub(super) struct PatchsetCiFlowConfig {
    pub(super) kind: String,
    pub(super) contract: String,
    pub(super) include_modes: Vec<String>,
    pub(super) prewarm_required: bool,
    pub(super) fixed_cpu_tokens: Option<i64>,
    pub(super) require_exact_cpu_tokens: bool,
    pub(super) rust_runner_only: bool,
    pub(super) shared_cargo_target_required: bool,
    pub(super) finish_after_all_suites: bool,
}

impl PatchsetCiFlowConfig {
    fn default_legacy() -> Self {
        Self {
            kind: "legacy".to_string(),
            contract: "ait.server.patchset_ci.legacy.v1".to_string(),
            include_modes: Vec::from(["gate".to_string()]),
            prewarm_required: false,
            fixed_cpu_tokens: None,
            require_exact_cpu_tokens: false,
            rust_runner_only: false,
            shared_cargo_target_required: false,
            finish_after_all_suites: false,
        }
    }

    pub(super) fn from_request(request: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let Some(flow) = request_flow_config(request) else {
            return Ok(Self::default_legacy());
        };
        let kind = optional_text(flow, "kind").unwrap_or_else(|| "legacy".to_string());
        if !matches!(kind.as_str(), "legacy" | "tg1_patchset_ci") {
            return Err(format!("Unsupported patchset CI flow kind `{kind}`."));
        }
        if kind == "legacy" {
            return Ok(Self::default_legacy());
        }

        let suite_selection = flow
            .get("suite_selection")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let mut include_modes = string_array(&suite_selection, "include_modes")?;
        if include_modes.is_empty() {
            include_modes = Vec::from(["gate".to_string()]);
        }
        include_modes = include_modes
            .into_iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect();

        let prewarm = flow
            .get("prewarm")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let parallelism = flow
            .get("parallelism")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let runner_authority = flow
            .get("runner_authority")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let cargo = flow
            .get("cargo")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let finish = flow
            .get("finish")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let fixed_cpu_tokens = optional_i64(&parallelism, "cpu_tokens")
            .or_else(|| optional_i64(&parallelism, "fixed_cpu_tokens"))
            .unwrap_or(TG1_REQUIRED_CPU_TOKENS);

        Ok(Self {
            kind,
            contract: optional_text(flow, "contract")
                .unwrap_or_else(|| TG1_PATCHSET_CI_FLOW_CONTRACT.to_string()),
            include_modes,
            prewarm_required: optional_bool(&prewarm, "required")?.unwrap_or(true),
            fixed_cpu_tokens: Some(fixed_cpu_tokens.max(1)),
            require_exact_cpu_tokens: optional_bool(&parallelism, "require_exact")?.unwrap_or(true),
            rust_runner_only: optional_bool(&runner_authority, "rust_only")?.unwrap_or(true),
            shared_cargo_target_required: optional_bool(&cargo, "shared_target_required")?
                .unwrap_or(true),
            finish_after_all_suites: optional_text(&finish, "policy")
                .map(|value| value == "aggregate_after_all_suites")
                .unwrap_or(true),
        })
    }

    pub(super) fn is_tg1_patchset_ci(&self) -> bool {
        self.kind == "tg1_patchset_ci"
    }

    pub(super) fn default_profile(&self) -> Option<String> {
        self.is_tg1_patchset_ci()
            .then(|| PATCHSET_CI_PROFILE_TG1_FLOW.to_string())
    }

    pub(super) fn suite_pool_tokens(&self, requested: Option<i64>) -> Result<i64, String> {
        let requested = requested
            .or(self.fixed_cpu_tokens)
            .unwrap_or(PATCHSET_CI_DEFAULT_SUITE_POOL_TOKENS)
            .max(1);
        if self.require_exact_cpu_tokens {
            let expected = self.fixed_cpu_tokens.unwrap_or(TG1_REQUIRED_CPU_TOKENS);
            if requested > expected {
                return Err(format!(
                    "tg1_patchset_ci flow allows at most {expected} scheduler CPU token(s); got {requested}."
                ));
            }
        }
        Ok(requested)
    }

    pub(super) fn includes_mode(&self, mode: &str) -> bool {
        self.include_modes
            .iter()
            .any(|value| value == &mode.trim().to_ascii_lowercase())
    }
}

fn request_flow_config(
    request: &JsonMap<String, JsonValue>,
) -> Option<&JsonMap<String, JsonValue>> {
    request
        .get("patchset_ci_flow")
        .or_else(|| request.get("flow"))
        .and_then(JsonValue::as_object)
        .or_else(|| suite_manifest_flow_config(request))
}

fn suite_manifest_flow_config(
    request: &JsonMap<String, JsonValue>,
) -> Option<&JsonMap<String, JsonValue>> {
    request
        .get("suites")
        .and_then(JsonValue::as_array)?
        .iter()
        .find_map(|suite| {
            suite.as_object().and_then(|suite| {
                suite
                    .get("patchset_ci_flow")
                    .or_else(|| suite.get("flow"))
                    .and_then(JsonValue::as_object)
            })
        })
}

pub(super) fn selected_patchset_suites(
    config: &PatchsetCiRuntimeConfig,
) -> Result<Vec<PatchsetSuiteManifest>, String> {
    let mut selected = config
        .suites
        .iter()
        .filter(|suite| suite.plane.trim().eq_ignore_ascii_case("patchset"))
        .filter(|suite| config.flow.includes_mode(suite.mode.trim()))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| left.suite_id.trim().cmp(right.suite_id.trim()));
    for suite in &selected {
        if suite.suite_id.trim().is_empty() {
            return Err("patchset CI suite manifest requires `suite_id`.".to_string());
        }
    }
    Ok(selected)
}

pub(super) fn suites_for_execution_profile(
    suites: &[PatchsetSuiteManifest],
    execution_profile: &str,
) -> Result<Vec<PatchsetSuiteManifest>, String> {
    match execution_profile {
        PATCHSET_CI_PROFILE_FULL => Ok(suites.to_vec()),
        PATCHSET_CI_PROFILE_TG1_FLOW => Ok(suites.to_vec()),
        PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND => Ok(suites
            .iter()
            .filter(|suite| {
                suite.default_blocking || suite.suite_id.trim() == TG1_REQUIRED_SUITE_ID
            })
            .cloned()
            .collect()),
        _ => Err(format!(
            "Unsupported patchset CI execution_profile `{execution_profile}`."
        )),
    }
}

pub(super) fn normalize_execution_profile(value: Option<String>) -> Result<String, String> {
    let profile = value.unwrap_or_else(|| PATCHSET_CI_PROFILE_FULL.to_string());
    let profile = profile.trim();
    match profile {
        PATCHSET_CI_PROFILE_FULL
        | PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND
        | PATCHSET_CI_PROFILE_TG1_FLOW => Ok(profile.to_string()),
        _ => Err(format!(
            "Unsupported patchset CI execution_profile `{profile}`."
        )),
    }
}
