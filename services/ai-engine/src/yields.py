"""
Real yield numbers (audit A3) - read from the chain, never assumed.

* Blend v2 supply rate: Blend's own interest model (pool/src/pool/interest.rs)
  on the pool's live reserve data, in the same integer arithmetic and
  rounding: three-part curve (kink at 95 %), scaled by ir_mod; supply rate =
  borrow rate x utilisation x (1 - bstop_rate). Verified against the REAL
  pool's accrual in contracts/vault-strategy/tests/security_regressions.rs
  (blend_sleeve::the_blend_supply_rate_formula_matches_what_the_real_pool_accrues);
  the vector from that test is reused in tests/test_yields.py.
* Vault realised return: the vault's own daily conservative NAV history
  (`get_nav_history`), annualised as simple interest.

Rates are APRs (simple, as Blend accrues), not compounded APYs.
"""
from __future__ import annotations

from typing import Dict, List, Optional, Sequence, Tuple

S7 = 10 ** 7
S12 = 10 ** 12
SECONDS_PER_YEAR = 31_536_000  # Blend's year (365 days), confirmed by the real-pool test
KINK = 9_500_000


def _mul_ceil(a: int, b: int, s: int) -> int:
    return -(-(a * b) // s)


def _div_ceil(a: int, b: int, s: int) -> int:
    return -(-(a * s) // b)


def blend_utilization(data: Dict) -> int:
    """7 decimals, capped at 1.0 (Blend `Reserve::utilization`)."""
    liabilities = _mul_ceil(int(data["d_supply"]), int(data["d_rate"]), S12)
    supply = int(data["b_supply"]) * int(data["b_rate"]) // S12
    if liabilities == 0:
        return 0
    if liabilities >= supply:
        return S7
    return _div_ceil(liabilities, supply, S7)


def blend_rates(config: Dict, data: Dict, bstop_rate: int) -> Tuple[int, int]:
    """(borrow APR, supply APR), both 7 decimals."""
    util = blend_utilization(data)
    target = int(config["util"])
    r0, r1, r2, r3 = (int(config[k]) for k in ("r_base", "r_one", "r_two", "r_three"))
    ir_mod = int(data["ir_mod"])
    if util <= target:
        borrow = _mul_ceil(_mul_ceil(_div_ceil(util, target, S7), r1, S7) + r0, ir_mod, S7)
    elif util <= KINK:
        borrow = _mul_ceil(_mul_ceil(_div_ceil(util - target, KINK - target, S7), r2, S7) + r1 + r0, ir_mod, S7)
    else:
        # Above the kink only the intersection is scaled by ir_mod (as in Blend).
        borrow = _mul_ceil(_div_ceil(util - KINK, S7 - KINK, S7), r3, S7) + _mul_ceil(ir_mod, r2 + r1 + r0, S7)
    supply = borrow * util // S7 * (S7 - int(bstop_rate)) // S7
    return borrow, supply


def blend_supply_apr_pct(config: Dict, data: Dict, bstop_rate: int) -> float:
    return blend_rates(config, data, bstop_rate)[1] * 100 / S7


def realized_apr_pct(points: Sequence[Tuple[int, int]], window_days: Optional[int] = None) -> Optional[float]:
    """Annualised change of the vault's conservative NAV per share between the
    first point inside the window and the latest one. None with fewer than
    two points or less than a day between them (not enough data)."""
    pts = sorted((int(t), int(n)) for t, n in points)
    if len(pts) < 2:
        return None
    end_t, end_nav = pts[-1]
    start = pts[0]
    if window_days is not None:
        inside = [p for p in pts if p[0] >= end_t - window_days * 86_400]
        start = inside[0]
    dt = end_t - start[0]
    if dt < 86_400 or start[1] <= 0:
        return None
    return (end_nav - start[1]) / start[1] * SECONDS_PER_YEAR / dt * 100


# ---------------------------------------------------------------- chain reads
def _sim(rpc_url: str, passphrase: str, source: str, contract: str, fn: str, args: List):
    from stellar_sdk import SorobanServer, TransactionBuilder, scval, xdr
    server = SorobanServer(rpc_url)
    tx = (TransactionBuilder(server.load_account(source), passphrase, base_fee=100)
          .append_invoke_contract_function_op(contract, fn, args).set_timeout(30).build())
    sim = server.simulate_transaction(tx)
    if sim.error:
        raise RuntimeError(f"{fn}: {sim.error[:200]}")
    return scval.to_native(xdr.SCVal.from_xdr(sim.results[0].xdr))


def read_blend_supply_apr_pct(rpc_url: str, passphrase: str, source: str, pool: str, asset: str) -> float:
    from stellar_sdk import scval
    reserve = _sim(rpc_url, passphrase, source, pool, "get_reserve", [scval.to_address(asset)])
    cfg = _sim(rpc_url, passphrase, source, pool, "get_config", [])
    return blend_supply_apr_pct(reserve["config"], reserve["data"], int(cfg["bstop_rate"]))


def read_nav_history(rpc_url: str, passphrase: str, source: str, vault: str) -> List[Tuple[int, int]]:
    return [(int(p["timestamp"]), int(p["nav"])) for p in _sim(rpc_url, passphrase, source, vault, "get_nav_history", [])]


def live_lending_yields(rpc_url: str, passphrase: str, source: str, oracle: str, candidates) -> Dict[str, float]:
    """{asset_id: supply APR %} for every Blend position the oracle knows.
    Assets without a live source are left out (unknown, not zero)."""
    from stellar_sdk import scval
    out: Dict[str, float] = {}
    for c in candidates:
        pos = _sim(rpc_url, passphrase, source, oracle, "blend_position", [scval.to_address(c.contract_address)])
        if pos:
            underlying = getattr(pos["underlying"], "address", str(pos["underlying"]))
            out[c.asset_id] = read_blend_supply_apr_pct(rpc_url, passphrase, source, c.contract_address, underlying)
    return out
