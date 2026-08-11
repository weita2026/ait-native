use super::*;

fn packed_payload(
    service: &BinaryDbServerPlanServiceV1<FilesystemServerRemoteBinaryDb>,
    label: &str,
    title: &str,
    item_ref: &str,
    done: bool,
) -> JsonValue {
    packed_payload_at_path(
        service,
        label,
        title,
        item_ref,
        done,
        "docs/sprints/demo.md",
    )
}

fn packed_payload_at_path(
    service: &BinaryDbServerPlanServiceV1<FilesystemServerRemoteBinaryDb>,
    label: &str,
    title: &str,
    item_ref: &str,
    done: bool,
    artifact_path: &str,
) -> JsonValue {
    let body = if done {
        format!("- [x] {item_ref} Task item")
    } else {
        format!("- [ ] {item_ref} Task item")
    };
    let mut payload = create_payload(title, item_ref);
    let object = payload.as_object_mut().expect("payload object");
    object.insert("artifact_path".to_string(), json!(artifact_path));
    object.insert("artifact_body".to_string(), json!(body));
    object.insert(
        "packed_artifact".to_string(),
        seed_packed_plan_content(service, label, artifact_path, &body),
    );
    object.insert(
        "items".to_string(),
        json!([{
            "plan_item_ref": item_ref,
            "text": "Task item",
            "checkbox_state": if done { "done" } else { "open" },
            "heading_path": ["Demo"],
            "line_number": 1,
        }]),
    );
    payload
}

#[test]
fn exact_artifact_lookup_does_not_read_nonmatching_plan_titles() {
    let service = service("bounded-artifact-lookup");
    service
        .create_plan(
            "repo-bin",
            &packed_payload_at_path(
                &service,
                "nonmatching",
                "Nonmatching title",
                "OTHER-1",
                false,
                "docs/sprints/other.md",
            ),
        )
        .expect("create nonmatching Plan");
    service
        .create_plan(
            "repo-bin",
            &packed_payload(&service, "matching", "Matching title", "MATCH-1", false),
        )
        .expect("create matching Plan");

    let nonmatching = service
        .store()
        .read_plan_record(0)
        .expect("nonmatching Plan record");
    let title_path = service
        .db()
        .authority_root()
        .as_path()
        .join(plan_payload_file().as_str());
    let mut title_file = OpenOptions::new()
        .write(true)
        .open(title_path)
        .expect("Plan title payload should open");
    title_file
        .seek(SeekFrom::Start(nonmatching.payload_offset))
        .expect("nonmatching title should seek");
    title_file
        .write_all(&vec![0xff; usize::from(nonmatching.payload_len)])
        .expect("nonmatching title should corrupt");

    let matching = service
        .list_plans("repo-bin", Some("docs/sprints/demo.md"))
        .expect("exact lookup must not decode nonmatching titles");
    assert_eq!(matching.as_array().map(Vec::len), Some(1));
    assert_eq!(matching[0]["plan_id"], "PR-1");
    assert_eq!(matching[0]["title"], "Matching title");

    let error = service
        .list_plans("repo-bin", None)
        .expect_err("unfiltered listing must still fail closed on a corrupt title");
    assert!(error.contains("UTF-8"), "{error}");
}

#[test]
fn server_plan_runtime_uses_only_compact_plan_bins_and_same_repo_object_packs() {
    let service = service("compact-only");
    let reopened_db = service.db().clone();
    let created = service
        .create_plan(
            "repo-bin",
            &packed_payload(&service, "create", "Demo", "PLAN-1", false),
        )
        .expect("compact Plan create");
    assert_eq!(created["plan_id"], "PR-0");
    assert_eq!(created["head_revision_id"], "plan-revision:0");
    let initial = service
        .get_plan_revision("PR-0", "plan-revision:0")
        .expect("initial compact revision read");
    assert_eq!(initial["artifact_body"], "- [ ] PLAN-1 Task item");

    let mut revision = packed_payload(&service, "revise", "Demo revised", "PLAN-1", true);
    revision.as_object_mut().expect("revision object").insert(
        "expected_head_revision_id".to_string(),
        json!("plan-revision:0"),
    );
    service
        .revise_plan("PR-0", &revision)
        .expect("compact Plan revise");
    let revised = service
        .get_plan_revision("PR-0", "plan-revision:1")
        .expect("compact revision read");
    assert_eq!(revised["artifact_body"], "- [x] PLAN-1 Task item");
    assert_eq!(revised["artifact_blob_id"], revised["blob"]["blob_id"]);

    let root = service.db().authority_root().as_path();
    for entry in fs::read_dir(root).expect("authority directory") {
        let entry = entry.expect("authority entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(!name.starts_with("server_plan_"), "unexpected {name}");
        if name.ends_with(".bin") {
            assert!(
                crate::foundation::server_binary_db_schema_registry::server_binary_db_bin_path_is_declared(&name),
                "undeclared {name}"
            );
        }
    }

    let reopened = BinaryDbServerPlanService::new(reopened_db);
    let plans = reopened
        .list_plans("repo-bin", Some("docs/sprints/demo.md"))
        .expect("restart list");
    assert_eq!(plans.as_array().map(Vec::len), Some(1));
    assert_eq!(plans[0]["plan_id"], "PR-0");
}

#[test]
fn absent_plan_index_returns_not_found_without_reading_past_plan_bin() {
    let service = service("absent-plan-index");
    let error = service
        .get_plan("PR-0")
        .expect_err("an empty authority has no PR-0");

    assert_eq!(error, "Unknown plan: PR-0");
    assert!(!error.contains("out of bounds"), "{error}");
}

#[test]
fn task_binding_projection_does_not_hydrate_plan_artifact_body() {
    let service = service("task-binding-without-artifact-hydration");
    let payload = packed_payload(
        &service,
        "task-binding",
        "Task binding",
        "TASK-BINDING-1",
        false,
    );
    let pack_id = payload["packed_artifact"]["object_pack"]["pack_id"]
        .as_str()
        .expect("fixture pack id")
        .to_string();
    service
        .create_plan("repo-bin", &payload)
        .expect("create Plan with packed artifact");

    let pack_path = service
        .db()
        .authority_root()
        .as_path()
        .join(".ait/objects/packs")
        .join(format!("{pack_id}.zstpack"));
    fs::remove_file(pack_path).expect("remove Plan artifact pack");
    assert!(
        service
            .get_plan_revision("PR-0", "plan-revision:0")
            .is_err(),
        "full revision hydration must observe the missing artifact pack"
    );

    let read = BinaryDbReadTxn::new(service.db());
    let projection = service
        .task_binding_projection_with_read(&read, 0, 0)
        .expect("Task binding must use compact Plan records only");
    assert_eq!(
        projection,
        (0, "TASK-BINDING-1".to_string(), vec!["Demo".to_string()])
    );
}

#[test]
fn normal_plan_reads_and_revise_do_not_scan_historical_content() {
    let service = service("no-historical-content-scan");
    let first = packed_payload(&service, "history-1", "Demo", "PLAN-1", false);
    let first_pack_id = first["packed_artifact"]["object_pack"]["pack_id"]
        .as_str()
        .expect("first pack id")
        .to_string();
    service
        .create_plan("repo-bin", &first)
        .expect("create first revision");

    let mut second = packed_payload(&service, "history-2", "Demo", "PLAN-2", true);
    second
        .as_object_mut()
        .expect("second revision object")
        .insert(
            "expected_head_revision_id".to_string(),
            json!("plan-revision:0"),
        );
    service
        .revise_plan("PR-0", &second)
        .expect("create second revision");

    let historical_pack_path = service
        .db()
        .authority_root()
        .as_path()
        .join(".ait/objects/packs")
        .join(format!("{first_pack_id}.zstpack"));
    fs::remove_file(&historical_pack_path).expect("historical pack should remove");

    let plans = service
        .list_plans("repo-bin", None)
        .expect("Plan list must not resolve historical artifact bodies");
    assert_eq!(plans.as_array().map(Vec::len), Some(1));
    let revisions = service
        .list_plan_revisions("PR-0")
        .expect("revision list must follow records without resolving bodies");
    assert_eq!(revisions.as_array().map(Vec::len), Some(2));

    let mut third = packed_payload(&service, "history-3", "Demo", "PLAN-3", false);
    third
        .as_object_mut()
        .expect("third revision object")
        .insert(
            "expected_head_revision_id".to_string(),
            json!("plan-revision:1"),
        );
    service
        .revise_plan("PR-0", &third)
        .expect("revise must read only the selected Plan head record");
    assert_eq!(
        service
            .list_plan_revisions("PR-0")
            .expect("three revision records")
            .as_array()
            .map(Vec::len),
        Some(3)
    );

    let error = service
        .get_plan_revision("PR-0", "plan-revision:0")
        .expect_err("an explicit historical body read must report its missing pack");
    assert!(error.contains(&first_pack_id), "{error}");
}

#[test]
fn plan_artifact_closure_audit_reads_every_referenced_historical_blob() {
    let service = service("artifact-closure-audit");
    let payload = packed_payload(&service, "closure", "Demo", "PLAN-1", false);
    let blob_id = payload["packed_artifact"]["artifact_blob_id"]
        .as_str()
        .expect("artifact blob id")
        .to_string();
    let pack_id = payload["packed_artifact"]["object_pack"]["pack_id"]
        .as_str()
        .expect("object pack id")
        .to_string();
    service
        .create_plan("repo-bin", &payload)
        .expect("create packed Plan");

    let healthy = service
        .audit_artifact_blob_closure()
        .expect("healthy closure audit");
    assert!(healthy.is_complete());
    assert_eq!(healthy.plan_count, 1);
    assert_eq!(healthy.revision_count, 1);
    assert_eq!(healthy.referenced_revision_count, 1);
    assert_eq!(healthy.referenced_blob_count, 1);
    assert_eq!(healthy.healthy_blob_count, 1);
    assert!(healthy.issues.is_empty());
    assert_eq!(
        service
            .referenced_artifact_blob_ids()
            .expect("historical artifact identities"),
        vec![blob_id.clone()]
    );

    let pack_path = service
        .db()
        .authority_root()
        .as_path()
        .join(".ait/objects/packs")
        .join(format!("{pack_id}.zstpack"));
    fs::remove_file(pack_path).expect("remove historical pack");

    let blocked = service
        .audit_artifact_blob_closure()
        .expect("blocked closure audit");
    assert!(!blocked.is_complete());
    assert_eq!(blocked.unhealthy_blob_count, 1);
    assert_eq!(blocked.unhealthy_revision_count, 1);
    assert_eq!(blocked.issues.len(), 1);
    assert_eq!(blocked.issues[0].plan_id, "PR-0");
    assert_eq!(blocked.issues[0].plan_revision_id, "plan-revision:0");
    assert_eq!(blocked.issues[0].artifact_blob_id, blob_id);
    assert!(blocked.issues[0].error.contains(&pack_id));
    assert_eq!(
        service
            .referenced_artifact_blob_ids()
            .expect("artifact identities do not require pack reads"),
        vec![blocked.issues[0].artifact_blob_id.clone()]
    );
}

#[test]
fn server_plan_read_accepts_canonical_nonzero_revision_root_fields() {
    let service = service("canonical-revision-root-fields");
    service
        .create_plan(
            "repo-bin",
            &packed_payload(&service, "canonical-root", "Demo", "PLAN-1", false),
        )
        .expect("compact Plan create");

    let root = service.db().authority_root().as_path();
    let path = root.join(plan_revision_file().as_str());
    let mut bytes = fs::read(&path).expect("canonical revision file should read");
    let record_offset = 4;
    bytes[record_offset + 32..record_offset + 36].copy_from_slice(&9_u32.to_le_bytes());
    bytes[record_offset + 36..record_offset + 40].copy_from_slice(&10_u32.to_le_bytes());
    fs::write(&path, bytes).expect("canonical revision fields should write");

    let revision = service
        .get_plan_revision("PR-0", "plan-revision:0")
        .expect("canonical nonzero revision root fields must remain readable");
    assert_eq!(revision["artifact_body"], "- [ ] PLAN-1 Task item");
}

#[test]
fn server_plan_inline_body_without_existing_pack_is_rejected_without_files() {
    let service = service("inline-rejected");
    let mut payload = create_payload("Demo", "PLAN-1");
    payload
        .as_object_mut()
        .expect("payload object")
        .insert("artifact_body".to_string(), json!("- [ ] PLAN-1 Task item"));
    let error = service
        .create_plan("repo-bin", &payload)
        .expect_err("inline Plan body must not create an extra content bin");
    assert!(error.contains("same-repository packed_artifact"), "{error}");
    assert!(!service
        .db()
        .authority_root()
        .as_path()
        .join("plan.bin")
        .exists());
}

#[test]
fn server_plan_stale_expected_head_fails_without_writes() {
    let service = service("stale-head");
    service
        .create_plan(
            "repo-bin",
            &packed_payload(&service, "stale-create", "Demo", "PLAN-1", false),
        )
        .expect("create");
    let mut revision = packed_payload(&service, "stale-revise", "Demo", "PLAN-1", true);
    revision.as_object_mut().expect("revision object").insert(
        "expected_head_revision_id".to_string(),
        json!("plan-revision:99"),
    );
    let before = authority_data_files(&service);
    let error = service
        .revise_plan("PR-0", &revision)
        .expect_err("stale head must fail");
    assert!(error.contains("head advanced"), "{error}");
    assert_eq!(authority_data_files(&service), before);
}

#[test]
fn unsupported_artifact_write_route_fails_without_writes() {
    let service = service("artifact-write-rejected");
    service
        .create_plan(
            "repo-bin",
            &packed_payload(&service, "artifact-create", "Demo", "PLAN-1", false),
        )
        .expect("create");
    let before = authority_data_files(&service);
    let error = service
        .put_plan_revision_artifacts(
            "PR-0",
            "plan-revision:0",
            &json!({"artifacts": [{"artifact_path": "docs/extra.md", "body": "extra"}]}),
        )
        .expect_err("unsupported artifact write route must fail");
    assert!(error.contains("not part of the compact layout-1 Plan schema"));
    assert_eq!(authority_data_files(&service), before);
}

#[test]
fn unsupported_compact_layout_fails_closed() {
    let service = service("unsupported-layout");
    overwrite_record_file_layout(&service, plan_file(), UNSUPPORTED_TEST_LAYOUT);
    let error = service
        .list_plans("repo-bin", None)
        .expect_err("unsupported layout must fail");
    assert!(
        error.contains("unsupported compact Plan Binary DB layout"),
        "{error}"
    );
}
