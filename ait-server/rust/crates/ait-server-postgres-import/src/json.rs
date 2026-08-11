use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use std::collections::BTreeSet;
use std::fmt;

pub(crate) fn parse_object_without_duplicates(
    raw: &str,
    label: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    let value = NoDuplicateValue
        .deserialize(&mut deserializer)
        .map_err(|error| format!("{label} is invalid JSON: {error}"))?;
    deserializer
        .end()
        .map_err(|error| format!("{label} has trailing JSON data: {error}"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} must be a JSON object"))
}

struct NoDuplicateValue;

impl<'de> DeserializeSeed<'de> for NoDuplicateValue {
    type Value = JsonValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateVisitor)
    }
}

struct NoDuplicateVisitor;

impl<'de> Visitor<'de> for NoDuplicateVisitor {
    type Value = JsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(JsonValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(JsonValue::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(JsonValue::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        JsonNumber::from_f64(value)
            .map(JsonValue::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(JsonValue::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(JsonValue::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(JsonValue::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(JsonValue::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        NoDuplicateValue.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(NoDuplicateValue)? {
            values.push(value);
        }
        Ok(JsonValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        let mut values = JsonMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(A::Error::custom(format!("duplicate object key {key:?}")));
            }
            values.insert(key, map.next_value_seed(NoDuplicateValue)?);
        }
        Ok(JsonValue::Object(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_keys_fail_at_every_depth() {
        assert!(parse_object_without_duplicates(r#"{"a":1,"a":2}"#, "fixture").is_err());
        assert!(parse_object_without_duplicates(r#"{"a":{"b":1,"b":2}}"#, "fixture").is_err());
        assert!(parse_object_without_duplicates(r#"{"a":1}"#, "fixture").is_ok());
    }
}
