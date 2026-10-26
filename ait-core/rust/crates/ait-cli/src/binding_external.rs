//! Existing External selection and diagnostic policy shared by frontends.
use crate::{binding_services::WorkflowOperationResult, runtime::RepoRuntime};
use ait_core::external::update::ExternalUpdateOptions;

#[derive(Clone, Debug, Default)]
pub struct ExternalUpdateRequest {
    pub name: Option<String>,
    pub snapshot: Option<String>,
    pub latest: bool,
    pub locked: bool,
    pub validate: bool,
    pub no_recursive: bool,
}

pub fn external_update_options(
    request: &ExternalUpdateRequest,
) -> Result<ExternalUpdateOptions, String> {
    if request.snapshot.is_some() && request.latest {
        return Err(
            "`ait external update` accepts either `--to <snapshot>` or `--latest`, not both."
                .to_string(),
        );
    }
    if request.locked && (request.name.is_some() || request.snapshot.is_some() || request.latest) {
        return Err("`ait external update --locked` restores the files recorded by the committed lockfile and does not accept a target, `--to`, or `--latest`.".to_string());
    }
    let mut options = match (&request.name, &request.snapshot, request.latest) {
        (Some(name), Some(snapshot), false) => ExternalUpdateOptions::exact(name, snapshot),
        (Some(name), None, true) => ExternalUpdateOptions::latest(name),
        (None, None, false) => ExternalUpdateOptions::manifest_pins(),
        (None, Some(_), false) => {
            return Err(
                "`ait external update --to <snapshot>` requires an external name.".to_string(),
            );
        }
        (None, None, true) => {
            return Err("`ait external update --latest` requires an external name.".to_string());
        }
        (Some(_), None, false) => {
            return Err(
                "`ait external update <name>` requires `--to <snapshot>` or `--latest`."
                    .to_string(),
            );
        }
        (_, Some(_), true) => unreachable!("--to and --latest conflict is handled above"),
    };
    options = options
        .with_locked(request.locked)
        .with_validate(request.validate)
        .with_no_recursive(request.no_recursive);
    Ok(options)
}

pub fn update_external(
    repo: &RepoRuntime,
    request: &ExternalUpdateRequest,
) -> Result<ait_core::json_support::JsonValue, String> {
    crate::external_surface::external_update(repo, external_update_options(request)?)
}

pub fn diagnose_external(
    repo: &RepoRuntime,
    fail_on_blocking: bool,
) -> Result<WorkflowOperationResult, String> {
    let payload = crate::external_surface::external_doctor(repo)?;
    let release_ready = payload
        .get("release_ready")
        .and_then(ait_core::json_support::JsonValue::as_bool)
        .ok_or_else(|| "external doctor payload is missing Boolean `release_ready`".to_string())?;
    let exit_code = if fail_on_blocking && !release_ready {
        2
    } else {
        0
    };
    Ok(WorkflowOperationResult { payload, exit_code })
}
