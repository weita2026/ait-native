from __future__ import annotations

import os
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

from ait.repo_paths import RepoContext

ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(ROOT).repo_root
DEPLOY = ROOT / "deploy" / "site" / "macos-nginx"
AUTHORED_DEPLOY = AUTHORED_ROOT / "deploy" / "site" / "macos-nginx"
SCRIPT = ROOT / "scripts" / "official_site_https.py"


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_official_site_macos_nginx_package_files_exist_and_document_prerequisites():
    expected = [
        AUTHORED_DEPLOY / "README.md",
        DEPLOY / "site.env.example",
        DEPLOY / "site-http.conf.template",
        DEPLOY / "site-https.conf.template",
        DEPLOY / "issue_and_install_letsencrypt.sh.template",
        SCRIPT,
    ]
    for path in expected:
        assert path.exists(), path

    readme = _read(AUTHORED_DEPLOY / "README.md")
    assert "Nginx" in readme
    assert "acme.sh" in readme
    assert "Let's Encrypt" in readme
    assert "ports `80` and `443`" in readme
    assert "brew install nginx" in readme
    assert "curl https://get.acme.sh | sh -s email=" in readme
    assert "SITE_DOMAIN_ALIASES" in readme
    assert "render-nginx" in readme
    assert "./ait.sh site release" in readme
    assert "./ait.sh site preview" in _read(AUTHORED_ROOT / "site" / "README.md")
    assert "Landing a `site/` change onto repo `main` does **not** make the live Mac host update by itself." in readme
    assert "Nginx does **not** need a reload by itself" in readme


def test_official_site_macos_nginx_templates_cover_http_https_and_install_flow():
    http_template = _read(DEPLOY / "site-http.conf.template")
    https_template = _read(DEPLOY / "site-https.conf.template")
    install_template = _read(DEPLOY / "issue_and_install_letsencrypt.sh.template")

    assert "listen ${SITE_HTTP_PORT};" in http_template
    assert "server_name ${SITE_ALL_SERVER_NAMES};" in http_template
    assert "root ${SITE_BUILD_DIR};" in http_template
    assert "root ${SITE_ACME_WEBROOT};" in http_template
    assert "try_files $uri $uri/ =404;" in http_template

    assert "server_name ${SITE_ALL_SERVER_NAMES};" in https_template
    assert "listen ${SITE_HTTPS_PORT} ssl http2;" in https_template
    assert "ssl_certificate ${SITE_NGINX_CERT_DIR}/fullchain.pem;" in https_template
    assert "Strict-Transport-Security" in https_template
    assert "return 301 https://${SITE_DOMAIN}$request_uri;" in https_template
    assert "${SITE_ALIAS_HTTPS_SERVER_BLOCK}" in https_template

    assert "--server letsencrypt" in install_template
    assert "--webroot \"${WEBROOT}\"" in install_template
    assert "${SITE_ACME_DOMAIN_ARGS}" in install_template
    assert "--install-cert -d \"${DOMAIN}\" --ecc" in install_template
    assert "--reloadcmd \"${RELOAD_CMD}\"" in install_template


def test_official_site_macos_nginx_helper_validates_env_and_renders_templates(tmp_path: Path):
    output = tmp_path / "site-dist"
    build = subprocess.run(
        ["python3", str(SCRIPT), "build", "--output", str(output)],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    assert "Built" in build.stdout

    env_file = tmp_path / "site.env"
    acme_root = tmp_path / "acme-webroot"
    env_file.write_text(
        "\n".join(
            [
                "SITE_DOMAIN=ait-native.dev",
                "SITE_DOMAIN_ALIASES=www.ait-native.dev",
                "SITE_EMAIL=ops@ait-native.dev",
                f"SITE_BUILD_DIR={output}",
                "SITE_HTTP_PORT=80",
                "SITE_HTTPS_PORT=443",
                f"SITE_ACME_WEBROOT={acme_root}",
                "SITE_NGINX_CONF_DIR=/opt/homebrew/etc/nginx/servers",
                "SITE_NGINX_LOG_DIR=/opt/homebrew/var/log/nginx",
                "SITE_NGINX_CERT_DIR=/opt/homebrew/etc/nginx/certs/ait-native.dev",
                "SITE_ACME_HOME=$HOME/.acme.sh",
                "SITE_NGINX_RELOAD_CMD=nginx -s reload",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    doctor = subprocess.run(
        ["python3", str(SCRIPT), "doctor-nginx", "--env-file", str(env_file)],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    assert doctor.returncode == 0, doctor.stdout + doctor.stderr
    assert "[OK] SITE_DOMAIN: ait-native.dev" in doctor.stdout
    assert "[OK] SITE_DOMAIN_ALIASES: www.ait-native.dev" in doctor.stdout
    assert "[OK] SITE_NGINX_CONF_DIR: /opt/homebrew/etc/nginx/servers" in doctor.stdout
    assert "[INFO] macOS hosting also depends on a non-sleeping host" in doctor.stdout

    rendered = tmp_path / "rendered"
    render = subprocess.run(
        [
            "python3",
            str(SCRIPT),
            "render-nginx",
            "--env-file",
            str(env_file),
            "--output-dir",
            str(rendered),
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    assert render.returncode == 0, render.stdout + render.stderr

    http_conf = rendered / "official-site-http.conf"
    https_conf = rendered / "official-site-https.conf"
    install_script = rendered / "issue_and_install_letsencrypt.sh"
    assert http_conf.exists()
    assert https_conf.exists()
    assert install_script.exists()
    assert install_script.stat().st_mode & 0o111

    assert "server_name ait-native.dev www.ait-native.dev;" in _read(http_conf)
    assert f"root {output};" in _read(http_conf)
    assert "server_name www.ait-native.dev;" in _read(https_conf)
    assert "return 301 https://ait-native.dev$request_uri;" in _read(https_conf)
    assert "ssl_certificate /opt/homebrew/etc/nginx/certs/ait-native.dev/fullchain.pem;" in _read(https_conf)
    assert "curl https://get.acme.sh | sh -s email=ops@ait-native.dev" in _read(install_script)
    assert '-d "www.ait-native.dev"' in _read(install_script)


def test_official_site_macos_nginx_release_helper_and_ait_sh_wrapper(tmp_path: Path):
    env_file = tmp_path / "site.env"
    output = tmp_path / "site-dist"
    rendered = tmp_path / "rendered"
    acme_root = tmp_path / "acme-webroot"
    env_file.write_text(
        "\n".join(
            [
                "SITE_DOMAIN=ait-native.dev",
                "SITE_EMAIL=ops@ait-native.dev",
                f"SITE_BUILD_DIR={output}",
                "SITE_HTTP_PORT=34567",
                "SITE_HTTPS_PORT=34568",
                f"SITE_ACME_WEBROOT={acme_root}",
                f"SITE_NGINX_CONF_DIR={tmp_path / 'nginx-conf'}",
                f"SITE_NGINX_LOG_DIR={tmp_path / 'nginx-log'}",
                f"SITE_NGINX_CERT_DIR={tmp_path / 'nginx-certs' / 'ait-native.dev'}",
                "SITE_ACME_HOME=$HOME/.acme.sh",
                "SITE_NGINX_RELOAD_CMD=nginx -s reload",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    release = subprocess.run(
        [
            "python3",
            str(SCRIPT),
            "release-nginx",
            "--env-file",
            str(env_file),
            "--output-dir",
            str(rendered),
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    assert release.returncode == 0, release.stdout + release.stderr
    assert output.joinpath("index.html").exists()
    assert output.joinpath("public", "styles.css").exists()
    assert output.joinpath("robots.txt").exists()
    assert output.joinpath("sitemap.xml").exists()
    assert rendered.joinpath("official-site-http.conf").exists()
    assert rendered.joinpath("official-site-https.conf").exists()
    assert rendered.joinpath("issue_and_install_letsencrypt.sh").exists()
    assert "Content-only site changes are live from the rebuilt static output and do not require an nginx reload by themselves." in release.stdout
    assert f"If you deploy changed nginx config into {tmp_path / 'nginx-conf'}" in release.stdout

    env = os.environ.copy()
    env.update(
        {
            "AIT_SITE_ENV_PATH": str(env_file),
            "AIT_SITE_RENDER_DIR": str(rendered),
            "AIT_SITE_HELPER_SCRIPT": str(SCRIPT),
        }
    )
    wrapped = subprocess.run(
        ["bash", "ait.sh", "site", "release"],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )
    assert wrapped.returncode == 0, wrapped.stdout + wrapped.stderr
    assert "macOS nginx release assets are ready" in wrapped.stdout


def test_official_site_preview_wrapper_builds_and_serves_static_output(tmp_path: Path):
    readme = _read(AUTHORED_ROOT / "site" / "README.md")
    assert "./ait.sh site preview" in readme
    assert "http://192.168.1.106:1234" in readme
    assert "python3 -m http.server 1234 --bind 192.168.1.106 --directory site/dist" in readme

    preview_dir = tmp_path / "site-dist"
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        preview_port = sock.getsockname()[1]

    env = os.environ.copy()
    env.update(
        {
            "AIT_SITE_HELPER_SCRIPT": str(SCRIPT),
            "AIT_SITE_PREVIEW_DIR": str(preview_dir),
            "AIT_SITE_PREVIEW_HOST": "127.0.0.1",
            "AIT_SITE_PREVIEW_PORT": str(preview_port),
        }
    )

    preview = subprocess.Popen(
        ["bash", "ait.sh", "site", "preview"],
        cwd=ROOT,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        deadline = time.time() + 15
        body = ""
        while time.time() < deadline:
            if preview.poll() is not None:
                output = preview.stdout.read() if preview.stdout is not None else ""
                raise AssertionError(f"preview helper exited early:\n{output}")
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{preview_port}/learn/", timeout=1) as response:
                    body = response.read().decode("utf-8")
                    break
            except (urllib.error.URLError, TimeoutError):
                time.sleep(0.2)
        else:
            output = preview.stdout.read() if preview.stdout is not None else ""
            raise AssertionError(f"preview helper never served the site:\n{output}")

        assert "Learn — ait" in body
        assert preview_dir.joinpath("index.html").exists()
        assert preview_dir.joinpath("public", "styles.css").exists()
        assert preview_dir.joinpath("robots.txt").exists()
        assert preview_dir.joinpath("sitemap.xml").exists()
    finally:
        preview.terminate()
        try:
            preview.wait(timeout=5)
        except subprocess.TimeoutExpired:
            preview.kill()
            preview.wait(timeout=5)
