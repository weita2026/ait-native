use crate::json_support::{JsonMap, JsonValue};

pub(super) trait PlanCommandLocalShadowSource {
    fn local_shadow_index(&mut self) -> Result<JsonMap<String, JsonValue>, String>;
}

pub(super) fn local_shadow_index_with_plan_command_local_shadow_source<S>(
    source: &mut S,
) -> Result<JsonMap<String, JsonValue>, String>
where
    S: PlanCommandLocalShadowSource + ?Sized,
{
    source.local_shadow_index()
}

pub(super) fn local_shadow_for_plan_with_plan_command_local_shadow_source<S>(
    source: &mut S,
    plan_id: &str,
) -> Result<JsonValue, String>
where
    S: PlanCommandLocalShadowSource + ?Sized,
{
    Ok(
        local_shadow_index_with_plan_command_local_shadow_source(source)?
            .get(plan_id)
            .cloned()
            .unwrap_or(JsonValue::Null),
    )
}
