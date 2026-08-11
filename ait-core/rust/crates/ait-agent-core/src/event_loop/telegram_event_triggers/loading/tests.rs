use std::fs;

use tempfile::tempdir;

use super::*;

struct FixedPlanner {
    result: Result<JsonValue, String>,
}

impl TelegramEventTriggerPlanner for FixedPlanner {
    fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        self.result.clone()
    }
}

fn write_markdown(root: &Path, name: &str, payload: &str) {
    let directory = root.join(EVENT_TRIGGER_DIRECTORY);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(name),
        format!("# Trigger\n\n```json\n{payload}\n```\n"),
    )
    .unwrap();
}

fn trigger(id: &str, priority: i64, command: &str) -> String {
    format!(
        r#"{{
  "kind": "telegram_operational_trigger",
  "id": "{id}",
  "displayTrigger": "{id}",
  "handlerCommand": ["/bin/echo", "{id}"],
  "match": {{"commands": ["{command}"]}},
  "priority": {priority}
}}"#
    )
}

#[test]
fn absent_directory_returns_native_defaults_without_operational_triggers() {
    let temp = tempdir().unwrap();
    let loader = NativeTelegramEventTriggerRegistryLoader::new();
    let registry = loader.load(temp.path()).unwrap();

    assert!(registry["fresh_topic"].is_object());
    assert!(registry["planning_mode"].is_object());
    assert_eq!(registry["telegram_operational"], json!([]));
    let debug = format!("{loader:?}");
    assert!(!debug.contains(temp.path().to_string_lossy().as_ref()));
}

#[test]
fn loader_normalizes_sorted_markdown_sources_and_configuration_overrides() {
    let temp = tempdir().unwrap();
    write_markdown(
        temp.path(),
        "fresh_topic.md",
        r#"{"clear":{"phrases":["new subject"],"displayTrigger":"new subject"}}"#,
    );
    write_markdown(
        temp.path(),
        "planning_mode.md",
        r#"{"phrases":["make plan"],"displayTrigger":"make plan"}"#,
    );
    write_markdown(temp.path(), "z-last.md", &trigger("last", 1, "last"));
    write_markdown(temp.path(), "a-first.md", &trigger("first", 9, "first"));

    let registry = NativeTelegramEventTriggerRegistryLoader::new()
        .load(temp.path())
        .unwrap();
    assert_eq!(
        registry["fresh_topic"]["clear"]["phrases"],
        json!(["new subject"])
    );
    assert_eq!(registry["planning_mode"]["phrases"], json!(["make plan"]));
    let operational = registry["telegram_operational"].as_array().unwrap();
    assert_eq!(operational.len(), 2);
    assert_eq!(operational[0]["trigger_id"], "first");
    assert_eq!(
        operational[0]["source_path"],
        "docs/event_trigger/a-first.md"
    );
    assert_eq!(operational[1]["trigger_id"], "last");
}

#[test]
fn malformed_non_object_and_unreadable_markdown_sources_are_skipped() {
    let temp = tempdir().unwrap();
    let directory = temp.path().join(EVENT_TRIGGER_DIRECTORY);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("malformed.md"), "```json\n{bad}\n```").unwrap();
    fs::write(directory.join("array.md"), "```json\n[]\n```").unwrap();
    fs::write(directory.join("missing.md"), "no fenced config").unwrap();
    fs::write(directory.join("unreadable.md"), [0xff, 0xfe]).unwrap();
    write_markdown(temp.path(), "valid.md", &trigger("valid", 0, "valid"));

    let registry = NativeTelegramEventTriggerRegistryLoader::new()
        .load(temp.path())
        .unwrap();
    let operational = registry["telegram_operational"].as_array().unwrap();
    assert_eq!(operational.len(), 1);
    assert_eq!(operational[0]["trigger_id"], "valid");
}

#[test]
fn loader_rejects_oversized_files_and_unbounded_file_counts_generically() {
    let temp = tempdir().unwrap();
    let directory = temp.path().join(EVENT_TRIGGER_DIRECTORY);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("oversized-secret.md"),
        vec![b'x'; MAX_TRIGGER_FILE_BYTES as usize + 1],
    )
    .unwrap();
    let failure = NativeTelegramEventTriggerRegistryLoader::new()
        .load(temp.path())
        .unwrap_err();
    assert_eq!(failure, registry_loading_error());
    assert!(!failure.contains("secret"));

    let temp = tempdir().unwrap();
    let directory = temp.path().join(EVENT_TRIGGER_DIRECTORY);
    fs::create_dir_all(&directory).unwrap();
    for index in 0..=MAX_TRIGGER_FILES {
        fs::write(directory.join(format!("{index:03}.md")), "empty").unwrap();
    }
    assert_eq!(
        NativeTelegramEventTriggerRegistryLoader::new()
            .load(temp.path())
            .unwrap_err(),
        registry_loading_error()
    );
}

#[test]
fn planner_errors_and_corrupt_normalized_contracts_fail_closed() {
    let temp = tempdir().unwrap();
    let loader = NativeTelegramEventTriggerRegistryLoader::with_planner(FixedPlanner {
        result: Err("private-planner-secret".to_string()),
    });
    let failure = loader.load(temp.path()).unwrap_err();
    assert_eq!(failure, registry_loading_error());
    assert!(!failure.contains("secret"));

    let mut planned = DefaultTelegramEventTriggerPlanner
        .plan_json(&json!({"stage": "normalize_registry"}))
        .unwrap();
    planned["python_event_trigger_allowed"] = json!(true);
    let loader = NativeTelegramEventTriggerRegistryLoader::with_planner(FixedPlanner {
        result: Ok(planned),
    });
    assert_eq!(
        loader.load(temp.path()).unwrap_err(),
        registry_loading_error()
    );
}

#[test]
fn forged_normalized_trigger_and_invalid_repo_root_are_rejected() {
    let temp = tempdir().unwrap();
    let mut planned = DefaultTelegramEventTriggerPlanner
        .plan_json(&json!({
            "stage": "normalize_registry",
            "operational_triggers": [{
                "source_path": "docs/event_trigger/valid.md",
                "payload": JsonCodec::parse_value(&trigger("valid", 0, "valid"), "fixture").unwrap(),
            }],
        }))
        .unwrap();
    planned["registry"]["telegram_operational"][0]["source_path"] =
        json!("/private/forged-secret.md");
    let loader = NativeTelegramEventTriggerRegistryLoader::with_planner(FixedPlanner {
        result: Ok(planned),
    });
    let failure = loader.load(temp.path()).unwrap_err();
    assert_eq!(failure, registry_loading_error());
    assert!(!failure.contains("forged-secret"));

    assert_eq!(
        NativeTelegramEventTriggerRegistryLoader::new()
            .load(Path::new("bad\nsecret"))
            .unwrap_err(),
        registry_loading_error()
    );
}
