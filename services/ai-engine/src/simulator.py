"""
Thesis Engine Step 4: Simulate (docs/build-spec/ai-engine/THESIS-ENGINE.md).
Shown to the user BEFORE any money moves, worst case first.

  * Yield estimate      - weighted average of current constituent yields (unknown = 0).
  * Redemption ladder   - share redeemable <1 h / 24 h / 1 week / longer, by liquidity tier.
  * Depeg stress        - 5 % depeg of the LARGEST stable constituent, applied to it and
                          to every position denominated in it (a Blend USDC supply is USDC).
  * Issuer outage       - the largest-weight issuer frozen 30 days: what is trapped, and
                          what can still be redeemed meanwhile (by tier).
  * Exit times          - normal full exit (slowest tier held) and the worst plausible
                          full exit (the 30-day freeze when the issuer holds anything).
  * Historical replay   - only from real constituent history; without it, NO series and
                          an explicit note (step 7: never a made-up line labelled history).
  * Rule 10 disclaimer  - verbatim (AGENT-RULES.md).
"""

from typing import Dict, List, Optional

import pandas as pd

from src.models import AssetCandidate, PolicyDraft, RedemptionLadder, SimulationReport, SleeveType, StressScenario

DISCLAIMER = ("This simulation uses historical data. Past performance does not predict future returns. "
              "This is not financial advice.")
DEPEG = 0.05
FREEZE_HOURS = 720
TIER_EXIT_HOURS = {0: 1, 1: 24, 2: 168}  # tier 3+ -> FREEZE_HOURS (illiquid / gated)


def _ladder(weights: Dict[str, float], cands: List[AssetCandidate], skip_issuer: Optional[str] = None) -> RedemptionLadder:
    def share(pred) -> float:
        return round(sum(weights.get(c.asset_id, 0.0) for c in cands if pred(c) and c.issuer != skip_issuer), 4)
    return RedemptionLadder(instant_pct=share(lambda c: c.liquidity_tier == 0),
                            within_24h_pct=share(lambda c: c.liquidity_tier == 1),
                            within_1w_pct=share(lambda c: c.liquidity_tier == 2),
                            illiquid_pct=share(lambda c: c.liquidity_tier >= 3))


def _stable_denomination(c: AssetCandidate, stables: Dict[str, AssetCandidate]) -> Optional[str]:
    """The stable constituent a position is denominated in: itself for cash; for a
    lending supply, the stable of the same currency it lends (e.g. Blend USDC -> USDC)."""
    if c.sleeve_type == SleeveType.CASH_STABLE:
        return c.asset_id
    if c.sleeve_type == SleeveType.LENDING:
        for s in stables.values():
            if s.currency.upper() == c.currency.upper() and s.asset_id in c.asset_id.upper():
                return s.asset_id
    return None


def simulate_portfolio(
    weights: Dict[str, float],
    candidates: List[AssetCandidate],
    policy: PolicyDraft,
    historical_nav: Optional[pd.DataFrame] = None,
) -> SimulationReport:
    held = [c for c in candidates if weights.get(c.asset_id, 0.0) > 0]
    by_id = {c.asset_id: c for c in candidates}

    weighted_yield_pct = round(sum(w * by_id[a].current_yield_pct for a, w in weights.items() if a in by_id), 4)
    ladder = _ladder(weights, candidates)

    # Depeg: the largest stable constituent, and every position denominated in it.
    stables = {c.asset_id: c for c in held if c.sleeve_type == SleeveType.CASH_STABLE}
    if stables:
        exposure: Dict[str, List[str]] = {}
        for c in held:
            d = _stable_denomination(c, stables)
            if d:
                exposure.setdefault(d, []).append(c.asset_id)
        target = max(stables, key=lambda a: weights.get(a, 0.0))
        hit = exposure.get(target, [target])
        hit_w = sum(weights.get(a, 0.0) for a in hit)
        impact = round(-DEPEG * hit_w, 4)
        depeg = StressScenario(
            scenario_name=f"{DEPEG:.0%} depeg of {target}", nav_impact_pct=impact, trapped_liquidity_pct=0.0,
            description=(f"{target}, the largest stable holding, loses {DEPEG:.0%} of its value. Positions denominated in it "
                         f"({', '.join(hit)}, {hit_w:.0%} of the basket) fall with it: basket value {impact:.2%} in dollar terms."))
    else:
        depeg = StressScenario(scenario_name="Stablecoin depeg", nav_impact_pct=0.0, trapped_liquidity_pct=0.0,
                               description="The basket holds no stable constituent.")

    # Issuer outage: the largest-weight issuer frozen for 30 days.
    issuers: Dict[str, float] = {}
    for c in held:
        issuers[c.issuer] = issuers.get(c.issuer, 0.0) + weights[c.asset_id]
    top = max(issuers, key=issuers.get) if issuers else None
    trapped = round(issuers.get(top, 0.0), 4) if top else 0.0
    meanwhile = _ladder(weights, candidates, skip_issuer=top)
    freeze = StressScenario(
        scenario_name="30-day issuer freeze", nav_impact_pct=0.0, trapped_liquidity_pct=trapped,
        description=(f"{top}, the largest issuer, freezes its tokens for 30 days: {trapped:.0%} of the basket cannot be "
                     f"redeemed until then. Meanwhile {meanwhile.instant_pct:.0%} can be redeemed instantly and "
                     f"{meanwhile.within_24h_pct:.0%} within 24 hours." if top else "The basket is empty."))

    # Exit times: normal (slowest tier held) and worst plausible (the freeze).
    normal = max((TIER_EXIT_HOURS.get(c.liquidity_tier, FREEZE_HOURS) for c in held), default=0)
    worst = max(normal, FREEZE_HOURS) if trapped > 0 else normal
    scenario = (f"a 30-day freeze of {top}, the largest issuer ({trapped:.0%} of the basket)" if worst > normal
                else "the slowest holding's normal redemption time")

    # Historical replay - real history only.
    series: List[float] = []
    note = "No constituent price history is available yet, so no replay is shown (nothing is made up)."
    if historical_nav is not None and not historical_nav.empty:
        cols = [c.asset_id for c in held if c.asset_id in historical_nav.columns]
        if cols:
            nav = historical_nav[cols].dropna()
            norm = nav / nav.iloc[0] * 100.0
            total = sum(weights[c] for c in cols)
            series = [round(float(v), 4) for v in (sum(weights[c] * norm[c] for c in cols) / total).tolist()]
            note = f"Replay of {len(series)} observations of {', '.join(cols)}."

    return SimulationReport(
        weighted_yield_pct=weighted_yield_pct,
        redemption_ladder=ladder,
        depeg_stress=depeg,
        issuer_freeze_stress=freeze,
        worst_case_exit_hours=worst,
        historical_nav_series=series,
        disclaimer=DISCLAIMER,
        normal_exit_hours=normal,
        worst_case_scenario=scenario,
        redeemable_during_freeze=meanwhile,
        history_available=bool(series),
        history_note=note,
    )
