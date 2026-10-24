//! Agent Teams uses local Agent bindings and numbered role Plans. No scheduler.
use crate::runtime::RepoRuntime;
use crate::team_remote::bootstrap::{fence, operation_lock};
use crate::team_remote::session_host::{CodexSessionHost, SessionHost};
use ait_core::agent_binary_db::{single_team, Agent};
use ait_core::json_support::{json, JsonValue};
use std::{fs, path::Path};
mod activation;
mod roles;
mod sessions;
#[cfg(test)]
mod tests;

pub fn enabled(repo: &RepoRuntime) -> bool {
    repo.config.get("agent_teams").and_then(JsonValue::as_bool) == Some(true)
}
pub fn registered(repo: &RepoRuntime) -> Result<bool, String> {
    crate::team_remote::lookup::has_registration(repo)
}
pub(crate) fn caller_session_identity() -> Result<Option<String>, String> {
    activation::caller_session()
}
fn synchronize_assets(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let before = single_team::read(&repo.binary_db_stores::<1>().local_db())
        .map_err(|e| e.to_string())?
        .len();
    single_team::ensure_pool(&repo.binary_db_stores::<1>().local_db())
        .map_err(|e| e.to_string())?;
    let roles = roles::initialize(repo)?;
    Ok(
        json!({"status":roles["status"],"contacts":{"status":"ready","entry_count":50,"added_count":50-before},"roles":roles}),
    )
}
pub fn bootstrap(repo: &RepoRuntime) -> JsonValue {
    let result = (|| {
        let _lease = crate::config_surface::lock_root_config(repo, true)?;
        let repo = RepoRuntime::discover_from_path(&repo.authoritative_repo_root())?;
        let (_op, _lock) = operation_lock(&repo.authoritative_repo_root())?;
        let synchronization = synchronize_assets(&repo)?;
        if synchronization["status"] != "ready" {
            let mut result=activation_failure(&repo,"Customized role defaults require reconciliation; local Markdown was synced and preserved".into());
            result["synchronization"] = synchronization;
            return Ok(result);
        }
        if !enabled(&repo) {
            return Ok(
                json!({"status":"disabled","synchronization":synchronization,"session_readiness":"disabled","work_started":false}),
            );
        }
        let result = activation::caller_session().and_then(|caller| {
            let mut host = CodexSessionHost::connect()?;
            activation::prepare(&repo, &mut host, caller.as_deref())
        });
        let mut result = result.unwrap_or_else(|error| activation_failure(&repo, error));
        result["session_readiness"] = result["status"].clone();
        result["synchronization"] = synchronization;
        Ok(result)
    })();
    result.unwrap_or_else(|e: String| activation_failure(repo, e))
}
fn activation_failure(repo: &RepoRuntime, detail: String) -> JsonValue {
    let owner = sessions::current(repo, 0)
        .ok()
        .map(|agent| agent.session)
        .filter(|id| !id.is_empty());
    let pending = activation::pending(repo).ok().flatten();
    let candidate = pending
        .as_ref()
        .and_then(|v| v["session_id"].as_str().map(str::to_owned));
    json!({"status":"pending","session_readiness":"pending","detail":detail,"work_started":false,
        "target_session_id":candidate.clone().or_else(||activation::caller_session().ok().flatten()),
        "previous_session_id":pending.as_ref().map(|v|v["previous_session_id"].clone()),
        "replacement_reason":pending.as_ref().map(|v|v["replacement_reason"].clone()),
        "session_id":owner.or(candidate).or_else(||activation::caller_session().ok().flatten())})
}
pub fn acknowledge(repo: &RepoRuntime, token: &str) -> JsonValue {
    let result = (|| {
        let _lease = crate::config_surface::lock_root_config(repo, true)?;
        let repo = RepoRuntime::discover_from_path(&repo.authoritative_repo_root())?;
        if !enabled(&repo) {
            return Err("Agent Teams is disabled".into());
        }
        let (_op, _lock) = operation_lock(&repo.authoritative_repo_root())?;
        let caller = activation::caller_session()?
            .ok_or("Native caller identity is required for acknowledgment")?;
        let synchronization = synchronize_assets(&repo)?;
        let result = if synchronization["status"] != "ready" {
            Err("Customized role defaults require reconciliation before acknowledgment".into())
        } else {
            CodexSessionHost::connect().and_then(|mut host| {
                activation::acknowledge_with_host(&repo, &mut host, &caller, token)
            })
        };
        let mut result = result.unwrap_or_else(|error| activation_failure(&repo, error));
        result["synchronization"] = synchronization;
        result["session_readiness"] = result["status"].clone();
        Ok(result)
    })();
    result.unwrap_or_else(|error| activation_failure(repo, error))
}
pub fn render_activation(value: &JsonValue) -> Option<String> {
    (value["status"] == "activation_required").then(|| {
        format!(
            "{}\nApp actions: {}\n{}",
            value["instructions"].as_str().unwrap_or(""),
            value["app_actions"],
            value["role_instructions"].as_str().unwrap_or("")
        )
    })
}
pub fn ensure(repo: &RepoRuntime, index: u32, role: Option<u32>) -> JsonValue {
    let result = (|| {
        let _lease = crate::config_surface::lock_root_config(repo, true)?;
        let repo = RepoRuntime::discover_from_path(&repo.authoritative_repo_root())?;
        if !enabled(&repo) {
            return Err("Agent Teams is disabled".into());
        }
        let (_op, _lock) = operation_lock(&repo.authoritative_repo_root())?;
        let mut host = CodexSessionHost::connect()?;
        sessions::ensure_with_host(&repo, &mut host, index, role)
    })();
    result.unwrap_or_else(
        |e: String| json!({"status":"pending","agent_index":index,"detail":e,"work_started":false}),
    )
}
pub fn directory(repo: &RepoRuntime) -> Result<JsonValue, String> {
    if !enabled(repo) {
        return Ok(json!({"status":"ok","team_enabled":false,"members":[],"mutated":false}));
    }
    let registered = registered(repo)?;
    let agents =
        single_team::read(&repo.binary_db_stores::<1>().local_db()).map_err(|e| e.to_string())?;
    let mut members = Vec::new();
    for (i, agent) in agents.iter().enumerate() {
        let baseline = single_team::BASE_SEATS
            .iter()
            .find(|(id, _)| *id == i as u32)
            .map(|(_, role)| *role);
        if baseline.is_none() && agent.session.is_empty() {
            continue;
        }
        if agent.archived {
            continue;
        }
        let role = if agent.session.is_empty() {
            baseline
        } else {
            agent.role
        };
        let definition = role.map(|role| roles::load(repo, role)).transpose()?;
        let receipt = sessions::loaded_receipt(&repo.authoritative_repo_root(), i as u32)?;
        let loaded = definition
            .as_ref()
            .is_some_and(|r| sessions::receipt_matches(&receipt, agent, i as u32, r));
        let loaded_revision = (receipt["phase"] == "complete"
            && receipt["session_id"] == agent.session
            && receipt["role_index"] == json!(role))
        .then(|| receipt["revision"].as_u64())
        .flatten();
        members.push(json!({"agent_index":i,"name":agent.name,"role_index":role,
            "role_title":definition.as_ref().map(|r|&r.title),"role_artifact_path":definition.as_ref().map(|r|&r.path),
            "role_revision_index":definition.as_ref().map(|r|r.revision),"session_id":agent.session,
            "loaded_role_revision_index":loaded_revision,
            "role_reload_status":if agent.session.is_empty(){"unbound"}else if loaded{"complete"}else{"pending"},
            "role_state":if role==Some(2)&&!registered{"pending_server_registration"}else{"active"},
            "availability":if agent.session.is_empty(){"unbound"}else{"unknown"},"host_status":"unknown"}));
    }
    // Reserve identities are not members until their bindings are committed.
    let bound = members
        .iter()
        .filter(|m| m["session_id"].as_str().is_some_and(|s| !s.is_empty()))
        .count();
    let actual = members.len();
    Ok(
        json!({"status":"ok","team_enabled":true,"authority":format!("local:{}",repo.authoritative_repo_root().display()),
        "owner_agent_index":0,"planned_seat_count":4,"actual_member_count":actual,"bound_session_count":bound,
        "registered":registered,"members":members,"mutated":false}),
    )
}
pub fn contacts(repo: &RepoRuntime) -> JsonValue {
    let result = (|| {
        let repo = RepoRuntime::discover_from_path(&repo.authoritative_repo_root())?;
        let mut result = directory(&repo)?;
        if !enabled(&repo) {
            return Ok(result);
        }
        let mut host = CodexSessionHost::connect_for_contacts().ok();
        if let Some(members) = result["members"].as_array_mut() {
            for member in members {
                if let Some(id) = member["session_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                {
                    let status = host
                        .as_mut()
                        .map(|h| {
                            h.contact_health(
                                &id,
                                &repo.authoritative_repo_root(),
                                fs::read_to_string(repo.authoritative_repo_root().join(format!(
                                    ".ait/team-session-bootstrap/agent-{}.host",
                                    member["agent_index"]
                                )))
                                .ok()
                                .as_deref(),
                            )
                        })
                        .unwrap_or_else(
                            || json!({"availability":"unknown","host_status":"unknown"}),
                        );
                    member["availability"] = status["availability"].clone();
                    member["host_status"] = status["host_status"].clone();
                }
            }
        }
        let fresh = directory(&repo)?;
        if fresh["members"] != result["members"] {
            // Host fields are observational. Verify binding identities independently.
            let ids = |v: &JsonValue| {
                v["members"].as_array().map(|a| {
                    a.iter()
                        .map(|m| {
                            (
                                m["agent_index"].clone(),
                                m["session_id"].clone(),
                                m["role_index"].clone(),
                                m["role_revision_index"].clone(),
                                m["role_artifact_path"].clone(),
                                m["loaded_role_revision_index"].clone(),
                                m["role_reload_status"].clone(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
            };
            if ids(&fresh) != ids(&result) {
                return Err("Directory changed during host inspection; retry contacts".into());
            }
        }
        Ok(result)
    })();
    result.unwrap_or_else(|e: String| json!({"status":"blocked","detail":e,"mutated":false}))
}
pub fn render_contacts(v: &JsonValue) -> String {
    if v["team_enabled"] == false {
        return "Agent Teams disabled.".into();
    }
    if v["status"] != "ok" {
        return format!("Agent Teams unavailable: {}", v["detail"]);
    }
    let mut text = format!(
        "Agent Teams: {} members, {} bound sessions\nAgent\tName\tRole\tSession\tHost / reload\tRole latest / loaded",
        v["actual_member_count"], v["bound_session_count"]
    );
    if let Some(rows) = v["members"].as_array() {
        for m in rows {
            text.push_str(&format!(
                "\n{}\t{}\t{} ({})\t{}\t{} / {}\t{} / {}",
                m["agent_index"],
                m["name"].as_str().unwrap_or(""),
                m["role_title"].as_str().unwrap_or("unassigned"),
                m["role_state"].as_str().unwrap_or(""),
                m["session_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unbound"),
                m["host_status"],
                m["role_reload_status"],
                m["role_revision_index"],
                m["loaded_role_revision_index"]
            ));
        }
    }
    text
}
pub struct CoordinatorSource;
impl crate::team_remote::CoordinatorSource for CoordinatorSource {
    fn resolve(
        &self,
        q: &crate::team_remote::ContactQuery,
    ) -> Result<crate::team_remote::CoordinatorContact, crate::team_remote::ContactUnavailable>
    {
        use crate::team_remote::ContactUnavailable as E;
        let repo = RepoRuntime::discover_from_path(&q.repository_root)
            .map_err(|_| E::SourceUnavailable)?;
        if !enabled(&repo) {
            return Err(E::MissingBinding);
        }
        let agents = single_team::read(&repo.binary_db_stores::<1>().local_db())
            .map_err(|_| E::SourceUnavailable)?;
        let a = agents
            .first()
            .filter(|a| !a.archived && a.role == Some(0) && !a.session.is_empty())
            .ok_or(E::MissingBinding)?;
        Ok(crate::team_remote::CoordinatorContact {
            authority: format!("local:{}", q.repository_root.display()),
            team_index: None,
            agent_index: 0,
            name: a.name.clone(),
            session_id: a.session.clone(),
        })
    }
}
pub fn before_plan_sync(repo: &RepoRuntime, target: &Path, prune: bool) -> Result<(), String> {
    if !enabled(repo) || !prune {
        return Ok(());
    }
    let root = repo.authoritative_repo_root();
    let path = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    };
    let role_dir = root.join("docs/roles");
    if !path.starts_with(&role_dir) && !role_dir.starts_with(&path) {
        return Ok(());
    }
    let known = roles::paths(&root)?;
    for a in single_team::read_for_role_prune(&root, &repo.repo_name())? {
        if !a.session.is_empty() && a.role.is_some_and(|r| !known.contains_key(&r)) {
            return Err(
                "Cannot prune a bound role MD; preserve its readable Plan and restore the file"
                    .into(),
            );
        }
    }
    Ok(())
}
