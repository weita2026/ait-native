use super::*;

/// Command-scoped metadata only; payload bytes are never retained. New Blob
/// ordinals are scanned once, including those appended by Snapshot authoring.
#[derive(Default)]
pub struct RawBlobBatchSession {
    scanned: u32,
    ordinals: BTreeMap<String, u32>,
    delta_heads: BTreeMap<String, u32>,
}

impl RawBlobBatchSession {
    pub(crate) fn note_delta_hint(&mut self, hint: &str, ordinal: u32) {
        self.delta_heads.insert(hint.to_string(), ordinal);
    }
    pub(crate) fn refresh<const W: u32>(
        &mut self,
        content: &LocalContentBinaryDb<W>,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    ) -> Result<(), String> {
        let count = read.record_count(crate::binary_db::BinaryFileId::new(
            crate::content_binary_db::BLOB_BIN,
            W,
            crate::content_binary_db::BLOB_RECORD_SIZE,
        ))?;
        if count < self.scanned {
            return Err("Blob authority changed; reopen batch metadata session".into());
        }
        for ordinal in self.scanned..count {
            let view = content.blobs.blob_view_at(read, ordinal)?;
            if self
                .ordinals
                .insert(view.blob_id.clone(), ordinal)
                .is_some()
            {
                return Err(format!("duplicate native Blob identity {}", view.blob_id));
            }
        }
        self.scanned = count;
        Ok(())
    }
}

impl<const WRITE_LAYOUT: u32> LocalContentBinaryDb<WRITE_LAYOUT> {
    /// One bounded input batch, one existing native zstd object pack. Full-byte
    /// verification also applies to reused Blob IDs; no body cache survives.
    pub fn ensure_blob_bytes_batch(&self, bodies: &[&[u8]]) -> Result<Vec<u32>, String> {
        self.ensure_blob_bytes_batch_with_session(bodies, &mut RawBlobBatchSession::default())
    }

    pub fn ensure_blob_bytes_batch_with_session(
        &self,
        bodies: &[&[u8]],
        session: &mut RawBlobBatchSession,
    ) -> Result<Vec<u32>, String> {
        self.ensure_blob_bytes_batch_with_read_session(bodies, session, &mut Default::default())
    }

    pub(crate) fn ensure_blob_bytes_batch_with_read_session(
        &self,
        bodies: &[&[u8]],
        session: &mut RawBlobBatchSession,
        body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
    ) -> Result<Vec<u32>, String> {
        self.ensure_blob_bytes_batch_with_hints(bodies, None, None, session, body_read)
    }

    pub(crate) fn ensure_blob_bytes_batch_with_delta_hints(
        &self,
        bodies: &[&[u8]],
        hints: &[Option<&str>],
        session: &mut RawBlobBatchSession,
    ) -> Result<Vec<u32>, String> {
        if bodies.len() != hints.len() || bodies.len() > 128 {
            return Err("raw delta batch requires one hint per body and at most 128 bodies".into());
        }
        self.ensure_blob_bytes_batch_with_hints(
            bodies,
            Some(hints),
            None,
            session,
            &mut Default::default(),
        )
    }

    pub(crate) fn ensure_blob_bytes_batch_with_delta_hints_and_partitions(
        &self,
        bodies: &[&[u8]],
        hints: &[Option<&str>],
        partitions: &[bool],
        session: &mut RawBlobBatchSession,
    ) -> Result<Vec<u32>, String> {
        if bodies.len() != hints.len() || bodies.len() != partitions.len() || bodies.len() > 128 {
            return Err(
                "raw partitioned batch requires one hint/class per body and at most 128 bodies"
                    .into(),
            );
        }
        self.ensure_blob_bytes_batch_with_hints(
            bodies,
            Some(hints),
            Some(partitions),
            session,
            &mut Default::default(),
        )
    }
    pub(crate) fn raw_delta_pack_members(
        &self,
        candidates: Vec<PackCandidate>,
        heads: &BTreeMap<String, u32>,
    ) -> Result<Vec<ObjectPackWriteMember>, String> {
        self.raw_delta_pack_members_with_partitions(candidates, heads, None)
    }
    fn raw_delta_pack_members_with_partitions(
        &self,
        candidates: Vec<PackCandidate>,
        heads: &BTreeMap<String, u32>,
        priority_ids: Option<&BTreeSet<String>>,
    ) -> Result<Vec<ObjectPackWriteMember>, String> {
        if candidates.len() > 128 {
            return Err("delta pack input exceeds 128 members".into());
        }
        let read = BinaryDbReadTxn::new(self.db());
        let mut body_read = Default::default();
        let mut initial = BTreeMap::new();
        let mut base_bytes = 0usize;
        let mut base_packs = BTreeSet::new();
        let mut base_pack_bytes = 0u64;
        for candidate in &candidates {
            let Some(hint) = candidate.path_hint.as_ref() else {
                continue;
            };
            let Some(index) = heads.get(hint) else {
                continue;
            };
            let view = self.blobs.blob_view_at(&read, *index)?;
            if view.blob_id == candidate.blob_id
                || !(crate::pack_substrate::MIN_DELTA_BLOB_BYTES
                    ..=crate::pack_substrate::MAX_DELTA_BLOB_BYTES)
                    .contains(&candidate.data.len())
                || !(crate::pack_substrate::MIN_DELTA_BLOB_BYTES as u64
                    ..=crate::pack_substrate::MAX_DELTA_BLOB_BYTES as u64)
                    .contains(&view.size_bytes)
            {
                continue;
            }
            let member = self.object_packs.object_pack_member_view_at(
                &read,
                view.pack_member_index.ok_or("delta base has no member")?,
            )?;
            if member.record.delta_chain_depth as usize >= DEFAULT_MAX_DELTA_CHAIN_DEPTH {
                continue;
            }
            let pack = self
                .object_packs
                .object_pack_view_at(&read, member.record.pack_index)?;
            let additional_pack_bytes = if base_packs.contains(&member.record.pack_index) {
                0
            } else {
                pack.record.total_bytes
            };
            if base_bytes + view.size_bytes as usize > 8 * 1024 * 1024
                || base_pack_bytes.saturating_add(additional_pack_bytes) > 32 * 1024 * 1024
            {
                continue;
            }
            let data = self.blobs.read_blob_bytes_for_view_with_session(
                &read,
                &view,
                &mut body_read,
                0,
                None,
            )?;
            base_bytes += data.len();
            base_pack_bytes += additional_pack_bytes;
            base_packs.insert(member.record.pack_index);
            initial.insert(
                hint.clone(),
                PackCandidate {
                    entry_name: format!("blobs/{}", view.blob_id),
                    blob_id: view.blob_id,
                    data,
                    path_hint: None,
                    chain_depth: member.record.delta_chain_depth as usize,
                },
            );
        }
        drop(read);
        if priority_ids.is_none() {
            crate::pack_substrate::build_typed_pack_members_with_storage_gate(
                candidates,
                DEFAULT_MAX_DELTA_CHAIN_DEPTH,
                Some(&initial),
            )
        } else {
            crate::pack_substrate::build_typed_pack_members_with_storage_gate_and_partitions(
                candidates,
                DEFAULT_MAX_DELTA_CHAIN_DEPTH,
                Some(&initial),
                priority_ids,
            )
        }
    }

    fn ensure_blob_bytes_batch_with_hints(
        &self,
        bodies: &[&[u8]],
        hints: Option<&[Option<&str>]>,
        partitions: Option<&[bool]>,
        session: &mut RawBlobBatchSession,
        body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
    ) -> Result<Vec<u32>, String> {
        let ids = bodies
            .iter()
            .map(|body| blob_id_from_sha256(&sha256_array(body)))
            .collect::<Vec<_>>();
        let priority_ids = partitions.map(|classes| {
            ids.iter()
                .zip(classes)
                .filter(|(_, p)| **p)
                .map(|(id, _)| id.clone())
                .collect::<BTreeSet<_>>()
        });
        let read = BinaryDbReadTxn::new(self.db());
        session.refresh(self, &read)?;
        let mut candidates = BTreeMap::new();
        for (body, id) in bodies.iter().zip(&ids) {
            if let Some(ordinal) = session.ordinals.get(id) {
                let view = self.blobs.blob_view_at(&read, *ordinal)?;
                if &view.blob_id != id
                    || view.record.sha256 != sha256_array(body)
                    || view.size_bytes != body.len() as u64
                    || self
                        .blobs
                        .read_blob_bytes_for_view_with_session(&read, &view, body_read, 0, None)?
                        != *body
                {
                    return Err(format!("native Blob collision or content drift for {id}"));
                }
            } else if let Some(previous) = candidates.insert(id.clone(), *body) {
                if previous != *body {
                    return Err(format!("native Blob prefix collision for {id}"));
                }
            }
        }
        let hint_by_id = hints
            .map(|values| {
                ids.iter()
                    .zip(values)
                    .filter_map(|(id, hint)| hint.map(|h| (id.clone(), h.to_string())))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        drop(read);
        if !candidates.is_empty() {
            let created_at = current_timestamp();
            let mut seed = b"native-raw-blob-batch/v1\n".to_vec();
            let mut typed = Vec::with_capacity(candidates.len());
            for (id, body) in &candidates {
                seed.extend_from_slice(&sha256_array(body));
                typed.push(PackCandidate {
                    entry_name: format!("blobs/{id}"),
                    blob_id: id.clone(),
                    data: body.to_vec(),
                    path_hint: hint_by_id.get(id).cloned(),
                    chain_depth: 0,
                });
            }
            let members = if priority_ids.is_some() {
                self.raw_delta_pack_members_with_partitions(
                    typed,
                    &session.delta_heads,
                    priority_ids.as_ref(),
                )?
            } else {
                self.raw_delta_pack_members(typed, &session.delta_heads)?
            };
            let layout = priority_ids.as_ref().map(|ids| {
                crate::pack_substrate::ObjectPackChunkLayout::for_members(&members, Some(ids))
            });
            if members.iter().any(|m| m.entry_type == "delta") {
                for member in &members {
                    seed.extend_from_slice(&sha256_array(&member.data));
                    if let Some(base) = &member.base_blob_id {
                        seed.extend_from_slice(base.as_bytes());
                    }
                    seed.extend_from_slice(&(member.chain_depth as u64).to_le_bytes());
                }
            }
            if let Some(layout) = &layout {
                layout.extend_identity_seed(&members, &mut seed);
            }
            let metadata = members
                .iter()
                .map(|member| {
                    let body = candidates
                        .get(&member.blob_id)
                        .ok_or("raw member logical body absent")?;
                    Ok(BinaryDbObjectPackMemberWriteInput {
                        blob_id: member.blob_id.clone(),
                        sha256: hex_lower(&sha256_array(body)),
                        size_bytes: i64::try_from(body.len())
                            .map_err(|_| "raw Blob size exceeds i64")?,
                        pack_entry_type: member.entry_type.clone(),
                        pack_base_blob_id: member.base_blob_id.clone(),
                        pack_chain_depth: member.chain_depth as i64,
                        created_at: created_at.clone(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let pack_id = format!(
                "PCK-{}",
                hex_lower(&sha256_array(&seed))[..12].to_ascii_uppercase()
            );
            let pack_rel_path = default_object_pack_relative_path(&pack_id);
            let path = self.pack_root.as_path().join(&pack_rel_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            let stats = if let Some(layout) = &layout {
                crate::pack_substrate::write_typed_pack_archive_immutable_with_layout(
                    path.to_string_lossy().as_ref(),
                    &pack_id,
                    &members,
                    layout,
                )?
            } else {
                write_typed_pack_archive_with_format(
                    path.to_string_lossy().as_ref(),
                    &pack_id,
                    CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
                    &members,
                    zstd_only_object_pack_write_format(),
                )?
            };
            BinaryDbContentWriteCoordinator::new(
                &self.blobs,
                &self.object_packs,
                &self.tree_packs,
                &self.trees,
                &self.snapshots,
            )
            .record_object_pack_metadata(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbObjectPackWriteInput {
                    pack_id,
                    pack_rel_path,
                    pack_format: json_string_field(&stats, "pack_format")?,
                    member_count: json_i64_field(&stats, "member_count")?,
                    total_bytes: json_i64_field(&stats, "total_bytes")?,
                    created_at,
                    members: metadata,
                },
            )
            .map_err(|error| error.to_string())?;
        }
        let read = BinaryDbReadTxn::new(self.db());
        session.refresh(self, &read)?;
        let output = bodies
            .iter()
            .zip(&ids)
            .map(|(body, id)| {
                let ordinal = session
                    .ordinals
                    .get(id)
                    .ok_or_else(|| format!("raw Blob {id} missing after batch"))?;
                let view = self.blobs.blob_view_at(&read, *ordinal)?;
                if &view.blob_id != id
                    || view.record.sha256 != sha256_array(body)
                    || view.size_bytes != body.len() as u64
                    || self
                        .blobs
                        .read_blob_bytes_for_view_with_session(&read, &view, body_read, 0, None)?
                        != *body
                {
                    return Err(format!("raw Blob {id} batch content mismatch"));
                }
                Ok(view.blob_index)
            })
            .collect::<Result<Vec<_>, String>>()?;
        if let Some(hints) = hints {
            for (hint, index) in hints.iter().zip(&output) {
                if let Some(hint) = hint {
                    session.note_delta_hint(hint, *index);
                }
            }
        }
        Ok(output)
    }
}
