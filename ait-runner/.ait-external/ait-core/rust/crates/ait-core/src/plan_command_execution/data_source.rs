use crate::json_support::JsonValue;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlanCommandCandidateInputs {
    pub plans: Vec<JsonValue>,
    pub tasks: Vec<JsonValue>,
}

pub(super) trait PlanCommandPlanLister {
    fn list_plans(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String>;
}

pub(super) trait PlanCommandPlanReader {
    fn get_plan(&mut self, plan_id: &str) -> Result<JsonValue, String>;
}

pub(super) trait PlanCommandRevisionLister {
    fn list_plan_revisions(&mut self, plan_id: &str) -> Result<Vec<JsonValue>, String>;
}

pub(super) trait PlanCommandRevisionReader {
    fn get_plan_revision(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String>;
}

pub(super) trait PlanCommandTaskLister {
    fn list_tasks(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String>;
}

pub(super) trait PlanCommandCandidateInputSource {
    fn candidate_inputs(
        &mut self,
        repo_name: &str,
        contains_terms: &[String],
    ) -> Result<PlanCommandCandidateInputs, String>;
}

pub(super) trait PlanCommandPlanRevisionReader:
    PlanCommandPlanReader + PlanCommandRevisionReader
{
}

impl<S> PlanCommandPlanRevisionReader for S where
    S: PlanCommandPlanReader + PlanCommandRevisionReader + ?Sized
{
}

pub(super) trait PlanCommandInspectSource:
    PlanCommandPlanRevisionReader + PlanCommandTaskLister
{
}

impl<S> PlanCommandInspectSource for S where
    S: PlanCommandPlanRevisionReader + PlanCommandTaskLister + ?Sized
{
}

pub(super) fn list_plans_with_plan_command_data_source<S>(
    source: &mut S,
    repo_name: &str,
) -> Result<Vec<JsonValue>, String>
where
    S: PlanCommandPlanLister + ?Sized,
{
    source.list_plans(repo_name)
}

pub(super) fn get_plan_with_plan_command_data_source<S>(
    source: &mut S,
    plan_id: &str,
) -> Result<JsonValue, String>
where
    S: PlanCommandPlanReader + ?Sized,
{
    source.get_plan(plan_id)
}

pub(super) fn list_plan_revisions_with_plan_command_data_source<S>(
    source: &mut S,
    plan_id: &str,
) -> Result<Vec<JsonValue>, String>
where
    S: PlanCommandRevisionLister + ?Sized,
{
    source.list_plan_revisions(plan_id)
}

pub(super) fn get_plan_revision_with_plan_command_data_source<S>(
    source: &mut S,
    plan_id: &str,
    plan_revision_id: &str,
) -> Result<JsonValue, String>
where
    S: PlanCommandRevisionReader + ?Sized,
{
    source.get_plan_revision(plan_id, plan_revision_id)
}

pub(super) fn list_tasks_with_plan_command_data_source<S>(
    source: &mut S,
    repo_name: &str,
) -> Result<Vec<JsonValue>, String>
where
    S: PlanCommandTaskLister + ?Sized,
{
    source.list_tasks(repo_name)
}

pub(super) fn candidate_inputs_with_plan_command_data_source<S>(
    source: &mut S,
    repo_name: &str,
    contains_terms: &[String],
) -> Result<PlanCommandCandidateInputs, String>
where
    S: PlanCommandCandidateInputSource + ?Sized,
{
    source.candidate_inputs(repo_name, contains_terms)
}
