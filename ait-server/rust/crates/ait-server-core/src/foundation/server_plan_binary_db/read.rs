use super::codec::{
    decode_plan_item_record_for_layout, decode_plan_record_for_layout,
    decode_plan_revision_record_for_layout, ServerPlanCodec, ServerPlanItemCodec,
    ServerPlanRevisionCodec,
};
use super::*;

struct PersistedCompactFile {
    layout: u32,
    file: BinaryFileId,
}

fn compact_file_with_persisted_layout<D>(
    read: &BinaryDbReadTxn<'_, D>,
    v1_file: BinaryFileId,
    compact_file: CompactPlanFile,
) -> Result<PersistedCompactFile, String>
where
    D: ServerRemoteBinaryDb + Clone,
{
    let layout = read.layout_id(v1_file).map_err(binary_error)?;
    let file = compact_plan_file_for(layout, compact_file)?;
    Ok(PersistedCompactFile { layout, file })
}

fn optional_compact_file_with_persisted_layout<D>(
    read: &BinaryDbReadTxn<'_, D>,
    v1_file: BinaryFileId,
    compact_file: CompactPlanFile,
) -> Result<Option<PersistedCompactFile>, String>
where
    D: ServerRemoteBinaryDb + Clone,
{
    let layout = match read.layout_id(v1_file) {
        Ok(layout) => layout,
        Err(err) if err.kind() == BinaryDbErrorKind::MissingData => return Ok(None),
        Err(err) => return Err(binary_error(err)),
    };
    let file = compact_plan_file_for(layout, compact_file)?;
    Ok(Some(PersistedCompactFile { layout, file }))
}

#[cfg_attr(test, allow(dead_code))]
impl<D, const WRITE_LAYOUT: u32> ServerPlanBinaryDbStore<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + Clone,
{
    pub(super) fn repo_scope(&self, operation: &str, repo_name: &str) -> Result<(), String> {
        let bound = self.db.repo_name().as_str();
        if repo_name == bound {
            Ok(())
        } else {
            Err(format!(
                "Binary DB plan runtime {operation} is bound to repository {bound}, not {repo_name}."
            ))
        }
    }

    pub(super) fn read_txn(&self) -> BinaryDbReadTxn<'_, D> {
        BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::PLAN)
    }

    pub(super) fn read_txn_with_content(&self) -> BinaryDbReadTxn<'_, D> {
        BinaryDbReadTxn::new_bounded_for_scope(
            &self.db,
            BinaryDbReadScope::PLAN.union(BinaryDbReadScope::CONTENT),
        )
    }

    #[cfg(test)]
    pub(super) fn record_count(&self, file: BinaryFileId) -> Result<u32, String> {
        let read = self.read_txn();
        self.record_count_with_read(&read, file)
    }

    pub(super) fn record_count_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        file: BinaryFileId,
    ) -> Result<u32, String> {
        read.record_count(file).map_err(binary_error)
    }

    pub(super) fn compact_record_count_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        v1_file: BinaryFileId,
        compact_file: CompactPlanFile,
    ) -> Result<u32, String> {
        let Some(file) = optional_compact_file_with_persisted_layout(read, v1_file, compact_file)?
        else {
            return Ok(0);
        };
        self.record_count_with_read(read, file.file)
    }

    #[cfg(test)]
    pub(super) fn read_plan_record(&self, index: u32) -> Result<PlanRecord, String> {
        let read = self.read_txn();
        self.read_plan_record_with_read(&read, index)
    }

    pub(super) fn read_plan_record_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        index: u32,
    ) -> Result<PlanRecord, String> {
        let persisted =
            compact_file_with_persisted_layout(read, plan_file(), CompactPlanFile::Plan)?;
        let raw = read
            .read_record(persisted.file, index)
            .map_err(binary_error)?;
        decode_plan_record_for_layout(persisted.layout, &raw)
    }

    fn read_all_plan_records_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
    ) -> Result<Vec<PlanRecord>, String> {
        let Some(persisted) =
            optional_compact_file_with_persisted_layout(read, plan_file(), CompactPlanFile::Plan)?
        else {
            return Ok(Vec::new());
        };
        let count = self.record_count_with_read(read, persisted.file.clone())?;
        let records = read
            .read_records(persisted.file, 0, count)
            .map_err(binary_error)?
            .into_iter()
            .map(|raw| decode_plan_record_for_layout(persisted.layout, &raw))
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() != count as usize {
            return Err(format!(
                "plan.bin bulk read returned {} records, expected {count}",
                records.len()
            ));
        }
        Ok(records)
    }

    #[cfg(test)]
    pub(super) fn read_plan_payload(&self, record: &PlanRecord) -> Result<String, String> {
        let read = self.read_txn();
        self.read_plan_payload_with_read(&read, record)
    }

    pub(super) fn read_plan_payload_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        record: &PlanRecord,
    ) -> Result<String, String> {
        let raw = read
            .read_payload(
                plan_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(binary_error)?;
        ServerPlanCodec::<PLAN_LAYOUT_ID>::decode_title_payload(raw)
    }

    #[cfg(test)]
    pub(super) fn current_plan_record(
        &self,
        plan_index: u32,
    ) -> Result<(PlanRecord, String), String> {
        let read = self.read_txn();
        self.current_plan_record_with_read(&read, plan_index)
    }

    pub(super) fn current_plan_record_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        plan_index: u32,
    ) -> Result<(PlanRecord, String), String> {
        let record = self.read_plan_record_with_read(read, plan_index)?;
        let title = self.read_plan_payload_with_read(read, &record)?;
        Ok((record, title))
    }

    #[cfg(test)]
    pub(super) fn read_plan_revision_record(
        &self,
        index: u32,
    ) -> Result<PlanRevisionRecord, String> {
        let read = self.read_txn();
        self.read_plan_revision_record_with_read(&read, index)
    }

    pub(super) fn read_plan_revision_record_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        index: u32,
    ) -> Result<PlanRevisionRecord, String> {
        let persisted = compact_file_with_persisted_layout(
            read,
            plan_revision_file(),
            CompactPlanFile::PlanRevision,
        )?;
        let raw = read
            .read_record(persisted.file, index)
            .map_err(binary_error)?;
        decode_plan_revision_record_for_layout(persisted.layout, &raw)
    }

    fn read_plan_revision_record_range_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        first_index: u32,
        count: u32,
    ) -> Result<Vec<PlanRevisionRecord>, String> {
        let persisted = compact_file_with_persisted_layout(
            read,
            plan_revision_file(),
            CompactPlanFile::PlanRevision,
        )?;
        let records = read
            .read_records(persisted.file, first_index, count)
            .map_err(binary_error)?
            .into_iter()
            .map(|raw| decode_plan_revision_record_for_layout(persisted.layout, &raw))
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() != count as usize {
            return Err(format!(
                "plan_revision.bin bulk read returned {} records, expected {count}",
                records.len()
            ));
        }
        Ok(records)
    }

    #[cfg(test)]
    pub(super) fn read_plan_revision_payload(
        &self,
        record: &PlanRevisionRecord,
    ) -> Result<PlanRevisionPayload, String> {
        let read = self.read_txn();
        self.read_plan_revision_payload_with_read(&read, record)
    }

    pub(super) fn read_plan_revision_payload_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        record: &PlanRevisionRecord,
    ) -> Result<PlanRevisionPayload, String> {
        let raw = read
            .read_payload(
                plan_revision_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(binary_error)?;
        ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_payload(&raw)
    }

    #[cfg(test)]
    pub(super) fn read_plan_item_record(&self, index: u32) -> Result<PlanItemRecord, String> {
        let read = self.read_txn();
        self.read_plan_item_record_with_read(&read, index)
    }

    pub(super) fn read_plan_item_record_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        index: u32,
    ) -> Result<PlanItemRecord, String> {
        let persisted =
            compact_file_with_persisted_layout(read, plan_item_file(), CompactPlanFile::PlanItem)?;
        let raw = read
            .read_record(persisted.file, index)
            .map_err(binary_error)?;
        decode_plan_item_record_for_layout(persisted.layout, &raw)
    }

    #[cfg(test)]
    pub(super) fn read_plan_item_payload(
        &self,
        record: &PlanItemRecord,
    ) -> Result<PlanItemPayload, String> {
        let read = self.read_txn();
        self.read_plan_item_payload_with_read(&read, record)
    }

    pub(super) fn read_plan_item_payload_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        record: &PlanItemRecord,
    ) -> Result<PlanItemPayload, String> {
        let raw = read
            .read_payload(
                plan_item_payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(binary_error)?;
        ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(&raw)
    }

    pub(super) fn revision_items_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        revision: &PlanRevisionRecord,
    ) -> Result<Vec<JsonValue>, String> {
        let mut items = Vec::with_capacity(usize::from(revision.item_count));
        for offset in 0..u32::from(revision.item_count) {
            let item_index = revision
                .item_start_index
                .checked_add(offset)
                .ok_or_else(|| {
                    format!(
                        "plan_revision.bin item range overflow at start {} count {}",
                        revision.item_start_index, revision.item_count
                    )
                })?;
            let record = self.read_plan_item_record_with_read(read, item_index)?;
            let payload = self.read_plan_item_payload_with_read(read, &record)?;
            items.push(plan_item_view_from_compact_record(&record, payload)?);
        }
        Ok(items)
    }

    #[cfg(test)]
    pub(super) fn read_plan_meta_at(&self, record_index: u32) -> Result<ServerPlanMeta, String> {
        let read = self.read_txn();
        self.read_plan_meta_at_with_read(&read, record_index)
    }

    pub(super) fn read_plan_meta_at_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        record_index: u32,
    ) -> Result<ServerPlanMeta, String> {
        let record = self.read_plan_record_with_read(read, record_index)?;
        self.plan_meta_from_record_with_read(read, record_index, &record)
    }

    fn plan_meta_from_record_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        record_index: u32,
        record: &PlanRecord,
    ) -> Result<ServerPlanMeta, String> {
        let title = self.read_plan_payload_with_read(read, record)?;
        Ok(ServerPlanMeta {
            plan_index: record_index,
            repo_id: self.db.repo_id().as_str().to_string(),
            title,
            status: plan_status_from_record(record)?,
            created_by: None,
            created_at: timestamp_string(record.created_at_s)?,
            updated_at: timestamp_string(record.updated_at_s)?,
        })
    }

    pub(super) fn plan_metas_matching_artifact_path_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        artifact_path: &str,
    ) -> Result<Vec<ServerPlanMeta>, String> {
        let plans = self.read_all_plan_records_with_read(read)?;
        let mut heads = Vec::with_capacity(plans.len());
        for (plan_index, plan) in plans.iter().enumerate() {
            let Some(revision_index) = plan.latest_revision_index_plus1.checked_sub(1) else {
                continue;
            };
            heads.push((plan_index as u32, plan, revision_index));
        }
        let Some(first_revision_index) = heads.iter().map(|(_, _, index)| *index).min() else {
            return Ok(Vec::new());
        };
        let last_revision_index = heads
            .iter()
            .map(|(_, _, index)| *index)
            .max()
            .expect("a first Plan head revision implies a last revision");
        let revision_count = last_revision_index
            .checked_sub(first_revision_index)
            .and_then(|distance| distance.checked_add(1))
            .ok_or_else(|| "plan_revision.bin head range overflow".to_string())?;
        let persisted_revision_count = self.compact_record_count_with_read(
            read,
            plan_revision_file(),
            CompactPlanFile::PlanRevision,
        )?;
        if last_revision_index >= persisted_revision_count {
            return Err(format!(
                "plan.bin references missing plan_revision.bin[{last_revision_index}] (record count {persisted_revision_count})"
            ));
        }
        let revisions = self.read_plan_revision_record_range_with_read(
            read,
            first_revision_index,
            revision_count,
        )?;

        let mut selected = Vec::with_capacity(heads.len());
        for (plan_index, plan, revision_index) in heads {
            let relative_index = revision_index
                .checked_sub(first_revision_index)
                .and_then(|index| usize::try_from(index).ok())
                .ok_or_else(|| "plan_revision.bin relative head index overflow".to_string())?;
            let revision = revisions.get(relative_index).ok_or_else(|| {
                format!("plan_revision.bin[{revision_index}] is missing from the bulk read")
            })?;
            if revision.plan_index != plan_index {
                return Err(format!(
                    "plan_revision.bin[{revision_index}] belongs to plan {}, not plan {plan_index}",
                    revision.plan_index
                ));
            }
            selected.push((plan_index, plan, revision_index, revision));
        }

        let first_payload_offset = selected
            .iter()
            .map(|(_, _, _, revision)| revision.payload_offset)
            .min()
            .expect("selected Plan heads must not be empty");
        let payload_end = selected
            .iter()
            .map(|(_, _, revision_index, revision)| {
                revision
                    .payload_offset
                    .checked_add(u64::from(revision.payload_len))
                    .ok_or_else(|| {
                        format!("plan_revision.bin[{revision_index}] payload range overflow")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .expect("selected Plan heads must have a payload end");
        let payload_span_len = payload_end
            .checked_sub(first_payload_offset)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or_else(|| "Plan head revision payload span exceeds u32".to_string())?;
        let payload_span = read
            .read_payload(
                plan_revision_payload_file(),
                first_payload_offset,
                payload_span_len,
            )
            .map_err(binary_error)?;

        let mut matching = Vec::new();
        for (plan_index, plan, revision_index, revision) in selected {
            let start = revision
                .payload_offset
                .checked_sub(first_payload_offset)
                .and_then(|offset| usize::try_from(offset).ok())
                .ok_or_else(|| {
                    format!("plan_revision.bin[{revision_index}] payload offset overflow")
                })?;
            let end = start
                .checked_add(usize::from(revision.payload_len))
                .ok_or_else(|| {
                    format!("plan_revision.bin[{revision_index}] payload length overflow")
                })?;
            let raw = payload_span.get(start..end).ok_or_else(|| {
                format!("plan_revision.bin[{revision_index}] payload is outside the bulk span")
            })?;
            let payload = ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_payload(raw)?;
            if payload.artifact_path == artifact_path {
                matching.push(self.plan_meta_from_record_with_read(read, plan_index, plan)?);
            }
        }
        Ok(matching)
    }

    #[cfg(test)]
    pub(super) fn latest_plan_meta_by_index(
        &self,
        plan_index: u32,
    ) -> Result<ServerPlanMeta, String> {
        let read = self.read_txn();
        self.latest_plan_meta_by_index_with_read(&read, plan_index)
    }

    pub(super) fn latest_plan_meta_by_index_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        plan_index: u32,
    ) -> Result<ServerPlanMeta, String> {
        self.read_plan_meta_at_with_read(read, plan_index)
    }

    #[cfg(test)]
    pub(super) fn plan_meta_by_id(&self, plan_id: &str) -> Result<ServerPlanMeta, String> {
        let read = self.read_txn();
        self.plan_meta_by_id_with_read(&read, plan_id)
    }

    pub(super) fn plan_meta_by_id_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        plan_id: &str,
    ) -> Result<ServerPlanMeta, String> {
        let plan_index = parse_server_plan_ref(plan_id)?;
        let plan_count =
            self.compact_record_count_with_read(read, plan_file(), CompactPlanFile::Plan)?;
        if plan_index >= plan_count {
            return Err(format!("Unknown plan: {plan_id}"));
        }
        self.latest_plan_meta_by_index_with_read(read, plan_index)
    }

    #[cfg(test)]
    pub(super) fn latest_plan_metas(&self) -> Result<Vec<ServerPlanMeta>, String> {
        let read = self.read_txn();
        self.latest_plan_metas_with_read(&read)
    }

    pub(super) fn latest_plan_metas_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
    ) -> Result<Vec<ServerPlanMeta>, String> {
        let count =
            self.compact_record_count_with_read(read, plan_file(), CompactPlanFile::Plan)?;
        let mut latest = Vec::with_capacity(count as usize);
        for record_index in 0..count {
            latest.push(self.read_plan_meta_at_with_read(read, record_index)?);
        }
        Ok(latest)
    }
}

#[cfg_attr(test, allow(dead_code))]
impl<D, const WRITE_LAYOUT: u32> ServerPlanBinaryDbStore<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub(super) fn begin_write(
        &self,
        purpose: ServerPlanBinaryDbWritePurpose,
    ) -> Result<ServerPlanDefaultWriteTxn<'_, D, WRITE_LAYOUT>, String> {
        ServerPlanBinaryDbWriteTxn::begin(&self.db, purpose)
    }

    pub(super) fn begin_write_with_plan_cas(
        &self,
        purpose: ServerPlanBinaryDbWritePurpose,
        plan_index: u32,
        expected: &PlanRecord,
    ) -> Result<ServerPlanDefaultWriteTxn<'_, D, WRITE_LAYOUT>, String> {
        let tx = self.begin_write(purpose)?;
        tx.require_unchanged_plan(plan_index, expected)?;
        Ok(tx)
    }
}
