"""
TDD Test Suite for Slice 2: Thesis Engine Optimise & Simulate (Steps 3 & 4)
Written FIRST before implementation code is created.
"""

from tests.fixtures import fixture_registry
import pytest
import pandas as pd
import numpy as np


def test_slice_2_imports_will_exist():
    """
    Sanity test confirming that Slice 2 modules and data contracts are importable.
    """
    try:
        from src.models import (
            OptimizationResult,
            RedemptionLadder,
            StressScenario,
            SimulationReport,
        )
        from src.optimizer import optimize_portfolio
        from src.simulator import simulate_portfolio
        assert True
    except ImportError as e:
        pytest.fail(f"Slice 2 interfaces not yet implemented or misconfigured: {e}")


def test_t05_hrp_and_constrained_weights_sum_and_caps():
    """
    T05: HRP / Constrained Weights Sum & Cap Enforcement
    Verifies:
    1. Sum of weights == 1.0 (within float tolerance) and sum of weights_bps == 10000.
    2. No single issuer exceeds policy max_issuer_weight (default 0.30 / 3000 bps).
    3. No single asset exceeds policy max_single_asset_weight (default 0.25 / 2500 bps).
    4. Cash sleeve (USDC/EURC) meets or exceeds policy min_liquidity_sleeve (0.10 / 1000 bps).
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio

    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)

    result = optimize_portfolio(policy, candidates)

    # 1. Weights sum to 1.0 / 10,000 bps
    total_weight = sum(result.weights.values())
    assert abs(total_weight - 1.0) < 1e-4, f"Weights sum to {total_weight}, expected 1.0"
    assert sum(result.weights_bps.values()) == 10000, f"Weights bps sum to {sum(result.weights_bps.values())}"

    # D-A (owner, 17 Nov 2026): the issuer and single-asset caps apply to every
    # asset EXCEPT the vault currency (USDC); Cash Reserve keeps USDC >= 85 %
    # and lending <= 15 %. Checked exactly, integer bps.
    from src.optimizer import check_allocation
    assert check_allocation(result.weights_bps, policy, candidates, "USDC") == []
    by = {c.asset_id: c for c in candidates}
    non_num_issuers = {}
    for asset_id, weight in result.weights.items():
        if asset_id == "USDC":
            continue
        assert weight <= policy.max_single_asset_weight + 1e-9, f"{asset_id} above the single-asset cap"
        non_num_issuers[by[asset_id].issuer] = non_num_issuers.get(by[asset_id].issuer, 0.0) + weight
    for issuer, weight in non_num_issuers.items():
        assert weight <= policy.max_issuer_weight + 1e-9, f"issuer {issuer} {weight} above the cap (vault currency excluded)"
    assert result.weights["USDC"] >= policy.min_liquidity_sleeve - 1e-9  # D-P: the thesis's cash floor, not a fixed 85 %

    # 4. Liquidity floor (>= 10%)
    cash_weight = result.sleeve_weights.get("cash", 0.0)
    assert cash_weight >= policy.min_liquidity_sleeve - 1e-4, f"Cash sleeve {cash_weight} below floor {policy.min_liquidity_sleeve}"


def test_t06_currency_matching_constraint():
    """
    T06: Currency Matching Constraint
    Input: Runway thesis with 70% USD / 20% EUR target exposure.
    Verifies: Portfolio currency allocation satisfies the target proportions within +/- 5% tolerance.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio

    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)

    result = optimize_portfolio(policy, candidates)

    # USD assets should be close to 70-80% (including USD cash), EUR assets should be ~20%
    usd_weight = result.currency_weights.get("USD", 0.0)
    eur_weight = result.currency_weights.get("EUR", 0.0)

    assert usd_weight >= 0.65, f"USD currency weight {usd_weight} lower than expected"
    assert eur_weight >= 0.15, f"EUR currency weight {eur_weight} lower than expected"


def test_t07_redemption_ladder_breakdown():
    """
    T07: Redemption Ladder Horizon Breakdown
    Verifies that the simulation report accurately aggregates constituent liquidity tiers:
    - Tier 0 -> instant_pct (< 1 hour)
    - Tier 1 -> within_24h_pct (24 hours)
    - Tier 2 -> within_1w_pct (1 week)
    - Tier 3 -> illiquid_pct
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.simulator import simulate_portfolio

    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)

    sim_report = simulate_portfolio(opt_result.weights, candidates, policy)

    ladder = sim_report.redemption_ladder
    total_ladder = ladder.instant_pct + ladder.within_24h_pct + ladder.within_1w_pct + ladder.illiquid_pct
    assert abs(total_ladder - 1.0) < 1e-4, f"Ladder sum {total_ladder} != 1.0"

    # D-P (step 4): the founding doc's journey 1 is a T-bill-heavy basket again
    # - the cash floor is instant, the T-bill funds redeem within 24h.
    assert ladder.instant_pct >= policy.min_liquidity_sleeve - 1e-9
    assert ladder.within_24h_pct >= 0.60
    assert ladder.illiquid_pct == 0.0


def test_t08_stress_simulations_depeg_and_freeze():
    """
    T08: Depeg & Issuer Freeze Stress Simulations
    Verifies:
    1. 5% depeg on stablecoins computes the proper negative NAV drawdown impact.
    2. 30-day issuer freeze correctly identifies the trapped liquidity percentage for the largest issuer.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.simulator import simulate_portfolio

    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)

    sim_report = simulate_portfolio(opt_result.weights, candidates, policy)

    # Depeg stress: should be negative NAV impact
    assert sim_report.depeg_stress.nav_impact_pct < 0.0
    assert sim_report.depeg_stress.nav_impact_pct >= -0.05  # cannot exceed 5% total drawdown

    # Issuer freeze hits the LARGEST issuer, vault currency included (the issuer
    # cap exempts USDC, the risk does not go away - D-A disclosure).
    top, weight = max(opt_result.issuer_weights.items(), key=lambda kv: kv[1])
    assert opt_result.issuer_weights["CIRCLE"] >= policy.min_liquidity_sleeve - 1e-9
    assert abs(sim_report.issuer_freeze_stress.trapped_liquidity_pct - weight) < 1e-4
    assert top in sim_report.issuer_freeze_stress.description


def test_t09_mandatory_legal_disclaimer():
    """
    T09: Mandatory Legal Simulation Disclaimer (Rule 10)
    Verifies that the exact legal string required by Rule 10 is present in the report.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.simulator import simulate_portfolio

    thesis = "Park runway."
    policy = parse_thesis(thesis, jurisdiction="non-US", investor_class="retail")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)

    sim_report = simulate_portfolio(opt_result.weights, candidates, policy)

    expected_disclaimer = "This simulation uses historical data. Past performance does not predict future returns. This is not financial advice."
    assert expected_disclaimer in sim_report.disclaimer


def test_t10_hrp_with_historical_returns_dataframe():
    """
    T10: Hierarchical Risk Parity (HRP) execution when historical returns are provided.
    Verifies that algorithm_used is 'HRP', and weights/bps still satisfy all hard policy bounds.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio

    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)

    np.random.seed(42)
    candidate_ids = [c.asset_id for c in candidates]
    rets = pd.DataFrame(
        np.random.normal(0.0002, 0.005, (100, len(candidate_ids))),
        columns=candidate_ids,
    )

    result = optimize_portfolio(policy, candidates, historical_returns=rets)

    assert result.algorithm_used == "HRP"
    assert abs(sum(result.weights.values()) - 1.0) < 1e-4
    assert sum(result.weights_bps.values()) == 10000

    # Hard rules still hold exactly under the HRP refinement.
    # D-A (owner, 17 Nov 2026): the issuer and single-asset caps apply to every
    # asset EXCEPT the vault currency (USDC); Cash Reserve keeps USDC >= 85 %
    # and lending <= 15 %. Checked exactly, integer bps.
    from src.optimizer import check_allocation
    assert check_allocation(result.weights_bps, policy, candidates, "USDC") == []
    by = {c.asset_id: c for c in candidates}
    non_num_issuers = {}
    for asset_id, weight in result.weights.items():
        if asset_id == "USDC":
            continue
        assert weight <= policy.max_single_asset_weight + 1e-9, f"{asset_id} above the single-asset cap"
        non_num_issuers[by[asset_id].issuer] = non_num_issuers.get(by[asset_id].issuer, 0.0) + weight
    for issuer, weight in non_num_issuers.items():
        assert weight <= policy.max_issuer_weight + 1e-9, f"issuer {issuer} {weight} above the cap (vault currency excluded)"
    assert result.weights["USDC"] >= policy.min_liquidity_sleeve - 1e-9  # D-P: the thesis's cash floor, not a fixed 85 %



def test_t11_simulation_with_custom_historical_nav():
    """
    T11: Simulation with provided historical NAV DataFrame.
    Verifies replay series matches weighted constituent NAVs.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.simulator import simulate_portfolio

    thesis = "Park runway."
    policy = parse_thesis(thesis, jurisdiction="non-US", investor_class="retail")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)

    dates = pd.date_range("2026-01-01", periods=10)
    nav_df = pd.DataFrame(
        {c.asset_id: [100.0 + i * 0.1 for i in range(10)] for c in candidates},
        index=dates,
    )

    sim_report = simulate_portfolio(opt_result.weights, candidates, policy, historical_nav=nav_df)

    assert len(sim_report.historical_nav_series) == 10
    assert abs(sim_report.historical_nav_series[0] - 100.0) < 1e-3
    assert sim_report.historical_nav_series[-1] > sim_report.historical_nav_series[0]


def test_t12_empty_candidates_rejection():
    """
    T12: Optimizer rejects empty candidate list.
    """
    from src.parser import parse_thesis
    from src.optimizer import optimize_portfolio

    policy = parse_thesis("Park runway safe.", jurisdiction="EU", investor_class="retail")
    with pytest.raises(ValueError, match="cannot be empty"):
        optimize_portfolio(policy, [])


def test_t13_exit_hours_derivation():
    """
    T13: Worst-case exit hours logic derived from liquidity tiers.
    """
    from src.parser import parse_thesis
    from src.retriever import retrieve_candidates, AssetRegistry
    from src.optimizer import optimize_portfolio
    from src.simulator import simulate_portfolio

    # Runway thesis: assets are tier 0 (cash) and tier 1 (T-bills) -> 24 hours
    thesis = "Park 18 months of runway safe, 70% dollar T-bill funds, 20% euro T-bills for payroll, 10% instant liquidity."
    policy = parse_thesis(thesis, jurisdiction="EU", investor_class="professional")
    registry = fixture_registry()
    candidates = retrieve_candidates(policy, registry)
    opt_result = optimize_portfolio(policy, candidates)
    sim_report = simulate_portfolio(opt_result.weights, candidates, policy)
    assert sim_report.worst_case_exit_hours == 24

