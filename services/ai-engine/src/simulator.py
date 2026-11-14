"""
Portfolio Simulation Module for the Constella AI Thesis Engine (Step 4: Simulate).
Produces comprehensive multi-dimensional risk, yield, redemption, and stress reports
before user funds are committed. Enforces mandatory Rule 10 legal disclaimer.
"""

from typing import Dict, List, Optional
import pandas as pd
from src.models import (
    AssetCandidate,
    PolicyDraft,
    RedemptionLadder,
    SimulationReport,
    SleeveType,
    StressScenario,
)


def simulate_portfolio(
    weights: Dict[str, float],
    candidates: List[AssetCandidate],
    policy: PolicyDraft,
    historical_nav: Optional[pd.DataFrame] = None,
) -> SimulationReport:
    """
    Simulate portfolio yield, liquidity tier partition, and stress scenario responses.
    """
    cand_map = {c.asset_id: c for c in candidates}

    # 1. Weighted yield calculation across assets
    weighted_yield_pct = sum(
        weights.get(aid, 0.0) * cand_map[aid].current_yield_pct
        for aid in weights
        if aid in cand_map
    )
    weighted_yield_pct = round(weighted_yield_pct, 4)

    # 2. Partition redemption ladder strictly by liquidity_tier
    # Tier 0: instant (< 1 hour)
    instant_pct = sum(
        weights.get(c.asset_id, 0.0)
        for c in candidates
        if c.liquidity_tier == 0
    )
    # Tier 1: within 24 hours
    within_24h_pct = sum(
        weights.get(c.asset_id, 0.0)
        for c in candidates
        if c.liquidity_tier == 1
    )
    # Tier 2: within 1 week
    within_1w_pct = sum(
        weights.get(c.asset_id, 0.0)
        for c in candidates
        if c.liquidity_tier == 2
    )
    # Tier 3: illiquid / gated
    illiquid_pct = sum(
        weights.get(c.asset_id, 0.0)
        for c in candidates
        if c.liquidity_tier >= 3
    )

    ladder = RedemptionLadder(
        instant_pct=round(instant_pct, 4),
        within_24h_pct=round(within_24h_pct, 4),
        within_1w_pct=round(within_1w_pct, 4),
        illiquid_pct=round(illiquid_pct, 4),
    )

    # 3. Simulate depeg stress (5% stablecoin depeg: nav_impact_pct = -0.05 * cash_weight)
    cash_weight = sum(
        weights.get(c.asset_id, 0.0)
        for c in candidates
        if (c.sleeve_type == SleeveType.CASH_STABLE or 
            (hasattr(c.sleeve_type, "value") and c.sleeve_type.value == "cash") or
            c.sleeve_type == "cash")
    )
    depeg_impact = round(-0.05 * cash_weight, 4)
    depeg_stress = StressScenario(
        scenario_name="5% Stablecoin Depeg",
        nav_impact_pct=depeg_impact,
        trapped_liquidity_pct=0.0,
        description=f"Simulates a 5% depeg in stablecoin/cash reserves, resulting in a {depeg_impact:.2%} portfolio NAV drawdown.",
    )

    # 4. Simulate issuer freeze stress (trapped_liquidity_pct = largest issuer weight)
    issuer_weights: Dict[str, float] = {}
    for aid, w in weights.items():
        if aid in cand_map:
            iss = cand_map[aid].issuer
            issuer_weights[iss] = issuer_weights.get(iss, 0.0) + w

    if issuer_weights:
        largest_issuer = max(issuer_weights, key=issuer_weights.get)
        trapped_pct = round(issuer_weights[largest_issuer], 4)
    else:
        largest_issuer = "None"
        trapped_pct = 0.0

    issuer_freeze_stress = StressScenario(
        scenario_name="30-Day Issuer Freeze",
        nav_impact_pct=0.0,
        trapped_liquidity_pct=trapped_pct,
        description=f"Simulates a 30-day freeze of the largest issuer ({largest_issuer}), trapping {trapped_pct:.1%} of portfolio liquidity.",
    )

    # 5. Worst case exit hours derived from constituent liquidity tiers
    active_candidates = [c for c in candidates if weights.get(c.asset_id, 0.0) > 0]
    if any(c.liquidity_tier >= 3 for c in active_candidates):
        worst_case_exit_hours = 720  # 30 days
    elif any(c.liquidity_tier == 2 for c in active_candidates):
        worst_case_exit_hours = 168  # 1 week
    elif any(c.liquidity_tier == 1 for c in active_candidates):
        worst_case_exit_hours = 24   # 24 hours
    else:
        worst_case_exit_hours = 1    # < 1 hour (instant)

    # 6. Historical NAV replay series
    if historical_nav is not None and not historical_nav.empty:
        avail_cols = [c.asset_id for c in candidates if c.asset_id in historical_nav.columns and weights.get(c.asset_id, 0.0) > 0]
        if avail_cols:
            norm_nav = historical_nav[avail_cols] / historical_nav[avail_cols].iloc[0] * 100.0
            weighted_series = sum(weights[col] * norm_nav[col] for col in avail_cols)
            historical_nav_series = [round(float(v), 4) for v in weighted_series.dropna().tolist()]
        else:
            historical_nav_series = [round(100.0 * (1.0 + (weighted_yield_pct / 100.0) * (d / 365.0)), 4) for d in range(31)]
    else:
        historical_nav_series = [round(100.0 * (1.0 + (weighted_yield_pct / 100.0) * (d / 365.0)), 4) for d in range(31)]

    # 7. Mandatory legal disclaimer (Rule 10)
    disclaimer = "This simulation uses historical data. Past performance does not predict future returns. This is not financial advice."

    return SimulationReport(
        weighted_yield_pct=weighted_yield_pct,
        redemption_ladder=ladder,
        depeg_stress=depeg_stress,
        issuer_freeze_stress=issuer_freeze_stress,
        worst_case_exit_hours=worst_case_exit_hours,
        historical_nav_series=historical_nav_series,
        disclaimer=disclaimer,
    )
