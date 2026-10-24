use super::*;

pub type RemoteZstdPackPayloads = std::collections::BTreeMap<String, Vec<u8>>;

impl PlanHttpClientManager {
    pub fn prepare_history_promotion(
        &mut self,
        repo_name: &str,
        payload: &Value,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_prepare_history_promotion_request_spec(&self.config, repo_name, payload)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_tasks(&mut self, repo_name: &str) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_tasks_request_spec(&self.config, repo_name)?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn get_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_task_request_spec(&self.config, task_id, repo_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn read_task_audit(
        &mut self,
        repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_read_task_audit_request_spec(&self.config, repo_name, task_id, target_line)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn read_task_queue(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_read_task_queue_request_spec(&self.config, repo_name, status)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn read_reviewer_inbox(&mut self, repo_name: &str) -> PlanHttpClientResult<Value> {
        let spec = build_read_reviewer_inbox_request_spec(&self.config, repo_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn read_queue_summary_bundle(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_read_queue_summary_bundle_request_spec(&self.config, repo_name, status)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_line(&mut self, repo_name: &str, line_name: &str) -> PlanHttpClientResult<Value> {
        let spec = build_get_line_request_spec(&self.config, repo_name, line_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_lines(&mut self, repo_name: &str) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_lines_request_spec(&self.config, repo_name)?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn list_changes(&mut self, repo_name: &str) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_changes_request_spec(&self.config, repo_name)?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn get_change_detail(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_change_detail_request_spec(&self.config, change_ref, repo_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_change(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_change_request_spec(&self.config, change_ref, repo_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn close_change(&mut self, change_id: &str, status: &str) -> PlanHttpClientResult<Value> {
        let spec = build_close_change_request_spec(&self.config, change_id, status)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_update_remote_line_request_spec(
            &self.config,
            repo_name,
            line_name,
            head_snapshot_id,
            expected_head_snapshot_id,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_close_line_request_spec(&self.config, repo_name, line_name, status)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rename_remote_line(
        &mut self,
        repo_name: &str,
        old_line_name: &str,
        new_line_name: &str,
        expected_line_id: &str,
        expected_head_snapshot_id: Option<&str>,
        idempotency_key: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_rename_remote_line_request_spec(
            &self.config,
            repo_name,
            old_line_name,
            new_line_name,
            expected_line_id,
            expected_head_snapshot_id,
            idempotency_key,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn delete_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        expected_line_id: &str,
        expected_head_snapshot_id: Option<&str>,
        idempotency_key: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_delete_remote_line_request_spec(
            &self.config,
            repo_name,
            line_name,
            expected_line_id,
            expected_head_snapshot_id,
            idempotency_key,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn plan_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkPlanRequest,
    ) -> PlanHttpClientResult<ZstdBulkPlanResponse> {
        let spec =
            build_plan_remote_zstd_bulk_typed_request_spec(&self.config, repo_name, request)?;
        let payload = parse_object_payload(self.execute_json(spec)?)?;
        ZstdBulkPlanResponseJson::stateless()
            .decode_value(payload)
            .map_err(PlanHttpClientError::Remote)
    }

    pub fn put_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> PlanHttpClientResult<ZstdPackUploadResponse> {
        let spec = build_put_remote_zstd_object_pack_request_spec(
            &self.config,
            repo_name,
            pack_id,
            pack_bytes,
        )?;
        let method = spec.method.clone();
        let url = spec.url.clone();
        let response_bytes = self.execute_bytes(spec)?;
        let payload =
            parse_object_payload(parse_json_bytes_payload(&method, &url, response_bytes)?)?;
        ZstdPackUploadResponseJson::stateless()
            .decode_value(payload)
            .map_err(PlanHttpClientError::Remote)
    }

    pub fn put_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> PlanHttpClientResult<ZstdPackUploadResponse> {
        let spec = build_put_remote_zstd_tree_pack_request_spec(
            &self.config,
            repo_name,
            pack_id,
            pack_bytes,
        )?;
        let method = spec.method.clone();
        let url = spec.url.clone();
        let response_bytes = self.execute_bytes(spec)?;
        let payload =
            parse_object_payload(parse_json_bytes_payload(&method, &url, response_bytes)?)?;
        ZstdPackUploadResponseJson::stateless()
            .decode_value(payload)
            .map_err(PlanHttpClientError::Remote)
    }

    pub fn put_remote_zstd_packs_bounded(
        &mut self,
        repo_name: &str,
        object_packs: &[(String, Vec<u8>)],
        tree_packs: &[(String, Vec<u8>)],
        max_parallelism: usize,
    ) -> PlanHttpClientResult<(Vec<ZstdPackUploadResponse>, Vec<ZstdPackUploadResponse>)> {
        let mut specs = Vec::with_capacity(object_packs.len() + tree_packs.len());
        let mut response_context = Vec::with_capacity(specs.capacity());
        for (pack_id, pack_bytes) in object_packs {
            let spec = build_put_remote_zstd_object_pack_request_spec(
                &self.config,
                repo_name,
                pack_id,
                pack_bytes,
            )?;
            response_context.push((spec.method.clone(), spec.url.clone()));
            specs.push(spec);
        }
        for (pack_id, pack_bytes) in tree_packs {
            let spec = build_put_remote_zstd_tree_pack_request_spec(
                &self.config,
                repo_name,
                pack_id,
                pack_bytes,
            )?;
            response_context.push((spec.method.clone(), spec.url.clone()));
            specs.push(spec);
        }
        let responses = self.execute_bytes_bounded(specs, max_parallelism)?;
        let mut decoded = Vec::with_capacity(responses.len());
        for (bytes, (method, url)) in responses.into_iter().zip(response_context) {
            let payload = parse_object_payload(parse_json_bytes_payload(&method, &url, bytes)?)?;
            decoded.push(
                ZstdPackUploadResponseJson::stateless()
                    .decode_value(payload)
                    .map_err(PlanHttpClientError::Remote)?,
            );
        }
        let tree_responses = decoded.split_off(object_packs.len());
        Ok((decoded, tree_responses))
    }

    pub fn commit_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkCommitRequest,
    ) -> PlanHttpClientResult<ZstdBulkCommitResponse> {
        let spec =
            build_commit_remote_zstd_bulk_typed_request_spec(&self.config, repo_name, request)?;
        let payload = parse_object_payload(self.execute_json(spec)?)?;
        ZstdBulkCommitResponseJson::stateless()
            .decode_value(payload)
            .map_err(PlanHttpClientError::Remote)
    }

    pub fn get_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> PlanHttpClientResult<Vec<u8>> {
        let spec =
            build_get_remote_zstd_object_pack_request_spec(&self.config, repo_name, pack_id)?;
        self.execute_bytes(spec)
    }

    pub fn get_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> PlanHttpClientResult<Vec<u8>> {
        let spec = build_get_remote_zstd_tree_pack_request_spec(&self.config, repo_name, pack_id)?;
        self.execute_bytes(spec)
    }

    pub fn get_remote_zstd_packs_bounded(
        &mut self,
        repo_name: &str,
        object_pack_ids: &[String],
        tree_pack_ids: &[String],
        max_parallelism: usize,
    ) -> PlanHttpClientResult<(RemoteZstdPackPayloads, RemoteZstdPackPayloads)> {
        let mut specs = Vec::with_capacity(object_pack_ids.len() + tree_pack_ids.len());
        for pack_id in object_pack_ids {
            specs.push(build_get_remote_zstd_object_pack_request_spec(
                &self.config,
                repo_name,
                pack_id,
            )?);
        }
        for pack_id in tree_pack_ids {
            specs.push(build_get_remote_zstd_tree_pack_request_spec(
                &self.config,
                repo_name,
                pack_id,
            )?);
        }
        let responses = self.execute_bytes_bounded(specs, max_parallelism)?;
        let mut responses = responses.into_iter();
        let mut object_packs = std::collections::BTreeMap::new();
        for pack_id in object_pack_ids {
            let bytes = responses.next().ok_or_else(|| {
                PlanHttpClientError::Transport(
                    "bounded object-pack download lost a response".to_string(),
                )
            })?;
            object_packs.insert(pack_id.clone(), bytes);
        }
        let mut tree_packs = std::collections::BTreeMap::new();
        for pack_id in tree_pack_ids {
            let bytes = responses.next().ok_or_else(|| {
                PlanHttpClientError::Transport(
                    "bounded tree-pack download lost a response".to_string(),
                )
            })?;
            tree_packs.insert(pack_id.clone(), bytes);
        }
        debug_assert!(responses.next().is_none());
        Ok((object_packs, tree_packs))
    }

    pub fn get_remote_zstd_import_manifest(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> PlanHttpClientResult<ZstdImportManifestPayload> {
        let spec = build_get_remote_zstd_import_manifest_request_spec(
            &self.config,
            repo_name,
            snapshot_id,
        )?;
        let payload = parse_object_payload(self.execute_json(spec)?)?;
        ZstdImportManifestJson::stateless()
            .decode_value(payload)
            .map_err(PlanHttpClientError::Remote)
    }

    pub fn get_remote_zstd_pull_manifest(
        &mut self,
        repo_name: &str,
        request: &ZstdPullManifestRequest,
    ) -> PlanHttpClientResult<ZstdPullManifestPayload> {
        let spec =
            build_get_remote_zstd_pull_manifest_request_spec(&self.config, repo_name, request)?;
        let payload = parse_object_payload(self.execute_json(spec)?)?;
        ZstdPullManifestJson::stateless()
            .decode_value(payload)
            .map_err(PlanHttpClientError::Remote)
    }

    pub fn get_remote_snapshot(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_remote_snapshot_request_spec(
            &self.config,
            repo_name,
            snapshot_id,
            include_content,
            path,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_remote_snapshots_existence_request_spec(
            &self.config,
            repo_name,
            snapshot_ids,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_release(
        &mut self,
        repo_name: &str,
        release_id: &str,
        version: &str,
        line: &str,
        snapshot_id: &str,
        manifest_hash: &str,
        profile: &str,
        package: Value,
        checks: Value,
        artifacts: Value,
        formula: Value,
        metadata: Value,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_publish_release_request_spec(
            &self.config,
            repo_name,
            release_id,
            version,
            line,
            snapshot_id,
            manifest_hash,
            profile,
            package,
            checks,
            artifacts,
            formula,
            metadata,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_release(
        &mut self,
        repo_name: &str,
        release_ref: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_release_request_spec(&self.config, repo_name, release_ref)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_patchsets(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_patchsets_request_spec(&self.config, change_id, repo_name)?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_get_patchset_request_spec(&self.config, patchset_id, repo_name, change_ref)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_publish_patchset_request_spec(
            &self.config,
            change_id,
            base_snapshot_id,
            revision_snapshot_id,
            summary,
            author_mode,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_select_patchset_request_spec(&self.config, change_id, patchset_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_run_patchset_ci_request_spec(
            &self.config,
            patchset_id,
            trigger,
            execution_profile,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_request_review_request_spec(
            &self.config,
            change_id,
            patchset_id,
            reviewer_groups,
            note,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_read_patchset_ci_status_request_spec(&self.config, patchset_id, recent_limit)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn read_patchset_ci_readiness(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_read_patchset_ci_readiness_request_spec(&self.config, patchset_id, recent_limit)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_record_review_request_spec(
            &self.config,
            change_id,
            patchset_id,
            reviewer,
            action,
            comment,
            blocking,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_reviews(&mut self, change_id: &str) -> PlanHttpClientResult<Value> {
        let spec = build_list_reviews_request_spec(&self.config, change_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &Value,
        provenance_summary: &Value,
        detail: &Value,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_put_attestation_request_spec(
            &self.config,
            patchset_id,
            author_mode,
            evaluation_summary,
            provenance_summary,
            detail,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_attestation(&mut self, patchset_id: &str) -> PlanHttpClientResult<Value> {
        let spec = build_get_attestation_request_spec(&self.config, patchset_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn evaluate_policy(&mut self, patchset_id: &str) -> PlanHttpClientResult<Value> {
        let spec = build_evaluate_policy_request_spec(&self.config, patchset_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_policy(&mut self, patchset_id: &str) -> PlanHttpClientResult<Value> {
        let spec = build_get_policy_request_spec(&self.config, patchset_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_create_waiver_request_spec(
            &self.config,
            patchset_id,
            rule_name,
            reason,
            expires_at,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_submit_land_request_spec(
            &self.config,
            change_id,
            patchset_id,
            target_line,
            mode,
            repo_name,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn submit_task_land(
        &mut self,
        task_or_change_ref: &str,
        target_line: Option<&str>,
        mode: &str,
        idempotency_key: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_submit_task_land_request_spec(
            &self.config,
            task_or_change_ref,
            target_line,
            mode,
            idempotency_key,
            repo_name,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_land(
        &mut self,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_land_request_spec(&self.config, submission_id, repo_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_retry_land_request_spec(&self.config, submission_id, reason, repo_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn close_task(&mut self, task_id: &str, status: &str) -> PlanHttpClientResult<Value> {
        let spec = build_close_task_request_spec(&self.config, task_id, status)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn start_plan_bound_task(
        &mut self,
        repo_name: &str,
        payload: &Value,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_start_plan_bound_task_request_spec(&self.config, repo_name, payload)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_create_task_request_spec(
            &self.config,
            repo_name,
            title,
            intent,
            task_id,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_create_change_request_spec(
            &self.config,
            repo_name,
            task_id,
            title,
            base_line,
            change_id,
            fork_snapshot_id,
            forked_from_line,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }
}

pub fn list_tasks(
    config: PlanHttpClientConfig,
    repo_name: &str,
) -> PlanHttpClientResult<Vec<Value>> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.list_tasks(repo_name)
}
