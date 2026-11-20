"""Device pairing mailbox (one account, several devices).

The account's existing device opens a pairing code; the new device joins it
with its passkey's PUBLIC key; the existing device reads it back and approves
(it signs DeviceRegistry.add_device with its own passkey). Both screens show
the same short check code (from the key) so the owner can confirm it is
their device. Only public keys pass through here; nothing here can add a
device - only the account's own signature can. Codes: single use, 10 minutes.
In memory (a restart just cancels pending pairings)."""
from __future__ import annotations

import hashlib
import secrets
import threading
import time

from .accounts import parse_owner_key
from .gateway import Rejected

TTL = 600
ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789"  # no 0/O/1/I/L


def check_code(device_key_hex: str) -> str:
    return hashlib.sha256(bytes.fromhex(device_key_hex)).hexdigest()[:6].upper()


class Pairing:
    def __init__(self, clock=time.time, max_open: int = 1000):
        self.clock, self.max_open = clock, max_open
        self._p: dict[str, dict] = {}
        self._lock = threading.Lock()

    def _gc(self, now: float) -> None:
        for c in [c for c, v in self._p.items() if now - v["at"] > TTL]:
            del self._p[c]

    def start(self, account: str) -> dict:
        if not isinstance(account, str) or not account.startswith("C") or len(account) != 56:
            raise Rejected("INVALID_ADDRESS", "account must be a C... address")
        now = self.clock()
        with self._lock:
            self._gc(now)
            if len(self._p) >= self.max_open:
                raise Rejected("RATE_LIMITED", "too many pairings in progress")
            code = "".join(secrets.choice(ALPHABET) for _ in range(8))
            self._p[code] = {"at": now, "account": account, "device": None}
        return {"code": code, "expires_in": TTL}

    def join(self, code, device_key_hex, name) -> dict:
        parse_owner_key(device_key_hex)
        label = str(name or "device")[:40]
        now = self.clock()
        with self._lock:
            self._gc(now)
            p = self._p.get(code)
            if p is None:
                raise Rejected("PAIRING_NOT_FOUND", "unknown or expired pairing code")
            if p["device"] is not None:
                raise Rejected("PAIRING_USED", "this code was already used")
            p["device"] = {"key": device_key_hex, "name": label}
            return {"account": p["account"], "check": check_code(device_key_hex)}

    def status(self, code) -> dict:
        with self._lock:
            self._gc(self.clock())
            p = self._p.get(code)
            if p is None:
                raise Rejected("PAIRING_NOT_FOUND", "unknown or expired pairing code")
            d = p["device"]
            return {"account": p["account"], "device_key": d["key"] if d else None,
                    "name": d["name"] if d else None, "check": check_code(d["key"]) if d else None}
