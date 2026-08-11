use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadModelRowSetSpec {
    pub field: &'static str,
    pub required: bool,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadModelContract {
    pub domain_id: &'static str,
    pub reference_module: &'static str,
    pub payload_label: &'static str,
    pub public_surface: &'static str,
    pub output_shape: &'static str,
    pub mutates_state: bool,
    pub row_sets: &'static [ReadModelRowSetSpec],
}

impl ReadModelContract {
    pub fn row_set(&self, field: &str) -> Option<&ReadModelRowSetSpec> {
        self.row_sets.iter().find(|spec| spec.field == field)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadModelRows {
    rows: BTreeMap<String, Vec<JsonMap<String, JsonValue>>>,
}

impl ReadModelRows {
    pub fn from_payload(value: &JsonValue, contract: &ReadModelContract) -> Result<Self, String> {
        let obj = read_model_payload_object(value, contract.payload_label)?;
        Self::from_object(obj, contract)
    }

    pub fn from_object(
        obj: &JsonMap<String, JsonValue>,
        contract: &ReadModelContract,
    ) -> Result<Self, String> {
        let mut rows = BTreeMap::new();
        for spec in contract.row_sets {
            let parsed = object_row_array_for_spec(obj, spec)?;
            rows.insert(spec.field.to_string(), parsed);
        }
        Ok(Self { rows })
    }

    pub fn get(&self, field: &str) -> Option<&[JsonMap<String, JsonValue>]> {
        self.rows.get(field).map(Vec::as_slice)
    }

    pub fn take(&mut self, field: &str) -> Vec<JsonMap<String, JsonValue>> {
        self.rows.remove(field).unwrap_or_default()
    }

    pub fn counts(&self) -> BTreeMap<String, usize> {
        self.rows
            .iter()
            .map(|(field, rows)| (field.clone(), rows.len()))
            .collect()
    }
}

pub fn read_model_payload_object<'a>(
    value: &'a JsonValue,
    payload_label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{payload_label} payload must be a JSON object."))
}

pub fn object_row_array(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    let Some(value) = obj.get(field) else {
        return Ok(Vec::new());
    };
    let rows = value
        .as_array()
        .ok_or_else(|| format!("`{field}` must be an array."))?;
    rows.iter()
        .map(|row| {
            row.as_object()
                .cloned()
                .ok_or_else(|| format!("`{field}` rows must be JSON objects."))
        })
        .collect()
}

pub fn optional_text_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    obj.get(field).and_then(json_value_to_text)
}

pub fn object_text_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    optional_text_field(obj, field)
}

pub fn json_value_to_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn object_row_array_for_spec(
    obj: &JsonMap<String, JsonValue>,
    spec: &ReadModelRowSetSpec,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    if !obj.contains_key(spec.field) && spec.required {
        return Err(format!("`{}` is required.", spec.field));
    }
    object_row_array(obj, spec.field)
}
