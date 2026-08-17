use ait_core::json_support::{json, JsonValue};

pub const LANGUAGE_BINDING_CONTRACT: &str = "ait.language.binding.v1";

pub fn language_binding_info_json() -> JsonValue {
    json!({
        "contract": LANGUAGE_BINDING_CONTRACT,
        "version": env!("CARGO_PKG_VERSION"),
        "runtime_authority": "rust",
        "python_binding": "pyo3",
        "node_binding": "napi",
        "process_transport_allowed": false,
        "supported_surfaces": [
            "ait-core",
            "ait-agent-worker",
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_info_exposes_worker_runtime_without_management_surface() {
        let payload = language_binding_info_json();

        assert_eq!(payload["contract"], LANGUAGE_BINDING_CONTRACT);
        assert_eq!(payload["python_binding"], "pyo3");
        assert_eq!(payload["node_binding"], "napi");
        assert_eq!(payload["process_transport_allowed"], false);
        assert_eq!(
            payload["supported_surfaces"],
            json!(["ait-core", "ait-agent-worker"])
        );
    }
}
