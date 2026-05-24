from __future__ import annotations

from typing import Any

from ait_chat import generate_session_reply as _generate_session_reply
from ait_chat import load_reply_generation_config as _load_reply_generation_config


def load_reply_generation_config(*args: Any, **kwargs: Any) -> Any:
    return _load_reply_generation_config(*args, **kwargs)


def generate_session_reply(*args: Any, **kwargs: Any) -> Any:
    return _generate_session_reply(*args, **kwargs)
