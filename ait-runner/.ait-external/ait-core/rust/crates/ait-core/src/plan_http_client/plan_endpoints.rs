use super::*;

impl PlanHttpClientManager {
    pub fn list_plans(
        &mut self,
        repo_name: &str,
        artifact_path: Option<&str>,
    ) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_plans_request_spec(&self.config, repo_name, artifact_path)?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn get_plan(&mut self, plan_id: &str) -> PlanHttpClientResult<Value> {
        let spec = build_get_plan_request_spec(&self.config, plan_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_plan_revisions(&mut self, plan_id: &str) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_plan_revisions_request_spec(&self.config, plan_id)?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn resolve_task_plan_linkage(
        &mut self,
        repo_name: &str,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_resolve_task_plan_linkage_request_spec(
            &self.config,
            repo_name,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_plan_ids_matching_contains(
        &mut self,
        repo_name: &str,
        contains_terms: &[String],
    ) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_plan_ids_matching_contains_request_spec(
            &self.config,
            repo_name,
            contains_terms,
        )?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn read_plan_candidate_inputs(
        &mut self,
        repo_name: &str,
        contains_terms: &[String],
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_read_plan_candidate_inputs_request_spec(&self.config, repo_name, contains_terms)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_plan_revision(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_plan_revision_request_spec(&self.config, plan_id, plan_revision_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_plan(
        &mut self,
        repo_name: &str,
        title: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items: &[Value],
        summary: Option<&str>,
        status: &str,
        plan_id: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        self.create_plan_with_packed_artifact(
            repo_name,
            title,
            artifact_path,
            artifact_selector,
            artifact_heading,
            items,
            summary,
            status,
            plan_id,
            source_kind,
            artifact_body,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_plan_with_packed_artifact(
        &mut self,
        repo_name: &str,
        title: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items: &[Value],
        summary: Option<&str>,
        status: &str,
        plan_id: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
        packed_artifact: Option<&Value>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_create_plan_request_spec(
            &self.config,
            repo_name,
            title,
            artifact_path,
            artifact_selector,
            artifact_heading,
            items,
            summary,
            status,
            plan_id,
            source_kind,
            artifact_body,
            packed_artifact,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn revise_plan(
        &mut self,
        plan_id: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items: &[Value],
        title: Option<&str>,
        summary: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
        expected_head_revision_id: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        self.revise_plan_with_packed_artifact(
            plan_id,
            artifact_path,
            artifact_selector,
            artifact_heading,
            items,
            title,
            summary,
            source_kind,
            artifact_body,
            expected_head_revision_id,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn revise_plan_with_packed_artifact(
        &mut self,
        plan_id: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items: &[Value],
        title: Option<&str>,
        summary: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
        expected_head_revision_id: Option<&str>,
        packed_artifact: Option<&Value>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_revise_plan_request_spec(
            &self.config,
            plan_id,
            artifact_path,
            artifact_selector,
            artifact_heading,
            items,
            title,
            summary,
            source_kind,
            artifact_body,
            expected_head_revision_id,
            packed_artifact,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn update_plan_status(
        &mut self,
        plan_id: &str,
        status: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_update_plan_status_request_spec(&self.config, plan_id, status)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn put_plan_revision_artifacts(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
        artifacts: &[Value],
    ) -> PlanHttpClientResult<Value> {
        let spec = build_put_plan_revision_artifacts_request_spec(
            &self.config,
            plan_id,
            plan_revision_id,
            artifacts,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }
}

pub fn list_plans(
    config: PlanHttpClientConfig,
    repo_name: &str,
    artifact_path: Option<&str>,
) -> PlanHttpClientResult<Vec<Value>> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.list_plans(repo_name, artifact_path)
}

pub fn get_plan(config: PlanHttpClientConfig, plan_id: &str) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.get_plan(plan_id)
}

pub fn list_plan_revisions(
    config: PlanHttpClientConfig,
    plan_id: &str,
) -> PlanHttpClientResult<Vec<Value>> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.list_plan_revisions(plan_id)
}

pub fn resolve_task_plan_linkage(
    config: PlanHttpClientConfig,
    repo_name: &str,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.resolve_task_plan_linkage(repo_name, plan_id, origin_plan_revision_id, plan_item_ref)
}

pub fn list_plan_ids_matching_contains(
    config: PlanHttpClientConfig,
    repo_name: &str,
    contains_terms: &[String],
) -> PlanHttpClientResult<Vec<Value>> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.list_plan_ids_matching_contains(repo_name, contains_terms)
}

pub fn read_plan_candidate_inputs(
    config: PlanHttpClientConfig,
    repo_name: &str,
    contains_terms: &[String],
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.read_plan_candidate_inputs(repo_name, contains_terms)
}

pub fn get_plan_revision(
    config: PlanHttpClientConfig,
    plan_id: &str,
    plan_revision_id: &str,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.get_plan_revision(plan_id, plan_revision_id)
}

#[allow(clippy::too_many_arguments)]
pub fn create_plan(
    config: PlanHttpClientConfig,
    repo_name: &str,
    title: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: &[Value],
    summary: Option<&str>,
    status: &str,
    plan_id: Option<&str>,
    source_kind: &str,
    artifact_body: Option<&str>,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.create_plan(
        repo_name,
        title,
        artifact_path,
        artifact_selector,
        artifact_heading,
        items,
        summary,
        status,
        plan_id,
        source_kind,
        artifact_body,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn revise_plan(
    config: PlanHttpClientConfig,
    plan_id: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: &[Value],
    title: Option<&str>,
    summary: Option<&str>,
    source_kind: &str,
    artifact_body: Option<&str>,
    expected_head_revision_id: Option<&str>,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.revise_plan(
        plan_id,
        artifact_path,
        artifact_selector,
        artifact_heading,
        items,
        title,
        summary,
        source_kind,
        artifact_body,
        expected_head_revision_id,
    )
}

pub fn update_plan_status(
    config: PlanHttpClientConfig,
    plan_id: &str,
    status: &str,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.update_plan_status(plan_id, status)
}

pub fn put_plan_revision_artifacts(
    config: PlanHttpClientConfig,
    plan_id: &str,
    plan_revision_id: &str,
    artifacts: &[Value],
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.put_plan_revision_artifacts(plan_id, plan_revision_id, artifacts)
}
