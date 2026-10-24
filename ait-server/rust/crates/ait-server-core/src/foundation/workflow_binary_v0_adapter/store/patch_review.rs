use super::*;

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn task_inventory_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        path: &'static str,
        task_index: u32,
    ) -> Result<V0InventoryIndexRecord, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        WorkflowBinaryV0Codec::decode_inventory_index(
            &tx.read_record(WorkflowBinaryV0Codec::chain_index_file(path), task_index)
                .map_err(|error| Self::error("Task inventory read", error))?,
        )
        .map_err(|error| Self::error("Task inventory decode", error))
    }

    pub(super) fn append_patchset_index_rows<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        expected_patchset_index: u32,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let inventory =
            WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord::default())?;
        let ordinal = WorkflowBinaryV0Codec::encode_ordinal_index(V0OrdinalIndexRecord::default())?;
        for (path, raw) in [
            ("patchset_attest_index.bin", inventory.as_slice()),
            ("patchset_review_index.bin", ordinal.as_slice()),
            ("patchset_policy_index.bin", ordinal.as_slice()),
            ("patchset_waiver_index.bin", inventory.as_slice()),
        ] {
            let index = tx.append_record(WorkflowBinaryV0Codec::chain_index_file(path), raw)?;
            if index != expected_patchset_index {
                return Err(BinaryDbError::corruption(format!(
                    "{path} append index {index} disagrees with Patchset index {expected_patchset_index}"
                )));
            }
        }
        Ok(())
    }

    pub(super) fn author_mode_bits(value: &str) -> Result<u8, String> {
        let kind = match value {
            "human_only" => 0,
            "human_with_ai_assist" => 1,
            "ai_with_human_review" => 2,
            "ai_only_experimental" => 3,
            _ => {
                return Err(format!(
                    "native Binary DB v0 Patchset author_mode {value:?} is not supported"
                ))
            }
        };
        Ok(kind << 2)
    }

    pub(super) fn owner_for_patchset_index<A: ReadV0>(
        &self,
        read: &A,
        patchset_index: u32,
    ) -> Result<(WorkflowOwner, V0PatchsetRecord), String> {
        let patchset = self
            .read_patchset(read, patchset_index)
            .map_err(|e| e.to_string())?;
        Ok((self.patchset_owner(read, patchset)?, patchset))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn append_candidate_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        owner: WorkflowOwner,
        base: u32,
        revision: u32,
        summary: &str,
        author_mode: &str,
        now: u64,
        actor_index_plus1: u32,
    ) -> Result<u32, String> {
        self.require_active_owner(tx, owner)?;
        let ordinal = self.next_task_patchset_ordinal(tx, owner.task_index)?;
        let mut inventory = self.task_inventory(tx, "task_patchset_index.bin", owner.task_index)?;
        let mut change_inventory = owner
            .change
            .map(|(index, _)| self.owner_ordinal_index(tx, "change_patchset_index.bin", index))
            .transpose()?;
        let bytes = WorkflowBinaryV0Codec::encode_single_text_payload(summary, "Patchset summary")
            .map_err(|e| e.to_string())?;
        let range = tx
            .append_payload(WorkflowBinaryV0Codec::patchset_summary_file(), &bytes)
            .map_err(|e| e.to_string())?;
        self.sync_file(tx, "patchset_summary_payload.bin")
            .map_err(|e| e.to_string())?;
        let record = V0PatchsetRecord {
            patchset_meta: Self::author_mode_bits(author_mode)? | PATCHSET_EVALUATION_PENDING,
            patch_ordinal: ordinal,
            task_index: owner.task_index,
            change_ordinal_plus1: owner.change_ordinal_plus1(),
            previous_task_patchset_index_plus1: inventory.latest_index_plus1,
            previous_change_patchset_index_plus1: change_inventory
                .as_ref()
                .map_or(0, |i| i.latest_index_plus1),
            base_snapshot_index: base,
            revision_snapshot_index: revision,
            created_at_s: now,
            summary_offset: range.payload_offset,
            summary_len: u16::try_from(range.payload_len)
                .map_err(|_| "Patchset summary exceeds u16")?,
            ..V0PatchsetRecord::default()
        };
        let index = tx
            .append_record(
                WorkflowBinaryV0Codec::patchset_file(),
                &self
                    .encode_new_patchset(record)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        self.append_patchset_index_rows(tx, index)
            .map_err(|e| e.to_string())?;
        if let Some(previous) = inventory.latest_index_plus1.checked_sub(1) {
            let mut old = self
                .read_patchset(tx, previous)
                .map_err(|e| e.to_string())?;
            old.patchset_meta = (old.patchset_meta & !PATCHSET_PUBLISH_STATE_MASK) | (2 << 5);
            tx.overwrite_record(
                WorkflowBinaryV0Codec::patchset_file(),
                previous,
                &self
                    .encode_patchset_replacement(tx, previous, old)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        inventory.latest_index_plus1 = index + 1;
        inventory.count = inventory
            .count
            .checked_add(1)
            .ok_or("Task Patchset count exceeds u16")?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_patchset_index.bin"),
            owner.task_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(inventory).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if let (Some((change_index, _)), Some(change_inventory)) =
            (owner.change, change_inventory.as_mut())
        {
            change_inventory.latest_index_plus1 = index + 1;
            change_inventory.count = change_inventory
                .count
                .checked_add(1)
                .ok_or("Change Patchset count exceeds u16")?;
            change_inventory.next_ordinal = ordinal + 1;
            tx.overwrite_record(
                WorkflowBinaryV0Codec::chain_index_file("change_patchset_index.bin"),
                change_index,
                &WorkflowBinaryV0Codec::encode_ordinal_index(*change_inventory)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        self.update_owner_state(
            tx,
            owner,
            TASK_META_REVIEW_PENDING | TASK_META_VALIDATION_PENDING,
            TASK_META_READY_TO_LAND | TASK_META_BLOCKED,
            now,
            Some(actor_index_plus1),
            |change| {
                change.change_meta = (change.change_meta & !CHANGE_META_LIFECYCLE_MASK)
                    | CHANGE_LIFECYCLE_ACTIVE
                    | CHANGE_META_HAS_PATCHSETS
                    | CHANGE_META_REVIEW_PENDING
                    | CHANGE_META_VALIDATION_PENDING;
                change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
            },
        )?;
        Ok(index)
    }

    fn actor_key_hash(user_name: &str) -> u64 {
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in user_name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x00000100000001b3);
        }
        hash
    }

    fn actor_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        actor_index: u32,
    ) -> Result<(V0ActorRecord, V0ActorPayload), String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let record = WorkflowBinaryV0Codec::decode_actor(
            &tx.read_record(WorkflowBinaryV0Codec::actor_file(), actor_index)
                .map_err(|error| Self::error("Actor read", error))?,
        )
        .map_err(|error| Self::error("Actor decode", error))?;
        let raw = tx
            .read_payload(
                WorkflowBinaryV0Codec::actor_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| Self::error("Actor payload read", error))?;
        let payload = WorkflowBinaryV0Codec::decode_actor_payload(&raw)
            .map_err(|error| Self::error("Actor payload decode", error))?;
        Ok((record, payload))
    }

    fn find_or_create_actor<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        user_name: &str,
        actor_kind: u8,
        now: u64,
    ) -> Result<u32, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let payload = V0ActorPayload {
            user_name: user_name.to_string(),
            user_id: String::new(),
            email: String::new(),
            memo: String::new(),
        };
        self.find_or_create_actor_payload(tx, &payload, actor_kind, now)
    }

    pub(super) fn find_or_create_actor_payload<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        payload: &V0ActorPayload,
        actor_kind: u8,
        now: u64,
    ) -> Result<u32, String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let hash = Self::actor_key_hash(&payload.user_name);
        let key = hash.to_le_bytes();
        let candidates = tx
            .lookup_index(WorkflowBinaryV0Codec::actor_lookup_index(), &key)
            .map_err(|error| Self::error("Actor lookup", error))?;
        for index in candidates {
            let (mut record, candidate_payload) = self.actor_in_write(tx, index)?;
            if record.actor_meta & 0x80 == 0
                && record.actor_meta & 0b111 == actor_kind
                && &candidate_payload == payload
            {
                if record.created_at_s != 0 && now > record.last_seen_at_s {
                    record.last_seen_at_s = now;
                    let raw = WorkflowBinaryV0Codec::encode_actor(record)
                        .map_err(|error| Self::error("Actor encode", error))?;
                    tx.overwrite_record(WorkflowBinaryV0Codec::actor_file(), index, &raw)
                        .map_err(|error| Self::error("Actor last-seen write", error))?;
                }
                return Ok(index);
            }
        }
        let bytes = WorkflowBinaryV0Codec::encode_actor_payload(payload)
            .map_err(|error| Self::error("Actor payload encode", error))?;
        let range = tx
            .append_payload(WorkflowBinaryV0Codec::actor_payload_file(), &bytes)
            .map_err(|error| Self::error("Actor payload write", error))?;
        self.sync_file(tx, "actor_payload.bin")
            .map_err(|error| Self::error("Actor payload sync", error))?;
        let record = V0ActorRecord {
            actor_meta: actor_kind
                | if payload.user_id.is_empty() {
                    0
                } else {
                    1 << 3
                }
                | if payload.email.is_empty() { 0 } else { 1 << 4 }
                | if payload.memo.is_empty() { 0 } else { 1 << 5 },
            reserved0: 0,
            payload_len: u16::try_from(range.payload_len)
                .map_err(|_| "Actor payload exceeds u16".to_string())?,
            payload_offset: range.payload_offset,
            actor_key_hash: hash,
            created_at_s: now,
            last_seen_at_s: now,
        };
        let raw = WorkflowBinaryV0Codec::encode_actor(record)
            .map_err(|error| Self::error("Actor encode", error))?;
        let index = tx
            .append_record(WorkflowBinaryV0Codec::actor_file(), &raw)
            .map_err(|error| Self::error("Actor write", error))?;
        self.sync_file(tx, "actor.bin")
            .map_err(|error| Self::error("Actor record sync", error))?;
        tx.append_index_candidate(WorkflowBinaryV0Codec::actor_lookup_index(), &key, index)
            .map_err(|error| Self::error("Actor lookup index write", error))?;
        Ok(index)
    }

    pub(super) fn actor_at<A: ReadV0>(
        &self,
        read: &A,
        actor_index: u32,
    ) -> Result<(V0ActorRecord, V0ActorPayload), String> {
        let record = WorkflowBinaryV0Codec::decode_actor(
            &read
                .read_record_v0(WorkflowBinaryV0Codec::actor_file(), actor_index)
                .map_err(|error| Self::error("Actor read", error))?,
        )
        .map_err(|error| Self::error("Actor decode", error))?;
        if record.actor_meta & 0x80 != 0 {
            return Err(format!(
                "Review references tombstoned Actor index {actor_index}"
            ));
        }
        let raw = read
            .read_payload_v0(
                WorkflowBinaryV0Codec::actor_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| Self::error("Actor payload read", error))?;
        let payload = WorkflowBinaryV0Codec::decode_actor_payload(&raw)
            .map_err(|error| Self::error("Actor payload decode", error))?;
        Ok((record, payload))
    }

    pub(super) fn review_at(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        index: u32,
    ) -> Result<JsonValue, String> {
        let record = WorkflowBinaryV0Codec::decode_review(
            &read
                .read_record(WorkflowBinaryV0Codec::review_file(), index)
                .map_err(|error| Self::error("Review read", error))?,
        )
        .map_err(|error| Self::error("Review decode", error))?;
        if record.review_meta & REVIEW_TOMBSTONE != 0 {
            return Err(format!("Unknown Review index {index}"));
        }
        let patchset = self
            .read_patchset(read, record.patchset_index)
            .map_err(|error| Self::error("Review Patchset read", error))?;
        let owner = self.patchset_owner(read, patchset)?;
        if record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal_plus1 != patchset.change_ordinal_plus1
        {
            return Err("Binary DB v0 Review ownership ordinals disagree".to_string());
        }
        let (actor_record, actor) = self.actor_at(read, record.actor_index_plus1 - 1)?;
        let reviewer_kind = match actor_record.actor_meta & 0b111 {
            0 => "user",
            1 => "team",
            2 => "bot",
            3 => "service_account",
            4 => "automation",
            5 => "unknown",
            _ => return Err("Review references an invalid Actor kind".to_string()),
        };
        let message_raw = read
            .read_payload(
                WorkflowBinaryV0Codec::review_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(|error| Self::error("Review payload read", error))?;
        let message = WorkflowBinaryV0Codec::decode_review_payload(&message_raw)
            .map_err(|error| Self::error("Review payload decode", error))?;
        let action_kind = record.review_meta & REVIEW_ACTION_MASK;
        let action = match (
            action_kind,
            record.review_meta & REVIEW_TASK_LANE != 0,
            record.review_meta & REVIEW_CODE_REVIEW_SUMMARY != 0,
            record.review_meta & REVIEW_DEFER != 0,
        ) {
            (0, false, false, false) => "request",
            (1, false, false, false) => "comment",
            (1, true, false, false) => "task_comment",
            (1, false, true, false) => "code_review_summary",
            (1, false, false, true) => "defer",
            (1, true, false, true) => "task_defer",
            (2, false, false, false) => "approve",
            (2, true, false, false) => "task_approve",
            (3, false, false, false) => "request_changes",
            (3, true, false, false) => "task_request_changes",
            (4, false, false, false) => "dismiss",
            _ => return Err("Binary DB v0 Review action/modifier mapping is invalid".to_string()),
        };
        let change_ref = owner
            .change
            .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal));
        let patchset_id = self.patchset_public_id(patchset);
        let is_request = action_kind == 0;
        Ok(json!({
            "review_id": self.review_id(&patchset_id, record.review_ordinal),
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "task_id": self.task_id(owner.task_index),
            "change_id": owner.change.map(|(_, change)| format!("C-{:02}", change.change_ordinal + 1)),
            "change_ref": change_ref,
            "patchset_id": patchset_id,
            "requested_groups": if is_request { json!([actor.user_name]) } else { json!([]) },
            "reviewer": if is_request { JsonValue::Null } else { json!(actor.user_name) },
            "reviewer_kind": reviewer_kind,
            "action": action,
            "comment": message,
            "note": if is_request { json!(message) } else { JsonValue::Null },
            "blocking": record.review_meta & REVIEW_BLOCKING != 0,
            "created_at": Self::timestamp(record.created_at_s)?,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn append_review(
        &self,
        change_id: &str,
        patchset_id: &str,
        actor_name: &str,
        actor_kind: u8,
        review_meta: u8,
        message: &str,
        explicit_review_id: Option<&str>,
        request_actor: Option<&JsonMap<String, JsonValue>>,
    ) -> Result<JsonValue, String> {
        let operation = "Binary DB v0 Review append";
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let index = self.append_review_in_write(
            &mut tx,
            change_id,
            patchset_id,
            actor_name,
            actor_kind,
            review_meta,
            message,
            explicit_review_id,
            request_actor,
            None,
            Self::now_s()?,
        )?;
        tx.commit().map_err(|error| Self::error(operation, error))?;
        self.review_at(&BinaryDbReadTxn::new(&self.db), index)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn append_review_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        change_id: &str,
        patchset_id: &str,
        actor_name: &str,
        actor_kind: u8,
        review_meta: u8,
        message: &str,
        explicit_review_id: Option<&str>,
        request_actor: Option<&JsonMap<String, JsonValue>>,
        accepted_actor_index: Option<u32>,
        now: u64,
    ) -> Result<u32, String> {
        let operation = "Binary DB v0 Review append";
        let message_bytes = WorkflowBinaryV0Codec::encode_review_payload(message)
            .map_err(|error| Self::error(operation, error))?;
        let patchset_index = self.patchset_index_for_id(tx, patchset_id)?;
        let request_actor_index_plus1 = request_actor
            .map(|payload| self.request_actor_in_write(tx, payload))
            .transpose()?;
        self.require_patchset_governance_authority(
            tx,
            patchset_index,
            patchset_id,
            "Review mutation",
        )?;
        let patchset = self
            .read_patchset(tx, patchset_index)
            .map_err(|e| e.to_string())?;
        let workflow_owner = self.require_patchset_owner(tx, change_id, patchset)?;
        self.require_current_patchset(tx, patchset_index, patchset)?;
        self.require_active_owner(tx, workflow_owner)?;
        let actor_index = match accepted_actor_index {
            Some(index) => {
                self.actor_at(tx, index)?;
                index
            }
            None => self.find_or_create_actor(tx, actor_name, actor_kind, now)?,
        };
        let mut owner = WorkflowBinaryV0Codec::decode_ordinal_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("patchset_review_index.bin"),
                patchset_index,
            )
            .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if owner.next_ordinal >= 64 {
            return Err(format!(
                "Patchset {patchset_id} has exhausted its v0 Review ordinals"
            ));
        }
        let ordinal = owner.next_ordinal;
        let canonical_id = self.review_id(patchset_id, ordinal);
        if let Some(explicit) = explicit_review_id {
            if explicit != canonical_id {
                return Err(format!(
                    "Binary DB v0 Review identity is derived as {canonical_id}, not {explicit}"
                ));
            }
        }
        let mut inventory =
            self.task_inventory_in_write(tx, "task_review_index.bin", workflow_owner.task_index)?;
        let range = tx
            .append_payload(WorkflowBinaryV0Codec::review_payload_file(), &message_bytes)
            .map_err(|error| Self::error(operation, error))?;
        self.sync_file(tx, "review_payload.bin")
            .map_err(|error| Self::error(operation, error))?;
        let record = V0ReviewRecord {
            review_meta,
            review_ordinal: ordinal,
            patch_ordinal: patchset.patch_ordinal,
            change_ordinal_plus1: patchset.change_ordinal_plus1,
            actor_index_plus1: actor_index + 1,
            patchset_index,
            previous_task_review_index_plus1: inventory.latest_index_plus1,
            previous_patchset_review_index_plus1: owner.latest_index_plus1,
            payload_offset: range.payload_offset,
            payload_len: u16::try_from(range.payload_len)
                .map_err(|_| "Review payload exceeds u16".to_string())?,
            reserved0: 0,
            created_at_s: now,
        };
        let raw = WorkflowBinaryV0Codec::encode_review(record)
            .map_err(|error| Self::error(operation, error))?;
        let review_index = tx
            .append_record(WorkflowBinaryV0Codec::review_file(), &raw)
            .map_err(|error| Self::error(operation, error))?;
        owner.latest_index_plus1 = review_index + 1;
        owner.count = owner
            .count
            .checked_add(1)
            .ok_or_else(|| "Patchset Review count exceeds u16".to_string())?;
        owner.next_ordinal = ordinal + 1;
        let owner_raw = WorkflowBinaryV0Codec::encode_ordinal_index(owner)
            .map_err(|error| Self::error(operation, error))?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("patchset_review_index.bin"),
            patchset_index,
            &owner_raw,
        )
        .map_err(|error| Self::error(operation, error))?;
        inventory.latest_index_plus1 = review_index + 1;
        inventory.count = inventory
            .count
            .checked_add(1)
            .ok_or_else(|| "Task Review count exceeds u16".to_string())?;
        let inventory_raw = WorkflowBinaryV0Codec::encode_inventory_index(inventory)
            .map_err(|error| Self::error(operation, error))?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_review_index.bin"),
            workflow_owner.task_index,
            &inventory_raw,
        )
        .map_err(|error| Self::error(operation, error))?;
        let mut updated_patchset = patchset;
        updated_patchset.patchset_meta |= PATCHSET_EVALUATION_PENDING;
        let patchset_raw = self
            .encode_patchset_replacement(tx, patchset_index, updated_patchset)
            .map_err(|error| Self::error(operation, error))?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            patchset_index,
            &patchset_raw,
        )
        .map_err(|error| Self::error(operation, error))?;
        let review_action = review_meta & REVIEW_ACTION_MASK;
        let review_set = if matches!(review_action, 0 | 3) {
            TASK_META_REVIEW_PENDING
        } else {
            0
        };
        let review_clear = if matches!(review_action, 2 | 4) {
            TASK_META_REVIEW_PENDING
        } else {
            0
        };
        self.update_owner_state(
            tx,
            workflow_owner,
            TASK_META_VALIDATION_PENDING | review_set,
            TASK_META_READY_TO_LAND | TASK_META_BLOCKED | review_clear,
            now,
            request_actor_index_plus1,
            |change| {
                change.change_meta |= CHANGE_META_VALIDATION_PENDING;
                change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
                match review_action {
                    0 | 3 => change.change_meta |= CHANGE_META_REVIEW_PENDING,
                    2 | 4 => change.change_meta &= !CHANGE_META_REVIEW_PENDING,
                    _ => {}
                }
            },
        )?;
        Ok(review_index)
    }

    pub(super) fn review_meta_for_action(action: &str, blocking: bool) -> Result<u8, String> {
        let value = match action {
            "request" => 0,
            "comment" => 1,
            "task_comment" => 1 | REVIEW_TASK_LANE,
            "code_review_summary" => 1 | REVIEW_CODE_REVIEW_SUMMARY,
            "approve" => 2,
            "task_approve" => 2 | REVIEW_TASK_LANE,
            "request_changes" => 3,
            "task_request_changes" => 3 | REVIEW_TASK_LANE,
            "defer" => 1 | REVIEW_DEFER,
            "task_defer" => 1 | REVIEW_TASK_LANE | REVIEW_DEFER,
            "dismiss" => 4,
            _ => return Err(format!("unsupported Binary DB v0 Review action {action:?}")),
        };
        Ok(value | if blocking { REVIEW_BLOCKING } else { 0 })
    }
}

impl<D> ServerWorkflowPatchsetStore for BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn select_patchset(
        &self,
        task_or_change_ref: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let payload = Self::required_object(payload, "patchset select payload")?;
        self.request_actor(payload)?;
        let patchset_id = Self::required_text(payload, "patchset_id")?;
        let read = BinaryDbReadTxn::new(&self.db);
        let index = self.patchset_index_for_id(&read, &patchset_id)?;
        let patchset = self
            .read_patchset(&read, index)
            .map_err(|e| e.to_string())?;
        self.require_patchset_owner(&read, task_or_change_ref, patchset)?;
        self.require_current_patchset(&read, index, patchset)?;
        self.require_patchset_governance_authority(
            &read,
            index,
            &patchset_id,
            "current candidate readback",
        )?;
        let patchset_id = self.patchset_public_id(patchset);
        Ok(
            json!({"task_id": self.task_id(patchset.task_index), "patchset_id": patchset_id,
        "current_patchset_id": patchset_id, "selection_changed": false,
        "change_id": patchset.change_ordinal_plus1.checked_sub(1).map(|n|format!("C-{:02}",n+1)),
        "change_ref": patchset.change_ordinal_plus1.checked_sub(1).map(|n|self.change_ref(patchset.task_index,n)),
        "patchset": self.patchset_at(&read, index)?}),
        )
    }

    fn get_patchset(
        &self,
        repo_name: Option<&str>,
        patchset_id: &str,
    ) -> Result<JsonValue, String> {
        if let Some(repo_name) = repo_name {
            self.repo_scope("ServerWorkflowPatchsetStore::get_patchset", repo_name)?;
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let index = self.patchset_index_for_id(&read, patchset_id)?;
        self.patchset_at(&read, index)
    }

    fn publish_patchset(
        &self,
        task_or_change_ref: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let payload = Self::required_object(payload, "patchset publish payload")?;
        self.request_actor(payload)?;
        let base_id = Self::required_text(payload, "base_snapshot_id")?;
        let revision_id = Self::required_text(payload, "revision_snapshot_id")?;
        let summary = Self::required_text(payload, "summary")?;
        let author_mode = Self::required_text(payload, "author_mode")?;
        if [
            history_promotion::HISTORY_PROMOTION_SUMMARY_PREFIX,
            history_promotion::HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX,
            history_promotion::LOCAL_LAND_RECEIPT_SUMMARY_PREFIX,
        ]
        .iter()
        .any(|prefix| summary.starts_with(prefix))
        {
            return Err("AGGREGATE_REFRESH_REQUIRED: reserved history authority requires native history prepare".to_string());
        }
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|e| e.to_string())?;
        let owner = self.workflow_owner(&tx, task_or_change_ref)?;
        let actor_index_plus1 = self.request_actor_in_write(&tx, payload)?;
        self.require_active_owner(&tx, owner)?;
        if self
            .latest_history_manifest_for_task(&tx, owner.task_index)?
            .is_some()
        {
            return Err("AGGREGATE_REFRESH_REQUIRED: preserve typed history through native correction prepare".to_string());
        }
        let snapshots =
            ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let (base, base_record) = snapshots
            .snapshot_by_id_in_write(&tx, &base_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Unknown base Snapshot {base_id}"))?;
        let (revision, revision_record) = snapshots
            .snapshot_by_id_in_write(&tx, &revision_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Unknown revision Snapshot {revision_id}"))?;
        let content = ServerBinaryRepositoryContentStore::new(self.db.clone());
        content
            .snapshot_file_map_in_write(&tx, &base_record)
            .map_err(|e| e.to_string())?;
        content
            .snapshot_file_map_in_write(&tx, &revision_record)
            .map_err(|e| e.to_string())?;
        self.require_patchset_capture_range(&tx, owner, base, revision)?;
        if let Some((index, current)) = self.current_task_patchset(&tx, owner.task_index)? {
            if self.history_manifest_for_patchset(&tx, index)?.is_some() {
                return Err("AGGREGATE_REFRESH_REQUIRED: preserve aggregate history through native correction prepare".to_string());
            }
            if current.change_ordinal_plus1 == owner.change_ordinal_plus1()
                && current.base_snapshot_index == base
                && current.revision_snapshot_index == revision
            {
                self.require_current_patchset(&tx, index, current)?;
                if Self::optional_text(payload, "patchset_id")
                    .is_some_and(|id| self.patchset_index_for_id(&tx, &id).ok() != Some(index))
                    || payload
                        .get("patchset_number")
                        .and_then(JsonValue::as_u64)
                        .is_some_and(|n| n != u64::from(current.patch_ordinal) + 1)
                {
                    return Err(
                        "Patchset retry identity disagrees with current candidate".to_string()
                    );
                }
                drop(tx);
                return self.patchset_at(&BinaryDbReadTxn::new(&self.db), index);
            }
        }
        let ordinal = self.next_task_patchset_ordinal(&tx, owner.task_index)?;
        let expected_id = self.patchset_public_id(V0PatchsetRecord {
            task_index: owner.task_index,
            change_ordinal_plus1: owner.change_ordinal_plus1(),
            patch_ordinal: ordinal,
            ..V0PatchsetRecord::default()
        });
        if Self::optional_text(payload, "patchset_id")
            .is_some_and(|id| !self.new_patchset_ref_matches(&id, owner, ordinal))
            || payload
                .get("patchset_number")
                .and_then(JsonValue::as_u64)
                .is_some_and(|n| n != u64::from(ordinal) + 1)
        {
            return Err(format!("Next Task Patchset identity is {expected_id}"));
        }
        let index = self.append_candidate_in_write(
            &mut tx,
            owner,
            base,
            revision,
            &summary,
            &author_mode,
            Self::now_s()?,
            actor_index_plus1,
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        self.patchset_at(&BinaryDbReadTxn::new(&self.db), index)
    }

    fn list_patchsets(
        &self,
        repo_name: Option<&str>,
        change_ref: &str,
    ) -> Result<JsonValue, String> {
        if let Some(repo_name) = repo_name {
            self.repo_scope("ServerWorkflowPatchsetStore::list_patchsets", repo_name)?;
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let owner = self.workflow_owner(&read, change_ref)?;
        let count = read
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| Self::error("ServerWorkflowPatchsetStore::list_patchsets", error))?;
        let mut rows = Vec::new();
        for index in 0..count {
            let record = self.read_patchset(&read, index).map_err(|error| {
                Self::error("ServerWorkflowPatchsetStore::list_patchsets", error)
            })?;
            if record.task_index == owner.task_index
                && (owner.change.is_none()
                    || record.change_ordinal_plus1 == owner.change_ordinal_plus1())
            {
                rows.push((record.patch_ordinal, self.patchset_at(&read, index)?));
            }
        }
        rows.sort_by_key(|(ordinal, _)| *ordinal);
        Ok(JsonValue::Array(
            rows.into_iter().map(|(_, value)| value).collect(),
        ))
    }
}

impl<D> ServerWorkflowReviewStore for BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn request_review(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let payload = Self::required_object(payload, "review request payload")?;
        self.request_actor(payload)?;
        if payload.contains_key("reviewer_kind") {
            return Err("reviewer_kind conflicts with a team Review request".to_string());
        }
        let patchset_id = Self::required_text(payload, "patchset_id")?;
        let groups = payload
            .get("reviewer_groups")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "reviewer_groups must contain exactly one team".to_string())?;
        if groups.len() != 1 {
            return Err(
                "Binary DB v0 Review request requires exactly one reviewer group".to_string(),
            );
        }
        let group = groups[0]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "reviewer_groups must contain one non-empty team".to_string())?;
        let note = Self::optional_text(payload, "note").unwrap_or_default();
        let explicit = Self::optional_text(payload, "review_id");
        self.append_review(
            change_id,
            &patchset_id,
            group,
            1,
            0,
            &note,
            explicit.as_deref(),
            Some(payload),
        )
    }

    fn record_review(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let payload = Self::required_object(payload, "review record payload")?;
        let patchset_id = Self::required_text(payload, "patchset_id")?;
        let reviewer = Self::required_text(payload, "reviewer")?;
        let actor_kind = match payload.get("reviewer_kind") {
            None => 0,
            Some(JsonValue::String(kind)) if kind == "user" => 0,
            Some(JsonValue::String(kind)) if kind == "bot" => 2,
            _ => return Err("reviewer_kind must be \"user\" or \"bot\"".to_string()),
        };
        let action = Self::required_text(payload, "action")?;
        if action == "request"
            || ["reviewer_groups", "requested_groups"].iter().any(|field| {
                match payload.get(*field) {
                    None | Some(JsonValue::Null) => false,
                    Some(JsonValue::Array(groups)) => !groups.is_empty(),
                    Some(_) => true,
                }
            })
        {
            return Err(
                "Team Review requests must use reviewer_groups through request_review".to_string(),
            );
        }
        let message = Self::optional_text(payload, "comment").unwrap_or_default();
        let blocking = Self::optional_bool(payload, "blocking").unwrap_or(false);
        let review_meta = Self::review_meta_for_action(&action, blocking)?;
        let explicit = Self::optional_text(payload, "review_id");
        self.append_review(
            change_id,
            &patchset_id,
            &reviewer,
            actor_kind,
            review_meta,
            &message,
            explicit.as_deref(),
            None,
        )
    }

    fn list_reviews(&self, change_id: &str) -> Result<JsonValue, String> {
        let read = BinaryDbReadTxn::new(&self.db);
        let owner = self.workflow_owner(&read, change_id)?;
        let count = read
            .record_count(WorkflowBinaryV0Codec::review_file())
            .map_err(|error| Self::error("ServerWorkflowReviewStore::list_reviews", error))?;
        let mut rows = Vec::new();
        for index in 0..count {
            let record = WorkflowBinaryV0Codec::decode_review(
                &read
                    .read_record(WorkflowBinaryV0Codec::review_file(), index)
                    .map_err(|error| {
                        Self::error("ServerWorkflowReviewStore::list_reviews", error)
                    })?,
            )
            .map_err(|error| Self::error("ServerWorkflowReviewStore::list_reviews", error))?;
            if record.review_meta & REVIEW_TOMBSTONE != 0 {
                continue;
            }
            let patchset = self
                .read_patchset(&read, record.patchset_index)
                .map_err(|error| Self::error("ServerWorkflowReviewStore::list_reviews", error))?;
            if patchset.task_index == owner.task_index
                && (owner.change.is_none()
                    || patchset.change_ordinal_plus1 == owner.change_ordinal_plus1())
            {
                rows.push(self.review_at(&read, index)?);
            }
        }
        let approvals = rows
            .iter()
            .filter(|row| {
                matches!(
                    row.get("action").and_then(JsonValue::as_str),
                    Some("approve") | Some("task_approve")
                )
            })
            .count();
        let blocking = rows
            .iter()
            .filter(|row| row.get("blocking").and_then(JsonValue::as_bool) == Some(true))
            .count();
        let comments = rows
            .iter()
            .filter(|row| {
                row.get("comment")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|value| !value.is_empty())
            })
            .count();
        Ok(json!({
            "task_id": self.task_id(owner.task_index),
            "change_id": owner.change.map(|(_, change)| format!("C-{:02}", change.change_ordinal + 1)),
            "change_ref": owner.change.map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal)),
            "reviews": rows,
            "approvals": approvals,
            "blocking": blocking,
            "comments": comments,
            "summary": {
                "approval_count": approvals,
                "blocking_count": blocking,
                "comment_count": comments,
                "review_count": rows.len(),
            },
        }))
    }
}
