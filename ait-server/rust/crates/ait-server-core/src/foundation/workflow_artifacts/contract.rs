use super::*;

pub const WORKFLOW_ARTIFACTS_CONTRACT: &str = "ait.server.workflow_artifacts.v1";
pub const WORKFLOW_ARTIFACTS_REFERENCE_MODULE: &str = "../ait/src/ait_native/server_api.py";

pub fn workflow_artifacts_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "workflow-artifacts payload must be a JSON object.".to_string())?;
    match operation {
        "contract" => Ok(json!({
            "contract": WORKFLOW_ARTIFACTS_CONTRACT,
            "reference_module": WORKFLOW_ARTIFACTS_REFERENCE_MODULE,
            "mutates_state": false,
            "operations": [
                "release-artifact-media-type",
                "sanitize-release-artifact-path",
                "release-artifact-view",
                "release-artifact-pack",
                "release-formula-payload",
                "release-row",
                "patchset-changed-paths",
                "dedupe-text-values",
                "suite-manifest-catalog-path",
                "coerce-suite-catalog-payload",
                "patchset-rollout-suite-ids",
                "ci-rollout-summary-message",
                "ci-rollout-suite-checks",
                "policy-status-view",
                "effective-policy-status",
                "requires-code-review-summary",
                "review-decision-lane",
                "review-summary",
                "attestation-id-for-patchset",
                "land-submission-id-for-change"
            ],
        })),
        "release-artifact-media-type" => Ok(json!({
            "contract": WORKFLOW_ARTIFACTS_CONTRACT,
            "media_type": release_artifact_media_type(
                optional_text(payload.get("kind")).as_deref().unwrap_or(""),
                optional_text(payload.get("path")).as_deref().unwrap_or(""),
            ),
        })),
        "sanitize-release-artifact-path" => Ok(json!({
            "contract": WORKFLOW_ARTIFACTS_CONTRACT,
            "artifact_path": sanitize_release_artifact_path(optional_text(payload.get("path")).as_deref()),
        })),
        "release-artifact-view" => {
            let release_id = required_text(payload.get("release_id"), "release_id")?;
            let artifact = required_object(payload.get("artifact"), "artifact")?;
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "artifact": release_artifact_view(&release_id, artifact),
            }))
        }
        "release-artifact-pack" => {
            let release_id = required_text(payload.get("release_id"), "release_id")?;
            let artifact = required_object(payload.get("artifact"), "artifact")?;
            let validation = validate_release_artifact_pack(&release_id, artifact)?;
            let mut out = json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "reference_module": RELEASES_REFERENCE_MODULE,
                "artifact": validation.artifact,
            });
            if truthy(payload.get("include_content_bytes")) {
                out["content_bytes"] = JsonValue::Array(
                    validation
                        .content
                        .into_iter()
                        .map(|byte| json!(byte))
                        .collect(),
                );
            }
            Ok(out)
        }
        "release-formula-payload" => {
            let formula = optional_object(payload.get("formula"));
            let artifacts = payload
                .get("artifacts")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_object)
                .cloned()
                .collect::<Vec<_>>();
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "reference_module": RELEASES_REFERENCE_MODULE,
                "formula": release_formula_payload(formula, &artifacts),
            }))
        }
        "release-row" => {
            let row = payload
                .get("row")
                .and_then(JsonValue::as_object)
                .unwrap_or(payload);
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "release": release_row(row)?,
            }))
        }
        "patchset-changed-paths" => {
            let patchset = payload
                .get("patchset")
                .and_then(JsonValue::as_object)
                .unwrap_or(payload);
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "changed_paths": patchset_changed_paths(patchset),
            }))
        }
        "dedupe-text-values" => Ok(json!({
            "contract": WORKFLOW_ARTIFACTS_CONTRACT,
            "values": dedupe_text_values(payload.get("values")),
        })),
        "suite-manifest-catalog-path" => {
            let ci_config = optional_object(payload.get("ci_config")).unwrap_or(payload);
            let manifest = optional_object(payload.get("manifest"));
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "catalog_path": suite_manifest_catalog_path(ci_config, manifest),
                "checked_in_ci_contract_path": CHECKED_IN_CI_CONTRACT_PATH,
            }))
        }
        "coerce-suite-catalog-payload" => {
            let catalog_path = required_text(payload.get("catalog_path"), "catalog_path")?;
            let catalog_payload = payload.get("payload").unwrap_or(request);
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "suites": coerce_suite_catalog_payload(catalog_payload, &catalog_path),
            }))
        }
        "patchset-rollout-suite-ids" => {
            let suites_by_id = optional_object(payload.get("suites_by_id")).unwrap_or(payload);
            let (patchset, required, informational) = patchset_rollout_suite_ids(suites_by_id);
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "patchset_suite_ids": patchset,
                "required_suite_ids": required,
                "informational_suite_ids": informational,
            }))
        }
        "ci-rollout-summary-message" => Ok(json!({
            "contract": WORKFLOW_ARTIFACTS_CONTRACT,
            "message": ci_rollout_summary_message(payload),
        })),
        "ci-rollout-suite-checks" => Ok(json!({
            "contract": WORKFLOW_ARTIFACTS_CONTRACT,
            "checks": ci_rollout_patchset_suite_checks(payload),
        })),
        "policy-status-view" => {
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            let decision = required_text(payload.get("decision"), "decision")?;
            let checks = payload
                .get("checks")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            let evaluated_at = optional_text(payload.get("evaluated_at"));
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "status": policy_status_view(&patchset_id, &decision, checks, evaluated_at),
            }))
        }
        "effective-policy-status" => {
            let patchset = required_object(payload.get("patchset"), "patchset")?;
            let latest_status = optional_object(payload.get("latest_status"));
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "status": effective_policy_status(patchset, latest_status)?,
            }))
        }
        "requires-code-review-summary" => Ok(json!({
            "contract": WORKFLOW_ARTIFACTS_CONTRACT,
            "required": requires_code_review_summary(payload),
        })),
        "review-decision-lane" => {
            let action = required_text(payload.get("action"), "action")?;
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "lane": review_decision_lane(&action),
            }))
        }
        "review-summary" => {
            let reviews_value = payload
                .get("reviews")
                .ok_or_else(|| "Field `reviews` must be a JSON array.".to_string())?;
            let reviews = reviews_value
                .as_array()
                .ok_or_else(|| "Field `reviews` must be a JSON array.".to_string())?
                .iter()
                .filter_map(JsonValue::as_object)
                .cloned()
                .collect::<Vec<_>>();
            let patchset_id = optional_text(payload.get("patchset_id"));
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "migration_status": "rust_owned_no_python_reference",
                "summary": review_summary_from_rows(&reviews, patchset_id.as_deref()),
            }))
        }
        "attestation-id-for-patchset" => {
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "attestation_id": attestation_id_for_patchset(&patchset_id)?,
            }))
        }
        "land-submission-id-for-change" => {
            let change_id = required_text(payload.get("change_id"), "change_id")?;
            let prior_request_count = optional_i64(payload.get("prior_request_count"))?
                .ok_or_else(|| {
                    "Field `prior_request_count` must be a non-negative integer.".to_string()
                })?;
            Ok(json!({
                "contract": WORKFLOW_ARTIFACTS_CONTRACT,
                "submission_id": land_submission_id_for_change(&change_id, prior_request_count)?,
            }))
        }
        other => Err(format!(
            "Unsupported workflow-artifacts operation `{other}`."
        )),
    }
}
