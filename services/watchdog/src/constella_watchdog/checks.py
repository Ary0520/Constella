"""Health checks against the chain (simulation / ledger reads only; the
watchdog never signs or sends a transaction)."""
from __future__ import annotations

import json
import time
import urllib.request
from typing import Any, Dict, List, Optional

from stellar_sdk import Address, Keypair, SorobanServer, TransactionBuilder, scval, xdr

from constella_watchdog.alerts import Finding

DAY_LEDGERS = 17_280
STROOPS = 10_000_000


class Reader:
    def __init__(self, rpc_url: str, passphrase: str, source: str):
        self.server = SorobanServer(rpc_url)
        self.passphrase = passphrase
        self.source = source

    def read(self, contract: str, fn: str, *args: xdr.SCVal) -> Any:
        tx = (TransactionBuilder(self.server.load_account(self.source), self.passphrase, base_fee=100)
              .append_invoke_contract_function_op(contract, fn, list(args)).set_timeout(30).build())
        sim = self.server.simulate_transaction(tx)
        if sim.error:
            raise RuntimeError(f"{fn}: {sim.error[:200]}")
        return scval.to_native(xdr.SCVal.from_xdr(sim.results[0].xdr))

    def xlm_balance(self, account: str) -> Optional[float]:
        key = xdr.LedgerKey(type=xdr.LedgerEntryType.ACCOUNT,
                            account=xdr.LedgerKeyAccount(account_id=Keypair.from_public_key(account).xdr_account_id()))
        entries = self.server.get_ledger_entries([key]).entries or []
        if not entries:
            return None
        return xdr.LedgerEntryData.from_xdr(entries[0].xdr).account.balance.int64 / STROOPS

    def instance_days_left(self, contract: str) -> Optional[float]:
        key = xdr.LedgerKey(type=xdr.LedgerEntryType.CONTRACT_DATA, contract_data=xdr.LedgerKeyContractData(
            contract=Address(contract).to_xdr_sc_address(),
            key=xdr.SCVal(xdr.SCValType.SCV_LEDGER_KEY_CONTRACT_INSTANCE),
            durability=xdr.ContractDataDurability.PERSISTENT))
        resp = self.server.get_ledger_entries([key])
        if not resp.entries:
            return None
        return (resp.entries[0].live_until_ledger - resp.latest_ledger) / DAY_LEDGERS


def addr(a: Any) -> str:
    return getattr(a, "address", str(a))


def check_balance(r: Reader, label: str, account: str, min_xlm: float) -> Finding:
    bal = r.xlm_balance(account)
    key = f"balance:{label}"
    if bal is None:
        return Finding(key, False, f"{label} account {account[:8]}... does not exist")
    return Finding(key, bal >= min_xlm, f"{label} balance {bal:.2f} XLM (minimum {min_xlm:g})",
                   f"account {account}; below the minimum, sponsored / keeper transactions will stop.")


def check_vault(r: Reader, vault: str, state: Dict, now: float, claim_max_hours: float, ttl_min_days: float) -> List[Finding]:
    out: List[Finding] = []
    short = vault[:8] + "..."
    paused = bool(r.read(vault, "is_paused"))
    out.append(Finding(f"paused:{vault}", not paused, f"vault {short} is {'PAUSED' if paused else 'running'}",
                       "Deposits and rebalances are stopped until the pause authority unpauses."))
    cfg = r.read(vault, "get_config")
    oracle = addr(cfg["oracle_contract"])
    claims_seen = state.setdefault("claims_since", {})
    for s in r.read(vault, "get_sleeves"):
        raw = s["asset_id"]
        a, aid = addr(s["contract_address"]), raw.decode() if isinstance(raw, bytes) else str(raw)
        if int(s["current_balance"]) > 0 and int(s["sleeve_type"]) != 0:
            price = r.read(oracle, "asset_price", scval.to_address(a))
            out.append(Finding(f"price:{vault}:{aid}", price is not None,
                               f"{aid} in vault {short}: {'priced' if price else 'NO VALID PRICE'}",
                               "Deposits and normal redemptions halt while a funded sleeve is unpriced (frozen-sleeve "
                               "redemptions remain available). Check the oracle sources / circuit breaker."))
        total = int(r.read(vault, "get_claim_total", scval.to_address(a)))
        ck = f"{vault}:{aid}"
        if total > 0:
            claims_seen.setdefault(ck, now)
        else:
            claims_seen.pop(ck, None)
        age_h = (now - claims_seen[ck]) / 3600 if total > 0 else 0
        out.append(Finding(f"claims:{ck}", age_h <= claim_max_hours,
                           f"{aid} claims in vault {short} open for {age_h:.0f} h" if total > 0 else f"no open {aid} claims",
                           f"{total} units owed to holders; the keeper settles them once {aid} is priced and cash is available."))
    for label, c in [("vault", vault), ("policy", addr(cfg["policy_contract"])), ("oracle", oracle),
                     ("registry", addr(cfg["registry_contract"]))]:
        days = r.instance_days_left(c)
        out.append(Finding(f"rent:{c}", days is not None and days >= ttl_min_days,
                           f"{label} storage live for {days:.1f} more days" if days is not None else f"{label} storage ARCHIVED or missing",
                           "The keeper renews contract storage; if this drops, the keeper is not running."))
    return out


def check_relayer(url: str) -> Finding:
    try:
        with urllib.request.urlopen(url, timeout=10) as resp:
            ok = json.loads(resp.read()).get("ok") is True
    except Exception as e:
        return Finding("relayer:health", False, f"relayer health check failed ({type(e).__name__})", url)
    return Finding("relayer:health", ok, "relayer is " + ("healthy" if ok else "unhealthy"), url)
