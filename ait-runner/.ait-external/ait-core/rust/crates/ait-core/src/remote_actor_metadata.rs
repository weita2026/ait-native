//! Remote-only Actor tails and conversion kernels. Local workflow/content
//! codecs retain their existing byte layouts. No function here opens a DB.
use crate::binary_db::{BinaryDbError, BinaryFileId, StoreResult};

pub const REMOTE_ACTOR_LAYOUT_ID: u32 = 2;

/// Header sentinels select the complete supported source generation. In
/// particular, Change/Land header 2 alone cannot select their record widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteActorHeaders {
    pub plan: u32,
    pub plan_revision: u32,
    pub task: u32,
    pub change: u32,
    pub snapshot: u32,
    pub land: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteActorProfile {
    Old,
    PlanOnly,
    Actor,
}

pub fn classify_remote_actor_profile(
    headers: RemoteActorHeaders,
) -> StoreResult<RemoteActorProfile> {
    match (
        headers.plan,
        headers.plan_revision,
        headers.task,
        headers.change,
        headers.snapshot,
        headers.land,
    ) {
        (1, 1, 1, 2, 1, 2) => Ok(RemoteActorProfile::Old),
        (2, 2, 1, 2, 1, 2) => Ok(RemoteActorProfile::PlanOnly),
        (2, 2, 2, 2, 2, 2) => Ok(RemoteActorProfile::Actor),
        _ => Err(BinaryDbError::unsupported(
            "unsupported partial or mixed Remote Actor metadata profile",
        )),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteWorkflowFile {
    Task,
    Change,
    Snapshot,
    Land,
}

impl RemoteWorkflowFile {
    pub const ALL: [Self; 4] = [Self::Task, Self::Change, Self::Snapshot, Self::Land];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Task => "task.bin",
            Self::Change => "change.bin",
            Self::Snapshot => "snapshot.bin",
            Self::Land => "land.bin",
        }
    }
    pub const fn source_width(self) -> usize {
        match self {
            Self::Task => 60,
            Self::Change => 68,
            Self::Snapshot => 88,
            Self::Land => 48,
        }
    }
    pub const fn target_width(self) -> usize {
        self.source_width() + 4
    }
    pub const fn source_layout(self) -> u32 {
        match self {
            Self::Task | Self::Snapshot => 1,
            Self::Change | Self::Land => 2,
        }
    }
}

/// An all-target header profile needs no body read. Callers skip the converter
/// in that case and leave every unchanged family in place.
pub const fn workflow_conversion_required(profile: RemoteActorProfile) -> bool {
    !matches!(profile, RemoteActorProfile::Actor)
}

/// Convert only this changed fixed file. The caller owns writer exclusion,
/// complete-batch publication and durability; this kernel adds no DB proof pass.
/// None means the all-target profile or an unchanged empty header needs no
/// replacement.
pub fn convert_remote_workflow_file(
    profile: RemoteActorProfile,
    family: RemoteWorkflowFile,
    source: &[u8],
) -> StoreResult<Option<Vec<u8>>> {
    if !workflow_conversion_required(profile) {
        return Ok(None);
    }
    if source.get(..4) != Some(family.source_layout().to_le_bytes().as_slice()) {
        return Err(BinaryDbError::unsupported(format!(
            "unsupported {} source header",
            family.name()
        )));
    }
    let body = &source[4..];
    if !body.len().is_multiple_of(family.source_width()) {
        return Err(BinaryDbError::corruption(format!(
            "truncated {} source record",
            family.name()
        )));
    }
    if body.is_empty() && family.source_layout() == REMOTE_ACTOR_LAYOUT_ID {
        return Ok(None);
    }
    let count = body.len() / family.source_width();
    u32::try_from(count).map_err(|_| {
        BinaryDbError::invalid_domain_data("Remote Actor record count exceeds u32 bounds")
    })?;
    let capacity = count
        .checked_mul(family.target_width())
        .and_then(|size| size.checked_add(4))
        .ok_or_else(|| BinaryDbError::invalid_domain_data("Remote Actor metadata size overflow"))?;
    let mut output = Vec::new();
    output.try_reserve_exact(capacity).map_err(|error| {
        BinaryDbError::invalid_domain_data(format!("Remote Actor allocation failed: {error}"))
    })?;
    output.extend_from_slice(&REMOTE_ACTOR_LAYOUT_ID.to_le_bytes());
    for prefix in body.chunks_exact(family.source_width()) {
        output.extend_from_slice(prefix);
        output.extend_from_slice(&0_u32.to_le_bytes());
    }
    Ok(Some(output))
}

/// The exact schema prefix remains byte-for-byte authority; the only new
/// component is the declared nullable Actor reference at its fixed tail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteActorRecord<const PREFIX: usize> {
    pub prefix: [u8; PREFIX],
    pub actor_index_plus1: u32,
}

impl<const PREFIX: usize> RemoteActorRecord<PREFIX> {
    pub fn from_actor_key(prefix: [u8; PREFIX], actor: Option<u32>) -> StoreResult<Self> {
        let actor_index_plus1 = actor
            .map(|key| {
                key.checked_add(1).ok_or_else(|| {
                    BinaryDbError::invalid_domain_data("Actor key exceeds plus-one bounds")
                })
            })
            .transpose()?
            .unwrap_or(0);
        Ok(Self {
            prefix,
            actor_index_plus1,
        })
    }
    pub fn actor_key_index(&self) -> Option<u32> {
        self.actor_index_plus1.checked_sub(1)
    }
}

macro_rules! remote_actor_codec {
    ($codec:ident, $record:ident, $prefix:expr, $name:literal) => {
        pub type $record = RemoteActorRecord<$prefix>;
        pub struct $codec;
        impl $codec {
            pub const RECORD_SIZE: u32 = $prefix + 4;
            pub fn record_file() -> BinaryFileId {
                BinaryFileId::new($name, REMOTE_ACTOR_LAYOUT_ID, Self::RECORD_SIZE)
            }
            pub fn encode_record(record: &$record) -> Vec<u8> {
                let mut bytes = record.prefix.to_vec();
                bytes.extend_from_slice(&record.actor_index_plus1.to_le_bytes());
                bytes
            }
            pub fn decode_record(bytes: &[u8]) -> StoreResult<$record> {
                let prefix = bytes
                    .get(..$prefix)
                    .and_then(|prefix| prefix.try_into().ok());
                let tail = bytes
                    .get($prefix..)
                    .and_then(|tail| tail.try_into().ok())
                    .map(u32::from_le_bytes);
                match (prefix, tail) {
                    (Some(prefix), Some(actor_index_plus1)) => Ok(RemoteActorRecord {
                        prefix,
                        actor_index_plus1,
                    }),
                    _ => Err(BinaryDbError::corruption(concat!(
                        $name,
                        " Actor record has invalid width"
                    ))),
                }
            }
        }
    };
}

remote_actor_codec!(RemoteTaskActorCodec, RemoteTaskActorRecord, 60, "task.bin");
remote_actor_codec!(
    RemoteChangeActorCodec,
    RemoteChangeActorRecord,
    68,
    "change.bin"
);
remote_actor_codec!(
    RemoteSnapshotActorCodec,
    RemoteSnapshotActorRecord,
    88,
    "snapshot.bin"
);
remote_actor_codec!(RemoteLandActorCodec, RemoteLandActorRecord, 48, "land.bin");

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(
        plan: u32,
        revision: u32,
        task: u32,
        change: u32,
        snapshot: u32,
        land: u32,
    ) -> RemoteActorHeaders {
        RemoteActorHeaders {
            plan,
            plan_revision: revision,
            task,
            change,
            snapshot,
            land,
        }
    }
    #[test]
    fn whole_profile_uses_task_snapshot_and_plan_header_sentinels() {
        assert_eq!(
            classify_remote_actor_profile(headers(1, 1, 1, 2, 1, 2)).unwrap(),
            RemoteActorProfile::Old
        );
        assert_eq!(
            classify_remote_actor_profile(headers(2, 2, 1, 2, 1, 2)).unwrap(),
            RemoteActorProfile::PlanOnly
        );
        assert_eq!(
            classify_remote_actor_profile(headers(2, 2, 2, 2, 2, 2)).unwrap(),
            RemoteActorProfile::Actor
        );
        for partial in [
            headers(2, 1, 1, 2, 1, 2),
            headers(2, 2, 2, 2, 1, 2),
            headers(2, 2, 1, 2, 2, 2),
            headers(1, 1, 2, 2, 2, 2),
            headers(2, 2, 1, 1, 1, 2),
            headers(3, 3, 3, 3, 3, 3),
        ] {
            assert!(classify_remote_actor_profile(partial).is_err());
        }
    }
    #[test]
    fn conversion_preserves_each_prefix_and_initializes_only_unknown_tails() {
        for profile in [RemoteActorProfile::Old, RemoteActorProfile::PlanOnly] {
            for file in RemoteWorkflowFile::ALL {
                let mut source = file.source_layout().to_le_bytes().to_vec();
                let empty = convert_remote_workflow_file(profile, file, &source).unwrap();
                if file.source_layout() == REMOTE_ACTOR_LAYOUT_ID {
                    assert_eq!(empty, None);
                } else {
                    assert_eq!(empty, Some(REMOTE_ACTOR_LAYOUT_ID.to_le_bytes().to_vec()));
                }
                source.extend((0..file.source_width() * 2).map(|offset| offset as u8));
                let target = convert_remote_workflow_file(profile, file, &source)
                    .unwrap()
                    .unwrap();
                assert_eq!(&target[..4], &2_u32.to_le_bytes());
                for (before, after) in source[4..]
                    .chunks_exact(file.source_width())
                    .zip(target[4..].chunks_exact(file.target_width()))
                {
                    assert_eq!(before, &after[..file.source_width()]);
                    assert_eq!(&after[file.source_width()..], &[0; 4]);
                }
                assert!(
                    convert_remote_workflow_file(profile, file, &source[..source.len() - 1])
                        .is_err()
                );
                assert!(convert_remote_workflow_file(profile, file, &[]).is_err());
                assert_eq!(
                    convert_remote_workflow_file(RemoteActorProfile::Actor, file, &[]).unwrap(),
                    None
                );
            }
        }
    }
    #[test]
    fn remote_codecs_support_unknown_zero_and_maximum_representable_actor() {
        for actor in [None, Some(0), Some(u32::MAX - 1)] {
            let record = RemoteTaskActorRecord::from_actor_key([7; 60], actor).unwrap();
            let bytes = RemoteTaskActorCodec::encode_record(&record);
            assert_eq!(bytes.len(), 64);
            assert_eq!(
                RemoteTaskActorCodec::decode_record(&bytes)
                    .unwrap()
                    .actor_key_index(),
                actor
            );
            assert!(RemoteTaskActorCodec::decode_record(&bytes[..63]).is_err());
        }
        assert!(RemoteTaskActorRecord::from_actor_key([0; 60], Some(u32::MAX)).is_err());
        assert_eq!(RemoteChangeActorCodec::record_file().record_size(), 72);
        assert_eq!(RemoteSnapshotActorCodec::record_file().record_size(), 92);
        assert_eq!(RemoteLandActorCodec::record_file().record_size(), 52);
        assert_eq!(
            crate::content_binary_db::BinarySnapshotCodec::<1>::record_file().record_size(),
            88
        );
        assert_eq!(crate::workflow_binary_db::LOCAL_TASK_RECORD_SIZE, 64);
        assert_eq!(crate::workflow_binary_db::LOCAL_CHANGE_RECORD_SIZE, 68);
        assert_eq!(crate::workflow_binary_db::LOCAL_LAND_RECORD_SIZE, 44);
    }
}
