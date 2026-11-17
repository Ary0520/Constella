"""
Rebalance planner (pure; no network).

Mirrors the vault's own valuation exactly (vault-strategy `sleeve_value`):
the cash sleeve (only ever the vault currency) is worth its units; any other
sleeve is units x oracle `asset_price` / 10^decimals, rounded down. Holdings
are min(booked, real) like `sleeve_holdings`.

Trades go through the vault currency (hub): overweight sleeves are sold for
it, underweight sleeves are bought with it, sells first. Every leg's
minimum output is the ORACLE value of what goes in, less the policy's
maximum slippage - so a pool trading away from the oracle makes the plan
fail here instead of trading at a bad price. The keeper never decides what
is allowed: the policy contract validates the proposal and the vault
re-checks its actual holdings after the trades.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Callable, List, Optional, Tuple

ROUTE_SOROSWAP = 1
ROUTE_BLEND = 4


class PlanError(RuntimeError):
    pass


@dataclass(frozen=True)
class Sleeve:
    asset_id: str
    address: str
    target_bps: int
    band_bps: int
    held: int                 # min(booked, real) units
    price: Optional[int]      # oracle asset_price (scaled; the vault currency = 1.0); None = unpriced
    is_cash: bool             # the vault currency
    is_blend: bool = False
    decimals: int = 7         # token decimals (a Blend position: its underlying's)


@dataclass
class Trade:
    asset_in: str
    asset_out: str
    amount_in: int
    min_amount_out: int
    route: int


@dataclass
class Plan:
    operations: List[Trade]
    weights_after: List[Tuple[str, int]]
    drift_bps: dict = field(default_factory=dict)


def value(s: Sleeve, scale: int) -> int:
    if s.is_cash:
        return s.held
    if s.held == 0:
        return 0
    if s.price is None or s.price <= 0:
        raise PlanError(f"{s.asset_id} has no valid oracle price")
    return s.held * s.price // scale


def to_bps(values: List[int]) -> List[int]:
    """Largest-remainder rounding to exactly 10000 (0s if empty)."""
    total = sum(values)
    if total <= 0:
        return [0] * len(values)
    raw = [v * 10000 for v in values]
    out = [r // total for r in raw]
    rest = 10000 - sum(out)
    order = sorted(range(len(values)), key=lambda i: -(raw[i] % total))
    for i in order[:rest]:
        out[i] += 1
    return out


def weights(sleeves: List[Sleeve], scale: int) -> List[int]:
    return to_bps([value(s, scale) for s in sleeves])


def needs_rebalance(sleeves: List[Sleeve], scale: int, default_band_bps: int) -> dict:
    """{asset_id: drift_bps} for sleeves outside their band (empty = in band)."""
    if sum(value(s, scale) for s in sleeves) == 0:
        return {}
    out = {}
    for s, w in zip(sleeves, weights(sleeves, scale)):
        band = s.band_bps if s.band_bps > 0 else default_band_bps
        if abs(w - s.target_bps) > band:
            out[s.asset_id] = w - s.target_bps
    return out


Quote = Callable[[str, str, int], int]


def min_out_for(s_in: Sleeve, s_out: Sleeve, amount_in: int, max_slippage_bps: int) -> int:
    """Smallest min_amount_out the policy accepts (policy step 9, exact):
    floor(m * p_out / 10^d_out) * 10000 >= floor(amount_in * p_in / 10^d_in) * (10000 - slip)."""
    for leg_s in (s_in, s_out):
        if leg_s.price is None or leg_s.price <= 0:
            raise PlanError(f"{leg_s.asset_id} has no valid oracle price")
    keep = 10000 - max_slippage_bps
    need = amount_in * s_in.price // 10 ** s_in.decimals * keep  # = required min_value_out * 10000
    min_value_out = -(-need // 10000)
    # m = ceil(min_value_out * 10^d / p) gives m * p / 10^d >= min_value_out
    # exactly, so its floor is >= the integer min_value_out, which is
    # ceil(need / 10000); and m - 1 falls short. Exact, no search needed.
    m = -(-min_value_out * 10 ** s_out.decimals // s_out.price)
    if (m * s_out.price // 10 ** s_out.decimals) * 10000 < need:  # unreachable (proof above)
        raise PlanError("internal rounding error")
    return m


def plan(sleeves: List[Sleeve], scale: int, default_band_bps: int, max_slippage_bps: int,
         quote: Quote, min_trade_value: int = 1) -> Optional[Plan]:
    """A proposal that brings every sleeve to its target, or None when all
    sleeves are within their bands. `quote(asset_in, asset_out, amount_in)`
    returns the AMM's expected output (Soroswap legs only)."""
    drift = needs_rebalance(sleeves, scale, default_band_bps)
    if not drift:
        return None
    cash = [s for s in sleeves if s.is_cash]
    if len(cash) != 1:
        raise PlanError("exactly one cash (vault currency) sleeve is required")
    hub = cash[0]
    if not 0 <= max_slippage_bps < 10000:
        raise PlanError("invalid slippage limit")
    total = sum(value(s, scale) for s in sleeves)

    def leg(s_in: Sleeve, s_out: Sleeve, amount_in: int, blend: bool) -> Trade:
        min_out = min_out_for(s_in, s_out, amount_in, max_slippage_bps)
        if min_out <= 0:
            raise PlanError(f"{s_in.asset_id}->{s_out.asset_id}: trade too small")
        if not blend:
            q = quote(s_in.address, s_out.address, amount_in)
            if q < min_out:
                raise PlanError(f"{s_in.asset_id}->{s_out.asset_id}: pool quote {q} is below the oracle minimum {min_out}; "
                                "not trading against an off-oracle pool")
        return Trade(s_in.address, s_out.address, amount_in, min_out, ROUTE_BLEND if blend else ROUTE_SOROSWAP)

    sells: List[Trade] = []
    buys: List[Tuple[Sleeve, int]] = []
    after = {s.asset_id: value(s, scale) for s in sleeves}
    cash_in = 0
    for s in sleeves:
        if s.is_cash:
            continue
        delta = total * s.target_bps // 10000 - value(s, scale)
        if abs(delta) < min_trade_value:
            continue
        if delta < 0:
            units = min((-delta) * scale // s.price, s.held)
            if units <= 0:
                continue
            out_value = units * s.price // scale
            t = leg(s, hub, units, s.is_blend)
            sells.append(t)
            cash_in += t.min_amount_out
            after[s.asset_id] -= out_value
            after[hub.asset_id] += out_value
        else:
            buys.append((s, delta))

    # Buys are paid from the cash the vault will surely have (sells at their
    # minimum), scaled down pro rata if short.
    available = hub.held + cash_in
    wanted = sum(d for _, d in buys)
    ratio_num, ratio_den = (available, wanted) if wanted > available else (1, 1)
    ops = list(sells)
    for s, d in buys:
        amount = d * ratio_num // ratio_den
        if amount < min_trade_value:
            continue
        if s.price is None or s.price <= 0:
            raise PlanError(f"{s.asset_id} has no valid oracle price")
        ops.append(leg(hub, s, amount, s.is_blend))
        after[s.asset_id] += amount
        after[hub.asset_id] -= amount
    if not ops:
        return None
    ids = [s.asset_id for s in sleeves]
    return Plan(ops, list(zip(ids, to_bps([after[i] for i in ids]))), drift)
