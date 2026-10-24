//! Pure metadata conversion for coordinated Server startup. Codecs and reads
//! never invoke this function or change an activated authority.
use super::{
    PlanCodec, PlanRevisionCodec, PLAN_RECORD_SIZE, PLAN_REVISION_RECORD_SIZE,
    REMOTE_PLAN_LAYOUT_ID, REMOTE_PLAN_RECORD_SIZE, REMOTE_PLAN_REVISION_RECORD_SIZE,
};
use crate::binary_db::{BinaryDbError, StoreResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemotePlanMetadataProfile {
    Legacy,
    Actor,
}

fn fixed_profile(bytes: &[u8], layout: u32, width: u32, name: &str) -> StoreResult<()> {
    if bytes.get(..4) != Some(layout.to_le_bytes().as_slice())
        || !bytes.len().saturating_sub(4).is_multiple_of(width as usize)
    {
        return Err(BinaryDbError::unsupported(format!(
            "unsupported {name} metadata profile"
        )));
    }
    Ok(())
}

pub fn classify_remote_plan_metadata(
    plan: &[u8],
    revisions: &[u8],
) -> StoreResult<RemotePlanMetadataProfile> {
    let profile = match (plan.get(..4), revisions.get(..4)) {
        (Some([1, 0, 0, 0]), Some([1, 0, 0, 0])) => RemotePlanMetadataProfile::Legacy,
        (Some([2, 0, 0, 0]), Some([2, 0, 0, 0])) => RemotePlanMetadataProfile::Actor,
        _ => {
            return Err(BinaryDbError::unsupported(
                "Remote Plan metadata requires one complete old or Actor profile",
            ))
        }
    };
    let (layout, plan_width, revision_width) = match profile {
        RemotePlanMetadataProfile::Legacy => (1, PLAN_RECORD_SIZE, PLAN_REVISION_RECORD_SIZE),
        RemotePlanMetadataProfile::Actor => (
            REMOTE_PLAN_LAYOUT_ID,
            REMOTE_PLAN_RECORD_SIZE,
            REMOTE_PLAN_REVISION_RECORD_SIZE,
        ),
    };
    fixed_profile(plan, layout, plan_width, "plan.bin")?;
    fixed_profile(revisions, layout, revision_width, "plan_revision.bin")?;
    Ok(profile)
}

/// Preserve each complete source record prefix and append only unknown Actor
/// tails. The caller publishes only converted fixed files under existing writer
/// exclusion and durability ordering, without an independent DB proof pass.
pub fn convert_remote_plan_metadata(
    plan: &[u8],
    revisions: &[u8],
) -> StoreResult<(Vec<u8>, Vec<u8>)> {
    match classify_remote_plan_metadata(plan, revisions)? {
        RemotePlanMetadataProfile::Actor => Ok((plan.to_vec(), revisions.to_vec())),
        RemotePlanMetadataProfile::Legacy => {
            let mut target_plan = REMOTE_PLAN_LAYOUT_ID.to_le_bytes().to_vec();
            for bytes in plan[4..].chunks_exact(PLAN_RECORD_SIZE as usize) {
                let record = PlanCodec::<1>::decode_record(bytes)?;
                target_plan.extend(PlanCodec::<2>::encode_record(&record)?);
            }
            let mut target_revisions = REMOTE_PLAN_LAYOUT_ID.to_le_bytes().to_vec();
            for bytes in revisions[4..].chunks_exact(PLAN_REVISION_RECORD_SIZE as usize) {
                let record = PlanRevisionCodec::<1>::decode_record(bytes)?;
                target_revisions.extend(PlanRevisionCodec::<2>::encode_record(&record)?);
            }
            Ok((target_plan, target_revisions))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old_files() -> (Vec<u8>, Vec<u8>) {
        let mut plan = 1_u32.to_le_bytes().to_vec();
        plan.extend([0; 48]);
        let mut revision = 1_u32.to_le_bytes().to_vec();
        revision.extend([0; 56]);
        (plan, revision)
    }

    #[test]
    fn conversion_preserves_all_prefix_bytes_and_unknown_history() {
        let (mut old_plan, mut old_revision) = old_files();
        for (index, byte) in old_plan[4..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        for (index, byte) in old_revision[4..].iter_mut().enumerate() {
            *byte = (index as u8) + 60;
        }
        let source = (old_plan.clone(), old_revision.clone());
        let (plan, revision) = convert_remote_plan_metadata(&old_plan, &old_revision).unwrap();
        assert_eq!(
            classify_remote_plan_metadata(&plan, &revision).unwrap(),
            RemotePlanMetadataProfile::Actor
        );
        assert_eq!(&plan[4..52], &old_plan[4..]);
        assert_eq!(&revision[4..60], &old_revision[4..]);
        assert_eq!(&plan[52..], &[0; 4]);
        assert_eq!(&revision[60..], &[0; 4]);
        assert_eq!(
            PlanCodec::<2>::decode_record(&plan[4..])
                .unwrap()
                .actor_key_index(),
            None
        );
        assert_eq!(
            PlanRevisionCodec::<2>::decode_record(&revision[4..])
                .unwrap()
                .actor_key_index(),
            None
        );
        assert_eq!((old_plan, old_revision), source);
        assert_eq!(
            convert_remote_plan_metadata(&plan, &revision).unwrap(),
            (plan, revision)
        );
    }

    #[test]
    fn layout_two_has_exact_actor_tails_and_layout_one_bytes_are_unchanged() {
        let (old_plan, old_revision) = old_files();
        let mut plan = PlanCodec::<1>::decode_record(&old_plan[4..]).unwrap();
        let mut revision = PlanRevisionCodec::<1>::decode_record(&old_revision[4..]).unwrap();
        assert_eq!(PlanCodec::<1>::encode_record(&plan).unwrap(), old_plan[4..]);
        assert_eq!(
            PlanRevisionCodec::<1>::encode_record(&revision).unwrap(),
            old_revision[4..]
        );
        plan.actor_index_plus1 = 1;
        revision.actor_index_plus1 = u32::MAX;
        let plan_bytes = PlanCodec::<2>::encode_record(&plan).unwrap();
        let revision_bytes = PlanRevisionCodec::<2>::encode_record(&revision).unwrap();
        assert_eq!(&plan_bytes[48..], &1_u32.to_le_bytes());
        assert_eq!(&revision_bytes[56..], &u32::MAX.to_le_bytes());
        assert_eq!(
            PlanCodec::<2>::decode_record(&plan_bytes)
                .unwrap()
                .actor_key_index(),
            Some(0)
        );
        assert_eq!(
            PlanRevisionCodec::<2>::decode_record(&revision_bytes)
                .unwrap()
                .actor_key_index(),
            Some(u32::MAX - 1)
        );
        assert!(PlanCodec::<1>::encode_record(&plan).is_err());
        assert!(PlanRevisionCodec::<1>::encode_record(&revision).is_err());
        assert_eq!(
            PlanCodec::<2>::payload_file(),
            PlanCodec::<1>::payload_file()
        );
        assert_eq!(
            PlanRevisionCodec::<2>::payload_file(),
            PlanRevisionCodec::<1>::payload_file()
        );
        assert_eq!(
            super::super::PlanItemCodec::<2>::record_file(),
            super::super::PlanItemCodec::<1>::record_file()
        );
    }

    #[test]
    fn partial_mixed_unknown_and_wrong_width_profiles_reject() {
        let (old_plan, old_revision) = old_files();
        let (plan, revision) = convert_remote_plan_metadata(&old_plan, &old_revision).unwrap();
        for (a, b) in [(&old_plan, &revision), (&plan, &old_revision)] {
            assert!(classify_remote_plan_metadata(a, b).is_err());
        }
        for bytes in [
            vec![],
            vec![1, 0, 0],
            vec![3, 0, 0, 0],
            [vec![2, 0, 0, 0], vec![0; 48]].concat(),
        ] {
            assert!(classify_remote_plan_metadata(&bytes, &revision).is_err());
        }
        let (a, b) =
            convert_remote_plan_metadata(&1_u32.to_le_bytes(), &1_u32.to_le_bytes()).unwrap();
        assert_eq!(a, 2_u32.to_le_bytes());
        assert_eq!(b, 2_u32.to_le_bytes());
        assert!(PlanCodec::<3>::decode_record(&plan[4..]).is_err());
    }

    #[test]
    fn synthetic_metadata_conversion_and_upgraded_reopen_are_measured() {
        let mut plan = 1_u32.to_le_bytes().to_vec();
        let mut revision = 1_u32.to_le_bytes().to_vec();
        plan.resize(4 + 5000 * 48, 0);
        revision.resize(4 + 5000 * 56, 0);
        let start = std::time::Instant::now();
        let (target_plan, target_revision) =
            convert_remote_plan_metadata(&plan, &revision).unwrap();
        let transform = start.elapsed();
        let start = std::time::Instant::now();
        assert_eq!(
            classify_remote_plan_metadata(&target_plan, &target_revision).unwrap(),
            RemotePlanMetadataProfile::Actor
        );
        let reopen = start.elapsed();
        assert_eq!(target_plan.len() + target_revision.len(), 560008);
        eprintln!("synthetic Plan metadata: 10000 rows, 520008->560008 bytes; transform={transform:?}, profile reopen={reopen:?}; memory-only, warm process, no fsync/startup claim");
    }

    #[test]
    fn remote_store_uses_scoped_files_and_preserves_immutable_author() {
        use crate::binary_db::{
            BinaryDbCommandScope, BinaryDbNoopFsyncPolicy, RemoteBinaryDbFs, RepoId, RepoName,
        };
        use crate::plan_binary_db::{PlanPayload, PlanRevisionPayload, RemoteFsPlanBinaryDb};
        let temp = tempfile::tempdir().unwrap();
        let remote = RemoteFsPlanBinaryDb::<2>::from_fs(RemoteBinaryDbFs::test_fixture(
            temp.path(),
            RepoId::new("repo"),
            RepoName::new("repo"),
        ));
        let (plan, revision) = old_files();
        let mut plan = PlanCodec::<1>::decode_record(&plan[4..]).unwrap();
        plan.actor_index_plus1 = 1;
        let mut revision = PlanRevisionCodec::<1>::decode_record(&revision[4..]).unwrap();
        revision.actor_index_plus1 = 2;
        let mut tx = remote
            .begin_write_txn_with_fsync_policy(
                BinaryDbCommandScope::PlanSyncRemote,
                BinaryDbNoopFsyncPolicy,
            )
            .unwrap();
        remote
            .append_plan(
                &mut tx,
                plan,
                &PlanPayload {
                    title_bytes: b"title".to_vec(),
                },
            )
            .unwrap();
        let (index, record) = remote
            .append_plan_revision(
                &mut tx,
                revision,
                &PlanRevisionPayload {
                    title_snapshot_bytes: b"title".to_vec(),
                    summary_bytes: vec![],
                    artifact_path_bytes: vec![],
                    artifact_selector_bytes: vec![],
                    artifact_heading_bytes: vec![],
                    artifact_blob_id_bytes: vec![],
                },
            )
            .unwrap();
        tx.commit().unwrap();
        let read = remote.begin_read_txn();
        let loaded = remote.read_plan_revision(&read, index).unwrap().0;
        assert_eq!(loaded.actor_key_index(), Some(1));
        drop(read);
        let mut tx = remote
            .begin_write_txn_with_fsync_policy(
                BinaryDbCommandScope::PlanSyncRemote,
                BinaryDbNoopFsyncPolicy,
            )
            .unwrap();
        let mut other = record.clone();
        other.actor_index_plus1 = 3;
        assert!(remote
            .overwrite_plan_revision_record(&mut tx, index, &other)
            .unwrap_err()
            .contains("immutable"));
        remote
            .overwrite_plan_revision_record(&mut tx, index, &record)
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("plan.bin")).unwrap().len(),
            56
        );
        assert_eq!(
            std::fs::read(temp.path().join("plan_revision.bin"))
                .unwrap()
                .len(),
            64
        );
        assert_eq!(
            &std::fs::read(temp.path().join("plan_payload.bin")).unwrap()[..4],
            &1_u32.to_le_bytes()
        );
        assert_eq!(
            &std::fs::read(temp.path().join("plan_revision_payload.bin")).unwrap()[..4],
            &1_u32.to_le_bytes()
        );
    }
}
