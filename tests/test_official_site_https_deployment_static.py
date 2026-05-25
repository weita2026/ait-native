from __future__ import annotations

import subprocess
from pathlib import Path

from ait.repo_paths import RepoContext

ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(ROOT).repo_root
DEPLOY = ROOT / "deploy" / "site"
AUTHORED_DEPLOY = AUTHORED_ROOT / "deploy" / "site"
SCRIPT = ROOT / "scripts" / "official_site_https.py"


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_official_site_helper_keeps_local_nginx_assets_and_avoids_docker_operator_routing() -> None:
    expected = [
        DEPLOY / "macos-nginx" / "site.env.example",
        SCRIPT,
    ]
    for path in expected:
        assert path.exists(), path

    unexpected = [
        DEPLOY / ".env.example",
        DEPLOY / "docker-compose.yml",
        DEPLOY / "Caddyfile",
    ]
    for path in unexpected:
        assert not path.exists(), path

    script_text = _read(SCRIPT)
    assert "../ait_docker" not in script_text
    assert "ait-docker.sh" not in script_text
    assert "macos-nginx" in script_text


def test_official_site_https_helper_builds_static_output_and_validates_env(tmp_path: Path):
    output = tmp_path / "site-dist"
    build = subprocess.run(
        ["python3", str(SCRIPT), "build", "--output", str(output)],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    assert "Built" in build.stdout
    assert output.joinpath("index.html").exists()
    assert output.joinpath("public", "styles.css").exists()

    env_file = tmp_path / ".env"
    env_file.write_text(
        "\n".join(
            [
                "SITE_DOMAIN=ait-native.dev",
                "SITE_EMAIL=ops@ait-native.dev",
                f"SITE_BUILD_DIR={output}",
                "SITE_HTTP_PORT=80",
                "SITE_HTTPS_PORT=443",
                "SITE_ACME_CA=https://acme-v02.api.letsencrypt.org/directory",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    doctor = subprocess.run(
        ["python3", str(SCRIPT), "doctor", "--env-file", str(env_file)],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    assert doctor.returncode == 0, doctor.stdout + doctor.stderr
    assert "[OK] SITE_DOMAIN: ait-native.dev" in doctor.stdout
    assert f"[OK] SITE_BUILD_DIR: {output}" in doctor.stdout
    assert "[INFO] Live issuance still requires inbound reachability on ports 80 and 443 plus correct DNS." in doctor.stdout
    assert "../ait_docker" not in doctor.stdout
    assert "Docker" not in doctor.stdout
    assert "operator-managed workspace outside this repository checkout" in doctor.stdout
