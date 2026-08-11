pub(super) trait RuntimeConfigEnvironmentSource {
    fn env_value(&self, name: &str) -> Option<String>;
}

pub(super) fn env_value_with_runtime_config_environment_source<S>(
    source: &S,
    name: &str,
) -> Option<String>
where
    S: RuntimeConfigEnvironmentSource + ?Sized,
{
    source.env_value(name)
}
