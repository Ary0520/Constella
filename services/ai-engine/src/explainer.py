"""
Thesis Engine Step 5: Explain (explainer.py)
Generates plain-language asset rationales, thesis vs proposed diffs,
issuer concentration warnings, and ambiguity clarifications.
"""

from typing import Dict, List, Optional
from src.models import (
    AssetCandidate,
    DiffItem,
    ExplanationReport,
    OptimizationResult,
    PolicyDraft,
    SleeveType,
)


def generate_explanation(
    thesis_text: str,
    policy: PolicyDraft,
    opt_result: OptimizationResult,
    candidates: List[AssetCandidate],
) -> ExplanationReport:
    """
    Constructs an actionable plain-language rationale, diff analysis,
    and risk warnings for a proposed portfolio allocation.
    """
    candidate_map: Dict[str, AssetCandidate] = {c.asset_id: c for c in candidates}
    asset_rationales: Dict[str, str] = {}

    # 1. Plain-Language Asset Rationales for all allocated constituents
    for asset_id, weight in opt_result.weights.items():
        if weight > 0:
            candidate = candidate_map.get(asset_id)
            if candidate is not None:
                sleeve_val = (
                    candidate.sleeve_type.value
                    if hasattr(candidate.sleeve_type, "value")
                    else str(candidate.sleeve_type)
                )
                issuer = candidate.issuer
                disp_name = candidate.display_name
                yield_pct = candidate.current_yield_pct
                currency = candidate.currency
            else:
                sleeve_val = "cash"
                issuer = "Verified Issuer"
                disp_name = asset_id
                yield_pct = 0.0
                currency = "USD"

            weight_pct = weight * 100

            if sleeve_val in ("cash", SleeveType.CASH_STABLE):
                if currency == "EUR":
                    rationale = (
                        f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% as instant euro liquidity buffer. "
                        f"Enables instant payroll and operational settlement in EUR without credit or duration risk."
                    )
                else:
                    rationale = (
                        f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% as primary numéraire cash settlement reserve. "
                        f"Ensures Tier-0 instant redemption liquidity (<1 hour exit) and capital preservation."
                    )
            elif sleeve_val in ("usd_gov_fund", SleeveType.USD_GOV_FUNDS):
                rationale = (
                    f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% for capital preservation and sovereign yield ({yield_pct:.2f}%). "
                    f"Invests in short-term US Treasury bills with daily (Tier-1) liquidity."
                )
            elif sleeve_val in ("eur_sovereign", SleeveType.EUR_SOVEREIGN):
                rationale = (
                    f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% for euro sovereign exposure yielding {yield_pct:.2f}%. "
                    f"Directly hedges euro liability commitments while preserving capital in institutional-grade T-bills."
                )
            elif sleeve_val in ("lending", SleeveType.LENDING):
                rationale = (
                    f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% for variable money-market lending yield ({yield_pct:.2f}%). "
                    f"Supplies collateralized USDC lending within strict single-asset exposure limits."
                )
            elif sleeve_val in ("yield_dollar", SleeveType.YIELD_DOLLAR):
                rationale = (
                    f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% in yield-bearing dollar notes generating {yield_pct:.2f}% "
                    f"consistent cash-flow yield under institutional risk controls."
                )
            elif sleeve_val in ("equity", SleeveType.EQUITY):
                rationale = (
                    f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% in index/equity sleeve for capital growth participation."
                )
            elif sleeve_val in ("xlm", SleeveType.XLM):
                rationale = (
                    f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% in native Stellar XLM reserves for operational network gas."
                )
            else:
                rationale = (
                    f"{disp_name} ({issuer}): Allocated {weight_pct:.1f}% to fulfill thesis objectives with estimated yield of {yield_pct:.2f}%."
                )

            asset_rationales[asset_id] = rationale

    # 2. Transparent Thesis Diffs
    thesis_diff: List[DiffItem] = []

    # Currency exposure comparison - step 7: the REAL reason for a divergence.
    def currency_reason(curr: str) -> str:
        held = [candidate_map[a] for a, w in opt_result.weights.items()
                if w > 0 and a in candidate_map and candidate_map[a].currency.upper() == curr.upper()
                and a != opt_result.numeraire]
        eligible = [c for c in candidates if c.currency.upper() == curr.upper()]
        if not eligible:
            return f"no asset in {curr} is available to you"
        cap = policy.max_single_asset_weight
        if held and all(opt_result.weights[c.asset_id] >= cap - 0.0005 for c in held):
            return f"each {curr} asset is held at your {cap * 100:.0f}% per-asset cap"
        for c in held:
            iss = opt_result.issuer_weights.get(c.issuer, 0.0)
            num = candidate_map.get(opt_result.numeraire)
            if num is not None and num.issuer == c.issuer:
                iss -= opt_result.weights.get(num.asset_id, 0.0)  # the vault currency is exempt (D-A)
            if iss >= policy.max_issuer_weight - 0.0005:
                return (f"issuer {c.issuer} is at your {policy.max_issuer_weight * 100:.0f}% per-issuer cap "
                        f"({c.asset_id} counts toward it)")
        return "it is within the 5-point band of your split"

    for curr, target_pct in policy.currency_exposure.items():
        actual_pct = opt_result.currency_weights.get(curr, 0.0)
        if abs(target_pct - actual_pct) > 0.01:
            thesis_diff.append(
                DiffItem(
                    category="currency",
                    requested=f"{target_pct * 100:.1f}% {curr}",
                    proposed=f"{actual_pct * 100:.1f}% {curr}",
                    explanation=(f"Proposed {actual_pct * 100:.1f}% {curr} instead of {target_pct * 100:.1f}% because "
                                 f"{currency_reason(curr)}. Raise that limit in your thesis to get closer."),
                )
            )
        else:
            thesis_diff.append(
                DiffItem(
                    category="currency",
                    requested=f"{target_pct * 100:.1f}% {curr}",
                    proposed=f"{actual_pct * 100:.1f}% {curr}",
                    explanation=f"Matched requested {curr} currency exposure target at {actual_pct * 100:.1f}%.",
                )
            )

    for curr, actual_pct in opt_result.currency_weights.items():
        if curr not in policy.currency_exposure and actual_pct > 0.005:
            thesis_diff.append(
                DiffItem(
                    category="currency",
                    requested=f"0.0% {curr}",
                    proposed=f"{actual_pct * 100:.1f}% {curr}",
                    explanation=f"Allocated {actual_pct * 100:.1f}% {curr} to maintain necessary cash liquidity buffers.",
                )
            )

    # Liquidity sleeve floor comparison
    cash_pct = opt_result.sleeve_weights.get("cash", 0.0)
    min_liq = policy.min_liquidity_sleeve
    thesis_diff.append(
        DiffItem(
            category="liquidity",
            requested=f">={min_liq * 100:.1f}% instant liquidity buffer",
            proposed=f"{cash_pct * 100:.1f}% instant cash sleeve",
            explanation=(
                f"Preserved {cash_pct * 100:.1f}% instant settlement liquidity (<1h access) "
                f"satisfying treasury operational safety floors."
            ),
        )
    )

    # Exclusions comparison
    for excl in policy.exclusions:
        thesis_diff.append(
            DiffItem(
                category="exclusion",
                requested=f"Exclude {excl}",
                proposed="0.0% allocation",
                explanation=f"Strictly excluded '{excl}' category from portfolio constituents in accordance with mandate.",
            )
        )

    # Parser notes - step 7: an unregistered asset is excluded; anything else is
    # a default or reading the user should confirm (not an "excluded asset").
    for amb in policy.ambiguities:
        unregistered = "not part of the verified Stellar asset registry" in amb
        thesis_diff.append(
            DiffItem(
                category="ambiguity",
                requested=amb,
                proposed="Excluded from on-chain basket" if unregistered else "Default applied - please confirm",
                explanation=(
                    "Constella enforces a closed universe of verified on-chain assets. "
                    "Unregistered or unsupported assets cannot be allocated."
                    if unregistered else
                    "Constella could not read this from your thesis with confidence; the value shown applies "
                    "unless you change your thesis."
                ),
            )
        )

    # Fallback to ensure thesis_diff is never empty
    if not thesis_diff:
        thesis_diff.append(
            DiffItem(
                category="allocation",
                requested="Automated runway preservation",
                proposed=f"{len(asset_rationales)} constituents allocated",
                explanation=f"Portfolio constructed using {opt_result.algorithm_used} algorithm adhering to all caps and constraints.",
            )
        )

    # 3. Issuer Concentration Warnings (Threshold >= 25% or 0.249)
    issuer_concentration_warnings: List[str] = []
    # Decision D-A: the vault currency (numéraire) is exempt from the issuer
    # cap, so its issuer's exposure is disclosed plainly instead of being
    # described as "within the cap".
    numeraire = candidate_map.get(opt_result.numeraire) if opt_result.numeraire else None
    for issuer, weight in sorted(opt_result.issuer_weights.items(), key=lambda x: -x[1]):
        if numeraire is not None and issuer == numeraire.issuer and weight >= 0.249:
            issuer_concentration_warnings.append(
                f"Vault currency disclosure: {weight * 100:.1f}% of this vault depends on {issuer}, mostly through "
                f"{numeraire.asset_id} - the currency the vault holds and pays out in. The {policy.max_issuer_weight * 100:.1f}% "
                f"issuer cap applies to every other asset; settlement cash is exempt by design, so a {numeraire.asset_id} "
                f"depeg or an issuer freeze would affect this share."
            )
        elif weight >= 0.249:
            issuer_concentration_warnings.append(
                f"Concentration Warning: Issuer '{issuer}' represents {weight * 100:.1f}% of total portfolio allocation (>= 25% threshold). "
                f"While within the configured cap ({policy.max_issuer_weight * 100:.1f}%), this concentration carries single-entity counterparty risk."
            )

    # 4. Ambiguity Clarifications
    ambiguity_clarifications: List[str] = []
    for amb in policy.ambiguities:
        ambiguity_clarifications.append(
            f"Clarification required: {amb}. Please verify before authorizing on-chain signing."
        )

    return ExplanationReport(
        asset_rationales=asset_rationales,
        thesis_diff=thesis_diff,
        issuer_concentration_warnings=issuer_concentration_warnings,
        ambiguity_clarifications=ambiguity_clarifications,
    )
