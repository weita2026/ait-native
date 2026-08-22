use super::helpers::*;
use super::*;

#[derive(Default)]
pub(super) struct QueueIndex<'a> {
    unique_change_key_by_short_id: HashMap<String, String>,
    pub(super) patchsets_by_change_key: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    pub(super) reviews_by_change_key: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    pub(super) review_requests_by_change_key: HashMap<String, Vec<&'a JsonMap<String, JsonValue>>>,
    pub(super) attestations_by_patchset: HashMap<String, &'a JsonMap<String, JsonValue>>,
    pub(super) policies_by_patchset: HashMap<String, &'a JsonMap<String, JsonValue>>,
    pub(super) refs_by_repo_line: HashMap<(String, String), String>,
    pub(super) ci_by_patchset: HashMap<String, &'a JsonMap<String, JsonValue>>,
}

impl<'a> QueueIndex<'a> {
    pub(super) fn new(input: &'a QueueReadModelInput) -> Self {
        let mut index = Self::default();
        let mut ambiguous_short_ids = HashSet::new();
        for change in input.changes.iter() {
            let Some(short_id) = object_text(change, "change_id") else {
                continue;
            };
            if ambiguous_short_ids.contains(&short_id) {
                continue;
            }
            let change_key = object_text(change, "change_ref").unwrap_or_else(|| short_id.clone());
            if index
                .unique_change_key_by_short_id
                .insert(short_id.clone(), change_key)
                .is_some()
            {
                index.unique_change_key_by_short_id.remove(&short_id);
                ambiguous_short_ids.insert(short_id);
            }
        }
        for patchset in input.patchsets.iter() {
            if let Some(change_key) = index.change_key(patchset) {
                index
                    .patchsets_by_change_key
                    .entry(change_key)
                    .or_default()
                    .push(patchset);
            }
        }
        for patchsets in index.patchsets_by_change_key.values_mut() {
            patchsets.sort_by_key(|left| patchset_number(left));
        }
        for review in input.reviews.iter() {
            if let Some(change_key) = index.change_key(review) {
                index
                    .reviews_by_change_key
                    .entry(change_key)
                    .or_default()
                    .push(review);
            }
        }
        for request in input.review_requests.iter() {
            if let Some(change_key) = index.change_key(request) {
                index
                    .review_requests_by_change_key
                    .entry(change_key)
                    .or_default()
                    .push(request);
            }
        }
        for attestation in input.attestations.iter() {
            if let Some(patchset_id) = object_text(attestation, "patchset_id") {
                index
                    .attestations_by_patchset
                    .insert(patchset_id, attestation);
            }
        }
        for policy in input.policy_decisions.iter() {
            if let Some(patchset_id) = object_text(policy, "patchset_id") {
                let replace = index
                    .policies_by_patchset
                    .get(&patchset_id)
                    .map(|existing| {
                        object_text(policy, "created_at") >= object_text(existing, "created_at")
                    })
                    .unwrap_or(true);
                if replace {
                    index.policies_by_patchset.insert(patchset_id, policy);
                }
            }
        }
        for row in input.refs.iter() {
            if let (Some(repo_name), Some(line_name), Some(head)) = (
                object_text(row, "repo_name"),
                object_text(row, "line_name"),
                object_text(row, "head_snapshot_id"),
            ) {
                index.refs_by_repo_line.insert((repo_name, line_name), head);
            }
        }
        for row in input.ci_statuses.iter() {
            if let Some(patchset_id) = object_text(row, "patchset_id") {
                index.ci_by_patchset.insert(patchset_id, row);
            }
        }
        index
    }

    pub(super) fn change_key(&self, row: &JsonMap<String, JsonValue>) -> Option<String> {
        object_text(row, "change_ref").or_else(|| {
            object_text(row, "change_id")
                .and_then(|short_id| self.unique_change_key_by_short_id.get(&short_id).cloned())
        })
    }

    pub(super) fn current_patchset(
        &self,
        change: &JsonMap<String, JsonValue>,
    ) -> Option<&'a JsonMap<String, JsonValue>> {
        let change_key = self.change_key(change)?;
        if let Some(current_id) = object_text(change, "current_patchset_id") {
            if let Some(patchset) =
                self.patchsets_by_change_key
                    .get(&change_key)?
                    .iter()
                    .find(|patchset| {
                        object_text(patchset, "patchset_id").as_deref() == Some(current_id.as_str())
                    })
            {
                return Some(*patchset);
            }
        }
        self.patchsets_by_change_key
            .get(&change_key)?
            .last()
            .copied()
    }
}

pub(super) fn patchset_number(row: &JsonMap<String, JsonValue>) -> i64 {
    row.get("patchset_number")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
}
