use super::data_source::{
    candidate_inputs_with_plan_command_data_source,
    get_plan_revision_with_plan_command_data_source, get_plan_with_plan_command_data_source,
    list_plan_revisions_with_plan_command_data_source, list_plans_with_plan_command_data_source,
    list_tasks_with_plan_command_data_source, PlanCommandCandidateInputSource,
    PlanCommandCandidateInputs, PlanCommandInspectSource, PlanCommandPlanLister,
    PlanCommandPlanReader, PlanCommandPlanRevisionReader, PlanCommandRevisionLister,
    PlanCommandRevisionReader, PlanCommandTaskLister,
};
use super::local_shadow_ports::{
    local_shadow_for_plan_with_plan_command_local_shadow_source,
    local_shadow_index_with_plan_command_local_shadow_source, PlanCommandLocalShadowSource,
};
use super::*;
use crate::binary_db::{
    AuthorityId, BinaryDbCommandScope, BinaryDbNoopFsyncPolicy, LocalBinaryDbFs, LocalStateScope,
};
use crate::json_support::json;
use crate::plan_binary_db::{
    BinaryDbPlanStore, PlanItemPayload, PlanItemRecord, PlanPayload, PlanRecord,
    PlanRevisionPayload, PlanRevisionRecord,
};
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use tempfile::{tempdir, TempDir};

const COMMAND_TEST_WRITE_LAYOUT: u32 = 1;
type CommandTestPlanStore = BinaryDbPlanStore<LocalBinaryDbFs, COMMAND_TEST_WRITE_LAYOUT>;

#[derive(Default)]
struct FakePlanCommandSource {
    plans: Vec<JsonValue>,
    plan_details: BTreeMap<String, JsonValue>,
    revisions: BTreeMap<String, Vec<JsonValue>>,
    revision_details: BTreeMap<(String, String), JsonValue>,
    tasks: Vec<JsonValue>,
    candidate_inputs: VecDeque<PlanCommandCandidateInputs>,
    calls: Vec<String>,
}

#[derive(Default)]
struct FakePlanCommandLocalShadowSource {
    shadow_index: JsonMap<String, JsonValue>,
    calls: Vec<String>,
}

impl PlanCommandLocalShadowSource for FakePlanCommandLocalShadowSource {
    fn local_shadow_index(&mut self) -> Result<JsonMap<String, JsonValue>, String> {
        self.calls.push("local_shadow_index".to_string());
        Ok(self.shadow_index.clone())
    }
}

impl FakePlanCommandSource {
    fn with_plan(plan_id: &str, repo_name: &str) -> Self {
        let plan_summary = json!({
            "plan_id": plan_id,
            "repo_name": repo_name,
            "title": "Plan title",
            "status": "active",
        });
        let revision = json!({
            "plan_id": plan_id,
            "plan_revision_id": "REV-1",
            "repo_name": repo_name,
            "revision_number": 1,
            "publication_state": "draft",
            "status": "active",
            "artifact_path": "docs/plan.md",
            "artifact_heading": "Plan",
            "items": [
                {
                    "plan_item_ref": "item-a",
                    "text": "Do the thing",
                    "checkbox_state": "open",
                    "heading_path": ["Plan"],
                    "line_number": 12,
                    "task_id": "TASK-1"
                }
            ],
        });
        let plan_detail = json!({
            "plan_id": plan_id,
            "repo_name": repo_name,
            "title": "Plan title",
            "status": "active",
            "publication_state": "published",
            "head_revision_id": "REV-1",
            "artifact_path": "docs/plan.md",
            "artifact_heading": "Plan",
            "published_plan_id": "REMOTE-PLAN-1",
            "published_head_revision_id": "REMOTE-REV-1",
            "head_revision": revision.clone(),
        });
        let task = json!({
            "task_id": "TASK-1",
            "plan_id": plan_id,
            "plan_item_ref": "item-a",
            "title": "Task title",
            "status": "active",
        });
        let mut source = Self {
            plans: vec![plan_summary],
            tasks: vec![task.clone()],
            ..Default::default()
        };
        source
            .plan_details
            .insert(plan_id.to_string(), plan_detail.clone());
        source
            .revisions
            .insert(plan_id.to_string(), vec![revision.clone()]);
        source
            .revision_details
            .insert((plan_id.to_string(), "REV-1".to_string()), revision);
        source
            .candidate_inputs
            .push_back(PlanCommandCandidateInputs {
                plans: vec![plan_detail],
                tasks: vec![task],
            });
        source
    }
}

fn new_binary_plan_command_fixture() -> (CommandTestPlanStore, TempDir) {
    let temp_dir = tempdir().expect("tempdir");
    let authority_root = temp_dir.path().join(".ait/binary-db");
    fs::create_dir_all(&authority_root).expect("create Binary DB authority");
    fs::create_dir_all(temp_dir.path().join(".ait/objects"))
        .expect("create Binary DB pack authority");
    fs::write(
        temp_dir.path().join(".ait/config.json"),
        br#"{"repo_name":"ait-core"}"#,
    )
    .expect("write repository config");
    let db = LocalBinaryDbFs::new(
        authority_root.as_path(),
        temp_dir.path(),
        AuthorityId::new("local:ait-core"),
        LocalStateScope::Repository,
    );
    let store = CommandTestPlanStore::new(db);
    seed_binary_plan_command_fixture(&store);
    (store, temp_dir)
}

fn seed_binary_plan_command_fixture(store: &CommandTestPlanStore) {
    let mut tx = store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin binary plan command fixture txn");

    store
        .append_plan_item(
            &mut tx,
            PlanItemRecord {
                item_meta: 0b0000_1101,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                line_number: 12,
            },
            &PlanItemPayload {
                plan_item_ref_bytes: b"item-a".to_vec(),
                text_bytes: b"Do the thing".to_vec(),
                heading_path: vec!["Plan".to_string()],
            },
        )
        .expect("append binary item");

    let (revision_index, _) = store
        .append_plan_revision(
            &mut tx,
            PlanRevisionRecord {
                revision_meta: 0b0000_0001,
                reserved0: 0,
                payload_len: 0,
                revision_number: 1,
                item_count: 1,
                payload_offset: 0,
                plan_index: 0,
                previous_revision_index_plus1: 0,
                item_start_index: 0,
                published_revision_index_plus1: 9,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                created_at_s: 1_700_000_200,
                published_at_s: 1_700_000_210,
            },
            &PlanRevisionPayload {
                title_snapshot_bytes: b"Plan title".to_vec(),
                summary_bytes: b"Fixture head".to_vec(),
                artifact_path_bytes: b"docs/plan.md".to_vec(),
                artifact_selector_bytes: b"plan".to_vec(),
                artifact_heading_bytes: b"Plan".to_vec(),
                artifact_blob_id_bytes: b"BLB-1".to_vec(),
            },
        )
        .expect("append binary revision");
    assert_eq!(revision_index, 0);

    let (plan_index, _) = store
        .append_plan(
            &mut tx,
            PlanRecord {
                plan_meta: 0b0000_0100,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                latest_revision_index_plus1: 1,
                published_plan_index_plus1: 7,
                published_latest_revision_index_plus1: 9,
                created_at_s: 1_700_000_100,
                updated_at_s: 1_700_000_300,
                published_at_s: 1_700_000_310,
            },
            &PlanPayload {
                title_bytes: b"Plan title".to_vec(),
            },
        )
        .expect("append binary plan");
    assert_eq!(plan_index, 0);

    tx.commit().expect("commit binary plan command fixture");
}

fn binary_plan_command_request(temp_dir: &TempDir) -> JsonValue {
    json!({
        "scope": "local",
        "repo_name": "ait-core",
        "plan_storage": {
            "write_layout": COMMAND_TEST_WRITE_LAYOUT,
            "authority_root": temp_dir.path().join(".ait/binary-db").to_string_lossy(),
            "repo_root": temp_dir.path().to_string_lossy(),
            "local_authority_id": "local:ait-core",
            "current_line_state_scope": "repository",
        },
    })
}

impl PlanCommandPlanLister for FakePlanCommandSource {
    fn list_plans(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String> {
        self.calls.push(format!("list_plans:{repo_name}"));
        Ok(self.plans.clone())
    }
}

impl PlanCommandPlanReader for FakePlanCommandSource {
    fn get_plan(&mut self, plan_id: &str) -> Result<JsonValue, String> {
        self.calls.push(format!("get_plan:{plan_id}"));
        self.plan_details
            .get(plan_id)
            .cloned()
            .ok_or_else(|| format!("missing plan {plan_id}"))
    }
}

impl PlanCommandRevisionLister for FakePlanCommandSource {
    fn list_plan_revisions(&mut self, plan_id: &str) -> Result<Vec<JsonValue>, String> {
        self.calls.push(format!("list_plan_revisions:{plan_id}"));
        self.revisions
            .get(plan_id)
            .cloned()
            .ok_or_else(|| format!("missing revisions {plan_id}"))
    }
}

impl PlanCommandRevisionReader for FakePlanCommandSource {
    fn get_plan_revision(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        self.calls
            .push(format!("get_plan_revision:{plan_id}:{plan_revision_id}"));
        self.revision_details
            .get(&(plan_id.to_string(), plan_revision_id.to_string()))
            .cloned()
            .ok_or_else(|| format!("missing revision {plan_revision_id}"))
    }
}

impl PlanCommandTaskLister for FakePlanCommandSource {
    fn list_tasks(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String> {
        self.calls.push(format!("list_tasks:{repo_name}"));
        Ok(self.tasks.clone())
    }
}

impl PlanCommandCandidateInputSource for FakePlanCommandSource {
    fn candidate_inputs(
        &mut self,
        repo_name: &str,
        contains_terms: &[String],
    ) -> Result<PlanCommandCandidateInputs, String> {
        self.calls.push(format!(
            "candidate_inputs:{repo_name}:{}",
            contains_terms.join(",")
        ));
        self.candidate_inputs
            .pop_front()
            .ok_or_else(|| "missing candidate inputs".to_string())
    }
}

#[test]
fn plan_command_data_source_bound_helpers_accept_command_specific_trait_objects() {
    let mut source = FakePlanCommandSource::with_plan("PLAN-1", "remote-repo");
    let contains_terms = vec!["Plan".to_string()];

    {
        let source_port: &mut dyn PlanCommandPlanLister = &mut source;
        let plans = list_plans_with_plan_command_data_source(source_port, "request-repo").unwrap();
        assert_eq!(plans[0]["plan_id"], "PLAN-1");
    }

    {
        let source_port: &mut dyn PlanCommandPlanRevisionReader = &mut source;
        let plan = get_plan_with_plan_command_data_source(source_port, "PLAN-1").unwrap();
        assert_eq!(plan["repo_name"], "remote-repo");
        let revision =
            get_plan_revision_with_plan_command_data_source(source_port, "PLAN-1", "REV-1")
                .unwrap();
        assert_eq!(revision["artifact_path"], "docs/plan.md");
    }

    {
        let source_port: &mut dyn PlanCommandRevisionLister = &mut source;
        let revisions =
            list_plan_revisions_with_plan_command_data_source(source_port, "PLAN-1").unwrap();
        assert_eq!(revisions[0]["plan_revision_id"], "REV-1");
    }

    {
        let source_port: &mut dyn PlanCommandInspectSource = &mut source;
        let tasks = source_port.list_tasks("request-repo").unwrap();
        assert_eq!(tasks[0]["task_id"], "TASK-1");
    }

    {
        let source_port: &mut dyn PlanCommandCandidateInputSource = &mut source;
        let candidates = candidate_inputs_with_plan_command_data_source(
            source_port,
            "request-repo",
            &contains_terms,
        )
        .unwrap();
        assert_eq!(candidates.plans[0]["plan_id"], "PLAN-1");
        assert_eq!(candidates.tasks[0]["task_id"], "TASK-1");
    }

    assert_eq!(
        source.calls,
        vec![
            "list_plans:request-repo",
            "get_plan:PLAN-1",
            "get_plan_revision:PLAN-1:REV-1",
            "list_plan_revisions:PLAN-1",
            "list_tasks:request-repo",
            "candidate_inputs:request-repo:Plan",
        ]
    );
}

#[test]
fn plan_command_data_source_helpers_accept_single_capability_ports() {
    struct PlanListerOnly;

    impl PlanCommandPlanLister for PlanListerOnly {
        fn list_plans(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String> {
            Ok(vec![json!({
                "plan_id": "PLAN-LIST",
                "repo_name": repo_name,
                "title": "Listed plan",
                "status": "active",
            })])
        }
    }

    let mut plan_lister = PlanListerOnly;
    let plans =
        list_plans_with_plan_command_data_source(&mut plan_lister, "capability-repo").unwrap();
    assert_eq!(plans[0]["plan_id"], "PLAN-LIST");
    assert_eq!(plans[0]["repo_name"], "capability-repo");

    struct PlanReaderOnly;

    impl PlanCommandPlanReader for PlanReaderOnly {
        fn get_plan(&mut self, plan_id: &str) -> Result<JsonValue, String> {
            Ok(json!({
                "plan_id": plan_id,
                "repo_name": "capability-repo",
                "title": "Readable plan",
            }))
        }
    }

    let mut plan_reader = PlanReaderOnly;
    let plan = get_plan_with_plan_command_data_source(&mut plan_reader, "PLAN-READ").unwrap();
    assert_eq!(plan["plan_id"], "PLAN-READ");

    struct RevisionListerOnly;

    impl PlanCommandRevisionLister for RevisionListerOnly {
        fn list_plan_revisions(&mut self, plan_id: &str) -> Result<Vec<JsonValue>, String> {
            Ok(vec![json!({
                "plan_id": plan_id,
                "plan_revision_id": "REV-LIST",
                "revision_number": 1,
            })])
        }
    }

    let mut revision_lister = RevisionListerOnly;
    let revisions =
        list_plan_revisions_with_plan_command_data_source(&mut revision_lister, "PLAN-REV")
            .unwrap();
    assert_eq!(revisions[0]["plan_revision_id"], "REV-LIST");

    struct RevisionReaderOnly;

    impl PlanCommandRevisionReader for RevisionReaderOnly {
        fn get_plan_revision(
            &mut self,
            plan_id: &str,
            plan_revision_id: &str,
        ) -> Result<JsonValue, String> {
            Ok(json!({
                "plan_id": plan_id,
                "plan_revision_id": plan_revision_id,
                "artifact_path": "docs/plan.md",
            }))
        }
    }

    let mut revision_reader = RevisionReaderOnly;
    let revision = get_plan_revision_with_plan_command_data_source(
        &mut revision_reader,
        "PLAN-REV",
        "REV-READ",
    )
    .unwrap();
    assert_eq!(revision["artifact_path"], "docs/plan.md");

    struct TaskListerOnly;

    impl PlanCommandTaskLister for TaskListerOnly {
        fn list_tasks(&mut self, repo_name: &str) -> Result<Vec<JsonValue>, String> {
            Ok(vec![json!({
                "task_id": "TASK-LIST",
                "repo_name": repo_name,
                "title": "Listed task",
            })])
        }
    }

    let mut task_lister = TaskListerOnly;
    let tasks =
        list_tasks_with_plan_command_data_source(&mut task_lister, "capability-repo").unwrap();
    assert_eq!(tasks[0]["task_id"], "TASK-LIST");

    struct CandidateInputOnly;

    impl PlanCommandCandidateInputSource for CandidateInputOnly {
        fn candidate_inputs(
            &mut self,
            repo_name: &str,
            contains_terms: &[String],
        ) -> Result<PlanCommandCandidateInputs, String> {
            Ok(PlanCommandCandidateInputs {
                plans: vec![json!({
                    "plan_id": "PLAN-CANDIDATE",
                    "repo_name": repo_name,
                    "matched_terms": contains_terms,
                })],
                tasks: vec![json!({"task_id": "TASK-CANDIDATE"})],
            })
        }
    }

    let mut candidate_source = CandidateInputOnly;
    let contains_terms = vec!["candidate".to_string()];
    let candidates = candidate_inputs_with_plan_command_data_source(
        &mut candidate_source,
        "capability-repo",
        &contains_terms,
    )
    .unwrap();
    assert_eq!(candidates.plans[0]["plan_id"], "PLAN-CANDIDATE");
    assert_eq!(candidates.tasks[0]["task_id"], "TASK-CANDIDATE");
}

#[test]
fn plan_command_local_shadow_helpers_accept_trait_object_source() {
    let mut source = FakePlanCommandLocalShadowSource {
        shadow_index: JsonMap::from_iter([(
            "PLAN-1".to_string(),
            json!({
                "plan_id": "PLAN-1",
                "unpublished_head": true,
            }),
        )]),
        ..Default::default()
    };
    let source_port: &mut dyn PlanCommandLocalShadowSource = &mut source;

    let shadow =
        local_shadow_for_plan_with_plan_command_local_shadow_source(source_port, "PLAN-1").unwrap();

    assert_eq!(shadow["plan_id"], "PLAN-1");
    assert_eq!(shadow["unpublished_head"], true);
    assert_eq!(source.calls, vec!["local_shadow_index"]);

    let source_port: &mut dyn PlanCommandLocalShadowSource = &mut source;
    let index = local_shadow_index_with_plan_command_local_shadow_source(source_port).unwrap();
    assert!(index.contains_key("PLAN-1"));
    assert_eq!(
        source.calls,
        vec!["local_shadow_index", "local_shadow_index"]
    );
}

#[test]
fn local_plan_storage_requires_explicit_binary_config() {
    let request = json!({
        "scope": "local",
        "repo_name": "ait",
    });
    let request = request.as_object().unwrap().clone();

    let error = local_plan_binary_storage(&request).expect_err("Binary DB config is required");
    assert!(error.contains("requires a Binary DB `plan_storage` object"));
}

#[test]
fn local_plan_storage_requires_u32_layout() {
    let (_store, temp_dir) = new_binary_plan_command_fixture();
    let request = json!({
        "scope": "local",
        "repo_name": "ait",
        "plan_storage": {
            "write_layout": 1,
            "authority_root": temp_dir.path().to_string_lossy(),
            "repo_root": temp_dir.path().to_string_lossy(),
            "local_authority_id": "local:ait",
            "current_line_state_scope": "repository",
        },
    });
    let request = request.as_object().unwrap().clone();

    assert_eq!(local_plan_binary_storage(&request).unwrap().write_layout, 1);

    let request = json!({
        "scope": "local",
        "repo_name": "ait",
        "plan_storage": {},
    });
    let request = request.as_object().unwrap().clone();
    let err = local_plan_binary_storage(&request).expect_err("layout is required");
    assert!(err.contains("plan_storage.write_layout"));
}

#[test]
fn local_plan_command_rejects_retired_storage_mode() {
    let err = execute_plan_list_command_request_json(
        &json!({
            "scope": "local",
            "repo_name": "ait",
            "plan_storage": {
                "mode": "compare_read",
                "write_layout": 1,
            },
        })
        .to_string(),
    )
    .expect_err("unsupported selector must fail closed");

    assert!(err.contains("does not support plan_storage field"));
}

#[test]
fn local_plan_command_binary_storage_reads_fixture() {
    let (_store, temp_dir) = new_binary_plan_command_fixture();
    let request = binary_plan_command_request(&temp_dir);

    let list_payload = execute_plan_list_command_request_json(&request.to_string()).unwrap();
    assert_eq!(list_payload[0]["plan_id"], "PR-0");
    assert_eq!(list_payload[0]["title"], "Plan title");
    assert_eq!(list_payload[0]["head_revision_id"], "plan-revision:0");

    let mut show_request = request.clone();
    show_request
        .as_object_mut()
        .unwrap()
        .insert("plan_id".to_string(), JsonValue::String("PR-0".to_string()));
    show_request
        .as_object_mut()
        .unwrap()
        .insert("revision".to_string(), JsonValue::Null);
    let show_payload = execute_plan_show_command_request_json(&show_request.to_string()).unwrap();
    assert_eq!(show_payload["plan_id"], "PR-0");
    assert_eq!(
        show_payload["head_revision"]["plan_revision_id"],
        "plan-revision:0"
    );

    let revisions_payload =
        execute_plan_revisions_command_request_json(&show_request.to_string()).unwrap();
    assert_eq!(revisions_payload[0]["plan_revision_id"], "plan-revision:0");
    assert_eq!(revisions_payload[0]["artifact_path"], "docs/plan.md");

    show_request.as_object_mut().unwrap().insert(
        "revision".to_string(),
        JsonValue::String("plan-revision:0".to_string()),
    );
    let items_payload = execute_plan_items_command_request_json(&show_request.to_string()).unwrap();
    assert_eq!(items_payload["plan_id"], "PR-0");
    assert_eq!(items_payload["items"][0]["plan_item_ref"], "item-a");
    assert_eq!(items_payload["items"][0]["heading_path"][0], "Plan");

    let inspect_payload =
        execute_plan_inspect_command_request_json(&show_request.to_string()).unwrap();
    assert_eq!(inspect_payload["plan"]["plan_id"], "PR-0");

    let shadow =
        with_plan_command_local_shadow_source(show_request.as_object().unwrap(), |source| {
            source.local_shadow_index()
        })
        .unwrap();
    assert!(shadow.contains_key("PR-0"));
}

#[test]
fn local_plan_command_binary_storage_resolves_scan_refs_and_rejects_legacy_ids() {
    let (_store, temp_dir) = new_binary_plan_command_fixture();
    let request = binary_plan_command_request(&temp_dir);

    let mut show_by_artifact = request.clone();
    show_by_artifact.as_object_mut().unwrap().insert(
        "plan_id".to_string(),
        JsonValue::String("artifact:docs/plan.md".to_string()),
    );
    show_by_artifact
        .as_object_mut()
        .unwrap()
        .insert("revision".to_string(), JsonValue::Null);
    let artifact_payload =
        execute_plan_show_command_request_json(&show_by_artifact.to_string()).unwrap();
    assert_eq!(artifact_payload["plan_id"], "PR-0");

    let mut show_by_bare_path = request.clone();
    show_by_bare_path.as_object_mut().unwrap().insert(
        "plan_id".to_string(),
        JsonValue::String("docs/plan.md".to_string()),
    );
    show_by_bare_path
        .as_object_mut()
        .unwrap()
        .insert("revision".to_string(), JsonValue::Null);
    let bare_path_payload =
        execute_plan_show_command_request_json(&show_by_bare_path.to_string()).unwrap();
    assert_eq!(bare_path_payload["plan_id"], "PR-0");

    let mut revisions_by_title = request.clone();
    revisions_by_title.as_object_mut().unwrap().insert(
        "plan_id".to_string(),
        JsonValue::String("title:Plan title".to_string()),
    );
    let revisions_payload =
        execute_plan_revisions_command_request_json(&revisions_by_title.to_string()).unwrap();
    assert_eq!(revisions_payload[0]["plan_revision_id"], "plan-revision:0");

    let mut items_by_mapping = request.clone();
    items_by_mapping.as_object_mut().unwrap().insert(
        "plan_id".to_string(),
        JsonValue::String("published-plan:6".to_string()),
    );
    items_by_mapping.as_object_mut().unwrap().insert(
        "revision".to_string(),
        JsonValue::String("revision-number:1".to_string()),
    );
    let items_payload =
        execute_plan_items_command_request_json(&items_by_mapping.to_string()).unwrap();
    assert_eq!(items_payload["plan_id"], "PR-0");
    assert_eq!(items_payload["items"][0]["plan_item_ref"], "item-a");

    items_by_mapping.as_object_mut().unwrap().insert(
        "revision".to_string(),
        JsonValue::String("published-revision:8".to_string()),
    );
    let published_revision_payload =
        execute_plan_items_command_request_json(&items_by_mapping.to_string()).unwrap();
    assert_eq!(
        published_revision_payload["plan_revision_id"],
        "plan-revision:0"
    );

    let mut indexed_plan = request.clone();
    indexed_plan.as_object_mut().unwrap().insert(
        "plan_id".to_string(),
        JsonValue::String("CPL-COMMAND-1".to_string()),
    );
    indexed_plan
        .as_object_mut()
        .unwrap()
        .insert("revision".to_string(), JsonValue::Null);
    let indexed_plan_err =
        execute_plan_show_command_request_json(&indexed_plan.to_string()).unwrap_err();
    assert!(indexed_plan_err.contains("is not canonical"));

    let mut indexed_revision = request.clone();
    indexed_revision
        .as_object_mut()
        .unwrap()
        .insert("plan_id".to_string(), JsonValue::String("PR-0".to_string()));
    indexed_revision.as_object_mut().unwrap().insert(
        "revision".to_string(),
        JsonValue::String("CPR-COMMAND-1".to_string()),
    );
    let indexed_revision_err =
        execute_plan_items_command_request_json(&indexed_revision.to_string()).unwrap_err();
    assert!(indexed_revision_err.contains("is not canonical"));

    indexed_plan.as_object_mut().unwrap().insert(
        "plan_id".to_string(),
        JsonValue::String("plan:0".to_string()),
    );
    let legacy_plan_err =
        execute_plan_show_command_request_json(&indexed_plan.to_string()).unwrap_err();
    assert!(legacy_plan_err.contains("use `PR-<plan.bin ordinal>`"));
}

#[test]
fn local_plan_command_binary_storage_missing_files_returns_binary_db_error() {
    let temp_dir = tempdir().expect("tempdir");
    fs::create_dir_all(temp_dir.path().join(".ait/binary-db"))
        .expect("create empty Binary DB authority");
    fs::create_dir_all(temp_dir.path().join(".ait/objects")).expect("create empty pack authority");
    fs::write(
        temp_dir.path().join(".ait/config.json"),
        br#"{"repo_name":"ait-core"}"#,
    )
    .expect("write repository config");
    let request = binary_plan_command_request(&temp_dir);

    let err = execute_plan_list_command_request_json(&request.to_string())
        .expect_err("missing Binary DB files must fail closed");
    assert!(err.contains("plan.bin"));
}

#[test]
fn plan_command_list_uses_data_source_rows() {
    let mut source = FakePlanCommandSource::with_plan("PLAN-1", "remote-repo");

    let payload = execute_plan_list_from_source(
        &mut source,
        "remote",
        "request-repo",
        JsonValue::String("origin".to_string()),
    )
    .unwrap();

    assert_eq!(
        payload,
        json!([{"plan_id": "PLAN-1", "repo_name": "remote-repo", "title": "Plan title", "status": "active"}])
    );
    assert_eq!(source.calls, vec!["list_plans:request-repo"]);
}

#[test]
fn plan_command_show_and_items_fetch_plan_and_optional_revision_through_trait() {
    let mut show_source = FakePlanCommandSource::with_plan("PLAN-1", "remote-repo");
    let show_payload = execute_plan_show_from_source(
        &mut show_source,
        "remote",
        "request-repo",
        "PLAN-1",
        Some("REV-1"),
        JsonValue::String("origin".to_string()),
    )
    .unwrap();
    assert_eq!(show_payload["plan"]["plan_id"], "PLAN-1");
    assert_eq!(show_payload["revision"]["plan_revision_id"], "REV-1");
    assert_eq!(
        show_source.calls,
        vec!["get_plan:PLAN-1", "get_plan_revision:PLAN-1:REV-1"]
    );

    let mut items_source = FakePlanCommandSource::with_plan("PLAN-1", "remote-repo");
    let items_payload = execute_plan_items_from_source(
        &mut items_source,
        "remote",
        "request-repo",
        "PLAN-1",
        Some("REV-1"),
        JsonValue::String("origin".to_string()),
    )
    .unwrap();
    assert_eq!(items_payload["plan_id"], "PLAN-1");
    assert_eq!(items_payload["items"][0]["plan_item_ref"], "item-a");
    assert_eq!(
        items_source.calls,
        vec!["get_plan:PLAN-1", "get_plan_revision:PLAN-1:REV-1"]
    );
}

#[test]
fn plan_command_show_and_items_accept_plan_revision_reader_only_source() {
    struct PlanRevisionReaderOnly;

    impl PlanCommandPlanReader for PlanRevisionReaderOnly {
        fn get_plan(&mut self, plan_id: &str) -> Result<JsonValue, String> {
            Ok(json!({
                "plan_id": plan_id,
                "repo_name": "reader-only-repo",
                "title": "Readable plan",
                "status": "active",
                "head_revision": {
                    "plan_revision_id": "REV-ONLY",
                    "items": []
                }
            }))
        }
    }

    impl PlanCommandRevisionReader for PlanRevisionReaderOnly {
        fn get_plan_revision(
            &mut self,
            plan_id: &str,
            plan_revision_id: &str,
        ) -> Result<JsonValue, String> {
            Ok(json!({
                "plan_id": plan_id,
                "plan_revision_id": plan_revision_id,
                "items": [
                    {
                        "plan_item_ref": "item-reader",
                        "text": "Read through the narrow port",
                        "checkbox_state": "open"
                    }
                ]
            }))
        }
    }

    let mut show_source = PlanRevisionReaderOnly;
    let show_payload = execute_plan_show_from_source(
        &mut show_source,
        "remote",
        "request-repo",
        "PLAN-READ",
        Some("REV-ONLY"),
        JsonValue::String("origin".to_string()),
    )
    .unwrap();
    assert_eq!(show_payload["plan"]["repo_name"], "reader-only-repo");
    assert_eq!(show_payload["revision"]["plan_revision_id"], "REV-ONLY");

    let mut items_source = PlanRevisionReaderOnly;
    let items_payload = execute_plan_items_from_source(
        &mut items_source,
        "remote",
        "request-repo",
        "PLAN-READ",
        Some("REV-ONLY"),
        JsonValue::String("origin".to_string()),
    )
    .unwrap();
    assert_eq!(items_payload["items"][0]["plan_item_ref"], "item-reader");
}

#[test]
fn plan_command_revisions_uses_revision_listing_trait_method() {
    let mut source = FakePlanCommandSource::with_plan("PLAN-1", "remote-repo");

    let payload = execute_plan_revisions_from_source(
        &mut source,
        "remote",
        "request-repo",
        "PLAN-1",
        JsonValue::String("origin".to_string()),
    )
    .unwrap();

    assert_eq!(payload[0]["plan_revision_id"], "REV-1");
    assert_eq!(source.calls, vec!["list_plan_revisions:PLAN-1"]);
}

#[test]
fn plan_command_candidates_uses_candidate_inputs_and_shadow_index() {
    let mut source = FakePlanCommandSource::with_plan("PLAN-1", "remote-repo");
    let mut shadow_index = JsonMap::new();
    shadow_index.insert(
        "PLAN-1".to_string(),
        json!({
            "plan_id": "PLAN-1",
            "publication_state": "published",
            "head_publication_state": "draft",
            "head_revision_id": "REV-1",
            "head_revision_number": 1,
            "published_plan_id": "REMOTE-PLAN-1",
            "published_head_revision_id": "REMOTE-REV-1",
            "unpublished_head": true,
        }),
    );

    let contains_terms = vec!["item-a".to_string()];
    let payload = execute_plan_candidates_from_source(
        &mut source,
        "remote",
        "request-repo",
        JsonValue::String("origin".to_string()),
        true,
        &contains_terms,
        shadow_index,
    )
    .unwrap();

    assert_eq!(payload["scope"], "remote");
    assert_eq!(payload["remote"], "origin");
    assert_eq!(payload["candidates"][0]["plan_id"], "PLAN-1");
    assert_eq!(payload["candidates"][0]["local_unpublished_head"], true);
    assert_eq!(source.calls, vec!["candidate_inputs:request-repo:item-a"]);
}

#[test]
fn plan_command_inspect_fetches_plan_revision_tasks_and_local_shadow() {
    let mut source = FakePlanCommandSource::with_plan("PLAN-1", "remote-repo");
    let local_shadow = json!({
        "plan_id": "PLAN-1",
        "publication_state": "published",
        "head_publication_state": "draft",
        "head_revision_id": "REV-1",
        "head_revision_number": 1,
        "published_plan_id": "REMOTE-PLAN-1",
        "published_head_revision_id": "REMOTE-REV-1",
        "unpublished_head": true,
    });

    let payload = execute_plan_inspect_from_source(
        &mut source,
        "remote",
        "request-repo",
        "PLAN-1",
        Some("REV-1"),
        JsonValue::String("origin".to_string()),
        local_shadow,
    )
    .unwrap();

    assert_eq!(payload["scope"], "remote");
    assert_eq!(payload["repo_name"], "remote-repo");
    assert_eq!(payload["plan"]["plan_id"], "PLAN-1");
    assert_eq!(payload["plan"]["local_unpublished_head"], true);
    assert_eq!(
        source.calls,
        vec![
            "get_plan:PLAN-1",
            "get_plan_revision:PLAN-1:REV-1",
            "list_tasks:request-repo"
        ]
    );
}
