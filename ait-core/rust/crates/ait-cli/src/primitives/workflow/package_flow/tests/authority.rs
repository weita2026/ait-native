use super::*;
use ait_core::change_store::ChangeStore;
use ait_core::plan_http_client::{
    build_get_change_request_spec, build_get_patchset_request_spec,
    build_get_remote_snapshot_request_spec, build_get_task_request_spec,
    build_list_patchsets_request_spec, PlanHttpRequestSpec,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
}

impl Reply {
    fn json(value: JsonValue) -> Self {
        Self {
            status: 200,
            body: value.to_string(),
        }
    }
    fn value(&self) -> JsonValue {
        serde_json::from_str(&self.body).unwrap()
    }
}

struct AuthorityFixture {
    local: Fixture,
    entries: Vec<JsonValue>,
    originals: BTreeMap<String, Vec<(String, SnapshotWorkflowBinding)>>,
    rows: BTreeMap<String, Reply>,
    checkpoints: Vec<String>,
    endpoints: Vec<String>,
    receipt_paths: Vec<String>,
    endpoint_paths: Vec<String>,
    list_paths: Vec<String>,
    task_paths: Vec<String>,
    checkpoint_paths: Vec<String>,
    change_path: Option<String>,
    server: tiny_http::Server,
    remote: RemoteRow,
}

fn path(spec: PlanHttpRequestSpec) -> String {
    let (_, address) = spec.url.split_once("://").unwrap();
    address[address.find('/').unwrap()..].to_string()
}

impl AuthorityFixture {
    fn new(mapped_count: usize, mixed_changes: bool) -> Self {
        let mut local = fixture();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let remote = RemoteRow {
            url: format!("http://{}", server.server_addr()),
            ..local.remote.clone()
        };
        let config = http_config(&local.repo, &remote);
        let content = local
            .repo
            .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .unwrap();
        let tasks = local.repo.task_store().unwrap();
        let changes = local.repo.change_store().unwrap();
        let mut rows = BTreeMap::new();
        let mut entries = Vec::new();
        let mut originals = BTreeMap::new();
        let mut checkpoints = Vec::new();
        let mut endpoints = Vec::new();
        let mut receipt_paths = Vec::new();
        let mut endpoint_paths = Vec::new();
        let mut list_paths = Vec::new();
        let mut task_paths = Vec::new();
        let mut checkpoint_paths = Vec::new();
        let mut change_path = None;
        let mut previous = local.original.clone();
        local.target = required_string_field(
            &content
                .create_snapshot_content_with_workflow_binding(
                    "fixture",
                    "main",
                    std::slice::from_ref(&previous),
                    Some("Independent target advancement"),
                    false,
                    SnapshotAuthoringOptions::default(),
                    None,
                )
                .unwrap(),
            "snapshot_id",
        )
        .unwrap();
        for ordinal in 0..=mapped_count {
            let task = TaskStore::create_task(
                &tasks,
                "fixture",
                &format!("Source {ordinal}"),
                "Keep exact source history",
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let id = required_string_field(&task, "task_id").unwrap();
            let line = format!("feature/source-{ordinal}");
            local
                .repo
                .line_store()
                .unwrap()
                .create_line(&line, Some(&previous), "2026-10-20T00:00:00Z")
                .unwrap();
            let mut owner = SnapshotWorkflowBinding {
                task_id: id.clone(),
                change_id: String::new(),
                line_name: line.clone(),
            };
            if mixed_changes && ordinal == 0 {
                ChangeStore::create_change(
                    &changes,
                    &id,
                    "fixture",
                    "Published revision",
                    &line,
                    None,
                    Some(&previous),
                )
                .unwrap();
                owner.change_id = "C-01".into();
            }
            fs::write(
                local.temp.path().join(format!("checkpoint-{ordinal}.txt")),
                "original",
            )
            .unwrap();
            let checkpoint = content
                .create_snapshot_content_with_workflow_binding(
                    "fixture",
                    &line,
                    std::slice::from_ref(&previous),
                    Some("Original feature checkpoint"),
                    true,
                    SnapshotAuthoringOptions::default(),
                    Some((&owner, "")),
                )
                .unwrap();
            let checkpoint = required_string_field(&checkpoint, "snapshot_id").unwrap();
            if mixed_changes && ordinal == 0 {
                ChangeStore::create_change(
                    &changes,
                    &id,
                    "fixture",
                    "Later revision",
                    &line,
                    None,
                    Some(&previous),
                )
                .unwrap();
                owner.change_id = "C-02".into();
            }
            fs::write(
                local.temp.path().join(format!("endpoint-{ordinal}.txt")),
                "rebased result",
            )
            .unwrap();
            // The original checkpoint is a sibling, not an ancestor, of the
            // accepted result. Fixed ownership must retain both after rebase.
            let endpoint_parent = if mixed_changes && ordinal == 0 {
                &checkpoint
            } else {
                &previous
            };
            let endpoint = content
                .create_snapshot_content_with_workflow_binding(
                    "fixture",
                    &line,
                    std::slice::from_ref(endpoint_parent),
                    Some("Rebased Local result"),
                    true,
                    SnapshotAuthoringOptions::default(),
                    Some((&owner, "")),
                )
                .unwrap();
            let endpoint = required_string_field(&endpoint, "snapshot_id").unwrap();
            let selected =
                (!owner.change_id.is_empty()).then(|| format!("{id}/{}", owner.change_id));
            tasks
                .finish_local_task(
                    &id,
                    &endpoint,
                    "main",
                    Some(&previous),
                    selected.as_deref(),
                    None,
                )
                .unwrap();
            let mapped = format!("RT-{:04}", 1001 + ordinal);
            if ordinal < mapped_count {
                TaskStore::mark_task_published(&tasks, &id, Some("origin"), Some(&mapped)).unwrap();
                if mixed_changes && ordinal == 0 {
                    ChangeStore::mark_change_published(
                        &changes,
                        &format!("{id}/C-01"),
                        Some("origin"),
                        Some(&format!("{mapped}/C-07")),
                        true,
                    )
                    .unwrap();
                    let change = ChangeStore::get_change(&changes, &format!("{id}/C-01")).unwrap();
                    let p = path(
                        build_get_change_request_spec(
                            &config,
                            &format!("{mapped}/C-07"),
                            Some("fixture"),
                        )
                        .unwrap(),
                    );
                    rows.insert(p.clone(), Reply::json(json!({"repository_index":0,"task_id":mapped,"change_ref":format!("{mapped}/C-07"),"title":change["title"],"fork_snapshot_id":previous})));
                    change_path = Some(p);
                }
            }
            let source = tasks.completed_task_source_snapshots(&id).unwrap();
            assert_eq!(source.len(), 2);
            assert_eq!(source[0].0, checkpoint);
            assert_eq!(source[1].0, endpoint);
            assert!(source.iter().all(|(_, binding)| binding.task_id == id));
            assert_eq!(
                snapshot_distance_if_ancestor(&local.repo, Some(&checkpoint), Some(&endpoint))
                    .unwrap()
                    .is_none(),
                !(mixed_changes && ordinal == 0)
            );
            let mut entry =
                workflow_completed_local_entry_if_present(&local.repo, &id, Some("origin"))
                    .unwrap()
                    .unwrap()["state"]
                    .clone();
            entry["local_task_id"] = json!(id);
            if ordinal < mapped_count {
                let receipt = &entry["local_finish"];
                let time = snapshot_seconds(receipt["updated_at"].as_str().unwrap()).unwrap();
                let original_patchset = format!("{mapped}/P-03");
                let latest = format!("{mapped}/P-09");
                let mixed = mixed_changes && ordinal == 0;
                let summary = json!({"contract":if mixed {"ait-local-land-receipt/v2"} else {"ait-local-land-receipt/v1"},"local_task_id":id,"local_change_id":if mixed {JsonValue::Null} else {json!("C-01")},"local_change_ref":if mixed {JsonValue::Null} else {json!(format!("{id}/C-01"))},"task_id":mapped,"change_ref":if mixed {JsonValue::Null} else {json!(format!("{mapped}/C-01"))},"target_line":"main","pre_land_target_snapshot_id":previous,"landed_snapshot_id":endpoint,"landed_at_s":time,"source_kind":"imported_local_land_receipt"});
                let p =
                    path(build_get_task_request_spec(&config, &mapped, Some("fixture")).unwrap());
                rows.insert(p.clone(), Reply::json(json!({"repository_index":0,"task_id":mapped,"status":"active","current_patchset_id":latest})));
                task_paths.push(p);
                let p = path(
                    build_list_patchsets_request_spec(&config, &mapped, Some("fixture")).unwrap(),
                );
                rows.insert(
                    p.clone(),
                    Reply::json(json!([{"patchset_id":latest},{"patchset_id":original_patchset}])),
                );
                list_paths.push(p);
                let p = path(
                    build_get_patchset_request_spec(&config, &latest, Some("fixture"), None)
                        .unwrap(),
                );
                rows.insert(p, Reply::json(json!({"repository_index":0,"task_id":mapped,"patchset_id":latest,"change_ref":null,"source_kind":"manual","summary":"Later mutable candidate"})));
                let p = path(
                    build_get_patchset_request_spec(
                        &config,
                        &original_patchset,
                        Some("fixture"),
                        None,
                    )
                    .unwrap(),
                );
                rows.insert(p.clone(), Reply::json(json!({"repository_index":0,"task_id":mapped,"patchset_id":original_patchset,"change_ref":null,"source_kind":"imported_local_land_receipt","summary":format!("ait-local-land-receipt/v1 {summary}"),"base_snapshot_id":previous,"revision_snapshot_id":endpoint,"created_at":receipt["updated_at"],"ci_run_seq":0})));
                receipt_paths.push(p);
                let p = path(
                    build_get_remote_snapshot_request_spec(
                        &config,
                        "fixture",
                        &checkpoint,
                        false,
                        None,
                    )
                    .unwrap(),
                );
                rows.insert(p.clone(), Reply { status: 404, body: json!({"detail":format!("Unknown snapshot {checkpoint} for repository 0")}).to_string() });
                checkpoint_paths.push(p);
                if mixed {
                    let mut snapshot = snapshot_show(&local.repo, &checkpoint).unwrap();
                    snapshot["repository_index"] = json!(0);
                    snapshot["workflow_owner"] =
                        json!({"task_id":mapped,"change_ref":format!("{mapped}/C-07")});
                    rows.insert(
                        checkpoint_paths.last().unwrap().clone(),
                        Reply::json(snapshot),
                    );
                }
                let mut snapshot = snapshot_show(&local.repo, &endpoint).unwrap();
                snapshot["repository_index"] = json!(0);
                snapshot["workflow_owner"] = json!({"task_id":mapped,"change_ref":if mixed {json!(format!("{mapped}/C-08"))} else {JsonValue::Null}});
                let p = path(
                    build_get_remote_snapshot_request_spec(
                        &config, "fixture", &endpoint, false, None,
                    )
                    .unwrap(),
                );
                rows.insert(p.clone(), Reply::json(snapshot));
                endpoint_paths.push(p);
            }
            originals.insert(id.clone(), source);
            entries.push(entry);
            checkpoints.push(checkpoint);
            endpoints.push(endpoint.clone());
            previous = endpoint;
            local.id = id;
        }
        local.entry =
            workflow_completed_local_entry_if_present(&local.repo, &local.id, Some("origin"))
                .unwrap()
                .unwrap();
        Self {
            local,
            entries,
            originals,
            rows,
            checkpoints,
            endpoints,
            receipt_paths,
            endpoint_paths,
            list_paths,
            task_paths,
            checkpoint_paths,
            change_path,
            server,
            remote,
        }
    }

    fn run(self, prepare: bool) -> (Result<Option<PreparedSource>, String>, Vec<String>) {
        let reads = Arc::new(Mutex::new(Vec::new()));
        let requested = reads.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let rows = self.rows;
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Some(request) = self.server.recv_timeout(Duration::from_millis(25)).unwrap()
                else {
                    continue;
                };
                assert_eq!(
                    request.method().as_str(),
                    "GET",
                    "authority checks must not publish early"
                );
                let url = request.url().to_string();
                requested.lock().unwrap().push(url.clone());
                let reply = rows
                    .get(&url)
                    .unwrap_or_else(|| panic!("Unexpected authority read {url}"));
                request
                    .respond(
                        tiny_http::Response::from_string(reply.body.clone())
                            .with_status_code(reply.status),
                    )
                    .unwrap();
            }
        });
        let tasks = self.local.repo.task_store().unwrap();
        let before = self
            .entries
            .iter()
            .map(|entry| {
                TaskStore::get_task(&tasks, entry["local_task_id"].as_str().unwrap()).unwrap()
            })
            .collect::<Vec<_>>();
        let result = if prepare {
            prepare_source_history_at_base(
                &self.local.repo,
                &self.remote,
                &self.local.entry,
                &self.local.target,
                None,
                None,
            )
            .map(Some)
        } else {
            verify_published_source_authority(
                &self.local.repo,
                &self.remote,
                &self.entries,
                &self.originals,
            )
            .map(|_| None)
        };
        for (entry, before) in self.entries.iter().zip(before) {
            let after =
                TaskStore::get_task(&tasks, entry["local_task_id"].as_str().unwrap()).unwrap();
            assert_eq!(
                after, before,
                "guard/replay must preserve original receipt and publication mappings"
            );
        }
        stop.store(true, Ordering::SeqCst);
        thread.join().unwrap();
        let reads = reads.lock().unwrap().clone();
        (result, reads)
    }

    fn edit_receipt(&mut self, edit: impl FnOnce(&mut JsonValue)) {
        let p = &self.receipt_paths[0];
        let mut row = self.rows[p].value();
        let mut receipt: JsonValue = serde_json::from_str(
            row["summary"]
                .as_str()
                .unwrap()
                .strip_prefix("ait-local-land-receipt/v1 ")
                .unwrap(),
        )
        .unwrap();
        edit(&mut receipt);
        row["summary"] = json!(format!("ait-local-land-receipt/v1 {receipt}"));
        self.rows.insert(p.clone(), Reply::json(row));
    }
}

#[test]
fn six_mapped_receipts_allow_off_dag_checkpoints_and_an_unmapped_existing_suffix() {
    let mut fixture = AuthorityFixture::new(6, false);
    fixture.local.target = fixture.local.original.clone();
    let originals = fixture.originals.clone();
    let checkpoints = fixture.checkpoints.clone();
    let (result, reads) = fixture.run(true);
    let prepared = result.unwrap().unwrap();
    assert_eq!(prepared.manifest.history_entries.len(), 7);
    for (entry, checkpoint) in prepared.manifest.history_entries.iter().zip(checkpoints) {
        let task = entry["local_task_id"].as_str().unwrap();
        let snapshots = entry["snapshots"].as_array().unwrap();
        assert!(snapshots
            .iter()
            .any(|snapshot| snapshot["snapshot_id"] == checkpoint));
        for (id, owner) in &originals[task] {
            assert_eq!(owner.task_id, task);
            assert!(snapshots
                .iter()
                .any(|snapshot| snapshot["snapshot_id"] == *id));
        }
        assert_eq!(
            snapshots.len(),
            originals[task].len(),
            "workflow ready must not append replay history"
        );
    }
    assert_eq!(
        reads.iter().filter(|p| p.contains("/snapshots/")).count(),
        12
    );
    assert_eq!(
        reads.iter().filter(|p| p.ends_with("/patchsets")).count(),
        6
    );
    assert!(prepared.mappings.last().unwrap().remote_task_id.is_none());
}

#[test]
fn source_authority_rejects_wrong_or_ambiguous_typed_receipts() {
    for field in [
        "local_task_id",
        "task_id",
        "target_line",
        "pre_land_target_snapshot_id",
        "landed_snapshot_id",
        "landed_at_s",
        "contract",
        "source_kind",
        "local_change_ref",
    ] {
        let mut fixture = AuthorityFixture::new(1, false);
        fixture.edit_receipt(|row| {
            row[field] = if field == "landed_at_s" {
                json!(1)
            } else {
                json!("wrong")
            }
        });
        assert!(fixture.run(false).0.is_err(), "{field}");
    }
    let mut fixture = AuthorityFixture::new(1, false);
    let row = fixture.rows[&fixture.receipt_paths[0]].value();
    let mut list = fixture.rows[&fixture.list_paths[0]].value();
    list.as_array_mut()
        .unwrap()
        .push(json!({"patchset_id":"RT-1001/P-04"}));
    fixture
        .rows
        .insert(fixture.list_paths[0].clone(), Reply::json(list));
    let mut duplicate = row;
    duplicate["patchset_id"] = json!("RT-1001/P-04");
    fixture.rows.insert(
        fixture.receipt_paths[0].replace("P-03", "P-04"),
        Reply::json(duplicate),
    );
    assert!(fixture.run(false).0.err().unwrap().contains("ambiguous"));
}

#[test]
fn source_authority_rejects_foreign_owners_and_immutable_endpoint_conflicts() {
    for field in [
        "repository_index",
        "snapshot_id",
        "workflow_owner",
        "manifest_hash",
        "line_name",
        "created_at",
        "parent_snapshot_ids",
    ] {
        let mut fixture = AuthorityFixture::new(1, false);
        let p = fixture.endpoint_paths[0].clone();
        let mut row = fixture.rows[&p].value();
        row[field] = match field {
            "repository_index" => json!(1),
            "workflow_owner" => json!({"task_id":"RT-9000","change_ref":null}),
            "parent_snapshot_ids" => json!([]),
            _ => json!("different"),
        };
        fixture.rows.insert(p, Reply::json(row));
        assert!(fixture.run(false).0.is_err(), "{field}");
    }
    let mut fixture = AuthorityFixture::new(1, false);
    let mut row = snapshot_show(&fixture.local.repo, &fixture.checkpoints[0]).unwrap();
    row["repository_index"] = json!(0);
    row["workflow_owner"] = json!({"task_id":"RT-9000","change_ref":null});
    fixture
        .rows
        .insert(fixture.checkpoint_paths[0].clone(), Reply::json(row));
    assert!(
        fixture.run(false).0.is_err(),
        "a positive endpoint cannot hide a conflicting existing checkpoint"
    );
}

#[test]
fn only_a_recognized_missing_incoming_snapshot_is_allowed() {
    for (status, body) in [
        (403, "permission denied"),
        (408, "Request timed out"),
        (500, "unavailable"),
        (404, "Unknown repository 0"),
        (200, "not JSON"),
    ] {
        let mut fixture = AuthorityFixture::new(1, false);
        fixture.rows.insert(
            fixture.checkpoint_paths[0].clone(),
            Reply {
                status,
                body: body.into(),
            },
        );
        assert!(fixture.run(false).0.is_err(), "{status}: {body}");
    }
    let mut fixture = AuthorityFixture::new(1, false);
    fixture.rows.insert(fixture.endpoint_paths[0].clone(), Reply { status:404, body:json!({"detail":format!("Unknown snapshot {} for repository 0",fixture.endpoints[0])}).to_string() });
    assert!(
        fixture.run(false).0.is_err(),
        "missing publication witness is not new content authority"
    );
}

#[test]
fn mapped_changes_need_their_own_current_owner_and_preserve_unmapped_changes() {
    assert!(AuthorityFixture::new(1, true).run(false).0.is_ok());
    for field in ["task_id", "change_ref", "fork_snapshot_id", "title"] {
        let mut fixture = AuthorityFixture::new(1, true);
        let p = fixture.change_path.clone().unwrap();
        let mut row = fixture.rows[&p].value();
        row[field] = json!("different");
        fixture.rows.insert(p, Reply::json(row));
        assert!(fixture.run(false).0.is_err(), "{field}");
    }
    let mut fixture = AuthorityFixture::new(1, true);
    let p = fixture.endpoint_paths[0].clone();
    let mut row = fixture.rows[&p].value();
    row["workflow_owner"]["change_ref"] = json!("RT-1001/C-07");
    fixture.rows.insert(p, Reply::json(row));
    assert!(
        fixture.run(false).0.is_err(),
        "distinct Local Changes cannot collapse onto the published mapping"
    );
}

#[test]
fn source_receipt_numeric_projection_and_original_time_must_agree() {
    for field in [
        "repository_index",
        "task_id",
        "patchset_id",
        "source_kind",
        "change_ref",
        "base_snapshot_id",
        "revision_snapshot_id",
        "created_at",
        "ci_run_seq",
        "summary",
    ] {
        let mut fixture = AuthorityFixture::new(1, false);
        let p = fixture.receipt_paths[0].clone();
        let mut row = fixture.rows[&p].value();
        row[field] = match field {
            "repository_index" | "ci_run_seq" => json!(1),
            _ => json!("different"),
        };
        fixture.rows.insert(p, Reply::json(row));
        assert!(fixture.run(false).0.is_err(), "{field}");
    }
    for field in ["repository_index", "task_id"] {
        let mut fixture = AuthorityFixture::new(1, false);
        let p = fixture.task_paths[0].clone();
        let mut row = fixture.rows[&p].value();
        row[field] = if field == "repository_index" {
            json!(1)
        } else {
            json!("RT-9000")
        };
        fixture.rows.insert(p, Reply::json(row));
        assert!(
            fixture.run(false).0.is_err(),
            "same Local IDs/title cannot authenticate {field}"
        );
    }
}

#[test]
fn missing_all_witnesses_and_a_prior_tasks_base_cannot_authenticate_a_mapping() {
    let mut fixture = AuthorityFixture::new(1, false);
    let p = fixture.endpoint_paths[0].clone();
    fixture.rows.insert(p, Reply { status:404, body:json!({"detail":format!("Unknown snapshot {} for repository 0",fixture.endpoints[0])}).to_string() });
    fixture
        .rows
        .insert(fixture.list_paths[0].clone(), Reply::json(json!([])));
    let mut base = snapshot_show(&fixture.local.repo, &fixture.local.original).unwrap();
    base["repository_index"] = json!(0);
    base["workflow_owner"] = json!({"task_id":"RT-0900","change_ref":null});
    let config = http_config(&fixture.local.repo, &fixture.remote);
    let p = path(
        build_get_remote_snapshot_request_spec(
            &config,
            "fixture",
            &fixture.local.original,
            false,
            None,
        )
        .unwrap(),
    );
    fixture.rows.insert(p.clone(), Reply::json(base));
    let (result, reads) = fixture.run(false);
    assert!(result.is_err());
    assert!(
        !reads.contains(&p),
        "a preceding Task's base must not become the source witness"
    );
}

#[test]
fn transport_failure_is_not_classified_as_absent_incoming_content() {
    let mut fixture = AuthorityFixture::new(1, false);
    fixture.remote.url = "http://127.0.0.1:0".into();
    let (result, reads) = fixture.run(false);
    assert!(result.is_err());
    assert!(reads.is_empty());
}

#[test]
fn native_optional_v2_quotations_do_not_create_change_owners() {
    let mut fixture = AuthorityFixture::new(1, true);
    fixture.edit_receipt(|row| {
        for field in ["local_change_id", "local_change_ref", "change_ref"] {
            row.as_object_mut().unwrap().remove(field);
        }
    });
    assert!(fixture.run(false).0.is_ok());
}

#[test]
fn stored_unowned_content_is_incoming_history_and_never_a_mapping_witness() {
    for fault in ["none", "immutable", "unparseable_owner", "all_unowned"] {
        let mut fixture = AuthorityFixture::new(1, false);
        let mut row = snapshot_show(&fixture.local.repo, &fixture.checkpoints[0]).unwrap();
        row["repository_index"] = json!(0);
        row["workflow_owner"] = JsonValue::Null;
        if fault == "immutable" {
            row["manifest_hash"] = json!("different");
        }
        if fault == "unparseable_owner" {
            row.as_object_mut().unwrap().remove("workflow_owner");
        }
        fixture
            .rows
            .insert(fixture.checkpoint_paths[0].clone(), Reply::json(row));
        if fault == "all_unowned" {
            let p = fixture.endpoint_paths[0].clone();
            let mut row = fixture.rows[&p].value();
            row["workflow_owner"] = JsonValue::Null;
            fixture.rows.insert(p, Reply::json(row));
            fixture
                .rows
                .insert(fixture.list_paths[0].clone(), Reply::json(json!([])));
        }
        assert_eq!(fixture.run(false).0.is_ok(), fault == "none", "{fault}");
    }
    let mut fixture = AuthorityFixture::new(1, false);
    let p = fixture.endpoint_paths[0].clone();
    let mut row = fixture.rows[&p].value();
    row["workflow_owner"] = JsonValue::Null;
    fixture.rows.insert(p, Reply::json(row));
    assert!(
        fixture.run(false).0.is_err(),
        "a typed receipt with an unowned endpoint is insufficient"
    );
}
