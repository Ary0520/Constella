"""
TDD Test Suite for Slice 3: Thesis Engine Explain & Sign (Steps 5 & 6)
Written FIRST before implementation code is created.
"""

from tests.fixtures import fixture_registry
import pytest
import json
import hashlib


def test_slice_3_imports_will_exist():
    """
    Sanity test confirming that Slice 3 modules and data contracts are importable.
    """
    try:
        from src.models import (
            DiffItem,
            ExplanationReport,
            CanonicalSleeve,
            CanonicalConstraints,
            CanonicalEligibility,
            CanonicalAdmin,
            CanonicalPolicy,
            SignedPolicyEnvelope,
        )
        from src.explainer import generate_explanation
        from src.signer import (
            assemble_canonical_policy,
            to_canonical_json,
            compute_policy_hash,
            prepare_signing_envelope,
        )
        assert True
    except ImportError as e:
        pytest.fail(f"Slice 3 interfaces not yet implemented or misconfigured: {e}")


def test_t14_asset_rationales_and_diff():
    """
    T14: Asset Rationales & Diff Generation
    Verifies:
    1. Every allocated asset has a non-empty, plain-language rationale.
    2. Any requested currency or asset divergence (e.g. cash buffer, blocked assets) produces clear DiffItems.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.explainer import generate_explanation

    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)

    report = generate_explanation(thesis, policy, opt_result, candidates)

    # 1. Every asset with non-zero weight has a rationale
    for asset_id, weight in opt_result.weights.items():
        if weight > 0:
            assert asset_id in report.asset_rationales
            assert len(report.asset_rationales[asset_id]) > 10

    # 2. Diff exists explaining allocations
    assert len(report.thesis_diff) > 0


def test_t15_concentration_warning_trigger():
    """
    T15: Concentration Warning Trigger
    Verifies:
    Any issuer with total allocation >= 25% (2500 bps) generates an explicit concentration risk warning.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.explainer import generate_explanation

    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)

    report = generate_explanation(thesis, policy, opt_result, candidates)

    # Check if any issuer >= 0.25 triggered a warning
    heavy_issuers = [issuer for issuer, weight in opt_result.issuer_weights.items() if weight >= 0.249]
    if heavy_issuers:
        assert len(report.issuer_concentration_warnings) > 0
        for issuer in heavy_issuers:
            assert any(issuer in w for w in report.issuer_concentration_warnings)


def test_t16_canonical_json_determinism():
    """
    T16: Canonical JSON Determinism (RFC 8785)
    Verifies that dictionary ordering does not alter the output JSON string or its SHA-256 hash.
    """
    from src.models import (
        CanonicalSleeve,
        CanonicalConstraints,
        CanonicalEligibility,
        CanonicalAdmin,
        CanonicalPolicy,
    )
    from src.signer import to_canonical_json, compute_policy_hash

    sleeves = [
        CanonicalSleeve(
            asset_id="USDC",
            contract_address="CUSDC1234567890",
            target_weight_bps=1000,
            band_bps=200,
            sleeve_type="cash",
        ),
        CanonicalSleeve(
            asset_id="FRANKLIN_BENJI",
            contract_address="CBENJI1234567890",
            target_weight_bps=4500,
            band_bps=500,
            sleeve_type="usd_gov_fund",
        ),
    ]

    constraints = CanonicalConstraints(
        max_issuer_weight_bps=3000,
        max_single_asset_weight_bps=2500,
        min_liquidity_sleeve_bps=1000,
        max_slippage_bps=50,
        rebalance_band_bps=500,
    )

    eligibility = CanonicalEligibility(
        jurisdiction="non-US",
        investor_class="professional",
        share_class="open",
    )

    admin = CanonicalAdmin(
        admin_address="GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR",
        multisig_threshold=2,
        multisig_signers=["GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR", "GCATS5YOVB6ROX2WUNKGNQ2MP3GMXDMKSG2O4N5CLX3A6W4PZGZZI55U"],
    )

    policy = CanonicalPolicy(
        schema_version="1.0",
        created_at="2026-11-14T00:00:00Z",
        basket_name="cRUNWAY",
        numeraire="USDC",
        sleeves=sleeves,
        constraints=constraints,
        eligibility=eligibility,
        admin=admin,
    )

    json_str_1 = to_canonical_json(policy)
    json_str_2 = to_canonical_json(policy)
    assert json_str_1 == json_str_2

    # Verifies compact formatting (no extra whitespace around : or ,)
    assert ": " not in json_str_1
    assert ", " not in json_str_1

    # Verifies key sorting (e.g. admin before basket_name, basket_name before constraints)
    parsed = json.loads(json_str_1)
    keys = list(parsed.keys())
    assert keys == sorted(keys)

    # Hash determinism
    hex_1, bytes_1 = compute_policy_hash(policy)
    hex_2, bytes_2 = compute_policy_hash(policy)
    assert hex_1 == hex_2
    assert bytes_1 == bytes_2


def test_t17_policy_hash_bytes_and_hex():
    """
    T17: SHA-256 Policy Hash Format
    Verifies that the policy hash is exactly 32 bytes and 64 hex characters, matching Soroban BytesN<32>.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.signer import assemble_canonical_policy, prepare_signing_envelope

    thesis = "Park runway."
    policy = parse_thesis(thesis, jurisdiction="non-US", investor_class="retail")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)

    canonical = assemble_canonical_policy(
        policy_draft=policy,
        opt_result=opt_result,
        candidates=candidates,
        basket_name="cSAFE",
        owner="GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR",
        multisig_signers=["GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR"],
        multisig_threshold=1,
    )

    envelope = prepare_signing_envelope(canonical)

    assert len(envelope.policy_hash_bytes) == 32, f"Expected 32 bytes, got {len(envelope.policy_hash_bytes)}"
    assert len(envelope.policy_hash_hex) == 64, f"Expected 64 hex chars, got {len(envelope.policy_hash_hex)}"
    assert envelope.policy_hash_hex == hashlib.sha256(envelope.canonical_json.encode("utf-8")).hexdigest()


def test_t18_full_pipeline_end_to_end():
    """
    T18: End-to-End Pipeline Sanity (Steps 1 to 6)
    Runs a user thesis seamlessly from Parse -> Retrieve -> Optimise -> Simulate -> Explain -> Sign.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.simulator import simulate_portfolio
    from src.explainer import generate_explanation
    from src.signer import assemble_canonical_policy, prepare_signing_envelope

    # 1. Parse
    thesis = "Hold 80% USD Treasury and 20% EUR sovereign with 10% instant cash liquidity for DAO runway."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    assert policy.currency_exposure["USD"] == 0.80

    # 2. Retrieve
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    assert len(candidates) >= 3

    # 3. Optimise
    opt_result = optimize_portfolio(policy, candidates)
    assert sum(opt_result.weights_bps.values()) == 10000

    # 4. Simulate
    sim_report = simulate_portfolio(opt_result.weights, candidates, policy)
    assert sim_report.redemption_ladder.instant_pct >= 0.10
    assert "Past performance does not predict future returns" in sim_report.disclaimer

    # 5. Explain
    explanation = generate_explanation(thesis, policy, opt_result, candidates)
    assert len(explanation.asset_rationales) > 0

    # 6. Sign
    canonical = assemble_canonical_policy(
        policy_draft=policy,
        opt_result=opt_result,
        candidates=candidates,
        basket_name="cDAO_RUNWAY",
        owner="GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR",
        multisig_signers=["GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR", "GCATS5YOVB6ROX2WUNKGNQ2MP3GMXDMKSG2O4N5CLX3A6W4PZGZZI55U"],
        multisig_threshold=2,
    )
    envelope = prepare_signing_envelope(canonical)

    assert envelope.policy.basket_name == "cDAO_RUNWAY"
    assert len(envelope.policy_hash_bytes) == 32
    assert len(envelope.policy.sleeves) > 0


def test_t19_ambiguity_clarifications_and_unsupported_assets():
    """
    T19: Ambiguity Clarifications & Unsupported Assets
    Verifies that ambiguities and unsupported assets in PolicyDraft produce
    explicit clarifications and diff items in ExplanationReport.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.explainer import generate_explanation

    thesis = "Allocate 50% to TSLA stock, 30% to BTC, and 20% to safe dollar T-bills."
    policy = parse_thesis(thesis, jurisdiction="non-US", investor_class="retail")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)

    report = generate_explanation(thesis, policy, opt_result, candidates)

    # Policy should have flagged ambiguities for unsupported assets TSLA and BTC
    assert len(policy.ambiguities) > 0
    assert len(report.ambiguity_clarifications) > 0
    assert any("TSLA" in c or "stock" in c for c in report.ambiguity_clarifications)
    assert any(item.category == "ambiguity" for item in report.thesis_diff)


def test_t20_canonical_policy_sleeve_sorting_determinism():
    """
    T20: Deterministic Sleeve Sorting
    Verifies that assemble_canonical_policy always produces sleeves sorted by asset_id
    regardless of input dictionary iteration order.
    """
    from src.models import PolicyDraft, AssetCandidate, OptimizationResult, SleeveType
    from src.signer import assemble_canonical_policy, to_canonical_json

    policy_draft = PolicyDraft(
        objectives=["runway"],
        currency_exposure={"USD": 1.0},
        min_liquidity_sleeve=0.10,
        max_issuer_weight=0.30,
        max_single_asset_weight=0.25,
    )

    candidates = [
        AssetCandidate(
            asset_id="USDC",
            display_name="USD Coin",
            issuer="Circle",
            sleeve_type=SleeveType.CASH_STABLE,
            currency="USD",
            contract_address="CUSDC",
        ),
        AssetCandidate(
            asset_id="FRANKLIN_BENJI",
            display_name="BENJI",
            issuer="Franklin Templeton",
            sleeve_type=SleeveType.USD_GOV_FUNDS,
            currency="USD",
            contract_address="CBENJI",
        ),
        AssetCandidate(
            asset_id="SPIKO_USD",
            display_name="Spiko USD",
            issuer="Spiko",
            sleeve_type=SleeveType.USD_GOV_FUNDS,
            currency="USD",
            contract_address="CSPIKO",
        ),
    ]

    # Deliberately reverse the order of keys in weights_bps
    opt_result_1 = OptimizationResult(
        weights={"USDC": 0.20, "FRANKLIN_BENJI": 0.40, "SPIKO_USD": 0.40},
        weights_bps={"USDC": 2000, "FRANKLIN_BENJI": 4000, "SPIKO_USD": 4000},
        sleeve_weights={"cash": 0.20, "usd_gov_fund": 0.80},
        currency_weights={"USD": 1.0},
        issuer_weights={"Circle": 0.20, "Franklin Templeton": 0.40, "Spiko": 0.40},
        algorithm_used="HRP",
    )

    opt_result_2 = OptimizationResult(
        weights={"SPIKO_USD": 0.40, "USDC": 0.20, "FRANKLIN_BENJI": 0.40},
        weights_bps={"SPIKO_USD": 4000, "USDC": 2000, "FRANKLIN_BENJI": 4000},
        sleeve_weights={"cash": 0.20, "usd_gov_fund": 0.80},
        currency_weights={"USD": 1.0},
        issuer_weights={"Spiko": 0.40, "Circle": 0.20, "Franklin Templeton": 0.40},
        algorithm_used="HRP",
    )

    p1 = assemble_canonical_policy(policy_draft, opt_result_1, candidates, owner="GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR", created_at="2026-11-14T00:00:00Z")
    p2 = assemble_canonical_policy(policy_draft, opt_result_2, candidates, owner="GCFIRY65OQE7DFP5KLNS2PF2LVZMUZYJX4OZIEQ36N2IQANUB5XVYOJR", created_at="2026-11-14T00:00:00Z")

    assert to_canonical_json(p1) == to_canonical_json(p2)
    assert [s.asset_id for s in p1.sleeves] == ["FRANKLIN_BENJI", "SPIKO_USD", "USDC"]

