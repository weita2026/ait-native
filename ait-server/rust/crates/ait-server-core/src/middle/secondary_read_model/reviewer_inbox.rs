use super::*;

const REVIEWABLE_CHANGE_STATES: &[&str] = &["review", "gated", "approved", "landable", "blocked"];

pub fn reviewer_inbox_read_model(input: &ReviewerInboxInput) -> Result<JsonValue, String> {
    let index = ReviewerInboxIndex::new(input);
    let mut items = Vec::new();
    for change in input
        .changes
        .iter()
        .filter(|change| repo_matches(input.repo_name.as_deref(), change))
        .filter(|change| {
            object_text(change, "status")
                .map(|status| REVIEWABLE_CHANGE_STATES.contains(&status.as_str()))
                .unwrap_or(false)
        })
    {
        let change_id = object_text(change, "change_id").unwrap_or_default();
        let current_patchset = index.current_patchset(change);
        let current_patchset_id =
            current_patchset.and_then(|patchset| object_text(patchset, "patchset_id"));
        let current_policy = index.policy_summary(current_patchset_id.as_deref());
        let current_attestation = index.attestation_summary(current_patchset_id.as_deref());
        let author_mode = current_attestation
            .as_ref()
            .and_then(|attestation| value_text(attestation, "author_mode"));
        let tests_state = effective_validation_state(
            &current_policy,
            current_attestation.as_ref(),
            "tests",
            "require_tests",
        );
        if !matches_author_class(author_mode.as_deref(), input.author_class.as_deref()) {
            continue;
        }
        if !matches_filter(author_mode.as_deref(), input.author_mode.as_deref()) {
            continue;
        }
        if !matches_filter(Some(tests_state.as_str()), input.tests.as_deref()) {
            continue;
        }
        let review_summary = index.review_summary(&change_id, current_patchset_id.as_deref());
        let requested_groups = index.requested_groups(&change_id, current_patchset_id.as_deref());
        let freshness = index.freshness(change, current_patchset);
        let freshness_state =
            value_text(&freshness, "state").unwrap_or_else(|| "stale".to_string());
        let policy_decision =
            value_text(&current_policy, "decision").unwrap_or_else(|| "pending".to_string());
        if !matches_filter(Some(policy_decision.as_str()), input.policy.as_deref()) {
            continue;
        }
        if !matches_filter(Some(freshness_state.as_str()), input.freshness.as_deref()) {
            continue;
        }
        if !matches_review_filter(&review_summary, &requested_groups, input.review.as_deref()) {
            continue;
        }
        let task = index.task_for_change(change);
        let patchsets = index.patchsets_for_change(&change_id);
        let selected_patchset = index.selected_patchset(change);
        let provenance = current_attestation
            .as_ref()
            .and_then(|attestation| attestation.get("provenance_summary"))
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        items.push(json!({
            "change_id": change_id,
            "title": object_text(change, "title"),
            "repo": object_text(change, "repo_name"),
            "base_line": object_text(change, "base_line"),
            "task": {
                "task_id": task.as_ref().and_then(|task| object_text(task, "task_id")),
                "title": task.as_ref().and_then(|task| object_text(task, "title")),
                "status": task.as_ref().and_then(|task| object_text(task, "status")),
                "intent": task.as_ref().and_then(|task| object_text(task, "intent")),
            },
            "change_status": object_text(change, "status"),
            "current_patchset": {
                "patchset_id": current_patchset_id,
                "patchset_number": current_patchset.map(patchset_number).unwrap_or(0),
            },
            "selected_patchset": selected_patchset.map(|patchset| json!({
                "patchset_id": object_text(patchset, "patchset_id"),
                "patchset_number": patchset_number(patchset),
            })),
            "patchsets": patchsets.into_iter().map(|patchset| json!({
                "patchset_id": object_text(patchset, "patchset_id"),
                "patchset_number": patchset_number(patchset),
                "summary": patchset.get("summary").cloned().unwrap_or(JsonValue::Null),
            })).collect::<Vec<_>>(),
            "review_state": {
                "approvals": value_int(&review_summary, "approvals"),
                "blocking": value_int(&review_summary, "blocking"),
                "comments": value_int(&review_summary, "comments"),
            },
            "policy_state": {
                "decision": policy_decision,
                "missing_requirements": missing_requirements(&current_policy),
            },
            "freshness": freshness,
            "attestation": {
                "completeness": if current_attestation.is_some() {"summary_present"} else {"missing"},
                "author_mode": author_mode,
                "model_name": provenance.get("model_name").cloned().unwrap_or(JsonValue::Null),
                "evidence_readiness": provenance.get("evidence_readiness").cloned().unwrap_or(JsonValue::Null),
                "tests": tests_state,
                "updated_at": current_attestation.as_ref().and_then(|attestation| value_text(attestation, "updated_at")),
            },
            "landing_summary": index.latest_land_summary(&change_id),
            "requested_groups": requested_groups,
            "updated_at": object_text(change, "updated_at"),
        }));
    }
    items.sort_by(|left, right| {
        value_text(right, "updated_at").cmp(&value_text(left, "updated_at"))
    });
    Ok(json!({
        "items": items,
        "count": items.len(),
        "filters": {
            "repo_name": input.repo_name,
            "author_class": input.author_class,
            "author_mode": input.author_mode,
            "tests": input.tests,
            "policy": input.policy,
            "freshness": input.freshness,
            "review": input.review,
        },
    }))
}

#[derive(Default)]
struct ReviewerInboxIndex<'a> {
    tasks_by_repo_task: HashMap<(String, String), &'a JsonMap<String, JsonValue>>,
    patchsets_by_change: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    reviews_by_change: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    review_requests_by_change: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    attestations_by_patchset: HashMap<String, &'a JsonMap<String, JsonValue>>,
    policies_by_patchset: HashMap<String, &'a JsonMap<String, JsonValue>>,
    refs_by_repo_line: HashMap<(String, String), String>,
    lands_by_change: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
}

impl<'a> ReviewerInboxIndex<'a> {
    fn new(input: &'a ReviewerInboxInput) -> Self {
        let mut index = Self::default();
        for task in &input.tasks {
            if let (Some(repo_name), Some(task_id)) =
                (object_text(task, "repo_name"), object_text(task, "task_id"))
            {
                index.tasks_by_repo_task.insert((repo_name, task_id), task);
            }
        }
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
            patchsets.sort_by_key(|left| patchset_number(left));
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
        for request in &input.review_requests {
            if let Some(change_id) = object_text(request, "change_id") {
                index
                    .review_requests_by_change
                    .entry(change_id)
                    .or_default()
                    .push(request);
            }
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
                    })
                    .unwrap_or(true);
                if replace {
                    index.policies_by_patchset.insert(patchset_id, policy);
                }
            }
        }
        for row in &input.refs {
            if let (Some(repo_name), Some(line_name), Some(head)) = (
                object_text(row, "repo_name"),
                object_text(row, "line_name"),
                object_text(row, "head_snapshot_id"),
            ) {
                index.refs_by_repo_line.insert((repo_name, line_name), head);
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
        index
    }

    fn task_for_change(
        &self,
        change: &JsonMap<String, JsonValue>,
    ) -> Option<&'a JsonMap<String, JsonValue>> {
        self.tasks_by_repo_task
            .get(&(
                object_text(change, "repo_name")?,
                object_text(change, "task_id")?,
            ))
            .copied()
    }

    fn current_patchset(
        &self,
        change: &JsonMap<String, JsonValue>,
    ) -> Option<&'a JsonMap<String, JsonValue>> {
        self.patchset_for_change_field(change, "current_patchset_id", "current_patchset_number")
            .or_else(|| {
                let change_id = object_text(change, "change_id")?;
                self.patchsets_by_change.get(&change_id)?.last().copied()
            })
    }

    fn selected_patchset(
        &self,
        change: &JsonMap<String, JsonValue>,
    ) -> Option<&'a JsonMap<String, JsonValue>> {
        self.patchset_for_change_field(change, "selected_patchset_id", "selected_patchset_number")
    }

    fn patchsets_for_change(&self, change_id: &str) -> Vec<&'a JsonMap<String, JsonValue>> {
        self.patchsets_by_change
            .get(change_id)
            .cloned()
            .unwrap_or_default()
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

    fn review_summary(&self, change_id: &str, patchset_id: Option<&str>) -> JsonValue {
        let reviews = self
            .reviews_by_change
            .get(change_id)
            .into_iter()
            .flatten()
            .map(|review| (*review).clone())
            .collect::<Vec<_>>();
        let mut summary = review_summary_from_rows(&reviews, patchset_id);
        summary.insert("change_id".to_string(), json!(change_id));
        JsonValue::Object(summary)
    }

    fn requested_groups(&self, change_id: &str, patchset_id: Option<&str>) -> Vec<String> {
        let groups = self
            .review_requests_by_change
            .get(change_id)
            .into_iter()
            .flatten()
            .filter(|request| {
                patchset_id.is_none()
                    || object_text(request, "patchset_id").as_deref() == patchset_id
            })
            .filter_map(|request| object_text(request, "reviewer_group"))
            .collect::<BTreeSet<_>>();
        groups.into_iter().collect()
    }

    fn policy_summary(&self, patchset_id: Option<&str>) -> JsonValue {
        let Some(patchset_id) = patchset_id else {
            return json!({"decision": "pending", "checks": []});
        };
        let Some(row) = self.policies_by_patchset.get(patchset_id) else {
            return json!({"patchset_id": patchset_id, "decision": "pending", "checks": []});
        };
        json!({
            "policy_decision_id": row.get("policy_decision_id").cloned().unwrap_or(JsonValue::Null),
            "patchset_id": patchset_id,
            "decision": object_text(row, "decision").unwrap_or_else(|| "pending".to_string()),
            "checks": parse_json_field(row, "checks_json").or_else(|| row.get("checks").cloned()).unwrap_or_else(|| json!([])),
            "effective_requirements": parse_json_field(row, "effective_requirements_json")
                .or_else(|| row.get("effective_requirements").cloned())
                .unwrap_or_else(|| json!({})),
        })
    }

    fn attestation_summary(&self, patchset_id: Option<&str>) -> Option<JsonValue> {
        let row = self.attestations_by_patchset.get(patchset_id?)?;
        Some(json!({
            "attestation_id": row.get("attestation_id").cloned().unwrap_or(JsonValue::Null),
            "patchset_id": row.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
            "author_mode": row.get("author_mode").cloned().unwrap_or(JsonValue::Null),
            "evaluation_summary": parse_json_field(row, "evaluation_summary_json")
                .or_else(|| row.get("evaluation_summary").cloned())
                .unwrap_or_else(|| json!({})),
            "provenance_summary": parse_json_field(row, "provenance_summary_json")
                .or_else(|| row.get("provenance_summary").cloned())
                .unwrap_or_else(|| json!({})),
            "updated_at": row.get("updated_at").cloned().unwrap_or(JsonValue::Null),
        }))
    }

    fn freshness(
        &self,
        change: &JsonMap<String, JsonValue>,
        patchset: Option<&JsonMap<String, JsonValue>>,
    ) -> JsonValue {
        let Some(patchset) = patchset else {
            return json!({"base_is_fresh": false, "state": "stale", "current_base_head": null});
        };
        let repo_name = object_text(change, "repo_name").unwrap_or_default();
        let base_line = object_text(change, "base_line").unwrap_or_else(|| "main".to_string());
        let base_head = self.refs_by_repo_line.get(&(repo_name, base_line)).cloned();
        let base_snapshot = object_text(patchset, "base_snapshot_id");
        let is_fresh = base_head
            .as_ref()
            .zip(base_snapshot.as_ref())
            .map(|(head, base)| head == base)
            .unwrap_or(false);
        json!({
            "base_is_fresh": is_fresh,
            "state": if is_fresh {"fresh"} else {"stale"},
            "current_base_head": base_head,
        })
    }

    fn latest_land_summary(&self, change_id: &str) -> Option<JsonValue> {
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
            "blocker_class": result.get("code").or_else(|| result.get("blocker_class")).cloned().unwrap_or(JsonValue::Null),
            "suggested_action": result.get("message").or_else(|| result.get("suggested_action")).cloned().unwrap_or(JsonValue::Null),
            "updated_at": land.get("updated_at").cloned().unwrap_or(JsonValue::Null),
            "result": result,
        }))
    }
}
