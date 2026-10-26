pub type StashStoreResult<T> = Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashRecord {
    pub stash_id: String,
    pub snapshot_id: String,
    pub source_line_name: String,
    pub base_snapshot_id: Option<String>,
    pub message: Option<String>,
    pub workspace_cleared: bool,
    pub created_at: String,
    pub snapshot_created_at: String,
    pub snapshot_kind: String,
    pub parent_snapshot_id: Option<String>,
    pub file_count: i64,
    pub total_bytes: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewStashRecord<'a> {
    pub stash_id: &'a str,
    pub snapshot_id: &'a str,
    pub source_line_name: &'a str,
    pub base_snapshot_id: Option<&'a str>,
    pub message: Option<&'a str>,
    pub workspace_cleared: bool,
    pub created_at: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedStashRecord {
    pub stash: StashRecord,
    pub snapshot_deleted: bool,
}

pub trait StashStore {
    fn create_stash(&self, record: NewStashRecord<'_>) -> StashStoreResult<StashRecord>;
    fn list_stashes(&self) -> StashStoreResult<Vec<StashRecord>>;
    fn stash_by_id(&self, stash_id: &str) -> StashStoreResult<Option<StashRecord>>;
    fn drop_stash(&self, stash_id: &str) -> StashStoreResult<Option<DroppedStashRecord>>;
}

pub fn list_stashes_with_stash_store<S>(store: &S) -> StashStoreResult<Vec<StashRecord>>
where
    S: StashStore + ?Sized,
{
    store.list_stashes()
}

pub fn stash_by_id_with_stash_store<S>(
    store: &S,
    stash_id: &str,
) -> StashStoreResult<Option<StashRecord>>
where
    S: StashStore + ?Sized,
{
    store.stash_by_id(stash_id)
}

pub fn drop_stash_with_stash_store<S>(
    store: &S,
    stash_id: &str,
) -> StashStoreResult<Option<DroppedStashRecord>>
where
    S: StashStore + ?Sized,
{
    store.drop_stash(stash_id)
}
