from __future__ import annotations

import html
import re
from dataclasses import dataclass
from typing import Any


MAX_TELEGRAM_MESSAGE = 3800
TELEGRAM_HTML_PARSE_MODE = "HTML"
TELEGRAM_PARSE_ERROR_MARKERS = (
    "can't parse entities",
    "unsupported start tag",
    "unsupported end tag",
    "entity end tag",
)


@dataclass(frozen=True)
class TelegramMessageChunk:
    text: str
    plain_text: str
    parse_mode: str | None = None


def _split_message_chunks(text: str, *, limit: int = MAX_TELEGRAM_MESSAGE) -> list[str]:
    content = str(text or "").strip()
    if not content:
        return ["(empty)"]
    chunks: list[str] = []
    remaining = content
    while len(remaining) > limit:
        split_at = remaining.rfind("\n", 0, limit)
        if split_at < int(limit * 0.5):
            split_at = remaining.rfind(" ", 0, limit)
        if split_at < int(limit * 0.5):
            split_at = limit
        chunks.append(remaining[:split_at].rstrip())
        remaining = remaining[split_at:].lstrip()
    if remaining:
        chunks.append(remaining)
    return chunks


def _markdownish_parse_error(exc: BaseException) -> bool:
    lowered = str(exc or "").lower()
    return any(marker in lowered for marker in TELEGRAM_PARSE_ERROR_MARKERS)


def _split_text_fragments(text: str, *, limit: int) -> list[str]:
    content = str(text or "").strip()
    if not content:
        return [""]
    chunks: list[str] = []
    remaining = content
    while len(remaining) > limit:
        split_at = remaining.rfind("\n", 0, limit)
        if split_at < int(limit * 0.5):
            split_at = remaining.rfind(" ", 0, limit)
        if split_at < int(limit * 0.5):
            split_at = limit
        chunks.append(remaining[:split_at].rstrip())
        remaining = remaining[split_at:].lstrip()
    if remaining:
        chunks.append(remaining)
    return chunks


def _render_inline_markdownish_html(text: str) -> str:
    raw = str(text or "")
    pieces: list[str] = []
    buffer: list[str] = []
    index = 0

    def flush() -> None:
        if buffer:
            pieces.append(html.escape("".join(buffer)))
            buffer.clear()

    while index < len(raw):
        if raw.startswith("`", index):
            end = raw.find("`", index + 1)
            if end > index + 1 and "\n" not in raw[index + 1 : end]:
                flush()
                pieces.append(f"<code>{html.escape(raw[index + 1 : end])}</code>")
                index = end + 1
                continue
        matched = False
        for marker, tag in (("**", "b"), ("~~", "s"), ("*", "i")):
            if not raw.startswith(marker, index):
                continue
            end = raw.find(marker, index + len(marker))
            inner = raw[index + len(marker) : end] if end != -1 else ""
            if end != -1 and inner.strip():
                flush()
                pieces.append(f"<{tag}>{_render_inline_markdownish_html(inner)}</{tag}>")
                index = end + len(marker)
                matched = True
                break
        if matched:
            continue
        buffer.append(raw[index])
        index += 1
    flush()
    return "".join(pieces)


def _unwrap_quoted_line(text: str) -> str | None:
    stripped = str(text or "").strip()
    if len(stripped) < 2:
        return None
    pairs = {
        "'": "'",
        '"': '"',
        "‘": "’",
        "“": "”",
        "「": "」",
        "『": "』",
    }
    closer = pairs.get(stripped[0])
    if closer is None or stripped[-1] != closer:
        return None
    inner = stripped[1:-1].strip()
    return inner or None


def _split_markdownish_line_fragments(
    raw_line: str,
    *,
    limit: int,
    emphasize_title: bool = False,
) -> list[tuple[str, str]]:
    stripped = str(raw_line or "").strip()
    if not stripped:
        return [("", "")]
    kind = "plain"
    prefix = ""
    content = stripped
    bullet_match = re.match(r"^[-*]\s+(.*)$", stripped)
    ordered_match = re.match(r"^(\d+)[.)]\s+(.*)$", stripped)
    heading_match = re.match(r"^#{1,6}\s+(.*)$", stripped)
    quote_match = re.match(r"^>\s?(.*)$", stripped)
    quoted_inner = _unwrap_quoted_line(stripped)
    if heading_match:
        kind = "heading"
        content = heading_match.group(1).strip()
    elif bullet_match:
        kind = "bullet"
        content = bullet_match.group(1).strip()
        prefix = "• "
    elif ordered_match:
        kind = "ordered"
        content = ordered_match.group(2).strip()
        prefix = f"{ordered_match.group(1)}. "
    elif quote_match:
        kind = "quote"
        content = quote_match.group(1).strip()
        prefix = "❝ "
    elif quoted_inner is not None:
        kind = "quoted"
        content = quoted_inner
    elif emphasize_title:
        kind = "title"
    content_chunks = _split_text_fragments(content, limit=max(limit - 32, 24)) if content else [""]
    fragments: list[tuple[str, str]] = []
    for index, chunk in enumerate(content_chunks):
        rendered_content = _render_inline_markdownish_html(chunk)
        if kind in {"heading", "title"}:
            plain_line = chunk
            rendered_line = f"<b>{rendered_content}</b>"
        elif kind == "bullet":
            continuation_prefix = prefix if index == 0 else "  "
            plain_line = f"{continuation_prefix}{chunk}".rstrip()
            rendered_line = f"{continuation_prefix}{rendered_content}".rstrip()
        elif kind == "ordered":
            continuation_prefix = prefix if index == 0 else "   "
            plain_line = f"{continuation_prefix}{chunk}".rstrip()
            rendered_line = f"{continuation_prefix}{rendered_content}".rstrip()
        elif kind == "quote":
            continuation_prefix = prefix if index == 0 else "  "
            plain_line = f"{continuation_prefix}{chunk}".rstrip()
            rendered_line = f"{continuation_prefix}{rendered_content}".rstrip()
        elif kind == "quoted":
            plain_line = f"❝ {chunk} ❞".strip()
            rendered_line = f"❝ {rendered_content} ❞".strip()
        else:
            plain_line = chunk
            rendered_line = rendered_content
        fragments.append((plain_line, rendered_line))
    return fragments or [("", "")]


def _render_markdownish_paragraph_blocks(paragraph: str, *, limit: int) -> list[tuple[str, str]]:
    lines = paragraph.split("\n")
    rendered_lines: list[tuple[str, str]] = []
    for index, raw_line in enumerate(lines):
        rendered_lines.extend(
            _split_markdownish_line_fragments(
                raw_line,
                limit=limit,
                emphasize_title=index == 0 and len(lines) > 1,
            )
        )
    blocks: list[tuple[str, str]] = []
    current_plain: list[str] = []
    current_rendered: list[str] = []
    for plain_line, rendered_line in rendered_lines:
        candidate_rendered = "\n".join([*current_rendered, rendered_line]) if current_rendered else rendered_line
        if current_rendered and len(candidate_rendered) > limit:
            blocks.append(("\n".join(current_plain), "\n".join(current_rendered)))
            current_plain = [plain_line]
            current_rendered = [rendered_line]
            continue
        current_plain.append(plain_line)
        current_rendered.append(rendered_line)
    if current_rendered:
        blocks.append(("\n".join(current_plain), "\n".join(current_rendered)))
    return blocks or [(paragraph, _render_inline_markdownish_html(paragraph))]


def _render_markdownish_code_blocks(code_text: str, *, limit: int) -> list[tuple[str, str]]:
    content = str(code_text or "").rstrip("\n")
    if not content:
        return [("```\n```", "<pre><code></code></pre>")]
    lines = content.split("\n")
    blocks: list[tuple[str, str]] = []
    current_lines: list[str] = []

    def rendered_html(value: str) -> str:
        return f"<pre><code>{html.escape(value)}</code></pre>"

    def append_block(value: str) -> None:
        blocks.append((f"```\n{value}\n```", rendered_html(value)))

    for line in lines:
        if len(rendered_html(line)) > limit:
            if current_lines:
                append_block("\n".join(current_lines))
                current_lines.clear()
            fragment_limit = max(limit - len(rendered_html("")) - 8, 24)
            for fragment in _split_text_fragments(line, limit=fragment_limit):
                append_block(fragment)
            continue
        candidate_lines = [*current_lines, line]
        candidate_value = "\n".join(candidate_lines)
        if current_lines and len(rendered_html(candidate_value)) > limit:
            append_block("\n".join(current_lines))
            current_lines = [line]
            continue
        current_lines = candidate_lines
    if current_lines:
        append_block("\n".join(current_lines))
    return blocks


def _render_markdownish_message_chunks(
    text: str,
    *,
    limit: int = MAX_TELEGRAM_MESSAGE,
) -> list[TelegramMessageChunk]:
    content = str(text or "").strip()
    if not content:
        return [TelegramMessageChunk(text="(empty)", plain_text="(empty)", parse_mode=TELEGRAM_HTML_PARSE_MODE)]
    blocks: list[tuple[str, str]] = []
    paragraph_lines: list[str] = []
    code_lines: list[str] = []
    code_fence = ""
    in_code_block = False

    def flush_paragraph() -> None:
        nonlocal paragraph_lines
        if paragraph_lines:
            blocks.extend(_render_markdownish_paragraph_blocks("\n".join(paragraph_lines), limit=limit))
            paragraph_lines = []

    def flush_code() -> None:
        nonlocal code_lines, code_fence
        blocks.extend(_render_markdownish_code_blocks("\n".join(code_lines), limit=limit))
        code_lines = []
        code_fence = ""

    for raw_line in re.sub(r"\r\n?", "\n", content).split("\n"):
        if in_code_block:
            if raw_line.strip().startswith("```"):
                flush_code()
                in_code_block = False
            else:
                code_lines.append(raw_line)
            continue
        if raw_line.strip().startswith("```"):
            flush_paragraph()
            code_fence = raw_line
            in_code_block = True
            continue
        if not raw_line.strip():
            flush_paragraph()
            continue
        paragraph_lines.append(raw_line)
    if in_code_block:
        paragraph_lines = [code_fence, *code_lines]
        in_code_block = False
        code_lines = []
        code_fence = ""
    flush_paragraph()
    message_chunks: list[TelegramMessageChunk] = []
    current_plain_blocks: list[str] = []
    current_rendered_blocks: list[str] = []
    for plain_block, rendered_block in blocks:
        candidate_rendered = (
            "\n\n".join([*current_rendered_blocks, rendered_block]) if current_rendered_blocks else rendered_block
        )
        if current_rendered_blocks and len(candidate_rendered) > limit:
            message_chunks.append(
                TelegramMessageChunk(
                    text="\n\n".join(current_rendered_blocks),
                    plain_text="\n\n".join(current_plain_blocks),
                    parse_mode=TELEGRAM_HTML_PARSE_MODE,
                )
            )
            current_plain_blocks = [plain_block]
            current_rendered_blocks = [rendered_block]
            continue
        current_plain_blocks.append(plain_block)
        current_rendered_blocks.append(rendered_block)
    if current_rendered_blocks:
        message_chunks.append(
            TelegramMessageChunk(
                text="\n\n".join(current_rendered_blocks),
                plain_text="\n\n".join(current_plain_blocks),
                parse_mode=TELEGRAM_HTML_PARSE_MODE,
            )
        )
    return message_chunks or [TelegramMessageChunk(text="(empty)", plain_text="(empty)", parse_mode=TELEGRAM_HTML_PARSE_MODE)]


def _telegram_message_chunks(config: Any, text: str) -> list[TelegramMessageChunk]:
    if getattr(config, "reply_markdown_enabled", False):
        return _render_markdownish_message_chunks(text)
    return [TelegramMessageChunk(text=chunk, plain_text=chunk) for chunk in _split_message_chunks(text)]
