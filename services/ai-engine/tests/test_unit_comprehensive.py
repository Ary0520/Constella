"""
Comprehensive unit tests for Slice 1: Models, Parser, and Retriever.
Verifies boundary conditions, schema validation, and edge cases.
"""

from tests.fixtures import fixture_registry
import pytest
from pydantic import ValidationError
from src.models import PolicyDraft, AssetCandidate, SleeveType
from src.parser import parse_thesis
from src.retriever import AssetRegistry, retrieve_candidates


def test_policy_draft_bounds_validation():
    # Valid model instantiation
    draft = PolicyDraft(
        max_issuer_weight=0.30,
        max_single_asset_weight=0.25,
        min_liquidity_sleeve=0.10,
        risk_tolerance="conservative",
        investor_class="retail",
    )
    assert draft.max_issuer_weight == 0.30

    # Invalid max_issuer_weight > 1.0
    with pytest.raises(ValidationError):
        PolicyDraft(max_issuer_weight=1.5)

    # Invalid min_liquidity_sleeve < 0.0
    with pytest.raises(ValidationError):
        PolicyDraft(min_liquidity_sleeve=-0.1)

    # Invalid risk_tolerance
    with pytest.raises(ValidationError):
        PolicyDraft(risk_tolerance="gambling")

    # Invalid investor_class
    with pytest.raises(ValidationError):
        PolicyDraft(investor_class="anonymous")


def test_asset_registry_lookups():
    registry = fixture_registry()
    assets = registry.list_assets()
    assert len(assets) == 8

    usdc = registry.get_asset("USDC")
    assert usdc is not None
    assert usdc.permissionless is True
    assert usdc.currency == "USD"
    assert usdc.liquidity_tier == 0

    jqqq = registry.get_asset("JH_JQQQ")
    assert jqqq is not None
    assert jqqq.investor_classes_required == ["professional"]
    assert "US" in jqqq.blocked_jurisdictions


def test_institutional_investor_access():
    registry = fixture_registry()
    # Institutional investor in EU should have access to JH_JQQQ (equity)
    policy = PolicyDraft(
        jurisdiction="EU",
        investor_class="institutional",
        currency_exposure={"USD": 1.0},
        exclusions=[],
    )
    candidates = retrieve_candidates(policy, registry)
    candidate_ids = [c.asset_id for c in candidates]
    assert "JH_JQQQ" in candidate_ids
    assert "FRANKLIN_BENJI" in candidate_ids
    assert "USDC" in candidate_ids


def test_exclusions_filtering():
    registry = fixture_registry()
    # Explicitly exclude lending and equity
    policy = PolicyDraft(
        jurisdiction="EU",
        investor_class="professional",
        exclusions=["lending", "equity"],
        currency_exposure={"USD": 1.0},
    )
    candidates = retrieve_candidates(policy, registry)
    candidate_ids = [c.asset_id for c in candidates]

    assert "BLEND_USDC_SUPPLY" not in candidate_ids
    assert "JH_JQQQ" not in candidate_ids
    assert "USDC" in candidate_ids


def test_parser_detects_multiple_hallucinations():
    thesis = "I want 40% in Tesla, 30% in Ethereum, and 30% in Apple stock."
    policy = parse_thesis(thesis, jurisdiction="US", investor_class="retail")

    assert len(policy.ambiguities) >= 3
    ambiguity_text = " ".join(policy.ambiguities).lower()
    assert "tesla" in ambiguity_text
    assert "ethereum" in ambiguity_text
    assert "apple" in ambiguity_text
    assert any(score < 0.8 for score in policy.confidence.values())
