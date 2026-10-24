use crate::json_support::{json, JsonMap, JsonValue};
use crate::plan_http_client::{
    build_plan_http_request_spec, configured_repository_authority_path_segment,
    encode_path_segment, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult,
    PlanHttpRequestSpec,
};
use crate::text_normalization::normalize_optional_text;
use crate::workflow_primitives;
use reqwest::Method;

pub struct PatchsetJson<S> {
    store: S,
}

impl<S> PatchsetJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl PatchsetJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> PatchsetJson<S> {
    pub fn build_publish_patchset_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let change_id =
            encode_path_segment(&require_plan_http_non_empty_text(change_id, "change_id")?);
        let body = self.build_publish_patchset_body(
            base_snapshot_id,
            revision_snapshot_id,
            summary,
            author_mode,
        )?;
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/changes/{change_id}/patchsets"
        );
        build_plan_http_request_spec(config, Method::POST, &path, Vec::new(), Some(body))
    }

    pub fn build_list_patchsets_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let change_id =
            encode_path_segment(&require_plan_http_non_empty_text(change_id, "change_id")?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/changes/{change_id}/patchsets"
        );
        build_plan_http_request_spec(config, Method::GET, &path, Vec::new(), None)
    }

    pub fn build_get_patchset_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let patchset_id = encode_path_segment(&require_plan_http_non_empty_text(
            patchset_id,
            "patchset_id",
        )?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path =
            format!("/v1/native/repository-authorities/{repository_index}/patchsets/{patchset_id}");
        build_plan_http_request_spec(
            config,
            Method::GET,
            &path,
            patchset_lookup_query_pairs(change_ref),
            None,
        )
    }

    pub fn build_select_patchset_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        change_id: &str,
        patchset_id: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let change_id =
            encode_path_segment(&require_plan_http_non_empty_text(change_id, "change_id")?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/changes/{change_id}:selectPatchset"
        );
        build_plan_http_request_spec(
            config,
            Method::POST,
            &path,
            Vec::new(),
            Some(self.build_select_patchset_body(patchset_id)?),
        )
    }

    pub fn build_run_patchset_ci_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let patchset_id = encode_path_segment(&require_plan_http_non_empty_text(
            patchset_id,
            "patchset_id",
        )?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/patchsets/{patchset_id}:runCi"
        );
        build_plan_http_request_spec(
            config,
            Method::POST,
            &path,
            Vec::new(),
            Some(self.build_run_patchset_ci_body(trigger, execution_profile)?),
        )
    }

    pub fn build_read_patchset_ci_status_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        patchset_id: &str,
        recent_limit: i64,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let patchset_id = encode_path_segment(&require_plan_http_non_empty_text(
            patchset_id,
            "patchset_id",
        )?);
        let path = patchset_ci_status_path(config, &patchset_id)?;
        build_plan_http_request_spec(
            config,
            Method::GET,
            &path,
            self.patchset_ci_status_query_pairs(recent_limit),
            None,
        )
    }

    pub fn build_read_patchset_ci_readiness_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        patchset_id: &str,
        recent_limit: i64,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let patchset_id = encode_path_segment(&require_plan_http_non_empty_text(
            patchset_id,
            "patchset_id",
        )?);
        let path = patchset_ci_status_path(config, &patchset_id)?;
        build_plan_http_request_spec(
            config,
            Method::GET,
            &path,
            self.patchset_ci_readiness_query_pairs(recent_limit),
            None,
        )
    }

    pub fn build_publish_patchset_body(
        &self,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
    ) -> PlanHttpClientResult<JsonValue> {
        let _ = &self.store;
        let base_snapshot_id =
            require_plan_http_non_empty_text(base_snapshot_id, "base_snapshot_id")?;
        let revision_snapshot_id =
            require_plan_http_non_empty_text(revision_snapshot_id, "revision_snapshot_id")?;
        let summary = require_plan_http_non_empty_text(summary, "summary")?;
        let author_mode = require_plan_http_non_empty_text(author_mode, "author_mode")?;
        Ok(json!({
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "author_mode": author_mode,
        }))
    }

    pub fn build_select_patchset_body(&self, patchset_id: &str) -> PlanHttpClientResult<JsonValue> {
        let _ = &self.store;
        let patchset_id = require_plan_http_non_empty_text(patchset_id, "patchset_id")?;
        Ok(json!({ "patchset_id": patchset_id }))
    }

    pub fn build_run_patchset_ci_body(
        &self,
        trigger: &str,
        execution_profile: Option<&str>,
    ) -> PlanHttpClientResult<JsonValue> {
        let _ = &self.store;
        let trigger = require_plan_http_non_empty_text(trigger, "trigger")?;
        let mut body = JsonMap::new();
        body.insert("trigger".to_string(), JsonValue::String(trigger));
        insert_optional_string(&mut body, "execution_profile", execution_profile);
        Ok(JsonValue::Object(body))
    }

    pub fn patchset_ci_status_query_pairs(&self, recent_limit: i64) -> Vec<(String, String)> {
        let _ = &self.store;
        let limit = if recent_limit < 1 { 1 } else { recent_limit };
        vec![("recent_limit".to_string(), limit.to_string())]
    }

    pub fn patchset_ci_readiness_query_pairs(&self, recent_limit: i64) -> Vec<(String, String)> {
        let _ = &self.store;
        let limit = recent_limit.clamp(1, 20);
        vec![
            ("recent_limit".to_string(), limit.to_string()),
            ("projection".to_string(), "readiness".to_string()),
        ]
    }

    pub fn normalize_patchset_payload(&self, payload: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "patchset payload")?.clone(),
        ))
    }

    pub fn normalize_patchset_list_payload(
        &self,
        payload: Vec<JsonValue>,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        let mut normalized = Vec::with_capacity(payload.len());
        for value in payload {
            let object = require_object(Some(&value), "patchset list entry")?;
            normalized.push(JsonValue::Object(object.clone()));
        }
        Ok(JsonValue::Array(normalized))
    }

    pub fn normalize_patchset_ci_status_payload(
        &self,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "patchset CI status payload")?.clone(),
        ))
    }

    pub fn optional_patchset_id(&self, patchset: &JsonValue) -> Option<String> {
        let _ = &self.store;
        normalize_optional_text(patchset.get("patchset_id").and_then(JsonValue::as_str))
    }

    pub fn resolved_patchset_id_from_payload(
        &self,
        patchset: &JsonValue,
        fallback_patchset_id: &str,
    ) -> String {
        self.optional_patchset_id(patchset)
            .unwrap_or_else(|| fallback_patchset_id.to_string())
    }

    pub fn patchset_number(&self, patchset: &JsonValue) -> i64 {
        let _ = &self.store;
        patchset
            .get("patchset_number")
            .and_then(JsonValue::as_i64)
            .unwrap_or_default()
    }

    pub fn derive_patchset_id(
        &self,
        change_id: &str,
        patchset_number: i64,
        namespace_prefix: Option<&str>,
    ) -> Result<String, String> {
        let _ = &self.store;
        workflow_primitives::derive_patchset_id(change_id, patchset_number, namespace_prefix)
    }

    pub fn recover_published_patchset_from_rows(
        &self,
        rows: Vec<JsonValue>,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        prior_patchset_number: i64,
    ) -> Option<JsonValue> {
        let _ = &self.store;
        let mut candidates = rows
            .into_iter()
            .filter(|row| {
                self.patchset_matches_publish_request(
                    row,
                    base_snapshot_id,
                    revision_snapshot_id,
                    prior_patchset_number,
                )
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by_key(|row| self.patchset_number(row));
        candidates
            .pop()
            .map(|patchset| self.attach_publish_response_recovery(patchset, change_id))
    }

    pub fn patchset_matches_publish_request(
        &self,
        row: &JsonValue,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        prior_patchset_number: i64,
    ) -> bool {
        let _ = &self.store;
        row.get("base_snapshot_id").and_then(JsonValue::as_str) == Some(base_snapshot_id)
            && row.get("revision_snapshot_id").and_then(JsonValue::as_str)
                == Some(revision_snapshot_id)
            && self.patchset_number(row) > prior_patchset_number
    }

    pub fn attach_publish_response_recovery(
        &self,
        mut patchset: JsonValue,
        change_id: &str,
    ) -> JsonValue {
        let _ = &self.store;
        if let Some(object) = patchset.as_object_mut() {
            object.insert(
                "response_recovery".to_string(),
                json!({
                    "action": "publish_patchset",
                    "state": "recovered_from_remote_publish",
                    "change_id": change_id,
                }),
            );
        }
        patchset
    }
}

fn patchset_ci_status_path(
    config: &PlanHttpClientConfig,
    encoded_patchset_id: &str,
) -> PlanHttpClientResult<String> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    Ok(format!(
        "/v1/native/repository-authorities/{repository_index}/read/patchsets/{encoded_patchset_id}/ci-status"
    ))
}

fn patchset_lookup_query_pairs(change_ref: Option<&str>) -> Vec<(String, String)> {
    normalize_optional_text(change_ref)
        .map(|change_ref| vec![("change_ref".to_string(), change_ref)])
        .unwrap_or_default()
}

fn insert_optional_string(body: &mut JsonMap<String, JsonValue>, key: &str, value: Option<&str>) {
    if let Some(value) = normalize_optional_text(value) {
        body.insert(key.to_string(), JsonValue::String(value));
    }
}

fn require_plan_http_non_empty_text(value: &str, label: &str) -> PlanHttpClientResult<String> {
    normalize_optional_text(Some(value)).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!("Plan HTTP `{label}` must be non-empty."))
    })
}

fn require_object<'a>(
    value: Option<&'a JsonValue>,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("{label} must be an object."))
}

#[cfg(test)]
mod tests;
