use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub const SERVER_AUTH_POLICY_CONTRACT_VERSION: &str = "ait.server.auth_policy.v1";
pub const SERVER_AUTH_REFERENCE_MODULE: &str = "../ait/src/ait_web/server_auth_runtime.py";

pub const REPO_READER: &str = "repo_reader";
pub const REPO_CONTRIBUTOR: &str = "repo_contributor";
pub const REPO_REVIEWER: &str = "repo_reviewer";
pub const REPO_OWNER: &str = "repo_owner";
pub const RELEASE_MANAGER: &str = "release_manager";
pub const POLICY_ADMIN: &str = "policy_admin";
pub const SECURITY_REVIEWER: &str = "security_reviewer";
pub const OPERATOR: &str = "operator";

const ALL_ROLES: &[&str] = &[
    REPO_READER,
    REPO_CONTRIBUTOR,
    REPO_REVIEWER,
    REPO_OWNER,
    RELEASE_MANAGER,
    POLICY_ADMIN,
    SECURITY_REVIEWER,
    OPERATOR,
];

const ROLE_SET_READ: &[&str] = &[
    REPO_READER,
    REPO_CONTRIBUTOR,
    REPO_REVIEWER,
    REPO_OWNER,
    RELEASE_MANAGER,
    POLICY_ADMIN,
    SECURITY_REVIEWER,
    OPERATOR,
];
const ROLE_SET_CONTRIBUTE: &[&str] = &[
    REPO_CONTRIBUTOR,
    REPO_REVIEWER,
    REPO_OWNER,
    RELEASE_MANAGER,
    OPERATOR,
];
const ROLE_SET_REVIEW: &[&str] = &[
    REPO_REVIEWER,
    REPO_OWNER,
    RELEASE_MANAGER,
    SECURITY_REVIEWER,
    OPERATOR,
];
const ROLE_SET_APPROVE_CRITICAL: &[&str] =
    &[REPO_OWNER, RELEASE_MANAGER, SECURITY_REVIEWER, OPERATOR];
const ROLE_SET_WAIVE: &[&str] = &[POLICY_ADMIN, SECURITY_REVIEWER, REPO_OWNER, OPERATOR];
const ROLE_SET_LAND: &[&str] = &[RELEASE_MANAGER, REPO_OWNER, OPERATOR];
const ROLE_SET_ADMIN: &[&str] = &[REPO_OWNER, OPERATOR];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorContext {
    pub identity: String,
    pub actor_type: String,
    pub claimed_roles: BTreeSet<String>,
    pub claimed_repos: BTreeSet<String>,
    pub mode: String,
}

impl ActorContext {
    pub fn to_json(&self) -> JsonValue {
        json!({
            "identity": self.identity,
            "actor_type": self.actor_type,
            "claimed_roles": sorted_values(&self.claimed_roles),
            "claimed_repos": sorted_values(&self.claimed_repos),
            "mode": self.mode,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthDecision {
    pub allowed: bool,
    pub status: u16,
    pub detail: String,
    pub required_action: String,
    pub effective_roles: BTreeSet<String>,
}

impl AuthDecision {
    pub fn allow(required_action: impl Into<String>, effective_roles: BTreeSet<String>) -> Self {
        Self {
            allowed: true,
            status: 200,
            detail: "allowed".to_string(),
            required_action: required_action.into(),
            effective_roles,
        }
    }

    pub fn deny(
        status: u16,
        detail: impl Into<String>,
        required_action: impl Into<String>,
        effective_roles: BTreeSet<String>,
    ) -> Self {
        Self {
            allowed: false,
            status,
            detail: detail.into(),
            required_action: required_action.into(),
            effective_roles,
        }
    }

    pub fn to_json(&self) -> JsonValue {
        json!({
            "allowed": self.allowed,
            "status": self.status,
            "detail": self.detail,
            "required_action": self.required_action,
            "effective_roles": sorted_values(&self.effective_roles),
        })
    }
}

pub fn server_auth_policy_contract() -> JsonValue {
    json!({
        "contract": SERVER_AUTH_POLICY_CONTRACT_VERSION,
        "reference_modules": [SERVER_AUTH_REFERENCE_MODULE],
        "auth_modes": ["open", "strict"],
        "roles": all_roles(),
        "role_sets": role_sets_json(),
        "operations": [
            "actor-from-headers",
            "repo-action",
            "line-update",
            "review-action",
            "admin-action",
        ],
        "request_fields": {
            "auth_mode": "Optional auth mode; defaults to open and is lower-cased after trimming.",
            "headers": "HTTP header map carrying X-AIT-Actor, X-AIT-Actor-Type, X-AIT-Roles, and X-AIT-Repos.",
            "bound_roles": "Roles resolved from durable repository role bindings by the caller.",
            "repo_lifecycle_state": "Repository lifecycle state; write-like actions require active.",
        },
        "compatibility_notes": {
            "python_reference": "Web authorization caller glue lives in ait_web.server_auth_runtime; Rust owns the server auth policy contract.",
            "open_mode": "Open mode grants all roles and all repositories, preserving server_auth.py compatibility.",
            "strict_mode": "Strict mode requires X-AIT-Actor and only trusts claimed roles for claimed repositories, except operator claims.",
            "storage": "Role binding lookup remains outside this deterministic policy contract.",
            "community_auth": "Community account passwords and web sessions remain separate security follow-up scope.",
            "task_dag": "Task DAG is retired and is not a server auth policy surface.",
        },
    })
}

pub fn server_auth_policy_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "server auth policy payload must be a JSON object.".to_string())?;
    match operation {
        "contract" => Ok(server_auth_policy_contract()),
        "actor-from-headers" => Ok(actor_operation_json(payload)),
        "repo-action" => Ok(repo_action_operation_json(payload)?),
        "line-update" => Ok(line_update_operation_json(payload)?),
        "review-action" => Ok(review_action_operation_json(payload)?),
        "admin-action" => Ok(admin_action_operation_json(payload)?),
        other => Err(format!(
            "Unsupported server auth policy operation `{other}`."
        )),
    }
}

pub fn all_roles() -> Vec<String> {
    ALL_ROLES.iter().map(|role| (*role).to_string()).collect()
}

pub fn role_set(action: &str) -> Option<BTreeSet<String>> {
    let roles = match action {
        "read" => ROLE_SET_READ,
        "contribute" => ROLE_SET_CONTRIBUTE,
        "review" => ROLE_SET_REVIEW,
        "approve_assisted" => ROLE_SET_REVIEW,
        "approve_critical" => ROLE_SET_APPROVE_CRITICAL,
        "waive" => ROLE_SET_WAIVE,
        "land" => ROLE_SET_LAND,
        "admin" => ROLE_SET_ADMIN,
        _ => return None,
    };
    Some(roles.iter().map(|role| (*role).to_string()).collect())
}

pub fn normalize_auth_mode(value: Option<&str>) -> String {
    let mode = value.unwrap_or("open").trim().to_ascii_lowercase();
    if mode.is_empty() {
        "open".to_string()
    } else {
        mode
    }
}

pub fn actor_from_headers(
    auth_mode: Option<&str>,
    headers: &JsonMap<String, JsonValue>,
) -> Result<ActorContext, AuthDecision> {
    let mode = normalize_auth_mode(auth_mode);
    let mut identity = header_text(headers, "X-AIT-Actor").unwrap_or_default();
    let actor_type = header_text(headers, "X-AIT-Actor-Type")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "human".to_string());
    let claimed_roles = csv_header_set(headers, "X-AIT-Roles")
        .into_iter()
        .filter(|role| ALL_ROLES.contains(&role.as_str()))
        .collect::<BTreeSet<_>>();
    let claimed_repos = csv_header_set(headers, "X-AIT-Repos");

    if mode == "open" {
        if identity.is_empty() {
            identity = "anonymous".to_string();
        }
        return Ok(ActorContext {
            identity,
            actor_type,
            claimed_roles: ALL_ROLES.iter().map(|role| (*role).to_string()).collect(),
            claimed_repos: BTreeSet::from(["*".to_string()]),
            mode,
        });
    }

    if identity.is_empty() {
        return Err(AuthDecision::deny(
            401,
            "Missing X-AIT-Actor in strict auth mode",
            "actor",
            BTreeSet::new(),
        ));
    }
    Ok(ActorContext {
        identity,
        actor_type,
        claimed_roles,
        claimed_repos,
        mode,
    })
}

pub fn effective_roles(
    actor: &ActorContext,
    repo_name: &str,
    bound_roles: BTreeSet<String>,
) -> BTreeSet<String> {
    if actor.mode == "open" {
        return ALL_ROLES.iter().map(|role| (*role).to_string()).collect();
    }

    let mut roles = bound_roles
        .into_iter()
        .filter(|role| ALL_ROLES.contains(&role.as_str()))
        .collect::<BTreeSet<_>>();
    if actor.claimed_roles.contains(OPERATOR) {
        roles.insert(OPERATOR.to_string());
    }
    if actor.claimed_repos.contains("*") || actor.claimed_repos.contains(repo_name) {
        roles.extend(actor.claimed_roles.iter().cloned());
    }
    roles
}

pub fn evaluate_repo_action(
    actor: &ActorContext,
    repo_name: &str,
    action: &str,
    bound_roles: BTreeSet<String>,
    repo_lifecycle_state: Option<&str>,
    detail: Option<&str>,
) -> Result<AuthDecision, String> {
    let allowed_roles =
        role_set(action).ok_or_else(|| format!("Unsupported repo action `{action}`."))?;
    let roles = effective_roles(actor, repo_name, bound_roles);
    if roles.is_disjoint(&allowed_roles) {
        let message = detail.map(str::to_string).unwrap_or_else(|| {
            format!(
                "Actor {} lacks permission for {} on repository {}",
                actor.identity, action, repo_name
            )
        });
        return Ok(AuthDecision::deny(403, message, action, roles));
    }

    if action != "read" {
        let lifecycle_state = normalize_lifecycle_state(repo_lifecycle_state);
        if lifecycle_state != "active" {
            return Ok(AuthDecision::deny(
                409,
                format!(
                    "Repository {} is {} and does not accept {} actions",
                    repo_name, lifecycle_state, action
                ),
                action,
                roles,
            ));
        }
    }

    Ok(AuthDecision::allow(action, roles))
}

pub fn line_update_required_action(line_name: &str, default_line: &str) -> (&'static str, String) {
    if line_name == default_line {
        (
            "land",
            format!(
                "Updating default line {} requires release or owner authority",
                line_name
            ),
        )
    } else {
        ("contribute", String::new())
    }
}

pub fn review_required_action(action: &str) -> &'static str {
    if action == "approve" || action == "task_approve" {
        "approve_assisted"
    } else {
        "review"
    }
}

fn actor_operation_json(payload: &JsonMap<String, JsonValue>) -> JsonValue {
    let headers = optional_object(payload.get("headers"));
    match actor_from_headers(optional_text(payload.get("auth_mode")).as_deref(), &headers) {
        Ok(actor) => json!({
            "contract": SERVER_AUTH_POLICY_CONTRACT_VERSION,
            "actor": actor.to_json(),
        }),
        Err(decision) => json!({
            "contract": SERVER_AUTH_POLICY_CONTRACT_VERSION,
            "decision": decision.to_json(),
        }),
    }
}

fn repo_action_operation_json(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    evaluate_operation_json(
        payload,
        required_text(payload.get("repo_name"), "repo_name")?,
        required_text(payload.get("action"), "action")?,
        None,
    )
}

fn line_update_operation_json(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let line_name = required_text(payload.get("line_name"), "line_name")?;
    let default_line = required_text(payload.get("default_line"), "default_line")?;
    let (action, detail) = line_update_required_action(&line_name, &default_line);
    evaluate_operation_json(
        payload,
        required_text(payload.get("repo_name"), "repo_name")?,
        action.to_string(),
        (!detail.is_empty()).then_some(detail),
    )
}

fn review_action_operation_json(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let action = required_text(payload.get("review_action"), "review_action")?;
    evaluate_operation_json(
        payload,
        required_text(payload.get("repo_name"), "repo_name")?,
        review_required_action(&action).to_string(),
        None,
    )
}

fn admin_action_operation_json(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let repo_name = required_text(payload.get("repo_name"), "repo_name")?;
    let detail = format!("Managing role bindings for {repo_name} requires repo_owner or operator");
    evaluate_operation_json(payload, repo_name, "admin".to_string(), Some(detail))
}

fn evaluate_operation_json(
    payload: &JsonMap<String, JsonValue>,
    repo_name: String,
    action: String,
    detail: Option<String>,
) -> Result<JsonValue, String> {
    let headers = optional_object(payload.get("headers"));
    let actor =
        match actor_from_headers(optional_text(payload.get("auth_mode")).as_deref(), &headers) {
            Ok(actor) => actor,
            Err(decision) => {
                return Ok(json!({
                    "contract": SERVER_AUTH_POLICY_CONTRACT_VERSION,
                    "decision": decision.to_json(),
                }))
            }
        };
    let bound_roles = string_set_value(payload.get("bound_roles"));
    let decision = evaluate_repo_action(
        &actor,
        &repo_name,
        &action,
        bound_roles,
        optional_text(payload.get("repo_lifecycle_state")).as_deref(),
        detail.as_deref(),
    )?;
    Ok(json!({
        "contract": SERVER_AUTH_POLICY_CONTRACT_VERSION,
        "actor": actor.to_json(),
        "repo_name": repo_name,
        "decision": decision.to_json(),
    }))
}

fn role_sets_json() -> JsonValue {
    let mut sets = BTreeMap::new();
    for action in [
        "read",
        "contribute",
        "review",
        "approve_assisted",
        "approve_critical",
        "waive",
        "land",
        "admin",
    ] {
        sets.insert(
            action,
            sorted_values(&role_set(action).expect("known role set")),
        );
    }
    json!(sets)
}

fn normalize_lifecycle_state(value: Option<&str>) -> String {
    let state = value.unwrap_or("active").trim().to_ascii_lowercase();
    if state.is_empty() {
        "active".to_string()
    } else {
        state
    }
}

fn optional_object(value: Option<&JsonValue>) -> JsonMap<String, JsonValue> {
    value
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value).ok_or_else(|| format!("Field `{field}` must be non-empty."))
}

fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    let text = match value? {
        JsonValue::String(text) => text.clone(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => String::new(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Null => String::new(),
        JsonValue::Array(_) | JsonValue::Object(_) => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn header_text(headers: &JsonMap<String, JsonValue>, name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| optional_text(Some(value)))
}

fn csv_header_set(headers: &JsonMap<String, JsonValue>, name: &str) -> BTreeSet<String> {
    header_text(headers, name)
        .map(|value| csv_set(&value))
        .unwrap_or_default()
}

fn csv_set(value: &str) -> BTreeSet<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn string_set_value(value: Option<&JsonValue>) -> BTreeSet<String> {
    match value {
        Some(JsonValue::Array(values)) => values
            .iter()
            .filter_map(|value| optional_text(Some(value)))
            .collect(),
        Some(JsonValue::String(text)) => csv_set(text),
        _ => BTreeSet::new(),
    }
}

fn sorted_values(values: &BTreeSet<String>) -> Vec<String> {
    values.iter().cloned().collect()
}
