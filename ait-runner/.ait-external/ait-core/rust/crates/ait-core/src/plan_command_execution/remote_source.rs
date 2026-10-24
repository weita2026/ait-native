use crate::json_support::JsonValue;

use super::data_source::{
    PlanCommandCandidateInputSource, PlanCommandCandidateInputs, PlanCommandPlanLister,
    PlanCommandPlanReader, PlanCommandRevisionLister, PlanCommandRevisionReader,
    PlanCommandTaskLister,
};
use crate::plan_http_client::PlanHttpClientManager;

pub(super) struct RemotePlanCommandSource {
    client: PlanHttpClientManager,
}

impl RemotePlanCommandSource {
    pub(super) fn new(client: PlanHttpClientManager) -> Self {
        Self { client }
    }
}

impl PlanCommandPlanLister for RemotePlanCommandSource {
    fn list_plans(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String> {
        self.client
            .list_plans(repo_name, None)
            .map_err(|err| err.to_string())
    }
}

impl PlanCommandPlanReader for RemotePlanCommandSource {
    fn get_plan(&mut self, plan_id: &str) -> Result<JsonValue, String> {
        self.client.get_plan(plan_id).map_err(|err| err.to_string())
    }
}

impl PlanCommandRevisionLister for RemotePlanCommandSource {
    fn list_plan_revisions(&mut self, plan_id: &str) -> Result<Vec<JsonValue>, String> {
        self.client
            .list_plan_revisions(plan_id)
            .map_err(|err| err.to_string())
    }
}

impl PlanCommandRevisionReader for RemotePlanCommandSource {
    fn get_plan_revision(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        self.client
            .get_plan_revision(plan_id, plan_revision_id)
            .map_err(|err| err.to_string())
    }
}

impl PlanCommandTaskLister for RemotePlanCommandSource {
    fn list_tasks(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String> {
        self.client
            .list_tasks(repo_name)
            .map_err(|err| err.to_string())
    }
}

impl PlanCommandCandidateInputSource for RemotePlanCommandSource {
    fn candidate_inputs(
        &mut self,
        repo_name: &str,
        contains_terms: &[String],
    ) -> Result<PlanCommandCandidateInputs, String> {
        let (plans, tasks) =
            load_remote_candidate_inputs(&mut self.client, repo_name, contains_terms)?;
        Ok(PlanCommandCandidateInputs { plans, tasks })
    }
}

fn load_remote_candidate_inputs(
    client: &mut PlanHttpClientManager,
    repo_name: &str,
    contains_terms: &[String],
) -> Result<(Vec<JsonValue>, Vec<JsonValue>), String> {
    let payload = client
        .read_plan_candidate_inputs(repo_name, contains_terms)
        .map_err(|err| err.to_string())?;
    let object = payload
        .as_object()
        .ok_or_else(|| "Remote plan candidate inputs payload must be an object.".to_string())?;
    let plans = object
        .get("plans")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Remote plan candidate inputs payload must include plans.".to_string())?
        .clone();
    let tasks = object
        .get("tasks")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Remote plan candidate inputs payload must include tasks.".to_string())?
        .clone();
    Ok((plans, tasks))
}
