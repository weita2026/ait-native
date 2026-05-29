from __future__ import annotations

import json
import re
import shlex
from dataclasses import dataclass
from pathlib import Path
from typing import Any


DEFAULT_CLEAR_PHRASES = ('換個話題', '換個主題')
DEFAULT_TOPIC_LEAD_PHRASES = ('換個話題', '換個主題')
DEFAULT_TOPIC_JOINERS = ('跟', '和', '與')
DEFAULT_TOPIC_TAIL = '有關'
DEFAULT_PLANNING_MODE_PHRASES = ('進行計劃', '進行計畫', '進行计划')
TRAILING_PUNCTUATION_PATTERN = re.compile(r'[\s,，.。!！?？:：;；]+$')


@dataclass(frozen=True)
class FreshTopicClearTriggerConfig:
    phrases: tuple[str, ...]
    display_trigger: str
    allow_trailing_punctuation: bool = True


@dataclass(frozen=True)
class FreshTopicTopicTriggerConfig:
    lead_phrases: tuple[str, ...]
    joiners: tuple[str, ...]
    tail: str
    display_trigger: str
    allow_trailing_punctuation: bool = True


@dataclass(frozen=True)
class FreshTopicTriggerConfig:
    clear: FreshTopicClearTriggerConfig
    topic: FreshTopicTopicTriggerConfig


@dataclass(frozen=True)
class PlanningModeTriggerConfig:
    phrases: tuple[str, ...]
    display_trigger: str
    allow_trailing_punctuation: bool = True


@dataclass(frozen=True)
class TelegramOperationalTriggerMatchConfig:
    phrases: tuple[str, ...] = ()
    commands: tuple[str, ...] = ()
    pattern: str | None = None
    allow_trailing_punctuation: bool = True
    reply_only: bool = False
    case_sensitive: bool = False


@dataclass(frozen=True)
class TelegramOperationalTriggerConfig:
    trigger_id: str
    display_trigger: str
    handler_command: tuple[str, ...]
    source_path: str
    match: TelegramOperationalTriggerMatchConfig
    priority: int = 0


@dataclass(frozen=True)
class EventTriggerRegistry:
    fresh_topic: FreshTopicTriggerConfig
    planning_mode: PlanningModeTriggerConfig
    telegram_operational: tuple[TelegramOperationalTriggerConfig, ...] = ()


def _repo_root_from_path(repo_root: Path | None = None) -> Path:
    if repo_root is not None:
        return repo_root
    return Path(__file__).resolve().parents[2]


def _extract_json_code_block(markdown: str, file_path: Path) -> dict[str, Any]:
    match = re.search(r'```json\s*([\s\S]*?)```', markdown, re.IGNORECASE)
    if not match:
        raise ValueError(f'No JSON trigger config found in {file_path}.')
    parsed = json.loads(match.group(1).strip())
    if not isinstance(parsed, dict):
        raise ValueError(f'Trigger config in {file_path} must be a JSON object.')
    return parsed


def _clean_tuple(values: Any, *, fallback: tuple[str, ...]) -> tuple[str, ...]:
    if not isinstance(values, list):
        return fallback
    normalized: list[str] = []
    seen: set[str] = set()
    for value in values:
        if not isinstance(value, str):
            continue
        cleaned = value.strip()
        key = cleaned.lower()
        if not cleaned or key in seen:
            continue
        seen.add(key)
        normalized.append(cleaned)
    return tuple(normalized) or fallback


def _clean_optional_str(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _clean_command_tuple(values: Any) -> tuple[str, ...]:
    if isinstance(values, str):
        return tuple(part for part in shlex.split(values.strip()) if part)
    if not isinstance(values, list):
        return ()
    normalized: list[str] = []
    for value in values:
        cleaned = _clean_optional_str(value)
        if cleaned:
            normalized.append(cleaned)
    return tuple(normalized)


def _clean_command_names(values: Any) -> tuple[str, ...]:
    if not isinstance(values, list):
        return ()
    normalized: list[str] = []
    seen: set[str] = set()
    for value in values:
        cleaned = str(value or '').strip().lstrip('/').lower()
        if not cleaned or cleaned in seen:
            continue
        seen.add(cleaned)
        normalized.append(cleaned)
    return tuple(normalized)


def default_event_trigger_registry() -> EventTriggerRegistry:
    return EventTriggerRegistry(
        fresh_topic=FreshTopicTriggerConfig(
            clear=FreshTopicClearTriggerConfig(
                phrases=DEFAULT_CLEAR_PHRASES,
                display_trigger='換個話題',
                allow_trailing_punctuation=True,
            ),
            topic=FreshTopicTopicTriggerConfig(
                lead_phrases=DEFAULT_TOPIC_LEAD_PHRASES,
                joiners=DEFAULT_TOPIC_JOINERS,
                tail=DEFAULT_TOPIC_TAIL,
                display_trigger='換個話題跟…有關',
                allow_trailing_punctuation=True,
            ),
        ),
        planning_mode=PlanningModeTriggerConfig(
            phrases=DEFAULT_PLANNING_MODE_PHRASES,
            display_trigger='進行計劃',
            allow_trailing_punctuation=True,
        ),
        telegram_operational=(),
    )


def _load_json_config(resolved_root: Path, relative_path: str) -> dict[str, Any]:
    config_path = resolved_root / relative_path
    try:
        return _extract_json_code_block(config_path.read_text(encoding='utf-8'), config_path)
    except (OSError, ValueError, json.JSONDecodeError):
        return {}


def _relative_repo_path(path: Path, repo_root: Path) -> str:
    try:
        return path.resolve().relative_to(repo_root.resolve()).as_posix()
    except ValueError:
        return str(path)


def _parse_operational_trigger_config(
    repo_root: Path,
    path: Path,
    payload: dict[str, Any],
) -> TelegramOperationalTriggerConfig | None:
    kind = str(payload.get('kind') or '').strip().lower()
    if kind not in {'telegram_operational_trigger', 'telegram-operational-trigger'}:
        return None
    match_payload = payload.get('match') if isinstance(payload.get('match'), dict) else {}
    handler_command = _clean_command_tuple(payload.get('handlerCommand'))
    trigger_id = _clean_optional_str(payload.get('id'))
    display_trigger = _clean_optional_str(payload.get('displayTrigger'))
    phrases = _clean_tuple(match_payload.get('phrases'), fallback=())
    commands = _clean_command_names(match_payload.get('commands'))
    pattern = _clean_optional_str(match_payload.get('pattern'))
    if not trigger_id or not handler_command or (not phrases and not commands and not pattern):
        return None
    return TelegramOperationalTriggerConfig(
        trigger_id=trigger_id,
        display_trigger=display_trigger or trigger_id,
        handler_command=handler_command,
        source_path=_relative_repo_path(path, repo_root),
        match=TelegramOperationalTriggerMatchConfig(
            phrases=phrases,
            commands=commands,
            pattern=pattern,
            allow_trailing_punctuation=bool(match_payload.get('allowTrailingPunctuation', True)),
            reply_only=bool(match_payload.get('replyOnly', False)),
            case_sensitive=bool(match_payload.get('caseSensitive', False)),
        ),
        priority=int(payload.get('priority') or 0),
    )


def _load_operational_trigger_configs(resolved_root: Path) -> tuple[TelegramOperationalTriggerConfig, ...]:
    trigger_dir = resolved_root / 'docs' / 'event_trigger'
    if not trigger_dir.is_dir():
        return ()
    configs: list[TelegramOperationalTriggerConfig] = []
    for path in sorted(trigger_dir.glob('*.md')):
        try:
            payload = _extract_json_code_block(path.read_text(encoding='utf-8'), path)
        except (OSError, ValueError, json.JSONDecodeError):
            continue
        config = _parse_operational_trigger_config(resolved_root, path, payload)
        if config is not None:
            configs.append(config)
    configs.sort(key=lambda item: (-item.priority, item.source_path, item.trigger_id))
    return tuple(configs)


def load_event_trigger_registry(repo_root: Path | None = None) -> EventTriggerRegistry:
    resolved_root = _repo_root_from_path(repo_root)
    fallback = default_event_trigger_registry()
    payload = _load_json_config(resolved_root, 'docs/event_trigger/fresh_topic.md')
    planning_payload = _load_json_config(resolved_root, 'docs/event_trigger/planning_mode.md')
    clear_payload = payload.get('clear') if isinstance(payload.get('clear'), dict) else {}
    topic_payload = payload.get('topic') if isinstance(payload.get('topic'), dict) else {}
    return EventTriggerRegistry(
        fresh_topic=FreshTopicTriggerConfig(
            clear=FreshTopicClearTriggerConfig(
                phrases=_clean_tuple(clear_payload.get('phrases'), fallback=fallback.fresh_topic.clear.phrases),
                display_trigger=str(clear_payload.get('displayTrigger') or fallback.fresh_topic.clear.display_trigger).strip()
                or fallback.fresh_topic.clear.display_trigger,
                allow_trailing_punctuation=bool(
                    clear_payload.get('allowTrailingPunctuation', fallback.fresh_topic.clear.allow_trailing_punctuation)
                ),
            ),
            topic=FreshTopicTopicTriggerConfig(
                lead_phrases=_clean_tuple(topic_payload.get('leadPhrases'), fallback=fallback.fresh_topic.topic.lead_phrases),
                joiners=_clean_tuple(topic_payload.get('joiners'), fallback=fallback.fresh_topic.topic.joiners),
                tail=str(topic_payload.get('tail') or fallback.fresh_topic.topic.tail).strip()
                or fallback.fresh_topic.topic.tail,
                display_trigger=str(topic_payload.get('displayTrigger') or fallback.fresh_topic.topic.display_trigger).strip()
                or fallback.fresh_topic.topic.display_trigger,
                allow_trailing_punctuation=bool(
                    topic_payload.get('allowTrailingPunctuation', fallback.fresh_topic.topic.allow_trailing_punctuation)
                ),
            ),
        ),
        planning_mode=PlanningModeTriggerConfig(
            phrases=_clean_tuple(planning_payload.get('phrases'), fallback=fallback.planning_mode.phrases),
            display_trigger=str(planning_payload.get('displayTrigger') or fallback.planning_mode.display_trigger).strip()
            or fallback.planning_mode.display_trigger,
            allow_trailing_punctuation=bool(
                planning_payload.get('allowTrailingPunctuation', fallback.planning_mode.allow_trailing_punctuation)
            ),
        ),
        telegram_operational=_load_operational_trigger_configs(resolved_root),
    )


def _strip_allowed_trailing_punctuation(text: str, *, enabled: bool) -> str:
    if not enabled:
        return text.strip()
    return TRAILING_PUNCTUATION_PATTERN.sub('', str(text or '').strip()).strip()


def parse_fresh_topic_trigger(text: str, config: FreshTopicTriggerConfig) -> dict[str, Any] | None:
    raw = str(text or '').strip()
    if not raw:
        return None
    stripped = _strip_allowed_trailing_punctuation(
        raw,
        enabled=config.clear.allow_trailing_punctuation or config.topic.allow_trailing_punctuation,
    )
    lowered = stripped.lower()
    for phrase in config.clear.phrases:
        if lowered == phrase.lower():
            return {
                'mode': 'clear',
                'topic': None,
                'display_trigger': config.clear.display_trigger,
            }
    escaped_leads = '|'.join(re.escape(item) for item in config.topic.lead_phrases)
    escaped_joiners = '|'.join(re.escape(item) for item in config.topic.joiners)
    escaped_tail = re.escape(config.topic.tail)
    pattern = re.compile(
        rf'^(?:{escaped_leads})(?:{escaped_joiners})(?P<topic>.+?)(?:{escaped_tail})$',
        re.IGNORECASE,
    )
    match = pattern.match(stripped)
    if not match:
        return None
    topic = match.group('topic').strip().strip(' ,，.。!！?？:：;；')
    if not topic:
        return None
    return {
        'mode': 'topic',
        'topic': topic,
        'display_trigger': config.topic.display_trigger,
    }


def parse_planning_mode_trigger(text: str, config: PlanningModeTriggerConfig) -> dict[str, Any] | None:
    raw = str(text or '').strip()
    if not raw:
        return None
    stripped = _strip_allowed_trailing_punctuation(raw, enabled=config.allow_trailing_punctuation)
    lowered = stripped.lower()
    for phrase in config.phrases:
        if lowered == phrase.lower():
            return {
                'mode': 'planning',
                'display_trigger': config.display_trigger,
            }
    return None


def parse_telegram_operational_trigger(
    *,
    raw_text: str,
    normalized_text: str,
    command: tuple[str, str] | None,
    reply_to_message_id: int | None,
    config: TelegramOperationalTriggerConfig,
) -> dict[str, Any] | None:
    match_config = config.match
    if match_config.reply_only and reply_to_message_id is None:
        return None
    if command is not None and match_config.commands:
        command_name, command_args = command
        normalized_name = str(command_name or '').strip().lstrip('/').lower()
        if normalized_name in match_config.commands:
            return {
                'mode': 'command',
                'display_trigger': config.display_trigger,
                'command_name': normalized_name,
                'command_args': str(command_args or '').strip(),
            }
    candidate = _strip_allowed_trailing_punctuation(
        normalized_text,
        enabled=match_config.allow_trailing_punctuation,
    )
    if not candidate:
        return None
    compare_candidate = candidate if match_config.case_sensitive else candidate.lower()
    for phrase in match_config.phrases:
        compare_phrase = phrase if match_config.case_sensitive else phrase.lower()
        if compare_candidate == compare_phrase:
            return {
                'mode': 'phrase',
                'display_trigger': config.display_trigger,
                'matched_text': candidate,
            }
    if match_config.pattern:
        pattern_flags = 0 if match_config.case_sensitive else re.IGNORECASE
        match = re.match(match_config.pattern, raw_text or candidate, pattern_flags)
        if match:
            return {
                'mode': 'pattern',
                'display_trigger': config.display_trigger,
                'matched_text': match.group(0),
                'groups': list(match.groups()),
                'groupdict': dict(match.groupdict()),
            }
    return None


__all__ = [
    'EventTriggerRegistry',
    'FreshTopicClearTriggerConfig',
    'FreshTopicTopicTriggerConfig',
    'FreshTopicTriggerConfig',
    'PlanningModeTriggerConfig',
    'TelegramOperationalTriggerConfig',
    'TelegramOperationalTriggerMatchConfig',
    'default_event_trigger_registry',
    'load_event_trigger_registry',
    'parse_fresh_topic_trigger',
    'parse_planning_mode_trigger',
    'parse_telegram_operational_trigger',
]
