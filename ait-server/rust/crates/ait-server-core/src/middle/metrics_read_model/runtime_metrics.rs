use super::live_turn_pressure::{
    live_turn_pressure_summary_from_normalized, normalize_live_turn_metrics,
};
use super::*;

pub fn runtime_metrics_read_model(input: &RuntimeMetricsInput) -> Result<JsonValue, String> {
    let live_turn_metrics = normalize_live_turn_metrics(&input.live_turn_metrics)?;
    let live_turn_pressure = live_turn_pressure_summary_from_normalized(&live_turn_metrics);
    Ok(json!({
        "live_turn_metrics": live_turn_metrics,
        "live_turn_pressure": live_turn_pressure,
    }))
}
