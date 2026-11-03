//! Shared Agent Teams session infrastructure, registration and finish presentation.
pub mod bootstrap;
pub mod lookup;
pub(crate) mod session_host;
use ait_core::json_support::{json, JsonValue};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishScope {
    Local,
    Remote,
}
impl FinishScope {
    fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug)]
pub struct ContactQuery {
    pub repository_root: PathBuf,
    pub task_id: String,
    pub scope: FinishScope,
}

#[derive(Clone, Debug)]
pub struct CoordinatorContact {
    pub authority: String,
    pub team_index: Option<u32>,
    pub agent_index: u32,
    pub name: String,
    pub session_id: String,
}

#[derive(Clone, Copy, Debug)]
pub enum ContactUnavailable {
    SourceUnavailable,
    MissingBinding,
    AmbiguousBinding,
}
impl ContactUnavailable {
    fn code(self) -> &'static str {
        match self {
            Self::SourceUnavailable => "source_unavailable",
            Self::MissingBinding => "missing_binding",
            Self::AmbiguousBinding => "ambiguous_binding",
        }
    }
    fn detail(self) -> &'static str {
        match self {
            Self::SourceUnavailable => {
                "Local Coordinator data could not be read or validated; check the team binding before notifying."
            }
            Self::MissingBinding => "Coordinator identity or session binding is not configured.",
            Self::AmbiguousBinding => {
                "Coordinator binding is ambiguous; resolve the assignment before sending."
            }
        }
    }
}

/// Implementations must resolve the selected repository Team, not a display name or
/// a guessed default team. Each call reads the current binding.
pub trait CoordinatorSource {
    fn resolve(&self, query: &ContactQuery) -> Result<CoordinatorContact, ContactUnavailable>;
}

/// Explicit first-slice boundary: no Agent files or alternate store are read.
pub struct UnavailableCoordinatorSource;
impl CoordinatorSource for UnavailableCoordinatorSource {
    fn resolve(&self, _: &ContactQuery) -> Result<CoordinatorContact, ContactUnavailable> {
        Err(ContactUnavailable::SourceUnavailable)
    }
}

/// Only decorate authoritative completed/partial finish results. This never
/// changes finish status, exit semantics, recovery commands or sends a message.
pub fn attach_finish_handoff(
    teams_enabled: bool,
    root: &Path,
    scope: FinishScope,
    payload: &mut JsonValue,
    source: &impl CoordinatorSource,
) {
    if !teams_enabled || payload["apply_status"].as_str() != Some("done") {
        return;
    }
    let delivery = match payload["closeout_status"].as_str() {
        Some("complete" | "complete_unbound" | "already_complete") => "complete",
        Some("partial") => "recovery",
        _ => return,
    };
    let Some(task_id) = payload["task_id"]
        .as_str()
        .or_else(|| payload["task"]["task_id"].as_str())
        .filter(|id| !id.trim().is_empty())
    else {
        return;
    };
    let query = ContactQuery {
        repository_root: root.to_path_buf(),
        task_id: task_id.to_owned(),
        scope,
    };
    let mut handoff = json!({
        "scope": scope.label(), "delivery_state": delivery,
        "notification_sent": false, "task_id": task_id,
    });
    for key in ["landed_snapshot_id", "patchset_id", "plan_binding"] {
        if let Some(value) = payload.get(key).filter(|value| !value.is_null()) {
            handoff[key] = value.clone();
        }
    }
    for key in ["plan_item_ref", "artifact_path"] {
        if let Some(value) = payload["plan_checklist_closeout"]
            .get(key)
            .filter(|value| !value.is_null())
        {
            handoff[key] = value.clone();
        }
    }
    let contact = source.resolve(&query).and_then(|contact| {
        if contact.authority.trim().is_empty()
            || contact.name.trim().is_empty()
            || contact.session_id.trim().is_empty()
        {
            Err(ContactUnavailable::MissingBinding)
        } else {
            Ok(contact)
        }
    });
    match contact {
        Ok(contact) => {
            handoff["recipient_resolution"] = json!("resolved");
            handoff["coordinator"] = json!({
                "authority": contact.authority, "team_index": contact.team_index,
                "agent_index": contact.agent_index, "name": contact.name,
                "session_id": contact.session_id,
            });
        }
        Err(error) => {
            handoff["recipient_resolution"] = json!("blocked");
            handoff["reason"] = json!(error.code());
            handoff["detail"] = json!(error.detail());
        }
    }
    payload["team_handoff"] = handoff;
}

pub fn append_handoff_text(mut rendered: String, payload: &JsonValue) -> String {
    let Some(handoff) = payload.get("team_handoff") else {
        return rendered;
    };
    if handoff["delivery_state"].as_str() == Some("recovery") {
        rendered.push_str("\n\nTeam handoff: finish requires recovery; delivery is not complete.");
    } else {
        rendered.push_str("\n\nTeam handoff:");
    }
    if handoff["recipient_resolution"].as_str() == Some("resolved") {
        let contact = &handoff["coordinator"];
        rendered.push_str(&format!(
            "\nNotify Coordinator: {}\nCoordinator Session ID: {}",
            contact["name"].as_str().unwrap_or_default(),
            contact["session_id"].as_str().unwrap_or_default()
        ));
    } else {
        rendered.push_str(&format!(
            "\nHandoff blocked: {}",
            handoff["detail"]
                .as_str()
                .unwrap_or("Coordinator contact is unavailable.")
        ));
    }
    for (key, label) in [
        ("task_id", "Task"),
        ("landed_snapshot_id", "Snapshot"),
        ("patchset_id", "Patchset"),
        ("plan_item_ref", "Sprint ref"),
        ("artifact_path", "Sprint path"),
    ] {
        if let Some(value) = handoff[key].as_str() {
            rendered.push_str(&format!("\n{label}: {value}"));
        }
    }
    rendered.push_str("\nNotification has not been sent.");
    rendered
}

#[cfg(test)]
mod tests;
