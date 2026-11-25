"""Guardian-approval mailbox for account recovery (step 8; decision D-F).

A recovery proposal is ONE authorization entry of the user's smart account:
root invocation `account.propose_recovery(new_owner)`. Every guardian signs
that same entry (same nonce, expiry and invocation) with its passkey under the
account's recovery rule; the signatures are merged into one AuthPayload
(`{context_rule_ids, signers: Map<Signer, sig>}`) once the threshold is met.

Safety: nothing here can recover an account. The account's `__check_auth`
verifies each guardian signature and the threshold policy, and the account
only lets the recovery take effect after 48 hours, cancellable by the owner.
The mailbox only refuses anything that is not exactly the opened proposal.
State is a small JSON file so pending approvals survive a restart; requests
expire after seven days.
"""
from __future__ import annotations

import base64
import json
import os
import secrets
import threading
import time
from typing import Callable, Optional

from stellar_sdk import Address, scval, xdr

from .gateway import Rejected, address_credentials

# ADDRESS and CAP-71 ADDRESS_V2 (what testnet simulation returns); delegated
# credentials are not used by Constella accounts.
_KINDS = (xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS, xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS_V2)

TTL = 7 * 86_400
ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789"


def _entry(b64: str) -> xdr.SorobanAuthorizationEntry:
    try:
        e = xdr.SorobanAuthorizationEntry.from_xdr(b64)
    except Exception as ex:  # malformed of any kind
        raise Rejected("INVALID_XDR", f"entry: {ex}") from None
    if e.credentials.type not in _KINDS:
        raise Rejected("UNSUPPORTED_CREDENTIALS", "address credentials (ADDRESS / ADDRESS_V2) only")
    return e


def _shape(e: xdr.SorobanAuthorizationEntry) -> bytes:
    """Everything a guardian signs over except the signature itself."""
    a = address_credentials(e.credentials)
    return (e.credentials.type.value.to_bytes(4, "big") + a.address.to_xdr_bytes() + a.nonce.to_xdr_bytes() + a.signature_expiration_ledger.to_xdr_bytes()
            + e.root_invocation.to_xdr_bytes())


def _payload(e: xdr.SorobanAuthorizationEntry) -> tuple[bytes, list]:
    """(rule ids XDR, [(signer key ScVal, sig ScVal)]) of a signed entry."""
    sig = address_credentials(e.credentials).signature
    if sig.type != xdr.SCValType.SCV_MAP or sig.map is None:
        raise Rejected("UNSIGNED", "the guardian entry carries no signature")
    top = {scval.to_native(m.key): m.val for m in sig.map.sc_map}
    if set(top) != {"context_rule_ids", "signers"}:
        raise Rejected("INVALID_SIGNATURE", "expected {context_rule_ids, signers}")
    return top["context_rule_ids"].to_xdr_bytes(), [(m.key, m.val) for m in top["signers"].map.sc_map]


def _key_order(k: xdr.SCVal) -> tuple:
    # Signer::External(verifier, key) -> vec [Symbol, Address, Bytes]; Soroban
    # orders ScVal vecs element by element: symbol, then the address, then the key.
    items = k.vec.sc_vec
    return tuple(i.to_xdr_bytes() for i in items)


def merge_signed(base_b64: str, signed_b64: list[str]) -> str:
    """Merge guardian signatures over the SAME entry into one entry."""
    base = _entry(base_b64)
    shape = _shape(base)
    rules: Optional[bytes] = None
    signers: dict[bytes, tuple] = {}
    for b in signed_b64:
        e = _entry(b)
        if _shape(e) != shape:
            raise Rejected("NOT_THIS_REQUEST", "a signature over a different entry (account, nonce, expiry or call)")
        r, pairs = _payload(e)
        if rules is None:
            rules = r
        elif r != rules:
            raise Rejected("NOT_THIS_REQUEST", "a signature under a different rule")
        for k, v in pairs:
            signers[k.to_xdr_bytes()] = (k, v)
    if rules is None:
        raise Rejected("UNSIGNED", "no guardian signature")
    ordered = sorted(signers.values(), key=lambda kv: _key_order(kv[0]))
    payload = xdr.SCVal(xdr.SCValType.SCV_MAP, map=xdr.SCMap([
        xdr.SCMapEntry(scval.to_symbol("context_rule_ids"), xdr.SCVal.from_xdr_bytes(rules)),
        xdr.SCMapEntry(scval.to_symbol("signers"), xdr.SCVal(xdr.SCValType.SCV_MAP, map=xdr.SCMap(
            [xdr.SCMapEntry(k, v) for k, v in ordered]))),
    ]))
    address_credentials(base.credentials).signature = payload
    return base.to_xdr()


def _is_proposal(account: str, func_b64: str, entry_b64: str) -> None:
    try:
        hf = xdr.HostFunction.from_xdr(func_b64)
    except Exception as ex:
        raise Rejected("INVALID_XDR", f"func: {ex}") from None
    if hf.type != xdr.HostFunctionType.HOST_FUNCTION_TYPE_INVOKE_CONTRACT:
        raise Rejected("NOT_A_PROPOSAL", "not a contract call")
    c = hf.invoke_contract
    if Address.from_xdr_sc_address(c.contract_address).address != account or c.function_name.sc_symbol != b"propose_recovery":
        raise Rejected("NOT_A_PROPOSAL", "only account.propose_recovery on the named account")
    e = _entry(entry_b64)
    if Address.from_xdr_sc_address(address_credentials(e.credentials).address).address != account:
        raise Rejected("NOT_A_PROPOSAL", "the entry must be the account's")
    fn = e.root_invocation.function
    if fn.contract_fn is None or fn.contract_fn.to_xdr_bytes() != c.to_xdr_bytes():
        raise Rejected("NOT_A_PROPOSAL", "the entry must authorize exactly this call")


class RecoveryMailbox:
    def __init__(self, path: Optional[str], clock: Callable[[], float] = time.time, max_open: int = 1000):
        self.path, self.clock, self.max_open = path, clock, max_open
        self._lock = threading.Lock()
        self._r: dict[str, dict] = {}
        if path and os.path.exists(path):
            with open(path) as f:
                self._r = json.load(f)

    def _save(self) -> None:
        if not self.path:
            return
        tmp = self.path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(self._r, f)
        os.replace(tmp, self.path)

    def _gc(self, now: float) -> None:
        for k in [k for k, v in self._r.items() if now - v["at"] > TTL]:
            del self._r[k]

    def open(self, account: str, new_key: str, threshold: int, func_b64: str, entry_b64: str) -> dict:
        if not isinstance(threshold, int) or threshold < 1:
            raise Rejected("INVALID_THRESHOLD", "threshold >= 1")
        if not (isinstance(new_key, str) and len(new_key) == 130 and new_key.startswith("04")):
            raise Rejected("INVALID_KEY", "new_key: 65-byte uncompressed P-256 key, hex")
        _is_proposal(account, func_b64, entry_b64)
        with self._lock:
            now = self.clock()
            self._gc(now)
            if len(self._r) >= self.max_open:
                raise Rejected("BUSY", "too many open recovery requests")
            rid = "".join(secrets.choice(ALPHABET) for _ in range(8))
            self._r[rid] = {"account": account, "new_key": new_key, "threshold": threshold, "func": func_b64,
                            "entry": entry_b64, "signed": {}, "at": now, "submitted": None}
            self._save()
            return {"id": rid}

    def get(self, rid: str) -> dict:
        with self._lock:
            self._gc(self.clock())
            r = self._r.get(rid)
            if r is None:
                raise Rejected("UNKNOWN_REQUEST", "no such recovery request (or it expired)")
            return {k: r[k] for k in ("account", "new_key", "threshold", "func", "entry", "submitted")} | {
                "id": rid, "approvals": sorted(r["signed"])}

    def approve(self, rid: str, signed_b64: str) -> dict:
        with self._lock:
            self._gc(self.clock())
            r = self._r.get(rid)
            if r is None:
                raise Rejected("UNKNOWN_REQUEST", "no such recovery request (or it expired)")
            merge_signed(r["entry"], [signed_b64] + list(r["signed"].values()))  # same entry, same rule
            for k, _ in _payload(_entry(signed_b64))[1]:
                r["signed"][bytes(scval.to_native(k)[2]).hex()] = signed_b64
            self._save()
            n = len(r["signed"])
            out = {"id": rid, "approvals": n, "threshold": r["threshold"], "ready": n >= r["threshold"], "func": r["func"]}
            if out["ready"]:
                out["entry"] = merge_signed(r["entry"], list(r["signed"].values()))
            return out

    def mark_submitted(self, rid: str, tx: str) -> None:
        with self._lock:
            if rid in self._r:
                self._r[rid]["submitted"] = tx
                self._save()


def b64(obj) -> str:
    return base64.b64encode(obj.to_xdr_bytes()).decode()
