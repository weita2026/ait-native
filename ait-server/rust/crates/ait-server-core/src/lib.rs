pub mod foundation;
pub mod middle;

#[cfg(feature = "perfetto-tracing")]
pub mod perfetto_trace;

#[cfg(all(test, feature = "patch-ci-harness"))]
extern crate self as ait_server_core;
#[cfg(all(test, feature = "patch-ci-harness"))]
#[path = "../tests/patchset_integration_harness.rs"]
mod patchset_integration_harness;
#[cfg(all(
    test,
    feature = "patch-ci-harness",
    feature = "legacy-postgres-runtime"
))]
#[path = "bin/ait-server-core-postgres-driver.rs"]
mod patchset_postgres_driver;
