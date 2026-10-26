//! Acting AI identity is per operation and never borrowed from human config or the CI worker.
use crate::runtime::RepoRuntime;
use ait_core::agent_binary_db::single_team;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActingReviewer {
    pub name: String,
}

impl ActingReviewer {
    pub const ACTOR_KIND: &'static str = "bot";

    pub fn resolve(repo: &RepoRuntime, requested: Option<&str>) -> Result<Self, String> {
        let caller = crate::agent_teams::caller_session_identity()?;
        Self::resolve_for_caller(repo, requested, caller.as_deref())
    }

    fn resolve_for_caller(
        repo: &RepoRuntime,
        requested: Option<&str>,
        caller: Option<&str>,
    ) -> Result<Self, String> {
        let requested = requested.map(valid_name).transpose()?;
        // Disabled team management does not erase an existing native binding.
        // Reading it creates neither contacts nor sessions, and unbound external
        // orchestrators remain free to supply their own Reviewer explicitly.
        let agents = single_team::read(&repo.binary_db_stores::<1>().local_db())
            .map_err(|error| error.to_string())?;
        let bound = caller.and_then(|id| agents.iter().find(|agent| agent.session == id));
        let name = if let Some(agent) = bound {
            if agent.archived {
                return Err(
                    "The calling Agent is archived; native review identity is unavailable".into(),
                );
            }
            if agent.role != Some(2) {
                return Err("The calling Agent is not assigned the Reviewer role; it cannot identify itself as another Reviewer".into());
            }
            let name = valid_name(&agent.name)?;
            if name != agent.name {
                return Err("The bound Reviewer name requires normalization; reconcile the binding before review".into());
            }
            if requested
                .as_ref()
                .is_some_and(|requested| requested != &name)
            {
                return Err(
                    "Supplied Reviewer identity conflicts with the calling Agent binding".into(),
                );
            }
            name
        } else {
            requested.ok_or_else(|| {
                if repo.team_review_enabled() {
                    "Team review requires an independent Reviewer assignment; assign a Reviewer and run review from that agent's existing session".to_string()
                } else {
                    "Native AI review requires the acting Reviewer's own name; pass --reviewer or use its existing Reviewer Agent session".to_string()
                }
            })?
        };
        Ok(Self { name })
    }
}

fn valid_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() || name.len() > u8::MAX as usize || name.chars().any(char::is_control) {
        return Err(
            "Reviewer name must contain 1 to 255 UTF-8 bytes without control characters".into(),
        );
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    use ait_core::json_support::json;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, RepoRuntime) {
        let temp = tempfile::TempDir::new().unwrap();
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: None,
            default_line: "main".into(),
            policy_profile: "prototype".into(),
            default_author_mode: "ai_with_human_review".into(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        (temp, repo)
    }

    #[test]
    fn external_reviewer_needs_no_team_manager_or_human_identity() {
        let (_temp, mut repo) = fixture();
        for enabled in [false, true] {
            repo.config.insert("agent_teams".into(), json!(enabled));
            repo.config
                .insert("user_name".into(), json!("Human Approver"));
            repo.config
                .insert("user_email".into(), json!("human@example.com"));
            let reviewer =
                ActingReviewer::resolve_for_caller(&repo, Some("Einstein"), None).unwrap();
            assert_eq!(reviewer.name, "Einstein");
            assert_eq!(ActingReviewer::ACTOR_KIND, "bot");
            let missing = ActingReviewer::resolve_for_caller(&repo, None, None).unwrap_err();
            if enabled {
                assert!(missing.contains("independent Reviewer assignment"));
                assert!(missing.contains("existing session"));
            } else {
                assert!(missing.contains("--reviewer"));
            }
            assert!(single_team::read(&repo.binary_db_stores::<1>().local_db())
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn interleaved_bound_reviewers_preserve_identity_and_never_write_human_config() {
        let (_temp, mut repo) = fixture();
        let path = repo.authoritative_repo_root().join(".ait/config.json");
        let before = fs::read(&path).unwrap();
        let db = repo.binary_db_stores::<1>().local_db();
        single_team::ensure_pool(&db).unwrap();
        for (index, session) in [(1, "einstein-session"), (5, "faraday-session")] {
            let agents = single_team::read(&db).unwrap();
            single_team::assign(&db, index, &agents[index as usize], 2, session).unwrap();
        }
        for enabled in [false, true] {
            repo.config.insert("agent_teams".into(), json!(enabled));
            let config = repo.config.clone();
            for (session, expected) in [
                ("einstein-session", "Einstein"),
                ("faraday-session", "Faraday"),
                ("einstein-session", "Einstein"),
            ] {
                assert_eq!(
                    ActingReviewer::resolve_for_caller(&repo, None, Some(session))
                        .unwrap()
                        .name,
                    expected
                );
                assert_eq!(
                    ActingReviewer::resolve_for_caller(&repo, Some(expected), Some(session))
                        .unwrap()
                        .name,
                    expected
                );
            }
            assert_eq!(repo.config, config);
        }
        assert_eq!(fs::read(path).unwrap(), before);
        assert!(ActingReviewer::resolve_for_caller(
            &repo,
            Some("Faraday"),
            Some("einstein-session")
        )
        .unwrap_err()
        .contains("conflicts"));
    }

    #[test]
    fn bound_developer_cannot_claim_another_reviewer_and_invalid_names_fail() {
        let (_temp, repo) = fixture();
        let db = repo.binary_db_stores::<1>().local_db();
        single_team::ensure_pool(&db).unwrap();
        let agents = single_team::read(&db).unwrap();
        single_team::assign(&db, 4, &agents[4], 1, "developer-session").unwrap();
        assert!(ActingReviewer::resolve_for_caller(
            &repo,
            Some("Einstein"),
            Some("developer-session")
        )
        .unwrap_err()
        .contains("Reviewer role"));
        for name in [
            "".to_string(),
            "  ".to_string(),
            "A\nB".to_string(),
            "x".repeat(256),
        ] {
            assert!(ActingReviewer::resolve_for_caller(&repo, Some(&name), None).is_err());
        }
    }
}
