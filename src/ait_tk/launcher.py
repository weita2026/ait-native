"""Launcher for the local `aitk` viewer."""

from __future__ import annotations

import argparse
import importlib
import inspect
import json
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Any, Callable

Payload = dict[str, Any]
DEFAULT_LAZY_DIFF_MAX_BYTES = 128_000


def default_script_path() -> Path:
    """Return the expected Tcl script location relative to this package."""
    return Path(__file__).resolve().parent / "aitk.tcl"


def build_wish_command(payload_path: str | Path, script_path: str | Path | None = None, *, wish: str = "wish") -> list[str]:
    """Compose the command that opens aitk with a payload file."""
    script = Path(script_path) if script_path is not None else default_script_path()
    payload = Path(payload_path)
    return [wish, str(script), str(payload)]


def write_payload_temp(
    payload: Payload,
    *,
    path: str | Path | None = None,
    directory: str | Path | None = None,
    prefix: str = "aitk-",
    suffix: str = ".json",
) -> Path:
    """Write payload JSON to an on-disk file and return its path."""
    if path is not None:
        out = Path(path)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(payload, ensure_ascii=False, sort_keys=True, indent=2), encoding="utf-8")
        return out

    with NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        delete=False,
        dir=(Path(directory) if directory is not None else None),
        prefix=prefix,
        suffix=suffix,
    ) as handle:
        json.dump(payload, handle, ensure_ascii=False, sort_keys=True, indent=2)
        handle.write("\n")
        return Path(handle.name)


def _fallback_payload(reason: str) -> Payload:
    return {
        "schema_version": 1,
        "payload_type": "aitk-fallback",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "status": "fallback",
        "message": reason,
        "snapshots": [],
        "lines": [],
    }


def _ait_cli_path() -> str:
    sibling = Path(sys.executable).with_name("ait")
    if sibling.exists():
        return str(sibling)
    return shutil.which("ait") or "ait"


def _load_payload_builder(
    *,
    preload_diffs: bool = False,
    include_diff_text: bool = True,
    diff_max_bytes: int = DEFAULT_LAZY_DIFF_MAX_BYTES,
) -> Callable[[], Payload]:
    """Load payload builder lazily to avoid hard dependency on `ait.aitk_export`."""
    try:
        module = importlib.import_module("ait.aitk_export")
    except ModuleNotFoundError as exc:
        if exc.name != "ait.aitk_export":
            raise
        return lambda: _fallback_payload(
            "Optional module `ait.aitk_export` is not available; using fallback payload."
        )

    builder = getattr(module, "build_aitk_history_payload", None)
    if not callable(builder):
        return lambda: _fallback_payload("`build_aitk_history_payload` is not available in `ait.aitk_export`.")

    def _build_from_current_repo() -> Payload:
        from ait.repo_paths import RepoContext

        ctx = RepoContext.discover()
        try:
            parameters = inspect.signature(builder).parameters
        except (TypeError, ValueError):
            return builder(ctx)

        kwargs: dict[str, Any] = {}
        if "include_snapshot_diffs" in parameters:
            kwargs["include_snapshot_diffs"] = preload_diffs
        if "snapshot_diff_include_text" in parameters:
            kwargs["snapshot_diff_include_text"] = include_diff_text
        if "snapshot_diff_max_bytes" in parameters:
            kwargs["snapshot_diff_max_bytes"] = diff_max_bytes
        if "include_provenance" in parameters:
            kwargs["include_provenance"] = True
        payload = builder(ctx, **kwargs)
        if "include_snapshot_diffs" in parameters:
            payload["diff_loader"] = {
                "kind": "ait_snapshot_diff",
                "enabled": not preload_diffs,
                "preloaded": preload_diffs,
                "ait_cli_path": _ait_cli_path(),
                "include_text": include_diff_text,
                "max_bytes": diff_max_bytes,
            }
        return payload

    return _build_from_current_repo


def _build_payload(
    payload_builder: Callable[[], Payload] | None = None,
    *,
    preload_diffs: bool = False,
    include_diff_text: bool = True,
    diff_max_bytes: int = DEFAULT_LAZY_DIFF_MAX_BYTES,
) -> Payload:
    builder = payload_builder or _load_payload_builder(
        preload_diffs=preload_diffs,
        include_diff_text=include_diff_text,
        diff_max_bytes=diff_max_bytes,
    )
    try:
        return builder()
    except TypeError:
        return _fallback_payload("Payload builder signature is incompatible with launcher invocation.")
    except Exception as exc:  # pragma: no cover - hard-failure paths are surfaced to CLI
        return _fallback_payload(f"Payload builder failed: {exc}")

_Runner = Callable[[list[str]], int]


def _run_command(command: list[str]) -> int:
    proc = subprocess.run(command, check=False)
    return proc.returncode


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Launch aitk with snapshot history payload.")
    parser.add_argument("--script", type=Path, default=default_script_path(), help="Path to aitk.tcl")
    parser.add_argument("--wish", default="wish", help="wish executable")
    parser.add_argument("--json-only", dest="json_only", action="store_true", help="Only write payload JSON; do not launch UI")
    parser.add_argument("--no-open", dest="no_open", action="store_true", help="Write payload JSON and skip launching UI")
    parser.add_argument("--output", type=Path, default=None, help="Write payload to this path")
    parser.add_argument("--preload-diffs", action="store_true", help="Precompute all snapshot diffs before opening the UI")
    parser.add_argument("--diff-no-text", action="store_true", help="Do not include inline text diff bodies when loading diffs")
    parser.add_argument(
        "--diff-max-bytes",
        type=int,
        default=DEFAULT_LAZY_DIFF_MAX_BYTES,
        help="Maximum blob size to decode for inline text diffs",
    )
    return parser


def main(argv: list[str] | None = None, *, payload_builder: Callable[[], Payload] | None = None, run_command: _Runner | None = None) -> int:
    """
    Build an aitk payload and optionally launch wish.

    Return code is 0 for success, non-zero for launch/build errors.
    """
    if run_command is None:
        run_command = _run_command
    parser = _build_parser()
    args = parser.parse_args(argv)

    payload = _build_payload(
        payload_builder=payload_builder,
        preload_diffs=args.preload_diffs,
        include_diff_text=not args.diff_no_text,
        diff_max_bytes=args.diff_max_bytes,
    )
    payload_path = write_payload_temp(payload, path=args.output)

    if args.no_open or args.json_only:
        return 0

    if not args.script.exists():
        raise FileNotFoundError(f"aitk script not found at: {args.script}")

    try:
        command = build_wish_command(payload_path, script_path=args.script, wish=args.wish)
        return int(run_command(command))
    except FileNotFoundError:
        return 1
    except Exception:  # pragma: no cover - delegated safety
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
