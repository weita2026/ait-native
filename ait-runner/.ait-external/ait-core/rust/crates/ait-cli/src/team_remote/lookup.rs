//! Registration-derived profile query. No connectivity probe or state mutation.
use crate::runtime::RepoRuntime;
use ait_core::agent_binary_db::{DirectoryLookup, LookupProfiles};
use ait_core::binary_db::BinaryDbReadTxn;
use ait_core::remote_store::RemoteStore;
use ait_core::server_operational::ServerRepositoryAuthorityConfig;

/// Read fresh existing authority. A URL alone is not successful registration;
/// malformed/incomplete evidence never silently selects Developer fallback.
pub fn lookup_profile(
    repo: &RepoRuntime,
    profiles: LookupProfiles,
) -> Result<DirectoryLookup, String> {
    let fresh = RepoRuntime::discover_from_path(&repo.authoritative_repo_root())?;
    let registered = has_registration(&fresh)?;
    let stores = fresh.binary_db_stores::<1>();
    let db = stores.local_db();
    let read = BinaryDbReadTxn::new(&db);
    profiles.read(&read, registered).map_err(|e| e.to_string())
}
pub(crate) fn has_registration(repo: &RepoRuntime) -> Result<bool, String> {
    let authority = ServerRepositoryAuthorityConfig::from_config_object(&repo.config)?;
    let remotes = repo.remote_store()?.list_remotes()?;
    match (authority,remotes.is_empty()) {
        (None,true)=>Ok(false),
        (Some(_),false) if remotes.iter().any(|r|r.repo_name.as_ref().is_some_and(|n|!n.trim().is_empty()))=>Ok(true),
        _=>Err("Repository server registration is incomplete; reconcile existing repository_index and registered remote evidence before selecting a Team role profile.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions};
    use std::fs;
    use tempfile::TempDir;
    fn fixture(config: ait_core::json_support::JsonValue) -> (TempDir, RepoRuntime) {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(".ait")).unwrap();
        fs::write(
            temp.path().join(".ait/config.json"),
            JsonCodec::encode_value(&config, JsonEncodeOptions::pretty()).unwrap(),
        )
        .unwrap();
        let repo = RepoRuntime {
            root: temp.path().to_path_buf(),
            ait_dir: temp.path().join(".ait"),
            config: config.as_object().unwrap().clone(),
            worktree_config_path: None,
        };
        (temp, repo)
    }
    #[test]
    fn registration_is_authority_evidence_and_never_network_reachability() {
        let (_t, repo) = fixture(json!({}));
        assert!(!has_registration(&repo).unwrap());
        let (_t, repo) = fixture(
            json!({"repository_index":0,"remotes":{"origin":{"url":"http://127.0.0.1:1","repo_name":"registered-repo"}}}),
        );
        assert!(has_registration(&repo).unwrap()); // index0 is valid; endpoint is offline.
        for config in [
            json!({"repository_index":"invalid"}),
            json!({"repository_index":0}),
            json!({"remotes":{"origin":{"url":"http://127.0.0.1:1","repo_name":"r"}}}),
            json!({"repository_index":0,"remotes":{"origin":{"url":"http://127.0.0.1:1"}}}),
        ] {
            let (_t, repo) = fixture(config);
            assert!(has_registration(&repo).is_err());
        }
    }
}
