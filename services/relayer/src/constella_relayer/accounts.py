"""Sponsored account creation (audit A2): limits and input checks.

The account factory's `create_account(owner_key)` is permissionless and the
address is bound to the key, so creating an account "for" someone is harmless
- but every creation costs the relayer XLM. Abuse is bounded three ways:
per client IP per day and per day in total (here), and by the fee-sponsor's
on-chain daily budget (record_sponsored_op). In-memory windows reset on
restart; run one relayer process per deployment or put a shared limiter in
front of it.
"""
from __future__ import annotations

import threading
import time
from collections import defaultdict, deque

from .gateway import Rejected

DAY = 86_400


def parse_owner_key(value) -> bytes:
    """A 65-byte uncompressed P-256 public key as hex (0x04 || X || Y)."""
    if not isinstance(value, str) or len(value) != 130:
        raise Rejected("INVALID_OWNER_KEY", "expected 130 hex characters")
    try:
        raw = bytes.fromhex(value)
    except ValueError:
        raise Rejected("INVALID_OWNER_KEY", "not hex") from None
    if raw[0] != 0x04:
        raise Rejected("INVALID_OWNER_KEY", "expected an uncompressed key (0x04...)")
    return raw


class RateLimiter:
    """Sliding 24 h windows: per client and global."""

    def __init__(self, per_client: int, total: int, clock=time.time):
        self.per_client, self.total, self.clock = per_client, total, clock
        self._by_client: dict[str, deque] = defaultdict(deque)
        self._all: deque = deque()
        self._lock = threading.Lock()

    def _trim(self, q: deque, now: float) -> None:
        while q and now - q[0] >= DAY:
            q.popleft()

    def check(self, client: str) -> None:
        """Raise if `client` (or everyone) is over the limit. Does not count."""
        now = self.clock()
        with self._lock:
            self._trim(self._by_client[client], now)
            self._trim(self._all, now)
            if len(self._by_client[client]) >= self.per_client:
                raise Rejected("RATE_LIMITED", "too many new accounts from this client today")
            if len(self._all) >= self.total:
                raise Rejected("RATE_LIMITED", "daily new-account capacity reached")

    def count(self, client: str) -> None:
        now = self.clock()
        with self._lock:
            self._by_client[client].append(now)
            self._all.append(now)
