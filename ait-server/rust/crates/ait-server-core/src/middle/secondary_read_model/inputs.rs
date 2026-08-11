use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityMapInput {
    pub repo_name: String,
    pub local_repo_name: Option<String>,
    pub documents: Vec<JsonMap<String, JsonValue>>,
    pub authority_nodes: Vec<JsonMap<String, JsonValue>>,
    pub actors: Vec<JsonMap<String, JsonValue>>,
    pub roles: Vec<JsonMap<String, JsonValue>>,
    pub permissions: Vec<JsonMap<String, JsonValue>>,
}

impl AuthorityMapInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = authority_map_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        Ok(Self {
            repo_name: optional_text(obj, "repo_name").unwrap_or_else(|| "ait".to_string()),
            local_repo_name: optional_text(obj, "local_repo_name"),
            documents: rows.take("documents"),
            authority_nodes: rows.take("authority_nodes"),
            actors: rows.take("actors"),
            roles: rows.take("roles"),
            permissions: rows.take("permissions"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerInboxInput {
    pub repo_name: Option<String>,
    pub author_class: Option<String>,
    pub author_mode: Option<String>,
    pub tests: Option<String>,
    pub policy: Option<String>,
    pub freshness: Option<String>,
    pub review: Option<String>,
    pub changes: Vec<JsonMap<String, JsonValue>>,
    pub tasks: Vec<JsonMap<String, JsonValue>>,
    pub patchsets: Vec<JsonMap<String, JsonValue>>,
    pub reviews: Vec<JsonMap<String, JsonValue>>,
    pub review_requests: Vec<JsonMap<String, JsonValue>>,
    pub attestations: Vec<JsonMap<String, JsonValue>>,
    pub policy_decisions: Vec<JsonMap<String, JsonValue>>,
    pub refs: Vec<JsonMap<String, JsonValue>>,
    pub land_requests: Vec<JsonMap<String, JsonValue>>,
}

impl ReviewerInboxInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = reviewer_inbox_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        Ok(Self {
            repo_name: optional_text(obj, "repo_name"),
            author_class: normalize_filter(optional_text(obj, "author_class")),
            author_mode: normalize_filter(optional_text(obj, "author_mode")),
            tests: normalize_filter(optional_text(obj, "tests")),
            policy: normalize_filter(optional_text(obj, "policy")),
            freshness: normalize_filter(optional_text(obj, "freshness")),
            review: normalize_filter(optional_text(obj, "review")),
            changes: rows.take("changes"),
            tasks: rows.take("tasks"),
            patchsets: rows.take("patchsets"),
            reviews: rows.take("reviews"),
            review_requests: rows.take("review_requests"),
            attestations: rows.take("attestations"),
            policy_decisions: rows.take("policy_decisions"),
            refs: rows.take("refs"),
            land_requests: rows.take("land_requests"),
        })
    }
}
