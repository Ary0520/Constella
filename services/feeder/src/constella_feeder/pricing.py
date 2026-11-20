"""EURC price in USDC from the ECB rate, and the safety gates applied before
anything is pushed on-chain.

Methodology (a *fundamental* price, independent of market feeds): Circle
redeems EURC 1:1 for EUR and USDC 1:1 for USD (circle.com/eurc, /usdc), so the
par value of 1 EURC in USDC equals USD per EUR. A depeg of either coin makes
market sources diverge from this value, which the oracle's cross-source
deviation breaker then turns into a halt.
"""
from __future__ import annotations

from dataclasses import dataclass
from decimal import ROUND_DOWN, Decimal

from .ecb import EcbRate

ORACLE_DECIMALS = 7

# EUR/USD has traded between ~0.82 (2000) and ~1.60 (2008) since 1999.
# Anything outside a wider band is a data error, never a price to publish.
SANE_MIN = Decimal("0.70")
SANE_MAX = Decimal("1.80")
# Largest accepted move versus the price already on-chain. EUR/USD daily moves
# above 2-3% are exceptional; a bigger jump needs a human look, so the feeder
# refuses and the source goes stale (fail closed) instead of pushing it.
MAX_JUMP_BPS = 500


class GateRejected(Exception):
    """A safety gate refused the update; nothing must be submitted."""


@dataclass(frozen=True)
class Update:
    price: int        # oracle units (7 decimals)
    timestamp: int    # unix seconds of the data
    liquidity_depth_usd: int = 0  # fundamental source: no market depth


def eurc_in_usdc(rate: EcbRate) -> Update:
    """Convert to oracle units, rounding DOWN (never overstates value)."""
    if not (SANE_MIN <= rate.usd_per_eur <= SANE_MAX):
        raise GateRejected(f"EUR/USD {rate.usd_per_eur} outside sanity band [{SANE_MIN}, {SANE_MAX}]")
    units = (rate.usd_per_eur * (Decimal(10) ** ORACLE_DECIMALS)).to_integral_value(rounding=ROUND_DOWN)
    return Update(price=int(units), timestamp=rate.reference_unix)


@dataclass(frozen=True)
class OnChain:
    price: int
    timestamp: int


def decide(update: Update, current: OnChain | None, ledger_now: int) -> str:
    """Return "submit" or "skip"; raise GateRejected when unsafe.

    * skip when the same data is already on-chain (idempotent re-runs)
    * reject data older than what is on-chain or dated in the future
    * reject a jump larger than MAX_JUMP_BPS versus the on-chain price
    """
    if update.timestamp > ledger_now:
        raise GateRejected(f"data timestamp {update.timestamp} is after ledger time {ledger_now}")
    if current is not None:
        if update.timestamp == current.timestamp:
            if update.price != current.price:
                raise GateRejected("same data timestamp but a different price than on-chain")
            return "skip"
        if update.timestamp < current.timestamp:
            raise GateRejected("data older than the price already on-chain")
        if current.price > 0:
            jump_bps = abs(update.price - current.price) * 10_000 // current.price
            if jump_bps > MAX_JUMP_BPS:
                raise GateRejected(f"jump of {jump_bps} bps exceeds {MAX_JUMP_BPS} bps; needs human review")
    return "submit"
