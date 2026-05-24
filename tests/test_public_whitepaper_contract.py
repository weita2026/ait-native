from __future__ import annotations

import json
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = REPO_ROOT / "docs" / "public_whitepaper_contract.json"
WORKTREE_METADATA_PATH = REPO_ROOT / ".ait-worktree.json"


def load_contract() -> dict:
    return json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))


def public_doc_path(name: str) -> Path:
    local_path = REPO_ROOT / name
    if local_path.exists():
        return local_path
    if WORKTREE_METADATA_PATH.exists():
        metadata = json.loads(WORKTREE_METADATA_PATH.read_text(encoding="utf-8"))
        source_root = Path(metadata["repo_root"])
        source_path = source_root / name
        if source_path.exists():
            return source_path
    raise FileNotFoundError(name)


def public_doc_contract_path(path: Path) -> str:
    try:
        return path.relative_to(REPO_ROOT).as_posix()
    except ValueError:
        if WORKTREE_METADATA_PATH.exists():
            metadata = json.loads(WORKTREE_METADATA_PATH.read_text(encoding="utf-8"))
            source_root = Path(metadata["repo_root"])
            return path.relative_to(source_root).as_posix()
        raise


def test_public_whitepaper_contract_shape_and_boundary() -> None:
    contract = load_contract()

    assert contract["schema_version"] == 1
    assert contract["doctrine_path"] == "docs/PUBLIC_DOCTRINE.md"
    assert contract["whitepaper_draft_path"] == "docs/AIT_WHITEPAPER_DRAFT.md"
    assert contract["publication_state"] == "draft_only_pending_patent_gate"
    assert contract["requires_private_execution_plan_context"] is False
    assert contract["patent_gate_ref"] == "engineering/release/whitepaper-patent-gate"

    assert contract["required_claims"] == [
        "constitutional_legal_command_layer_split",
        "agent_first_md_task_land_contract",
        "local_only_vs_self_hosted_boundary",
        "package_and_license_boundary_awareness",
        "public_demo_data_as_bounded_proof_point",
    ]
    assert contract["excluded_claims"] == [
        "final_publication_clearance",
        "patent_sensitive_scheduler_detail",
        "new_package_or_license_promise",
        "generalized_token_savings_claim",
    ]
    assert contract["recommended_read_order"] == [
        "README.md",
        "docs/PUBLIC_DOCTRINE.md",
        "docs/LOCAL_QUICKSTART.md",
        "docs/SELF_HOSTED_TEAM_DEPLOYMENT.md",
        "docs/COMPATIBILITY_MATRIX.md",
        "docs/PACKAGE_TARGETS.md",
        "docs/PUBLIC_DEMO_DATA.md",
        "docs/AIT_WHITEPAPER_DRAFT.md",
    ]


def test_public_whitepaper_docs_include_required_operator_narrative() -> None:
    contract = load_contract()
    doctrine_path = public_doc_path("docs/PUBLIC_DOCTRINE.md")
    whitepaper_path = public_doc_path("docs/AIT_WHITEPAPER_DRAFT.md")
    doctrine = doctrine_path.read_text(encoding="utf-8")
    whitepaper = whitepaper_path.read_text(encoding="utf-8")

    assert "constitutional layer" in doctrine
    assert "legal layer" in doctrine
    assert "command layer" in doctrine
    assert "md → task → land" in doctrine
    assert "local-only" in doctrine
    assert "self-hosted" in doctrine
    assert "PACKAGE_TARGETS.md" in doctrine
    assert "PUBLIC_DEMO_DATA.md" in doctrine

    assert "draft_only_pending_patent_gate" in whitepaper
    assert "agent-mediated" in whitepaper
    assert "local trust layer" in whitepaper
    assert "shared control-plane" in whitepaper
    assert "PUBLIC_DEMO_DATA.md" in whitepaper
    assert "generalized token-savings guarantee" in whitepaper

    assert contract["doctrine_path"] == public_doc_contract_path(doctrine_path)
    assert contract["whitepaper_draft_path"] == public_doc_contract_path(whitepaper_path)
