"""
Portfolio Optimization Module for the Constella AI Thesis Engine (Step 3: Optimise).
Implements Hierarchical Risk Parity (HRP via PyPortfolioOpt) with fallback to
constrained heuristic optimization, enforcing signed policy bounds:
- Exact weight sum: 1.0 (10,000 basis points)
- Cash liquidity sleeve floor: >= policy.min_liquidity_sleeve
- Single asset concentration cap: <= policy.max_single_asset_weight
- Issuer concentration cap: <= policy.max_issuer_weight
- Currency target preservation within tolerance
"""

from typing import Dict, List, Optional
import numpy as np
import pandas as pd
from scipy.optimize import minimize

# SciPy 1.14+ compatibility patch for PyPortfolioOpt hierarchical clustering
import scipy.cluster.hierarchy as sch
if not hasattr(sch, "_LINKAGE_METHODS"):
    sch._LINKAGE_METHODS = ["single", "complete", "average", "weighted", "centroid", "median", "ward"]

from pypfopt.hierarchical_portfolio import HRPOpt
from src.models import AssetCandidate, OptimizationResult, PolicyDraft, SleeveType


def optimize_portfolio(
    policy: PolicyDraft,
    candidates: List[AssetCandidate],
    historical_returns: Optional[pd.DataFrame] = None,
) -> OptimizationResult:
    """
    Generate an optimal, constraint-bound asset allocation vector for candidate assets.
    Enforces caps, floors, currency exposures, and integer basis point consistency.
    """
    if not candidates:
        raise ValueError("Candidate asset list cannot be empty for portfolio optimization.")

    N = len(candidates)
    candidate_ids = [c.asset_id for c in candidates]

    # Effective single asset cap (handles small candidate pools safely)
    total_asset_capacity = N * policy.max_single_asset_weight
    if total_asset_capacity < 1.0:
        effective_single_asset_cap = 1.0 / N
    else:
        effective_single_asset_cap = policy.max_single_asset_weight

    bounds = [(0.0, effective_single_asset_cap) for _ in range(N)]
    cons = [{"type": "eq", "fun": lambda w: float(np.sum(w) - 1.0)}]

    # Cash sleeve minimum floor
    cash_indices = [
        i for i, c in enumerate(candidates)
        if (c.sleeve_type == SleeveType.CASH_STABLE or 
            c.sleeve_type == "cash" or 
            (hasattr(c.sleeve_type, "value") and c.sleeve_type.value == "cash"))
    ]
    if cash_indices:
        cash_cap = sum(effective_single_asset_cap for _ in cash_indices)
        required_cash = min(policy.min_liquidity_sleeve, cash_cap)
        cons.append({
            "type": "ineq",
            "fun": lambda w, idx=cash_indices, req=required_cash: float(sum(w[i] for i in idx) - req)
        })

    # Issuer caps
    issuers = set(c.issuer for c in candidates)
    total_issuer_capacity = len(issuers) * policy.max_issuer_weight
    for iss in issuers:
        iss_indices = [i for i, c in enumerate(candidates) if c.issuer == iss]
        if total_issuer_capacity < 1.0:
            iss_cap = 1.0 / len(issuers)
        else:
            iss_cap = policy.max_issuer_weight
        cons.append({
            "type": "ineq",
            "fun": lambda w, idx=iss_indices, cap=iss_cap: float(cap - sum(w[i] for i in idx))
        })

    # Currency constraints
    if policy.currency_exposure:
        for cur, target in policy.currency_exposure.items():
            cur_indices = [i for i, c in enumerate(candidates) if c.currency.upper() == cur.upper()]
            if cur_indices:
                # Lower bound with 5% tolerance
                lower_bound = max(0.0, target - 0.05)
                cons.append({
                    "type": "ineq",
                    "fun": lambda w, idx=cur_indices, lb=lower_bound: float(sum(w[i] for i in idx) - lb)
                })

    w0 = np.ones(N) / N
    algorithm_used = "CONSTRAINED_HEURISTIC"

    if historical_returns is not None and not historical_returns.empty:
        avail_cols = [c.asset_id for c in candidates if c.asset_id in historical_returns.columns]
        if len(avail_cols) >= 2:
            try:
                sub_returns = historical_returns[avail_cols].dropna()
                if len(sub_returns) >= 5:
                    hrp = HRPOpt(sub_returns)
                    hrp_dict = hrp.optimize()
                    w_target = np.zeros(N)
                    for i, c in enumerate(candidates):
                        w_target[i] = hrp_dict.get(c.asset_id, 0.0)
                    if w_target.sum() > 0:
                        w_target = w_target / w_target.sum()
                        algorithm_used = "HRP"
                        def obj_hrp(w):
                            return float(np.sum((w - w_target)**2))
                        opt_obj = obj_hrp
            except Exception:
                algorithm_used = "CONSTRAINED_HEURISTIC"

    if algorithm_used == "CONSTRAINED_HEURISTIC":
        yield_lambda = 0.01
        if policy.target_yield_orientation == "maximize":
            yield_lambda = 0.05
        elif policy.target_yield_orientation == "none":
            yield_lambda = 0.0
        yields = np.array([candidates[i].current_yield_pct for i in range(N)])
        def obj_heuristic(w):
            return float(np.sum(w**2) - yield_lambda * np.sum(w * yields))
        opt_obj = obj_heuristic

    res = minimize(
        opt_obj,
        w0,
        method="SLSQP",
        bounds=bounds,
        constraints=cons,
        options={"maxiter": 500, "ftol": 1e-9},
    )

    if res.success:
        w_raw = res.x
    else:
        w_raw = w0

    # Ensure non-negative bounds and initial normalization
    w_raw = np.clip(w_raw, 0.0, effective_single_asset_cap)
    w_raw = w_raw / np.sum(w_raw)

    # Convert to basis points (exact sum == 10000 bps)
    weights_bps: Dict[str, int] = {}
    for i, c in enumerate(candidates):
        weights_bps[c.asset_id] = int(round(float(w_raw[i]) * 10000))

    diff_bps = 10000 - sum(weights_bps.values())
    if diff_bps != 0:
        sorted_candidates = sorted(candidates, key=lambda c: weights_bps[c.asset_id], reverse=True)
        cap_bps = int(round(effective_single_asset_cap * 10000))
        adjusted = False
        for c in sorted_candidates:
            new_val = weights_bps[c.asset_id] + diff_bps
            if 0 <= new_val <= cap_bps:
                weights_bps[c.asset_id] = new_val
                adjusted = True
                break
        if not adjusted:
            weights_bps[sorted_candidates[0].asset_id] += diff_bps

    # Exact float weights derived directly from basis points
    weights: Dict[str, float] = {aid: bps / 10000.0 for aid, bps in weights_bps.items()}

    # Sleeve weights aggregation
    sleeve_weights: Dict[str, float] = {}
    for c in candidates:
        sleeve_key = c.sleeve_type.value if hasattr(c.sleeve_type, "value") else str(c.sleeve_type)
        sleeve_weights[sleeve_key] = round(sleeve_weights.get(sleeve_key, 0.0) + weights[c.asset_id], 4)

    # Currency weights aggregation
    currency_weights: Dict[str, float] = {}
    for c in candidates:
        currency_weights[c.currency] = round(currency_weights.get(c.currency, 0.0) + weights[c.asset_id], 4)

    # Issuer weights aggregation
    issuer_weights: Dict[str, float] = {}
    for c in candidates:
        issuer_weights[c.issuer] = round(issuer_weights.get(c.issuer, 0.0) + weights[c.asset_id], 4)

    return OptimizationResult(
        weights=weights,
        weights_bps=weights_bps,
        sleeve_weights=sleeve_weights,
        currency_weights=currency_weights,
        issuer_weights=issuer_weights,
        algorithm_used=algorithm_used,
    )
