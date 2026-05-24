from __future__ import annotations

import copy
import json
import re
import secrets
import sqlite3
import time
import urllib.parse
from datetime import datetime, timezone
from enum import Enum
from pathlib import Path
from typing import Any

POLICY_REQUIREMENT_FLAGS = (
    "require_attestation",
    "require_tests",
    "require_lint",
    "require_security_scan",
    "require_license_scan",
    "require_ai_provenance",
    "require_code_review_summary",
)

POLICY_CONTENT_CLASSES = ("docs_only", "code_change")
POLICY_AUTHOR_CLASSES = ("human_only", "ai_related")
DEFAULT_ID_NAMESPACE_PREFIX = "AIT"
LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX = "L"
REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX = "R"
WORKFLOW_TASK_CHANGE_ORIGIN_NAMESPACE_PREFIXES = (
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX,
    REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX,
)
WORKFLOW_ID_FAMILIES = frozenset({"T", "C", "P", "R", "S", "PS", "K", "PL", "PR", "SK", "HP", "AM", "AN", "AMU"})
RESERVED_WORKFLOW_TOKENS = frozenset({"AT", "LAND", "W"})
CODE_REVIEW_SUMMARY_SECTION_LABELS: dict[str, tuple[str, ...]] = {
    "Reviewed files": ("reviewed files", "files reviewed", "reviewed file", "files", "paths reviewed"),
    "Findings": ("findings", "issues", "observations"),
    "Risks": ("risks", "risk", "residual risks", "regression risks"),
    "Tests": ("tests", "verification", "validation", "checks"),
    "Recommendation": ("recommendation", "promotion recommendation", "land recommendation", "verdict", "decision"),
}
CODE_REVIEW_SUMMARY_TEMPLATE = (
    "Reviewed files: <paths reviewed>; Findings: <blocking/non-blocking findings>; "
    "Risks: <residual risks>; Tests: <checks run>; Recommendation: <land/defer/request changes>"
)
CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE = (
    "1. Reviewed files\n<paths reviewed>\n"
    "2. Findings\n<blocking/non-blocking findings>\n"
    "3. Risks\n<residual risks>\n"
    "4. Tests\n<checks run>\n"
    "5. Recommendation\n<land/defer/request changes>"
)
CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND = "ait review code template --style numbered"


def _default_policy_class_overrides() -> list[dict[str, Any]]:
    return [
        {
            "when": {"content_class": "docs_only"},
            "set": {
                "require_tests": False,
                "require_lint": False,
                "require_security_scan": False,
                "require_license_scan": False,
            },
        },
    ]


POLICY_PROFILES: dict[str, dict[str, Any]] = {
    "prototype": {
        "version": 1,
        "policy_id": "prototype",
        "defaults": {
            "require_attestation": True,
            "require_tests": True,
            "require_lint": False,
            "require_security_scan": False,
            "require_license_scan": False,
            "require_ai_provenance": False,
            "require_code_review_summary": False,
        },
        "class_overrides": _default_policy_class_overrides(),
    },
    "team": {
        "version": 1,
        "policy_id": "team",
        "defaults": {
            "require_attestation": True,
            "require_tests": True,
            "require_lint": True,
            "require_security_scan": False,
            "require_license_scan": False,
            "require_ai_provenance": False,
            "require_code_review_summary": False,
        },
        "class_overrides": _default_policy_class_overrides(),
    },
    "release": {
        "version": 1,
        "policy_id": "release",
        "defaults": {
            "require_attestation": True,
            "require_tests": True,
            "require_lint": True,
            "require_security_scan": True,
            "require_license_scan": True,
            "require_ai_provenance": False,
            "require_code_review_summary": False,
        },
        "class_overrides": _default_policy_class_overrides(),
    },
}


class AuthorMode(str, Enum):
    HUMAN_ONLY = "human_only"
    HUMAN_WITH_AI_ASSIST = "human_with_ai_assist"
    AI_WITH_HUMAN_REVIEW = "ai_with_human_review"
    AI_ONLY_EXPERIMENTAL = "ai_only_experimental"


class StorageIngestMode(str, Enum):
    DEFAULT = "default"
    PACK_FULL = "pack_full"
    PACK_DELTA = "pack_delta"


AI_RELATED_AUTHOR_MODES = frozenset(
    {
        AuthorMode.HUMAN_WITH_AI_ASSIST.value,
        AuthorMode.AI_WITH_HUMAN_REVIEW.value,
        AuthorMode.AI_ONLY_EXPERIMENTAL.value,
    }
)


_CROCKFORD_BASE32 = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
_PLAN_ITEM_REF_RE = re.compile(r"\[ref:\s*([A-Za-z0-9][A-Za-z0-9._/-]*)\]", re.IGNORECASE)
_PLAN_SECTION_REF_RE = re.compile(r"\[plan-ref:\s*([A-Za-z0-9][A-Za-z0-9._/-]*)\]", re.IGNORECASE)
_MARKDOWN_HEADING_RE = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
_MARKDOWN_LIST_ITEM_RE = re.compile(r"^\s*(?:[-*+]|\d+\.)\s+(?:\[(?P<checked>[ xX])\]\s+)?(?P<text>.+?)\s*$")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def connect_sqlite(db_path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    conn.execute("pragma foreign_keys = on")
    return conn


def read_json(path: Path, default: Any | None = None) -> Any:
    if not path.exists():
        return default
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, data: Any) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=True), encoding="utf-8")


def policy_profile_names() -> list[str]:
    return sorted(POLICY_PROFILES)


def author_mode_values() -> list[str]:
    return [mode.value for mode in AuthorMode]


def storage_ingest_mode_values(*, include_default: bool = True) -> list[str]:
    values = [mode.value for mode in StorageIngestMode]
    if include_default:
        return values
    return [value for value in values if value != StorageIngestMode.DEFAULT.value]


def policy_content_class_values() -> list[str]:
    return list(POLICY_CONTENT_CLASSES)


def policy_author_class_values() -> list[str]:
    return list(POLICY_AUTHOR_CLASSES)


def normalize_author_mode(value: str | AuthorMode) -> str:
    if isinstance(value, AuthorMode):
        return value.value
    text = str(value).strip()
    try:
        return AuthorMode(text).value
    except ValueError as exc:
        raise ValueError(
            f"Unknown author_mode: {value}. Expected one of: {', '.join(author_mode_values())}"
        ) from exc


def normalize_optional_text(value: Any | None) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _code_review_summary_label_pattern() -> re.Pattern[str]:
    labels = sorted(
        {
            label
            for aliases in CODE_REVIEW_SUMMARY_SECTION_LABELS.values()
            for label in aliases
        },
        key=len,
        reverse=True,
    )
    label_pattern = "|".join(re.escape(label) for label in labels)
    return re.compile(
        rf"(?:^|[\n;])\s*(?:(?:[-*]|\d+\.)\s*)?(?:#{{1,6}}\s*)?(?:\*\*)?(?P<label>{label_pattern})(?:\*\*)?\s*(?:[:\-–—]\s*|\n+)",
        re.IGNORECASE,
    )


def _code_review_summary_section_has_content(value: str) -> bool:
    text = value.strip()
    placeholder = text.lower().strip(". ")
    if not text or placeholder in {"", "todo", "tbd", "replace me", "replace_me"}:
        return False
    return not (text.startswith("<") and text.endswith(">"))


def missing_code_review_summary_sections(value: Any | None) -> list[str]:
    text = normalize_optional_text(value)
    if text is None:
        return list(CODE_REVIEW_SUMMARY_SECTION_LABELS)
    matches = list(_code_review_summary_label_pattern().finditer(text))
    present: set[str] = set()
    for index, match in enumerate(matches):
        label = match.group("label").strip().lower()
        canonical = next(
            (
                section
                for section, aliases in CODE_REVIEW_SUMMARY_SECTION_LABELS.items()
                if label in aliases
            ),
            None,
        )
        if canonical is None:
            continue
        next_start = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        section_body = text[match.end() : next_start].strip()
        if _code_review_summary_section_has_content(section_body):
            present.add(canonical)
    return [section for section in CODE_REVIEW_SUMMARY_SECTION_LABELS if section not in present]


def is_structured_code_review_summary(value: Any | None) -> bool:
    return not missing_code_review_summary_sections(value)


def render_code_review_summary_template(style: str = "inline") -> str:
    normalized_style = str(style or "").strip().lower()
    if normalized_style == "inline":
        return CODE_REVIEW_SUMMARY_TEMPLATE
    if normalized_style == "numbered":
        return CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE
    raise ValueError("Unknown code review summary template style. Expected one of: inline, numbered.")


def code_review_summary_requirement_text(value: Any | None = None) -> str:
    required = ", ".join(CODE_REVIEW_SUMMARY_SECTION_LABELS)
    missing = missing_code_review_summary_sections(value) if value is not None else []
    if missing:
        return (
            "Code review summary is missing sections with non-placeholder content: "
            + ", ".join(missing)
            + f". Required sections: {required}. Run `{CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND}` for a safe scaffold."
        )
    return (
        "Code review summary requires sections with non-placeholder content: "
        + required
        + f". Run `{CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND}` for a safe scaffold."
    )


def extract_plan_items(body_markdown: str | None) -> list[dict[str, Any]]:
    items: list[dict[str, Any]] = []
    heading_path: list[str] = []
    for line_number, raw_line in enumerate(str(body_markdown or "").splitlines(), start=1):
        heading_match = _MARKDOWN_HEADING_RE.match(raw_line)
        if heading_match is not None:
            level = len(heading_match.group(1))
            title = _PLAN_SECTION_REF_RE.sub("", heading_match.group(2)).strip()
            if title:
                heading_path = heading_path[: level - 1]
                heading_path.append(title)
            continue
        list_match = _MARKDOWN_LIST_ITEM_RE.match(raw_line)
        if list_match is None:
            continue
        text = list_match.group("text").strip()
        ref_match = _PLAN_ITEM_REF_RE.search(text)
        if ref_match is None:
            continue
        plan_item_ref = ref_match.group(1).strip()
        display_text = _PLAN_ITEM_REF_RE.sub("", text).strip()
        checked = list_match.group("checked")
        checkbox_state = "done" if checked and checked.lower() == "x" else "open" if checked is not None else "none"
        items.append(
            {
                "plan_item_ref": plan_item_ref,
                "text": display_text,
                "checkbox_state": checkbox_state,
                "heading_path": list(heading_path),
                "line_number": line_number,
            }
        )
    return items


def find_plan_item(body_markdown: str | None, plan_item_ref: str | None) -> dict[str, Any] | None:
    normalized_ref = normalize_optional_text(plan_item_ref)
    if normalized_ref is None:
        return None
    for item in extract_plan_items(body_markdown):
        if item["plan_item_ref"] == normalized_ref:
            return item
    return None


def list_plan_section_refs(body_markdown: str | None) -> list[dict[str, Any]]:
    refs: list[dict[str, Any]] = []
    for line_number, raw_line in enumerate(str(body_markdown or "").splitlines(), start=1):
        heading_match = _MARKDOWN_HEADING_RE.match(raw_line)
        if heading_match is None:
            continue
        ref_match = _PLAN_SECTION_REF_RE.search(heading_match.group(2))
        if ref_match is None:
            continue
        refs.append(
            {
                "plan_ref": ref_match.group(1).strip(),
                "heading_title": _PLAN_SECTION_REF_RE.sub("", heading_match.group(2)).strip(),
                "heading_level": len(heading_match.group(1)),
                "line_number": line_number,
            }
        )
    return refs


def extract_plan_section(body_markdown: str | None, plan_ref: str | None) -> dict[str, Any] | None:
    normalized_ref = normalize_optional_text(plan_ref)
    if normalized_ref is None:
        return None
    lines = str(body_markdown or "").splitlines()
    start_index: int | None = None
    heading_level = 0
    heading_title = ""
    for index, raw_line in enumerate(lines):
        heading_match = _MARKDOWN_HEADING_RE.match(raw_line)
        if heading_match is None:
            continue
        ref_match = _PLAN_SECTION_REF_RE.search(heading_match.group(2))
        if ref_match is None or ref_match.group(1).strip() != normalized_ref:
            continue
        start_index = index
        heading_level = len(heading_match.group(1))
        heading_title = _PLAN_SECTION_REF_RE.sub("", heading_match.group(2)).strip()
        break
    if start_index is None:
        return None
    end_index = len(lines)
    for index in range(start_index + 1, len(lines)):
        heading_match = _MARKDOWN_HEADING_RE.match(lines[index])
        if heading_match is None:
            continue
        if len(heading_match.group(1)) <= heading_level:
            end_index = index
            break
    section_markdown = "\n".join(lines[start_index:end_index]).strip()
    items = extract_plan_items(section_markdown)
    for item in items:
        item["line_number"] = int(item.get("line_number") or 0) + start_index
    return {
        "plan_ref": normalized_ref,
        "heading_title": heading_title,
        "heading_level": heading_level,
        "line_number": start_index + 1,
        "section_markdown": section_markdown,
        "items": items,
    }


def normalize_plan_items(items: Any | None) -> list[dict[str, Any]]:
    normalized_items: list[dict[str, Any]] = []
    seen_refs: set[str] = set()
    for raw_item in items or []:
        if not isinstance(raw_item, dict):
            raise ValueError("Plan items must be objects.")
        plan_item_ref = normalize_optional_text(raw_item.get("plan_item_ref"))
        if plan_item_ref is None:
            raise ValueError("Plan items must include plan_item_ref.")
        if plan_item_ref in seen_refs:
            raise ValueError(f"Duplicate plan_item_ref in plan revision: {plan_item_ref}")
        seen_refs.add(plan_item_ref)
        checkbox_state = normalize_optional_text(raw_item.get("checkbox_state")) or "none"
        if checkbox_state not in {"open", "done", "none"}:
            raise ValueError(
                f"Unsupported checkbox_state for plan item {plan_item_ref}: {checkbox_state}. Expected open, done, or none."
            )
        heading_path_raw = raw_item.get("heading_path")
        if heading_path_raw is None:
            heading_path: list[str] = []
        elif isinstance(heading_path_raw, list):
            heading_path = [str(value).strip() for value in heading_path_raw if str(value).strip()]
        else:
            raise ValueError(f"Plan item {plan_item_ref} heading_path must be a list.")
        try:
            line_number = int(raw_item.get("line_number") or 0)
        except (TypeError, ValueError) as exc:
            raise ValueError(f"Plan item {plan_item_ref} line_number must be an integer.") from exc
        normalized_items.append(
            {
                "plan_item_ref": plan_item_ref,
                "text": str(raw_item.get("text") or "").strip(),
                "checkbox_state": checkbox_state,
                "heading_path": heading_path,
                "line_number": line_number,
            }
        )
    return normalized_items


def find_plan_item_in_items(items: Any | None, plan_item_ref: str | None) -> dict[str, Any] | None:
    normalized_ref = normalize_optional_text(plan_item_ref)
    if normalized_ref is None:
        return None
    for item in normalize_plan_items(items):
        if item["plan_item_ref"] == normalized_ref:
            return item
    return None


def derive_policy_content_class(changed_paths: list[str] | None) -> str:
    normalized_paths = [str(path).strip() for path in (changed_paths or []) if str(path).strip()]
    if normalized_paths and all(path.lower().endswith(".md") for path in normalized_paths):
        return "docs_only"
    return "code_change"


def derive_policy_author_class(author_mode: str | None) -> str | None:
    normalized = normalize_optional_text(author_mode)
    if normalized is None:
        return None
    try:
        author_mode_value = normalize_author_mode(normalized)
    except ValueError:
        return None
    if author_mode_value in AI_RELATED_AUTHOR_MODES:
        return "ai_related"
    return "human_only"


def build_minimum_provenance(
    author_mode: str | AuthorMode,
    *,
    model_name: str | None = None,
    session_id: str | None = None,
    checkpoint_id: str | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    author_mode_value = normalize_author_mode(author_mode)
    model_name_value = normalize_optional_text(model_name)
    session_id_value = normalize_optional_text(session_id)
    checkpoint_id_value = normalize_optional_text(checkpoint_id)
    required_fields = ["model_name", "session_id", "checkpoint_id"] if author_mode_value in AI_RELATED_AUTHOR_MODES else []
    field_values = {
        "model_name": model_name_value,
        "session_id": session_id_value,
        "checkpoint_id": checkpoint_id_value,
    }
    missing_fields = [name for name in required_fields if not field_values.get(name)]
    if not required_fields:
        evidence_readiness = "not_required"
    elif missing_fields:
        evidence_readiness = "partial"
    else:
        evidence_readiness = "complete"
    policy_readable = not missing_fields
    provenance_summary = {
        "model_name": model_name_value,
        "session_id": session_id_value,
        "checkpoint_id": checkpoint_id_value,
        "evidence_readiness": evidence_readiness,
        "missing_fields": missing_fields,
        "policy_readable": policy_readable,
    }
    detail = {
        "minimum_evidence": {
            "author_mode": author_mode_value,
            "model_name": model_name_value,
            "session_id": session_id_value,
            "checkpoint_id": checkpoint_id_value,
            "required_fields": required_fields,
            "missing_fields": missing_fields,
            "policy_readable": policy_readable,
        }
    }
    return provenance_summary, detail


def normalize_storage_ingest_mode(value: str | StorageIngestMode | None, *, allow_default: bool = True) -> str:
    if value is None:
        resolved = StorageIngestMode.DEFAULT.value if allow_default else StorageIngestMode.PACK_DELTA.value
    elif isinstance(value, StorageIngestMode):
        resolved = value.value
    else:
        resolved = str(value).strip() or (StorageIngestMode.DEFAULT.value if allow_default else StorageIngestMode.PACK_DELTA.value)
    if not allow_default and resolved == StorageIngestMode.DEFAULT.value:
        resolved = StorageIngestMode.PACK_DELTA.value
    try:
        normalized = StorageIngestMode(resolved).value
    except ValueError as exc:
        expected = ", ".join(storage_ingest_mode_values(include_default=allow_default))
        raise ValueError(f"Unknown storage_ingest_mode: {value}. Expected one of: {expected}") from exc
    if not allow_default and normalized == StorageIngestMode.DEFAULT.value:
        return StorageIngestMode.PACK_DELTA.value
    return normalized


def lane_from_risk(risk_tier: str) -> str:
    if risk_tier == "low":
        return "auto"
    if risk_tier == "critical":
        return "critical"
    return "assisted"


def normalize_id_namespace_prefix(value: Any, *, default: str | None = None) -> str:
    if value is None:
        if default is None:
            raise ValueError("id namespace prefix is required")
        value = default
    text = str(value).strip().upper()
    if text and not re.fullmatch(r"[A-Z0-9]+", text):
        raise ValueError("id namespace prefix must contain only ASCII letters or digits")
    for code in WORKFLOW_ID_FAMILIES:
        token = f"{text}{code}" if text else code
        if token in RESERVED_WORKFLOW_TOKENS:
            raise ValueError(
                f"id namespace prefix {text!r} collides with reserved workflow token {token!r}"
            )
    return text


def workflow_id_token(family: str, namespace_prefix: str | None = None) -> str:
    resolved_family = str(family or "").strip().upper()
    if resolved_family not in WORKFLOW_ID_FAMILIES:
        raise ValueError(f"Unsupported workflow id family: {family!r}")
    resolved_namespace = normalize_id_namespace_prefix(namespace_prefix, default=DEFAULT_ID_NAMESPACE_PREFIX)
    return f"{resolved_namespace}{resolved_family}" if resolved_namespace else resolved_family


def workflow_id_tokens(
    family: str,
    namespace_prefix: str | None = None,
    *,
    include_legacy: bool = True,
) -> tuple[str, ...]:
    tokens = [workflow_id_token(family, namespace_prefix)]
    legacy = workflow_id_token(family, DEFAULT_ID_NAMESPACE_PREFIX)
    if include_legacy and legacy not in tokens:
        tokens.append(legacy)
    return tuple(tokens)


def workflow_origin_namespace_prefix(origin_prefix: str, namespace_prefix: str | None = None) -> str:
    resolved_origin = str(origin_prefix or "").strip().upper()
    if resolved_origin not in WORKFLOW_TASK_CHANGE_ORIGIN_NAMESPACE_PREFIXES:
        raise ValueError(f"Unsupported workflow origin prefix: {origin_prefix!r}")
    resolved_namespace = normalize_id_namespace_prefix(namespace_prefix, default=DEFAULT_ID_NAMESPACE_PREFIX)
    return f"{resolved_origin}{resolved_namespace}" if resolved_namespace else resolved_origin


def workflow_id_namespace_prefix_candidates(
    namespace_prefix: str | None = None,
    *,
    include_legacy: bool = True,
    include_task_change_origins: bool = False,
) -> tuple[str, ...]:
    candidates: list[str] = []
    base_prefixes: list[str] = []

    def _append(value: str | None) -> None:
        if value is None:
            return
        normalized = normalize_id_namespace_prefix(value, default=DEFAULT_ID_NAMESPACE_PREFIX)
        if normalized not in base_prefixes:
            base_prefixes.append(normalized)
        if normalized not in candidates:
            candidates.append(normalized)

    if namespace_prefix is not None:
        _append(namespace_prefix)
    if include_legacy:
        _append("")
        _append(DEFAULT_ID_NAMESPACE_PREFIX)
    if include_task_change_origins:
        origin_candidates: list[str] = []
        for base_prefix in base_prefixes:
            for origin_prefix in WORKFLOW_TASK_CHANGE_ORIGIN_NAMESPACE_PREFIXES:
                derived_prefix = workflow_origin_namespace_prefix(origin_prefix, base_prefix)
                if derived_prefix not in origin_candidates:
                    origin_candidates.append(derived_prefix)
        candidates = [*origin_candidates, *candidates]
    return tuple(candidates)


def workflow_id_namespace_prefix_for_value(
    value: str | None,
    family: str,
    namespace_prefix: str | None = None,
    *,
    include_legacy: bool = True,
    include_task_change_origins: bool = False,
) -> str | None:
    text = str(value or "").strip().upper()
    if "-" not in text:
        return None
    token, _ = text.split("-", 1)
    for prefix in workflow_id_namespace_prefix_candidates(
        namespace_prefix,
        include_legacy=include_legacy,
        include_task_change_origins=include_task_change_origins,
    ):
        if token == workflow_id_token(family, prefix):
            return prefix
    return None


def workflow_id_matches_any_namespace_prefix(
    value: str | None,
    family: str,
    namespace_prefix: str | None = None,
    *,
    include_legacy: bool = True,
    include_task_change_origins: bool = False,
) -> bool:
    return (
        workflow_id_namespace_prefix_for_value(
            value,
            family,
            namespace_prefix,
            include_legacy=include_legacy,
            include_task_change_origins=include_task_change_origins,
        )
        is not None
    )


def workflow_id_matches(
    value: str | None,
    family: str,
    namespace_prefix: str | None = None,
    *,
    include_legacy: bool = True,
) -> bool:
    text = str(value or "").strip().upper()
    if "-" not in text:
        return False
    token, _ = text.split("-", 1)
    return token in workflow_id_tokens(family, namespace_prefix, include_legacy=include_legacy)


def generate_namespaced_workflow_id(family: str, namespace_prefix: str | None = None) -> str:
    return f"{workflow_id_token(family, namespace_prefix)}-{_generate_ulid()}"


def generate_namespaced_sequence_id(
    family: str,
    number: int,
    namespace_prefix: str | None = None,
    *,
    width: int = 4,
) -> str:
    return f"{workflow_id_token(family, namespace_prefix)}-{int(number):0{width}d}"


def derive_patchset_id(
    change_id: str,
    patchset_number: int,
    namespace_prefix: str | None = None,
) -> str:
    text = str(change_id or "").strip().upper()
    if "-" not in text:
        raise ValueError(f"Unsupported change id: {change_id!r}")
    resolved_prefix = workflow_id_namespace_prefix_for_value(
        text,
        "C",
        namespace_prefix,
        include_task_change_origins=True,
    )
    if resolved_prefix is None:
        raise ValueError(f"Unsupported change id: {change_id!r}")
    patch_token = workflow_id_token("P", resolved_prefix)
    return f"{patch_token}-{text.split('-', 1)[1]}-{int(patchset_number)}"


def generate_workflow_id(prefix: str) -> str:
    return f"{prefix}-{_generate_ulid()}"


def _generate_ulid() -> str:
    timestamp_ms = int(time.time() * 1000)
    randomness = int.from_bytes(secrets.token_bytes(10), "big")
    return _encode_crockford_base32(timestamp_ms, 10) + _encode_crockford_base32(randomness, 16)


def _encode_crockford_base32(value: int, length: int) -> str:
    if value < 0:
        raise ValueError("ULID parts must be non-negative")
    chars = ["0"] * length
    for idx in range(length - 1, -1, -1):
        chars[idx] = _CROCKFORD_BASE32[value & 0b11111]
        value >>= 5
    if value:
        raise ValueError("Value does not fit requested Crockford base32 length")
    return "".join(chars)


def policy_profile(name: str) -> dict[str, Any]:
    profile_name = (name or "prototype").strip().lower()
    if profile_name not in POLICY_PROFILES:
        raise KeyError(f"Unknown policy profile: {name}")
    return copy.deepcopy(POLICY_PROFILES[profile_name])


def normalize_policy(policy: dict[str, Any] | None, *, fallback_profile: str = "prototype") -> dict[str, Any]:
    base = policy_profile(fallback_profile)
    payload = policy if isinstance(policy, dict) else {}
    policy_id = str(payload.get("policy_id") or base["policy_id"]).strip() or base["policy_id"]
    if policy_id in POLICY_PROFILES:
        normalized = policy_profile(policy_id)
    else:
        normalized = copy.deepcopy(base)
        normalized["policy_id"] = policy_id
    raw_defaults = payload.get("defaults")
    defaults = raw_defaults if isinstance(raw_defaults, dict) else {}
    for key in POLICY_REQUIREMENT_FLAGS:
        normalized["defaults"][key] = _coerce_bool(defaults.get(key), normalized["defaults"].get(key, False))
    try:
        normalized["version"] = int(payload.get("version", normalized.get("version", 1)))
    except Exception:
        normalized["version"] = 1
    raw_overrides = payload.get("class_overrides")
    if raw_overrides is None:
        raw_overrides = normalized.get("class_overrides", [])
    normalized["class_overrides"] = _normalize_policy_class_overrides(raw_overrides, normalized["defaults"])
    return normalized


def resolve_effective_policy(
    policy: dict[str, Any] | None,
    *,
    content_class: str | None = None,
    author_class: str | None = None,
    fallback_profile: str = "prototype",
) -> dict[str, Any]:
    normalized = normalize_policy(policy, fallback_profile=fallback_profile)
    effective_requirements = dict(normalized.get("defaults", {}))
    matched_overrides: list[dict[str, Any]] = []
    for index, override in enumerate(normalized.get("class_overrides", []), start=1):
        when = dict(override.get("when") or {})
        if when.get("content_class") and when["content_class"] != content_class:
            continue
        if when.get("author_class") and when["author_class"] != author_class:
            continue
        for key, value in (override.get("set") or {}).items():
            effective_requirements[key] = value
        matched_overrides.append(
            {
                "index": index,
                "when": when,
                "set": dict(override.get("set") or {}),
            }
        )
    return {
        "policy": normalized,
        "content_class": content_class,
        "author_class": author_class,
        "effective_requirements": effective_requirements,
        "matched_overrides": matched_overrides,
    }


def policy_to_yaml(policy: dict[str, Any] | None, *, fallback_profile: str = "prototype") -> str:
    normalized = normalize_policy(policy, fallback_profile=fallback_profile)
    lines = [
        f"version: {normalized['version']}",
        f"policy_id: {normalized['policy_id']}",
        "defaults:",
    ]
    for key in POLICY_REQUIREMENT_FLAGS:
        value = normalized["defaults"].get(key, False)
        lines.append(f"  {key}: {'true' if value else 'false'}")
    class_overrides = normalized.get("class_overrides") or []
    if class_overrides:
        lines.append("class_overrides:")
        for override in class_overrides:
            lines.append("  - when:")
            for key, value in (override.get("when") or {}).items():
                lines.append(f"      {key}: {value}")
            lines.append("    set:")
            for key in POLICY_REQUIREMENT_FLAGS:
                if key in (override.get("set") or {}):
                    value = override["set"][key]
                    lines.append(f"      {key}: {'true' if value else 'false'}")
    return "\n".join(lines) + "\n"


def parse_policy_yaml(text: str, *, fallback_profile: str = "prototype") -> dict[str, Any]:
    payload: dict[str, Any] = {}
    defaults: dict[str, Any] = {}
    class_overrides: list[dict[str, Any]] = []
    in_defaults = False
    in_class_overrides = False
    current_override: dict[str, Any] | None = None
    current_override_section: str | None = None
    for raw_line in text.splitlines():
        line = raw_line.split("#", 1)[0].rstrip()
        if not line.strip():
            continue
        stripped = line.strip()
        indent = len(line) - len(line.lstrip(" "))
        if indent == 0:
            if line.strip() == "defaults:":
                in_defaults = True
                in_class_overrides = False
                current_override = None
                current_override_section = None
                continue
            if line.strip() == "class_overrides:":
                in_defaults = False
                in_class_overrides = True
                current_override = None
                current_override_section = None
                continue
            in_defaults = False
            in_class_overrides = False
            current_override = None
            current_override_section = None
            key, value = _split_key_value(stripped)
            payload[key] = _parse_policy_scalar(value)
            continue
        if in_defaults:
            key, value = _split_key_value(stripped)
            defaults[key] = _parse_policy_scalar(value)
            continue
        if in_class_overrides:
            if indent == 2 and stripped == "- when:":
                current_override = {"when": {}, "set": {}}
                class_overrides.append(current_override)
                current_override_section = "when"
                continue
            if current_override is None:
                continue
            if indent == 4 and stripped in {"when:", "set:"}:
                current_override_section = stripped[:-1]
                continue
            if indent >= 4 and current_override_section in {"when", "set"}:
                key, value = _split_key_value(stripped)
                current_override[current_override_section][key] = _parse_policy_scalar(value)
    if defaults:
        payload["defaults"] = defaults
    if class_overrides:
        payload["class_overrides"] = class_overrides
    return normalize_policy(payload, fallback_profile=fallback_profile)


def _normalize_policy_class_overrides(raw_overrides: Any, defaults: dict[str, bool]) -> list[dict[str, Any]]:
    if not isinstance(raw_overrides, list):
        return []
    normalized: list[dict[str, Any]] = []
    for item in raw_overrides:
        if not isinstance(item, dict):
            continue
        when_raw = item.get("when")
        set_raw = item.get("set")
        if not isinstance(when_raw, dict) or not isinstance(set_raw, dict):
            continue
        when: dict[str, Any] = {}
        content_class = normalize_optional_text(when_raw.get("content_class"))
        if content_class in POLICY_CONTENT_CLASSES:
            when["content_class"] = content_class
        author_class = normalize_optional_text(when_raw.get("author_class"))
        if author_class in POLICY_AUTHOR_CLASSES:
            when["author_class"] = author_class
        if not when:
            continue
        set_values: dict[str, bool] = {}
        for key in POLICY_REQUIREMENT_FLAGS:
            if key in set_raw:
                set_values[key] = _coerce_bool(set_raw.get(key), defaults.get(key, False))
        if not set_values:
            continue
        normalized.append({"when": when, "set": set_values})
    return normalized


def _coerce_bool(value: Any, default: bool) -> bool:
    if isinstance(value, bool):
        return value
    if value is None:
        return default
    if isinstance(value, (int, float)):
        return bool(value)
    text = str(value).strip().lower()
    if text in {"true", "yes", "on", "1"}:
        return True
    if text in {"false", "no", "off", "0"}:
        return False
    return default


def _split_key_value(line: str) -> tuple[str, str]:
    if ":" not in line:
        raise ValueError(f"Invalid policy line: {line}")
    key, value = line.split(":", 1)
    return key.strip(), value.strip()


def _parse_policy_scalar(value: str) -> Any:
    if value == "":
        return ""
    lower = value.lower()
    if lower == "true":
        return True
    if lower == "false":
        return False
    if value.isdigit():
        try:
            return int(value)
        except Exception:
            return value
    if (value.startswith('"') and value.endswith('"')) or (value.startswith("'") and value.endswith("'")):
        return value[1:-1]
    return value


def encode_ref_name(name: str) -> str:
    return urllib.parse.quote(name, safe="")


def decode_ref_name(name: str) -> str:
    return urllib.parse.unquote(name)
