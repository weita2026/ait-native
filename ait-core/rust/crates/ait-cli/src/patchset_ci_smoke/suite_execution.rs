use super::*;

trait PatchsetSmokeSuite {
    fn suite_id(&self) -> &'static str;
    fn contract(&self) -> &'static str;
    fn is_rust_only(&self) -> bool {
        true
    }
    fn runner(&self) -> &'static str {
        "ait-cli"
    }
    fn run(&self, repo: &RepoRuntime) -> Result<JsonValue, String>;
}

#[derive(Clone, Copy)]
struct PreflightSmokeSuite;

#[derive(Clone, Copy)]
struct PackageSmokeSuite;

#[derive(Clone, Copy)]
struct StableSmokeSuite;

#[derive(Clone, Copy)]
struct ReleaseArtifactSmokeSuite;

fn run_smoke_suite(
    suite: &impl PatchsetSmokeSuite,
    repo: &RepoRuntime,
) -> Result<JsonValue, String> {
    let mut payload = suite.run(repo)?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "Patchset smoke payload is not a JSON object".to_string())?;
    object.insert("suite_id".to_string(), json!(suite.suite_id()));
    object.insert("runner".to_string(), json!(suite.runner()));
    object.insert("rust_only".to_string(), json!(suite.is_rust_only()));
    object.insert("contract".to_string(), json!(suite.contract()));
    Ok(payload)
}

impl PatchsetSmokeSuite for PreflightSmokeSuite {
    fn suite_id(&self) -> &'static str {
        "preflight"
    }

    fn contract(&self) -> &'static str {
        "AT.patchset_ci.preflight.v1"
    }

    fn run(&self, repo: &RepoRuntime) -> Result<JsonValue, String> {
        let root = repo.workspace_root();
        let issues = find_broken_links(&root)?;
        if !issues.is_empty() {
            return Err(format_broken_links(&issues));
        }
        Ok(json!({
            "contract": "AT.patchset_ci.preflight.v1",
            "status": "pass",
            "workspace_root": root.to_string_lossy().to_string(),
            "markdown_file_count": iter_markdown_files(&root)?.len(),
            "broken_link_count": 0,
        }))
    }
}

impl PatchsetSmokeSuite for PackageSmokeSuite {
    fn suite_id(&self) -> &'static str {
        "package-smoke"
    }

    fn contract(&self) -> &'static str {
        "AT.patchset_ci.package_smoke.v1"
    }

    fn run(&self, repo: &RepoRuntime) -> Result<JsonValue, String> {
        let repo_root = repo.workspace_root();
        assert_public_plan_contract(&repo_root)?;
        assert_release_python_authority_retired(&repo_root)?;
        assert_plan_sync_stays_lineage_only()?;
        assert_plan_sync_bypasses_root_worktree_guard()?;
        assert_plan_source_files_omit_legacy_line_alignment_contract(&repo_root)?;
        assert_init_establishes_agent_contract()?;
        assert_sprint_readme_contract(&repo_root)?;
        Ok(json!({
            "contract": "AT.patchset_ci.package_smoke.v1",
            "status": "pass",
            "workspace_root": repo_root.to_string_lossy().to_string(),
            "checks": [
                "public_plan_contract",
                "release_python_authority_retired",
                "plan_sync_lineage_only",
                "plan_sync_root_guard_bypass",
                "plan_source_token_guard",
                "init_bootstrap_guard",
                "sprint_readme_contract"
            ]
        }))
    }
}

impl PatchsetSmokeSuite for ReleaseArtifactSmokeSuite {
    fn suite_id(&self) -> &'static str {
        "release-artifact-smoke"
    }

    fn contract(&self) -> &'static str {
        "AT.patchset_ci.release_artifact_smoke.v1"
    }

    fn run(&self, repo: &RepoRuntime) -> Result<JsonValue, String> {
        crate::release_surface::release_artifact_smoke(repo)
    }
}

impl PatchsetSmokeSuite for StableSmokeSuite {
    fn suite_id(&self) -> &'static str {
        "stable-smoke"
    }

    fn contract(&self) -> &'static str {
        "AT.patchset_ci.stable_smoke.v1"
    }

    fn run(&self, _repo: &RepoRuntime) -> Result<JsonValue, String> {
        let mut remote = spawn_fake_remote();
        let temp = init_fixture_repo(&remote.base_url)?;
        let root = temp.path();
        remote
            .state
            .lock()
            .map_err(|_| "fake remote state lock poisoned".to_string())?
            .remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

        write_file(
            &root.join("src/lib.rs"),
            "pub fn example() -> &'static str { \"reviewable\" }\n",
        )?;
        let snapshot = json_output(
            root,
            &[
                "snapshot",
                "create",
                "--message",
                "reviewable snapshot",
                "--json",
            ],
        )?;
        let snapshot_id = string_field(&snapshot, "snapshot_id")
            .ok_or_else(|| "snapshot create did not return snapshot_id".to_string())?;

        let patchset = json_output(
            root,
            &[
                "patchset",
                "publish",
                "--change",
                "RC-1",
                "--summary",
                "Native Rust patchset",
                "--json",
            ],
        )?;
        let patchset_id = string_field(&patchset["patchset"], "patchset_id")
            .ok_or_else(|| "patchset publish smoke did not return patchset_id".to_string())?;
        if patchset_id != "RP-2" {
            return Err("patchset publish smoke did not return RP-2".to_string());
        }
        if string_field(&patchset, "revision_snapshot_id").as_deref() != Some(snapshot_id.as_str())
        {
            return Err(
                "patchset publish smoke did not use the fresh revision snapshot".to_string(),
            );
        }

        let ci_status = json_output(root, &["patchset", "ci-status", &patchset_id, "--json"])?;
        if string_field(&ci_status, "tests_status").as_deref() != Some("pass") {
            return Err("patchset ci-status smoke did not report pass".to_string());
        }

        let rerun = json_output(root, &["patchset", "rerun-ci", &patchset_id, "--json"])?;
        if rerun.get("queued").and_then(JsonValue::as_bool) != Some(true) {
            return Err("patchset rerun-ci smoke did not queue a job".to_string());
        }

        let approve = json_output(
            root,
            &[
                "review",
                "team",
                "approve",
                "RC-1",
                "--patchset",
                &patchset_id,
                "--json",
            ],
        )?;
        if string_field(&approve, "action").as_deref() != Some("approve") {
            return Err("review team approve smoke did not record approve".to_string());
        }

        let code_review = json_output(
            root,
            &[
                "review",
                "code",
                "submit",
                "RC-1",
                "--patchset",
                &patchset_id,
                "--message",
                "Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: cargo test; Recommendation: land",
                "--json",
            ],
        )?;
        if string_field(&code_review, "action").as_deref() != Some("code_review_summary") {
            return Err(
                "review code submit smoke did not record a code review summary".to_string(),
            );
        }

        let attestation = json_output(
            root,
            &["attest", "put", &patchset_id, "--tests", "pass", "--json"],
        )?;
        if string_field(&attestation, "patchset_id").as_deref() != Some(patchset_id.as_str()) {
            return Err("attest put smoke did not target the published patchset".to_string());
        }
        let attestation_show = json_output(root, &["attest", "show", &patchset_id, "--json"])?;
        if string_field(&attestation_show, "patchset_id").as_deref() != Some(patchset_id.as_str()) {
            return Err("attest show smoke did not target the published patchset".to_string());
        }

        let policy = json_output(root, &["policy", "eval", &patchset_id, "--json"])?;
        if string_field(&policy, "decision").as_deref() != Some("pass") {
            return Err("policy eval smoke did not pass".to_string());
        }

        let task_land = json_output(
            root,
            &[
                "task", "land", "RT-1", "--target", "main", "--mode", "direct", "--json",
            ],
        )?;
        if string_field(&task_land, "apply_status").as_deref() != Some("done") {
            return Err("task land smoke did not finish land and task completion".to_string());
        }

        remote.stop()?;
        let logged = remote
            .log
            .lock()
            .map_err(|_| "fake remote log lock poisoned".to_string())?
            .clone();
        if !logged.iter().any(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets"
        }) {
            return Err(
                "stable smoke did not publish a patchset to the remote contract".to_string(),
            );
        }
        if !logged.iter().any(|row| {
            row.method == "PUT"
                && is_attestation_request(row.url.as_str())
                && body_mentions_pass_tests(&row.body)
        }) {
            return Err("stable smoke did not write the patchset attestation".to_string());
        }
        if !logged.iter().any(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/changes/RC-1/reviews"
                && row.body.contains("\"action\":\"code_review_summary\"")
        }) {
            return Err("stable smoke did not submit the code review summary".to_string());
        }
        let atomic_land_posts = logged
            .iter()
            .filter(|row| {
                row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
            })
            .count();
        if atomic_land_posts != 1 {
            return Err(format!(
                "stable smoke submitted {atomic_land_posts} atomic Task Land requests instead of one"
            ));
        }
        if logged.iter().any(|row| {
            row.method == "POST"
                && row
                    .url
                    .starts_with("/v1/native/repository-authorities/7/changes/")
                && row.url.ends_with(":submit")
        }) {
            return Err("stable smoke fell back to legacy Land submission".to_string());
        }
        Ok(json!({
            "contract": "AT.patchset_ci.stable_smoke.v1",
            "status": "pass",
            "snapshot_id": snapshot_id,
            "request_count": logged.len(),
            "checks": [
                "snapshot_publish",
                "patchset_ci_status",
                "review_flow",
                "attestation_flow",
                "policy_eval",
                "task_land"
            ]
        }))
    }
}

pub fn run_preflight(repo: &RepoRuntime) -> Result<JsonValue, String> {
    run_smoke_suite(&PreflightSmokeSuite, repo)
}

pub fn run_package_smoke(repo: &RepoRuntime) -> Result<JsonValue, String> {
    run_smoke_suite(&PackageSmokeSuite, repo)
}

pub fn run_stable_smoke(repo: &RepoRuntime) -> Result<JsonValue, String> {
    run_smoke_suite(&StableSmokeSuite, repo)
}

pub fn run_release_artifact_smoke(repo: &RepoRuntime) -> Result<JsonValue, String> {
    run_smoke_suite(&ReleaseArtifactSmokeSuite, repo)
}

pub fn run_tg1_required(
    repo: &RepoRuntime,
    requested_case_ids: &[String],
) -> Result<JsonValue, String> {
    let root = repo.workspace_root();
    let cases = resolve_tg1_cases(requested_case_ids)?;
    let mut check_ids = BTreeSet::new();
    for case in &cases {
        check_ids.insert(case.check_id);
    }

    let mut checks = Vec::new();
    for check_id in check_ids {
        run_tg1_check(check_id, repo, &root)?;
        checks.push(json!({
            "check_id": check_id,
            "status": "pass",
        }));
    }

    Ok(json!({
        "contract": "AT.tg1_required.native_runner.v1",
        "status": "pass",
        "runner": "ait-cli",
        "rust_only": true,
        "requested_case_count": requested_case_ids.len(),
        "executed_case_count": cases.len(),
        "formal_member_count": TG1_CASES.len(),
        "case_indices": cases.iter().map(|case| case.index).collect::<Vec<_>>(),
        "case_ids": cases.iter().map(|case| case.local_node_id).collect::<Vec<_>>(),
        "checks": checks,
    }))
}

fn resolve_tg1_cases(requested_case_ids: &[String]) -> Result<Vec<Tg1Case>, String> {
    if requested_case_ids.is_empty() {
        return Ok(TG1_CASES.to_vec());
    }
    let mut cases = Vec::new();
    for requested in requested_case_ids {
        let Some(case) = find_tg1_case(requested) else {
            return Err(format!("Unknown TG1 case id `{requested}`."));
        };
        cases.push(case);
    }
    Ok(cases)
}

fn find_tg1_case(requested: &str) -> Option<Tg1Case> {
    let normalized = requested.trim();
    if normalized.is_empty() {
        return None;
    }
    let requested_test_name = normalized.rsplit("::").next().unwrap_or(normalized);
    TG1_CASES.iter().copied().find(|case| {
        normalized == case.local_node_id
            || normalized == case.corpus_node_id
            || requested_test_name
                == case
                    .local_node_id
                    .rsplit("::")
                    .next()
                    .unwrap_or(case.local_node_id)
            || requested_test_name
                == case
                    .corpus_node_id
                    .rsplit("::")
                    .next()
                    .unwrap_or(case.corpus_node_id)
            || normalized == case.index.to_string()
    })
}
