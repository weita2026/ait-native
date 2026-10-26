//! Retained stateful task and workflow HTTP adapter classes.
use crate::{
    plan_http::{client_config, http_error, PlanHttpOptions},
    JsonInput,
};
use ait_core::{
    json_support::JsonValue, plan_http_sdk, repository_pack_json::*,
    task_workflow_http_adapter as remote,
};
use napi::{bindgen_prelude::Buffer, Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct CreateRemoteTaskOptions {
    pub task_id: Option<String>,
    pub plan_id: Option<String>,
    pub origin_plan_revision_id: Option<String>,
    pub plan_item_ref: Option<String>,
}
#[napi(object)]
#[derive(Default)]
pub struct CreateRemoteChangeOptions {
    pub change_id: Option<String>,
    pub fork_snapshot_id: Option<String>,
    pub forked_from_line: Option<String>,
}
#[napi(object)]
#[derive(Default)]
pub struct RemoteLineHeadOptions {
    pub head_snapshot_id: Option<String>,
    pub expected_head_snapshot_id: Option<String>,
}
#[napi(object)]
#[derive(Default)]
pub struct RemoteSnapshotOptions {
    pub include_content: Option<bool>,
    pub path: Option<String>,
}

#[napi]
pub struct HttpTaskRemote {
    adapter: remote::HttpTaskRemote,
}
#[napi]
impl HttpTaskRemote {
    #[napi(constructor)]
    pub fn new(base_url: String, options: Option<PlanHttpOptions>) -> Result<Self> {
        Ok(Self {
            adapter: remote::HttpTaskRemote::new(client_config(&base_url, options)?)
                .map_err(http_error)?,
        })
    }
    #[napi]
    pub fn inspect_client(&self) -> JsonValue {
        plan_http_sdk::client_stats_payload(self.adapter.inspect_client())
    }
    #[napi]
    pub fn close_client(&mut self) -> JsonValue {
        plan_http_sdk::client_stats_payload(self.adapter.close_client())
    }
    #[napi]
    pub fn change_lineage_payload(
        &self,
        base_line: String,
        line_row: Option<JsonInput>,
    ) -> Result<JsonValue> {
        self.adapter
            .change_lineage_payload(&base_line, line_row.as_deref())
            .map_err(Error::from_reason)
    }

    #[napi]
    pub fn get_line(&mut self, repo_name: String, line_name: String) -> Result<JsonValue> {
        self.adapter
            .get_line(&repo_name, &line_name)
            .map_err(http_error)
    }

    #[napi]
    pub fn get_task(&mut self, task_id: String, repo_name: Option<String>) -> Result<JsonValue> {
        self.adapter
            .get_task(&task_id, repo_name.as_deref())
            .map_err(http_error)
    }

    #[napi]
    pub fn list_tasks(&mut self, repo_name: String) -> Result<Vec<JsonValue>> {
        self.adapter.list_tasks(&repo_name).map_err(http_error)
    }

    #[napi]
    pub fn create_task(
        &mut self,
        repo_name: String,
        title: String,
        intent: String,
        options: Option<CreateRemoteTaskOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .create_task(
                &repo_name,
                &title,
                &intent,
                options.task_id.as_deref(),
                options.plan_id.as_deref(),
                options.origin_plan_revision_id.as_deref(),
                options.plan_item_ref.as_deref(),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn list_changes(&mut self, repo_name: String) -> Result<Vec<JsonValue>> {
        self.adapter.list_changes(&repo_name).map_err(http_error)
    }

    #[napi]
    pub fn get_change_detail(
        &mut self,
        change_ref: String,
        repo_name: Option<String>,
    ) -> Result<JsonValue> {
        self.adapter
            .get_change_detail(&change_ref, repo_name.as_deref())
            .map_err(http_error)
    }

    #[napi]
    pub fn get_change(
        &mut self,
        change_ref: String,
        repo_name: Option<String>,
    ) -> Result<JsonValue> {
        self.adapter
            .get_change(&change_ref, repo_name.as_deref())
            .map_err(http_error)
    }

    #[napi]
    pub fn close_change(
        &mut self,
        change_ref: String,
        status: Option<String>,
        repo_name: Option<String>,
    ) -> Result<JsonValue> {
        self.adapter
            .close_change(
                &change_ref,
                status.as_deref().unwrap_or("archived"),
                repo_name.as_deref(),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn create_change(
        &mut self,
        repo_name: String,
        task_id: String,
        title: String,
        base_line: String,
        options: Option<CreateRemoteChangeOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .create_change(
                &repo_name,
                &task_id,
                &title,
                &base_line,
                options.change_id.as_deref(),
                options.fork_snapshot_id.as_deref(),
                options.forked_from_line.as_deref(),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn update_remote_line(
        &mut self,
        repo_name: String,
        line_name: String,
        options: Option<RemoteLineHeadOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .update_remote_line(
                &repo_name,
                &line_name,
                options.head_snapshot_id.as_deref(),
                options.expected_head_snapshot_id.as_deref(),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn get_remote_snapshot(
        &mut self,
        repo_name: String,
        snapshot_id: String,
        options: Option<RemoteSnapshotOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .get_remote_snapshot(
                &repo_name,
                &snapshot_id,
                options.include_content.unwrap_or(false),
                options.path.as_deref(),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn get_remote_snapshots_existence(
        &mut self,
        repo_name: String,
        snapshot_ids: Vec<String>,
    ) -> Result<JsonValue> {
        self.adapter
            .get_remote_snapshots_existence(&repo_name, &snapshot_ids)
            .map_err(http_error)
    }

    #[napi]
    pub fn plan_remote_zstd_bulk(
        &mut self,
        repo_name: String,
        request: JsonInput,
    ) -> Result<JsonValue> {
        let request = ZstdBulkPlanRequestJson::stateless()
            .decode_value(request.0)
            .map_err(Error::from_reason)?;
        let response = self
            .adapter
            .plan_remote_zstd_bulk(&repo_name, &request)
            .map_err(http_error)?;
        ZstdBulkPlanResponseJson::stateless()
            .encode_value(&response)
            .map_err(Error::from_reason)
    }

    #[napi]
    pub fn commit_remote_zstd_bulk(
        &mut self,
        repo_name: String,
        request: JsonInput,
    ) -> Result<JsonValue> {
        let request = ZstdBulkCommitRequestJson::stateless()
            .decode_value(request.0)
            .map_err(Error::from_reason)?;
        let response = self
            .adapter
            .commit_remote_zstd_bulk(&repo_name, &request)
            .map_err(http_error)?;
        ZstdBulkCommitResponseJson::stateless()
            .encode_value(&response)
            .map_err(Error::from_reason)
    }

    #[napi]
    pub fn put_remote_zstd_object_pack(
        &mut self,
        repo_name: String,
        pack_id: String,
        pack_bytes: Buffer,
    ) -> Result<JsonValue> {
        let response = self
            .adapter
            .put_remote_zstd_object_pack(&repo_name, &pack_id, &pack_bytes)
            .map_err(http_error)?;
        ZstdPackUploadResponseJson::stateless()
            .encode_value(&response)
            .map_err(Error::from_reason)
    }

    #[napi]
    pub fn put_remote_zstd_tree_pack(
        &mut self,
        repo_name: String,
        pack_id: String,
        pack_bytes: Buffer,
    ) -> Result<JsonValue> {
        let response = self
            .adapter
            .put_remote_zstd_tree_pack(&repo_name, &pack_id, &pack_bytes)
            .map_err(http_error)?;
        ZstdPackUploadResponseJson::stateless()
            .encode_value(&response)
            .map_err(Error::from_reason)
    }

    #[napi]
    pub fn get_remote_zstd_import_manifest(
        &mut self,
        repo_name: String,
        snapshot_id: String,
    ) -> Result<JsonValue> {
        let response = self
            .adapter
            .get_remote_zstd_import_manifest(&repo_name, &snapshot_id)
            .map_err(http_error)?;
        ZstdImportManifestJson::stateless()
            .encode_value(&response)
            .map_err(Error::from_reason)
    }

    #[napi]
    pub fn get_remote_zstd_object_pack(
        &mut self,
        repo_name: String,
        pack_id: String,
    ) -> Result<Buffer> {
        self.adapter
            .get_remote_zstd_object_pack(&repo_name, &pack_id)
            .map(Buffer::from)
            .map_err(http_error)
    }

    #[napi]
    pub fn get_remote_zstd_tree_pack(
        &mut self,
        repo_name: String,
        pack_id: String,
    ) -> Result<Buffer> {
        self.adapter
            .get_remote_zstd_tree_pack(&repo_name, &pack_id)
            .map(Buffer::from)
            .map_err(http_error)
    }
}

#[napi(object)]
#[derive(Default)]
pub struct RemoteTargetOptions {
    pub repo_name: Option<String>,
    pub exact_id: Option<bool>,
}
#[napi(object)]
#[derive(Default)]
pub struct GetRemotePatchsetOptions {
    pub repo_name: Option<String>,
    pub change_ref: Option<String>,
}
#[napi(object)]
pub struct PublishRemotePatchsetInput {
    pub change_id: String,
    pub base_snapshot_id: String,
    pub revision_snapshot_id: String,
    pub summary: String,
    pub author_mode: String,
    pub repo_name: Option<String>,
    pub exact_id: Option<bool>,
}
#[napi(object)]
#[derive(Default)]
pub struct RemotePatchsetCiOptions {
    pub trigger: Option<String>,
    pub execution_profile: Option<String>,
    pub repo_name: Option<String>,
    pub exact_id: Option<bool>,
}
#[napi(object)]
#[derive(Default)]
pub struct RemotePatchsetCiStatusOptions {
    pub recent_limit: Option<JsonInput>,
    pub repo_name: Option<String>,
    pub exact_id: Option<bool>,
}
#[napi(object)]
pub struct RemoteReviewRequestInput {
    pub change_id: String,
    pub patchset_id: String,
    pub reviewer_groups: Vec<String>,
    pub note: Option<String>,
    pub repo_name: Option<String>,
    pub exact_id: Option<bool>,
}
#[napi(object)]
pub struct RemoteReviewRecordInput {
    pub change_id: String,
    pub patchset_id: String,
    pub reviewer: String,
    pub action: String,
    pub comment: Option<String>,
    pub blocking: Option<bool>,
    pub repo_name: Option<String>,
    pub exact_id: Option<bool>,
}
#[napi(object)]
pub struct RemoteWaiverInput {
    pub patchset_id: String,
    pub rule_name: String,
    pub reason: String,
    pub expires_at: Option<String>,
    pub repo_name: Option<String>,
    pub exact_id: Option<bool>,
}
#[napi(object)]
pub struct RemoteSubmissionInput {
    pub change_id: String,
    pub patchset_id: Option<String>,
    pub target_line: Option<String>,
    pub mode: Option<String>,
    pub repo_name: Option<String>,
}
#[napi(object)]
#[derive(Default)]
pub struct RemoteCloseTaskOptions {
    pub status: Option<String>,
    pub repo_name: Option<String>,
}
#[napi(object)]
pub struct RemoteMutationReceiptInput {
    pub action: String,
    pub source_action: String,
    pub delivery: String,
    pub response_recovery: Option<JsonInput>,
    pub result: Option<JsonInput>,
}

#[napi]
pub struct HttpWorkflowCloseoutRemote {
    adapter: remote::HttpWorkflowCloseoutRemote,
}
#[napi]
impl HttpWorkflowCloseoutRemote {
    #[napi(constructor)]
    pub fn new(base_url: String, options: Option<PlanHttpOptions>) -> Result<Self> {
        Ok(Self {
            adapter: remote::HttpWorkflowCloseoutRemote::new(client_config(&base_url, options)?)
                .map_err(http_error)?,
        })
    }
    #[napi]
    pub fn inspect_client(&self) -> JsonValue {
        plan_http_sdk::client_stats_payload(self.adapter.inspect_client())
    }
    #[napi]
    pub fn close_client(&mut self) -> JsonValue {
        plan_http_sdk::client_stats_payload(self.adapter.close_client())
    }
    #[napi]
    pub fn mutation_receipt(&self, input: RemoteMutationReceiptInput) -> Result<JsonValue> {
        self.adapter
            .mutation_receipt(
                &input.action,
                &input.source_action,
                &input.delivery,
                input.response_recovery.as_deref(),
                input.result.as_deref(),
            )
            .map_err(Error::from_reason)
    }
    #[napi]
    pub fn action_mutation_receipts(&self, code: String, result: JsonInput) -> Result<JsonValue> {
        self.adapter
            .action_mutation_receipts(&code, &result)
            .map_err(Error::from_reason)
    }

    #[napi]
    pub fn list_patchsets(
        &mut self,
        change_id: String,
        repo_name: Option<String>,
    ) -> Result<Vec<JsonValue>> {
        self.adapter
            .list_patchsets(&change_id, repo_name.as_deref())
            .map_err(http_error)
    }

    #[napi]
    pub fn get_patchset(
        &mut self,
        patchset_id: String,
        options: Option<GetRemotePatchsetOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .get_patchset(
                &patchset_id,
                options.repo_name.as_deref(),
                options.change_ref.as_deref(),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn publish_patchset(&mut self, input: PublishRemotePatchsetInput) -> Result<JsonValue> {
        self.adapter
            .publish_patchset(
                &input.change_id,
                &input.base_snapshot_id,
                &input.revision_snapshot_id,
                &input.summary,
                &input.author_mode,
                input.repo_name.as_deref(),
                input.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn select_patchset(
        &mut self,
        change_id: String,
        patchset_id: String,
        options: Option<RemoteTargetOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .select_patchset(
                &change_id,
                &patchset_id,
                options.repo_name.as_deref(),
                options.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn run_patchset_ci(
        &mut self,
        patchset_id: String,
        options: Option<RemotePatchsetCiOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .run_patchset_ci(
                &patchset_id,
                options.trigger.as_deref().unwrap_or("manual_rerun"),
                options.execution_profile.as_deref(),
                options.repo_name.as_deref(),
                options.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn read_patchset_ci_status(
        &mut self,
        patchset_id: String,
        options: Option<RemotePatchsetCiStatusOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        let recent_limit = options
            .recent_limit
            .map(|value| {
                value
                    .as_i64()
                    .ok_or_else(|| Error::from_reason("recent_limit must fit in i64."))
            })
            .transpose()?
            .unwrap_or(10);
        self.adapter
            .read_patchset_ci_status(
                &patchset_id,
                recent_limit,
                options.repo_name.as_deref(),
                options.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn request_review(&mut self, input: RemoteReviewRequestInput) -> Result<JsonValue> {
        self.adapter
            .request_review(
                &input.change_id,
                &input.patchset_id,
                &input.reviewer_groups,
                input.note.as_deref(),
                input.repo_name.as_deref(),
                input.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn list_reviews(
        &mut self,
        change_id: String,
        options: Option<RemoteTargetOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .list_reviews(
                &change_id,
                options.repo_name.as_deref(),
                options.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn record_review(&mut self, input: RemoteReviewRecordInput) -> Result<JsonValue> {
        self.adapter
            .record_review(
                &input.change_id,
                &input.patchset_id,
                &input.reviewer,
                &input.action,
                input.comment.as_deref(),
                input.blocking.unwrap_or(false),
                input.repo_name.as_deref(),
                input.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn get_attestation(
        &mut self,
        patchset_id: String,
        options: Option<RemoteTargetOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .get_attestation(
                &patchset_id,
                options.repo_name.as_deref(),
                options.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn evaluate_policy(
        &mut self,
        patchset_id: String,
        options: Option<RemoteTargetOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .evaluate_policy(
                &patchset_id,
                options.repo_name.as_deref(),
                options.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn get_policy(
        &mut self,
        patchset_id: String,
        options: Option<RemoteTargetOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .get_policy(
                &patchset_id,
                options.repo_name.as_deref(),
                options.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn create_waiver(&mut self, input: RemoteWaiverInput) -> Result<JsonValue> {
        self.adapter
            .create_waiver(
                &input.patchset_id,
                &input.rule_name,
                &input.reason,
                input.expires_at.as_deref(),
                input.repo_name.as_deref(),
                input.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn submit_land(&mut self, input: RemoteSubmissionInput) -> Result<JsonValue> {
        self.adapter
            .submit_land(
                &input.change_id,
                input.patchset_id.as_deref(),
                input.target_line.as_deref().unwrap_or("main"),
                input.mode.as_deref().unwrap_or("direct"),
                input.repo_name.as_deref(),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn get_land(
        &mut self,
        submission_id: String,
        repo_name: Option<String>,
    ) -> Result<JsonValue> {
        self.adapter
            .get_land(&submission_id, repo_name.as_deref())
            .map_err(http_error)
    }

    #[napi]
    pub fn retry_land(
        &mut self,
        submission_id: String,
        reason: Option<String>,
        repo_name: Option<String>,
    ) -> Result<JsonValue> {
        self.adapter
            .retry_land(&submission_id, reason.as_deref(), repo_name.as_deref())
            .map_err(http_error)
    }

    #[napi]
    pub fn close_task(
        &mut self,
        task_id: String,
        options: Option<RemoteCloseTaskOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        self.adapter
            .close_task(
                &task_id,
                options.status.as_deref().unwrap_or("completed"),
                options.repo_name.as_deref(),
            )
            .map_err(http_error)
    }

    #[napi]
    pub fn put_attestation(
        &mut self,
        patchset_id: String,
        author_mode: String,
        statement: JsonInput,
        options: Option<RemoteTargetOptions>,
    ) -> Result<JsonValue> {
        let options = options.unwrap_or_default();
        let statement = plan_http_sdk::parse_compact_attestation_statement(statement.0)
            .map_err(Error::from_reason)?;
        self.adapter
            .put_attestation(
                &patchset_id,
                &author_mode,
                &statement,
                options.repo_name.as_deref(),
                options.exact_id.unwrap_or(false),
            )
            .map_err(http_error)
    }
}
