from __future__ import annotations

import json
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = REPO_ROOT / "docs" / "public_whitepaper_patent_gate_contract.json"
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


def test_public_whitepaper_patent_gate_contract_shape() -> None:
    contract = load_contract()

    assert contract["schema_version"] == 1
    assert contract["gate_doc_path"] == "docs/WHITEPAPER_PATENT_GATE.md"
    assert contract["doctrine_path"] == "docs/PUBLIC_DOCTRINE.md"
    assert contract["whitepaper_draft_path"] == "docs/AIT_WHITEPAPER_DRAFT.md"
    assert contract["current_publication_state"] == "draft_only_pending_patent_gate"
    assert contract["repo_release_facing_draft_allowed"] is True
    assert contract["broader_public_whitepaper_release_allowed"] is False
    assert contract["broader_public_doctrine_repackaging_allowed"] is False
    assert contract["filing_order_review_required"] is True
    assert contract["claims_governance_review_required"] is True
    assert contract["explicit_approval_record_required"] is True

    assert contract["allowed_repo_draft_surfaces"] == [
        "README.md",
        "docs/PUBLIC_DOCTRINE.md",
        "docs/AIT_WHITEPAPER_DRAFT.md",
        "docs/WHITEPAPER_PATENT_GATE.md",
    ]
    assert contract["required_gate_decisions"] == [
        "patent_sensitive_content_review",
        "filing_order_decision",
        "claims_governance_review",
        "publication_scope_decision",
        "redaction_or_deferral_decision_when_needed",
    ]
    assert contract["manual_clearance_fields"] == [
        "approval_date",
        "approver_role",
        "publication_scope",
        "filing_order_status",
        "redaction_required",
        "notes_ref",
    ]
    assert contract["claim_boundary_reminders"] == [
        "no_generalized_token_savings_guarantee",
        "no_product_wide_one_hour_success_promise",
        "no_unreviewed_security_or_compliance_claim",
        "no_new_package_or_license_promise",
    ]


def test_public_whitepaper_patent_gate_docs_are_linked_and_explicit() -> None:
    contract = load_contract()
    gate_path = public_doc_path(contract["gate_doc_path"])
    doctrine_path = public_doc_path(contract["doctrine_path"])
    whitepaper_path = public_doc_path(contract["whitepaper_draft_path"])
    readme_path = public_doc_path("README.md")

    gate_doc = gate_path.read_text(encoding="utf-8")
    doctrine_doc = doctrine_path.read_text(encoding="utf-8")
    whitepaper_doc = whitepaper_path.read_text(encoding="utf-8")
    readme_doc = readme_path.read_text(encoding="utf-8")
    gate_doc_lower = gate_doc.lower()

    assert "draft_only_pending_patent_gate" in gate_doc
    assert "repo-contained release-facing draft visibility is allowed" in gate_doc_lower
    assert "broader public whitepaper promotion remains blocked" in gate_doc_lower
    assert "docs/legal/patent_license_policy.md" in gate_doc
    assert "approval date" in gate_doc
    assert "no generalized token-savings guarantee" in gate_doc

    assert "WHITEPAPER_PATENT_GATE.md" in doctrine_doc
    assert "WHITEPAPER_PATENT_GATE.md" in whitepaper_doc
    assert "WHITEPAPER_PATENT_GATE.md" in readme_doc

    assert contract["gate_doc_path"] == public_doc_contract_path(gate_path)
    assert contract["doctrine_path"] == public_doc_contract_path(doctrine_path)
    assert contract["whitepaper_draft_path"] == public_doc_contract_path(whitepaper_path)
