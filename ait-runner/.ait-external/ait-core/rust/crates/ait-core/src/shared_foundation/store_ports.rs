pub trait ConnectionManager {
    type Error;
    type LeaseMode;
    type LeaseReceipt;
    type Stats;

    fn inspect(&self) -> Self::Stats;
    fn acquire(&self, mode: Self::LeaseMode) -> Result<Self::LeaseReceipt, Self::Error>;
    fn release(&self, lease_id: u64) -> Result<Self::Stats, Self::Error>;
    fn close(&self) -> Result<Self::Stats, Self::Error>;
}

pub trait StorageReadinessProbe {
    type Error;
    type Output;

    fn inspect_storage_readiness(&self) -> Result<Self::Output, Self::Error>;
}
