use super::local_ports::PlanSyncLocalRevisionArtifact;
use super::*;
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct BoundaryBlobStore {
    calls: RefCell<Vec<String>>,
}

impl PlanSyncLocalBlobStore for BoundaryBlobStore {
    fn ensure_blob_bytes(&self, data: &[u8], path_hint: Option<&str>) -> Result<String, String> {
        let body = std::str::from_utf8(data).map_err(|error| error.to_string())?;
        let blob_id = artifact_blob_id(body);
        self.calls
            .borrow_mut()
            .push(format!("ensure:{}:{blob_id}", path_hint.unwrap_or("")));
        Ok(blob_id)
    }

    fn read_blob_bytes(&self, blob_id: &str) -> Result<Vec<u8>, String> {
        Err(format!("unexpected boundary-test blob read: {blob_id}"))
    }

    fn blob_chain_depth(&self, _blob_id: &str) -> Result<Option<i64>, String> {
        Ok(Some(0))
    }
}

#[derive(Default)]
struct BoundaryIdentitySource {
    workflow_ids: Cell<usize>,
    timestamps: Cell<usize>,
}

impl PlanSyncWorkflowIdentitySource for BoundaryIdentitySource {
    fn workflow_id(&self, family: &str, _namespace_prefix: Option<&str>) -> Result<String, String> {
        let next = self.workflow_ids.get() + 1;
        self.workflow_ids.set(next);
        Ok(format!("{family}-BOUNDARY-{next}"))
    }

    fn timestamp(&self) -> Result<String, String> {
        self.timestamps.set(self.timestamps.get() + 1);
        Ok("2026-07-15T00:00:00Z".to_string())
    }
}

#[derive(Default)]
struct BoundaryLocalStore {
    calls: RefCell<Vec<String>>,
    existing_plans: RefCell<BTreeMap<String, JsonValue>>,
    revisions: RefCell<BTreeMap<String, Vec<JsonValue>>>,
    next_created_plan_id: RefCell<Option<String>>,
    next_created_revision_id: RefCell<Option<String>>,
    preserve_remote_identity: bool,
}

impl PlanSyncLocalPlanStore for BoundaryLocalStore {
    fn get_plan(&self, plan_id: &str) -> Result<JsonValue, String> {
        self.existing_plans
            .borrow()
            .get(plan_id)
            .cloned()
            .ok_or_else(|| format!("missing boundary-test plan: {plan_id}"))
    }
}

impl PlanSyncLocalRevisionStore for BoundaryLocalStore {
    fn list_plan_revisions(&self, plan_id: &str) -> Result<Vec<JsonValue>, String> {
        self.revisions
            .borrow()
            .get(plan_id)
            .cloned()
            .ok_or_else(|| format!("unexpected boundary-test revision listing: {plan_id}"))
    }

    fn get_plan_revision_artifact(
        &self,
        _plan_revision_id: &str,
    ) -> Result<Option<PlanSyncLocalRevisionArtifact>, String> {
        Ok(None)
    }
}

impl PlanSyncLocalArtifactWriter for BoundaryLocalStore {
    fn create_plan(&self, request: &PlanSyncLocalPlanCreate<'_>) -> Result<JsonValue, String> {
        let created_plan_id = self
            .next_created_plan_id
            .borrow()
            .clone()
            .unwrap_or_else(|| request.plan_id.to_string());
        let created_revision_id = self
            .next_created_revision_id
            .borrow()
            .clone()
            .unwrap_or_else(|| request.plan_revision_id.to_string());
        self.calls.borrow_mut().push(format!(
            "create:{}:{}",
            request.plan_id, request.plan_revision_id
        ));
        let items =
            JsonCodec::parse_value_with_error_prefix(request.items_json, "boundary-test items")
                .map_err(String::from)?;
        Ok(json!({
            "plan_id": created_plan_id,
            "repo_name": request.repo_name,
            "title": request.title,
            "status": request.status,
            "publication_state": request.publication_state,
            "head_revision_id": created_revision_id,
            "head_revision": {
                "plan_revision_id": created_revision_id,
                "artifact_path": request.artifact_path,
                "artifact_selector": request.artifact_selector,
                "artifact_heading": request.artifact_heading,
                "artifact_blob_id": request.artifact_blob_id,
                "items": items,
                "publication_state": request.publication_state,
            }
        }))
    }

    fn revise_plan(&self, request: &PlanSyncLocalPlanRevision<'_>) -> Result<JsonValue, String> {
        Err(format!(
            "unexpected boundary-test local revise: {}",
            request.plan_id
        ))
    }
}

impl PlanSyncLocalPublicationStore for BoundaryLocalStore {
    fn remote_adoption_allocates_fresh_local_plan_identity(&self) -> bool {
        self.next_created_plan_id.borrow().is_some()
    }

    fn remote_adoption_preserves_local_plan_identity(&self) -> bool {
        self.preserve_remote_identity
    }

    fn mark_plan_published(
        &self,
        plan_id: &str,
        _remote_name: Option<&str>,
        published_plan_id: &str,
        published_head_revision_id: Option<&str>,
        revision_mappings: &[(String, String)],
        _published_at: &str,
    ) -> Result<JsonValue, String> {
        self.calls.borrow_mut().push(format!(
            "publish:{plan_id}:{}",
            published_head_revision_id.unwrap_or("")
        ));
        let local_head = revision_mappings
            .iter()
            .find(|(_, remote)| Some(remote.as_str()) == published_head_revision_id)
            .map(|(local, _)| local.clone())
            .ok_or_else(|| "boundary-test publication has no head mapping".to_string())?;
        Ok(json!({
            "plan_id": plan_id,
            "status": "draft",
            "publication_state": "published",
            "published_plan_id": published_plan_id,
            "published_head_revision_id": published_head_revision_id,
            "head_revision_id": local_head,
            "head_revision": {
                "plan_revision_id": local_head,
                "publication_state": "published",
            }
        }))
    }
}

impl PlanSyncLocalLifecycleStore for BoundaryLocalStore {
    fn close_plan(
        &self,
        plan_id: &str,
        status: &str,
        _closed_at: &str,
    ) -> Result<JsonValue, String> {
        self.calls
            .borrow_mut()
            .push(format!("close:{plan_id}:{status}"));
        Ok(json!({
            "plan_id": plan_id,
            "status": status,
            "head_revision": {
                "artifact_path": "docs/sprints/boundary.md",
                "artifact_selector": null,
            }
        }))
    }

    fn rekey_plan(
        &self,
        plan_id: &str,
        new_plan_id: &str,
        _rekeyed_at: &str,
    ) -> Result<JsonValue, String> {
        Err(format!(
            "unexpected boundary-test rekey: {plan_id} -> {new_plan_id}"
        ))
    }
}

#[derive(Default)]
struct BoundaryRemote {
    details: BTreeMap<(String, String), JsonValue>,
    histories: BTreeMap<String, Vec<JsonValue>>,
    calls: Vec<String>,
}

impl PlanSyncRemoteInventorySource for BoundaryRemote {
    fn list_plan_summaries(
        &mut self,
        repo_name: &str,
        _artifact_path: Option<&str>,
    ) -> Result<Vec<JsonValue>, String> {
        self.calls.push(format!("unexpected_inventory:{repo_name}"));
        Err("boundary test must use supplied remote inventory".to_string())
    }
}

impl PlanSyncRemoteRevisionLister for BoundaryRemote {
    fn list_plan_revisions(&mut self, plan_id: &str) -> Result<Vec<JsonValue>, String> {
        self.calls.push(format!("history:{plan_id}"));
        self.histories
            .get(plan_id)
            .cloned()
            .ok_or_else(|| "historical revision closure is unavailable".to_string())
    }
}

impl PlanSyncRemoteRevisionReader for BoundaryRemote {
    fn get_plan_revision(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        self.calls
            .push(format!("head:{plan_id}:{plan_revision_id}"));
        self.details
            .get(&(plan_id.to_string(), plan_revision_id.to_string()))
            .cloned()
            .ok_or_else(|| format!("missing boundary-test revision: {plan_revision_id}"))
    }
}

fn boundary_request(rebase: bool) -> SyncRequest {
    SyncRequest {
        root_path: "/tmp/ait-plan-boundary-test".to_string(),
        repo_name: "ait_test".to_string(),
        repository_index: Some(RepositoryIndex::new(7)),
        id_namespace_prefix: None,
        created_by: None,
        target: "docs/sprints/boundary.md".to_string(),
        plan_ref: None,
        prune: false,
        local: false,
        remote_name: Some("origin".to_string()),
        remote_repo_name: Some("ait_test".to_string()),
        base_url: Some("http://example.test".to_string()),
        rebase,
        reconcile: false,
        history_publish_plan_id: None,
        plan_storage: PlanSyncStorageRequest::default(),
        task_start: None,
    }
}

fn boundary_artifact() -> SyncArtifact {
    let artifact_body = "# Local Boundary\n".to_string();
    SyncArtifact {
        artifact_path: "docs/sprints/boundary.md".to_string(),
        artifact_selector: None,
        artifact_heading: "Local Boundary".to_string(),
        artifact_blob_id: artifact_blob_id(&artifact_body),
        artifact_body,
        items: Vec::new(),
    }
}

fn duplicate_current_plan(
    plan_id: &str,
    updated_at: JsonValue,
    published_plan_id: &str,
    items: JsonValue,
) -> JsonValue {
    json!({
        "plan_id": plan_id,
        "title": "Local Boundary",
        "status": "draft",
        "updated_at": updated_at,
        "publication_state": "published",
        "published_plan_id": published_plan_id,
        "published_head_revision_id": format!("R-{plan_id}"),
        "head_revision_id": format!("L-{plan_id}"),
        "head_revision": {
            "plan_revision_id": format!("L-{plan_id}"),
            "artifact_path": "docs/sprints/boundary.md",
            "artifact_selector": null,
            "artifact_heading": "Local Boundary",
            "artifact_blob_id": "BLB-boundary",
            "items": items,
            "publication_state": "published",
        }
    })
}

fn duplicate_current_inventory(plans: Vec<JsonValue>) -> LocalInventory {
    LocalInventory {
        indexed_plans: index_plans_by_path(&plans),
        indexed_by_identity: index_plans_by_identity(&plans),
        plans,
    }
}

fn boundary_local_plan(artifact: &SyncArtifact) -> JsonValue {
    json!({
        "plan_id": "PR-LOCAL-BOUNDARY",
        "title": "Local Boundary",
        "status": "draft",
        "publication_state": "local_draft",
        "head_revision": {
            "plan_revision_id": "PR-LOCAL-HEAD",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
        }
    })
}

fn boundary_remote_plan(blob_id: &str) -> JsonValue {
    json!({
        "plan_id": "PR-REMOTE-BOUNDARY",
        "title": "Remote Boundary",
        "status": "draft",
        "head_revision_id": "RPR-REMOTE-HEAD",
        "head_revision": {
            "plan_revision_id": "RPR-REMOTE-HEAD",
            "artifact_path": "docs/sprints/boundary.md",
            "artifact_selector": null,
            "artifact_heading": "Remote Boundary",
            "artifact_blob_id": blob_id,
            "items": [],
        }
    })
}

fn boundary_inventories(
    local_plan: JsonValue,
    remote_plan: JsonValue,
) -> (LocalInventory, RemoteInventory) {
    (
        LocalInventory {
            plans: vec![local_plan.clone()],
            indexed_plans: index_plans_by_path(std::slice::from_ref(&local_plan)),
            indexed_by_identity: index_plans_by_identity(std::slice::from_ref(&local_plan)),
        },
        RemoteInventory {
            plans: vec![remote_plan.clone()],
            indexed_plans: index_plans_by_path(std::slice::from_ref(&remote_plan)),
            indexed_by_identity: index_plans_by_identity(std::slice::from_ref(&remote_plan)),
            scoped_artifact_path: Some("docs/sprints/boundary.md".to_string()),
            full_loaded: false,
        },
    )
}

fn mixed_collision_local_fixture(
    artifact: &SyncArtifact,
) -> (JsonValue, Vec<JsonValue>, Vec<JsonValue>) {
    let first_bound_blob_id = artifact_blob_id("# Bound One\n");
    let second_bound_blob_id = artifact_blob_id("# Bound Two\n");
    let local_plan = json!({
        "plan_id": "PR-COLLISION",
        "title": "Local Boundary",
        "status": "draft",
        "publication_state": "published",
        "published_plan_id": "PR-BOUND-PUBLISHED",
        "published_head_revision_id": "RPR-BOUND-2",
        "head_revision_id": "LPR-DRAFT",
        "head_revision": {
            "plan_revision_id": "LPR-DRAFT",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
            "publication_state": "local_draft",
        }
    });
    let local_revisions = vec![
        json!({
            "plan_id": "PR-COLLISION",
            "plan_revision_id": "LPR-BOUND-1",
            "revision_number": 1,
            "artifact_path": "docs/sprints/bound.md",
            "artifact_selector": "bound/root",
            "artifact_heading": "Bound One",
            "title_snapshot": "Bound One",
            "artifact_blob_id": first_bound_blob_id.clone(),
            "items": [],
            "publication_state": "published",
            "published_plan_revision_id": "RPR-BOUND-1",
        }),
        json!({
            "plan_id": "PR-COLLISION",
            "plan_revision_id": "LPR-BOUND-2",
            "revision_number": 2,
            "artifact_path": "docs/sprints/bound.md",
            "artifact_selector": "bound/root",
            "artifact_heading": "Bound Two",
            "title_snapshot": "Bound Two",
            "artifact_blob_id": second_bound_blob_id.clone(),
            "items": [],
            "publication_state": "published",
            "published_plan_revision_id": "RPR-BOUND-2",
        }),
        json!({
            "plan_id": "PR-COLLISION",
            "plan_revision_id": "LPR-DRAFT",
            "revision_number": 3,
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "title_snapshot": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
            "publication_state": "local_draft",
        }),
    ];
    let bound_remote_revisions = vec![
        json!({
            "plan_id": "PR-BOUND-PUBLISHED",
            "plan_revision_id": "RPR-BOUND-1",
            "revision_number": 7,
            "artifact_path": "docs/sprints/bound.md",
            "artifact_selector": "bound/root",
            "artifact_heading": "Bound One",
            "title_snapshot": "Bound One",
            "artifact_blob_id": first_bound_blob_id,
            "items": [],
        }),
        json!({
            "plan_id": "PR-BOUND-PUBLISHED",
            "plan_revision_id": "RPR-BOUND-2",
            "revision_number": 8,
            "artifact_path": "docs/sprints/bound.md",
            "artifact_selector": "bound/root",
            "artifact_heading": "Bound Two",
            "title_snapshot": "Bound Two",
            "artifact_blob_id": second_bound_blob_id,
            "items": [],
        }),
    ];
    (local_plan, local_revisions, bound_remote_revisions)
}

fn same_ordinal_remote_plan(blob_id: &str) -> JsonValue {
    json!({
        "plan_id": "PR-COLLISION",
        "title": "Canonical Boundary",
        "status": "draft",
        "head_revision_id": "RPR-CANONICAL-HEAD",
        "head_revision": {
            "plan_revision_id": "RPR-CANONICAL-HEAD",
            "artifact_path": "docs/sprints/boundary.md",
            "artifact_selector": null,
            "artifact_heading": "Canonical Boundary",
            "artifact_blob_id": blob_id,
            "items": [],
        }
    })
}

fn exact_replacement_fixture(
    artifact: &SyncArtifact,
) -> (JsonValue, Vec<JsonValue>, JsonValue, Vec<JsonValue>) {
    let first_blob_id = artifact_blob_id("# Local Boundary One\n");
    let second_blob_id = artifact_blob_id("# Local Boundary Two\n");
    let local_plan = json!({
        "plan_id": "PR-COLLISION",
        "title": "Local Boundary",
        "status": "draft",
        "publication_state": "published",
        "published_plan_id": "PR-BOUND-PUBLISHED",
        "published_head_revision_id": "RPR-BOUND-2",
        "head_revision_id": "LPR-DRAFT",
        "head_revision": {
            "plan_revision_id": "LPR-DRAFT",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
            "publication_state": "local_draft",
        }
    });
    let local_revisions = vec![
        json!({
            "plan_id": "PR-COLLISION",
            "plan_revision_id": "LPR-BOUND-1",
            "revision_number": 1,
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "title_snapshot": artifact.artifact_heading,
            "artifact_blob_id": first_blob_id,
            "items": [],
            "publication_state": "published",
            "published_plan_revision_id": "RPR-BOUND-1",
        }),
        json!({
            "plan_id": "PR-COLLISION",
            "plan_revision_id": "LPR-BOUND-2",
            "revision_number": 2,
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "title_snapshot": artifact.artifact_heading,
            "artifact_blob_id": second_blob_id,
            "items": [],
            "publication_state": "published",
            "published_plan_revision_id": "RPR-BOUND-2",
        }),
        json!({
            "plan_id": "PR-COLLISION",
            "plan_revision_id": "LPR-DRAFT",
            "revision_number": 3,
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "title_snapshot": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
            "publication_state": "local_draft",
        }),
    ];
    let remote_plan = json!({
        "plan_id": "PR-EXACT-REPLACEMENT",
        "title": "Local Boundary",
        "status": "draft",
        "head_revision_id": "RPR-REPLACEMENT-3",
        "head_revision": {
            "plan_revision_id": "RPR-REPLACEMENT-3",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
        }
    });
    let remote_revisions = local_revisions
        .iter()
        .enumerate()
        .map(|(index, local_revision)| {
            let mut remote_revision = local_revision.clone();
            let remote_revision_id = format!("RPR-REPLACEMENT-{}", index + 1);
            let object = remote_revision
                .as_object_mut()
                .expect("replacement revision object");
            object.insert(
                "plan_id".to_string(),
                JsonValue::String("PR-EXACT-REPLACEMENT".to_string()),
            );
            object.insert(
                "plan_revision_id".to_string(),
                JsonValue::String(remote_revision_id),
            );
            object.remove("publication_state");
            object.remove("published_plan_revision_id");
            remote_revision
        })
        .collect::<Vec<_>>();
    (local_plan, local_revisions, remote_plan, remote_revisions)
}

fn invalid_same_target_receipt_fixture(
    artifact: &SyncArtifact,
) -> (
    JsonValue,
    Vec<JsonValue>,
    JsonValue,
    Vec<JsonValue>,
    JsonValue,
) {
    let old_blob_id = artifact_blob_id("# Old Boundary\n");
    let local_plan = json!({
        "plan_id": "PR-SAME-TARGET",
        "title": artifact.artifact_heading,
        "status": "draft",
        "publication_state": "published",
        "published_plan_id": "PR-SAME-TARGET",
        "published_head_revision_id": "RPR-SAME-HEAD",
        "head_revision_id": "LPR-SAME-HEAD",
        "head_revision": {
            "plan_revision_id": "LPR-SAME-HEAD",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
            "publication_state": "published",
            "published_plan_revision_id": "RPR-SAME-HEAD",
        },
    });
    let local_revisions = vec![
        json!({
            "plan_id": "PR-SAME-TARGET",
            "plan_revision_id": "LPR-SAME-OLD",
            "revision_number": 1,
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "title_snapshot": artifact.artifact_heading,
            "artifact_blob_id": old_blob_id.clone(),
            "items": [],
            "publication_state": "published",
            "published_plan_revision_id": "RPR-MISSING",
        }),
        json!({
            "plan_id": "PR-SAME-TARGET",
            "plan_revision_id": "LPR-SAME-HEAD",
            "revision_number": 2,
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "title_snapshot": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
            "publication_state": "published",
            "published_plan_revision_id": "RPR-SAME-HEAD",
        }),
    ];
    let remote_plan = json!({
        "plan_id": "PR-SAME-TARGET",
        "title": artifact.artifact_heading,
        "status": "draft",
        "head_revision_id": "RPR-SAME-HEAD",
        "head_revision": {
            "plan_revision_id": "RPR-SAME-HEAD",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
        },
    });
    let remote_revisions = vec![
        json!({
            "plan_id": "PR-SAME-TARGET",
            "plan_revision_id": "RPR-SAME-OLD",
            "revision_number": 1,
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "title_snapshot": artifact.artifact_heading,
            "artifact_blob_id": old_blob_id,
            "items": [],
        }),
        json!({
            "plan_id": "PR-SAME-TARGET",
            "plan_revision_id": "RPR-SAME-HEAD",
            "revision_number": 2,
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "title_snapshot": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
        }),
    ];
    let remote_head = json!({
        "plan_id": "PR-SAME-TARGET",
        "plan_revision_id": "RPR-SAME-HEAD",
        "revision_number": 2,
        "artifact_path": artifact.artifact_path,
        "artifact_selector": artifact.artifact_selector,
        "artifact_heading": artifact.artifact_heading,
        "title_snapshot": artifact.artifact_heading,
        "artifact_blob_id": artifact.artifact_blob_id,
        "artifact_body": artifact.artifact_body,
        "items": [],
    });
    (
        local_plan,
        local_revisions,
        remote_plan,
        remote_revisions,
        remote_head,
    )
}

#[test]
fn duplicate_current_plan_normal_sync_remains_fail_closed() {
    let request = boundary_request(false);
    let artifact = boundary_artifact();
    let plans = vec![
        duplicate_current_plan("PR-OLD", json!("10"), "PR-REMOTE", json!([])),
        duplicate_current_plan("PR-NEW", json!("20"), "PR-REMOTE", json!([])),
    ];
    let mut inventory = duplicate_current_inventory(plans);
    let local = BoundaryLocalStore::default();
    let identity = BoundaryIdentitySource::default();

    let error = select_or_reconcile_local_sync_plan_candidate(
        &local,
        &identity,
        &request,
        &artifact,
        &mut inventory,
    )
    .expect_err("normal sync must reject duplicate current Plans");

    assert!(error.contains("Multiple current plans already track"));
    assert!(local.calls.borrow().is_empty());
    assert_eq!(identity.timestamps.get(), 0);
}

#[test]
fn duplicate_current_plan_explicit_reconcile_archives_older_itemless_lineage() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let plans = vec![
        duplicate_current_plan(
            "PR-OLD",
            json!("2026-07-30T00:00:00+00:00"),
            "PR-REMOTE",
            json!([]),
        ),
        duplicate_current_plan(
            "PR-NEW",
            json!("2026-07-31T00:00:00+00:00"),
            "PR-REMOTE",
            json!([]),
        ),
    ];
    let mut inventory = duplicate_current_inventory(plans);
    let local = BoundaryLocalStore::default();
    let identity = BoundaryIdentitySource::default();

    let (selected, continuity) = select_or_reconcile_local_sync_plan_candidate(
        &local,
        &identity,
        &request,
        &artifact,
        &mut inventory,
    )
    .expect("explicit reconciliation should retain the unique newest Plan");

    assert_eq!(selected.expect("retained Plan")["plan_id"], json!("PR-NEW"));
    let continuity = continuity.expect("reconciliation continuity");
    assert_eq!(
        continuity["match_kind"],
        json!("duplicate_current_reconcile")
    );
    assert_eq!(continuity["retained_plan_id"], json!("PR-NEW"));
    assert_eq!(continuity["archived_plan_ids"], json!(["PR-OLD"]));
    assert_eq!(continuity["published_plan_id"], json!("PR-REMOTE"));
    assert_eq!(local.calls.borrow().as_slice(), ["close:PR-OLD:archived"]);
    assert_eq!(identity.timestamps.get(), 1);
    assert!(inventory.plans.iter().any(|plan| {
        text_field(plan, "plan_id").as_deref() == Some("PR-OLD")
            && text_field(plan, "status").as_deref() == Some("archived")
    }));
}

#[test]
fn duplicate_current_plan_reconcile_rejects_plan_items_before_mutation() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let plans = vec![
        duplicate_current_plan(
            "PR-OLD",
            json!("10"),
            "PR-REMOTE",
            json!([{"item_id": "PI-1"}]),
        ),
        duplicate_current_plan("PR-NEW", json!("20"), "PR-REMOTE", json!([])),
    ];
    let mut inventory = duplicate_current_inventory(plans);
    let local = BoundaryLocalStore::default();
    let identity = BoundaryIdentitySource::default();

    let error = select_or_reconcile_local_sync_plan_candidate(
        &local,
        &identity,
        &request,
        &artifact,
        &mut inventory,
    )
    .expect_err("item-bearing duplicate Plans must fail closed");

    assert!(error.contains("has Plan items"));
    assert!(local.calls.borrow().is_empty());
    assert_eq!(identity.timestamps.get(), 0);
}

#[test]
fn exact_history_publication_selects_the_archived_bound_plan_without_markdown_access() {
    let store = BoundaryLocalStore::default();
    store.existing_plans.borrow_mut().insert(
        "PR-649".to_string(),
        json!({
            "plan_id": "PR-649",
            "repo_name": "ait_test",
            "status": "archived",
            "publication_state": "local_draft",
            "head_revision_id": "plan-revision:2887",
            "head_revision": {
                "plan_revision_id": "plan-revision:2887",
                "artifact_path": "docs/sprints/missing.md",
                "artifact_selector": "missing/root",
                "artifact_blob_id": "BLB-exact"
            }
        }),
    );
    let mut request = boundary_request(false);
    request.history_publish_plan_id = Some("PR-649".to_string());

    let row = history_publish_result_row(&request, &store, "PR-649")
        .expect("archived Plan selection should use durable local authority");

    assert_eq!(row["action"], json!("history_publish"));
    assert_eq!(row["plan_id"], json!("PR-649"));
    assert_eq!(row["artifact_path"], json!("docs/sprints/missing.md"));
    assert_eq!(row["artifact_selector"], json!("missing/root"));
}

#[test]
fn duplicate_current_plan_reconcile_rejects_remote_target_mismatch_before_mutation() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let plans = vec![
        duplicate_current_plan("PR-OLD", json!("10"), "PR-REMOTE-A", json!([])),
        duplicate_current_plan("PR-NEW", json!("20"), "PR-REMOTE-B", json!([])),
    ];
    let mut inventory = duplicate_current_inventory(plans);
    let local = BoundaryLocalStore::default();
    let identity = BoundaryIdentitySource::default();

    let error = select_or_reconcile_local_sync_plan_candidate(
        &local,
        &identity,
        &request,
        &artifact,
        &mut inventory,
    )
    .expect_err("different published targets must fail closed");

    assert!(error.contains("do not share one published remote Plan target"));
    assert!(local.calls.borrow().is_empty());
    assert_eq!(identity.timestamps.get(), 0);
}

#[test]
fn duplicate_current_plan_reconcile_rejects_missing_or_malformed_updated_at() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    for invalid_updated_at in [
        JsonValue::Null,
        json!(""),
        json!(" 20"),
        json!("01"),
        json!("not-a-u64"),
        json!("18446744073709551616"),
        json!("2026-07-31T00:00:00.001+00:00"),
        json!("2026-07-31T01:00:00+01:00"),
        json!("1969-12-31T23:59:59+00:00"),
    ] {
        let plans = vec![
            duplicate_current_plan("PR-OLD", invalid_updated_at.clone(), "PR-REMOTE", json!([])),
            duplicate_current_plan("PR-NEW", json!("20"), "PR-REMOTE", json!([])),
        ];
        let mut inventory = duplicate_current_inventory(plans);
        let local = BoundaryLocalStore::default();
        let identity = BoundaryIdentitySource::default();

        let error = select_or_reconcile_local_sync_plan_candidate(
            &local,
            &identity,
            &request,
            &artifact,
            &mut inventory,
        )
        .expect_err("invalid updated_at must fail closed");

        assert!(error.contains("updated_at ordering authority"));
        assert!(local.calls.borrow().is_empty());
        assert_eq!(identity.timestamps.get(), 0);
    }
}

#[test]
fn duplicate_current_plan_reconcile_rejects_tied_latest_timestamp() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let plans = vec![
        duplicate_current_plan("PR-A", json!("20"), "PR-REMOTE", json!([])),
        duplicate_current_plan("PR-B", json!("20"), "PR-REMOTE", json!([])),
    ];
    let mut inventory = duplicate_current_inventory(plans);
    let local = BoundaryLocalStore::default();
    let identity = BoundaryIdentitySource::default();

    let error = select_or_reconcile_local_sync_plan_candidate(
        &local,
        &identity,
        &request,
        &artifact,
        &mut inventory,
    )
    .expect_err("tied newest Plans must fail closed");

    assert!(error.contains("tied greatest updated_at 20"));
    assert!(local.calls.borrow().is_empty());
    assert_eq!(identity.timestamps.get(), 0);
}

#[test]
fn explicit_rebase_adopts_verified_head_without_reading_unavailable_history() {
    let request = boundary_request(true);
    let artifact = boundary_artifact();
    let remote_body = "# Verified Remote Boundary\n";
    let remote_blob_id = artifact_blob_id(remote_body);
    let remote_plan = boundary_remote_plan(&remote_blob_id);
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(boundary_local_plan(&artifact), remote_plan);
    let local = BoundaryLocalStore::default();
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            (
                "PR-REMOTE-BOUNDARY".to_string(),
                "RPR-REMOTE-HEAD".to_string(),
            ),
            json!({
                "plan_id": "PR-REMOTE-BOUNDARY",
                "plan_revision_id": "RPR-REMOTE-HEAD",
                "artifact_path": "docs/sprints/boundary.md",
                "artifact_selector": null,
                "artifact_heading": "Remote Boundary",
                "artifact_blob_id": remote_blob_id,
                "artifact_body": remote_body,
                "items": [],
            }),
        )]),
        histories: BTreeMap::new(),
        calls: Vec::new(),
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let (selected, adoption, _) = resolve_local_sync_plan_candidate(
        &local,
        &blobs,
        &identity,
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect("explicit rebase should adopt the verified remote head boundary");

    assert_eq!(selected.unwrap()["plan_id"], "PR-REMOTE-BOUNDARY");
    assert_eq!(
        adoption.unwrap()["replaced_local_plan_id"],
        "PR-LOCAL-BOUNDARY"
    );
    assert!(history_cache.is_empty());
    assert_eq!(
        remote.calls,
        vec!["head:PR-REMOTE-BOUNDARY:RPR-REMOTE-HEAD"]
    );
    assert_eq!(blobs.calls.borrow().len(), 1);
    assert_eq!(
        local.calls.borrow().as_slice(),
        [
            "close:PR-LOCAL-BOUNDARY:archived",
            "create:PR-REMOTE-BOUNDARY:PR-BOUNDARY-1",
            "publish:PR-REMOTE-BOUNDARY:RPR-REMOTE-HEAD",
        ]
    );
}

#[test]
fn explicit_rebase_splits_published_stale_identity_at_a_distinct_local_plan() {
    let request = boundary_request(true);
    let artifact = boundary_artifact();
    let remote_body = "# Canonical Remote Boundary\n";
    let remote_blob_id = artifact_blob_id(remote_body);
    let remote_plan = boundary_remote_plan(&remote_blob_id);
    let (local_plan, _, _) = mixed_collision_local_fixture(&artifact);
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(local_plan.clone(), remote_plan);
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([
            ("PR-COLLISION".to_string(), local_plan),
            (
                "PR-REMOTE-BOUNDARY".to_string(),
                json!({
                    "plan_id": "PR-REMOTE-BOUNDARY",
                    "status": "archived",
                    "head_revision": {
                        "artifact_path": "docs/sprints/unrelated.md",
                        "artifact_selector": "unrelated/root",
                    }
                }),
            ),
        ])),
        next_created_plan_id: RefCell::new(Some("PR-LOCAL-DISTINCT".to_string())),
        next_created_revision_id: RefCell::new(Some("LPR-CANONICAL-HEAD".to_string())),
        preserve_remote_identity: true,
        ..BoundaryLocalStore::default()
    };
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            (
                "PR-REMOTE-BOUNDARY".to_string(),
                "RPR-REMOTE-HEAD".to_string(),
            ),
            json!({
                "plan_id": "PR-REMOTE-BOUNDARY",
                "plan_revision_id": "RPR-REMOTE-HEAD",
                "artifact_path": "docs/sprints/boundary.md",
                "artifact_selector": null,
                "artifact_heading": "Canonical Remote Boundary",
                "artifact_blob_id": remote_blob_id,
                "artifact_body": remote_body,
                "items": [],
            }),
        )]),
        histories: BTreeMap::new(),
        calls: Vec::new(),
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let (selected, adoption, _) = resolve_local_sync_plan_candidate(
        &local,
        &blobs,
        &identity,
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect("explicit rebase should split the stale published identity");

    let selected = selected.expect("adopted local Plan");
    assert_eq!(selected["plan_id"], "PR-LOCAL-DISTINCT");
    assert_eq!(selected["published_plan_id"], "PR-REMOTE-BOUNDARY");
    let adoption = adoption.expect("published identity rebase adoption row");
    assert_eq!(adoption["plan_id"], "PR-LOCAL-DISTINCT");
    assert_eq!(adoption["replaced_local_plan_id"], "PR-COLLISION");
    assert!(history_cache.is_empty());
    assert_eq!(remote.calls, ["head:PR-REMOTE-BOUNDARY:RPR-REMOTE-HEAD"]);
    assert_eq!(blobs.calls.borrow().len(), 1);
    assert_eq!(
        local.calls.borrow().as_slice(),
        [
            "close:PR-COLLISION:archived",
            "create:PR-REMOTE-BOUNDARY:PR-BOUNDARY-1",
            "publish:PR-LOCAL-DISTINCT:RPR-REMOTE-HEAD",
        ]
    );
    assert!(local_inventory.plans.iter().any(|plan| {
        text_field(plan, "plan_id").as_deref() == Some("PR-COLLISION")
            && text_field(plan, "status").as_deref() == Some("archived")
    }));
    assert!(local_inventory.plans.iter().any(|plan| {
        text_field(plan, "plan_id").as_deref() == Some("PR-LOCAL-DISTINCT")
            && text_field(plan, "published_plan_id").as_deref() == Some("PR-REMOTE-BOUNDARY")
    }));
}

#[test]
fn explicit_published_rebase_rejects_head_selector_mismatch_before_local_mutation() {
    let request = boundary_request(true);
    let artifact = boundary_artifact();
    let remote_body = "# Canonical Remote Boundary\n";
    let remote_blob_id = artifact_blob_id(remote_body);
    let remote_plan = boundary_remote_plan(&remote_blob_id);
    let (local_plan, _, _) = mixed_collision_local_fixture(&artifact);
    let (mut local_inventory, mut remote_inventory) = boundary_inventories(local_plan, remote_plan);
    let local = BoundaryLocalStore {
        next_created_plan_id: RefCell::new(Some("PR-LOCAL-DISTINCT".to_string())),
        preserve_remote_identity: true,
        ..BoundaryLocalStore::default()
    };
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            (
                "PR-REMOTE-BOUNDARY".to_string(),
                "RPR-REMOTE-HEAD".to_string(),
            ),
            json!({
                "plan_id": "PR-REMOTE-BOUNDARY",
                "plan_revision_id": "RPR-REMOTE-HEAD",
                "artifact_path": "docs/sprints/boundary.md",
                "artifact_selector": "different/root",
                "artifact_heading": "Canonical Remote Boundary",
                "artifact_blob_id": remote_blob_id,
                "artifact_body": remote_body,
                "items": [],
            }),
        )]),
        histories: BTreeMap::new(),
        calls: Vec::new(),
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let error = resolve_local_sync_plan_candidate(
        &local,
        &blobs,
        &identity,
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect_err("a selector mismatch must fail before archiving the local Plan");

    assert!(error.contains("does not match inventory selector"));
    assert!(local.calls.borrow().is_empty());
    assert!(blobs.calls.borrow().is_empty());
    assert_eq!(identity.workflow_ids.get(), 0);
    assert_eq!(identity.timestamps.get(), 0);
    assert_eq!(local_inventory.plans[0]["status"], "draft");
}

#[test]
fn explicit_rebase_rejects_head_hash_mismatch_before_local_mutation() {
    let request = boundary_request(true);
    let artifact = boundary_artifact();
    let remote_plan = boundary_remote_plan("BLB-declared-wrong");
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(boundary_local_plan(&artifact), remote_plan);
    let local = BoundaryLocalStore::default();
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            (
                "PR-REMOTE-BOUNDARY".to_string(),
                "RPR-REMOTE-HEAD".to_string(),
            ),
            json!({
                "plan_id": "PR-REMOTE-BOUNDARY",
                "plan_revision_id": "RPR-REMOTE-HEAD",
                "artifact_path": "docs/sprints/boundary.md",
                "artifact_heading": "Remote Boundary",
                "artifact_blob_id": "BLB-declared-wrong",
                "artifact_body": "# Different Bytes\n",
                "items": [],
            }),
        )]),
        histories: BTreeMap::new(),
        calls: Vec::new(),
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let error = resolve_local_sync_plan_candidate(
        &local,
        &blobs,
        &identity,
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect_err("a mismatched head hash must fail before archiving the local Plan");

    assert!(error.contains("does not match its declared artifact_blob_id"));
    assert!(local.calls.borrow().is_empty());
    assert!(blobs.calls.borrow().is_empty());
    assert_eq!(identity.workflow_ids.get(), 0);
    assert_eq!(identity.timestamps.get(), 0);
    assert_eq!(local_inventory.plans[0]["status"], "draft");
}

#[test]
fn normal_sync_does_not_enable_head_boundary_adoption() {
    let request = boundary_request(false);
    let artifact = boundary_artifact();
    let remote_plan = boundary_remote_plan("BLB-remote-different");
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(boundary_local_plan(&artifact), remote_plan);
    let local = BoundaryLocalStore::default();
    let mut remote = BoundaryRemote::default();
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let error = resolve_local_sync_plan_candidate(
        &local,
        &BoundaryBlobStore::default(),
        &BoundaryIdentitySource::default(),
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect_err("normal sync must keep duplicate Plan rejection fail closed");

    assert!(error.contains("would publish a duplicate"));
    assert!(local.calls.borrow().is_empty());
    assert!(remote.calls.is_empty());
}

#[test]
fn explicit_reconcile_splits_verified_same_ordinal_receipt_collision() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let canonical_remote_body = "# Canonical Remote Boundary\n";
    let canonical_remote_blob_id = artifact_blob_id(canonical_remote_body);
    let remote_plan = same_ordinal_remote_plan(&canonical_remote_blob_id);
    let (local_plan, local_revisions, bound_remote_revisions) =
        mixed_collision_local_fixture(&artifact);
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(local_plan.clone(), remote_plan);
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([("PR-COLLISION".to_string(), local_plan)])),
        revisions: RefCell::new(BTreeMap::from([(
            "PR-COLLISION".to_string(),
            local_revisions,
        )])),
        next_created_plan_id: RefCell::new(Some("PR-LOCAL-DISTINCT".to_string())),
        next_created_revision_id: RefCell::new(Some("LPR-CANONICAL-HEAD".to_string())),
        ..BoundaryLocalStore::default()
    };
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            ("PR-COLLISION".to_string(), "RPR-CANONICAL-HEAD".to_string()),
            json!({
                "plan_id": "PR-COLLISION",
                "plan_revision_id": "RPR-CANONICAL-HEAD",
                "artifact_path": "docs/sprints/boundary.md",
                "artifact_selector": null,
                "artifact_heading": "Canonical Boundary",
                "artifact_blob_id": canonical_remote_blob_id,
                "artifact_body": canonical_remote_body,
                "items": [],
            }),
        )]),
        histories: BTreeMap::from([("PR-BOUND-PUBLISHED".to_string(), bound_remote_revisions)]),
        calls: Vec::new(),
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let (selected, adoption, _) = resolve_local_sync_plan_candidate(
        &local,
        &blobs,
        &identity,
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect("explicit reconcile should split an exact mixed local lineage");

    let selected = selected.expect("adopted local Plan");
    assert_eq!(selected["plan_id"], "PR-LOCAL-DISTINCT");
    assert_eq!(selected["published_plan_id"], "PR-COLLISION");
    let adoption = adoption.expect("lineage-split adoption row");
    assert_eq!(adoption["plan_id"], "PR-LOCAL-DISTINCT");
    assert_eq!(adoption["replaced_local_plan_id"], "PR-COLLISION");
    assert_eq!(
        remote.calls,
        [
            "history:PR-BOUND-PUBLISHED",
            "head:PR-COLLISION:RPR-CANONICAL-HEAD",
        ]
    );
    assert_eq!(blobs.calls.borrow().len(), 1);
    assert_eq!(
        local.calls.borrow().as_slice(),
        [
            "close:PR-COLLISION:archived",
            "create:PR-COLLISION:PR-BOUNDARY-1",
            "publish:PR-LOCAL-DISTINCT:RPR-CANONICAL-HEAD",
        ]
    );
    assert!(local_inventory.plans.iter().any(|plan| {
        text_field(plan, "plan_id").as_deref() == Some("PR-COLLISION")
            && text_field(plan, "status").as_deref() == Some("archived")
    }));
    assert!(local_inventory.plans.iter().any(|plan| {
        text_field(plan, "plan_id").as_deref() == Some("PR-LOCAL-DISTINCT")
            && text_field(plan, "published_plan_id").as_deref() == Some("PR-COLLISION")
    }));
}

#[test]
fn normal_sync_keeps_same_ordinal_receipt_collision_fail_closed() {
    let request = boundary_request(false);
    let artifact = boundary_artifact();
    let remote_plan = same_ordinal_remote_plan("BLB-canonical-remote");
    let (local_plan, _, _) = mixed_collision_local_fixture(&artifact);
    let (mut local_inventory, mut remote_inventory) = boundary_inventories(local_plan, remote_plan);
    let local = BoundaryLocalStore::default();
    let mut remote = BoundaryRemote::default();
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let error = resolve_local_sync_plan_candidate(
        &local,
        &BoundaryBlobStore::default(),
        &BoundaryIdentitySource::default(),
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect_err("normal sync must not split an occupied local Plan lineage");

    assert!(error.contains("local plan PR-COLLISION is bound to remote PR-BOUND-PUBLISHED"));
    assert!(local.calls.borrow().is_empty());
    assert!(remote.calls.is_empty());
}

#[test]
fn normal_sync_rejects_published_stale_receipt_even_with_exact_remote_history() {
    let request = boundary_request(false);
    let artifact = boundary_artifact();
    let (local_plan, local_revisions, remote_plan, replacement_revisions) =
        exact_replacement_fixture(&artifact);
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(local_plan.clone(), remote_plan);
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([("PR-COLLISION".to_string(), local_plan)])),
        revisions: RefCell::new(BTreeMap::from([(
            "PR-COLLISION".to_string(),
            local_revisions,
        )])),
        preserve_remote_identity: true,
        ..BoundaryLocalStore::default()
    };
    let mut remote = BoundaryRemote {
        histories: BTreeMap::from([("PR-EXACT-REPLACEMENT".to_string(), replacement_revisions)]),
        ..BoundaryRemote::default()
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let error = resolve_local_sync_plan_candidate(
        &local,
        &BoundaryBlobStore::default(),
        &BoundaryIdentitySource::default(),
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect_err("normal sync must reject a published receipt rebind");

    assert!(error.contains("would publish a duplicate"));
    assert!(local.calls.borrow().is_empty());
    assert!(remote.calls.is_empty());
}

#[test]
fn explicit_reconcile_rebinds_published_stale_receipt_to_exact_remote_history() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let (local_plan, local_revisions, remote_plan, replacement_revisions) =
        exact_replacement_fixture(&artifact);
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(local_plan.clone(), remote_plan);
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([("PR-COLLISION".to_string(), local_plan)])),
        revisions: RefCell::new(BTreeMap::from([(
            "PR-COLLISION".to_string(),
            local_revisions,
        )])),
        preserve_remote_identity: true,
        ..BoundaryLocalStore::default()
    };
    let mut remote = BoundaryRemote {
        histories: BTreeMap::from([("PR-EXACT-REPLACEMENT".to_string(), replacement_revisions)]),
        ..BoundaryRemote::default()
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let (selected, adoption, _) = resolve_local_sync_plan_candidate(
        &local,
        &BoundaryBlobStore::default(),
        &BoundaryIdentitySource::default(),
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect("explicit reconcile should rebind an exact replacement history");

    let selected = selected.expect("rebound local Plan");
    assert_eq!(selected["plan_id"], "PR-COLLISION");
    assert_eq!(selected["published_plan_id"], "PR-EXACT-REPLACEMENT");
    assert_eq!(selected["published_head_revision_id"], "RPR-REPLACEMENT-3");
    assert_eq!(
        adoption.expect("rebind adoption row")["previous_local_plan_id"],
        JsonValue::Null
    );
    assert_eq!(
        local.calls.borrow().as_slice(),
        ["publish:PR-COLLISION:RPR-REPLACEMENT-3"]
    );
    assert_eq!(remote.calls, ["history:PR-EXACT-REPLACEMENT"]);
}

#[test]
fn equivalent_same_ordinal_local_and_remote_plans_still_require_receipt_binding() {
    let request = boundary_request(false);
    let blob_id = artifact_blob_id("# Same Ordinal\n");
    let local_plan = json!({
        "plan_id": "PR-COLLISION",
        "publication_state": "local_draft",
        "head_revision_id": "LPR-SAME-1",
        "head_revision": {
            "plan_revision_id": "LPR-SAME-1",
            "artifact_path": "docs/same.md",
            "artifact_selector": null,
            "artifact_heading": "Same Ordinal",
            "artifact_blob_id": blob_id,
            "items": [],
        },
    });
    let local_revision = json!({
        "plan_id": "PR-COLLISION",
        "plan_revision_id": "LPR-SAME-1",
        "revision_number": 1,
        "artifact_path": "docs/same.md",
        "artifact_selector": null,
        "artifact_heading": "Same Ordinal",
        "title_snapshot": "Same Ordinal",
        "artifact_blob_id": blob_id,
        "items": [],
        "publication_state": "local_draft",
    });
    let remote_plan = json!({
        "plan_id": "PR-COLLISION",
        "head_revision_id": "RPR-SAME-1",
        "head_revision": {"plan_revision_id": "RPR-SAME-1"},
    });
    let remote_revision = json!({
        "plan_id": "PR-COLLISION",
        "plan_revision_id": "RPR-SAME-1",
        "revision_number": 1,
        "artifact_path": "docs/same.md",
        "artifact_selector": null,
        "artifact_heading": "Same Ordinal",
        "title_snapshot": "Same Ordinal",
        "artifact_blob_id": blob_id,
        "items": [],
    });
    let local = BoundaryLocalStore {
        revisions: RefCell::new(BTreeMap::from([(
            "PR-COLLISION".to_string(),
            vec![local_revision],
        )])),
        preserve_remote_identity: true,
        ..BoundaryLocalStore::default()
    };

    let rebound = bind_equivalent_local_plan_to_remote_identity(
        &local,
        &BoundaryIdentitySource::default(),
        &request,
        &local_plan,
        &remote_plan,
        &[remote_revision],
    )
    .expect("equal raw ordinals must not bypass receipt validation")
    .expect("equivalent authority-scoped Plans should bind");

    assert_eq!(rebound["plan_id"], "PR-COLLISION");
    assert_eq!(rebound["published_plan_id"], "PR-COLLISION");
    assert_eq!(
        local.calls.borrow().as_slice(),
        ["publish:PR-COLLISION:RPR-SAME-1"]
    );
}

#[test]
fn explicit_reconcile_inspects_and_rebinds_a_fully_published_stale_identity() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let local_plan = json!({
        "plan_id": "PR-FULLY-PUBLISHED",
        "title": artifact.artifact_heading,
        "status": "draft",
        "publication_state": "published",
        "published_plan_id": "PR-STALE",
        "published_head_revision_id": "RPR-STALE-1",
        "head_revision_id": "LPR-FULL-1",
        "head_revision": {
            "plan_revision_id": "LPR-FULL-1",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
            "publication_state": "published",
            "published_plan_revision_id": "RPR-STALE-1",
        },
    });
    let local_revision = json!({
        "plan_id": "PR-FULLY-PUBLISHED",
        "plan_revision_id": "LPR-FULL-1",
        "revision_number": 1,
        "artifact_path": artifact.artifact_path,
        "artifact_selector": artifact.artifact_selector,
        "artifact_heading": artifact.artifact_heading,
        "title_snapshot": artifact.artifact_heading,
        "artifact_blob_id": artifact.artifact_blob_id,
        "items": [],
        "publication_state": "published",
        "published_plan_revision_id": "RPR-STALE-1",
    });
    let remote_plan = json!({
        "plan_id": "PR-CANONICAL",
        "title": artifact.artifact_heading,
        "status": "draft",
        "head_revision_id": "RPR-CANONICAL-1",
        "head_revision": {
            "plan_revision_id": "RPR-CANONICAL-1",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
        },
    });
    let remote_revision = json!({
        "plan_id": "PR-CANONICAL",
        "plan_revision_id": "RPR-CANONICAL-1",
        "revision_number": 1,
        "artifact_path": artifact.artifact_path,
        "artifact_selector": artifact.artifact_selector,
        "artifact_heading": artifact.artifact_heading,
        "title_snapshot": artifact.artifact_heading,
        "artifact_blob_id": artifact.artifact_blob_id,
        "items": [],
    });
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(local_plan.clone(), remote_plan);
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([(
            "PR-FULLY-PUBLISHED".to_string(),
            local_plan,
        )])),
        revisions: RefCell::new(BTreeMap::from([(
            "PR-FULLY-PUBLISHED".to_string(),
            vec![local_revision],
        )])),
        preserve_remote_identity: true,
        ..BoundaryLocalStore::default()
    };
    let mut remote = BoundaryRemote {
        histories: BTreeMap::from([("PR-CANONICAL".to_string(), vec![remote_revision])]),
        ..BoundaryRemote::default()
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let (selected, adoption, _) = resolve_local_sync_plan_candidate(
        &local,
        &BoundaryBlobStore::default(),
        &BoundaryIdentitySource::default(),
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect("explicit reconcile should inspect a fully published stale receipt");

    assert_eq!(
        selected.expect("rebound local Plan")["published_plan_id"],
        "PR-CANONICAL"
    );
    assert_eq!(
        adoption.expect("rebind adoption row")["remote_plan_ref"],
        "RPR-CANONICAL"
    );
    assert_eq!(
        local.calls.borrow().as_slice(),
        ["publish:PR-FULLY-PUBLISHED:RPR-CANONICAL-1"]
    );
    assert_eq!(remote.calls, ["history:PR-CANONICAL"]);
}

#[test]
fn explicit_reconcile_resets_an_exact_head_after_a_mixed_draft_tail() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let (local_plan, mut local_revisions, _) = mixed_collision_local_fixture(&artifact);
    local_revisions[2]["revision_number"] = json!(4);
    local_revisions.insert(
        2,
        json!({
            "plan_id": "PR-COLLISION",
            "plan_revision_id": "LPR-OLD-DRAFT",
            "revision_number": 3,
            "artifact_path": "docs/sprints/bound.md",
            "artifact_selector": "bound/root",
            "artifact_heading": "Bound Draft",
            "title_snapshot": "Bound Draft",
            "artifact_blob_id": artifact_blob_id("# Bound Draft\n"),
            "items": [],
            "publication_state": "local_draft",
        }),
    );
    let remote_plan = json!({
        "plan_id": "PR-CANONICAL-CURRENT",
        "title": artifact.artifact_heading,
        "status": "draft",
        "head_revision_id": "RPR-CANONICAL-CURRENT",
        "head_revision": {
            "plan_revision_id": "RPR-CANONICAL-CURRENT",
            "artifact_path": artifact.artifact_path,
            "artifact_selector": artifact.artifact_selector,
            "artifact_heading": artifact.artifact_heading,
            "artifact_blob_id": artifact.artifact_blob_id,
            "items": [],
        },
    });
    let remote_head = json!({
        "plan_id": "PR-CANONICAL-CURRENT",
        "plan_revision_id": "RPR-CANONICAL-CURRENT",
        "revision_number": 1,
        "artifact_path": artifact.artifact_path,
        "artifact_selector": artifact.artifact_selector,
        "artifact_heading": artifact.artifact_heading,
        "title_snapshot": artifact.artifact_heading,
        "artifact_blob_id": artifact.artifact_blob_id,
        "artifact_body": artifact.artifact_body,
        "items": [],
    });
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(local_plan.clone(), remote_plan);
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([("PR-COLLISION".to_string(), local_plan)])),
        revisions: RefCell::new(BTreeMap::from([(
            "PR-COLLISION".to_string(),
            local_revisions,
        )])),
        next_created_plan_id: RefCell::new(Some("PR-LOCAL-RESET".to_string())),
        next_created_revision_id: RefCell::new(Some("LPR-RESET-HEAD".to_string())),
        ..BoundaryLocalStore::default()
    };
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            (
                "PR-CANONICAL-CURRENT".to_string(),
                "RPR-CANONICAL-CURRENT".to_string(),
            ),
            remote_head,
        )]),
        ..BoundaryRemote::default()
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let (selected, adoption, _) = resolve_local_sync_plan_candidate(
        &local,
        &blobs,
        &identity,
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect("an exact current head should supersede an ambiguous mixed draft tail");

    let selected = selected.expect("reset local Plan");
    assert_eq!(selected["plan_id"], "PR-LOCAL-RESET");
    assert_eq!(selected["published_plan_id"], "PR-CANONICAL-CURRENT");
    assert_eq!(
        adoption.expect("identity reset adoption")["replaced_local_plan_id"],
        "PR-COLLISION"
    );
    assert!(history_cache.is_empty());
    assert_eq!(
        remote.calls,
        ["head:PR-CANONICAL-CURRENT:RPR-CANONICAL-CURRENT"]
    );
    assert_eq!(
        local.calls.borrow().as_slice(),
        [
            "close:PR-COLLISION:archived",
            "create:PR-CANONICAL-CURRENT:PR-BOUNDARY-1",
            "publish:PR-LOCAL-RESET:RPR-CANONICAL-CURRENT",
        ]
    );
}

#[test]
fn explicit_reconcile_resets_an_exact_same_target_invalid_populated_receipt() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let (local_plan, local_revisions, remote_plan, remote_revisions, remote_head) =
        invalid_same_target_receipt_fixture(&artifact);
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(local_plan.clone(), remote_plan);
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([("PR-SAME-TARGET".to_string(), local_plan)])),
        revisions: RefCell::new(BTreeMap::from([(
            "PR-SAME-TARGET".to_string(),
            local_revisions,
        )])),
        next_created_plan_id: RefCell::new(Some("PR-LOCAL-RESET".to_string())),
        next_created_revision_id: RefCell::new(Some("LPR-RESET-HEAD".to_string())),
        ..BoundaryLocalStore::default()
    };
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            ("PR-SAME-TARGET".to_string(), "RPR-SAME-HEAD".to_string()),
            remote_head,
        )]),
        histories: BTreeMap::from([("PR-SAME-TARGET".to_string(), remote_revisions)]),
        calls: Vec::new(),
    };
    let mut history_cache = BTreeMap::new();
    let mut detail_cache = BTreeMap::new();

    let (selected, adoption, _) = resolve_local_sync_plan_candidate(
        &local,
        &blobs,
        &identity,
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut history_cache,
        &mut detail_cache,
    )
    .expect("an exact remote head should replace a same-target invalid receipt lineage");

    let selected = selected.expect("reset local Plan");
    assert_eq!(selected["plan_id"], "PR-LOCAL-RESET");
    assert_eq!(selected["published_plan_id"], "PR-SAME-TARGET");
    assert_eq!(
        adoption.expect("identity reset adoption")["replaced_local_plan_id"],
        "PR-SAME-TARGET"
    );
    assert_eq!(
        remote.calls,
        [
            "history:PR-SAME-TARGET",
            "head:PR-SAME-TARGET:RPR-SAME-HEAD",
        ]
    );
    assert_eq!(
        local.calls.borrow().as_slice(),
        [
            "close:PR-SAME-TARGET:archived",
            "create:PR-SAME-TARGET:PR-BOUNDARY-1",
            "publish:PR-LOCAL-RESET:RPR-SAME-HEAD",
        ]
    );
}

#[test]
fn normal_sync_does_not_reset_a_same_target_invalid_populated_receipt() {
    let request = boundary_request(false);
    let artifact = boundary_artifact();
    let (local_plan, local_revisions, remote_plan, remote_revisions, remote_head) =
        invalid_same_target_receipt_fixture(&artifact);
    let (mut local_inventory, mut remote_inventory) =
        boundary_inventories(local_plan.clone(), remote_plan);
    let local = BoundaryLocalStore {
        revisions: RefCell::new(BTreeMap::from([(
            "PR-SAME-TARGET".to_string(),
            local_revisions,
        )])),
        ..BoundaryLocalStore::default()
    };
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            ("PR-SAME-TARGET".to_string(), "RPR-SAME-HEAD".to_string()),
            remote_head,
        )]),
        histories: BTreeMap::from([("PR-SAME-TARGET".to_string(), remote_revisions)]),
        calls: Vec::new(),
    };

    let (selected, adoption, _) = resolve_local_sync_plan_candidate(
        &local,
        &BoundaryBlobStore::default(),
        &BoundaryIdentitySource::default(),
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .expect("normal sync should preserve the existing fail-closed candidate boundary");

    assert_eq!(
        selected.expect("existing local Plan")["plan_id"],
        "PR-SAME-TARGET"
    );
    assert!(adoption.is_none());
    assert!(local.calls.borrow().is_empty());
    assert!(remote.calls.is_empty());
}

#[test]
fn explicit_reconcile_rejects_invalid_receipt_when_remote_head_is_not_exact() {
    let mut request = boundary_request(false);
    request.reconcile = true;
    let artifact = boundary_artifact();
    let (local_plan, local_revisions, mut remote_plan, remote_revisions, remote_head) =
        invalid_same_target_receipt_fixture(&artifact);
    remote_plan["head_revision"]["artifact_blob_id"] =
        JsonValue::String("BLB-different-remote-head".to_string());
    let (mut local_inventory, mut remote_inventory) = boundary_inventories(local_plan, remote_plan);
    let local = BoundaryLocalStore {
        revisions: RefCell::new(BTreeMap::from([(
            "PR-SAME-TARGET".to_string(),
            local_revisions,
        )])),
        next_created_plan_id: RefCell::new(Some("PR-LOCAL-RESET".to_string())),
        ..BoundaryLocalStore::default()
    };
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let mut remote = BoundaryRemote {
        details: BTreeMap::from([(
            ("PR-SAME-TARGET".to_string(), "RPR-SAME-HEAD".to_string()),
            remote_head,
        )]),
        histories: BTreeMap::from([("PR-SAME-TARGET".to_string(), remote_revisions)]),
        calls: Vec::new(),
    };

    let error = resolve_local_sync_plan_candidate(
        &local,
        &blobs,
        &identity,
        &request,
        &artifact,
        &mut local_inventory,
        &mut remote_inventory,
        Some(&mut remote),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    )
    .expect_err("an invalid receipt without an exact remote head must fail closed");

    assert!(error.contains("not an exact match"));
    assert_eq!(remote.calls, ["history:PR-SAME-TARGET"]);
    assert!(local.calls.borrow().is_empty());
    assert!(blobs.calls.borrow().is_empty());
    assert_eq!(identity.workflow_ids.get(), 0);
    assert_eq!(identity.timestamps.get(), 0);
}

#[test]
fn mixed_lineage_split_rejects_bound_receipt_mismatch_before_mutation() {
    let artifact = boundary_artifact();
    let (local_plan, local_revisions, mut bound_remote_revisions) =
        mixed_collision_local_fixture(&artifact);
    bound_remote_revisions[1]["artifact_blob_id"] =
        JsonValue::String("BLB-mismatched-bound-revision".to_string());

    let error = validate_exact_mixed_local_plan_lineage_split(
        &artifact,
        &local_plan,
        &local_revisions,
        "PR-BOUND-PUBLISHED",
        &bound_remote_revisions,
    )
    .expect_err("receipt metadata mismatch must fail closed");

    assert!(error.contains("does not match bound remote revision RPR-BOUND-2"));
}

#[test]
fn mixed_lineage_split_accepts_exact_bound_prefix_after_remote_advances() {
    let artifact = boundary_artifact();
    let (local_plan, local_revisions, mut bound_remote_revisions) =
        mixed_collision_local_fixture(&artifact);
    bound_remote_revisions.push(json!({
        "plan_id": "PR-BOUND-PUBLISHED",
        "plan_revision_id": "RPR-BOUND-3",
        "revision_number": 9,
        "artifact_path": "docs/sprints/bound.md",
        "artifact_selector": "bound/root",
        "artifact_heading": "Bound Three",
        "title_snapshot": "Bound Three",
        "artifact_blob_id": artifact_blob_id("# Bound Three\n"),
        "items": [],
    }));

    validate_exact_mixed_local_plan_lineage_split(
        &artifact,
        &local_plan,
        &local_revisions,
        "PR-BOUND-PUBLISHED",
        &bound_remote_revisions,
    )
    .expect("an exact recorded prefix remains authoritative after the old remote advances");
}

#[test]
fn mixed_lineage_split_keeps_equal_raw_local_and_bound_remote_ordinals_distinct() {
    let artifact = boundary_artifact();
    let (mut local_plan, local_revisions, mut bound_remote_revisions) =
        mixed_collision_local_fixture(&artifact);
    local_plan["published_plan_id"] = JsonValue::String("PR-COLLISION".to_string());
    for revision in &mut bound_remote_revisions {
        revision["plan_id"] = JsonValue::String("PR-COLLISION".to_string());
    }

    validate_exact_mixed_local_plan_lineage_split(
        &artifact,
        &local_plan,
        &local_revisions,
        "PR-COLLISION",
        &bound_remote_revisions,
    )
    .expect("LPR-COLLISION and RPR-COLLISION are different authority-scoped identities");
}

#[test]
fn same_ordinal_remote_adoption_allocates_distinct_local_plan() {
    let request = boundary_request(false);
    let remote_body = "# Remote Runtime\n";
    let remote_blob_id = artifact_blob_id(remote_body);
    let remote_plan = json!({
        "plan_id": "PR-COLLISION",
        "title": "Remote Runtime",
        "status": "draft",
        "head_revision_id": "RPR-RUNTIME-1",
        "head_revision": {
            "plan_revision_id": "RPR-RUNTIME-1",
            "artifact_path": "docs/runtime.md",
            "artifact_selector": null,
            "artifact_heading": "Remote Runtime",
            "artifact_blob_id": remote_blob_id,
            "items": [],
        }
    });
    let revisions = vec![json!({
        "plan_id": "PR-COLLISION",
        "plan_revision_id": "RPR-RUNTIME-1",
        "revision_number": 1,
        "artifact_path": "docs/runtime.md",
        "artifact_selector": null,
        "artifact_heading": "Remote Runtime",
        "title_snapshot": "Remote Runtime",
        "artifact_blob_id": remote_blob_id,
        "items": [],
    })];
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([(
            "PR-COLLISION".to_string(),
            json!({
                "plan_id": "PR-COLLISION",
                "status": "draft",
                "head_revision": {
                    "plan_revision_id": "LPR-UNRELATED",
                    "artifact_path": "docs/unrelated.md",
                    "artifact_selector": null,
                }
            }),
        )])),
        next_created_plan_id: RefCell::new(Some("PR-LOCAL-DISTINCT".to_string())),
        next_created_revision_id: RefCell::new(Some("LPR-RUNTIME-1".to_string())),
        ..BoundaryLocalStore::default()
    };
    let identity = BoundaryIdentitySource::default();

    let adopted = adopt_materialized_remote_plan_for_local_sync(
        &local,
        &identity,
        &request,
        &remote_plan,
        &revisions,
    )
    .expect("same remote ordinal must be adopted at a distinct local identity");

    assert_eq!(adopted["plan_id"], "PR-LOCAL-DISTINCT");
    assert_eq!(adopted["published_plan_id"], "PR-COLLISION");
    assert_eq!(
        local.calls.borrow().as_slice(),
        [
            "create:PR-COLLISION:PR-BOUNDARY-1",
            "publish:PR-LOCAL-DISTINCT:RPR-RUNTIME-1",
        ]
    );
}

#[test]
fn same_ordinal_remote_adoption_refuses_nonallocating_store_before_mutation() {
    let request = boundary_request(false);
    let remote_plan = json!({
        "plan_id": "PR-COLLISION",
        "title": "Remote Runtime",
        "status": "draft",
        "head_revision_id": "RPR-RUNTIME-1",
        "head_revision": {"plan_revision_id": "RPR-RUNTIME-1"},
    });
    let revisions = vec![json!({
        "plan_id": "PR-COLLISION",
        "plan_revision_id": "RPR-RUNTIME-1",
        "revision_number": 1,
        "artifact_path": "docs/runtime.md",
        "artifact_selector": null,
        "artifact_heading": "Remote Runtime",
        "artifact_blob_id": artifact_blob_id("# Remote Runtime\n"),
        "items": [],
    })];
    let local = BoundaryLocalStore {
        existing_plans: RefCell::new(BTreeMap::from([(
            "PR-COLLISION".to_string(),
            json!({
                "plan_id": "PR-COLLISION",
                "status": "draft",
                "head_revision": {
                    "plan_revision_id": "LPR-UNRELATED",
                    "artifact_path": "docs/unrelated.md",
                    "artifact_selector": null,
                }
            }),
        )])),
        ..BoundaryLocalStore::default()
    };
    let identity = BoundaryIdentitySource::default();

    let error = adopt_materialized_remote_plan_for_local_sync(
        &local,
        &identity,
        &request,
        &remote_plan,
        &revisions,
    )
    .expect_err("a non-allocating local store must fail before create/revise");

    assert!(error.contains("RPR-COLLISION"));
    assert!(error.contains("LPR-COLLISION"));
    assert!(local.calls.borrow().is_empty());
    assert_eq!(identity.workflow_ids.get(), 0);
    assert_eq!(identity.timestamps.get(), 0);
}

#[test]
fn full_history_adoption_prevalidates_every_revision_before_plan_mutation() {
    let request = boundary_request(false);
    let local = BoundaryLocalStore::default();
    let blobs = BoundaryBlobStore::default();
    let identity = BoundaryIdentitySource::default();
    let remote_plan = json!({
        "plan_id": "PR-REMOTE-HISTORY",
        "title": "Remote History",
        "status": "draft",
        "head_revision_id": "RPR-MISSING",
        "head_revision": {"plan_revision_id": "RPR-MISSING"},
    });
    let revisions = vec![
        json!({
            "plan_revision_id": "RPR-AVAILABLE",
            "revision_number": 1,
            "artifact_path": "docs/sprints/boundary.md",
            "artifact_heading": "Remote History",
            "artifact_body": "# Available\n",
            "items": [],
        }),
        json!({
            "plan_revision_id": "RPR-MISSING",
            "revision_number": 2,
            "artifact_path": "docs/sprints/boundary.md",
            "artifact_heading": "Remote History",
            "artifact_blob_id": "BLB-missing",
            "items": [],
        }),
    ];
    let mut remote = BoundaryRemote::default();
    let mut detail_cache = BTreeMap::new();

    let error = adopt_remote_plan_for_local_sync(
        &local,
        &blobs,
        &identity,
        &request,
        &remote_plan,
        &revisions,
        Some(&mut remote),
        &mut detail_cache,
    )
    .expect_err("unavailable history must abort before creating a partial local Plan");

    assert!(error.contains("missing boundary-test revision"));
    assert!(local.calls.borrow().is_empty());
    assert_eq!(identity.workflow_ids.get(), 0);
    assert_eq!(identity.timestamps.get(), 0);
    assert_eq!(blobs.calls.borrow().len(), 1);
}
