pub type SnapshotStoreResult<T> = Result<T, String>;

pub const MAX_SNAPSHOT_PARENT_COUNT: usize = 1_024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub snapshot_id: String,
    pub parent_snapshot_ids: Vec<String>,
    pub primary_parent_snapshot_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub root_tree_pack_id: Option<String>,
    pub root_entry_ordinal: Option<i64>,
    pub manifest_hash: String,
    pub message: Option<String>,
    pub line_name: String,
    pub snapshot_kind: String,
    pub file_count: i64,
    pub total_bytes: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotParentLink {
    pub snapshot_id: String,
    pub parent_snapshot_ids: Vec<String>,
    pub primary_parent_snapshot_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotParentLinkPage {
    pub links: Vec<SnapshotParentLink>,
    pub next_cursor: Option<usize>,
}

pub fn validate_snapshot_parent_set(
    snapshot_id: Option<&str>,
    parent_snapshot_ids: &[String],
    primary_parent_snapshot_id: Option<&str>,
    parent_snapshot_id: Option<&str>,
) -> SnapshotStoreResult<()> {
    if parent_snapshot_ids.len() > MAX_SNAPSHOT_PARENT_COUNT {
        return Err(format!(
            "snapshot parent count {} exceeds {MAX_SNAPSHOT_PARENT_COUNT}",
            parent_snapshot_ids.len()
        ));
    }
    let expected_primary = parent_snapshot_ids.first().map(String::as_str);
    if primary_parent_snapshot_id != expected_primary {
        return Err(format!(
            "primary_parent_snapshot_id projection {:?} does not match ordered parent ordinal 0 {:?}",
            primary_parent_snapshot_id, expected_primary
        ));
    }
    if parent_snapshot_id != expected_primary {
        return Err(format!(
            "parent_snapshot_id compatibility projection {:?} does not match ordered parent ordinal 0 {:?}",
            parent_snapshot_id, expected_primary
        ));
    }

    let mut seen = std::collections::BTreeSet::new();
    for (ordinal, parent_id) in parent_snapshot_ids.iter().enumerate() {
        if parent_id.trim().is_empty() || parent_id.trim() != parent_id {
            return Err(format!(
                "parent snapshot ordinal {ordinal} must be non-empty canonical text"
            ));
        }
        if snapshot_id.is_some_and(|child| child.eq_ignore_ascii_case(parent_id)) {
            return Err(format!(
                "snapshot {} cannot name itself as parent ordinal {ordinal}",
                snapshot_id.unwrap_or_default()
            ));
        }
        if !seen.insert(parent_id.to_ascii_lowercase()) {
            return Err(format!(
                "duplicate parent snapshot {parent_id} at ordinal {ordinal}"
            ));
        }
    }
    Ok(())
}

pub fn compatibility_parent_projections(
    parent_snapshot_ids: &[String],
) -> (Option<String>, Option<String>) {
    let primary = parent_snapshot_ids.first().cloned();
    (primary.clone(), primary)
}

pub fn normalize_snapshot_parent_set(
    snapshot_id: Option<&str>,
    parent_snapshot_ids: Option<Vec<String>>,
    primary_parent_snapshot_id: Option<String>,
    parent_snapshot_id: Option<String>,
) -> SnapshotStoreResult<(Vec<String>, Option<String>, Option<String>)> {
    let parent_snapshot_ids = match parent_snapshot_ids {
        Some(parent_snapshot_ids) => parent_snapshot_ids,
        None => primary_parent_snapshot_id
            .clone()
            .or_else(|| parent_snapshot_id.clone())
            .into_iter()
            .collect(),
    };
    let (expected_primary, expected_compatibility) =
        compatibility_parent_projections(&parent_snapshot_ids);
    if primary_parent_snapshot_id
        .as_ref()
        .is_some_and(|value| Some(value.as_str()) != expected_primary.as_deref())
    {
        return Err(format!(
            "primary_parent_snapshot_id projection {:?} does not match ordered parent ordinal 0 {:?}",
            primary_parent_snapshot_id, expected_primary
        ));
    }
    if parent_snapshot_id
        .as_ref()
        .is_some_and(|value| Some(value.as_str()) != expected_compatibility.as_deref())
    {
        return Err(format!(
            "parent_snapshot_id compatibility projection {:?} does not match ordered parent ordinal 0 {:?}",
            parent_snapshot_id, expected_compatibility
        ));
    }
    validate_snapshot_parent_set(
        snapshot_id,
        &parent_snapshot_ids,
        expected_primary.as_deref(),
        expected_compatibility.as_deref(),
    )?;
    Ok((
        parent_snapshot_ids,
        expected_primary,
        expected_compatibility,
    ))
}

pub trait SnapshotStore {
    fn snapshot_exists(&self, snapshot_id: &str) -> SnapshotStoreResult<bool>;
    fn snapshot_parent_link(
        &self,
        snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<SnapshotParentLink>>;
    fn snapshot_parent_links(
        &self,
        snapshot_ids: &[String],
    ) -> SnapshotStoreResult<Vec<Option<SnapshotParentLink>>> {
        snapshot_ids
            .iter()
            .map(|snapshot_id| self.snapshot_parent_link(snapshot_id))
            .collect()
    }
    fn snapshot_parent_link_page(
        &self,
        _cursor: usize,
        _limit: usize,
    ) -> SnapshotStoreResult<SnapshotParentLinkPage> {
        Err("Snapshot parent-link paging is unavailable for this store.".to_string())
    }
    fn snapshot_by_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<SnapshotRecord>>;
    fn list_line_snapshots(&self) -> SnapshotStoreResult<Vec<SnapshotRecord>>;
    fn snapshot_total_bytes(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<i64>>;
    fn snapshot_root_tree_pack_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<String>>;
    fn snapshot_kind(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<String>>;
    fn snapshot_chain(&self, snapshot_id: &str) -> SnapshotStoreResult<Vec<String>>;
    fn set_snapshot_kind(
        &self,
        snapshot_id: &str,
        snapshot_kind: &str,
    ) -> SnapshotStoreResult<usize>;
}

pub fn snapshot_exists_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> SnapshotStoreResult<bool>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_exists(snapshot_id)
}

pub fn snapshot_parent_link_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> SnapshotStoreResult<Option<SnapshotParentLink>>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_parent_link(snapshot_id)
}

pub fn snapshot_parent_links_with_snapshot_store<S>(
    store: &S,
    snapshot_ids: &[String],
) -> SnapshotStoreResult<Vec<Option<SnapshotParentLink>>>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_parent_links(snapshot_ids)
}

pub fn snapshot_parent_link_page_with_snapshot_store<S>(
    store: &S,
    cursor: usize,
    limit: usize,
) -> SnapshotStoreResult<SnapshotParentLinkPage>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_parent_link_page(cursor, limit)
}

pub fn snapshot_by_id_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> SnapshotStoreResult<Option<SnapshotRecord>>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_by_id(snapshot_id)
}

pub fn list_line_snapshots_with_snapshot_store<S>(
    store: &S,
) -> SnapshotStoreResult<Vec<SnapshotRecord>>
where
    S: SnapshotStore + ?Sized,
{
    store.list_line_snapshots()
}

pub fn snapshot_total_bytes_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> SnapshotStoreResult<Option<i64>>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_total_bytes(snapshot_id)
}

pub fn snapshot_root_tree_pack_id_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> SnapshotStoreResult<Option<String>>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_root_tree_pack_id(snapshot_id)
}

pub fn snapshot_kind_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> SnapshotStoreResult<Option<String>>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_kind(snapshot_id)
}

pub fn snapshot_chain_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> SnapshotStoreResult<Vec<String>>
where
    S: SnapshotStore + ?Sized,
{
    store.snapshot_chain(snapshot_id)
}

pub fn set_snapshot_kind_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
    snapshot_kind: &str,
) -> SnapshotStoreResult<usize>
where
    S: SnapshotStore + ?Sized,
{
    store.set_snapshot_kind(snapshot_id, snapshot_kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_parent_contract_accepts_many_parents_and_derives_compatibility_projections() {
        let parents = vec![
            "SNP-A".to_string(),
            "SNP-B".to_string(),
            "SNP-C".to_string(),
        ];
        let normalized =
            normalize_snapshot_parent_set(Some("SNP-CHILD"), Some(parents.clone()), None, None)
                .expect("ordered parent set");
        assert_eq!(normalized.0, parents);
        assert_eq!(normalized.1.as_deref(), Some("SNP-A"));
        assert_eq!(normalized.2, normalized.1);
    }

    #[test]
    fn ordered_parent_contract_rejects_duplicate_self_and_conflicting_projections() {
        let duplicate = normalize_snapshot_parent_set(
            Some("SNP-CHILD"),
            Some(vec!["SNP-A".to_string(), "snp-a".to_string()]),
            None,
            None,
        )
        .expect_err("duplicate parent ids are case-insensitive");
        assert!(duplicate.contains("duplicate parent"));

        let self_parent = normalize_snapshot_parent_set(
            Some("SNP-CHILD"),
            Some(vec!["snp-child".to_string()]),
            None,
            None,
        )
        .expect_err("self parent");
        assert!(self_parent.contains("cannot name itself"));

        let conflicting = normalize_snapshot_parent_set(
            Some("SNP-CHILD"),
            Some(vec!["SNP-A".to_string(), "SNP-B".to_string()]),
            Some("SNP-B".to_string()),
            Some("SNP-A".to_string()),
        )
        .expect_err("primary projection must be ordinal zero");
        assert!(conflicting.contains("primary_parent_snapshot_id"));

        let flattened_root = normalize_snapshot_parent_set(
            Some("SNP-CHILD"),
            Some(Vec::new()),
            None,
            Some("SNP-HIDDEN".to_string()),
        )
        .expect_err("explicit empty array cannot hide a legacy parent");
        assert!(flattened_root.contains("parent_snapshot_id compatibility projection"));
    }
}
