from __future__ import annotations

import json
import re
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
class EventTriggerRegistry:
    fresh_topic: FreshTopicTriggerConfig
    planning_mode: PlanningModeTriggerConfig


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
    )


def _load_json_config(resolved_root: Path, relative_path: str) -> dict[str, Any]:
    config_path = resolved_root / relative_path
    try:
        return _extract_json_code_block(config_path.read_text(encoding='utf-8'), config_path)
    except (OSError, ValueError, json.JSONDecodeError):
        return {}


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
