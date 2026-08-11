use super::*;

impl RepoRuntime {
    pub fn auth_headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        if let Some(actor) = self.actor_identity() {
            headers.insert("X-AIT-Actor".to_string(), actor);
        }
        if let Some(actor_type) =
            env_nonempty("AIT_NATIVE_ACTOR_TYPE").or_else(|| env_nonempty("AIT_ACTOR_TYPE"))
        {
            headers.insert("X-AIT-Actor-Type".to_string(), actor_type);
        }
        if let Some(roles) = env_nonempty("AIT_NATIVE_ROLES").or_else(|| env_nonempty("AIT_ROLES"))
        {
            headers.insert("X-AIT-Roles".to_string(), roles);
        }
        if let Some(repos) = env_nonempty("AIT_NATIVE_REPOS").or_else(|| env_nonempty("AIT_REPOS"))
        {
            headers.insert("X-AIT-Repos".to_string(), repos);
        }
        headers
    }

    pub fn default_remote_name(&self) -> Option<String> {
        self.config
            .get("default_remote")
            .and_then(as_nonempty_string)
    }

    pub fn remote_row(&self, requested: Option<&str>) -> Result<RemoteRow, String> {
        let Some(remote_name) = requested
            .map(str::to_string)
            .and_then(|value| nonempty(Some(value)))
            .or_else(|| self.default_remote_name())
        else {
            return Err(
                "No remote configured. Pass --remote or configure default_remote first."
                    .to_string(),
            );
        };
        self.remote_store()?
            .remote_by_name(&remote_name)?
            .map(|row| RemoteRow {
                name: row.name,
                url: row.url,
                repo_name: row.repo_name,
            })
            .ok_or_else(|| format!("Unknown remote: {remote_name}"))
    }
}
