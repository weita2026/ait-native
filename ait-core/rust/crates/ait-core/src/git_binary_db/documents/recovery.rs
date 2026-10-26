//! Recover only archived raw Git owners reserved by complete typed revisions.
use super::*;
use crate::plan_binary_db::{PlanRevisionCodec, PlanState};

struct Group {
    binding: (String, String, String),
    revisions: Vec<(u32, PlanRevisionRecord)>,
}
impl RawGitDocumentStore {
    /// Caller stages the authority and admits all content/Git closure before
    /// publishing. Existing Plan rows and every revision byte stay in place.
    pub fn recover_plan_roots(
        interop: &LocalBinaryDbFs,
        content: &LocalContentBinaryDb<1>,
    ) -> StoreResult<bool> {
        let store = BinaryDbPlanStore::<_, 1>::new(content.db().clone());
        let read = BinaryDbReadTxn::new(content.db());
        let plan_file = BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_file();
        let count = read.record_count(plan_file.clone())?;
        let mut plan_bytes = 1u32.to_le_bytes().to_vec();
        let mut payload_end = 4u64;
        for index in 0..count {
            let raw = read.read_record(plan_file.clone(), index)?;
            let row = PlanCodec::<1>::decode_record(&raw)?;
            store.read_plan_payload(&read, &row)?;
            payload_end = payload_end.max(
                row.payload_offset
                    .checked_add(u64::from(row.payload_len))
                    .ok_or("Plan payload range overflow")?,
            );
            plan_bytes.extend_from_slice(&raw);
        }
        let mut sources = std::collections::BTreeSet::new();
        let git = BinaryDbReadTxn::new(interop);
        for ordinal in 0..git.record_count(super::super::store::repository_file())? {
            let (kind, fingerprint, identity) =
                super::super::store::repository_identity_at(interop, ordinal)?;
            if kind == GitRepositoryKind::ImportSource {
                sources.insert((
                    format!("git-source/{}", hex(&fingerprint).to_ascii_uppercase()),
                    format!("Git source: {identity}"),
                ));
            }
        }
        let mut groups = BTreeMap::<u32, Group>::new();
        let mut tree_cache = crate::content_binary_db::BinaryDbTreeReadCache::default();
        let revisions = BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_revision_file();
        for ordinal in 0..read.record_count(revisions.clone())? {
            let (row, payload) = store.read_plan_revision(&read, ordinal)?;
            if !row.is_raw_git_artifact() {
                continue;
            }
            if row.is_tombstone() || row.item_count != 0 {
                return Err("invalid raw Git revision recovery owner".into());
            }
            let selector = payload.artifact_selector_text()?;
            let title = payload.title_snapshot_text()?;
            let path = payload.artifact_path_text()?;
            if !sources.contains(&(selector.clone(), title.clone())) {
                return Err("raw Git recovery source/full identity absent".into());
            }
            validate_git_artifact_path(&path)?;
            if !path.to_ascii_lowercase().ends_with(".md") {
                return Err("raw Git recovery path is not Markdown".into());
            }
            read_raw_artifact(
                content,
                &read,
                &row,
                &path,
                &payload.artifact_blob_id_text()?,
                &mut tree_cache,
            )?;
            tree_cache.clear_archives();
            tree_cache.clear_tree_entries();
            let binding = (selector, title, path);
            let group = groups.entry(row.plan_index).or_insert_with(|| Group {
                binding: binding.clone(),
                revisions: Vec::new(),
            });
            if group.binding != binding {
                return Err("raw Git recovery refuses conflicting reserved Plan identity".into());
            }
            if let Some(previous) = row.previous_revision_index() {
                if previous >= ordinal {
                    return Err("raw Git recovery revision chain must retain append order".into());
                }
                let prior = PlanRevisionCodec::<1>::decode_record(
                    &read.read_record(revisions.clone(), previous)?,
                )?;
                if !prior.is_raw_git_artifact()
                    || prior.plan_index != row.plan_index
                    || prior.revision_number.checked_add(1) != Some(row.revision_number)
                {
                    return Err("raw Git recovery previous revision binding differs".into());
                }
            } else if row.revision_number != 1 {
                return Err("raw Git recovery first revision number differs".into());
            }
            group.revisions.push((ordinal, row));
        }
        // Every reserved missing Plan is contiguous. No hole or unrelated
        // existing Plan may be replaced using guessed identity.
        let mut next = count;
        for (index, group) in &groups {
            if *index >= count {
                if *index != next {
                    return Err("raw Git recovery reserved Plan slots are not contiguous".into());
                }
                next = next.checked_add(1).ok_or("Plan ordinal overflow")?;
            } else {
                let row = store.read_plan_record(&read, *index)?;
                if row.state() != PlanState::Archived
                    || store.read_plan_payload(&read, &row)?.title_bytes
                        != group.binding.1.as_bytes()
                    || row.published_plan_index_plus1 != 0
                    || row.published_latest_revision_index_plus1 != 0
                {
                    return Err("raw Git recovery refuses an unrelated existing Plan slot".into());
                }
                let (last, _) = group.revisions.last().unwrap();
                if let Some(head) = row.latest_revision_index() {
                    if !group.revisions.iter().any(|(ordinal, _)| *ordinal == head) || head > *last
                    {
                        return Err(
                            "raw Git recovery Plan head is outside its exact revision chain".into(),
                        );
                    }
                }
            }
            for pair in group.revisions.windows(2) {
                if pair[1].1.previous_revision_index() != Some(pair[0].0) {
                    return Err(
                        "raw Git recovery refuses forked document revision ownership".into(),
                    );
                }
            }
        }
        drop(read);
        let payload_path = content
            .db()
            .authority_root()
            .as_path()
            .join("plan_payload.bin");
        let old_payload = match std::fs::read(&payload_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                1u32.to_le_bytes().to_vec()
            }
            Err(error) => return Err(error.to_string().into()),
        };
        if old_payload.len() < 4 || old_payload[..4] != 1u32.to_le_bytes() {
            return Err("raw Git recovery Plan payload layout differs".into());
        }
        let mut write = BinaryDbWriteTxn::begin(content.db(), BinaryDbCommandScope::PlanImport)?;
        let mut changed = false;
        for (index, group) in groups {
            let (last, last_row) = group.revisions.last().unwrap();
            let mut plan = if index < count {
                let start = 4 + index as usize * 48;
                PlanCodec::<1>::decode_record(&plan_bytes[start..start + 48])?
            } else {
                let title = group.binding.1.as_bytes();
                let suffix = old_payload
                    .get(
                        usize::try_from(payload_end)
                            .map_err(|_| "Plan payload offset overflow")?..,
                    )
                    .ok_or("Plan payload shorter than complete owners")?;
                let range = if let Some(position) =
                    suffix.windows(title.len()).position(|bytes| bytes == title)
                {
                    crate::binary_db::PayloadRange {
                        payload_offset: payload_end + position as u64,
                        payload_len: u32::try_from(title.len())
                            .map_err(|_| "Plan title length overflow")?,
                    }
                } else {
                    store.append_plan_payload(
                        &mut write,
                        &PlanPayload {
                            title_bytes: title.to_vec(),
                        },
                    )?
                };
                changed = true;
                PlanRecord {
                    actor_index_plus1: 0,
                    plan_meta: 1,
                    reserved0: 0,
                    payload_len: u16::try_from(range.payload_len)
                        .map_err(|_| "Plan title exceeds u16")?,
                    payload_offset: range.payload_offset,
                    latest_revision_index_plus1: 0,
                    published_plan_index_plus1: 0,
                    published_latest_revision_index_plus1: 0,
                    created_at_s: group.revisions[0].1.created_at_s,
                    updated_at_s: last_row.created_at_s,
                    published_at_s: 0,
                }
            };
            if plan.latest_revision_index_plus1 != last + 1 {
                plan.latest_revision_index_plus1 =
                    last.checked_add(1).ok_or("Revision ordinal overflow")?;
                plan.updated_at_s = last_row.created_at_s;
                changed = true;
            }
            let bytes = PlanCodec::<1>::encode_record(&plan)?;
            if index < count {
                let start = 4 + index as usize * 48;
                plan_bytes[start..start + 48].copy_from_slice(&bytes);
            } else {
                plan_bytes.extend_from_slice(&bytes);
            }
        }
        if changed {
            write.commit_with_atomic_file_replacement(
                plan_file,
                &plan_bytes,
                "raw Git recovered Plan roots",
            )?;
        } else {
            write.commit()?;
        }
        Ok(changed)
    }
}
