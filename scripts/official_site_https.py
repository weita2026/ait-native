#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import shutil
from dataclasses import dataclass
from pathlib import Path
from string import Template

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
SITE_BUILD_PATH = WORKSPACE_ROOT / "site" / "build.py"
DEFAULT_OUTPUT = WORKSPACE_ROOT / "site" / "dist"
MACOS_NGINX_DIR = WORKSPACE_ROOT / "deploy" / "site" / "macos-nginx"
DEFAULT_NGINX_ENV_FILE = MACOS_NGINX_DIR / "site.env"
DEFAULT_ENV_FILE = DEFAULT_NGINX_ENV_FILE
DEFAULT_NGINX_RENDER_DIR = WORKSPACE_ROOT / "deploy" / "site" / "nginx-rendered"
AUTHORED_MACOS_NGINX_DIR = AUTHORED_ROOT / "deploy" / "site" / "macos-nginx"


@dataclass
class EnvCheck:
    name: str
    ok: bool
    detail: str


def load_build_module():
    spec = importlib.util.spec_from_file_location("official_site_build", SITE_BUILD_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(module)
    return module


def parse_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path.exists():
        return values
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def is_absolute_host_path(value: str) -> bool:
    return Path(value).is_absolute()


def _split_host_list(value: str) -> list[str]:
    return [token for token in value.replace(",", " ").split() if token]


def _hostname_error(value: str) -> str | None:
    if not value:
        return "missing"
    if value.startswith(("http://", "https://")):
        return "must be a hostname, not a URL"
    if "." not in value:
        return "should look like a public hostname"
    return None


def _build_site_output(output: Path, *, clean: bool) -> list[Path]:
    if clean and output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True, exist_ok=True)

    build = load_build_module()
    return build.build_site(output)


def command_build(args: argparse.Namespace) -> int:
    output = Path(args.output).resolve()
    built = _build_site_output(output, clean=args.clean)
    print(f"Built {len(built)} page(s) into {output}")
    print(output / "index.html")
    print(output / "public" / "styles.css")
    return 0


def _domain_check(env: dict[str, str]) -> EnvCheck:
    value = env.get("SITE_DOMAIN", "")
    error = _hostname_error(value)
    if error:
        return EnvCheck("SITE_DOMAIN", False, error)
    return EnvCheck("SITE_DOMAIN", True, value)


def _domain_aliases(env: dict[str, str]) -> list[str]:
    return _split_host_list(env.get("SITE_DOMAIN_ALIASES", ""))


def _domain_aliases_check(env: dict[str, str]) -> EnvCheck:
    raw_value = env.get("SITE_DOMAIN_ALIASES", "")
    if not raw_value.strip():
        return EnvCheck("SITE_DOMAIN_ALIASES", True, "none")

    primary = env.get("SITE_DOMAIN", "")
    aliases = _domain_aliases(env)
    if not aliases:
        return EnvCheck("SITE_DOMAIN_ALIASES", False, "set but no hostnames were parsed")

    seen: set[str] = set()
    normalized: list[str] = []
    for alias in aliases:
        error = _hostname_error(alias)
        if error:
            return EnvCheck("SITE_DOMAIN_ALIASES", False, f"{alias}: {error}")
        if alias == primary:
            return EnvCheck("SITE_DOMAIN_ALIASES", False, f"{alias}: duplicates SITE_DOMAIN")
        if alias in seen:
            return EnvCheck("SITE_DOMAIN_ALIASES", False, f"{alias}: duplicate alias")
        seen.add(alias)
        normalized.append(alias)

    return EnvCheck("SITE_DOMAIN_ALIASES", True, ", ".join(normalized))


def _email_check(env: dict[str, str]) -> EnvCheck:
    value = env.get("SITE_EMAIL", "")
    if not value:
        return EnvCheck("SITE_EMAIL", False, "missing")
    if "@" not in value:
        return EnvCheck("SITE_EMAIL", False, "should look like an email address")
    return EnvCheck("SITE_EMAIL", True, value)


def _build_dir_check(env: dict[str, str]) -> EnvCheck:
    value = env.get("SITE_BUILD_DIR", "")
    if not value:
        return EnvCheck("SITE_BUILD_DIR", False, "missing")
    if not is_absolute_host_path(value):
        return EnvCheck("SITE_BUILD_DIR", False, "must be an absolute host path")
    build_dir = Path(value)
    if not build_dir.exists():
        return EnvCheck("SITE_BUILD_DIR", False, f"path does not exist: {build_dir}")
    expected = [build_dir / "index.html", build_dir / "public" / "styles.css"]
    missing = [str(path) for path in expected if not path.exists()]
    if missing:
        return EnvCheck("SITE_BUILD_DIR", False, f"missing build output: {', '.join(missing)}")
    return EnvCheck("SITE_BUILD_DIR", True, str(build_dir))


def _port_check(env: dict[str, str], key: str, default: str) -> EnvCheck:
    value = env.get(key, default)
    try:
        port = int(value)
    except ValueError:
        return EnvCheck(key, False, "must be an integer")
    if port <= 0 or port > 65535:
        return EnvCheck(key, False, "must be between 1 and 65535")
    return EnvCheck(key, True, str(port))


def _acme_check(env: dict[str, str]) -> EnvCheck:
    value = env.get("SITE_ACME_CA", "https://acme-v02.api.letsencrypt.org/directory")
    if not value.startswith("https://"):
        return EnvCheck("SITE_ACME_CA", False, "should be an https URL")
    return EnvCheck("SITE_ACME_CA", True, value)


def _absolute_path_check(env: dict[str, str], key: str, *, must_exist: bool = False) -> EnvCheck:
    value = env.get(key, "")
    if not value:
        return EnvCheck(key, False, "missing")
    if not is_absolute_host_path(value):
        return EnvCheck(key, False, "must be an absolute host path")
    path = Path(value)
    if must_exist and not path.exists():
        return EnvCheck(key, False, f"path does not exist: {path}")
    return EnvCheck(key, True, str(path))


def _command_check(env: dict[str, str], key: str) -> EnvCheck:
    value = env.get(key, "")
    if not value:
        return EnvCheck(key, False, "missing")
    return EnvCheck(key, True, value)


def _print_checks(checks: list[EnvCheck]) -> None:
    for check in checks:
        status = "OK" if check.ok else "FAIL"
        print(f"[{status}] {check.name}: {check.detail}")


def _doctor_checks(env_path: Path, env: dict[str, str]) -> list[EnvCheck]:
    return [
        EnvCheck("ENV_FILE", env_path.exists(), str(env_path) if env_path.exists() else f"missing: {env_path}"),
        _domain_check(env),
        _email_check(env),
        _build_dir_check(env),
        _port_check(env, "SITE_HTTP_PORT", "80"),
        _port_check(env, "SITE_HTTPS_PORT", "443"),
        _acme_check(env),
    ]


def _doctor_nginx_checks(
    env_path: Path,
    env: dict[str, str],
    *,
    build_dir_must_exist: bool,
) -> list[EnvCheck]:
    build_dir_check = _build_dir_check(env) if build_dir_must_exist else _absolute_path_check(env, "SITE_BUILD_DIR")
    return [
        EnvCheck("ENV_FILE", env_path.exists(), str(env_path) if env_path.exists() else f"missing: {env_path}"),
        _domain_check(env),
        _domain_aliases_check(env),
        _email_check(env),
        build_dir_check,
        _port_check(env, "SITE_HTTP_PORT", "80"),
        _port_check(env, "SITE_HTTPS_PORT", "443"),
        _absolute_path_check(env, "SITE_ACME_WEBROOT"),
        _absolute_path_check(env, "SITE_NGINX_CONF_DIR"),
        _absolute_path_check(env, "SITE_NGINX_LOG_DIR"),
        _absolute_path_check(env, "SITE_NGINX_CERT_DIR"),
        _command_check(env, "SITE_ACME_HOME"),
        _command_check(env, "SITE_NGINX_RELOAD_CMD"),
        _nginx_template_exists("site-http.conf.template"),
        _nginx_template_exists("site-https.conf.template"),
        _nginx_template_exists("issue_and_install_letsencrypt.sh.template"),
        EnvCheck("NGINX_README", (AUTHORED_MACOS_NGINX_DIR / "README.md").exists(), str(AUTHORED_MACOS_NGINX_DIR / "README.md")),
    ]


def command_doctor(args: argparse.Namespace) -> int:
    env_path = Path(args.env_file).resolve()
    env = parse_env_file(env_path)
    checks = _doctor_checks(env_path, env)
    _print_checks(checks)

    if args.require_public_ports:
        print("[INFO] Live issuance still requires inbound reachability on ports 80 and 443 plus correct DNS.")
    print("[INFO] Shared stack bootstrap belongs in an operator-managed workspace outside this repository checkout.")

    return 0 if all(check.ok for check in checks) else 1


def _nginx_template_exists(name: str) -> EnvCheck:
    path = MACOS_NGINX_DIR / name
    return EnvCheck(name.upper().replace('.', '_'), path.exists(), str(path))


def command_doctor_nginx(args: argparse.Namespace) -> int:
    env_path = Path(args.env_file).resolve()
    env = parse_env_file(env_path)
    checks = _doctor_nginx_checks(env_path, env, build_dir_must_exist=True)
    _print_checks(checks)

    if args.require_public_ports:
        print("[INFO] Live issuance still requires inbound reachability on ports 80 and 443 plus correct DNS.")
        print("[INFO] macOS hosting also depends on a non-sleeping host plus router/firewall forwarding that reaches this machine.")

    return 0 if all(check.ok for check in checks) else 1


def _render_template(path: Path, variables: dict[str, str]) -> str:
    return Template(path.read_text(encoding="utf-8")).safe_substitute(variables)


def _render_alias_https_server_block(env: dict[str, str], aliases: list[str]) -> str:
    if not aliases:
        return ""

    canonical_host = env["SITE_DOMAIN"]
    https_port = env.get("SITE_HTTPS_PORT", "443")
    cert_dir = env["SITE_NGINX_CERT_DIR"]
    log_dir = env["SITE_NGINX_LOG_DIR"]
    alias_names = " ".join(aliases)

    return "\n".join(
        [
            "",
            "server {",
            f"    listen {https_port} ssl http2;",
            f"    listen [::]:{https_port} ssl http2;",
            f"    server_name {alias_names};",
            "",
            f"    access_log {log_dir}/{canonical_host}.alias.ssl.access.log;",
            f"    error_log {log_dir}/{canonical_host}.alias.ssl.error.log warn;",
            "",
            f"    ssl_certificate {cert_dir}/fullchain.pem;",
            f"    ssl_certificate_key {cert_dir}/privkey.pem;",
            "    ssl_session_timeout 1d;",
            "    ssl_session_cache shared:SSL:10m;",
            "    ssl_session_tickets off;",
            "    ssl_protocols TLSv1.2 TLSv1.3;",
            "    ssl_prefer_server_ciphers off;",
            "",
            '    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;',
            "",
            "    location / {",
            f"        return 301 https://{canonical_host}$request_uri;",
            "    }",
            "}",
        ]
    )


def _render_nginx_outputs(env: dict[str, str], output: Path) -> list[Path]:
    output.mkdir(parents=True, exist_ok=True)
    aliases = _domain_aliases(env)
    all_domains = [env["SITE_DOMAIN"], *aliases]
    variables = {
        "SITE_DOMAIN": env["SITE_DOMAIN"],
        "SITE_DOMAIN_ALIASES": " ".join(aliases),
        "SITE_ALL_SERVER_NAMES": " ".join(all_domains),
        "SITE_EMAIL": env["SITE_EMAIL"],
        "SITE_BUILD_DIR": env["SITE_BUILD_DIR"],
        "SITE_HTTP_PORT": env.get("SITE_HTTP_PORT", "80"),
        "SITE_HTTPS_PORT": env.get("SITE_HTTPS_PORT", "443"),
        "SITE_ACME_WEBROOT": env["SITE_ACME_WEBROOT"],
        "SITE_NGINX_CONF_DIR": env["SITE_NGINX_CONF_DIR"],
        "SITE_NGINX_LOG_DIR": env["SITE_NGINX_LOG_DIR"],
        "SITE_NGINX_CERT_DIR": env["SITE_NGINX_CERT_DIR"],
        "SITE_ACME_HOME": env["SITE_ACME_HOME"],
        "SITE_NGINX_RELOAD_CMD": env["SITE_NGINX_RELOAD_CMD"],
        "SITE_ACME_DOMAIN_ARGS": " ".join(f'-d "{domain}"' for domain in all_domains),
        "SITE_ALIAS_HTTPS_SERVER_BLOCK": _render_alias_https_server_block(env, aliases),
        "SITE_CERTIFICATE_ALIAS_SUFFIX": "" if not aliases else f" plus aliases: {', '.join(aliases)}",
    }

    outputs = {
        "official-site-http.conf": _render_template(MACOS_NGINX_DIR / "site-http.conf.template", variables),
        "official-site-https.conf": _render_template(MACOS_NGINX_DIR / "site-https.conf.template", variables),
        "issue_and_install_letsencrypt.sh": _render_template(MACOS_NGINX_DIR / "issue_and_install_letsencrypt.sh.template", variables),
    }

    written: list[Path] = []
    for name, content in outputs.items():
        path = output / name
        path.write_text(content, encoding="utf-8")
        if name.endswith(".sh"):
            path.chmod(0o755)
        written.append(path)
    return written


def command_render_nginx(args: argparse.Namespace) -> int:
    env_path = Path(args.env_file).resolve()
    env = parse_env_file(env_path)
    checks = _doctor_nginx_checks(env_path, env, build_dir_must_exist=True)
    _print_checks(checks)
    if not all(check.ok for check in checks):
        return 1

    for path in _render_nginx_outputs(env, Path(args.output_dir).resolve()):
        print(path)

    return 0


def command_release_nginx(args: argparse.Namespace) -> int:
    env_path = Path(args.env_file).resolve()
    env = parse_env_file(env_path)
    aliases = _domain_aliases(env)
    checks = _doctor_nginx_checks(env_path, env, build_dir_must_exist=False)
    _print_checks(checks)
    if not all(check.ok for check in checks):
        return 1

    build_dir = Path(env["SITE_BUILD_DIR"]).expanduser().resolve()
    built = _build_site_output(build_dir, clean=not args.no_clean)
    print(f"Built {len(built)} page(s) into {build_dir}")
    print(build_dir / "index.html")
    print(build_dir / "public" / "styles.css")

    post_build_checks = _doctor_nginx_checks(env_path, env, build_dir_must_exist=True)
    _print_checks(post_build_checks)
    if not all(check.ok for check in post_build_checks):
        return 1

    rendered_paths = _render_nginx_outputs(env, Path(args.output_dir).resolve())
    for path in rendered_paths:
        print(path)

    print(f"[INFO] macOS nginx release assets are ready from SITE_BUILD_DIR={build_dir}")
    print("[INFO] Content-only site changes are live from the rebuilt static output and do not require an nginx reload by themselves.")
    print("[INFO] Copy or inspect the refreshed rendered nginx files only when config or certificate handling changed.")
    if aliases:
        print(f"[INFO] Alias hostnames covered by the rendered certificate/config: {', '.join(aliases)}")
        print(f"[INFO] Alias traffic should redirect to canonical https://{env['SITE_DOMAIN']} after the rendered HTTPS config is deployed.")
    print(f"[INFO] If you deploy changed nginx config into {env['SITE_NGINX_CONF_DIR']}, validate and reload with: sudo nginx -t && {env['SITE_NGINX_RELOAD_CMD']}")

    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Build and validate the official-site deployment packages."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    build_parser = subparsers.add_parser("build", help="Render the public site into a clean static output directory.")
    build_parser.add_argument("--output", default=str(DEFAULT_OUTPUT), help="Output directory for the built static site.")
    build_parser.add_argument("--no-clean", action="store_true", help="Do not remove the output directory before rebuilding.")
    build_parser.set_defaults(func=command_build)

    doctor_parser = subparsers.add_parser(
        "doctor",
        help="Validate the local official-site build/env contract without routing through legacy external operator wrappers.",
    )
    doctor_parser.add_argument("--env-file", default=str(DEFAULT_ENV_FILE), help="Path to the deployment env file.")
    doctor_parser.add_argument(
        "--no-require-public-ports",
        action="store_true",
        help="Suppress the reminder about public port/DNS prerequisites.",
    )
    doctor_parser.set_defaults(func=command_doctor)

    doctor_nginx_parser = subparsers.add_parser(
        "doctor-nginx", help="Validate the macOS-native Nginx deployment env file and templates."
    )
    doctor_nginx_parser.add_argument(
        "--env-file", default=str(DEFAULT_NGINX_ENV_FILE), help="Path to the macOS-native nginx env file."
    )
    doctor_nginx_parser.add_argument(
        "--no-require-public-ports",
        action="store_true",
        help="Suppress the reminder about public port/DNS prerequisites.",
    )
    doctor_nginx_parser.set_defaults(func=command_doctor_nginx)

    render_nginx_parser = subparsers.add_parser(
        "render-nginx", help="Render concrete macOS-native Nginx configs and helper scripts from templates."
    )
    render_nginx_parser.add_argument(
        "--env-file", default=str(DEFAULT_NGINX_ENV_FILE), help="Path to the macOS-native nginx env file."
    )
    render_nginx_parser.add_argument(
        "--output-dir",
        default=str(DEFAULT_NGINX_RENDER_DIR),
        help="Directory where rendered nginx configs/scripts should be written.",
    )
    render_nginx_parser.set_defaults(func=command_render_nginx)

    release_nginx_parser = subparsers.add_parser(
        "release-nginx",
        help="Build the macOS-native official site output and refresh rendered nginx deployment assets.",
    )
    release_nginx_parser.add_argument(
        "--env-file", default=str(DEFAULT_NGINX_ENV_FILE), help="Path to the macOS-native nginx env file."
    )
    release_nginx_parser.add_argument(
        "--output-dir",
        default=str(DEFAULT_NGINX_RENDER_DIR),
        help="Directory where rendered nginx configs/scripts should be written.",
    )
    release_nginx_parser.add_argument(
        "--no-clean",
        action="store_true",
        help="Do not remove the configured SITE_BUILD_DIR before rebuilding the static site.",
    )
    release_nginx_parser.set_defaults(func=command_release_nginx)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if getattr(args, "command", None) == "build":
        args.clean = not args.no_clean
    if getattr(args, "command", None) in {"doctor", "doctor-nginx"}:
        args.require_public_ports = not args.no_require_public_ports
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
