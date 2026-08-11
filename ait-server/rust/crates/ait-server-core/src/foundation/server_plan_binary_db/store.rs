use super::*;

#[derive(Clone, Debug)]
pub(super) struct ServerPlanBinaryDbStore<
    D,
    const WRITE_LAYOUT: u32 = SERVER_PLAN_BINARY_DB_LAYOUT_V1,
> where
    D: ServerRemoteBinaryDb + Clone,
{
    pub(super) db: D,
}

impl<D, const WRITE_LAYOUT: u32> ServerPlanBinaryDbStore<D, WRITE_LAYOUT>
where
    D: ServerRemoteBinaryDb + Clone,
{
    pub(super) fn new(db: D) -> Self {
        Self { db }
    }

    pub(super) fn db(&self) -> &D {
        &self.db
    }
}
