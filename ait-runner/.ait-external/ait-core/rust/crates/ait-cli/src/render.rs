use crate::json_support::{encode_value_or, encode_value_pretty};
use ait_core::json_support::JsonValue;

pub fn print_json(value: &JsonValue) -> Result<(), String> {
    println!("{}", encode_value_pretty(value, "Failed to encode JSON")?);
    Ok(())
}

pub fn print_list(rows: &[JsonValue], columns: &[&str]) {
    println!("{}", columns.join("\t"));
    for row in rows {
        let Some(obj) = row.as_object() else {
            println!("{}", row);
            continue;
        };
        let cells = columns
            .iter()
            .map(|name| public_cell(name, obj.get(*name)))
            .collect::<Vec<_>>();
        println!("{}", cells.join("\t"));
    }
}

pub fn print_key_values(title: &str, rows: &[(&str, String)]) {
    let public_title = title
        .strip_prefix("ait-cli")
        .map(|suffix| format!("ait{suffix}"))
        .unwrap_or_else(|| title.to_string());
    println!("{public_title}");
    for (key, value) in rows.iter().filter(|(_, value)| !value.trim().is_empty()) {
        let value = if key.ends_with("patchset_id") {
            ait_core::public_references::public_patchset_reference(value)
        } else {
            ait_core::public_references::public_work_reference_text(value)
        };
        println!("{key}: {value}");
    }
}

fn public_cell(name: &str, value: Option<&JsonValue>) -> String {
    let value = cell(value);
    if name.ends_with("patchset_id") {
        ait_core::public_references::public_patchset_reference(&value)
    } else {
        ait_core::public_references::public_work_reference_text(&value)
    }
}

pub fn cell(value: Option<&JsonValue>) -> String {
    match value {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::String(text)) => text.clone(),
        Some(JsonValue::Bool(flag)) => flag.to_string(),
        Some(JsonValue::Number(number)) => number.to_string(),
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(|entry| cell(Some(entry)))
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(JsonValue::Object(_)) => encode_value_or(value.unwrap(), ""),
    }
}
