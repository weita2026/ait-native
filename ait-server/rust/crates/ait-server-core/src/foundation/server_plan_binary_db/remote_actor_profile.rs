// Canonical algorithm port: Core LCT-1116 / SNP-7D27A46AAE12, remote_actor_profile.rs.
// Source SHA-256: 41d442d8aab40a469da1ea06c07e771af36946e81e49930a2419941d324a38a1. Only imports/codecs/errors are adapted.
//! Pure metadata conversion for coordinated Server startup. Codecs and reads
//! never invoke this function or change an activated authority.
use super::codec::{ServerPlanCodec as PlanCodec, ServerPlanRevisionCodec as PlanRevisionCodec};
use super::schema::{
    PLAN_RECORD_SIZE, PLAN_REVISION_RECORD_SIZE, REMOTE_PLAN_LAYOUT_ID, REMOTE_PLAN_RECORD_SIZE,
    REMOTE_PLAN_REVISION_RECORD_SIZE,
};
use crate::foundation::remote_binary_db::{BinaryDbError, StoreResult};

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
                let record =
                    PlanCodec::<1>::decode_record(bytes).map_err(BinaryDbError::unsupported)?;
                target_plan.extend(
                    PlanCodec::<2>::encode_record(&record).map_err(BinaryDbError::unsupported)?,
                );
            }
            let mut target_revisions = REMOTE_PLAN_LAYOUT_ID.to_le_bytes().to_vec();
            for bytes in revisions[4..].chunks_exact(PLAN_REVISION_RECORD_SIZE as usize) {
                let record = PlanRevisionCodec::<1>::decode_record(bytes)
                    .map_err(BinaryDbError::unsupported)?;
                target_revisions.extend(
                    PlanRevisionCodec::<2>::encode_record(&record)
                        .map_err(BinaryDbError::unsupported)?,
                );
            }
            Ok((target_plan, target_revisions))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old_files() -> (Vec<u8>, Vec<u8>) {
        (
            [1u32.to_le_bytes().as_slice(), &[0; 48]].concat(),
            [1u32.to_le_bytes().as_slice(), &[0; 56]].concat(),
        )
    }

    #[test]
    fn frozen_core_fixture_preserves_every_prefix_and_unknown_tail() {
        let (mut plan, mut revisions) = old_files();
        for (index, byte) in plan[4..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        for (index, byte) in revisions[4..].iter_mut().enumerate() {
            *byte = index as u8 + 60;
        }
        let (target_plan, target_revisions) =
            convert_remote_plan_metadata(&plan, &revisions).unwrap();
        assert_eq!(&target_plan[..4], &2u32.to_le_bytes());
        assert_eq!(&target_plan[4..52], &plan[4..]);
        assert_eq!(&target_revisions[4..60], &revisions[4..]);
        assert_eq!(&target_plan[52..], &[0; 4]);
        assert_eq!(&target_revisions[60..], &[0; 4]);
        assert_eq!(
            convert_remote_plan_metadata(&target_plan, &target_revisions).unwrap(),
            (target_plan, target_revisions)
        );
    }

    #[test]
    fn frozen_core_fixture_keeps_layout1_and_exact_remote_actor_tails() {
        let (plan, revisions) = old_files();
        let mut record = PlanCodec::<1>::decode_record(&plan[4..]).unwrap();
        let mut revision = PlanRevisionCodec::<1>::decode_record(&revisions[4..]).unwrap();
        assert_eq!(PlanCodec::<1>::encode_record(&record).unwrap(), plan[4..]);
        assert_eq!(
            PlanRevisionCodec::<1>::encode_record(&revision).unwrap(),
            revisions[4..]
        );
        record.actor_index_plus1 = 1;
        revision.actor_index_plus1 = u32::MAX;
        assert_eq!(
            &PlanCodec::<2>::encode_record(&record).unwrap()[48..],
            &1u32.to_le_bytes()
        );
        assert_eq!(
            &PlanRevisionCodec::<2>::encode_record(&revision).unwrap()[56..],
            &u32::MAX.to_le_bytes()
        );
        assert!(PlanCodec::<1>::encode_record(&record).is_err());
        assert!(PlanRevisionCodec::<1>::encode_record(&revision).is_err());
    }

    #[test]
    fn frozen_core_fixture_rejects_partial_truncated_and_wrong_width_profiles() {
        let (plan, revisions) = old_files();
        let (target_plan, target_revisions) =
            convert_remote_plan_metadata(&plan, &revisions).unwrap();
        for (a, b) in [(&plan, &target_revisions), (&target_plan, &revisions)] {
            assert!(classify_remote_plan_metadata(a, b).is_err());
        }
        for bytes in [
            vec![],
            vec![1, 0, 0],
            vec![3, 0, 0, 0],
            [vec![2, 0, 0, 0], vec![0; 48]].concat(),
        ] {
            assert!(classify_remote_plan_metadata(&bytes, &target_revisions).is_err());
        }
        assert_eq!(
            convert_remote_plan_metadata(&1u32.to_le_bytes(), &1u32.to_le_bytes()).unwrap(),
            (2u32.to_le_bytes().to_vec(), 2u32.to_le_bytes().to_vec())
        );
    }
}
