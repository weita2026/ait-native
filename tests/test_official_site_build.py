from __future__ import annotations

import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BUILD_PATH = ROOT / "site" / "build.py"


def load_build_module():
    spec = importlib.util.spec_from_file_location("official_site_build", BUILD_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(module)
    return module


def test_build_site_renders_first_public_routes(tmp_path):
    build = load_build_module()
    built = build.build_site(tmp_path)

    assert tmp_path.joinpath("index.html").exists()
    assert tmp_path.joinpath("learn", "index.html").exists()
    assert tmp_path.joinpath("prompt-patterns", "index.html").exists()
    assert tmp_path.joinpath("local-quickstart", "index.html").exists()
    assert tmp_path.joinpath("workflow-doctrine", "index.html").exists()
    assert tmp_path.joinpath("docs", "index.html").exists()
    assert tmp_path.joinpath("self-hosted", "index.html").exists()
    assert tmp_path.joinpath("services", "index.html").exists()
    assert tmp_path.joinpath("proof", "index.html").exists()
    assert tmp_path.joinpath("public", "styles.css").exists()
    assert len(built) == len(build.PAGES)

    home = tmp_path.joinpath("index.html").read_text()
    learn = tmp_path.joinpath("learn", "index.html").read_text()
    doctrine = tmp_path.joinpath("workflow-doctrine", "index.html").read_text()
    docs = tmp_path.joinpath("docs", "index.html").read_text()
    self_hosted = tmp_path.joinpath("self-hosted", "index.html").read_text()
    services = tmp_path.joinpath("services", "index.html").read_text()
    proof = tmp_path.joinpath("proof", "index.html").read_text()

    assert "Watch the workflow" in home
    assert ">Learn</a>" in home
    assert ">Docs</a>" in home
    assert ">Proof</a>" in home
    assert "Switch scene" in home
    assert "homepage_plan_story.md" in home
    assert "../public/styles.css" in learn
    assert "What you are learning" in learn
    assert "Read the Workflow Doctrine" in learn
    assert "Why the workflow is agent-mediated" in doctrine
    assert "Start with the right document" in docs
    assert "Use self-hosting when the work needs a shared control plane." in self_hosted
    assert "Services at a glance" in services
    assert "Keep proof explicit and measured." in proof
