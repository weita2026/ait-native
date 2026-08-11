use super::agent_telegram_message_formatting_plan_json;
use ait_core::json_support::json;

#[test]
fn formats_markdownish_message_chunks() {
    let planned = agent_telegram_message_formatting_plan_json(&json!({
        "kind": "message_chunks",
        "reply_markdown_enabled": true,
        "text": "# Title\n- **bold** and `code`\n> quoted",
    }))
    .unwrap();
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_telegram_message_formatting"
    );
    assert_eq!(planned["python_message_formatting_allowed"], false);
    let chunks = planned["chunks"].as_array().unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0]["parse_mode"], "HTML");
    assert!(chunks[0]["text"].as_str().unwrap().contains("<b>Title</b>"));
    assert!(chunks[0]["text"]
        .as_str()
        .unwrap()
        .contains("• <b>bold</b> and <code>code</code>"));
    assert!(chunks[0]["text"].as_str().unwrap().contains("❝ quoted"));
    assert!(chunks[0]["plain_text"]
        .as_str()
        .unwrap()
        .contains("• **bold** and `code`"));
}

#[test]
fn formats_plain_chunks_without_html_parse_mode() {
    let planned = agent_telegram_message_formatting_plan_json(&json!({
        "kind": "message_chunks",
        "reply_markdown_enabled": false,
        "limit": 8,
        "text": "alpha beta gamma",
    }))
    .unwrap();
    let chunks = planned["chunks"].as_array().unwrap();
    assert_eq!(chunks[0]["text"], "alpha");
    assert_eq!(chunks[0]["plain_text"], "alpha");
    assert!(chunks[0]["parse_mode"].is_null());
}

#[test]
fn detects_telegram_markdown_parse_errors() {
    let planned = agent_telegram_message_formatting_plan_json(&json!({
        "kind": "markdown_parse_error",
        "error": "Bad Request: can't parse entities",
    }))
    .unwrap();
    assert_eq!(planned["parse_error"], true);
    let other = agent_telegram_message_formatting_plan_json(&json!({
        "kind": "markdown_parse_error",
        "error": "connection reset",
    }))
    .unwrap();
    assert_eq!(other["parse_error"], false);
}
