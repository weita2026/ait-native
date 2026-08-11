use super::{
    agent_transport_config_clean_optional_text, agent_transport_config_normalize_base_url,
    agent_transport_config_parse_int, agent_transport_config_parse_timeout_seconds,
    agent_transport_config_split_message_chunks, AgentTransportConfigIntMode,
};

#[test]
fn cleans_optional_text_like_python_helpers() {
    assert_eq!(agent_transport_config_clean_optional_text(None), None);
    assert_eq!(
        agent_transport_config_clean_optional_text(Some("   ")),
        None
    );
    assert_eq!(
        agent_transport_config_clean_optional_text(Some("  abc  ")),
        Some("abc".to_string())
    );
}

#[test]
fn normalizes_base_urls_and_fallbacks() {
    assert_eq!(
        agent_transport_config_normalize_base_url(Some(" https://example.test/// "), "x"),
        "https://example.test"
    );
    assert_eq!(
        agent_transport_config_normalize_base_url(Some("  "), "https://fallback.test/"),
        "https://fallback.test"
    );
}

#[test]
fn parses_timeout_seconds_with_disable_tokens_and_minimums() {
    assert_eq!(
        agent_transport_config_parse_timeout_seconds(None, Some(5.0), 1.0),
        Some(5.0)
    );
    assert_eq!(
        agent_transport_config_parse_timeout_seconds(Some("bad"), Some(5.0), 1.0),
        Some(5.0)
    );
    assert_eq!(
        agent_transport_config_parse_timeout_seconds(Some("-1"), None, 1.0),
        None
    );
    assert_eq!(
        agent_transport_config_parse_timeout_seconds(Some("none"), Some(5.0), 1.0),
        None
    );
    assert_eq!(
        agent_transport_config_parse_timeout_seconds(Some("2"), Some(5.0), 5.0),
        Some(5.0)
    );
    assert_eq!(
        agent_transport_config_parse_timeout_seconds(Some("7"), Some(5.0), 5.0),
        Some(7.0)
    );
}

#[test]
fn parses_integer_modes_for_existing_transport_semantics() {
    assert_eq!(
        agent_transport_config_parse_int(
            Some("0"),
            100,
            1,
            AgentTransportConfigIntMode::PositiveOrMinimum
        ),
        100
    );
    assert_eq!(
        agent_transport_config_parse_int(
            Some("2"),
            100,
            5,
            AgentTransportConfigIntMode::PositiveOrMinimum
        ),
        5
    );
    assert_eq!(
        agent_transport_config_parse_int(Some("0"), 100, 0, AgentTransportConfigIntMode::Minimum),
        0
    );
    assert_eq!(
        agent_transport_config_parse_int(Some("-1"), 100, 0, AgentTransportConfigIntMode::Minimum),
        100
    );
}

#[test]
fn plans_message_chunks_like_python_transport_helpers() {
    assert_eq!(
        agent_transport_config_split_message_chunks("", 10),
        vec!["(empty)".to_string()]
    );
    assert_eq!(
        agent_transport_config_split_message_chunks("alpha beta gamma", 10),
        vec!["alpha".to_string(), "beta gamma".to_string()]
    );
    assert_eq!(
        agent_transport_config_split_message_chunks("alpha\nbeta gamma", 10),
        vec!["alpha".to_string(), "beta gamma".to_string()]
    );
    assert_eq!(
        agent_transport_config_split_message_chunks("abcdef", 3),
        vec!["abc".to_string(), "def".to_string()]
    );
    assert_eq!(
        agent_transport_config_split_message_chunks("你好世界再見", 3),
        vec!["你好世".to_string(), "界再見".to_string()]
    );
}
