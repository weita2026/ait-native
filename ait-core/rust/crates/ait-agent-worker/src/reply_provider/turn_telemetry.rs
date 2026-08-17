use std::collections::{BTreeMap, BTreeSet};

use ait_agent_core::AGENT_GATEWAY_TURN_TELEMETRY_CONTRACT;
use ait_core::json_support::{json, JsonValue};

const MAX_COMMAND_ID_BYTES: usize = 512;
const MAX_COMMAND_TEXT_BYTES: usize = 64 * 1024;
const MAX_SHELL_RECURSION: usize = 4;

const AIT_TOP_LEVEL_COMMANDS: &[&str] = &[
    "auth",
    "attest",
    "change",
    "config",
    "doctor",
    "gc",
    "init",
    "land",
    "line",
    "patchset",
    "policy",
    "pull",
    "push",
    "queue",
    "remote",
    "repo",
    "review",
    "stash",
    "snapshot",
    "status",
    "task",
    "workflow",
    "workspace",
    "worktree",
];

const AIT_COMMAND_GROUPS_WITH_SUBCOMMAND: &[&str] = &[
    "auth",
    "attest",
    "change",
    "config",
    "doctor",
    "gc",
    "land",
    "line",
    "patchset",
    "policy",
    "queue",
    "remote",
    "repo",
    "review",
    "stash",
    "snapshot",
    "task",
    "workflow",
    "workspace",
    "worktree",
];

const AIT_SCRIPT_GROUPS_WITH_SUBCOMMAND: &[&str] = &[
    "agent",
    "community",
    "community-web",
    "core",
    "server",
    "site",
    "telegram",
    "web",
];

#[derive(Clone, Debug, Default)]
struct CommandObservation {
    command: Option<String>,
    started: bool,
    completed: bool,
    exit_code: Option<i64>,
    status: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct TurnTelemetryCollector {
    commands: BTreeMap<String, CommandObservation>,
    dropped_command_event_count: u64,
}

impl TurnTelemetryCollector {
    pub(super) fn observe(&mut self, event: &JsonValue) {
        let Some(phase) = event.get("type").and_then(JsonValue::as_str) else {
            return;
        };
        if !matches!(phase, "item.started" | "item.completed") {
            return;
        }
        let Some(item) = event.get("item").and_then(JsonValue::as_object) else {
            return;
        };
        if !matches!(
            item.get("type").and_then(JsonValue::as_str),
            Some("command_execution" | "commandExecution")
        ) {
            return;
        }
        let Some(item_id) = bounded_text(item.get("id"), MAX_COMMAND_ID_BYTES) else {
            self.dropped_command_event_count += 1;
            return;
        };
        let observation = self.commands.entry(item_id).or_default();
        observation.started |= phase == "item.started";
        observation.completed |= phase == "item.completed";
        if let Some(command) = bounded_text(item.get("command"), MAX_COMMAND_TEXT_BYTES) {
            observation.command = Some(command);
        }
        if let Some(exit_code) = item
            .get("exit_code")
            .or_else(|| item.get("exitCode"))
            .and_then(JsonValue::as_i64)
        {
            observation.exit_code = Some(exit_code);
        }
        if let Some(status) = bounded_text(item.get("status"), 64) {
            observation.status = Some(status.to_ascii_lowercase());
        }
    }

    pub(super) fn into_json(self) -> JsonValue {
        let command_count = self.commands.len() as u64;
        let attempted_command_count = self
            .commands
            .values()
            .filter(|command| command.started)
            .count() as u64;
        let completed_command_count = self
            .commands
            .values()
            .filter(|command| command.completed)
            .count() as u64;
        let successful_command_count = self
            .commands
            .values()
            .filter(|command| command_outcome(command) == CommandOutcome::Successful)
            .count() as u64;
        let failed_command_count = self
            .commands
            .values()
            .filter(|command| command_outcome(command) == CommandOutcome::Failed)
            .count() as u64;
        let incomplete_command_count = command_count.saturating_sub(completed_command_count);
        let unknown_outcome_command_count = completed_command_count
            .saturating_sub(successful_command_count)
            .saturating_sub(failed_command_count);

        let distinct_command_count = self
            .commands
            .values()
            .filter_map(|command| command.command.as_deref())
            .collect::<BTreeSet<_>>()
            .len() as u64;

        let mut ait_command_count = 0_u64;
        let mut ait_attempted_command_count = 0_u64;
        let mut ait_completed_command_count = 0_u64;
        let mut ait_successful_command_count = 0_u64;
        let mut ait_failed_command_count = 0_u64;
        let mut ait_command_paths = BTreeMap::<String, u64>::new();
        for command in self.commands.values() {
            let Some(raw_command) = command.command.as_deref() else {
                continue;
            };
            let paths = extract_ait_command_paths(raw_command);
            let invocation_count = paths.len() as u64;
            ait_command_count += invocation_count;
            if command.started {
                ait_attempted_command_count += invocation_count;
            }
            if command.completed {
                ait_completed_command_count += invocation_count;
            }
            match command_outcome(command) {
                CommandOutcome::Successful => ait_successful_command_count += invocation_count,
                CommandOutcome::Failed => ait_failed_command_count += invocation_count,
                CommandOutcome::Unknown => {}
            }
            for path in paths {
                *ait_command_paths.entry(path).or_default() += 1;
            }
        }
        let distinct_ait_command_count = ait_command_paths.len() as u64;
        let ait_commands = ait_command_paths
            .into_iter()
            .map(|(command_path, count)| {
                json!({
                    "command_path": command_path,
                    "count": count,
                })
            })
            .collect::<Vec<_>>();

        json!({
            "contract": AGENT_GATEWAY_TURN_TELEMETRY_CONTRACT,
            "command_count": command_count,
            "distinct_command_count": distinct_command_count,
            "attempted_command_count": attempted_command_count,
            "completed_command_count": completed_command_count,
            "successful_command_count": successful_command_count,
            "failed_command_count": failed_command_count,
            "incomplete_command_count": incomplete_command_count,
            "unknown_outcome_command_count": unknown_outcome_command_count,
            "ait_command_count": ait_command_count,
            "distinct_ait_command_count": distinct_ait_command_count,
            "ait_attempted_command_count": ait_attempted_command_count,
            "ait_completed_command_count": ait_completed_command_count,
            "ait_successful_command_count": ait_successful_command_count,
            "ait_failed_command_count": ait_failed_command_count,
            "ait_commands": ait_commands,
            "dropped_command_event_count": self.dropped_command_event_count,
            "partial": incomplete_command_count > 0,
        })
    }
}

pub(super) fn telemetry_from_jsonl(output: &str) -> Option<JsonValue> {
    let mut collector = TurnTelemetryCollector::default();
    let mut saw_event = false;
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Ok(event) = line.parse::<JsonValue>() else {
            return None;
        };
        saw_event = true;
        collector.observe(&event);
    }
    saw_event.then(|| collector.into_json())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandOutcome {
    Successful,
    Failed,
    Unknown,
}

fn command_outcome(command: &CommandObservation) -> CommandOutcome {
    if !command.completed {
        return CommandOutcome::Unknown;
    }
    match command.exit_code {
        Some(0) => CommandOutcome::Successful,
        Some(_) => CommandOutcome::Failed,
        None if command.status.as_deref().is_some_and(|status| {
            matches!(status, "failed" | "error" | "cancelled" | "canceled")
        }) =>
        {
            CommandOutcome::Failed
        }
        None => CommandOutcome::Unknown,
    }
}

fn bounded_text(value: Option<&JsonValue>, limit: usize) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= limit)
        .filter(|value| !value.chars().any(|character| character == '\0'))
        .map(str::to_string)
}

fn extract_ait_command_paths(command: &str) -> Vec<String> {
    extract_ait_command_paths_at_depth(command, 0)
}

fn extract_ait_command_paths_at_depth(command: &str, depth: usize) -> Vec<String> {
    if command.is_empty() || command.len() > MAX_COMMAND_TEXT_BYTES || depth > MAX_SHELL_RECURSION {
        return Vec::new();
    }
    let mut paths = Vec::new();
    for segment in shell_segments(command) {
        if let Some(script) = wrapped_shell_script(&segment) {
            paths.extend(extract_ait_command_paths_at_depth(&script, depth + 1));
            continue;
        }
        if let Some(arguments) = ait_arguments(&segment) {
            paths.push(normalized_ait_command_path(arguments));
        }
    }
    paths
}

fn wrapped_shell_script(tokens: &[String]) -> Option<String> {
    let executable = executable_name(tokens.first()?);
    let shell = matches!(
        executable.as_str(),
        "bash" | "dash" | "fish" | "ksh" | "sh" | "zsh" | "cmd" | "powershell" | "pwsh"
    );
    if !shell {
        return None;
    }
    let position = tokens.iter().position(|token| {
        matches!(
            token.to_ascii_lowercase().as_str(),
            "-c" | "-lc" | "-cl" | "/c" | "-command"
        )
    })?;
    let script = tokens.get(position + 1..)?.join(" ");
    (!script.trim().is_empty()).then_some(script)
}

fn ait_arguments(tokens: &[String]) -> Option<&[String]> {
    for (index, token) in tokens.iter().enumerate() {
        if is_ait_executable(token) && acceptable_wrapper_prefix(&tokens[..index]) {
            return Some(&tokens[index + 1..]);
        }
        if token == "-m"
            && tokens.get(index + 1).is_some_and(|module| {
                matches!(module.as_str(), "ait" | "ait.cli" | "ait_native.cli")
            })
            && acceptable_wrapper_prefix(&tokens[..index])
        {
            return Some(&tokens[index + 2..]);
        }
    }
    None
}

fn acceptable_wrapper_prefix(tokens: &[String]) -> bool {
    tokens
        .iter()
        .filter(|token| !token.starts_with('-'))
        .all(|token| {
            let executable = executable_name(token);
            matches!(
                executable.as_str(),
                "bash"
                    | "builtin"
                    | "command"
                    | "dash"
                    | "env"
                    | "exec"
                    | "fish"
                    | "gtimeout"
                    | "ksh"
                    | "nice"
                    | "noglob"
                    | "nohup"
                    | "pipenv"
                    | "poetry"
                    | "python"
                    | "python3"
                    | "run"
                    | "sh"
                    | "stdbuf"
                    | "sudo"
                    | "time"
                    | "timeout"
                    | "uv"
                    | "uvx"
                    | "zsh"
                    | "!"
                    | "do"
                    | "elif"
                    | "else"
                    | "then"
            ) || is_environment_assignment(token)
                || is_duration_or_number(token)
        })
}

fn is_ait_executable(token: &str) -> bool {
    matches!(
        executable_name(token).as_str(),
        "ait" | "ait-cli" | "ait.sh"
    )
}

fn executable_name(token: &str) -> String {
    token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches(".exe")
        .to_ascii_lowercase()
}

fn normalized_ait_command_path(arguments: &[String]) -> String {
    let top_level = arguments
        .iter()
        .enumerate()
        .find_map(|(index, token)| {
            let normalized = normalized_atom(token)?;
            (AIT_TOP_LEVEL_COMMANDS.contains(&normalized.as_str())
                || AIT_SCRIPT_GROUPS_WITH_SUBCOMMAND.contains(&normalized.as_str()))
            .then_some((index, normalized))
        })
        .or_else(|| {
            arguments
                .iter()
                .enumerate()
                .find_map(|(index, token)| normalized_atom(token).map(|value| (index, value)))
        });
    let Some((top_level_index, top_level)) = top_level else {
        return "other".to_string();
    };
    let grouped = AIT_COMMAND_GROUPS_WITH_SUBCOMMAND.contains(&top_level.as_str())
        || AIT_SCRIPT_GROUPS_WITH_SUBCOMMAND.contains(&top_level.as_str());
    if !grouped {
        return top_level;
    }
    let subcommand = arguments[top_level_index + 1..]
        .iter()
        .find_map(|token| normalized_atom(token));
    match subcommand {
        Some(subcommand) => format!("{top_level} {subcommand}"),
        None => top_level,
    }
}

fn normalized_atom(token: &str) -> Option<String> {
    let token = token.trim().trim_end_matches(';');
    if token.is_empty()
        || token.starts_with('-')
        || token.len() > 64
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    Some(token.to_ascii_lowercase())
}

fn is_environment_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_duration_or_number(token: &str) -> bool {
    let token = token.trim_start_matches(['+', '-']);
    (!token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit()))
        || token
            .strip_suffix(['s', 'm', 'h', 'd'])
            .is_some_and(|value| {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn shell_segments(command: &str) -> Vec<Vec<String>> {
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut segments = Vec::<Vec<String>>::new();
    let mut segment = Vec::<String>::new();
    let mut token = String::new();
    let mut quote = Quote::None;
    let finish_token = |segment: &mut Vec<String>, token: &mut String| {
        if !token.is_empty() {
            segment.push(std::mem::take(token));
        }
    };
    let finish_segment =
        |segments: &mut Vec<Vec<String>>, segment: &mut Vec<String>, token: &mut String| {
            finish_token(segment, token);
            if !segment.is_empty() {
                segments.push(std::mem::take(segment));
            }
        };

    let mut characters = command.chars().peekable();
    while let Some(character) = characters.next() {
        match quote {
            Quote::Single => {
                if character == '\'' {
                    quote = Quote::None;
                } else {
                    token.push(character);
                }
            }
            Quote::Double => match character {
                '"' => quote = Quote::None,
                '\\' => match characters.peek().copied() {
                    Some('"' | '\\' | '$' | '`') => {
                        token.push(characters.next().expect("peeked escaped character"));
                    }
                    _ => token.push('\\'),
                },
                _ => token.push(character),
            },
            Quote::None => match character {
                '\'' => quote = Quote::Single,
                '"' => quote = Quote::Double,
                '\\' => match characters.peek().copied() {
                    Some(next)
                        if next.is_whitespace()
                            || matches!(next, '\'' | '"' | '\\' | ';' | '|' | '&') =>
                    {
                        token.push(characters.next().expect("peeked escaped character"));
                    }
                    _ => token.push('\\'),
                },
                ' ' | '\t' | '\r' => finish_token(&mut segment, &mut token),
                '\n' | ';' | '|' | '&' => finish_segment(&mut segments, &mut segment, &mut token),
                _ => token.push(character),
            },
        }
    }
    finish_segment(&mut segments, &mut segment, &mut token);
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lifecycle_is_deduplicated_and_compound_ait_paths_are_counted() {
        let mut collector = TurnTelemetryCollector::default();
        for event in [
            json!({
                "type": "item.started",
                "item": {
                    "id": "command-1",
                    "type": "command_execution",
                    "command": "/bin/zsh -lc 'ait task audit RCT-1 && /opt/bin/ait queue summary'",
                    "status": "in_progress",
                },
            }),
            json!({
                "type": "item.completed",
                "item": {
                    "id": "command-1",
                    "type": "command_execution",
                    "command": "/bin/zsh -lc 'ait task audit RCT-1 && /opt/bin/ait queue summary'",
                    "exit_code": 0,
                    "status": "completed",
                },
            }),
            json!({
                "type": "item.completed",
                "item": {
                    "id": "command-2",
                    "type": "command_execution",
                    "command": "rg -n ait rust",
                    "exit_code": 1,
                    "status": "failed",
                },
            }),
        ] {
            collector.observe(&event);
        }
        let telemetry = collector.into_json();
        assert_eq!(telemetry["command_count"], 2);
        assert_eq!(telemetry["attempted_command_count"], 1);
        assert_eq!(telemetry["completed_command_count"], 2);
        assert_eq!(telemetry["successful_command_count"], 1);
        assert_eq!(telemetry["failed_command_count"], 1);
        assert_eq!(telemetry["ait_command_count"], 2);
        assert_eq!(telemetry["distinct_ait_command_count"], 2);
        assert_eq!(
            telemetry["ait_commands"][0]["command_path"],
            "queue summary"
        );
        assert_eq!(telemetry["ait_commands"][1]["command_path"], "task audit");
    }

    #[test]
    fn telemetry_never_emits_raw_ait_arguments_or_secrets() {
        let mut collector = TurnTelemetryCollector::default();
        collector.observe(&json!({
            "type": "item.completed",
            "item": {
                "id": "command-secret",
                "type": "command_execution",
                "command": "AIT_TOKEN=secret ait config set auth.token super-secret",
                "exit_code": 0,
            },
        }));
        let encoded = collector.into_json().to_string();
        assert!(encoded.contains("config set"));
        assert!(!encoded.contains("super-secret"));
        assert!(!encoded.contains("AIT_TOKEN"));
        assert!(!encoded.contains("auth.token"));
    }

    #[test]
    fn python_module_and_native_helper_wrappers_are_classified() {
        assert_eq!(
            extract_ait_command_paths("python -m ait task show RCT-1"),
            ["task show"]
        );
        assert_eq!(
            extract_ait_command_paths("./ait.sh core build; ./ait.sh server status"),
            ["core build", "server status"]
        );
        assert_eq!(
            extract_ait_command_paths(r#""C:\tools\ait.exe" task audit RCT-1"#),
            ["task audit"]
        );
    }

    #[test]
    fn partial_stream_reports_started_command_without_inventing_an_outcome() {
        let telemetry = telemetry_from_jsonl(
            "{\"type\":\"item.started\",\"item\":{\"id\":\"command-1\",\"type\":\"command_execution\",\"command\":\"ait status\"}}\n",
        )
        .expect("partial telemetry");
        assert_eq!(telemetry["command_count"], 1);
        assert_eq!(telemetry["attempted_command_count"], 1);
        assert_eq!(telemetry["completed_command_count"], 0);
        assert_eq!(telemetry["incomplete_command_count"], 1);
        assert_eq!(telemetry["partial"], true);
    }

    #[test]
    fn invalid_command_ids_are_bounded_and_reported() {
        let mut collector = TurnTelemetryCollector::default();
        collector.observe(&json!({
            "type": "item.started",
            "item": {"type": "command_execution", "command": "ait status"},
        }));
        let telemetry = collector.into_json();
        assert_eq!(telemetry["command_count"], 0);
        assert_eq!(telemetry["dropped_command_event_count"], 1);
    }
}
