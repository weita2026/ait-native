include!("seam_contract_direct_tests/common.rs");
include!("seam_contract_direct_tests/handshake.rs");
include!("seam_contract_direct_tests/context.rs");
include!("seam_contract_direct_tests/ci.rs");
include!("seam_contract_direct_tests/scheduler.rs");
#[cfg(feature = "legacy-postgres-runtime")]
include!("seam_contract_direct_tests/worker_queue.rs");
#[cfg(feature = "legacy-postgres-runtime")]
include!("seam_contract_direct_tests/stores.rs");
include!("seam_contract_direct_tests/read_models.rs");
include!("seam_contract_direct_tests/fail_closed.rs");
