use serde_json::Value as JsonValue;

use crate::foundation::ci_command_bundle;
use crate::foundation::main_seed_prewarm;
use crate::foundation::patchset_ci_runtime;
use crate::foundation::repo_ci_runtime;
use crate::foundation::test_shard_runtime;
use crate::foundation::test_shards;

pub struct PatchsetCiRunJson<S> {
    store: S,
}

impl<S> PatchsetCiRunJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl PatchsetCiRunJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> PatchsetCiRunJson<S> {
    pub fn run(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        patchset_ci_runtime::patchset_ci_run_json_impl(request)
    }
}

pub struct RepoCiRunJson<S> {
    store: S,
}

impl<S> RepoCiRunJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RepoCiRunJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> RepoCiRunJson<S> {
    pub fn run(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        repo_ci_runtime::repo_ci_run_json_impl(request)
    }
}

pub struct TestShardPlanJson<S> {
    store: S,
}

impl<S> TestShardPlanJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl TestShardPlanJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> TestShardPlanJson<S> {
    pub fn plan(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        test_shards::ci_test_shard_plan_json_impl(request)
    }

    pub fn prepare(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        test_shard_runtime::ci_test_shard_prepare_json_impl(request)
    }

    pub fn cleanup(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        test_shard_runtime::ci_test_shard_cleanup_json_impl(request)
    }
}

pub struct CommandBundleRunJson<S> {
    store: S,
}

impl<S> CommandBundleRunJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl CommandBundleRunJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> CommandBundleRunJson<S> {
    pub fn run(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        ci_command_bundle::ci_command_bundle_run_json_impl(request)
    }
}

pub struct MainSeedPrewarmJson<S> {
    store: S,
}

impl<S> MainSeedPrewarmJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl MainSeedPrewarmJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> MainSeedPrewarmJson<S> {
    pub fn prewarm(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        main_seed_prewarm::ci_main_seed_prewarm_json_impl(request)
    }

    pub fn prewarm_for_plan(
        &self,
        request: &JsonValue,
        plan: &JsonValue,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        main_seed_prewarm::ci_main_seed_prewarm_for_plan_json_impl(request, plan)
    }
}
