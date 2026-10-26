//! Freeze declared command/option metadata for independent language parsers.
//! This test builds Clap metadata; it never parses or executes a command.
use super::Cli;
use ait_core::json_support::{json, JsonCodec, JsonValue};
use clap::CommandFactory;
use sha2::{Digest, Sha256};

fn inventory(command: &clap::Command, parents: &[String], rows: &mut Vec<JsonValue>) {
    if command.get_name() == "help" {
        return;
    }
    let mut path = parents.to_vec();
    path.push(command.get_name().to_string());
    let args = command.get_arguments().map(|arg| json!({
        "id":arg.get_id().as_str(), "short":arg.get_short().map(|c| c.to_string()), "long":arg.get_long(),
        "aliases":arg.get_all_aliases().unwrap_or_default(), "short_aliases":arg.get_all_short_aliases().unwrap_or_default().into_iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "required":arg.is_required_set(), "hidden":arg.is_hide_set(), "index":arg.get_index(),
        "action":format!("{:?}",arg.get_action()), "num_args":arg.get_num_args().map(|n| n.to_string()),
        "default_values":arg.get_default_values().iter().map(|v| v.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "possible_values":arg.get_possible_values().iter().map(|v| v.get_name()).collect::<Vec<_>>(),
        "conflicts":command.get_arg_conflicts_with(arg).iter().map(|a| a.get_id().as_str()).collect::<Vec<_>>(),
        "exclusive":arg.is_exclusive_set(), "require_equals":arg.is_require_equals_set(),
        "allow_hyphen_values":arg.is_allow_hyphen_values_set(), "allow_negative_numbers":arg.is_allow_negative_numbers_set(),
        "trailing_var_arg":arg.is_trailing_var_arg_set(), "last":arg.is_last_set(), "ignore_case":arg.is_ignore_case_set(),
        "value_delimiter":arg.get_value_delimiter().map(|c| c.to_string()), "value_terminator":arg.get_value_terminator().map(|v| v.as_str()),
    })).map(|mut value| {
        value.as_object_mut().unwrap().retain(|_, field| {
            !field.is_null() && field != &JsonValue::Bool(false)
                && !field.as_array().is_some_and(Vec::is_empty)
        });
        value
    }).collect::<Vec<_>>();
    let groups = command
        .get_groups()
        .map(|group| {
            json!({
                "id":group.get_id().as_str(), "required":group.is_required_set(),
                "args":group.get_args().map(|v| v.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    rows.push(json!({"path":path,"hidden":command.is_hide_set(),"aliases":command.get_all_aliases().collect::<Vec<_>>(),"args":args,"groups":groups}));
    for child in command.get_subcommands() {
        inventory(child, &path, rows);
    }
}

#[test]
fn language_cli_command_and_option_inventory_matches_frozen_source() {
    let mut command = Cli::command();
    command.build();
    let mut rows = Vec::new();
    inventory(&command, &[], &mut rows);
    let actual = json!({
        "schema":"ait.language-cli-inventory/v1", "source_path":"rust/crates/ait-cli/src/main_app/cli_args.rs",
        "source_sha256":format!("{:x}", Sha256::digest(include_bytes!("cli_args.rs"))), "commands":rows,
    });
    let expected = JsonCodec::parse_slice_with_error_prefix(
        include_bytes!("../../tests/fixtures/language_cli_inventory.json"),
        "frozen CLI inventory",
    )
    .unwrap();
    assert_eq!(
        actual["source_sha256"], expected["source_sha256"],
        "Review native grammar changes and update the language frontend contract together."
    );
    let actual = actual["commands"].as_array().unwrap();
    let expected = expected["commands"].as_array().unwrap();
    assert_eq!(
        actual.len(),
        expected.len(),
        "Declared command count changed."
    );
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            actual == expected,
            "Command/option metadata changed at {}",
            actual["path"]
        );
    }
}
