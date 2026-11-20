"""
Step 3: Optimise (audit A1, owner decision D-A, 17 Nov 2026).

Fails CLOSED: the optimizer never relaxes a rule and never falls back to
weights that break one. Rules:

Hard (never relaxed):
  * weights sum to exactly 10,000 bps, none negative;
  * the profile - "cash_reserve" (default, the only Phase-1 profile):
    vault currency (numéraire) >= 85 %, lending <= 15 %;
  * cash sleeve >= the policy's liquidity floor;
  * issuer cap and single-asset cap from the policy (the user's own caps,
    audit A2) on every asset EXCEPT the numéraire: a USDC vault is
    inherently USDC-exposed, so its settlement cash is governed by the
    liquidity floor and the profile instead (D-A; disclosed to the user).
Soft (the user's currency split, within 5 % tolerance): if they conflict
with the hard rules, `InfeasibleAllocation` explains why and carries the
nearest allocation without them for the user to accept - nothing is
silently changed.

Method: an exact linear program (HiGHS) proves feasibility and gives a
feasible point; SLSQP then refines it (diversification + yield, or HRP
targets) and is kept only if it still satisfies every rule; weights are
rounded to bps without breaking a cap; an integer-exact check runs last
and raises instead of returning anything that violates a rule.
"""

from dataclasses import dataclass
from typing import Dict, List, Optional, Sequence, Tuple

import numpy as np
import pandas as pd
from scipy.optimize import linprog, minimize

# SciPy 1.14+ compatibility patch for PyPortfolioOpt hierarchical clustering
import scipy.cluster.hierarchy as sch
if not hasattr(sch, "_LINKAGE_METHODS"):
    sch._LINKAGE_METHODS = ["single", "complete", "average", "weighted", "centroid", "median", "ward"]

from pypfopt.hierarchical_portfolio import HRPOpt
from src.models import AssetCandidate, OptimizationResult, PolicyDraft, SleeveType


@dataclass(frozen=True)
class Profile:
    min_numeraire: float
    max_lending: float


# D-A: Cash Reserve is the default and only Phase-1 profile. A "Yield"
# profile (<= 25 % per lending pool) is added once a second audited USDC pool
# exists.
PROFILES: Dict[str, Profile] = {"cash_reserve": Profile(min_numeraire=0.85, max_lending=0.15)}
DEFAULT_PROFILE = "cash_reserve"
CURRENCY_TOLERANCE = 0.05
BPS = 10_000


class InfeasibleAllocation(ValueError):
    """The rules cannot all hold. `reasons` explain why in plain language;
    `nearest` (if any) satisfies every hard rule but drops the targets listed
    in `relaxed` - the user must accept it explicitly."""

    def __init__(self, reasons: List[str], nearest: Optional[OptimizationResult] = None, relaxed: Sequence[str] = ()):
        super().__init__("; ".join(reasons))
        self.reasons = list(reasons)
        self.nearest = nearest
        self.relaxed = list(relaxed)


class OptimizerError(RuntimeError):
    """An internal failure (never silently replaced by fallback weights)."""


def _is_cash(c: AssetCandidate) -> bool:
    return c.sleeve_type == SleeveType.CASH_STABLE


Row = Tuple[np.ndarray, float, str]  # a.w <= b, label


def _hard_rows(policy: PolicyDraft, cands: List[AssetCandidate], n_idx: int, prof: Profile) -> List[Row]:
    N = len(cands)
    rows: List[Row] = []
    a = np.zeros(N); a[n_idx] = -1.0
    rows.append((a, -prof.min_numeraire, f"vault currency >= {prof.min_numeraire:.0%} (profile)"))
    lend = np.array([1.0 if c.sleeve_type == SleeveType.LENDING else 0.0 for c in cands])
    if lend.any():
        rows.append((lend, prof.max_lending, f"lending <= {prof.max_lending:.0%} (profile)"))
    cash = np.array([-1.0 if _is_cash(c) else 0.0 for c in cands])
    rows.append((cash, -policy.min_liquidity_sleeve, f"cash >= {policy.min_liquidity_sleeve:.0%} (liquidity floor)"))
    for iss in sorted({c.issuer for i, c in enumerate(cands) if i != n_idx}):
        row = np.array([1.0 if (c.issuer == iss and i != n_idx) else 0.0 for i, c in enumerate(cands)])
        rows.append((row, policy.max_issuer_weight, f"issuer {iss} <= {policy.max_issuer_weight:.0%}"))
    return rows


def _soft_rows(policy: PolicyDraft, cands: List[AssetCandidate]) -> Tuple[List[Row], List[str]]:
    rows: List[Row] = []
    impossible: List[str] = []
    for cur, target in sorted(policy.currency_exposure.items()):
        if target <= 0:
            continue
        lb = max(0.0, target - CURRENCY_TOLERANCE)
        a = np.array([-1.0 if c.currency.upper() == cur.upper() else 0.0 for c in cands])
        if not a.any():
            impossible.append(f"no eligible asset in {cur} for the requested {target:.0%}")
            continue
        rows.append((a, -lb, f"{cur} >= {lb:.0%} (your currency split, 5% tolerance)"))
    return rows, impossible


def _bounds(policy: PolicyDraft, N: int, n_idx: int) -> List[Tuple[float, float]]:
    return [(0.0, 1.0) if i == n_idx else (0.0, policy.max_single_asset_weight) for i in range(N)]


def _lp(rows: List[Row], bounds, yields: np.ndarray) -> Optional[np.ndarray]:
    N = len(bounds)
    A = np.array([r[0] for r in rows]) if rows else None
    b = np.array([r[1] for r in rows]) if rows else None
    res = linprog(-yields * 1e-3, A_ub=A, b_ub=b, A_eq=np.ones((1, N)), b_eq=[1.0], bounds=bounds, method="highs")
    return res.x if res.status == 0 else None


def _ok(w: np.ndarray, rows: List[Row], bounds, tol: float = 1e-9) -> bool:
    if abs(w.sum() - 1.0) > 1e-7:
        return False
    if any(w[i] < lo - tol or w[i] > hi + tol for i, (lo, hi) in enumerate(bounds)):
        return False
    return all(float(a @ w) <= b + tol for a, b, _ in rows)


def _refine(w0: np.ndarray, rows: List[Row], bounds, objective) -> np.ndarray:
    cons = [{"type": "eq", "fun": lambda w: float(np.sum(w) - 1.0)}]
    cons += [{"type": "ineq", "fun": (lambda w, a=a, b=b: float(b - a @ w))} for a, b, _ in rows]
    try:
        res = minimize(objective, w0, method="SLSQP", bounds=bounds, constraints=cons, options={"maxiter": 500, "ftol": 1e-12})
    except Exception:
        return w0
    return res.x if (res.success and _ok(res.x, rows, bounds, tol=1e-7)) else w0


_NOISE = 1e-4  # bps: solver float noise, e.g. 0.149999999 -> 1500 bps


def _to_bps(w: np.ndarray, n_idx: int, rows: List[Row], bounds) -> List[int]:
    """Whole bps without breaking a rule (largest-remainder rounding).
    Floor every weight (a value within _NOISE of the next bp is solver noise);
    then hand the leftover bps, one at a time, to the assets with the largest
    fractional parts - only where the increment breaks no cap or bound. The
    numéraire (no cap) takes whatever is left. Group targets such as a 15 %
    EUR split survive rounding instead of losing up to N-1 bps."""
    N = len(w)
    raw = [max(0.0, x) * BPS for x in w]
    bps = [int(np.floor(r + _NOISE)) for r in raw]
    caps = [int(np.floor(hi * BPS + _NOISE)) for _, hi in bounds]

    def fits(v: List[int]) -> bool:
        x = np.array(v, dtype=float) / BPS
        return all(v[k] <= caps[k] for k in range(N)) and all(float(a @ x) <= b + 1e-12 for a, b, _ in rows if (a > 0).any())

    diff = BPS - sum(bps)
    if diff < 0:  # noise-rounding overshot: undo it on the bumped assets first
        for k in sorted((k for k in range(N) if k != n_idx and bps[k] > raw[k]), key=lambda k: bps[k] - raw[k], reverse=True):
            if diff == 0:
                break
            bps[k] -= 1
            diff += 1
    for k in sorted(range(N), key=lambda k: raw[k] - np.floor(raw[k] + _NOISE), reverse=True):
        if diff <= 0:
            break
        if k == n_idx or raw[k] <= bps[k]:
            continue
        trial = bps.copy()
        trial[k] += 1
        if fits(trial):
            bps = trial
            diff -= 1
    bps[n_idx] += diff
    return bps


def check_allocation(bps: Dict[str, int], policy: PolicyDraft, cands: List[AssetCandidate], numeraire: str,
                     profile: str = DEFAULT_PROFILE, soft: bool = True) -> List[str]:
    """Integer-exact check of an allocation against every rule. Returns the
    violations (empty = valid). Soft currency targets allow N bps of rounding."""
    prof = PROFILES[profile]
    v: List[str] = []
    if sum(bps.values()) != BPS or any(x < 0 for x in bps.values()):
        v.append("weights must be non-negative and sum to 10,000 bps")
    by = {c.asset_id: c for c in cands}
    unknown = [a for a in bps if a not in by]
    if unknown:
        v.append(f"not eligible candidates: {', '.join(sorted(unknown))}")
        bps = {a: x for a, x in bps.items() if a in by}
    cap = lambda f: int(round(f * BPS))  # noqa: E731
    if bps.get(numeraire, 0) < cap(prof.min_numeraire):
        v.append(f"vault currency {numeraire} below {prof.min_numeraire:.0%}")
    if sum(x for a, x in bps.items() if by[a].sleeve_type == SleeveType.LENDING) > cap(prof.max_lending):
        v.append(f"lending above {prof.max_lending:.0%}")
    if sum(x for a, x in bps.items() if _is_cash(by[a])) < cap(policy.min_liquidity_sleeve):
        v.append(f"cash below the {policy.min_liquidity_sleeve:.0%} liquidity floor")
    issuers: Dict[str, int] = {}
    for a, x in bps.items():
        if a == numeraire:
            continue
        if x > cap(policy.max_single_asset_weight):
            v.append(f"{a} above the {policy.max_single_asset_weight:.0%} single-asset cap")
        issuers[by[a].issuer] = issuers.get(by[a].issuer, 0) + x
    for iss, x in issuers.items():
        if x > cap(policy.max_issuer_weight):
            v.append(f"issuer {iss} above the {policy.max_issuer_weight:.0%} cap")
    if soft:
        for cur, target in policy.currency_exposure.items():
            if target > 0 and any(c.currency.upper() == cur.upper() for c in cands):
                got = sum(x for a, x in bps.items() if by[a].currency.upper() == cur.upper())
                if got < cap(max(0.0, target - CURRENCY_TOLERANCE)) - len(cands):
                    v.append(f"{cur} below your requested split")
    return v


def _result(bps_list: List[int], cands: List[AssetCandidate], algorithm: str, profile: str, numeraire: str) -> OptimizationResult:
    weights_bps = {c.asset_id: b for c, b in zip(cands, bps_list)}
    weights = {a: b / BPS for a, b in weights_bps.items()}

    def agg(key) -> Dict[str, float]:
        out: Dict[str, float] = {}
        for c in cands:
            k = key(c)
            out[k] = round(out.get(k, 0.0) + weights[c.asset_id], 4)
        return out

    return OptimizationResult(
        weights=weights, weights_bps=weights_bps,
        sleeve_weights=agg(lambda c: c.sleeve_type.value),
        currency_weights=agg(lambda c: c.currency),
        issuer_weights=agg(lambda c: c.issuer),  # all-in, numéraire included (transparency)
        algorithm_used=algorithm, profile=profile, numeraire=numeraire,
    )


def optimize_portfolio(
    policy: PolicyDraft,
    candidates: List[AssetCandidate],
    historical_returns: Optional[pd.DataFrame] = None,
    numeraire: str = "USDC",
    profile: str = DEFAULT_PROFILE,
) -> OptimizationResult:
    if not candidates:
        raise ValueError("Candidate asset list cannot be empty for portfolio optimization.")
    if profile not in PROFILES:
        raise ValueError(f"unknown profile {profile!r}; available: {sorted(PROFILES)}")
    prof = PROFILES[profile]
    ids = [c.asset_id for c in candidates]
    if numeraire not in ids:
        raise InfeasibleAllocation([f"the vault currency {numeraire} is not available to this user, so no {profile} allocation exists"])
    n_idx = ids.index(numeraire)
    N = len(candidates)
    bounds = _bounds(policy, N, n_idx)
    yields = np.array([c.current_yield_pct for c in candidates])
    hard = _hard_rows(policy, candidates, n_idx, prof)
    soft, impossible = _soft_rows(policy, candidates)

    def solve(rows: List[Row]) -> Tuple[Optional[List[int]], str]:
        w = _lp(rows, bounds, yields)
        if w is None:
            return None, ""
        algorithm = "LP"
        if historical_returns is not None and not historical_returns.empty:
            cols = [a for a in ids if a in historical_returns.columns]
            sub = historical_returns[cols].dropna() if len(cols) >= 2 else None
            if sub is not None and len(sub) >= 5:
                try:
                    target = HRPOpt(sub).optimize()
                    t = np.array([target.get(a, 0.0) for a in ids])
                    if t.sum() > 0:
                        t = t / t.sum()
                        w2 = _refine(w, rows, bounds, lambda x: float(np.sum((x - t) ** 2)))
                        if w2 is not w:
                            w, algorithm = w2, "HRP"
                except Exception:
                    pass
        if algorithm == "LP":
            lam = {"maximize": 0.05, "none": 0.0}.get(policy.target_yield_orientation, 0.01)
            w2 = _refine(w, rows, bounds, lambda x: float(np.sum(x ** 2) - lam * np.sum(x * yields)))
            if w2 is not w:
                w, algorithm = w2, "CONSTRAINED_HEURISTIC"
        return _to_bps(w, n_idx, rows, bounds), algorithm

    bps, algorithm = solve(hard + soft) if not impossible else (None, "")
    if bps is not None:
        result = _result(bps, candidates, algorithm, profile, numeraire)
        violations = check_allocation(result.weights_bps, policy, candidates, numeraire, profile)
        if violations:
            raise OptimizerError("allocation failed its final check: " + "; ".join(violations))
        return result

    # The user's currency split conflicts with the hard rules (or cannot be
    # met at all): explain, and offer the nearest allocation without it.
    nearest_bps, algo = solve(hard)
    if nearest_bps is None:
        raise InfeasibleAllocation([
            "your caps cannot add up to 100% under the " + profile.replace("_", " ") + " profile: "
            + "; ".join(label for _, _, label in hard)
        ])
    nearest = _result(nearest_bps, candidates, algo, profile, numeraire)
    if check_allocation(nearest.weights_bps, policy, candidates, numeraire, profile, soft=False):
        raise OptimizerError("nearest allocation failed its final check")
    reasons = impossible or [
        f"your currency split conflicts with the {profile.replace('_', ' ')} profile "
        f"(vault currency {numeraire} >= {prof.min_numeraire:.0%}, lending <= {prof.max_lending:.0%})"
    ]
    raise InfeasibleAllocation(reasons, nearest=nearest, relaxed=["currency split"])
