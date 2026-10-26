use super::*;

const CONTRACT: &str = "ait.snapshot_capture_binding.v2";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRequest {
    contract: String,
    repository_index: u32,
    snapshot_id: String,
    task_id: String,
    change_ref: Option<String>,
    line_name: String,
}

fn task_capture_line(task: &str, line: &str) -> bool {
    let slug = task.to_ascii_lowercase();
    line == format!("feature/{slug}") || line == format!("feature/task-{slug}")
}

impl<D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone> BinaryDbServerWorkflowV0Store<D> {
    /// Resolve the fixed owner; parent, Line and ordinal equality are not ownership.
    pub fn snapshot_workflow_owner(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(
            &self.db,
            crate::foundation::remote_binary_db::BinaryDbReadScope::CONTENT
                .union(crate::foundation::remote_binary_db::BinaryDbReadScope::WORKFLOW),
        );
        let snapshots = ServerBinaryDbSnapshotStore::<D, 1>::new(self.db.clone());
        let (snapshot_index, _) = snapshots
            .snapshot_by_id(&read, snapshot_id)
            .map_err(|error| error.to_string())?
            .ok_or("Snapshot owner lookup requires an existing Snapshot")?;
        Ok(
            match self.numeric_snapshot_owner_with_access(&read, snapshot_index)? {
                None => JsonValue::Null,
                Some(owner) => json!({"task_id":self.task_id(owner.task_index),
                "change_ref":owner.change.map(|(_,change)| self.change_ref(owner.task_index, change.change_ordinal))}),
            },
        )
    }

    pub(crate) fn require_fresh_task_capture_binding<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        snapshot_id: &str,
        line_name: &str,
        bound_snapshot_id: Option<&str>,
    ) -> Result<(), String> {
        if bound_snapshot_id == Some(snapshot_id)
            || ServerBinaryDbSnapshotStore::<D, 1>::new(self.db.clone())
                .snapshot_by_id_in_write(tx, snapshot_id)
                .map_err(|error| error.to_string())?
                .is_some()
        {
            return Ok(());
        }
        let Some(slug) = line_name.strip_prefix("feature/") else {
            return Ok(());
        };
        let task_id = slug
            .strip_prefix("task-")
            .unwrap_or(slug)
            .to_ascii_uppercase();
        let Ok(index) = self.parse_task_index(&task_id) else {
            return Ok(());
        };
        let count = tx
            .record_count(WorkflowBinaryV0Codec::task_file())
            .map_err(|error| error.to_string())?;
        if index >= count {
            return Ok(());
        }
        let task = self
            .read_task(tx, index)
            .map_err(|error| error.to_string())?;
        if task.remote_meta & 1 == 0 && task_capture_line(&task_id, line_name) {
            return Err("Fresh Task Snapshot requires its exact capture_binding".into());
        }
        Ok(())
    }

    pub(super) fn immutable_snapshot_line<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
    ) -> Result<(ServerBinarySnapshotRecord, String), String> {
        let record = ServerBinarySnapshotCodec::<2>::decode_record(
            &read
                .read_record_v0(ServerBinarySnapshotCodec::<2>::record_file(), index)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if record.is_tombstone() {
            return Err("Snapshot content is retired".to_string());
        }
        let payload = ServerBinarySnapshotCodec::<2>::decode_payload(
            &read
                .read_payload_v0(
                    ServerBinarySnapshotCodec::<2>::payload_file(),
                    record.payload_offset,
                    u32::from(record.payload_len),
                )
                .map_err(|e| e.to_string())?,
            record.snapshot_meta & ServerBinarySnapshotRecord::META_HAS_LINE_NAME_PAYLOAD != 0,
        )
        .map_err(|e| e.to_string())?;
        Ok((record, payload.line_name))
    }

    fn capture_value(
        &self,
        snapshot_id: &str,
        owner: WorkflowOwner,
        line: &str,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "contract": CONTRACT,
            "repository_index": self.repository_index.ok_or("Capture requires numeric Repository authority")?,
            "snapshot_id": snapshot_id, "task_id": self.task_id(owner.task_index),
            "change_ref": owner.change.map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal)),
            "line_name": line,
        }))
    }

    pub(super) fn numeric_snapshot_owner_with_access<A: ReadV0>(
        &self,
        read: &A,
        snapshot_index: u32,
    ) -> Result<Option<WorkflowOwner>, String> {
        let mut found = None;
        for index in 0..read
            .record_count_v0(WorkflowBinaryV0Codec::snapshot_link_file())
            .map_err(|e| e.to_string())?
        {
            let link = WorkflowBinaryV0Codec::decode_snapshot_link(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::snapshot_link_file(), index)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if link.content_snapshot_index != snapshot_index {
                continue;
            }
            if found.is_some() || link.link_meta & SNAPSHOT_LINK_TOMBSTONE != 0 {
                return Err("Snapshot has duplicate or retired numeric ownership".to_string());
            }
            let task_id = self.task_id(link.task_index);
            let owner = if let Some(change_index) = link.change_index_plus1.checked_sub(1) {
                let change = self
                    .read_change(read, change_index)
                    .map_err(|e| e.to_string())?;
                if change.task_index != link.task_index || change.remote_meta & 1 != 0 {
                    return Err("Snapshot Change belongs to another Task or is retired".to_string());
                }
                WorkflowOwner {
                    task_index: link.task_index,
                    change: Some((change_index, change)),
                }
            } else {
                self.workflow_owner(read, &task_id)?
            };
            self.task_index_for_id(read, &task_id)?;
            let (snapshot, _) = self.immutable_snapshot_line(read, snapshot_index)?;
            if link.created_at_s != snapshot.created_at_s {
                return Err("Snapshot Link time disagrees with immutable content".to_string());
            }
            found = Some(owner);
        }
        Ok(found)
    }

    pub(super) fn capture_binding_with_access<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
    ) -> Result<Option<JsonValue>, String> {
        self.numeric_snapshot_owner_with_access(read, index)?
            .map(|owner| {
                let (_, line) = self.immutable_snapshot_line(read, index)?;
                self.capture_value(&self.content_snapshot_id(read, index)?, owner, &line)
            })
            .transpose()
    }

    pub(crate) fn capture_binding_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        snapshot_index: u32,
    ) -> Result<Option<JsonValue>, String> {
        self.capture_binding_with_access(read, snapshot_index)
    }

    pub(super) fn require_patchset_capture_range<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        owner: WorkflowOwner,
        base: u32,
        revision: u32,
    ) -> Result<(), String>
    where
        D: Send + Sync + 'static,
    {
        if self.repository_index.is_none() || !self.task_prefix.starts_with('R') {
            return Ok(());
        }
        let task_id = self.task_id(owner.task_index);
        let change_ref = owner
            .change
            .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal));
        let dag = self.load_history_snapshot_dag(tx)?;
        let base_scope = self.history_snapshot_ancestor_closure(tx, &dag, base)?;
        let revision_scope = self.history_snapshot_ancestor_closure(tx, &dag, revision)?;
        if !revision_scope.contains(&base) {
            return Err("Patchset capture revision must descend from its base".to_string());
        }
        let fork_scope = owner
            .change
            .map(|(_, change)| {
                let fork = change
                    .fork_snapshot_index_plus1
                    .checked_sub(1)
                    .ok_or("Explicit revision requires its exact reviewed fork Snapshot")?;
                let scope = self.history_snapshot_ancestor_closure(tx, &dag, fork)?;
                if !revision_scope.contains(&fork) || !scope.contains(&base) {
                    return Err(
                        "Revision fork is outside the exact candidate base/revision DAG"
                            .to_string(),
                    );
                }
                Ok::<_, String>(scope)
            })
            .transpose()?;
        let mut authoring_line = None;
        for index in revision_scope.difference(&base_scope) {
            let binding = self
                .capture_binding_with_access(tx, *index)?
                .ok_or("Patchset Snapshot is missing durable numeric ownership")?;
            if binding["task_id"].as_str() != Some(task_id.as_str())
                || !fork_scope
                    .as_ref()
                    .is_some_and(|scope| scope.contains(index))
                    && binding["change_ref"].as_str() != change_ref.as_deref()
            {
                return Err(
                    "Patchset Snapshot is not owned by this exact Task/optional Change".to_string(),
                );
            }
            let line = binding["line_name"]
                .as_str()
                .ok_or("Snapshot authoring Line is missing")?
                .to_string();
            if authoring_line.as_ref().is_some_and(|prior| prior != &line) {
                return Err("Patchset capture range spans different authoring Lines".to_string());
            }
            authoring_line = Some(line);
        }
        Ok(())
    }

    pub(super) fn append_numeric_snapshot_link<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        owner: WorkflowOwner,
        snapshot_index: u32,
        actor_index_plus1: u32,
    ) -> Result<(), String> {
        let (snapshot, _) = self.immutable_snapshot_line(tx, snapshot_index)?;
        let mut ordinals = BTreeSet::new();
        let mut latest = 0;
        for index in 0..tx
            .record_count(WorkflowBinaryV0Codec::snapshot_link_file())
            .map_err(|e| e.to_string())?
        {
            let link = WorkflowBinaryV0Codec::decode_snapshot_link(
                &tx.read_record(WorkflowBinaryV0Codec::snapshot_link_file(), index)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if link.content_snapshot_index == snapshot_index {
                return Err("Snapshot already has numeric ownership".to_string());
            }
            if link.task_index == owner.task_index {
                if !ordinals.insert(link.snapshot_ordinal) {
                    return Err("Task Snapshot ordinals are duplicated".to_string());
                }
                latest = index + 1;
            }
        }
        let mut task_index = WorkflowBinaryV0Codec::decode_task_snapshot_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
                owner.task_index,
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if ordinals.len() != usize::from(task_index.count)
            || latest != task_index.latest_index_plus1
            || ordinals
                .iter()
                .copied()
                .ne(0..=ordinals.last().copied().unwrap_or(0))
                && !ordinals.is_empty()
        {
            return Err("Task Snapshot index disagrees with allocated ordinals".to_string());
        }
        let next = ordinals.last().map_or(0_u16, |n| u16::from(*n) + 1);
        let ordinal =
            u8::try_from(next).map_err(|_| "Task has exhausted its 256 Snapshot links")?;
        let link = V0SnapshotLinkRecord {
            link_meta: if owner.change.is_some() {
                SNAPSHOT_LINK_HAS_CHANGE
            } else {
                0
            },
            snapshot_ordinal: ordinal,
            task_index: owner.task_index,
            change_index_plus1: owner.change.map_or(0, |(index, _)| index + 1),
            content_snapshot_index: snapshot_index,
            created_at_s: snapshot.created_at_s,
        };
        let index = tx
            .append_record(
                WorkflowBinaryV0Codec::snapshot_link_file(),
                &WorkflowBinaryV0Codec::encode_snapshot_link(link).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        task_index.latest_index_plus1 = index + 1;
        task_index.count = task_index
            .count
            .checked_add(1)
            .ok_or("Task Snapshot count overflow")?;
        task_index.next_ordinal = u8::try_from(next + 1).unwrap_or(u8::MAX);
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
            owner.task_index,
            &WorkflowBinaryV0Codec::encode_task_snapshot_index(task_index)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if let Some((change_index, _)) = owner.change {
            let mut index_record =
                self.task_inventory(tx, "change_snapshot_index.bin", change_index)?;
            index_record.latest_index_plus1 = index + 1;
            index_record.count = index_record
                .count
                .checked_add(1)
                .ok_or("Change Snapshot count overflow")?;
            tx.overwrite_record(
                WorkflowBinaryV0Codec::chain_index_file("change_snapshot_index.bin"),
                change_index,
                &WorkflowBinaryV0Codec::encode_inventory_index(index_record)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        self.update_owner_state(
            tx,
            owner,
            TASK_META_SNAPSHOTTED,
            0,
            snapshot.created_at_s,
            (actor_index_plus1 != 0).then_some(actor_index_plus1),
            |_| {},
        )?;
        Ok(())
    }

    pub(crate) fn record_snapshot_capture_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        request: &JsonValue,
        was_existing: bool,
        actor_index_plus1: u32,
    ) -> Result<(), String> {
        let request: CaptureRequest = serde_json::from_value(request.clone())
            .map_err(|e| format!("Invalid capture binding v2: {e}"))?;
        if request.contract != CONTRACT || Some(request.repository_index) != self.repository_index {
            return Err("Snapshot capture contract or Repository authority mismatch".to_string());
        }
        let task_index = self.task_index_for_id(tx, &request.task_id)?;
        if request
            .change_ref
            .as_ref()
            .is_some_and(|reference| !reference.contains("/C-"))
        {
            return Err(
                "Non-null capture Change must be an exact real Change reference".to_string(),
            );
        }
        let owner = self.workflow_owner(
            tx,
            request.change_ref.as_deref().unwrap_or(&request.task_id),
        )?;
        if owner.task_index != task_index {
            return Err("Snapshot capture Change belongs to a different Task".to_string());
        }
        let (index, snapshot) = ServerBinaryDbSnapshotStore::<D, 1>::new(self.db.clone())
            .snapshot_by_id_in_write(tx, &request.snapshot_id)
            .map_err(|e| e.to_string())?
            .ok_or("Capture Snapshot content is missing")?;
        let (_, line) = self.immutable_snapshot_line(tx, index)?;
        if let Some(existing) = self.capture_binding_with_access(tx, index)? {
            return if existing
                == self.capture_value(&request.snapshot_id, owner, &request.line_name)?
            {
                Ok(())
            } else {
                Err("Snapshot capture ownership is immutable".to_string())
            };
        }
        if was_existing {
            return Err("Existing unbound Snapshot cannot acquire capture ownership".to_string());
        }
        self.require_active_owner(tx, owner)?;
        if request.line_name != line
            || !task_capture_line(&request.task_id, &line)
            || snapshot.snapshot_meta & ServerBinarySnapshotRecord::META_KIND_MASK != 0
        {
            return Err("Snapshot capture Task/Line differs from immutable content".to_string());
        }
        self.append_numeric_snapshot_link(tx, owner, index, actor_index_plus1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::server_content_binary_db::{
        server_snapshot_hash48_from_id, ServerBinarySnapshotPayload,
    };
    use crate::foundation::workflow_binary_v0_adapter::tests::initialized_db;

    #[test]
    fn numeric_task_only_link_is_readable_and_ownership_cannot_be_changed() {
        let db = initialized_db("capture-legacy-task-only");
        ServerBinaryDbLineStore::<_, 1>::new(db.clone())
            .create_line("main", 0, 1)
            .unwrap();
        let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "CP")
            .unwrap()
            .with_repository_index(9);
        store
            .create_task(
                "repo",
                &json!({"actor_key_index":0,"title":"Legacy","intent":"Preserve valid Task-only history"}),
            )
            .unwrap();
        store
            .create_change(
                "repo",
                &json!({"actor_key_index":0,"task_id":"RCPT-0001","title":"New work","base_line":"main"}),
            )
            .unwrap();
        let id = "SNP-000000000001";
        let content = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
        content
            .append_snapshot(
                id,
                ServerBinarySnapshotRecord {
                    actor_index_plus1: 0,
                    snapshot_meta: 0,
                    history_flags: 0,
                    payload_len: 0,
                    payload_offset: 0,
                    snapshot_hash48: 1,
                    parent_snapshot_index_plus1: 0,
                    root_tree_pack_index_plus1: 0,
                    root_entry_ordinal: 0,
                    line_index_plus1: 0,
                    manifest_hash: [0; 32],
                    file_count: 0,
                    total_bytes: 0,
                    created_at_s: 1,
                },
                &ServerBinarySnapshotPayload {
                    line_name: "feature/rcpt-0001".to_string(),
                    message: None,
                },
            )
            .unwrap();
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
        tx.append_record(
            WorkflowBinaryV0Codec::snapshot_link_file(),
            &WorkflowBinaryV0Codec::encode_snapshot_link(V0SnapshotLinkRecord {
                link_meta: 0,
                snapshot_ordinal: 0,
                task_index: 0,
                change_index_plus1: 0,
                content_snapshot_index: 0,
                created_at_s: 1,
            })
            .unwrap(),
        )
        .unwrap();
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
            0,
            &WorkflowBinaryV0Codec::encode_task_snapshot_index(V0OrdinalIndexRecord {
                latest_index_plus1: 1,
                count: 1,
                next_ordinal: 1,
                reserved0: 0,
            })
            .unwrap(),
        )
        .unwrap();
        tx.commit().unwrap();
        drop(tx);
        validate_frozen_server_workflow_v0(&db).unwrap();
        assert!(store
            .capture_binding_with_read(&BinaryDbReadTxn::new(&db), 0)
            .unwrap()
            .is_some());
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerSnapshotCapture)
                .unwrap();
        let error = store.record_snapshot_capture_in_write(&mut tx,&json!({
            "contract":CONTRACT,"repository_index":9,"snapshot_id":id,"task_id":"RCPT-0001","change_ref":"RCPT-0001/C-01",
            "line_name":"feature/rcpt-0001"
        }),true, 1).unwrap_err();
        assert!(error.contains("immutable"));
        drop(tx);
        validate_frozen_server_workflow_v0(&db).unwrap();
    }

    #[test]
    fn capture_uses_all_256_task_ordinals_and_rejects_the_next_without_writes() {
        let db = initialized_db("capture-256");
        ServerBinaryDbLineStore::<_, 1>::new(db.clone())
            .create_line("main", 0, 1)
            .unwrap();
        let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "CP")
            .unwrap()
            .with_repository_index(9);
        store.create_task("repo", &json!({"actor_key_index":0,"title":"Capture capacity","intent":"Use existing full u8 Snapshot ordinals"})).unwrap();
        store
            .create_change(
                "repo",
                &json!({"actor_key_index":0,"task_id":"RCPT-0001","title":"Capture","base_line":"main"}),
            )
            .unwrap();
        let content = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
        for i in 0..257_u32 {
            let id = format!("SNP-{:012X}", i + 1);
            let mut tx =
                BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerSnapshotCapture)
                    .unwrap();
            let record = ServerBinarySnapshotRecord {
                actor_index_plus1: 0,
                snapshot_meta: 0,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: server_snapshot_hash48_from_id(&id).unwrap(),
                parent_snapshot_index_plus1: i,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: 0,
                manifest_hash: [0; 32],
                file_count: 0,
                total_bytes: 0,
                created_at_s: u64::from(i) + 1,
            };
            // The legacy Task Line remains a valid authoring identity.
            content
                .append_snapshot_in_tx(
                    &mut tx,
                    &id,
                    record,
                    &ServerBinarySnapshotPayload {
                        line_name: "feature/task-rcpt-0001".to_string(),
                        message: None,
                    },
                )
                .unwrap();
            let result = store.record_snapshot_capture_in_write(
                &mut tx,
                &json!({
                    "contract":CONTRACT,"repository_index":9,"snapshot_id":id,"task_id":"RCPT-0001",
                    "change_ref":"RCPT-0001/C-01","line_name":"feature/task-rcpt-0001"
                }),
                false,
                1,
            );
            if i == 256 {
                assert!(result.unwrap_err().contains("256 Snapshot"));
                drop(tx);
                break;
            }
            result.unwrap();
            tx.commit().unwrap();
        }
        validate_frozen_server_workflow_v0(&db).unwrap();
        let read = BinaryDbReadTxn::new(&db);
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::snapshot_link_file())
                .unwrap(),
            256
        );
        assert_eq!(
            read.record_count(ServerBinarySnapshotCodec::<2>::record_file())
                .unwrap(),
            256
        );
        let index = WorkflowBinaryV0Codec::decode_task_snapshot_index(
            &read
                .read_record(
                    WorkflowBinaryV0Codec::chain_index_file("task_snapshot_index.bin"),
                    0,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(index.count, 256);
        assert_eq!(index.next_ordinal, 255);
        let reopened = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "CP")
            .unwrap()
            .with_repository_index(9);
        let binding = reopened
            .capture_binding_with_read(&read, 255)
            .unwrap()
            .unwrap();
        assert_eq!(binding["change_ref"], "RCPT-0001/C-01");
        assert_eq!(binding["line_name"], "feature/task-rcpt-0001");
    }
}
