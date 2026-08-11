use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalContentStatsOptions {
    pub include_inventory: bool,
    pub compute_reachability: bool,
}

pub trait LocalContentStatsStore {
    fn storage_stats_with_options(
        &self,
        options: LocalContentStatsOptions,
    ) -> Result<JsonValue, String>;

    fn storage_stats(&self) -> Result<JsonValue, String> {
        self.storage_stats_with_options(LocalContentStatsOptions::default())
    }
}

pub trait LocalContentValidationStore {
    fn validate(&self) -> Result<JsonValue, String>;
}

pub trait LocalContentOrphanPackPruneStore {
    fn prune_orphan_packs(&self) -> Result<JsonValue, String>;
}

pub trait LocalContentMaintenanceStore:
    LocalContentStatsStore + LocalContentValidationStore + LocalContentOrphanPackPruneStore
{
}

impl<T> LocalContentMaintenanceStore for T where
    T: LocalContentStatsStore
        + LocalContentValidationStore
        + LocalContentOrphanPackPruneStore
        + ?Sized
{
}

impl<const WRITE_LAYOUT: u32> LocalContentStatsStore for LocalContentBinaryDb<WRITE_LAYOUT> {
    fn storage_stats_with_options(
        &self,
        options: LocalContentStatsOptions,
    ) -> Result<JsonValue, String> {
        binary_db_local_content_storage_stats(self, options)
    }
}

impl<const WRITE_LAYOUT: u32> LocalContentValidationStore for LocalContentBinaryDb<WRITE_LAYOUT> {
    fn validate(&self) -> Result<JsonValue, String> {
        let stats = binary_db_local_content_storage_stats(
            self,
            LocalContentStatsOptions {
                compute_reachability: true,
                ..LocalContentStatsOptions::default()
            },
        )?;
        Ok(storage_validation_view(&stats))
    }
}

impl<const WRITE_LAYOUT: u32> LocalContentOrphanPackPruneStore
    for LocalContentBinaryDb<WRITE_LAYOUT>
{
    fn prune_orphan_packs(&self) -> Result<JsonValue, String> {
        binary_db_prune_orphan_packs(self)
    }
}

pub fn storage_stats_with_local_content_maintenance_store<S>(store: &S) -> Result<JsonValue, String>
where
    S: LocalContentStatsStore + ?Sized,
{
    store.storage_stats()
}

pub fn validate_with_local_content_maintenance_store<S>(store: &S) -> Result<JsonValue, String>
where
    S: LocalContentValidationStore + ?Sized,
{
    store.validate()
}

pub fn prune_orphan_packs_with_local_content_maintenance_store<S>(
    store: &S,
) -> Result<JsonValue, String>
where
    S: LocalContentOrphanPackPruneStore + ?Sized,
{
    store.prune_orphan_packs()
}
