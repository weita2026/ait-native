"""Python terminal parsing for the named, in-process Rust domain APIs."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from functools import partial
from pathlib import Path
import sys
from typing import Any, Sequence, TextIO

from . import __version__
from .errors import NativeBridgeError, NativeProtocolError, NativeResolutionError
from .runtime import NativeRuntime


class _StoreOnce(argparse.Action):
    """Preserve native errors for repeated singleton options."""

    def __call__(self, parser: argparse.ArgumentParser, namespace: argparse.Namespace,
                 values: Any, option_string: str | None = None) -> None:
        if self.option_strings:
            seen = getattr(namespace, "_ait_seen_options", set())
            if self.dest in seen:
                parser.error(f"argument {self.option_strings[0]} cannot be used multiple times")
            seen.add(self.dest)
            namespace._ait_seen_options = seen
        setattr(namespace, self.dest, self.const if self.nargs == 0 else values)


class _ArgumentParser(argparse.ArgumentParser):
    """Parse singleton options and selected intermixed path grammars."""

    def __init__(self, *args: Any, intermixed: bool = False, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self.register("action", None, _StoreOnce)
        self.register("action", "store", _StoreOnce)
        self.register("action", "store_true", partial(_StoreOnce, nargs=0, const=True, default=False))
        self.register("action", "store_false", partial(_StoreOnce, nargs=0, const=False, default=True))
        self._intermixed = intermixed
        self._parsing_intermixed = False
        self._help_commands: dict[str, _ArgumentParser] = {}

    def add_subparsers(self, **kwargs: Any) -> Any:
        commands = super().add_subparsers(**kwargs)
        self._help_commands = commands.choices
        help_command = commands.add_parser(
            "help", add_help=False,
            help="Print this message or the help of the given subcommand(s)",
            description="Print this message or the help of the given subcommand(s)",
        )
        help_command.add_argument("command", nargs="*", metavar="COMMAND",
                                  help="Print help for the subcommand(s)")
        return commands

    def _show_command_help(self, path: Sequence[str]) -> None:
        target = self
        for name in path:
            child = target._help_commands.get(name)
            if child is None:
                target.error(f"unrecognized subcommand {name!r}")
            target = child
        target.print_help()
        target.exit()

    def parse_known_args(self, args: Any = None, namespace: Any = None) -> Any:
        arguments = list(sys.argv[1:] if args is None else args)
        if self._help_commands and arguments and arguments[0] == "help":
            self._show_command_help(arguments[1:])
        if not self._intermixed or self._parsing_intermixed:
            return super().parse_known_args(arguments, namespace)
        self._parsing_intermixed = True
        try:
            return self.parse_known_intermixed_args(arguments, namespace)
        finally:
            self._parsing_intermixed = False


@dataclass(frozen=True)
class BindingCommand:
    export: str
    positional: tuple[str, ...] = ()
    options: tuple[tuple[str, str], ...] = ()
    payload: tuple[tuple[str, str], ...] = ()
    repository: bool = True
    projector: str | None = None
    formatter: str | None = None
    requested: str | None = None
    exit_projector: str | None = None
    error_projector: str | None = None
    json_conflicts: tuple[str, ...] = ()
    formatter_options: tuple[tuple[str, str], ...] = ()
    formatter_request: bool = False
    projector_options: tuple[tuple[str, str], ...] = ()
    projector_repository: bool = False
    projector_requested: bool = False
    phase_result: bool = False
    requires: tuple[tuple[str, str], ...] = ()
    error_formatter: str = "format_domain_error"
    error_exit: str = "domain_error_exit_code"
    omit_empty_text: bool = False
    constant_options: tuple[tuple[str, Any], ...] = ()
    projector_full: bool = True
    inspection_result: bool = False
    line_inventory_result: bool = False
    conflicts: tuple[tuple[str, str], ...] = ()
    human_progress: bool = False
    argument_validators: tuple[tuple[str, str], ...] = ()
    formatter_choices: tuple[tuple[str, str], ...] = ()
    formatter_verbatim: bool = False
    formatter_multiple: str | None = None
    exit_projector_options: tuple[tuple[str, str], ...] = ()
    requires_any: tuple[tuple[str, tuple[str, ...]], ...] = ()

    def domain_request(self, args: argparse.Namespace) -> dict[str, Any] | None:
        if not self.payload:
            return None
        return {native: getattr(args, public) for public, native in self.payload
                if getattr(args, public) is not None}

    def invoke(self, args: argparse.Namespace, runtime: NativeRuntime, repo_root: str | None = None,
               request: dict[str, Any] | None = None) -> Any:
        positional = [repo_root if repo_root is not None else str(Path.cwd())] if self.repository else []
        positional.extend(getattr(args, name) for name in self.positional)
        options = {
            native: getattr(args, public)
            for public, native in self.options
            if getattr(args, public) is not None
        }
        options.update(self.constant_options)
        if self.human_progress:
            options["human_progress"] = not args.json and sys.stdout.isatty()
        if self.inspection_result:
            options["include_delta"] = not args.json
        if self.payload:
            options["payload"] = request if request is not None else self.domain_request(args)
        return runtime.call(self.export, *positional, **options)


def _json_option(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--json", action="store_true", help="Emit JSON.")


def _scope(parser: argparse.ArgumentParser) -> None:
    scope = parser.add_mutually_exclusive_group()
    scope.add_argument("--local", action="store_true")
    scope.add_argument("--remote", dest="remote_name")


def _full_option(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--full", action="store_true", help="With --json, emit the complete result.")


SCOPE_OPTIONS = (("local", "local"), ("remote_name", "remote_name"))
AUTHOR_MODES = ("human_only", "human_with_ai_assist", "ai_with_human_review", "ai_only_experimental")


def _usize(text: str) -> int:
    digits = text[1:] if text.startswith("+") else text
    if not digits or not digits.isascii() or not digits.isdecimal():
        raise argparse.ArgumentTypeError("value must be an unsigned integer")
    value = int(text)
    if value > 2**64 - 1:
        raise argparse.ArgumentTypeError("value exceeds the native unsigned integer range")
    return value


def _i64(text: str) -> int:
    digits = text[1:] if text.startswith(("+", "-")) else text
    if not digits or not digits.isascii() or not digits.isdecimal():
        raise argparse.ArgumentTypeError("value must be a signed integer")
    value = int(text)
    if not -(2**63) <= value <= 2**63 - 1:
        raise argparse.ArgumentTypeError("value exceeds the native signed integer range")
    return value


def _u32(text: str) -> int:
    value = _usize(text)
    if value > 2**32 - 1:
        raise argparse.ArgumentTypeError("value exceeds the unsigned 32-bit integer range")
    return value


def _repository_jobs_limit(text: str) -> int:
    value = _u32(text)
    if not 1 <= value <= 1000:
        raise argparse.ArgumentTypeError("limit must be an integer from 1 through 1000")
    return value


def _remote_recovery_jobs(text: str) -> int:
    try:
        value = _usize(text)
    except argparse.ArgumentTypeError as error:
        raise argparse.ArgumentTypeError("jobs must be an integer from 1 through 64") from error
    if not 1 <= value <= 64:
        raise argparse.ArgumentTypeError("jobs must be from 1 through 64")
    return value


def _positive_usize(text: str) -> int:
    value = _usize(text)
    if value == 0:
        raise argparse.ArgumentTypeError("value must be greater than zero")
    return value


def _f64(text: str) -> float:
    if not text.isascii() or text != text.strip() or "_" in text:
        raise argparse.ArgumentTypeError("value must be a floating-point number")
    try:
        return float(text)
    except ValueError as error:
        raise argparse.ArgumentTypeError("value must be a floating-point number") from error


def _task_reference(text: str) -> str:
    value = text.strip()
    if not value or (value.isascii() and value.isdecimal()):
        raise argparse.ArgumentTypeError("Task ID must be non-empty and non-numeric")
    return value


def _patchset_reference(text: str) -> str:
    value = text.strip()
    if not value:
        raise argparse.ArgumentTypeError("Patchset ID must be non-empty")
    if value.isascii() and value.isdecimal():
        raise argparse.ArgumentTypeError("public Patchset reference required; use TASK_ID/P-##")
    return value


def _option_label(name: str) -> str:
    return {"abort_merge": "--abort", "continue_merge": "--continue",
            "start_line": "--start", "end_line": "--end", "snapshot_id": "--snapshot",
            "patchset_id": "--patchset", "remote_name": "--remote",
            "via_parent_snapshot_id": "--via-parent", "snapshot": "--to",
            "name": "NAME"}.get(name, "--" + name.replace("_", "-"))


def _provided(value: Any) -> bool:
    return value is not None and value is not False


def build_parser() -> argparse.ArgumentParser:
    parser = _ArgumentParser(prog="ait", allow_abbrev=False)
    parser.add_argument("-V", "--version", action="version", version=f"ait {__version__}")
    commands = parser.add_subparsers(dest="command", required=True, parser_class=_ArgumentParser)

    def command(parent: Any, name: str, binding: BindingCommand, **kwargs: Any) -> Any:
        json_option = kwargs.pop("json_option", True)
        item = parent.add_parser(name, allow_abbrev=False, **kwargs)
        item.set_defaults(binding=binding)
        if json_option:
            _json_option(item)
        else:
            item.set_defaults(json=False)
        return item

    init = command(commands, "init", BindingCommand(
        "initialize_repository", options=(("policy_profile", "policy_profile"),
                                       ("repair_existing", "repair_existing")), formatter="format_initialize_repository"))
    init.add_argument("--policy-profile", default="prototype")
    init.add_argument("--repair-existing", action="store_true")
    status = command(commands, "status", BindingCommand(
        "task_workflow_status", projector="project_status", formatter="format_status"))
    _full_option(status)

    for verb in ("pull", "push"):
        options = (("remote_name", "remote_name"), ("line_name", "line_name"))
        if verb == "pull":
            options += (("merge", "merge"), ("restore", "restore"), ("force", "force"))
        item = command(commands, verb, BindingCommand(
            verb + "_operation", options=options, formatter="format_" + verb,
            projector="project_remote_sync", projector_full=False,
            requires=(("merge", "restore"), ("force", "restore")) if verb == "pull" else (),
            conflicts=(("force", "merge"),) if verb == "pull" else ()))
        item.add_argument("--remote", dest="remote_name")
        item.add_argument("--line", dest="line_name")
        if verb == "pull":
            item.add_argument("--merge", action="store_true")
            item.add_argument("--restore", action="store_true")
            item.add_argument("--force", action="store_true")

    diff = command(commands, "diff", BindingCommand(
        "workspace_diff_operation", ("paths",), formatter="render_workspace_diff_text",
        formatter_choices=(("name_only", "render_workspace_diff_names"), ("stat", "render_workspace_diff_stat")),
        formatter_verbatim=True, json_conflicts=("name_only", "stat")), intermixed=True)
    diff.add_argument("paths", nargs="*", metavar="PATH")
    mode = diff.add_mutually_exclusive_group()
    mode.add_argument("--name-only", action="store_true")
    mode.add_argument("--stat", action="store_true")
    blame_options = ("line", "start_line", "end_line", "snapshot_id", "via_parent_snapshot_id",
                     "patchset_id", "remote_name", "plan_id", "plan_ref")
    blame = command(commands, "blame", BindingCommand(
        "task_workflow_blame", ("path",), tuple((field, field) for field in blame_options),
        formatter="format_blame", requires=(("start_line", "end_line"), ("end_line", "start_line"),
        ("via_parent_snapshot_id", "snapshot_id"), ("remote_name", "patchset_id")),
        conflicts=(("line", "end_line"),), argument_validators=(("patchset_id", "validate_public_patchset_argument"),)))
    blame.add_argument("path", metavar="PATH")
    selection = blame.add_mutually_exclusive_group()
    selection.add_argument("--line", type=_positive_usize)
    selection.add_argument("--start", dest="start_line", type=_positive_usize)
    blame.add_argument("--end", dest="end_line", type=_positive_usize)
    baseline = blame.add_mutually_exclusive_group()
    baseline.add_argument("--snapshot", dest="snapshot_id")
    baseline.add_argument("--patchset", dest="patchset_id")
    baseline.add_argument("--plan-id")
    baseline.add_argument("--plan-ref")
    blame.add_argument("--via-parent", dest="via_parent_snapshot_id")
    blame.add_argument("--remote", dest="remote_name")

    git = commands.add_parser("git", allow_abbrev=False).add_subparsers(required=True)
    migrate = command(git, "migrate", BindingCommand(
        "git_migrate_operation", ("staging_root",), (("apply", "apply"),), formatter="format_git_migrate"))
    migrate.add_argument("--staging-root", required=True)
    migrate.add_argument("--apply", action="store_true")
    recover = command(git, "recover", BindingCommand(
        "git_recover_operation", options=(("apply", "apply"),), formatter="format_git_recover"))
    recover.add_argument("--apply", action="store_true")
    importer = command(git, "import", BindingCommand(
        "git_import_operation", ("source",), (("all_branches_and_tags", "all_branches_and_tags"), ("dry_run", "dry_run")),
        formatter="format_git_import"))
    importer.add_argument("source", metavar="SOURCE")
    importer.add_argument("--all-branches-and-tags", action="store_true")
    importer.add_argument("--dry-run", action="store_true")
    exporter = command(git, "export", BindingCommand(
        "git_export_operation", ("target",), (("all_lines_and_tags", "all_lines_and_tags"), ("dry_run", "dry_run"),
        ("generation", "generation"), ("git_object", "git_object")), formatter="format_git_export",
        requires=(("git_object", "generation"),)))
    exporter.add_argument("target", metavar="TARGET")
    exporter.add_argument("--generation")
    exporter.add_argument("--git-object")
    exporter.add_argument("--all-lines-and-tags", action="store_true")
    exporter.add_argument("--dry-run", action="store_true")
    restore = command(git, "restore", BindingCommand(
        "git_restore_operation", ("generation", "git_object", "target"), formatter="format_git_restore"))
    restore.add_argument("--generation", required=True)
    restore.add_argument("--git-object", required=True)
    restore.add_argument("--target", required=True)
    mirror = command(git, "mirror", BindingCommand(
        "git_mirror_operation", ("endpoint", "direction"), (("dry_run", "dry_run"),), formatter="format_git_mirror"))
    mirror.add_argument("endpoint", metavar="ENDPOINT")
    mirror.add_argument("--direction", choices=("inbound", "outbound", "bidirectional"), required=True)
    mirror.add_argument("--dry-run", action="store_true")

    maintenance = commands.add_parser("gc", allow_abbrev=False).add_subparsers(required=True)
    for verb in ("stats", "validate", "prune"):
        item = command(maintenance, verb, BindingCommand(
            "gc_" + verb + "_operation", options=(("apply", "apply"),) if verb == "prune" else (),
            formatter="format_gc_" + verb, exit_projector="gc_validate_exit_code" if verb == "validate" else None))
        if verb == "prune":
            item.add_argument("--apply", action="store_true")

    binary_db = commands.add_parser("binary-db", allow_abbrev=False).add_subparsers(required=True)
    for verb, operation, arguments in (
        ("upgrade-u64-seconds", "upgrade_u64_seconds", ("repo_root", "generation_root", "source_time_width")),
        ("agent-pool50", "agent_pool_50", ("repo_root", "generation_root")),
    ):
        item = command(binary_db, verb, BindingCommand(
            "binary_db_" + operation + "_operation", arguments, (("activate", "activate"),), repository=False,
            formatter="format_binary_db_" + operation))
        item.add_argument("--repo-root", default=".")
        item.add_argument("--generation-root", required=True)
        if verb == "upgrade-u64-seconds":
            item.add_argument("--source-time-width", required=True)
        item.add_argument("--activate", action="store_true")

    source_cache = commands.add_parser("current-source-cache", allow_abbrev=False).add_subparsers(required=True)
    cache_keywords = tuple((field, field) for field in
        ("ext_suffix", "rustflags", "worker_id", "core_source_fingerprint", "server_source_fingerprint"))

    def cache_path_options(item: argparse.ArgumentParser) -> None:
        item.add_argument("--namespace-root", required=True)
        item.add_argument("--core-repo-root", required=True)
        item.add_argument("--ext-suffix", required=True)
        item.add_argument("--rustflags", default="")
        item.add_argument("--worker-id", default="shared")
        item.add_argument("--core-source-fingerprint")
        item.add_argument("--server-source-fingerprint")

    contract = command(source_cache, "contract", BindingCommand(
        "current_source_native_cache_contract", ("namespace_root", "core_repo_root"), cache_keywords,
        repository=False, formatter="format_current_source_cache_contract"))
    cache_path_options(contract)
    for verb in ("mark-building", "activate"):
        operation = verb.replace("-", "_")
        options = cache_keywords + (("extra_json", "extra_json"),)
        if verb == "activate":
            options += (("register_lease", "register_lease"), ("owner_pid", "owner_pid"))
        item = command(source_cache, verb, BindingCommand(
            "current_source_cache_" + operation + "_operation", ("namespace_root", "core_repo_root", "source_mtime_ns"),
            options, repository=False, formatter="format_current_source_cache_" + operation,
            requires=(("owner_pid", "register_lease"),) if verb == "activate" else ()))
        cache_path_options(item)
        item.add_argument("--source-mtime-ns", type=_usize, required=True)
        item.add_argument("--extra-json", default="{}")
        if verb == "activate":
            item.add_argument("--register-lease", action="store_true")
            item.add_argument("--owner-pid", type=_u32)
    release = command(source_cache, "release-lease", BindingCommand(
        "current_source_cache_release_lease_operation", ("lease_path", "namespace_root"),
        (("remove_unleased_ready", "remove_unleased_ready"),), repository=False,
        formatter="format_current_source_cache_release_lease"))
    release.add_argument("--lease-path", required=True)
    release.add_argument("--namespace-root", required=True)
    release.add_argument("--remove-unleased-ready", action="store_true")
    prune = command(source_cache, "prune", BindingCommand(
        "current_source_cache_prune_operation", ("namespace_root",), tuple((field, field) for field in
        ("now", "idle_ttl_seconds", "build_stale_seconds", "max_bytes", "remove_unleased_ready")),
        repository=False, formatter="format_current_source_cache_prune"))
    prune.add_argument("--namespace-root", required=True)
    prune.add_argument("--now", type=_f64)
    for field in ("idle_ttl_seconds", "build_stale_seconds", "max_bytes"):
        prune.add_argument("--" + field.replace("_", "-"), type=_usize)
    prune.add_argument("--remove-unleased-ready", action="store_true")
    seed = command(source_cache, "seed-canonical", BindingCommand(
        "current_source_cache_seed_canonical_operation", ("namespace_root", "core_repo_root", "repo_root",
        "canonical_repo_root", "core_source_mtime_ns"), cache_keywords, repository=False,
        formatter="format_current_source_cache_seed_canonical"))
    cache_path_options(seed)
    seed.add_argument("--repo-root", required=True)
    seed.add_argument("--canonical-repo-root", required=True)
    seed.add_argument("--core-source-mtime-ns", type=_usize, required=True)
    for verb, fields in (
        ("extension-fresh", ("metadata_path", "extension_path", "source_mtime_ns", "source_fingerprint")),
        ("binary-fresh", ("metadata_path", "binary_path", "metadata_fingerprint_key", "metadata_source_mtime_key",
                          "metadata_mtime_key", "metadata_sha_key", "source_mtime_ns", "source_fingerprint")),
    ):
        operation = verb.replace("-", "_")
        item = command(source_cache, verb, BindingCommand(
            "current_source_cache_" + operation + "_operation", fields, repository=False,
            formatter="format_current_source_cache_" + operation))
        for field in fields:
            item.add_argument("--" + field.replace("_", "-"), type=_usize if field == "source_mtime_ns" else str, required=True)
    for verb in ("core-fingerprint", "server-fingerprint", "core-mtime", "server-mtime"):
        operation = verb.replace("-", "_")
        item = command(source_cache, verb, BindingCommand(
            "current_source_cache_" + operation + "_operation", ("repo_root",), repository=False,
            formatter="format_current_source_cache_" + operation))
        item.add_argument("repo_root")

    releases = commands.add_parser("release", allow_abbrev=False).add_subparsers(required=True)
    adapters = releases.add_parser("adapter", allow_abbrev=False).add_subparsers(required=True)
    for verb in ("check", "build"):
        item = command(adapters, verb, BindingCommand(
            "release_adapter_" + verb + "_operation", ("version",), (("line_name", "line_name"), ("target", "target")),
            formatter="format_release"))
        item.add_argument("--version", required=True)
        item.add_argument("--line", dest="line_name", default="main")
        item.add_argument("--target")
    candidates = releases.add_parser("candidate", allow_abbrev=False).add_subparsers(required=True)
    candidate = command(candidates, "create", BindingCommand(
        "release_candidate_create_operation", ("version",), tuple((field, field) for field in
        ("line_name", "profile", "channel", "public_source_root")), formatter="format_release"))
    candidate.add_argument("--version", required=True)
    candidate.add_argument("--line", dest="line_name", default="main")
    candidate.add_argument("--profile")
    candidate.add_argument("--channel", choices=("rc", "stable"))
    candidate.add_argument("--public-source-root", help=argparse.SUPPRESS)
    for verb in ("check", "build"):
        fields = ("receipts", "public_source_root")
        fields += ("tests_command", "skip_tests_reason") if verb == "check" else ("native_matrix_dir", "native_command_dir")
        item = command(releases, verb, BindingCommand(
            "release_" + verb + "_operation", ("release_id",), tuple((field, field) for field in fields), formatter="format_release"))
        item.add_argument("release_id", metavar="RELEASE_ID")
        for field in fields:
            options = {"help": argparse.SUPPRESS} if field == "public_source_root" else {}
            item.add_argument("--" + field.replace("_", "-"), **options)
    source_fields = ("target", "source_dir", "runner", "runner_image", "rust_toolchain", "rustc_path")
    native_source = command(releases, "native-source", BindingCommand(
        "release_native_source_operation", ("release_id",), tuple((field, field) for field in source_fields),
        formatter="format_release"))
    native_source.add_argument("release_id", metavar="RELEASE_ID")
    for field in source_fields:
        native_source.add_argument("--" + field.replace("_", "-"), required=True)
    bundle = command(releases, "native-bundle", BindingCommand(
        "release_native_bundle_operation", ("release_id", "native_matrix_dir"), formatter="format_release"))
    bundle.add_argument("release_id", metavar="RELEASE_ID")
    bundle.add_argument("--native-matrix-dir", required=True)
    for verb in ("package", "promote"):
        item = command(releases, verb, BindingCommand(
            "release_" + verb + "_operation", ("release_id", "channel"), (("public_source_root", "public_source_root"),),
            formatter="format_release"))
        item.add_argument("release_id", metavar="RELEASE_ID")
        item.add_argument("--channel", choices=("homebrew", "apt", "winget", "pypi", "npm") if verb == "package" else ("rc", "stable"),
                          required=True)
        item.add_argument("--public-source-root", help=argparse.SUPPRESS)
    formula = command(releases, "formula", BindingCommand(
        "release_formula_operation", ("release_id", "name"), (("python_formula", "python_formula"),), formatter="format_release"))
    formula.add_argument("release_id", metavar="RELEASE_ID")
    formula.add_argument("--name", required=True)
    formula.add_argument("--python-formula")
    for verb in ("show", "publish"):
        options = (("remote_name", "remote_name"),)
        if verb == "show":
            options += (("public_source_root", "public_source_root"),)
        item = command(releases, verb, BindingCommand(
            "release_" + verb + "_operation", ("release_id",), options, formatter="format_release"))
        item.add_argument("release_id", metavar="RELEASE_ID")
        item.add_argument("--remote", dest="remote_name")
        if verb == "show":
            item.add_argument("--public-source-root", help=argparse.SUPPRESS)

    config = commands.add_parser("config", allow_abbrev=False).add_subparsers(required=True)
    command(config, "show", BindingCommand("task_workflow_config_show", formatter="format_configuration_show"))
    config_fields = (
        "agent_teams", "plan_language", "plan_style", "sprint", "default_author_mode",
        "default_model", "task_review", "task_worktree_alias_root",
        "task_worktree_main_seed_ram_max_bytes", "id_namespace_prefix", "user_name",
        "user_email", "post_finish_hooks",
    )
    setter = command(config, "set", BindingCommand(
        "configure_repository", payload=tuple((field, field) for field in config_fields),
        error_projector="configuration_error", formatter="format_configuration", formatter_request=True))
    config_choices = {"agent_teams": ("enabled", "disabled"), "plan_style": ("concise", "detailed"),
                      "sprint": ("on", "off"), "default_author_mode": AUTHOR_MODES,
                      "task_review": ("required", "automatic")}
    for field in config_fields:
        options = {"type": _i64 if field.endswith("_bytes") else str}
        if field in config_choices:
            options["choices"] = config_choices[field]
        setter.add_argument("--" + field.replace("_", "-"), **options)
    unset = command(config, "unset", BindingCommand(
        "unset_configuration", ("key",), formatter="format_configuration_unset", requested="key"))
    unset.add_argument("key", choices=("plan-language", "plan-style", "default-author-mode", "default-model",
        "task-review", "task-worktree-alias-root", "task-worktree-main-seed-ram-max-bytes",
        "id-namespace-prefix", "user-name", "user-email", "post-finish-hooks"))

    remote = commands.add_parser("remote", allow_abbrev=False).add_subparsers(required=True)
    add = command(remote, "add", BindingCommand("task_workflow_remote_add", payload=(
        ("name", "name"), ("url", "url"), ("make_default", "make_default")), formatter="format_remote_add"))
    add.add_argument("name")
    add.add_argument("url")
    add.add_argument("--default", dest="make_default", action="store_true")
    command(remote, "list", BindingCommand("task_workflow_remote_list", formatter="format_remote_list"))
    recover_head = command(remote, "recover-head", BindingCommand(
        "remote_recover_head_operation", options=(("remote_name", "remote_name"), ("jobs", "jobs"), ("apply", "apply")),
        formatter="format_remote_recover_head"))
    recover_head.add_argument("--remote", dest="remote_name")
    recover_head.add_argument("--jobs", type=_remote_recovery_jobs, default=8)
    recover_head.add_argument("--apply", action="store_true")

    externals = commands.add_parser("external", allow_abbrev=False).add_subparsers(required=True)
    command(externals, "status", BindingCommand("external_status_operation", formatter="format_external_status"))
    doctor = command(externals, "doctor", BindingCommand(
        "external_doctor_operation", options=(("fail_on_blocking", "fail_on_blocking"),),
        formatter="format_external_doctor", phase_result=True))
    doctor.add_argument("--fail-on-blocking", action="store_true")
    update = command(externals, "update", BindingCommand(
        "external_update_operation", options=tuple((field, field) for field in
        ("name", "snapshot", "latest", "locked", "validate", "no_recursive")), formatter="format_external_update",
        requires=(("snapshot", "name"), ("latest", "name")), requires_any=(("name", ("snapshot", "latest")),),
        conflicts=(("locked", "name"), ("locked", "snapshot"), ("locked", "latest"))))
    update.add_argument("name", nargs="?")
    selection = update.add_mutually_exclusive_group()
    selection.add_argument("--to", dest="snapshot")
    selection.add_argument("--latest", action="store_true")
    update.add_argument("--locked", action="store_true")
    update.add_argument("--validate", action="store_true")
    update.add_argument("--no-recursive", action="store_true")
    link = command(externals, "link", BindingCommand(
        "external_link_operation", ("name", "path"), formatter="format_external_link"))
    link.add_argument("name")
    link.add_argument("path")
    unlink = command(externals, "unlink", BindingCommand(
        "external_unlink_operation", ("name",), formatter="format_external_unlink"))
    unlink.add_argument("name")

    teams = commands.add_parser("team", allow_abbrev=False).add_subparsers(required=True)
    command(teams, "contacts", BindingCommand(
        "team_contacts_operation", formatter="format_team_contacts", phase_result=True))
    sessions = teams.add_parser("session", allow_abbrev=False).add_subparsers(required=True)
    ensure = command(sessions, "ensure", BindingCommand(
        "team_session_ensure_operation", ("agent",), (("role", "role"),),
        formatter="format_team_session_ensure", requested="agent", phase_result=True))
    ensure.add_argument("--agent", type=_u32, required=True)
    ensure.add_argument("--role", type=_u32)
    acknowledge = command(sessions, "acknowledge", BindingCommand(
        "team_session_acknowledge_operation", ("activation",), formatter="format_team_session_acknowledge", phase_result=True))
    acknowledge.add_argument("--activation", required=True)

    repositories = commands.add_parser("repo", allow_abbrev=False).add_subparsers(required=True)
    remote_options = (("remote_name", "remote_name"),)
    for verb in ("show", "ci-capabilities", "retire", "restore"):
        operation = verb.replace("-", "_")
        options = remote_options + ((("abort", "abort"),) if verb == "retire" else ())
        query = command(repositories, verb, BindingCommand(
            "repository_" + operation + "_operation", options=options,
            formatter="format_repository_" + operation,
            formatter_options=remote_options if verb in ("show", "ci-capabilities") else ()))
        query.add_argument("--remote", dest="remote_name")
        if verb == "retire":
            query.add_argument("--abort", action="store_true")
    job_options = remote_options + (("worker_job_index", "worker_job_index"),
                                   ("state", "state"), ("limit", "limit"))
    jobs = command(repositories, "jobs", BindingCommand(
        "repository_jobs_operation", options=job_options, formatter="format_repository_jobs",
        formatter_options=job_options,
        conflicts=(("worker_job_index", "state"), ("worker_job_index", "limit"))))
    jobs.add_argument("--remote", dest="remote_name")
    jobs.add_argument("--worker-job-index", type=_u32)
    jobs.add_argument("--state", choices=("queued", "running", "succeeded", "failed"))
    jobs.add_argument("--limit", type=_repository_jobs_limit)

    auth = commands.add_parser("auth", allow_abbrev=False).add_subparsers(required=True)
    for verb in ("whoami", "bindings", "grant"):
        options = (("remote_name", "remote_name"), ("repo_name", "repo_name"))
        if verb == "grant":
            options += (("actor_identity", "actor_identity"), ("roles", "roles"))
        item = command(auth, verb, BindingCommand(
            "task_workflow_auth_" + verb, options=options, formatter="format_auth_" + verb))
        item.add_argument("--remote", dest="remote_name")
        item.add_argument("--repo", dest="repo_name")
        if verb == "grant":
            item.add_argument("--actor", dest="actor_identity", required=True)
            item.add_argument("--role", dest="roles", action="append", default=[])

    doctor = commands.add_parser("doctor", allow_abbrev=False).add_subparsers(required=True)
    command(doctor, "memory-root", BindingCommand("doctor_memory_root_operation", formatter="format_doctor_memory_root"))
    command(doctor, "runtime-root", BindingCommand("task_workflow_doctor_runtime_root", formatter="format_doctor_runtime_root"))
    command(doctor, "plan-authority", BindingCommand(
        "doctor_plan_authority_operation", formatter="format_doctor_plan_authority"))
    queue = commands.add_parser("queue", allow_abbrev=False).add_subparsers(required=True)
    summary = command(queue, "summary", BindingCommand(
        "task_workflow_queue_summary", options=(("remote_name", "remote_name"),), formatter="format_queue_summary"))
    summary.add_argument("--remote", dest="remote_name")

    plans = commands.add_parser("plan", allow_abbrev=False).add_subparsers(required=True)
    listing = command(plans, "list", BindingCommand(
        "plan_list_operation", options=SCOPE_OPTIONS, json_conflicts=("all",),
        formatter="format_plan_list", formatter_options=SCOPE_OPTIONS + (("all", "all"),)))
    _scope(listing)
    listing.add_argument("--all", action="store_true")
    revisions = command(plans, "revisions", BindingCommand(
        "plan_revisions_operation", ("plan_id",), SCOPE_OPTIONS, json_conflicts=("all",),
        formatter="format_plan_revisions", requested="plan_id",
        formatter_options=SCOPE_OPTIONS + (("all", "all"),)))
    revisions.add_argument("plan_id")
    revisions.add_argument("--all", action="store_true")
    _scope(revisions)
    for verb in ("show", "items", "inspect"):
        query = command(plans, verb, BindingCommand(
            "plan_" + verb + "_operation", ("plan_id",), SCOPE_OPTIONS + (("revision", "revision"),),
            formatter="format_plan_" + verb))
        query.add_argument("plan_id")
        query.add_argument("--revision")
        _scope(query)
    candidates = command(plans, "candidates", BindingCommand(
        "plan_candidates_operation", options=SCOPE_OPTIONS +
        (("include_all", "include_all"), ("contains", "contains")), formatter="format_plan_candidates"))
    candidates.add_argument("--all", dest="include_all", action="store_true")
    candidates.add_argument("--contains")
    _scope(candidates)
    sync = command(plans, "sync", BindingCommand(
        "plan_sync_operation", ("target",), SCOPE_OPTIONS +
        (("plan_ref", "plan_ref"), ("prune", "prune"), ("reconcile", "reconcile")),
        error_projector="plan_sync_error", formatter="format_plan_sync"))
    sync.add_argument("target")
    sync.add_argument("--plan-ref")
    sync.add_argument("--prune", action="store_true")
    sync.add_argument("--reconcile", action="store_true")
    _scope(sync)

    workflows = commands.add_parser("workflow", allow_abbrev=False).add_subparsers(required=True)
    for verb in ("ready", "finish"):
        operation_options = (("apply", "apply"), ("remote_name", "remote_name"),
                             ("expected_identity", "expected_identity_path"))
        apply_fields = (("snapshot_message", "summary", "tests", "lint", "security", "license", "author_mode", "model", "run_ci_once")
                        if verb == "ready" else ("reviewer", "review_message"))
        binding = BindingCommand(
            "workflow_" + verb + "_operation", ("task_id",), operation_options + tuple((field, field) for field in apply_fields),
            projector="project_workflow_" + verb, formatter="format_workflow_" + verb,
            requested="task_id", projector_options=(("remote_name", "remote_name"),),
            projector_repository=True, projector_requested=True, phase_result=True,
            requires=tuple((field, "apply") for field in apply_fields))
        item = command(workflows, verb, binding)
        item.add_argument("task_id")
        item.add_argument("--apply", action="store_true")
        item.add_argument("--remote", dest="remote_name")
        item.add_argument("--expected-identity")
        _full_option(item)
        for field in apply_fields:
            options = {"action": "store_true"} if field == "run_ci_once" else {}
            if field == "author_mode":
                options["choices"] = AUTHOR_MODES
            item.add_argument("--" + field.replace("_", "-"), **options)
    reconcile = command(workflows, "reconcile", BindingCommand(
        "workflow_reconcile_operation", options=(("remote_name", "remote_name"), ("task_id", "task_id"),
        ("apply", "apply"), ("safe_only", "safe_only"), ("scheduled", "scheduled"), ("limit", "limit")),
        formatter="format_workflow_reconcile", requires=(("scheduled", "apply"),)))
    reconcile.add_argument("--remote", dest="remote_name")
    reconcile.add_argument("--task", dest="task_id")
    mode = reconcile.add_mutually_exclusive_group()
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--dry-run", action="store_true")
    reconcile.add_argument("--safe-only", action="store_true")
    reconcile.add_argument("--scheduled", action="store_true")
    reconcile.add_argument("--limit", type=_usize, default=100)
    guide = command(workflows, "guide", BindingCommand(
        "workflow_guide", ("topic",), repository=False, formatter="format_workflow_guide"), json_option=False)
    guide.add_argument("topic", nargs="?")

    line = commands.add_parser("line", aliases=["branch"], allow_abbrev=False)
    lines = line.add_subparsers(required=True)
    listing = command(lines, "list", BindingCommand("line_inventory_operation", options=(
        ("all", "include_all"), ("archived", "archived"), ("remote_name", "remote_name")),
        line_inventory_result=True, formatter="format_line_list", formatter_options=(
        ("all", "include_all"), ("archived", "archived"), ("remote_name", "remote_name"))))
    line_scope = listing.add_mutually_exclusive_group()
    line_scope.add_argument("--all", action="store_true")
    line_scope.add_argument("--archived", action="store_true")
    listing.add_argument("--remote", dest="remote_name")
    create = command(lines, "create", BindingCommand("task_workflow_line_create", ("name",),
        (("from_snapshot", "from_snapshot"), ("switch", "switch")), formatter="format_line_create"))
    create.add_argument("name")
    create.add_argument("--from-snapshot")
    create.add_argument("--switch", action="store_true")
    show = command(lines, "show", BindingCommand("line_show_operation", options=(
        ("name", "name"), ("remote_name", "remote_name")), formatter="format_line_show"))
    show.add_argument("name", nargs="?")
    show.add_argument("--remote", dest="remote_name")
    switch = command(lines, "switch", BindingCommand("task_workflow_line_switch", ("name",),
        (("restore", "restore"), ("force", "force")), formatter="format_line_switch", requires=(("force", "restore"),)))
    switch.add_argument("name")
    switch.add_argument("--restore", action="store_true")
    switch.add_argument("--force", action="store_true")
    for verb, positions in (("archive", ("name",)), ("rename", ("old_name", "new_name")),
                            ("delete", ("name",))):
        options = (("remote_name", "remote_name"),)
        if verb == "delete":
            options += (("yes", "confirm"),)
        item = command(lines, verb, BindingCommand("task_workflow_line_" + verb, positions, options,
                                                  formatter="format_line_" + verb))
        for name in positions:
            item.add_argument(name)
        item.add_argument("--remote", dest="remote_name")
        if verb == "delete":
            item.add_argument("--yes", action="store_true", required=True)
    merge = command(lines, "merge", BindingCommand("line_merge_operation", options=(
        ("source", "source"), ("message", "message"), ("continue_merge", "continue_merge"),
        ("abort_merge", "abort_merge")), formatter="format_line_merge", conflicts=(("abort_merge", "message"),)))
    selection = merge.add_mutually_exclusive_group()
    selection.add_argument("source", nargs="?")
    selection.add_argument("--continue", dest="continue_merge", action="store_true")
    selection.add_argument("--abort", dest="abort_merge", action="store_true")
    merge.add_argument("--message")
    cleanup = command(lines, "cleanup", BindingCommand("task_workflow_line_cleanup", options=(
        ("idle_for", "idle_for"), ("cleanup_kind", "cleanup_kind"), ("limit", "limit"),
        ("include_protected", "include_protected"), ("yes", "apply")),
        formatter="format_line_cleanup", requested="idle_for", formatter_options=(
        ("all", "all"), ("include_protected", "include_protected"), ("cleanup_kind", "cleanup_kind"), ("limit", "limit"))))
    cleanup.add_argument("--idle-for", default="7d")
    cleanup.add_argument("--kind", dest="cleanup_kind")
    cleanup.add_argument("--limit", type=_usize)
    cleanup.add_argument("--include-protected", action="store_true")
    cleanup.add_argument("--all", action="store_true")
    cleanup.add_argument("--yes", action="store_true")

    tasks = commands.add_parser("task", allow_abbrev=False).add_subparsers(required=True)
    start_options = SCOPE_OPTIONS + (("title", "title"), ("edit_root", "edit_root"))
    start = command(tasks, "start", BindingCommand(
        "task_start_operation", ("intent",), start_options + (("source", "source"),),
        projector="project_task_start", formatter="format_task_start", human_progress=True))
    source = start.add_mutually_exclusive_group(required=True)
    source.add_argument("--title")
    source.add_argument("--from", dest="source")
    start.add_argument("--intent", required=True)
    start.add_argument("--edit-root")
    _scope(start)
    _full_option(start)
    quick = command(tasks, "quick", BindingCommand(
        "task_quick_operation", ("intent",), start_options,
        projector="project_task_start", formatter="format_task_quick", human_progress=True))
    quick.add_argument("--intent", required=True)
    quick.add_argument("--title")
    quick.add_argument("--edit-root")
    _scope(quick)
    _full_option(quick)
    finish = command(tasks, "finish", BindingCommand(
        "task_finish_operation", ("task_id",), SCOPE_OPTIONS +
        (("message", "message"), ("sync_plan_markdown", "sync_plan_markdown")),
        projector="project_task_finish", formatter="format_task_finish",
        requested="task_id", exit_projector="task_finish_exit_code"))
    finish.add_argument("task_id")
    finish.add_argument("--message")
    finish.add_argument("--sync-plan-markdown", action="store_true")
    _scope(finish)
    _full_option(finish)
    listing = command(tasks, "list", BindingCommand(
        "task_workflow_task_list", options=SCOPE_OPTIONS, constant_options=(("all", True),),
        projector="project_task_list", projector_full=False, projector_options=(("all", "all"),),
        formatter="format_task_list", formatter_options=SCOPE_OPTIONS + (("all", "all"),)))
    _scope(listing)
    listing.add_argument("--all", action="store_true")
    for verb in ("show", "audit", "abandon"):
        operation = "task_workflow_task_show" if verb == "show" else "task_" + verb + "_operation"
        item = command(tasks, verb, BindingCommand(
            operation, ("task_id",), SCOPE_OPTIONS,
            formatter="format_task_" + verb,
            requested=None if verb == "show" else "task_id", phase_result=verb == "abandon"))
        item.add_argument("task_id")
        _scope(item)

    worktrees = commands.add_parser("worktree", allow_abbrev=False).add_subparsers(required=True)
    status = command(worktrees, "status", BindingCommand(
        "task_workflow_worktree_status", options=(("name", "name"), ("snapshot_id", "snapshot_id"), ("line_name", "line_name")),
        formatter="format_worktree_status", formatter_options=(("verbose", "verbose"),)))
    status.add_argument("name", nargs="?")
    status.add_argument("--snapshot", dest="snapshot_id")
    status.add_argument("--line", dest="line_name")
    status.add_argument("--verbose", action="store_true")
    show = command(worktrees, "show", BindingCommand(
        "worktree_show_operation", options=(("name", "name"),), formatter="format_worktree_show"))
    show.add_argument("name", nargs="?")
    path = command(worktrees, "path", BindingCommand(
        "worktree_path_operation", options=(("name", "name"),), formatter="format_worktree_path",
        formatter_options=(("shell_output", "shell_output"),), projector="project_worktree_path", projector_full=False),
        aliases=["open"])
    path.add_argument("name", nargs="?")
    path.add_argument("--shell", dest="shell_output", action="store_true")
    for verb in ("list", "doctor"):
        item = command(worktrees, verb, BindingCommand(
            "task_workflow_worktree_" + verb, options=(("refresh_status", "refresh_status"),),
            formatter="format_worktree_" + verb))
        item.add_argument("--refresh", dest="refresh_status", action="store_true")
    restore = command(worktrees, "restore", BindingCommand(
        "worktree_restore_operation", options=tuple((field, field) for field in
        ("name", "snapshot_id", "line_name", "paths", "force", "dry_run")), formatter="format_worktree_restore"))
    restore.add_argument("name", nargs="?")
    restore.add_argument("--snapshot", dest="snapshot_id")
    restore.add_argument("--line", dest="line_name")
    restore.add_argument("--path", dest="paths", action="append", default=[])
    restore.add_argument("--force", action="store_true")
    restore.add_argument("--dry-run", action="store_true")
    for verb in ("cleanup-candidates", "cleanup"):
        operation = verb.replace("-", "_")
        fields = ("older_than", "cleanup_policy", "allow_manual_only")
        fields += ("include_protected",) if verb == "cleanup-candidates" else ("limit", "dry_run", "yes")
        item = command(worktrees, verb, BindingCommand(
            "task_workflow_worktree_cleanup_candidates" if verb == "cleanup-candidates" else "worktree_cleanup_operation",
            options=tuple((field, field) for field in fields), formatter="format_worktree_" + operation))
        item.add_argument("--older-than", default="7d")
        item.add_argument("--policy", dest="cleanup_policy")
        item.add_argument("--allow-manual-only", action="store_true")
        if verb == "cleanup-candidates":
            item.add_argument("--include-protected", action="store_true")
        else:
            item.add_argument("--limit", type=_usize)
            item.add_argument("--dry-run", action="store_true")
            item.add_argument("--yes", action="store_true")
    prune = command(worktrees, "prune-stale", BindingCommand(
        "worktree_prune_stale_operation", options=(("dry_run", "dry_run"), ("yes", "yes")),
        formatter="format_worktree_prune_stale"))
    prune.add_argument("--dry-run", action="store_true")
    prune.add_argument("--yes", action="store_true")
    sync = command(worktrees, "sync", BindingCommand(
        "worktree_sync_operation", options=tuple((field, field) for field in
        ("name", "line_name", "all_worktrees", "force", "dry_run")), formatter="format_worktree_sync",
        exit_projector="worktree_sync_exit_code", exit_projector_options=(("all_worktrees", "all_worktrees"),)))
    sync.add_argument("name", nargs="?")
    sync.add_argument("--line", dest="line_name")
    sync.add_argument("--all", dest="all_worktrees", action="store_true")
    sync.add_argument("--force", action="store_true")
    sync.add_argument("--dry-run", action="store_true")
    for verb in ("recreate", "restore-owned-head"):
        operation = verb.replace("-", "_")
        item = command(worktrees, verb, BindingCommand(
            "worktree_" + operation + "_operation", options=(("name", "name"), ("dry_run", "dry_run")),
            formatter="format_worktree_" + operation))
        item.add_argument("name", nargs="?")
        item.add_argument("--dry-run", action="store_true")
    recover = command(worktrees, "recover-task", BindingCommand(
        "worktree_recover_task_operation", ("task_id",),
        (("change_ref", "change_ref"), ("remote_name", "remote_name"), ("dry_run", "dry_run")),
        formatter="format_worktree_recover_task"))
    recover.add_argument("task_id", metavar="TASK_ID")
    recover.add_argument("--change", dest="change_ref", help=argparse.SUPPRESS)
    recover.add_argument("--remote", dest="remote_name")
    recover.add_argument("--dry-run", action="store_true")
    rebase = command(worktrees, "rebase", BindingCommand(
        "worktree_rebase_operation", options=tuple((field, field) for field in
        ("name", "onto_line", "continue_rebase", "abort_rebase", "dry_run")), formatter="format_worktree_rebase"))
    rebase.add_argument("name", nargs="?")
    rebase.add_argument("--onto", dest="onto_line")
    rebase.add_argument("--continue", dest="continue_rebase", action="store_true")
    rebase.add_argument("--abort", dest="abort_rebase", action="store_true")
    rebase.add_argument("--dry-run", action="store_true")
    remove = command(worktrees, "remove", BindingCommand(
        "worktree_remove_operation", ("names",), options=tuple((field, field) for field in
        ("all_stale", "delete_path", "force", "dry_run", "yes")), formatter="format_worktree_remove",
        formatter_options=(("all_stale", "all_stale"), ("dry_run", "dry_run")), formatter_multiple="names"), intermixed=True)
    remove.add_argument("names", nargs="*", metavar="NAME")
    remove.add_argument("--all-stale", action="store_true")
    remove.add_argument("--delete-path", action="store_true")
    remove.add_argument("--force", action="store_true")
    remove.add_argument("--dry-run", action="store_true")
    remove.add_argument("--yes", action="store_true")

    changes = commands.add_parser("change", allow_abbrev=False).add_subparsers(required=True)
    create = command(changes, "create", BindingCommand(
        "change_create_operation", ("task_id", "title"),
        SCOPE_OPTIONS + (("base_line", "base_line"), ("fork_snapshot", "fork_snapshot")),
        formatter="format_change_create"))
    create.add_argument("task_id")
    create.add_argument("--title", required=True)
    create.add_argument("--base-line")
    create.add_argument("--fork-snapshot")
    _scope(create)
    listing = command(changes, "list", BindingCommand(
        "task_workflow_change_list", options=SCOPE_OPTIONS,
        projector="project_change_list", projector_full=False, projector_options=(("all", "all"),),
        formatter="format_change_list", formatter_options=SCOPE_OPTIONS + (("all", "all"),)))
    listing.add_argument("--all", action="store_true")
    _scope(listing)
    for verb in ("show", "revert", "replay", "close", "publish"):
        options = (("remote_name", "remote_name"),) if verb == "publish" else SCOPE_OPTIONS
        if verb in ("revert", "replay"):
            options += (("force", "force"), ("dry_run", "dry_run"))
        if verb == "replay":
            options += (("onto", "onto"),)
        item = command(changes, verb, BindingCommand(
            "change_" + verb + "_operation", ("reference",), options,
            formatter="format_change_" + verb))
        item.add_argument("reference", metavar="TASK_ID[/C-##]")
        if verb == "publish":
            item.add_argument("--remote", dest="remote_name")
        else:
            _scope(item)
        if verb in ("revert", "replay"):
            item.add_argument("--force", action="store_true")
            item.add_argument("--dry-run", action="store_true")
        if verb == "replay":
            item.add_argument("--onto", help=argparse.SUPPRESS)

    patchsets = commands.add_parser("patchset", allow_abbrev=False).add_subparsers(required=True)
    publish = command(patchsets, "publish", BindingCommand(
        "patchset_publish_operation", ("reference", "summary"),
        (("author_mode", "author_mode"), ("remote_name", "remote_name")), formatter="format_patchset_publish"))
    publish.add_argument("reference", type=_task_reference, metavar="TASK_ID")
    publish.add_argument("--summary", required=True)
    publish.add_argument("--author-mode", choices=AUTHOR_MODES)
    publish.add_argument("--remote", dest="remote_name")
    listing = command(patchsets, "list", BindingCommand(
        "patchset_list_operation", ("reference",), (("remote_name", "remote_name"),),
        formatter="format_patchset_list"))
    listing.add_argument("reference", type=_task_reference, metavar="TASK_ID")
    listing.add_argument("--remote", dest="remote_name")
    for verb in ("show", "select", "ci-status", "rerun-ci"):
        operation = verb.replace("-", "_")
        item = command(patchsets, verb, BindingCommand(
            "task_workflow_patchset_" + operation, ("patchset_id",), (("remote_name", "remote_name"),),
            formatter="format_patchset_" + operation))
        item.add_argument("patchset_id", type=_patchset_reference, metavar="PATCHSET_ID")
        item.add_argument("--remote", dest="remote_name")

    reviews = commands.add_parser("review", allow_abbrev=False).add_subparsers(required=True)
    show = command(reviews, "show", BindingCommand(
        "review_show_operation", ("reference",), (("remote_name", "remote_name"),),
        projector="project_review_show", projector_full=False, formatter="format_review_show"))
    show.add_argument("reference", metavar="TASK_ID")
    show.add_argument("--remote", dest="remote_name")
    team_reviews = reviews.add_parser("team", allow_abbrev=False).add_subparsers(required=True)
    request = command(team_reviews, "request", BindingCommand(
        "review_team_request_operation", ("reference", "reviewer_groups"),
        (("patchset_id", "patchset_id"), ("note", "note"), ("remote_name", "remote_name")),
        formatter="format_review_team_request",
        argument_validators=(("patchset_id", "validate_public_patchset_argument"),)))
    request.add_argument("reference", metavar="TASK_ID")
    request.add_argument("--group", dest="reviewer_groups", action="append", required=True)
    request.add_argument("--patchset", dest="patchset_id")
    request.add_argument("--note")
    request.add_argument("--remote", dest="remote_name")
    for verb in ("approve", "request-changes", "comment", "defer"):
        operation = verb.replace("-", "_")
        item = command(team_reviews, verb, BindingCommand(
            "review_team_" + operation + "_operation", ("reference",),
            (("patchset_id", "patchset_id"), ("reviewer", "reviewer"), ("message", "message"),
             ("remote_name", "remote_name")), formatter="format_review_team_" + operation,
            argument_validators=(("patchset_id", "validate_public_patchset_argument"),)))
        item.add_argument("reference", metavar="TASK_ID")
        item.add_argument("--patchset", dest="patchset_id")
        item.add_argument("--reviewer")
        item.add_argument("--message")
        item.add_argument("--remote", dest="remote_name")
    task_reviews = reviews.add_parser("task", allow_abbrev=False).add_subparsers(required=True)
    approve = command(task_reviews, "approve", BindingCommand(
        "review_task_approve_operation", ("reference", "patchset_id", "message"),
        (("remote_name", "remote_name"),), projector="project_review_task_operation",
        projector_full=False, formatter="format_review_task_approve",
        argument_validators=(("patchset_id", "validate_review_patchset_selector"),)))
    approve.add_argument("reference", metavar="TASK_ID")
    approve.add_argument("--patchset", dest="patchset_id", required=True)
    approve.add_argument("--message", required=True)
    approve.add_argument("--remote", dest="remote_name")
    for verb in ("request-changes", "comment", "defer"):
        operation = verb.replace("-", "_")
        item = command(task_reviews, verb, BindingCommand(
            "review_task_" + operation + "_operation", ("reference",),
            (("patchset_id", "patchset_id"), ("message", "message"), ("remote_name", "remote_name")),
            projector="project_review_task_operation", projector_full=False,
            formatter="format_review_task_" + operation,
            argument_validators=(("patchset_id", "validate_review_patchset_selector"),)))
        item.add_argument("reference", metavar="TASK_ID")
        item.add_argument("--patchset", dest="patchset_id")
        item.add_argument("--message")
        item.add_argument("--remote", dest="remote_name")
    code_reviews = reviews.add_parser("code", allow_abbrev=False).add_subparsers(required=True)
    submit = command(code_reviews, "submit", BindingCommand(
        "review_code_submit_operation", ("reference", "patchset_id", "message"),
        (("reviewer", "reviewer"), ("remote_name", "remote_name")), formatter="format_review_code_submit",
        argument_validators=(("patchset_id", "validate_public_patchset_argument"),)))
    submit.add_argument("reference", metavar="TASK_ID")
    submit.add_argument("--patchset", dest="patchset_id", required=True)
    submit.add_argument("--message", required=True)
    submit.add_argument("--reviewer")
    submit.add_argument("--remote", dest="remote_name")
    template = command(code_reviews, "template", BindingCommand(
        "task_workflow_review_code_template", options=(("style", "style"),), repository=False,
        formatter="format_review_code_template"))
    template.add_argument("--style", choices=("inline", "numbered"), default="numbered")

    attestations = commands.add_parser("attest", allow_abbrev=False).add_subparsers(required=True)
    attest_fields = ("tests", "lint", "security", "license", "author_mode", "model")
    put = command(attestations, "put", BindingCommand(
        "task_workflow_attest_put", options=(("patchset_id", "patchset_id"), ("remote_name", "remote_name")) +
        tuple((field, field) for field in attest_fields), formatter="format_attest_put",
        argument_validators=(("patchset_id", "validate_public_patchset_argument"),)))
    put.add_argument("patchset_id", metavar="PATCHSET_ID")
    for field in attest_fields:
        put.add_argument("--" + field.replace("_", "-"))
    put.add_argument("--remote", dest="remote_name")
    show = command(attestations, "show", BindingCommand(
        "task_workflow_attest_show", ("patchset_id",), (("remote_name", "remote_name"),),
        formatter="format_attest_show", argument_validators=(("patchset_id", "validate_public_patchset_argument"),)))
    show.add_argument("patchset_id", metavar="PATCHSET_ID")
    show.add_argument("--remote", dest="remote_name")
    policies = commands.add_parser("policy", allow_abbrev=False).add_subparsers(required=True)
    for verb in ("eval", "show", "waive"):
        options = (("remote_name", "remote_name"),)
        if verb == "waive":
            options += (("rule_name", "rule_name"), ("reason", "reason"), ("expires_at", "expires_at"))
        item = command(policies, verb, BindingCommand(
            "task_workflow_policy_" + verb, ("patchset_id",), options,
            formatter="format_policy_" + verb, projector="project_policy_show" if verb == "show" else None,
            projector_full=False, argument_validators=(("patchset_id", "validate_public_patchset_argument"),)))
        item.add_argument("patchset_id", metavar="PATCHSET_ID")
        item.add_argument("--remote", dest="remote_name")
        if verb == "waive":
            item.add_argument("--rule", dest="rule_name", required=True)
            item.add_argument("--reason", required=True)
            item.add_argument("--expires-at")

    stashes = commands.add_parser("stash", allow_abbrev=False).add_subparsers(required=True)
    save = command(stashes, "save", BindingCommand(
        "stash_save_operation", options=(("message", "message"), ("keep_workspace", "keep_workspace")),
        formatter="format_stash_save"))
    save.add_argument("--message")
    save.add_argument("--keep-workspace", action="store_true")
    command(stashes, "list", BindingCommand("task_workflow_stash_list", formatter="format_stash_list"))
    for verb in ("show", "apply", "pop", "drop"):
        operation = "task_workflow_stash_show" if verb == "show" else "stash_" + verb + "_operation"
        options = (("force", "force"),) if verb in ("apply", "pop") else ()
        item = command(stashes, verb, BindingCommand(
            operation, ("stash_id",), options, formatter="format_stash_" + verb))
        item.add_argument("stash_id", metavar="STASH_ID")
        if verb in ("apply", "pop"):
            item.add_argument("--force", action="store_true")

    tags = commands.add_parser("tag", allow_abbrev=False).add_subparsers(required=True)
    create = command(tags, "create", BindingCommand(
        "tag_create_operation", ("name", "message"), (("snapshot_id", "snapshot_id"),),
        formatter="format_tag_create"))
    create.add_argument("name", metavar="NAME")
    create.add_argument("--message", required=True)
    create.add_argument("--snapshot", dest="snapshot_id")
    command(tags, "list", BindingCommand("tag_list_operation", formatter="format_tag_list"))
    for verb in ("show", "delete"):
        item = command(tags, verb, BindingCommand(
            "tag_" + verb + "_operation", ("name",), formatter="format_tag_" + verb))
        item.add_argument("name", metavar="NAME")

    capture_binding = BindingCommand(
        "snapshot_capture", ("reference",),
        (("message", "message"), ("resume_snapshot_id", "resume_snapshot_id")),
        projector="project_snapshot_capture", formatter="format_snapshot_capture",
        argument_validators=(("reference", "parse_snapshot_authoring_reference"),))

    def capture_options(item: argparse.ArgumentParser) -> None:
        item.add_argument("reference", metavar="TASK_ID")
        capture = item.add_mutually_exclusive_group()
        capture.add_argument("-m", "--message")
        capture.add_argument("--resume", dest="resume_snapshot_id")
        _full_option(item)

    snapshots = commands.add_parser("snapshot", allow_abbrev=False).add_subparsers(required=True)
    capture_options(command(snapshots, "create", capture_binding))
    capture_options(command(commands, "commit", capture_binding))
    query_errors = {"error_formatter": "format_snapshot_query_error", "error_exit": "snapshot_query_error_exit_code"}
    ancestry = command(snapshots, "ancestry", BindingCommand(
        "snapshot_ancestry_operation", ("selector",),
        (("descendants", "descendants"), ("first_parent", "first_parent"),
         ("max_depth", "max_depth"), ("limit", "limit")),
        formatter="format_snapshot_ancestry", formatter_options=(("all", "all"),), **query_errors))
    ancestry.add_argument("selector", metavar="SNAPSHOT_OR_TAG")
    direction = ancestry.add_mutually_exclusive_group()
    direction.add_argument("--ancestors", action="store_true")
    direction.add_argument("--descendants", action="store_true")
    ancestry.add_argument("--first-parent", action="store_true")
    ancestry.add_argument("--max-depth", type=_positive_usize, default=10000)
    ancestry.add_argument("--limit", type=_positive_usize, default=10000)
    ancestry.add_argument("--all", action="store_true")
    ancestor = command(snapshots, "is-ancestor", BindingCommand(
        "snapshot_is_ancestor_operation", ("older", "newer"),
        formatter="format_snapshot_is_ancestor", phase_result=True, **query_errors))
    ancestor.add_argument("older", metavar="OLDER_SNAPSHOT_OR_TAG")
    ancestor.add_argument("newer", metavar="NEWER_SNAPSHOT_OR_TAG")
    base = command(snapshots, "merge-base", BindingCommand(
        "snapshot_merge_base_operation", ("left", "right"), (("all", "all"),),
        formatter="format_snapshot_merge_base", phase_result=True, omit_empty_text=True, **query_errors))
    base.add_argument("left", metavar="LEFT_SNAPSHOT_OR_TAG")
    base.add_argument("right", metavar="RIGHT_SNAPSHOT_OR_TAG")
    base.add_argument("--all", action="store_true")
    listing = command(snapshots, "list", BindingCommand(
        "task_workflow_snapshot_list", projector="project_snapshot_list", projector_full=False,
        projector_options=(("all", "all"),), formatter="format_snapshot_list", formatter_options=(("all", "all"),)))
    listing.add_argument("--all", action="store_true")
    show = command(snapshots, "show", BindingCommand(
        "inspect_snapshot_operation", ("selector",), inspection_result=True,
        formatter="format_snapshot_show", formatter_options=(("files", "files"),)))
    show.add_argument("selector", metavar="SNAPSHOT_OR_TAG")
    show.add_argument("--files", action="store_true")
    diff = command(snapshots, "diff", BindingCommand(
        "snapshot_diff_operation", ("older", "newer"),
        (("include_text", "include_text"), ("max_bytes", "max_bytes")),
        formatter="format_snapshot_diff", requires=(("max_bytes", "include_text"),)))
    diff.add_argument("older", metavar="OLD_SNAPSHOT_OR_TAG")
    diff.add_argument("newer", metavar="NEW_SNAPSHOT_OR_TAG")
    diff.add_argument("--include-text", action="store_true")
    diff.add_argument("--max-bytes", type=_positive_usize)
    for verb in ("revert", "replay"):
        options = (("force", "force"), ("dry_run", "dry_run"))
        if verb == "replay":
            options += (("onto", "onto"),)
        item = command(snapshots, verb, BindingCommand(
            "snapshot_" + verb + "_operation", ("selector",), options,
            formatter="format_snapshot_" + verb))
        item.add_argument("selector", metavar="SNAPSHOT_OR_TAG")
        item.add_argument("--force", action="store_true")
        item.add_argument("--dry-run", action="store_true")
        if verb == "replay":
            item.add_argument("--onto", help=argparse.SUPPRESS)
    restore_lines = command(snapshots, "restore-lines", BindingCommand(
        "task_workflow_snapshot_restore_lines", ("snapshot_id", "path"),
        (("line", "line"), ("start_line", "start_line"), ("end_line", "end_line"), ("yes", "yes")),
        formatter="format_snapshot_restore_lines",
        requires=(("start_line", "end_line"), ("end_line", "start_line")),
        conflicts=(("line", "end_line"),)))
    restore_lines.add_argument("snapshot_id", metavar="SNAPSHOT_ID")
    restore_lines.add_argument("path", metavar="PATH")
    selection = restore_lines.add_mutually_exclusive_group(required=True)
    selection.add_argument("--line", type=_positive_usize)
    selection.add_argument("--start", dest="start_line", type=_positive_usize)
    restore_lines.add_argument("--end", dest="end_line", type=_positive_usize)
    restore_lines.add_argument("--yes", action="store_true")
    return parser


def _native_text(runtime: NativeRuntime, function: str, *args: Any, **kwargs: Any) -> str:
    result = runtime.call(function, *args, **kwargs)
    if not isinstance(result, str):
        raise NativeProtocolError(f"native {function} did not return text")
    return result


def _native_exit(runtime: NativeRuntime, function: str, *args: Any, **kwargs: Any) -> int:
    return _exit_value(runtime.call(function, *args, **kwargs), function)


def _exit_value(result: Any, function: str) -> int:
    if not isinstance(result, int) or isinstance(result, bool) or not 0 <= result <= 255:
        raise NativeProtocolError(f"native {function} did not return an exit code")
    return result


def _domain_error(runtime: NativeRuntime, message: str, formatter: str, exit_projector: str) -> int:
    print(_native_text(runtime, formatter, message), file=sys.stderr)
    return _native_exit(runtime, exit_projector)


def _render_json(payload: Any, stream: TextIO, runtime: NativeRuntime) -> None:
    print(_native_text(runtime, "encode_json", payload, pretty=True), file=stream)


def _resolve_command(runtime: NativeRuntime, binding: BindingCommand, json_output: bool, formatter: str | None) -> None:
    required = [binding.export, binding.error_formatter, binding.error_exit]
    required.extend(function for _, function in binding.argument_validators)
    if json_output:
        required.append("encode_json")
        if binding.projector:
            required.append(binding.projector)
        if binding.projector_repository:
            required.append("repository_authority_root")
    elif formatter:
        required.append(formatter)
    for function in (binding.exit_projector, binding.error_projector):
        if function:
            required.append(function)
    for function in dict.fromkeys(required):
        runtime.resolve_callable(function)


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if getattr(args, "full", False) and not args.json:
        parser.error("--full requires --json")
    for name in args.binding.json_conflicts:
        if args.json and getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} cannot be used with --json")
    for name, required in args.binding.requires:
        value = getattr(args, name)
        required_value = getattr(args, required)
        if _provided(value) and not _provided(required_value):
            parser.error(f"{_option_label(name)} requires {_option_label(required)}")
    for name, required in args.binding.requires_any:
        if _provided(getattr(args, name)) and not any(_provided(getattr(args, field)) for field in required):
            parser.error(f"{_option_label(name)} requires " + " or ".join(_option_label(field) for field in required))
    for left, right in args.binding.conflicts:
        a, b = getattr(args, left), getattr(args, right)
        if _provided(a) and _provided(b):
            parser.error(f"{_option_label(left)} conflicts with {_option_label(right)}")
    runtime = NativeRuntime()
    try:
        binding = args.binding
        formatter = next((function for field, function in binding.formatter_choices if getattr(args, field)), binding.formatter)
        if not args.json and formatter is None:
            raise NativeResolutionError("native text presentation for this command is not mapped yet")
        _resolve_command(runtime, binding, args.json, formatter)
        for name, validator in binding.argument_validators:
            value = getattr(args, name)
            if value is None:
                continue
            try:
                validated = runtime.call(validator, value)
            except ValueError as error:
                parser.error(str(error))
            if not isinstance(validated, str):
                raise NativeProtocolError(f"native {validator} did not return a parsed argument")
            setattr(args, name, validated)
        repo_root = str(Path.cwd()) if binding.repository or binding.projector_repository else None
        projection_root = repo_root
        if args.json and binding.projector_repository:
            projection_root = _native_text(runtime, "repository_authority_root", repo_root)
            if not projection_root:
                raise NativeProtocolError("native repository authority root is empty")
        request = binding.domain_request(args)
        payload = binding.invoke(args, runtime, repo_root, request)
        phase_exit = 0
        if binding.phase_result:
            if not isinstance(payload, dict) or "payload" not in payload or "exit_code" not in payload:
                raise NativeProtocolError("native workflow operation did not return payload and exit_code")
            phase_exit = _exit_value(payload["exit_code"], binding.export)
            payload = payload["payload"]
        if binding.inspection_result:
            if not isinstance(payload, dict) or "payload" not in payload or "delta" not in payload:
                raise NativeProtocolError("native Snapshot inspection did not return payload and delta")
        if binding.line_inventory_result:
            if not isinstance(payload, dict) or "payload" not in payload or "current_line" not in payload:
                raise NativeProtocolError("native Line inventory did not return payload and current_line")
        if args.json:
            projected = payload["payload"] if binding.inspection_result or binding.line_inventory_result else payload
            if binding.projector:
                parameters = ([projection_root] if binding.projector_repository else []) + [payload]
                if binding.projector_requested:
                    parameters.append(getattr(args, binding.requested))
                options = {native: getattr(args, public) for public, native in binding.projector_options
                           if getattr(args, public) is not None}
                if binding.projector_full:
                    options["full"] = getattr(args, "full", False)
                projected = runtime.call(binding.projector, *parameters, **options)
            _render_json(projected, sys.stdout, runtime)
        elif formatter:
            parameters = (payload, getattr(args, binding.requested)) if binding.requested else (payload,)
            if binding.formatter_request:
                parameters += (request,)
            options = {native: getattr(args, public) for public, native in binding.formatter_options
                       if getattr(args, public) is not None}
            if binding.formatter_multiple:
                options["multiple"] = len(getattr(args, binding.formatter_multiple)) > 1
            rendered = _native_text(runtime, formatter, *parameters, **options)
            if binding.formatter_verbatim:
                sys.stdout.write(rendered)
            elif rendered or not binding.omit_empty_text:
                print(rendered)
        if binding.error_projector:
            error = runtime.call(binding.error_projector, payload)
            if error is not None:
                if not isinstance(error, str):
                    raise NativeProtocolError("native CLI error projector did not return text or None")
                return _domain_error(runtime, error, binding.error_formatter, binding.error_exit)
        if binding.exit_projector:
            options = {native: getattr(args, public) for public, native in binding.exit_projector_options
                       if getattr(args, public) is not None}
            return _native_exit(runtime, binding.exit_projector, payload, **options)
        return phase_exit
    except NativeBridgeError as error:
        print(f"Error: {error}", file=sys.stderr)
        return 1
    except (RuntimeError, ValueError, OSError) as error:
        try:
            return _domain_error(runtime, str(error), binding.error_formatter, binding.error_exit)
        except (NativeBridgeError, RuntimeError, ValueError, OSError) as presentation_error:
            print(f"Error: {presentation_error}", file=sys.stderr)
            return 1
