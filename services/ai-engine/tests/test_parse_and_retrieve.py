"""
TDD Test Suite for Slice 1: Thesis Engine Parse & Retrieve
These tests define the behavioral contract BEFORE implementation code is written.
"""

from tests.fixtures import fixture_registry
import pytest
from pydantic import ValidationError


def test_contract_imports_will_exist():
    """
    Sanity test checking that the target modules are importable once implemented.
    """
    try:
        from src.models import PolicyDraft, AssetCandidate, SleeveType
        from src.parser import parse_thesis
        from src.retriever import retrieve_candidates, AssetRegistry
        assert True
    except ImportError as e:
        pytest.fail(f"Slice 1 interfaces not yet implemented or misconfigured: {e}")


def test_t01_standard_treasury_runway_parsing():
    """
    T01: Standard treasury runway thesis
    Input: "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    Expectation: Parses into USD 0.7, EUR 0.2, cash floor >= 0.1, conservative risk tolerance.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry

    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")

    assert policy.currency_exposure.get("USD") == 0.70
    assert policy.currency_exposure.get("EUR") == 0.20
    assert policy.min_liquidity_sleeve >= 0.10
    assert policy.risk_tolerance == "conservative"
    assert policy.jurisdiction == "EU"
    assert policy.investor_class == "professional"

    # Verify retrieval against registered assets
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)

    candidate_ids = [c.asset_id for c in candidates]
    # In EU for a professional investor, Spiko EUR and USD T-bills + cash USDC/EURC should be retrieved
    assert "USDC" in candidate_ids
    assert "EURC" in candidate_ids
    assert any("SPIKO" in cid or "BENJI" in cid for cid in candidate_ids)


def test_t02_closed_universe_blocks_hallucinated_assets():
    """
    T02: Closed Universe Rule enforcement
    Input: User asks for an unregistered stock/crypto like TSLA or BTC.
    Expectation: TSLA/BTC NEVER appears in candidates. Ambiguity/mismatch is explicitly flagged.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry

    thesis = "Put 50% in TSLA stock and 50% in Bitcoin on Stellar."
    policy = parse_thesis(thesis, jurisdiction="US", investor_class="retail")

    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)

    candidate_ids = [c.asset_id for c in candidates]
    assert "TSLA" not in candidate_ids
    assert "BTC" not in candidate_ids

    # The parser or retriever must flag that requested assets were unsupported/unavailable
    assert len(policy.ambiguities) > 0 or len(candidates) == 0 or all(c.permissionless for c in candidates)


def test_t03_retail_investor_eligibility_gating():
    """
    T03: Retail user jurisdiction & accreditation gating
    Input: Retail investor in US.
    Expectation: Gated institutional assets (JQQQ, Tradable private credit) are stripped.
                 Only eligible permissionless assets or US-retail eligible funds remain.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry

    thesis = "Invest 100% in high yield equities and tech funds."
    policy = parse_thesis(thesis, jurisdiction="US", investor_class="retail")

    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)

    for c in candidates:
        # D-B: retail users get permissionless assets only.
        assert c.permissionless, f"Gated asset {c.asset_id} leaked to retail"
        assert "US" not in c.blocked_jurisdictions


def test_t04_ambiguous_thesis_conservative_fallback():
    """
    T04: Ambiguous or minimal thesis
    Input: "I want yield."
    Expectation: Default conservative parameters, low confidence scores, and logged ambiguities.
    """
    from src.parser import parse_thesis

    thesis = "I want yield."
    policy = parse_thesis(thesis, jurisdiction="non-US", investor_class="retail")

    # Defaults to safe cash + short-term yield
    assert policy.min_liquidity_sleeve >= 0.05
    assert len(policy.ambiguities) > 0
    assert any(score < 0.8 for score in policy.confidence.values())
