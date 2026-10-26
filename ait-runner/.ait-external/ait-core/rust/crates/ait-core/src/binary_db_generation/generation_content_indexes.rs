use super::{GenerationFileManifest, GenerationResult, Path};
use crate::content_binary_db::{
    blob_id_from_sha256, blob_id_index_key, object_pack_id_from_hash48, object_pack_id_index_key,
    snapshot_id_from_hash48, snapshot_id_index_key, tree_id_from_hash80, tree_id_index_key,
    tree_pack_id_from_hash48, tree_pack_id_index_key, BinaryBlobCodec, BinaryObjectPackCodec,
    BinarySnapshotCodec, BinaryTreeCodec, BinaryTreePackCodec, BLOB_BIN, BLOB_ID_IDX,
    BLOB_RECORD_SIZE, OBJECT_PACK_BIN, OBJECT_PACK_ID_IDX, OBJECT_PACK_RECORD_SIZE, SNAPSHOT_BIN,
    SNAPSHOT_ID_IDX, SNAPSHOT_RECORD_SIZE, TREE_BIN, TREE_ID_IDX, TREE_PACK_BIN, TREE_PACK_ID_IDX,
    TREE_PACK_RECORD_SIZE, TREE_RECORD_SIZE,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};

const FILE_HEADER_BYTES: u64 = 4;

#[derive(Clone, Copy)]
struct ContentIdentityIndexPlan {
    record_name: &'static str,
    index_name: &'static str,
    record_size: u32,
    key_size: usize,
    key_from_record: fn(&[u8]) -> GenerationResult<Vec<u8>>,
}

const CONTENT_IDENTITY_INDEX_PLANS: &[ContentIdentityIndexPlan] = &[
    ContentIdentityIndexPlan {
        record_name: BLOB_BIN,
        index_name: BLOB_ID_IDX,
        record_size: BLOB_RECORD_SIZE,
        key_size: 10,
        key_from_record: blob_key,
    },
    ContentIdentityIndexPlan {
        record_name: SNAPSHOT_BIN,
        index_name: SNAPSHOT_ID_IDX,
        record_size: SNAPSHOT_RECORD_SIZE,
        key_size: 8,
        key_from_record: snapshot_key,
    },
    ContentIdentityIndexPlan {
        record_name: OBJECT_PACK_BIN,
        index_name: OBJECT_PACK_ID_IDX,
        record_size: OBJECT_PACK_RECORD_SIZE,
        key_size: 8,
        key_from_record: object_pack_key,
    },
    ContentIdentityIndexPlan {
        record_name: TREE_BIN,
        index_name: TREE_ID_IDX,
        record_size: TREE_RECORD_SIZE,
        key_size: 10,
        key_from_record: tree_key,
    },
    ContentIdentityIndexPlan {
        record_name: TREE_PACK_BIN,
        index_name: TREE_PACK_ID_IDX,
        record_size: TREE_PACK_RECORD_SIZE,
        key_size: 8,
        key_from_record: tree_pack_key,
    },
];

pub(super) fn is_content_identity_index(name: &str) -> bool {
    CONTENT_IDENTITY_INDEX_PLANS
        .iter()
        .any(|plan| plan.index_name == name)
}

pub(super) fn rebuild_content_identity_indexes(
    authority_root: &Path,
) -> GenerationResult<Vec<GenerationFileManifest>> {
    let mut manifests = Vec::new();
    for plan in CONTENT_IDENTITY_INDEX_PLANS {
        let Some(record_count) = record_count(authority_root, *plan)? else {
            let index_path = authority_root.join(plan.index_name);
            if index_path.exists() {
                return Err(format!(
                    "cannot rebuild orphaned content index {} without {}",
                    index_path.display(),
                    plan.record_name
                ));
            }
            continue;
        };
        manifests.push(rebuild_content_identity_index(
            authority_root,
            *plan,
            record_count,
        )?);
    }
    Ok(manifests)
}

pub(super) fn validate_content_identity_indexes(authority_root: &Path) -> GenerationResult<()> {
    for plan in CONTENT_IDENTITY_INDEX_PLANS {
        let Some(record_count) = record_count(authority_root, *plan)? else {
            let index_path = authority_root.join(plan.index_name);
            if index_path.exists() {
                return Err(format!(
                    "Binary DB content index {} exists without authoritative {}",
                    index_path.display(),
                    plan.record_name
                ));
            }
            continue;
        };
        validate_content_identity_index(authority_root, *plan, record_count)?;
    }
    Ok(())
}

fn rebuild_content_identity_index(
    authority_root: &Path,
    plan: ContentIdentityIndexPlan,
    record_count: u32,
) -> GenerationResult<GenerationFileManifest> {
    let record_path = authority_root.join(plan.record_name);
    let index_path = authority_root.join(plan.index_name);
    let mut records = fs::File::open(&record_path)
        .map_err(|error| format!("failed to open {}: {error}", record_path.display()))?;
    records
        .seek(SeekFrom::Start(FILE_HEADER_BYTES))
        .map_err(|error| format!("failed to seek {}: {error}", record_path.display()))?;
    let mut index = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&index_path)
        .map_err(|error| format!("failed to rebuild {}: {error}", index_path.display()))?;
    let header = 1_u32.to_le_bytes();
    index
        .write_all(&header)
        .map_err(|error| format!("failed to write {}: {error}", index_path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(header);
    let mut record = vec![0_u8; plan.record_size as usize];
    for record_index in 0..record_count {
        records.read_exact(&mut record).map_err(|error| {
            format!(
                "failed to read {} record {record_index}: {error}",
                record_path.display()
            )
        })?;
        let key = (plan.key_from_record)(&record)?;
        if key.len() != plan.key_size {
            return Err(format!(
                "rebuilt {} key for record {record_index} has {} bytes, expected {}",
                plan.index_name,
                key.len(),
                plan.key_size
            ));
        }
        let stored_index = record_index.checked_add(1).ok_or_else(|| {
            format!(
                "{} record count exceeds its u32 plus-one index capacity",
                plan.record_name
            )
        })?;
        index
            .write_all(&key)
            .and_then(|_| index.write_all(&stored_index.to_le_bytes()))
            .map_err(|error| format!("failed to write {}: {error}", index_path.display()))?;
        hasher.update(&key);
        hasher.update(stored_index.to_le_bytes());
    }
    index
        .sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", index_path.display()))?;
    let record_width = u64::try_from(plan.key_size + 4)
        .map_err(|_| format!("{} record width overflows u64", plan.index_name))?;
    let byte_size = FILE_HEADER_BYTES
        .checked_add(
            u64::from(record_count)
                .checked_mul(record_width)
                .ok_or_else(|| format!("{} byte size overflows u64", plan.index_name))?,
        )
        .ok_or_else(|| format!("{} byte size overflows u64", plan.index_name))?;
    Ok(GenerationFileManifest {
        relative_path: format!("local/{}", plan.index_name),
        byte_size,
        sha256: hex_lower(&hasher.finalize()),
        record_count: None,
    })
}

fn validate_content_identity_index(
    authority_root: &Path,
    plan: ContentIdentityIndexPlan,
    record_count: u32,
) -> GenerationResult<()> {
    let record_path = authority_root.join(plan.record_name);
    let index_path = authority_root.join(plan.index_name);
    let index_metadata = fs::metadata(&index_path).map_err(|error| {
        format!(
            "Binary DB content index {} is missing or unreadable: {error}",
            index_path.display()
        )
    })?;
    let index_record_size = u64::try_from(plan.key_size + 4)
        .map_err(|_| format!("{} record width overflows u64", plan.index_name))?;
    let expected_size = FILE_HEADER_BYTES
        .checked_add(
            u64::from(record_count)
                .checked_mul(index_record_size)
                .ok_or_else(|| format!("{} byte size overflows u64", plan.index_name))?,
        )
        .ok_or_else(|| format!("{} byte size overflows u64", plan.index_name))?;
    if index_metadata.len() != expected_size {
        return Err(format!(
            "Binary DB content index {} has {} bytes, expected {expected_size} canonical fixed-key bytes for {record_count} records.",
            index_path.display(),
            index_metadata.len()
        ));
    }

    let mut records = fs::File::open(&record_path)
        .map_err(|error| format!("failed to open {}: {error}", record_path.display()))?;
    let mut index = fs::File::open(&index_path)
        .map_err(|error| format!("failed to open {}: {error}", index_path.display()))?;
    validate_layout_header(&mut records, &record_path)?;
    validate_layout_header(&mut index, &index_path)?;
    let mut record = vec![0_u8; plan.record_size as usize];
    let mut indexed_key = vec![0_u8; plan.key_size];
    let mut indexed_ordinal = [0_u8; 4];
    for record_index in 0..record_count {
        records.read_exact(&mut record).map_err(|error| {
            format!(
                "failed to read {} record {record_index}: {error}",
                record_path.display()
            )
        })?;
        let expected_key = (plan.key_from_record)(&record)?;
        index.read_exact(&mut indexed_key).map_err(|error| {
            format!(
                "failed to read {} key {record_index}: {error}",
                index_path.display()
            )
        })?;
        index.read_exact(&mut indexed_ordinal).map_err(|error| {
            format!(
                "failed to read {} ordinal {record_index}: {error}",
                index_path.display()
            )
        })?;
        if indexed_key != expected_key {
            return Err(format!(
                "Binary DB content index {} key {record_index} does not match authoritative {} record {record_index}.",
                index_path.display(),
                plan.record_name
            ));
        }
        let stored_index = u32::from_le_bytes(indexed_ordinal);
        let expected_index = record_index.checked_add(1).ok_or_else(|| {
            format!(
                "{} record count exceeds its u32 plus-one index capacity",
                plan.record_name
            )
        })?;
        if stored_index != expected_index {
            return Err(format!(
                "Binary DB content index {} candidate {record_index} stores plus-one ordinal {stored_index}, expected {expected_index}.",
                index_path.display()
            ));
        }
    }
    Ok(())
}

fn record_count(
    authority_root: &Path,
    plan: ContentIdentityIndexPlan,
) -> GenerationResult<Option<u32>> {
    let path = authority_root.join(plan.record_name);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    let mut file = fs::File::open(&path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    validate_layout_header(&mut file, &path)?;
    let body_size = metadata
        .len()
        .checked_sub(FILE_HEADER_BYTES)
        .ok_or_else(|| format!("{} is shorter than its layout header", path.display()))?;
    if body_size % u64::from(plan.record_size) != 0 {
        return Err(format!(
            "{} is not aligned to {}-byte records",
            path.display(),
            plan.record_size
        ));
    }
    u32::try_from(body_size / u64::from(plan.record_size))
        .map(Some)
        .map_err(|_| format!("{} record count exceeds u32::MAX", path.display()))
}

fn validate_layout_header(file: &mut fs::File, path: &Path) -> GenerationResult<()> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to seek {}: {error}", path.display()))?;
    let mut header = [0_u8; 4];
    file.read_exact(&mut header)
        .map_err(|error| format!("failed to read {} layout header: {error}", path.display()))?;
    let layout = u32::from_le_bytes(header);
    if layout != 1 {
        return Err(format!(
            "Binary DB content index closure requires layout_id 1, found {layout} at {}",
            path.display()
        ));
    }
    Ok(())
}

fn blob_key(raw: &[u8]) -> GenerationResult<Vec<u8>> {
    let record = BinaryBlobCodec::<1>::decode_record(raw)
        .map_err(|error| format!("cannot decode Blob while rebuilding its index: {error}"))?;
    blob_id_index_key(&blob_id_from_sha256(&record.sha256))
        .map(|key| key.to_vec())
        .map_err(|error| format!("cannot derive Blob index key: {error}"))
}

fn snapshot_key(raw: &[u8]) -> GenerationResult<Vec<u8>> {
    let record = BinarySnapshotCodec::<1>::decode_record(raw)
        .map_err(|error| format!("cannot decode Snapshot while rebuilding its index: {error}"))?;
    snapshot_id_index_key(&snapshot_id_from_hash48(record.snapshot_hash48))
        .map(|key| key.to_vec())
        .map_err(|error| format!("cannot derive Snapshot index key: {error}"))
}

fn object_pack_key(raw: &[u8]) -> GenerationResult<Vec<u8>> {
    let record = BinaryObjectPackCodec::<1>::decode_record(raw).map_err(|error| {
        format!("cannot decode object pack while rebuilding its index: {error}")
    })?;
    object_pack_id_index_key(&object_pack_id_from_hash48(pack_hash48(
        record.pack_hash_hi16,
        record.pack_hash_lo32,
    )))
    .map(|key| key.to_vec())
    .map_err(|error| format!("cannot derive object-pack index key: {error}"))
}

fn tree_key(raw: &[u8]) -> GenerationResult<Vec<u8>> {
    let record = BinaryTreeCodec::<1>::decode_record(raw)
        .map_err(|error| format!("cannot decode Tree while rebuilding its index: {error}"))?;
    tree_id_index_key(&tree_id_from_hash80(&record.tree_hash80))
        .map(|key| key.to_vec())
        .map_err(|error| format!("cannot derive Tree index key: {error}"))
}

fn tree_pack_key(raw: &[u8]) -> GenerationResult<Vec<u8>> {
    let record = BinaryTreePackCodec::<1>::decode_record(raw)
        .map_err(|error| format!("cannot decode tree pack while rebuilding its index: {error}"))?;
    tree_pack_id_index_key(&tree_pack_id_from_hash48(pack_hash48(
        record.pack_hash_hi16,
        record.pack_hash_lo32,
    )))
    .map(|key| key.to_vec())
    .map_err(|error| format!("cannot derive tree-pack index key: {error}"))
}

fn pack_hash48(hi16: u16, lo32: u32) -> u64 {
    (u64::from(hi16) << 32) | u64::from(lo32)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_binary_db::BinaryTreeRecord;
    use tempfile::TempDir;

    fn write_tree_record(authority: &Path, hash: [u8; 10]) {
        let record = BinaryTreeRecord {
            tree_meta: 0,
            reserved0: 0,
            pack_entry_ordinal: 0,
            entry_count: 0,
            tree_hash80: hash,
        };
        let mut bytes = 1_u32.to_le_bytes().to_vec();
        bytes.extend(BinaryTreeCodec::<1>::encode_record(&record).unwrap());
        fs::write(authority.join(TREE_BIN), bytes).unwrap();
    }

    #[test]
    fn rebuild_replaces_historical_variable_key_index_with_canonical_fixed_index() {
        let temp = TempDir::new().unwrap();
        let authority = temp.path().join("binary-db");
        fs::create_dir_all(&authority).unwrap();
        let hash = [0x29; 10];
        write_tree_record(&authority, hash);
        let mut legacy = 1_u32.to_le_bytes().to_vec();
        legacy.extend(10_u32.to_le_bytes());
        legacy.extend(hash);
        legacy.extend(0_u32.to_le_bytes());
        fs::write(authority.join(TREE_ID_IDX), legacy).unwrap();

        let error = validate_content_identity_indexes(&authority).unwrap_err();
        assert!(error.contains("canonical fixed-key bytes"));

        let manifests = rebuild_content_identity_indexes(&authority).unwrap();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].relative_path, "local/tree_id.idx");
        let mut expected = 1_u32.to_le_bytes().to_vec();
        expected.extend(hash);
        expected.extend(1_u32.to_le_bytes());
        assert_eq!(fs::read(authority.join(TREE_ID_IDX)).unwrap(), expected);
        validate_content_identity_indexes(&authority).unwrap();
    }

    #[test]
    fn rebuild_preserves_an_already_canonical_index_byte_for_byte() {
        let temp = TempDir::new().unwrap();
        let authority = temp.path().join("binary-db");
        fs::create_dir_all(&authority).unwrap();
        let hash = [0x2f; 10];
        write_tree_record(&authority, hash);
        let mut canonical = 1_u32.to_le_bytes().to_vec();
        canonical.extend(hash);
        canonical.extend(1_u32.to_le_bytes());
        fs::write(authority.join(TREE_ID_IDX), &canonical).unwrap();

        rebuild_content_identity_indexes(&authority).unwrap();
        assert_eq!(fs::read(authority.join(TREE_ID_IDX)).unwrap(), canonical);
        validate_content_identity_indexes(&authority).unwrap();
    }

    #[test]
    fn validation_rejects_missing_and_wrong_plus_one_candidates() {
        let temp = TempDir::new().unwrap();
        let authority = temp.path().join("binary-db");
        fs::create_dir_all(&authority).unwrap();
        let hash = [0x31; 10];
        write_tree_record(&authority, hash);

        let missing = validate_content_identity_indexes(&authority).unwrap_err();
        assert!(missing.contains("missing or unreadable"));

        let mut wrong = 1_u32.to_le_bytes().to_vec();
        wrong.extend(hash);
        wrong.extend(2_u32.to_le_bytes());
        fs::write(authority.join(TREE_ID_IDX), wrong).unwrap();
        let error = validate_content_identity_indexes(&authority).unwrap_err();
        assert!(error.contains("stores plus-one ordinal 2, expected 1"));
    }
}
