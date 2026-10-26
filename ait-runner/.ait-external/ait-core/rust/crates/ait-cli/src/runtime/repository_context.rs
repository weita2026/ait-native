use super::*;

impl RepoRuntime {
    pub fn repository_index(&self) -> Option<RepositoryIndex> {
        ServerRepositoryAuthorityConfig::from_config_object(&self.config)
            .ok()
            .flatten()
            .map(|config| config.repository_index)
    }

    pub fn require_repository_index(&self) -> Result<RepositoryIndex, String> {
        ServerRepositoryAuthorityConfig::from_config_object(&self.config)?
            .map(|config| config.repository_index)
            .ok_or_else(|| {
                format!(
                    "{REPOSITORY_INDEX_CONFIG_KEY} is required for PostgreSQL-free remote repository authority"
                )
            })
    }

    pub fn repo_name(&self) -> String {
        self.config
            .get("repo_name")
            .and_then(as_nonempty_string)
            .unwrap_or_else(|| {
                self.root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("repo")
                    .to_string()
            })
    }

    pub fn workspace_root(&self) -> PathBuf {
        let configured = self
            .config
            .get("workspace_root")
            .and_then(as_nonempty_string);
        match configured {
            Some(raw) => {
                let path = PathBuf::from(raw);
                if path.is_absolute() {
                    path
                } else {
                    self.root.join(path)
                }
            }
            None => self.root.clone(),
        }
    }

    pub fn authoritative_repo_root(&self) -> PathBuf {
        let configured = self.config.get("repo_root").and_then(as_nonempty_string);
        match configured {
            Some(raw) => {
                let path = PathBuf::from(raw);
                if path.is_absolute() {
                    path
                } else {
                    self.root.join(path)
                }
            }
            None => self.root.clone(),
        }
    }

    pub fn id_namespace_prefix(&self) -> String {
        self.config
            .get("id_namespace_prefix")
            .and_then(as_string)
            .unwrap_or_default()
    }

    pub fn actor_identity(&self) -> Option<String> {
        env_nonempty(ait_core::environment_contract::names::AIT_NATIVE_ACTOR)
            .or_else(|| self.config.get("user_email").and_then(as_nonempty_string))
            .or_else(|| self.config.get("user_name").and_then(as_nonempty_string))
    }

    pub fn default_line_name(&self) -> String {
        self.config
            .get("default_line")
            .and_then(as_nonempty_string)
            .unwrap_or_else(|| "main".to_string())
    }

    pub fn current_line_name(&self) -> Result<String, String> {
        if let Some(current_line) = self.config.get("current_line").and_then(as_nonempty_string) {
            return Ok(current_line);
        }
        Ok(self.default_line_name())
    }

    pub fn is_worktree(&self) -> bool {
        self.worktree_config_path.is_some()
    }

    pub fn set_worktree_materialized_snapshot(
        &self,
        snapshot_id: Option<&str>,
    ) -> Result<(), String> {
        let Some(path) = self.worktree_config_path.as_ref() else {
            return Ok(());
        };
        let mut payload = read_json_object(path);
        payload.insert(
            "materialized_snapshot_id".to_string(),
            snapshot_id
                .map(|value| JsonValue::String(value.to_string()))
                .unwrap_or(JsonValue::Null),
        );
        payload.entry("repo_root".to_string()).or_insert_with(|| {
            JsonValue::String(self.authoritative_repo_root().to_string_lossy().to_string())
        });
        payload
            .entry("workspace_root".to_string())
            .or_insert_with(|| {
                JsonValue::String(self.workspace_root().to_string_lossy().to_string())
            });
        let encoded = encode_value_pretty_with_newline_error_string(&JsonValue::Object(payload))?;
        fs::write(path, encoded).map_err(|err| err.to_string())
    }

    pub fn effective_author_mode(&self, requested: Option<&str>) -> String {
        nonempty(requested.map(str::to_string))
            .or_else(|| {
                self.config
                    .get("default_author_mode")
                    .and_then(as_nonempty_string)
            })
            .unwrap_or_else(|| DEFAULT_AUTHOR_MODE.to_string())
    }

    pub fn effective_model_name(&self, requested: Option<&str>) -> Option<String> {
        nonempty(requested.map(str::to_string)).or_else(|| {
            self.config
                .get("default_model")
                .and_then(as_nonempty_string)
        })
    }

    pub fn reviewer_identity(&self, requested: Option<&str>) -> Option<String> {
        nonempty(requested.map(str::to_string))
            .or_else(|| self.formatted_user_identity())
            .or_else(|| env_nonempty(ait_core::environment_contract::names::AIT_NATIVE_ACTOR))
    }

    pub fn task_review_reviewer_identity(&self) -> Option<String> {
        self.config.get("user_name").and_then(as_nonempty_string)
    }

    pub fn ai_code_review_reviewer_identity(&self) -> Option<String> {
        command_executable_basename()
    }

    pub fn effective_workflow_mode(&self) -> String {
        let configured_mode = self
            .config
            .get("workflow_mode")
            .and_then(as_nonempty_string);
        let workflow_scope = self
            .config
            .get("workflow_default_scope")
            .and_then(as_nonempty_string)
            .unwrap_or_else(|| DEFAULT_WORKFLOW_SCOPE.to_string());
        let task_scope = self
            .config
            .get("task_default_scope")
            .and_then(as_nonempty_string)
            .unwrap_or_else(|| workflow_scope.clone());
        let change_scope = self
            .config
            .get("change_default_scope")
            .and_then(as_nonempty_string)
            .unwrap_or_else(|| workflow_scope.clone());
        let binding_mode = self
            .config
            .get("plan_task_binding")
            .and_then(plan_task_binding_mode)
            .unwrap_or_else(|| DEFAULT_PLAN_TASK_BINDING_MODE.to_string());
        if let Some(mode) = configured_mode.as_deref() {
            let preset = match mode {
                "solo_local" => Some(("local", "local", "local", "required")),
                "solo_remote" => Some(("remote", "remote", "remote", "required")),
                "team_remote" => Some(("remote", "remote", "remote", "required")),
                _ => None,
            };
            if let Some((preset_workflow, preset_task, preset_change, preset_binding)) = preset {
                if workflow_scope == preset_workflow
                    && task_scope == preset_task
                    && change_scope == preset_change
                    && (binding_mode == preset_binding || binding_mode == "off")
                {
                    return mode.to_string();
                }
            }
        }
        if workflow_scope == "local"
            && task_scope == "local"
            && change_scope == "local"
            && matches!(binding_mode.as_str(), "required" | "off")
        {
            return "solo_local".to_string();
        }
        if workflow_scope == "remote"
            && task_scope == "remote"
            && change_scope == "remote"
            && matches!(binding_mode.as_str(), "advisory" | "off")
        {
            return "solo_remote".to_string();
        }
        if workflow_scope == "remote"
            && task_scope == "remote"
            && change_scope == "remote"
            && binding_mode == "required"
        {
            return "team_remote".to_string();
        }
        "custom".to_string()
    }

    pub fn sprint_enabled(&self) -> bool {
        if let Some(value) = self.config.get("sprint").and_then(JsonValue::as_str) {
            return value.trim().eq_ignore_ascii_case("on");
        }
        self.config
            .get("plan_task_binding")
            .and_then(plan_task_binding_mode)
            .is_none_or(|mode| mode.trim().eq_ignore_ascii_case("required"))
    }

    pub fn team_review_enabled(&self) -> bool {
        self.effective_workflow_mode() == "team_remote"
    }

    fn formatted_user_identity(&self) -> Option<String> {
        let user_name = self.config.get("user_name").and_then(as_nonempty_string);
        let user_email = self.config.get("user_email").and_then(as_nonempty_string);
        match (user_name, user_email) {
            (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
            (None, Some(email)) => Some(email),
            (Some(name), None) => Some(name),
            (None, None) => None,
        }
    }

    pub fn task_uses_local_scope(
        &self,
        local_requested: bool,
        remote_requested: Option<&str>,
    ) -> Result<bool, String> {
        if local_requested && nonempty(remote_requested.map(str::to_string)).is_some() {
            return Err("`--local` cannot be combined with `--remote`.".to_string());
        }
        if local_requested {
            return Ok(true);
        }
        if nonempty(remote_requested.map(str::to_string)).is_some() {
            return Ok(false);
        }
        let configured = self
            .config
            .get("task_default_scope")
            .and_then(as_nonempty_string)
            .or_else(|| {
                self.config
                    .get("workflow_default_scope")
                    .and_then(as_nonempty_string)
            })
            .unwrap_or_else(|| DEFAULT_WORKFLOW_SCOPE.to_string());
        Ok(configured == "local")
    }

    pub fn change_uses_local_scope(
        &self,
        local_requested: bool,
        remote_requested: Option<&str>,
    ) -> bool {
        if local_requested {
            return true;
        }
        if nonempty(remote_requested.map(str::to_string)).is_some() {
            return false;
        }
        let configured = self
            .config
            .get("change_default_scope")
            .and_then(as_nonempty_string)
            .or_else(|| {
                self.config
                    .get("workflow_default_scope")
                    .and_then(as_nonempty_string)
            })
            .unwrap_or_else(|| DEFAULT_WORKFLOW_SCOPE.to_string());
        configured == "local"
    }

    pub fn workflow_uses_local_scope(
        &self,
        local_requested: bool,
        remote_requested: Option<&str>,
    ) -> bool {
        if local_requested {
            return true;
        }
        if nonempty(remote_requested.map(str::to_string)).is_some() {
            return false;
        }
        self.config
            .get("workflow_default_scope")
            .and_then(as_nonempty_string)
            .unwrap_or_else(|| DEFAULT_WORKFLOW_SCOPE.to_string())
            == "local"
    }
}
