use crate::repo_surface::{repo_command, RepoCommandRequest};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonCodec, JsonMap, JsonValue};

pub const FULL_TEST_SUITE_ID: &str = "full_repo";
pub const FULL_TEST_ZSTD_ONLY_SUITE_ID: &str = "full_repo_zstd_only";
pub const FULL_TEST_ZSTD_ONLY_VARIANT: &str = "zstd_only";
pub const DEFAULT_FULL_TEST_PLANE: &str = "nightly";
pub const DEFAULT_FULL_TEST_TRIGGER: &str = "manual_full_test";
const PATCH_CI_CATALOG: &str = include_str!("../../../../ci/patch_ci.json");

#[derive(Clone, Debug)]
pub struct TestRunFullRequest {
    pub remote_name: Option<String>,
    pub json_output: bool,
    pub variant: Option<String>,
    pub plane: String,
    pub target_line: String,
    pub trigger: String,
}

impl Default for TestRunFullRequest {
    fn default() -> Self {
        Self {
            remote_name: None,
            json_output: false,
            variant: None,
            plane: DEFAULT_FULL_TEST_PLANE.to_string(),
            target_line: "main".to_string(),
            trigger: DEFAULT_FULL_TEST_TRIGGER.to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TestStatusRequest {
    pub remote_name: Option<String>,
    pub json_output: bool,
    pub plane: String,
    pub suite_id: String,
    pub limit: i64,
}

impl Default for TestStatusRequest {
    fn default() -> Self {
        Self {
            remote_name: None,
            json_output: false,
            plane: DEFAULT_FULL_TEST_PLANE.to_string(),
            suite_id: FULL_TEST_SUITE_ID.to_string(),
            limit: 20,
        }
    }
}

pub fn test_run_full(
    repo: &RepoRuntime,
    request: &TestRunFullRequest,
) -> Result<JsonValue, String> {
    let suite_id = full_test_suite_id(request.variant.as_deref())?;
    if !matches!(
        request.plane.as_str(),
        "nightly" | "release" | "post_land_regression"
    ) {
        return Err(format!(
            "Unsupported repo CI plane {:?}; expected nightly, release, or post_land_regression",
            request.plane,
        ));
    }
    let mut args = JsonMap::new();
    args.insert("suite_ids".to_string(), json!([suite_id]));
    args.insert("plane".to_string(), json!(request.plane));
    args.insert("target_line".to_string(), json!(request.target_line));
    args.insert("trigger".to_string(), json!(request.trigger));
    args.insert("selector".to_string(), JsonValue::Null);
    args.insert("task_ids".to_string(), json!([]));
    args.insert("curated_corpus".to_string(), JsonValue::Null);
    args.insert("count".to_string(), JsonValue::Null);
    args.insert("window_days".to_string(), JsonValue::Null);
    args.insert("dependency_evidence".to_string(), json!([]));
    args.insert("compliance_evidence".to_string(), json!([]));

    repo_command(
        repo,
        &RepoCommandRequest {
            command: "run-ci".to_string(),
            remote_name: request.remote_name.clone(),
            json_output: request.json_output,
            args,
        },
    )
}

fn full_test_suite_id(variant: Option<&str>) -> Result<&'static str, String> {
    let suite_id = match variant.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(FULL_TEST_SUITE_ID),
        Some("compatibility") => Ok(FULL_TEST_SUITE_ID),
        Some(FULL_TEST_ZSTD_ONLY_VARIANT) => Ok(FULL_TEST_ZSTD_ONLY_SUITE_ID),
        Some(variant) => Err(format!(
            "Native `ait-cli test run --full` does not support variant {variant:?}; supported variants are compatibility and zstd_only"
        )),
    }?;
    catalog_suite_declared(suite_id)?;
    Ok(suite_id)
}

fn catalog_suite_declared(suite_id: &str) -> Result<(), String> {
    let catalog = JsonCodec::parse_value(PATCH_CI_CATALOG, "compiled ci/patch_ci.json")
        .map_err(|error| error.to_string())?;
    catalog
        .get("suites")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .find(|suite| suite.get("suite_id").and_then(JsonValue::as_str) == Some(suite_id))
        .map(|_| ())
        .ok_or_else(|| {
            format!(
                "Full-test suite {suite_id:?} is not declared in ci/patch_ci.json; refusing an unknown server job"
            )
        })
}

pub fn test_status(repo: &RepoRuntime, request: &TestStatusRequest) -> Result<JsonValue, String> {
    let mut args = JsonMap::new();
    args.insert("limit".to_string(), json!(request.limit.max(1)));
    let runs = repo_command(
        repo,
        &RepoCommandRequest {
            command: "ci-runs".to_string(),
            remote_name: request.remote_name.clone(),
            json_output: request.json_output,
            args,
        },
    )?;
    let latest = latest_run(&runs);
    let status = latest
        .as_ref()
        .and_then(|run| {
            text_field(run, "status")
                .or_else(|| text_field(run, "state"))
                .or_else(|| text_field(run, "tests_status"))
        })
        .unwrap_or_else(|| "unknown".to_string());
    Ok(json!({
        "surface": "ait.test.status",
        "suite_id": request.suite_id,
        "plane": request.plane,
        "limit": request.limit.max(1),
        "status": status,
        "latest": latest.unwrap_or(JsonValue::Null),
        "runs": runs,
    }))
}

fn latest_run(payload: &JsonValue) -> Option<JsonValue> {
    if let Some(values) = payload.as_array() {
        return values.first().cloned();
    }
    if let Some(values) = payload.get("jobs").and_then(JsonValue::as_array) {
        return values
            .iter()
            .find(|run| text_field(run, "job_type").as_deref() == Some("repo.ci"))
            .cloned();
    }
    if let Some(values) = payload.get("runs").and_then(JsonValue::as_array) {
        return values.first().cloned();
    }
    if payload.is_object() {
        return Some(payload.clone());
    }
    None
}

fn text_field(payload: &JsonValue, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_test_variants_are_catalog_backed_and_use_repo_ci_plane() {
        catalog_suite_declared(FULL_TEST_SUITE_ID).unwrap();
        catalog_suite_declared(FULL_TEST_ZSTD_ONLY_SUITE_ID).unwrap();
        assert_eq!(DEFAULT_FULL_TEST_PLANE, "nightly");
        assert_eq!(full_test_suite_id(None).unwrap(), FULL_TEST_SUITE_ID);
        assert_eq!(
            full_test_suite_id(Some(FULL_TEST_ZSTD_ONLY_VARIANT)).unwrap(),
            FULL_TEST_ZSTD_ONLY_SUITE_ID
        );
    }

    #[test]
    fn undeclared_suite_fails_before_remote_dispatch() {
        let error = catalog_suite_declared("full_repo_missing").unwrap_err();
        assert!(error.contains("full_repo_missing"));
        assert!(error.contains("ci/patch_ci.json"));
    }

    #[test]
    fn latest_run_selects_repo_ci_from_worker_job_projection() {
        let payload = json!({
            "repository_index": 7,
            "jobs": [
                {
                    "worker_job_index": 9,
                    "job_type": "patchset.ci",
                    "state": "running"
                },
                {
                    "worker_job_index": 8,
                    "job_type": "repo.ci",
                    "state": "succeeded"
                }
            ]
        });

        let latest = latest_run(&payload).expect("repo.ci Worker Job");
        assert_eq!(latest["worker_job_index"], 8);
        assert_eq!(latest["state"], "succeeded");
    }
}
