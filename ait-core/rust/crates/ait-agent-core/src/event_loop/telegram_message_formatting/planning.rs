use crate::transport::config::agent_transport_config_split_message_chunks;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_telegram_message_formatting";
const MESSAGE_FORMAT_CONTRACT: &str = "ait_agent_core.event_loop.TelegramMessageFormatting.v1";
const MAX_TELEGRAM_MESSAGE: usize = 3800;
const TELEGRAM_HTML_PARSE_MODE: &str = "HTML";
const TELEGRAM_PARSE_ERROR_MARKERS: &[&str] = &[
    "can't parse entities",
    "unsupported start tag",
    "unsupported end tag",
    "entity end tag",
];

pub trait TelegramMessageFormattingPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramMessageFormattingPlanner;

impl TelegramMessageFormattingPlanner for DefaultTelegramMessageFormattingPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_telegram_message_formatting_json(request)
    }
}

pub fn agent_telegram_message_formatting_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_telegram_message_formatting_planner(&DefaultTelegramMessageFormattingPlanner, request)
}

pub fn plan_with_telegram_message_formatting_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramMessageFormattingPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_telegram_message_formatting_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let kind = optional_text(object.get("kind"))
        .or_else(|| optional_text(object.get("stage")))
        .unwrap_or_else(|| "message_chunks".to_string());
    match kind.as_str() {
        "message_chunks" => {
            let text = text_field(object.get("text"));
            let limit = positive_usize(object.get("limit"), MAX_TELEGRAM_MESSAGE);
            let chunks = if bool_field(object, "reply_markdown_enabled") {
                render_markdownish_message_chunks(&text, limit)
            } else {
                render_plain_message_chunks(&text, limit)
            };
            Ok(base_result(
                &kind,
                json!({
                    "chunks": chunks.into_iter().map(message_chunk_json).collect::<Vec<_>>(),
                }),
            ))
        }
        "markdown_parse_error" => {
            let error_text = optional_text(object.get("error"))
                .unwrap_or_else(|| text_field(object.get("text")));
            Ok(base_result(
                &kind,
                json!({
                    "parse_error": markdownish_parse_error(&error_text),
                }),
            ))
        }
        other => Err(format!(
            "unsupported Telegram message formatting plan kind `{other}`"
        )),
    }
}

fn base_result(kind: &str, mut fields: JsonValue) -> JsonValue {
    let mut base = json!({
        "migration_stage": MIGRATION_STAGE,
        "message_format_contract": MESSAGE_FORMAT_CONTRACT,
        "kind": kind,
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_message_formatting_allowed": false,
    });
    if let (Some(base), Some(fields)) = (base.as_object_mut(), fields.as_object_mut()) {
        for (key, value) in std::mem::take(fields) {
            base.insert(key, value);
        }
    }
    base
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MessageChunk {
    text: String,
    plain_text: String,
    parse_mode: Option<String>,
}

fn message_chunk_json(chunk: MessageChunk) -> JsonValue {
    json!({
        "text": chunk.text,
        "plain_text": chunk.plain_text,
        "parse_mode": chunk.parse_mode,
    })
}

fn render_plain_message_chunks(text: &str, limit: usize) -> Vec<MessageChunk> {
    agent_transport_config_split_message_chunks(text, limit)
        .into_iter()
        .map(|chunk| MessageChunk {
            text: chunk.clone(),
            plain_text: chunk,
            parse_mode: None,
        })
        .collect()
}

fn markdownish_parse_error(error_text: &str) -> bool {
    let lowered = error_text.to_lowercase();
    TELEGRAM_PARSE_ERROR_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn render_markdownish_message_chunks(text: &str, limit: usize) -> Vec<MessageChunk> {
    let content = text.trim().to_string();
    if content.is_empty() {
        return vec![markdown_message_chunk("(empty)", "(empty)")];
    }

    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut paragraph_lines: Vec<String> = Vec::new();
    let mut code_lines: Vec<String> = Vec::new();
    let mut code_fence = String::new();
    let mut in_code_block = false;
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");

    for raw_line in normalized.split('\n') {
        if in_code_block {
            if raw_line.trim().starts_with("```") {
                blocks.extend(render_markdownish_code_blocks(
                    &code_lines.join("\n"),
                    limit,
                ));
                code_lines.clear();
                code_fence.clear();
                in_code_block = false;
            } else {
                code_lines.push(raw_line.to_string());
            }
            continue;
        }
        if raw_line.trim().starts_with("```") {
            if !paragraph_lines.is_empty() {
                blocks.extend(render_markdownish_paragraph_blocks(
                    &paragraph_lines.join("\n"),
                    limit,
                ));
                paragraph_lines.clear();
            }
            code_fence = raw_line.to_string();
            in_code_block = true;
            continue;
        }
        if raw_line.trim().is_empty() {
            if !paragraph_lines.is_empty() {
                blocks.extend(render_markdownish_paragraph_blocks(
                    &paragraph_lines.join("\n"),
                    limit,
                ));
                paragraph_lines.clear();
            }
            continue;
        }
        paragraph_lines.push(raw_line.to_string());
    }

    if in_code_block {
        paragraph_lines.clear();
        paragraph_lines.push(code_fence);
        paragraph_lines.extend(code_lines);
    }
    if !paragraph_lines.is_empty() {
        blocks.extend(render_markdownish_paragraph_blocks(
            &paragraph_lines.join("\n"),
            limit,
        ));
    }

    let mut message_chunks = Vec::new();
    let mut current_plain_blocks: Vec<String> = Vec::new();
    let mut current_rendered_blocks: Vec<String> = Vec::new();

    for (plain_block, rendered_block) in blocks {
        let candidate_rendered = if current_rendered_blocks.is_empty() {
            rendered_block.clone()
        } else {
            join_with_extra(&current_rendered_blocks, &rendered_block, "\n\n")
        };
        if !current_rendered_blocks.is_empty() && char_len(&candidate_rendered) > limit {
            message_chunks.push(markdown_message_chunk(
                &current_rendered_blocks.join("\n\n"),
                &current_plain_blocks.join("\n\n"),
            ));
            current_plain_blocks = vec![plain_block];
            current_rendered_blocks = vec![rendered_block];
            continue;
        }
        current_plain_blocks.push(plain_block);
        current_rendered_blocks.push(rendered_block);
    }

    if !current_rendered_blocks.is_empty() {
        message_chunks.push(markdown_message_chunk(
            &current_rendered_blocks.join("\n\n"),
            &current_plain_blocks.join("\n\n"),
        ));
    }
    if message_chunks.is_empty() {
        vec![markdown_message_chunk("(empty)", "(empty)")]
    } else {
        message_chunks
    }
}

fn markdown_message_chunk(text: &str, plain_text: &str) -> MessageChunk {
    MessageChunk {
        text: text.to_string(),
        plain_text: plain_text.to_string(),
        parse_mode: Some(TELEGRAM_HTML_PARSE_MODE.to_string()),
    }
}

fn render_markdownish_paragraph_blocks(paragraph: &str, limit: usize) -> Vec<(String, String)> {
    let mut rendered_lines: Vec<(String, String)> = Vec::new();
    for (index, raw_line) in paragraph.split('\n').enumerate() {
        rendered_lines.extend(split_markdownish_line_fragments(
            raw_line,
            limit,
            index == 0 && paragraph.split('\n').count() > 1,
        ));
    }

    let mut blocks = Vec::new();
    let mut current_plain: Vec<String> = Vec::new();
    let mut current_rendered: Vec<String> = Vec::new();
    for (plain_line, rendered_line) in rendered_lines {
        let candidate_rendered = if current_rendered.is_empty() {
            rendered_line.clone()
        } else {
            join_with_extra(&current_rendered, &rendered_line, "\n")
        };
        if !current_rendered.is_empty() && char_len(&candidate_rendered) > limit {
            blocks.push((current_plain.join("\n"), current_rendered.join("\n")));
            current_plain = vec![plain_line];
            current_rendered = vec![rendered_line];
            continue;
        }
        current_plain.push(plain_line);
        current_rendered.push(rendered_line);
    }
    if !current_rendered.is_empty() {
        blocks.push((current_plain.join("\n"), current_rendered.join("\n")));
    }
    if blocks.is_empty() {
        vec![(
            paragraph.to_string(),
            render_inline_markdownish_html(paragraph),
        )]
    } else {
        blocks
    }
}

fn render_markdownish_code_blocks(code_text: &str, limit: usize) -> Vec<(String, String)> {
    let content = code_text.trim_end_matches('\n').to_string();
    if content.is_empty() {
        return vec![(
            "```\n```".to_string(),
            "<pre><code></code></pre>".to_string(),
        )];
    }
    let mut blocks = Vec::new();
    let mut current_lines: Vec<String> = Vec::new();

    for line in content.split('\n') {
        if char_len(&rendered_code_html(line)) > limit {
            if !current_lines.is_empty() {
                append_code_block(&mut blocks, &current_lines.join("\n"));
                current_lines.clear();
            }
            let fragment_limit = limit
                .saturating_sub(char_len(&rendered_code_html("")))
                .saturating_sub(8)
                .max(24);
            for fragment in split_text_fragments(line, fragment_limit) {
                append_code_block(&mut blocks, &fragment);
            }
            continue;
        }

        let candidate_lines = join_with_extra(&current_lines, line, "\n");
        if !current_lines.is_empty() && char_len(&rendered_code_html(&candidate_lines)) > limit {
            append_code_block(&mut blocks, &current_lines.join("\n"));
            current_lines = vec![line.to_string()];
            continue;
        }
        current_lines.push(line.to_string());
    }
    if !current_lines.is_empty() {
        append_code_block(&mut blocks, &current_lines.join("\n"));
    }
    blocks
}

fn append_code_block(blocks: &mut Vec<(String, String)>, value: &str) {
    blocks.push((format!("```\n{value}\n```"), rendered_code_html(value)));
}

fn rendered_code_html(value: &str) -> String {
    format!("<pre><code>{}</code></pre>", html_escape(value))
}

fn split_markdownish_line_fragments(
    raw_line: &str,
    limit: usize,
    emphasize_title: bool,
) -> Vec<(String, String)> {
    let stripped = raw_line.trim();
    if stripped.is_empty() {
        return vec![(String::new(), String::new())];
    }

    let mut kind = "plain";
    let mut prefix = String::new();
    let mut content = stripped.to_string();

    if let Some(value) = parse_bullet(stripped) {
        kind = "bullet";
        content = value;
        prefix = "• ".to_string();
    } else if let Some((number, value)) = parse_ordered(stripped) {
        kind = "ordered";
        content = value;
        prefix = format!("{number}. ");
    } else if let Some(value) = parse_heading(stripped) {
        kind = "heading";
        content = value;
    } else if let Some(value) = parse_quote(stripped) {
        kind = "quote";
        content = value;
        prefix = "❝ ".to_string();
    } else if let Some(value) = unwrap_quoted_line(stripped) {
        kind = "quoted";
        content = value;
    } else if emphasize_title {
        kind = "title";
    }

    let content_chunks = if content.is_empty() {
        vec![String::new()]
    } else {
        split_text_fragments(&content, limit.saturating_sub(32).max(24))
    };
    let mut fragments = Vec::new();
    for (index, chunk) in content_chunks.iter().enumerate() {
        let rendered_content = render_inline_markdownish_html(chunk);
        let (plain_line, rendered_line) = match kind {
            "heading" | "title" => (chunk.to_string(), format!("<b>{rendered_content}</b>")),
            "bullet" => {
                let continuation_prefix = if index == 0 { prefix.as_str() } else { "  " };
                (
                    format!("{continuation_prefix}{chunk}")
                        .trim_end()
                        .to_string(),
                    format!("{continuation_prefix}{rendered_content}")
                        .trim_end()
                        .to_string(),
                )
            }
            "ordered" => {
                let continuation_prefix = if index == 0 { prefix.as_str() } else { "   " };
                (
                    format!("{continuation_prefix}{chunk}")
                        .trim_end()
                        .to_string(),
                    format!("{continuation_prefix}{rendered_content}")
                        .trim_end()
                        .to_string(),
                )
            }
            "quote" => {
                let continuation_prefix = if index == 0 { prefix.as_str() } else { "  " };
                (
                    format!("{continuation_prefix}{chunk}")
                        .trim_end()
                        .to_string(),
                    format!("{continuation_prefix}{rendered_content}")
                        .trim_end()
                        .to_string(),
                )
            }
            "quoted" => (
                format!("❝ {chunk} ❞").trim().to_string(),
                format!("❝ {rendered_content} ❞").trim().to_string(),
            ),
            _ => (chunk.to_string(), rendered_content),
        };
        fragments.push((plain_line, rendered_line));
    }
    if fragments.is_empty() {
        vec![(String::new(), String::new())]
    } else {
        fragments
    }
}

fn render_inline_markdownish_html(text: &str) -> String {
    let raw = text;
    let mut pieces: Vec<String> = Vec::new();
    let mut buffer = String::new();
    let mut index = 0;

    while index < raw.len() {
        if raw[index..].starts_with('`') {
            if let Some(end) = find_marker_end(raw, index, "`") {
                let inner = &raw[index + 1..end];
                if !inner.contains('\n') && !inner.is_empty() {
                    flush_html_buffer(&mut pieces, &mut buffer);
                    pieces.push(format!("<code>{}</code>", html_escape(inner)));
                    index = end + 1;
                    continue;
                }
            }
        }

        let mut matched = false;
        for (marker, tag) in [("**", "b"), ("~~", "s"), ("*", "i")] {
            if !raw[index..].starts_with(marker) {
                continue;
            }
            if let Some(end) = find_marker_end(raw, index, marker) {
                let inner = &raw[index + marker.len()..end];
                if !inner.trim().is_empty() {
                    flush_html_buffer(&mut pieces, &mut buffer);
                    pieces.push(format!(
                        "<{tag}>{}</{tag}>",
                        render_inline_markdownish_html(inner)
                    ));
                    index = end + marker.len();
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            continue;
        }

        let Some(ch) = raw[index..].chars().next() else {
            break;
        };
        buffer.push(ch);
        index += ch.len_utf8();
    }
    flush_html_buffer(&mut pieces, &mut buffer);
    pieces.join("")
}

fn find_marker_end(raw: &str, index: usize, marker: &str) -> Option<usize> {
    raw[index + marker.len()..]
        .find(marker)
        .map(|relative| index + marker.len() + relative)
}

fn flush_html_buffer(pieces: &mut Vec<String>, buffer: &mut String) {
    if !buffer.is_empty() {
        pieces.push(html_escape(buffer));
        buffer.clear();
    }
}

fn html_escape(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn split_text_fragments(text: &str, limit: usize) -> Vec<String> {
    let mut remaining = text.trim().to_string();
    if remaining.is_empty() {
        return vec![String::new()];
    }
    let effective_limit = limit.max(1);
    let threshold = effective_limit / 2;
    let mut chunks = Vec::new();
    while char_len(&remaining) > effective_limit {
        let split_at = rfind_char_position_before_limit(&remaining, '\n', effective_limit)
            .filter(|index| *index >= threshold)
            .or_else(|| {
                rfind_char_position_before_limit(&remaining, ' ', effective_limit)
                    .filter(|index| *index >= threshold)
            })
            .unwrap_or(effective_limit);
        let chunk: String = remaining.chars().take(split_at).collect();
        let rest: String = remaining.chars().skip(split_at).collect();
        chunks.push(chunk.trim_end().to_string());
        remaining = rest.trim_start().to_string();
    }
    if !remaining.is_empty() {
        chunks.push(remaining);
    }
    chunks
}

fn rfind_char_position_before_limit(text: &str, needle: char, limit: usize) -> Option<usize> {
    text.chars()
        .take(limit)
        .enumerate()
        .filter_map(|(index, value)| (value == needle).then_some(index))
        .last()
}

fn parse_bullet(stripped: &str) -> Option<String> {
    let mut chars = stripped.char_indices();
    let (_, first) = chars.next()?;
    if first != '-' && first != '*' {
        return None;
    }
    let (start, second) = chars.next()?;
    if !second.is_whitespace() {
        return None;
    }
    let content_start = stripped[start..]
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| start + idx)
        .unwrap_or(stripped.len());
    Some(stripped[content_start..].trim().to_string())
}

fn parse_ordered(stripped: &str) -> Option<(String, String)> {
    let mut digit_end = 0;
    for (idx, ch) in stripped.char_indices() {
        if ch.is_ascii_digit() {
            digit_end = idx + ch.len_utf8();
            continue;
        }
        break;
    }
    if digit_end == 0 {
        return None;
    }
    let mut chars = stripped[digit_end..].char_indices();
    let (_, delimiter) = chars.next()?;
    if delimiter != '.' && delimiter != ')' {
        return None;
    }
    let (after_delimiter, whitespace) = chars.next()?;
    if !whitespace.is_whitespace() {
        return None;
    }
    let whitespace_start = digit_end + after_delimiter;
    let content_start = stripped[whitespace_start..]
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| whitespace_start + idx)
        .unwrap_or(stripped.len());
    Some((
        stripped[..digit_end].to_string(),
        stripped[content_start..].trim().to_string(),
    ))
}

fn parse_heading(stripped: &str) -> Option<String> {
    let mut count = 0;
    let mut last_hash_end = 0;
    for (idx, ch) in stripped.char_indices() {
        if ch == '#' && count < 6 {
            count += 1;
            last_hash_end = idx + ch.len_utf8();
            continue;
        }
        break;
    }
    if count == 0 {
        return None;
    }
    let mut chars = stripped[last_hash_end..].char_indices();
    let (_, whitespace) = chars.next()?;
    if !whitespace.is_whitespace() {
        return None;
    }
    let content_start = stripped[last_hash_end..]
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| last_hash_end + idx)
        .unwrap_or(stripped.len());
    Some(stripped[content_start..].trim().to_string())
}

fn parse_quote(stripped: &str) -> Option<String> {
    if !stripped.starts_with('>') {
        return None;
    }
    let rest = &stripped[1..];
    let content = if let Some(first) = rest.chars().next() {
        if first.is_whitespace() {
            &rest[first.len_utf8()..]
        } else {
            rest
        }
    } else {
        rest
    };
    Some(content.trim().to_string())
}

fn unwrap_quoted_line(text: &str) -> Option<String> {
    let stripped = text.trim();
    let mut chars = stripped.chars();
    let first = chars.next()?;
    let last = stripped.chars().last()?;
    if char_len(stripped) < 2 {
        return None;
    }
    let closer = match first {
        '\'' => '\'',
        '"' => '"',
        '‘' => '’',
        '“' => '”',
        '「' => '」',
        '『' => '』',
        _ => return None,
    };
    if last != closer {
        return None;
    }
    let inner: String = stripped
        .chars()
        .skip(1)
        .take(char_len(stripped) - 2)
        .collect();
    let inner = inner.trim().to_string();
    (!inner.is_empty()).then_some(inner)
}

fn bool_field(object: &Map<String, JsonValue>, key: &str) -> bool {
    object
        .get(key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    let text = text_field(value);
    (!text.is_empty()).then_some(text)
}

fn text_field(value: Option<&JsonValue>) -> String {
    match value {
        Some(JsonValue::String(text)) => text.to_string(),
        Some(JsonValue::Number(number)) => number.to_string(),
        Some(JsonValue::Bool(value)) => value.to_string(),
        Some(JsonValue::Array(_)) | Some(JsonValue::Object(_)) => value.unwrap().to_string(),
        Some(JsonValue::Null) | None => String::new(),
    }
}

fn positive_usize(value: Option<&JsonValue>, fallback: usize) -> usize {
    let parsed = match value {
        Some(JsonValue::Number(number)) => number.as_u64().map(|value| value as usize),
        Some(JsonValue::String(text)) => text.trim().parse::<usize>().ok(),
        _ => None,
    };
    parsed.filter(|value| *value > 0).unwrap_or(fallback)
}

fn char_len(value: &str) -> usize {
    value.chars().count()
}

fn join_with_extra(items: &[String], extra: &str, separator: &str) -> String {
    if items.is_empty() {
        extra.to_string()
    } else {
        let mut values = items.to_vec();
        values.push(extra.to_string());
        values.join(separator)
    }
}

#[cfg(test)]
mod tests;
