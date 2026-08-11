use super::super::{http_task_remote, normalized_text, remote_context as parent_remote_context};
use super::sync::{
    pull_line_with_task_remote_and_capabilities,
    push_line_to_remote_with_task_remote_and_capabilities,
    sync_patchset_revision_snapshot_with_task_remote,
    upload_snapshot_chain_to_remote_with_task_remote_and_capabilities,
};
use crate::remote_repository::read_remote_repository_authority;
use crate::runtime::{RemoteRow, RepoRuntime};
use ait_core::json_support::{json, JsonValue};
use ait_core::remote_sync_backend::RemoteSyncCapabilities;
fn attach_http_transport_metrics(
    payload: &mut JsonValue,
    request_count: usize,
    retry_count: usize,
) -> Result<(), String> {
    let payload = payload
        .as_object_mut()
        .ok_or_else(|| "remote sync payload is malformed".to_string())?;
    let metrics = payload
        .entry("remote_sync_metrics".to_string())
        .or_insert_with(|| json!({}));
    let metrics = metrics
        .as_object_mut()
        .ok_or_else(|| "remote sync metrics payload is malformed".to_string())?;
    metrics.insert("remote_round_trips".to_string(), json!(request_count));
    metrics.insert("http_retry_count".to_string(), json!(retry_count));
    Ok(())
}

pub(in crate::primitives) trait RemoteSyncBackend {
    fn remote_context(
        &mut self,
        repo: &RepoRuntime,
        remote_name: Option<&str>,
    ) -> Result<(RemoteRow, String), String>;

    #[expect(
        clippy::too_many_arguments,
        reason = "remote backend port keeps line, storage, and history controls explicit"
    )]
    fn pull_line(
        &mut self,
        repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
        merge: bool,
        restore: bool,
        force: bool,
    ) -> Result<JsonValue, String>;

    fn push_line(
        &mut self,
        repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
    ) -> Result<JsonValue, String>;

    fn upload_snapshot_chain(
        &mut self,
        repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        snapshot_id: &str,
        line_name: Option<&str>,
        reason: Option<&str>,
    ) -> Result<JsonValue, String>;

    fn sync_patchset_revision_snapshot(
        &mut self,
        repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
        revision_snapshot_id: &str,
        base_line: &str,
    ) -> Result<JsonValue, String>;
}

#[derive(Debug, Default)]
pub(super) struct HttpRemoteSyncBackend;

impl RemoteSyncBackend for HttpRemoteSyncBackend {
    fn remote_context(
        &mut self,
        repo: &RepoRuntime,
        remote_name: Option<&str>,
    ) -> Result<(RemoteRow, String), String> {
        parent_remote_context(repo, remote_name, None)
    }

    fn pull_line(
        &mut self,
        repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
        merge: bool,
        restore: bool,
        force: bool,
    ) -> Result<JsonValue, String> {
        let mut task_remote = http_task_remote(repo, remote_row)?;
        let remote_repository =
            read_remote_repository_authority(repo, &mut task_remote, repo_name)?;
        let remote_sync_capabilities =
            RemoteSyncCapabilities::from_server_payload(Some(&remote_repository));
        let mut payload = pull_line_with_task_remote_and_capabilities(
            repo,
            &mut task_remote,
            &remote_row.name,
            repo_name,
            line_name,
            merge,
            restore,
            force,
            &remote_sync_capabilities,
        )?;
        let transport_stats = task_remote.inspect_client();
        attach_http_transport_metrics(
            &mut payload,
            transport_stats.request_count,
            transport_stats.retry_count,
        )?;
        payload
            .as_object_mut()
            .ok_or_else(|| "pull line payload is malformed".to_string())?
            .insert("remote_repository".to_string(), remote_repository);
        Ok(payload)
    }

    fn push_line(
        &mut self,
        repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
    ) -> Result<JsonValue, String> {
        let mut task_remote = http_task_remote(repo, remote_row)?;
        let remote_repository =
            read_remote_repository_authority(repo, &mut task_remote, repo_name)?;
        let remote_sync_capabilities =
            RemoteSyncCapabilities::from_server_payload(Some(&remote_repository));
        let mut payload = push_line_to_remote_with_task_remote_and_capabilities(
            repo,
            &mut task_remote,
            &remote_row.name,
            repo_name,
            line_name,
            &remote_sync_capabilities,
        )?;
        let transport_stats = task_remote.inspect_client();
        attach_http_transport_metrics(
            &mut payload,
            transport_stats.request_count,
            transport_stats.retry_count,
        )?;
        let obj = payload
            .as_object_mut()
            .ok_or_else(|| "push payload is malformed".to_string())?;
        obj.insert("remote_repository".to_string(), remote_repository);
        Ok(payload)
    }

    fn upload_snapshot_chain(
        &mut self,
        repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        snapshot_id: &str,
        line_name: Option<&str>,
        reason: Option<&str>,
    ) -> Result<JsonValue, String> {
        let mut task_remote = http_task_remote(repo, remote_row)?;
        let remote_repository =
            read_remote_repository_authority(repo, &mut task_remote, repo_name)?;
        let remote_sync_capabilities =
            RemoteSyncCapabilities::from_server_payload(Some(&remote_repository));
        let mut payload = upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
            repo,
            &mut task_remote,
            repo_name,
            snapshot_id,
            line_name,
            &remote_sync_capabilities,
        )?;
        let obj = payload
            .as_object_mut()
            .ok_or_else(|| "upload snapshot chain payload is malformed".to_string())?;
        let uploaded = obj
            .get("uploaded_snapshots")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        obj.insert("remote".to_string(), json!(remote_row.name));
        obj.insert(
            "line".to_string(),
            line_name.map(JsonValue::from).unwrap_or(JsonValue::Null),
        );
        obj.insert("line_updated".to_string(), json!(false));
        obj.insert(
            "line_update_skipped_reason".to_string(),
            normalized_text(reason)
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
        );
        obj.insert("pushed_snapshots".to_string(), json!(uploaded));
        obj.insert("head_snapshot_id".to_string(), json!(snapshot_id));
        obj.insert("remote_repository".to_string(), remote_repository);
        obj.insert("remote_line".to_string(), JsonValue::Null);
        Ok(payload)
    }

    fn sync_patchset_revision_snapshot(
        &mut self,
        repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
        revision_snapshot_id: &str,
        base_line: &str,
    ) -> Result<JsonValue, String> {
        let mut task_remote = http_task_remote(repo, remote_row)?;
        sync_patchset_revision_snapshot_with_task_remote(
            repo,
            &mut task_remote,
            &remote_row.name,
            repo_name,
            line_name,
            revision_snapshot_id,
            base_line,
        )
    }
}
