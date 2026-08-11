use super::env_ports::RuntimeConfigEnvironmentSource;
use std::env;

pub(super) struct ProcessRuntimeConfigEnvironmentSource;

impl RuntimeConfigEnvironmentSource for ProcessRuntimeConfigEnvironmentSource {
    fn env_value(&self, name: &str) -> Option<String> {
        env::var_os(name).map(|_| env::var(name).unwrap_or_default())
    }
}
