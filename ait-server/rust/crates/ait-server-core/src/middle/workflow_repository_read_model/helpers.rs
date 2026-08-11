use super::*;

#[derive(Default)]
pub(super) struct WorkflowDetailIndex<'a> {
    patchsets_by_change: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    reviews_by_change: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    attestations_by_patchset: HashMap<String, &'a JsonMap<String, JsonValue>>,
    policies_by_patchset: HashMap<String, &'a JsonMap<String, JsonValue>>,
    lands_by_change: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    refs_by_repo_line: HashMap<(String, String), String>,
    deltas_by_patchset: HashMap<String, &'a JsonMap<String, JsonValue>>,
    events: Vec<&'a JsonMap<String, JsonValue>>,
}

impl<'a> WorkflowDetailIndex<'a> {
    pub(super) fn new(input: &'a TaskWorkflowDetailInput) -> Self {
        let mut index = Self::default();
        for patchset in &input.patchsets {
            if let Some(change_id) = object_text(patchset, "change_id") {
                index
                    .patchsets_by_change
                    .entry(change_id)
                    .or_default()
                    .push(patchset);
            }
        }
        for patchsets in index.patchsets_by_change.values_mut() {
            patchsets.sort_by(|left, right| patchset_number(left).cmp(&patchset_number(right)));
        }
        for review in &input.reviews {
            if let Some(change_id) = object_text(review, "change_id") {
                index
                    .reviews_by_change
                    .entry(change_id)
                    .or_default()
                    .push(review);
            }
        }
        for reviews in index.reviews_by_change.values_mut() {
            reviews.sort_by(|left, right| {
                int_field(left, "review_id")
                    .cmp(&int_field(right, "review_id"))
                    .then_with(|| {
                        object_text(left, "created_at").cmp(&object_text(right, "created_at"))
                    })
            });
        }
        for attestation in &input.attestations {
            if let Some(patchset_id) = object_text(attestation, "patchset_id") {
                let replace = index
                    .attestations_by_patchset
                    .get(&patchset_id)
                    .map(|existing| {
                        object_text(attestation, "updated_at")
                            >= object_text(existing, "updated_at")
                    })
                    .unwrap_or(true);
                if replace {
                    index
                        .attestations_by_patchset
                        .insert(patchset_id, attestation);
                }
            }
        }
        for policy in &input.policy_decisions {
            if let Some(patchset_id) = object_text(policy, "patchset_id") {
                let replace = index
                    .policies_by_patchset
                    .get(&patchset_id)
                    .map(|existing| {
                        int_field(policy, "policy_decision_id")
                            >= int_field(existing, "policy_decision_id")
                            || object_text(policy, "created_at")
                                >= object_text(existing, "created_at")
                    })
                    .unwrap_or(true);
                if replace {
                    index.policies_by_patchset.insert(patchset_id, policy);
                }
            }
        }
        for land in &input.land_requests {
            if let Some(change_id) = object_text(land, "change_id") {
                index
                    .lands_by_change
                    .entry(change_id)
                    .or_default()
                    .push(land);
            }
        }
        for lands in index.lands_by_change.values_mut() {
            lands.sort_by(|left, right| {
                object_text(right, "created_at").cmp(&object_text(left, "created_at"))
            });
        }
        for row in &input.refs {
            if let (Some(repo_name), Some(line_name), Some(head)) = (
                object_text(row, "repo_name"),
                object_text(row, "line_name"),
                object_text(row, "head_snapshot_id").or_else(|| object_text(row, "snapshot_id")),
            ) {
                index.refs_by_repo_line.insert((repo_name, line_name), head);
            }
        }
        for delta in &input.patchset_deltas {
            if let Some(patchset_id) = object_text(delta, "patchset_id") {
                index.deltas_by_patchset.insert(patchset_id, delta);
            }
        }
        index.events = input.events.iter().collect();
        index.events.sort_by(|left, right| {
            object_text(left, "created_at").cmp(&object_text(right, "created_at"))
        });
        index
    }

    pub(super) fn current_patchset(
        &self,
        change: &JsonMap<String, JsonValue>,
    ) -> Option<&'a JsonMap<String, JsonValue>> {
        self.patchset_for_change_field(change, "current_patchset_id", "current_patchset_number")
            .or_else(|| {
                let change_id = object_text(change, "change_id")?;
                self.patchsets_by_change.get(&change_id)?.last().copied()
            })
    }

    pub(super) fn selected_patchset(
        &self,
        change: &JsonMap<String, JsonValue>,
    ) -> Option<&'a JsonMap<String, JsonValue>> {
        self.patchset_for_change_field(change, "selected_patchset_id", "selected_patchset_number")
    }

    fn patchset_for_change_field(
        &self,
        change: &JsonMap<String, JsonValue>,
        id_field: &str,
        number_field: &str,
    ) -> Option<&'a JsonMap<String, JsonValue>> {
        let change_id = object_text(change, "change_id")?;
        let patchsets = self.patchsets_by_change.get(&change_id)?;
        if let Some(patchset_id) = object_text(change, id_field) {
            if let Some(found) = patchsets.iter().find(|patchset| {
                object_text(patchset, "patchset_id").as_deref() == Some(patchset_id.as_str())
            }) {
                return Some(*found);
            }
        }
        let number = change.get(number_field).and_then(int_value)?;
        patchsets
            .iter()
            .find(|patchset| patchset_number(patchset) == number)
            .copied()
    }

    pub(super) fn review_summary(&self, change_id: &str) -> JsonValue {
        let reviews = self
            .reviews_by_change
            .get(change_id)
            .into_iter()
            .flatten()
            .map(|review| (*review).clone())
            .collect::<Vec<_>>();
        let mut summary = review_summary_from_rows(&reviews, None);
        summary.insert("change_id".to_string(), json!(change_id));
        let review_values = reviews
            .into_iter()
            .map(JsonValue::Object)
            .collect::<Vec<_>>();
        json!({
            "change_id": change_id,
            "reviews": review_values,
            "approvals": summary.get("approval_count").cloned().unwrap_or_else(|| json!(0)),
            "blocking": summary.get("blocking_count").cloned().unwrap_or_else(|| json!(0)),
            "comments": summary.get("comment_count").cloned().unwrap_or_else(|| json!(0)),
            "summary": summary,
        })
    }

    pub(super) fn policy_summary(&self, patchset_id: Option<&str>) -> JsonValue {
        let Some(patchset_id) = patchset_id else {
            return json!({"decision": "pending", "checks": []});
        };
        let Some(row) = self.policies_by_patchset.get(patchset_id) else {
            return json!({"patchset_id": patchset_id, "decision": "pending", "checks": []});
        };
        let checks = parse_json_field(row, "checks_json")
            .or_else(|| row.get("checks").cloned())
            .unwrap_or_else(|| json!([]));
        json!({
            "policy_decision_id": row.get("policy_decision_id").cloned().unwrap_or(JsonValue::Null),
            "repo_id": row.get("repo_id").cloned().unwrap_or(JsonValue::Null),
            "patchset_id": patchset_id,
            "decision": object_text(row, "decision").unwrap_or_else(|| "pending".to_string()),
            "checks": checks,
            "input_fingerprint": row.get("input_fingerprint").cloned().unwrap_or(JsonValue::Null),
            "evaluated_at": row.get("evaluated_at").or_else(|| row.get("created_at")).cloned().unwrap_or(JsonValue::Null),
        })
    }

    pub(super) fn attestation_summary(&self, patchset_id: Option<&str>) -> Option<JsonValue> {
        let patchset_id = patchset_id?;
        let row = self.attestations_by_patchset.get(patchset_id)?;
        Some(json!({
            "attestation_id": row.get("attestation_id").cloned().unwrap_or(JsonValue::Null),
            "repo_id": row.get("repo_id").cloned().unwrap_or(JsonValue::Null),
            "patchset_id": patchset_id,
            "author_mode": row.get("author_mode").cloned().unwrap_or(JsonValue::Null),
            "evaluation_summary": parse_json_field(row, "evaluation_summary_json")
                .or_else(|| row.get("evaluation_summary").cloned())
                .unwrap_or_else(|| json!({})),
            "provenance_summary": parse_json_field(row, "provenance_summary_json")
                .or_else(|| row.get("provenance_summary").cloned())
                .unwrap_or_else(|| json!({})),
            "detail": parse_json_field(row, "detail_json")
                .or_else(|| row.get("detail").cloned())
                .unwrap_or_else(|| json!({})),
            "created_at": row.get("created_at").cloned().unwrap_or(JsonValue::Null),
            "updated_at": row.get("updated_at").cloned().unwrap_or(JsonValue::Null),
        }))
    }

    pub(super) fn latest_land_summary(&self, change_id: &str) -> Option<JsonValue> {
        let land = self.lands_by_change.get(change_id)?.first()?;
        let result = parse_json_field(land, "result_json")
            .or_else(|| land.get("result").cloned())
            .unwrap_or_else(|| json!({}));
        Some(json!({
            "submission_id": land.get("submission_id").cloned().unwrap_or(JsonValue::Null),
            "change_id": land.get("change_id").cloned().unwrap_or(JsonValue::Null),
            "patchset_id": land.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
            "target_line": land.get("target_line").cloned().unwrap_or(JsonValue::Null),
            "status": land.get("status").cloned().unwrap_or(JsonValue::Null),
            "blocker_class": result.get("code")
                .or_else(|| result.get("blocker_class"))
                .cloned()
                .unwrap_or(JsonValue::Null),
            "suggested_action": result.get("message")
                .or_else(|| result.get("suggested_action"))
                .cloned()
                .unwrap_or(JsonValue::Null),
            "updated_at": land.get("updated_at").cloned().unwrap_or(JsonValue::Null),
            "result": result,
        }))
    }

    pub(super) fn line_head(&self, repo_name: &str, line_name: &str) -> Option<String> {
        self.refs_by_repo_line
            .get(&(repo_name.to_string(), line_name.to_string()))
            .cloned()
    }

    pub(super) fn delta_for_patchset(&self, patchset_id: &str) -> Option<JsonValue> {
        self.deltas_by_patchset
            .get(patchset_id)
            .map(|delta| JsonValue::Object((*delta).clone()))
    }

    pub(super) fn task_timeline(
        &self,
        task_id: &str,
        change_ids: &BTreeSet<String>,
        patchset_ids: &BTreeSet<String>,
    ) -> Vec<JsonValue> {
        self.events
            .iter()
            .filter(|event| {
                let entity_id = object_text(event, "entity_id").unwrap_or_default();
                entity_id == task_id
                    || change_ids.contains(&entity_id)
                    || patchset_ids.contains(&entity_id)
            })
            .map(|event| {
                let mut row = (*event).clone();
                if let Some(payload) = parse_json_field(&row, "payload_json") {
                    row.insert("payload".to_string(), payload);
                }
                row.remove("payload_json");
                JsonValue::Object(row)
            })
            .collect()
    }
}

pub(super) fn change_is_landable(row: &JsonValue) -> bool {
    if matches!(
        value_text_path(row, &["change", "status"]).as_deref(),
        Some(CHANGE_STATUS_LANDED | CHANGE_STATUS_ARCHIVED)
    ) {
        return true;
    }
    if row.get("current_patchset").is_none()
        || row.get("current_patchset") == Some(&JsonValue::Null)
    {
        return false;
    }
    if row.get("attestation_summary").is_none()
        || row.get("attestation_summary") == Some(&JsonValue::Null)
    {
        return false;
    }
    if change_has_failed_validation(row) {
        return false;
    }
    if value_text_path(row, &["policy_summary", "decision"])
        .unwrap_or_else(|| "pending".to_string())
        .to_ascii_lowercase()
        != "pass"
    {
        return false;
    }
    if value_int_path(row, &["review_summary", "approvals"]) < 1 {
        return false;
    }
    value_bool_path(row, &["freshness", "base_is_fresh"])
}

pub(super) fn change_has_failed_validation(row: &JsonValue) -> bool {
    if value_int_path(row, &["review_summary", "blocking"]) > 0 {
        return true;
    }
    if matches!(
        value_text_path(row, &["policy_summary", "decision"])
            .unwrap_or_else(|| "pending".to_string())
            .to_ascii_lowercase()
            .as_str(),
        "hard_fail" | "soft_fail" | "fail" | "failed"
    ) {
        return true;
    }
    for key in [
        "tests",
        "lint",
        "security",
        "security_scan",
        "license",
        "license_scan",
    ] {
        if matches!(
            attestation_status(row.get("attestation_summary"), key).as_str(),
            "fail" | "failed" | "hard_fail" | "soft_fail"
        ) {
            return true;
        }
    }
    false
}

pub(super) fn task_policy_missing_checks(policy: &JsonValue) -> Vec<String> {
    policy
        .get("checks")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|check| {
            let status = value_text(check, "status")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(status.as_str(), "pending" | "hard_fail" | "soft_fail") {
                Some(
                    value_text(check, "label")
                        .or_else(|| value_text(check, "name"))
                        .unwrap_or_else(|| "unknown".to_string()),
                )
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn attestation_status(attestation: Option<&JsonValue>, key: &str) -> String {
    attestation
        .and_then(|value| value.get("evaluation_summary"))
        .and_then(|summary| summary.get(key))
        .and_then(json_value_to_text)
        .unwrap_or_else(|| "pending".to_string())
        .to_ascii_lowercase()
}

pub(super) fn repository_ci_summary(ci_runs: &[JsonMap<String, JsonValue>]) -> JsonValue {
    let active_runs = ci_runs
        .iter()
        .filter(|run| {
            matches!(
                object_text(run, "state").as_deref(),
                Some("queued" | "running")
            )
        })
        .count();
    let failed_runs = ci_runs
        .iter()
        .filter(|run| object_text(run, "status").as_deref() == Some("fail"))
        .count();
    json!({
        "active_runs": active_runs,
        "failed_runs": failed_runs,
    })
}

pub(super) fn workflow_context(
    target: &str,
    focus_type: &str,
    focus_id: &str,
    focus_title: &str,
) -> JsonValue {
    json!({
        "target": target,
        "focus": {
            "type": focus_type,
            "id": focus_id,
            "title": focus_title,
        },
        "summary": {
            "document_count": 0,
            "diagram_count": 0,
            "layers": [],
        },
        "entries": [],
    })
}

pub(super) fn update_latest_activity(
    latest: &mut Option<JsonValue>,
    row: &JsonValue,
    kind: &str,
    id_field: &str,
) {
    let Some(updated_at) = value_text(row, "updated_at") else {
        return;
    };
    let replace = latest
        .as_ref()
        .and_then(|current| value_text(current, "updated_at"))
        .map(|current| updated_at > current)
        .unwrap_or(true);
    if replace {
        *latest = Some(json!({
            "kind": kind,
            "id": value_text(row, id_field),
            "updated_at": updated_at,
        }));
    }
}

pub(super) fn required_object(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    obj.get(field)
        .and_then(JsonValue::as_object)
        .cloned()
        .ok_or_else(|| format!("`{field}` must be a JSON object."))
}

pub(super) fn optional_object(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> JsonMap<String, JsonValue> {
    obj.get(field)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn required_text(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<String, String> {
    object_text(obj, field).ok_or_else(|| format!("`{field}` is required."))
}

pub(super) fn object_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    object_text_field(obj, field)
}

pub(super) fn value_text(value: &JsonValue, field: &str) -> Option<String> {
    value.as_object().and_then(|obj| object_text(obj, field))
}

pub(super) fn value_text_path(value: &JsonValue, path: &[&str]) -> Option<String> {
    let mut current = value;
    for field in path {
        current = current.get(*field)?;
    }
    json_value_to_text(current)
}

pub(super) fn value_bool_path(value: &JsonValue, path: &[&str]) -> bool {
    let mut current = value;
    for field in path {
        let Some(next) = current.get(*field) else {
            return false;
        };
        current = next;
    }
    bool_value(current).unwrap_or(false)
}

pub(super) fn value_int(value: &JsonValue, field: &str) -> i64 {
    value.get(field).and_then(int_value).unwrap_or(0)
}

pub(super) fn value_int_path(value: &JsonValue, path: &[&str]) -> i64 {
    let mut current = value;
    for field in path {
        let Some(next) = current.get(*field) else {
            return 0;
        };
        current = next;
    }
    int_value(current).unwrap_or(0)
}

fn int_field(obj: &JsonMap<String, JsonValue>, field: &str) -> i64 {
    obj.get(field).and_then(int_value).unwrap_or(0)
}

pub(super) fn int_value(value: &JsonValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

pub(super) fn bool_value(value: &JsonValue) -> Option<bool> {
    value.as_bool().or_else(
        || match value.as_str()?.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
    )
}

fn parse_json_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<JsonValue> {
    let value = obj.get(field)?;
    match value {
        JsonValue::String(text) => serde_json::from_str(text).ok(),
        JsonValue::Null => None,
        other => Some(other.clone()),
    }
}

fn patchset_number(row: &JsonMap<String, JsonValue>) -> i64 {
    row.get("patchset_number").and_then(int_value).unwrap_or(0)
}

pub(super) fn json_object(row: &JsonMap<String, JsonValue>) -> JsonValue {
    JsonValue::Object(row.clone())
}

pub(super) fn string_list(value: Option<&JsonValue>) -> Vec<String> {
    match value {
        Some(JsonValue::Array(items)) => items.iter().filter_map(json_value_to_text).collect(),
        Some(JsonValue::String(text)) if text.trim().is_empty() => Vec::new(),
        Some(JsonValue::String(text)) => text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        Some(other) => json_value_to_text(other).into_iter().collect(),
        None => Vec::new(),
    }
}
