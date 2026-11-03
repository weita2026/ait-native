use super::*;
use crate::json_support::JsonValue;
use crate::server_operational::RepositoryIndex;

/// Effective AIT config identity. Registration does not consult Git or the
/// process environment, and case remains part of the identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteActorIdentity {
    user_name: String,
    user_email: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RequestActorRegistration {
    repo_name: String,
    identity: Result<RemoteActorIdentity, String>,
    namespace: Option<String>,
}

impl RemoteActorIdentity {
    pub fn new(user_name: &str, user_email: &str) -> PlanHttpClientResult<Self> {
        fn component(value: &str, field: &str) -> PlanHttpClientResult<String> {
            let value = value.trim();
            if value.is_empty() || value.len() > usize::from(u8::MAX) {
                return Err(PlanHttpClientError::Invalid(format!(
                    "Actor registration requires effective AIT config {field} with 1..255 UTF-8 bytes."
                )));
            }
            Ok(value.to_string())
        }
        Ok(Self {
            user_name: component(user_name, "user_name")?,
            user_email: component(user_email, "user_email")?,
        })
    }

    pub fn user_name(&self) -> &str {
        &self.user_name
    }
    pub fn user_email(&self) -> &str {
        &self.user_email
    }
}

impl PlanHttpClientManager {
    /// Configure one CLI/API operation without network effects. Read-only
    /// callers retain compatibility even if no effective identity is available.
    pub fn set_actor_registration(
        &mut self,
        repo_name: &str,
        identity: Result<RemoteActorIdentity, String>,
        namespace: Option<&str>,
    ) {
        self.actor_context = None;
        self.actor_registration = Some(RequestActorRegistration {
            repo_name: repo_name.into(),
            identity,
            namespace: namespace.map(str::to_string),
        });
    }

    pub(super) fn ensure_request_actor(&mut self) -> PlanHttpClientResult<u32> {
        if let Ok(actor) = self.require_actor_key_index() {
            return Ok(actor);
        }
        let registration = self.actor_registration.clone().ok_or_else(|| PlanHttpClientError::Invalid(
            "Remote workflow mutation requires effective AIT config user_name/user_email Actor registration.".into()))?;
        let identity = registration
            .identity
            .map_err(PlanHttpClientError::Invalid)?;
        self.ensure_repository_with_actor(
            &registration.repo_name,
            "main",
            None,
            registration.namespace.as_deref(),
            &identity,
        )?;
        self.require_actor_key_index()
    }

    pub(super) fn attach_workflow_actor(
        &mut self,
        spec: &mut PlanHttpRequestSpec,
    ) -> PlanHttpClientResult<()> {
        if matches!(spec.method.as_str(), "GET" | "HEAD") {
            return Ok(());
        }
        let prefix = self
            .config
            .repository_index
            .map(|index| format!("/v1/native/repository-authorities/{index}/"));
        let Some(route) = prefix
            .as_deref()
            .and_then(|prefix| spec.path.strip_prefix(prefix))
        else {
            return Ok(());
        };
        let affected = route == "tasks"
            || route == "task-start"
            || route.starts_with("tasks/")
            || route == "changes"
            || (route.starts_with("changes/") && !route.ends_with(":selectPatchset"))
            || matches!(
                route,
                "remote-sync/zstd-bulk/commit" | "history-promotion:prepare" | "task-land"
            )
            || (route.starts_with("lands/") && route.ends_with(":retry"));
        if !affected {
            return Ok(());
        }
        let actor = self.ensure_request_actor()?;
        let body = spec
            .body
            .as_mut()
            .and_then(JsonValue::as_object_mut)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(
                    "Actor-aware mutation requires its existing JSON object body.".into(),
                )
            })?;
        body.insert("actor_key_index".into(), JsonValue::from(actor));
        Ok(())
    }

    /// Resolve attribution before an Actor-aware domain mutation. The caller
    /// owns this manager for one operation; this is not a durable Actor cache.
    pub fn ensure_repository_with_actor(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
        identity: &RemoteActorIdentity,
    ) -> PlanHttpClientResult<JsonValue> {
        // A failed re-resolution must never reuse the previous user's key.
        self.actor_context = None;
        let mut spec = if let Some(index) = self.config.repository_index {
            let current = self.get_repository_by_index(index)?;
            let repository = current.get("repository").unwrap_or(&current);
            if repository
                .get("repository_index")
                .and_then(JsonValue::as_u64)
                != Some(u64::from(index.get()))
                || repository.get("tombstoned").and_then(JsonValue::as_bool) == Some(true)
            {
                return Err(PlanHttpClientError::Invalid(
                    "Existing Actor registration requires the exact live Repository authority."
                        .into(),
                ));
            }
            let name = repository
                .get("repository_name")
                .and_then(JsonValue::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid(
                        "Repository authority requires repository_name.".into(),
                    )
                })?;
            let namespace = repository
                .get("namespace")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid("Repository authority requires namespace.".into())
                })?;
            if let Some(expected) = id_namespace_prefix {
                if namespace != expected {
                    return Err(PlanHttpClientError::Invalid(format!(
                        "Remote Repository authority namespace mismatch: configured={expected:?} remote={namespace:?}"
                    )));
                }
            }
            let flags = repository
                .get("policy_flags")
                .and_then(JsonValue::as_u64)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid(
                        "Repository authority requires policy_flags:u8.".into(),
                    )
                })?;
            let body = crate::json_support::json!({
                "repository_index": index.get(), "repository_name": name,
                "namespace": namespace, "policy_flags": flags,
            });
            build_plan_http_request_spec(
                &self.config,
                reqwest::Method::POST,
                "/v1/native/repository-authorities",
                Vec::new(),
                Some(body),
            )?
        } else {
            build_ensure_repository_request_spec(
                &self.config,
                repo_name,
                default_line,
                policy,
                id_namespace_prefix,
            )?
        };
        let body = spec
            .body
            .as_mut()
            .and_then(JsonValue::as_object_mut)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid("Actor registration body must be an object.".into())
            })?;
        body.insert(
            "user_name".into(),
            JsonValue::String(identity.user_name.clone()),
        );
        body.insert(
            "user_email".into(),
            JsonValue::String(identity.user_email.clone()),
        );
        let response = parse_object_payload(self.execute_json(spec)?)?;
        let repository = response.get("repository").unwrap_or(&response);
        let repository_index = repository
            .get("repository_index")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .map(RepositoryIndex::new)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(
                    "Actor registration response requires repository_index:u32.".into(),
                )
            })?;
        if self
            .config
            .repository_index
            .is_some_and(|expected| expected != repository_index)
        {
            return Err(PlanHttpClientError::Invalid(
                "Actor registration returned a different Repository authority.".into(),
            ));
        }
        let actor = response.get("actor_key_index").and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| value.checked_add(1).is_some())
            .ok_or_else(|| PlanHttpClientError::Invalid("Remote peer must return actor_key_index:u32 within plus-one bounds; Actor-aware mutation is unavailable.".into()))?;
        self.config.repository_index = Some(repository_index);
        self.actor_context = Some((repository_index, actor));
        Ok(response)
    }

    pub fn require_actor_key_index(&self) -> PlanHttpClientResult<u32> {
        self.actor_context.filter(|(repository, _)| Some(*repository) == self.config.repository_index)
            .map(|(_, actor)| actor)
            .ok_or_else(|| PlanHttpClientError::Invalid("Resolve effective AIT config user_name/user_email through Actor registration before Remote Plan mutation.".into()))
    }
}

pub(super) fn validate_actor_key_index(actor: u32) -> PlanHttpClientResult<()> {
    actor.checked_add(1).map(|_| ()).ok_or_else(|| {
        PlanHttpClientError::Invalid("actor_key_index exceeds stored plus-one bounds.".into())
    })
}

#[cfg(test)]
mod tests;
