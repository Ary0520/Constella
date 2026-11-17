"""Soroban reads (simulation only) and the keeper's submissions."""
from __future__ import annotations

import time
from typing import Any, Dict, List, Optional, Sequence

from stellar_sdk import Address, Keypair, SorobanDataBuilder, SorobanServer, TransactionBuilder, scval, xdr
from stellar_sdk.soroban_rpc import EventFilter, EventFilterType, GetTransactionStatus, SendTransactionStatus

from constella_keeper.planner import Plan, Sleeve

SOROSWAP = 1
CASH_STABLE = 0


class ChainError(RuntimeError):
    pass


def struct(fields: Dict[str, xdr.SCVal]) -> xdr.SCVal:
    """A #[contracttype] struct: a map with symbol keys in sorted order."""
    return scval.to_map({scval.to_symbol(k): fields[k] for k in sorted(fields)})


def addr(a: Any) -> str:
    return getattr(a, "address", str(a))


def proposal_scval(vault: str, policy_hash: bytes, plan: Plan, deadline: int, keeper: str) -> xdr.SCVal:
    ops = [struct({
        "asset_in": scval.to_address(t.asset_in), "asset_out": scval.to_address(t.asset_out),
        "amount_in": scval.to_int128(t.amount_in), "min_amount_out": scval.to_int128(t.min_amount_out),
        "route": scval.to_uint32(t.route),
    }) for t in plan.operations]
    weights = [scval.to_vec([scval.to_symbol(a), scval.to_uint32(w)]) for a, w in plan.weights_after]
    return struct({
        "vault": scval.to_address(vault), "policy_hash": scval.to_bytes(policy_hash),
        "operations": scval.to_vec(ops), "weights_after": scval.to_vec(weights),
        "deadline": scval.to_uint64(deadline), "proposer": scval.to_address(keeper),
    })


class Chain:
    def __init__(self, rpc_url: str, passphrase: str, keeper_secret: str, timeout_s: int = 60):
        self.server = SorobanServer(rpc_url)
        self.passphrase = passphrase
        self.kp = Keypair.from_secret(keeper_secret)
        self.keeper = self.kp.public_key
        self.timeout_s = timeout_s

    # ---------------------------------------------------------------- reads
    def _tx(self, contract: str, fn: str, args: Sequence[xdr.SCVal]):
        acct = self.server.load_account(self.keeper)
        return (TransactionBuilder(acct, self.passphrase, base_fee=100)
                .append_invoke_contract_function_op(contract, fn, list(args)).set_timeout(60).build())

    def read(self, contract: str, fn: str, *args: xdr.SCVal) -> Any:
        sim = self.server.simulate_transaction(self._tx(contract, fn, args))
        if sim.error:
            raise ChainError(f"{fn}: {sim.error[:300]}")
        return scval.to_native(xdr.SCVal.from_xdr(sim.results[0].xdr))

    def try_read(self, contract: str, fn: str, *args: xdr.SCVal) -> Optional[Any]:
        try:
            return self.read(contract, fn, *args)
        except ChainError:
            return None

    def latest_ledger(self) -> int:
        return self.server.get_latest_ledger().sequence

    def oldest_ledger(self) -> int:
        """Oldest ledger the RPC still serves events for (its retention window)."""
        return int(self.server.get_health().oldest_ledger)

    def ledger_time(self) -> int:
        # Close time of the latest ledger, from the RPC (never the local clock).
        info = self.server.get_latest_ledger()
        meta = getattr(info, "close_time", None)
        return int(meta) if meta else int(time.time())

    def vault_state(self, vault: str) -> Dict[str, Any]:
        cfg = self.read(vault, "get_config")
        oracle, policy = addr(cfg["oracle_contract"]), addr(cfg["policy_contract"])
        scale = 10 ** int(self.read(vault, "decimals"))
        pcfg = self.read(policy, "get_policy_config", scalar_addr(vault))
        if pcfg is None:
            raise ChainError("no policy bound")
        sleeves: List[Sleeve] = []
        for s in self.read(vault, "get_sleeves"):
            a = addr(s["contract_address"])
            is_cash = int(s["sleeve_type"]) == CASH_STABLE
            blend = self.try_read(oracle, "blend_position", scalar_addr(a))
            booked = int(s["current_balance"])
            if blend:  # bTokens: the vault reads them from the pool; booked is the upper bound
                held = booked
                dec_token = addr(blend["underlying"])
            else:
                real = int(self.read(a, "balance", scalar_addr(vault)))
                held = min(booked, real)
                dec_token = a
            px = self.try_read(oracle, "asset_price", scalar_addr(a))
            sleeves.append(Sleeve(
                asset_id=str(s["asset_id"].decode() if isinstance(s["asset_id"], bytes) else s["asset_id"]),
                address=a, target_bps=int(s["target_weight_bps"]), band_bps=int(s["band_bps"]), held=held,
                price=int(px["price"]) if px else None, is_cash=is_cash, is_blend=bool(blend),
                decimals=int(self.read(dec_token, "decimals")),
            ))
        h = self.read(vault, "get_policy_hash")
        return {
            "sleeves": sleeves, "scale": scale, "policy": policy, "oracle": oracle,
            "band_bps": int(pcfg["rebalance_band_bps"]), "max_slippage_bps": int(pcfg["max_slippage_bps"]),
            "policy_hash": bytes(h) if h is not None else None,
            "paused": bool(self.read(vault, "is_paused")),
            "keeper": addr(self.try_read(vault, "get_keeper") or ""),
            "router": addr(self.try_read(vault, "get_router", scval.to_uint32(SOROSWAP)) or ""),
        }

    def soroswap_quote(self, router: str, asset_in: str, asset_out: str, amount_in: int) -> int:
        out = self.read(router, "router_get_amounts_out", scval.to_int128(amount_in),
                        scval.to_vec([scval.to_address(asset_in), scval.to_address(asset_out)]))
        return int(out[-1])

    def validate(self, policy: str, vault: str, proposal: xdr.SCVal) -> Dict[str, Any]:
        return self.read(policy, "validate_rebalance", scalar_addr(vault), proposal)

    # ---------------------------------------------------------------- writes
    def submit(self, contract: str, fn: str, *args: xdr.SCVal) -> Dict[str, Any]:
        """Simulate (enforcing), sign as the keeper, send, wait. Nothing is
        sent if the simulation fails."""
        tx = self._tx(contract, fn, args)
        sim = self.server.simulate_transaction(tx)
        if sim.error:
            raise ChainError(f"{fn} simulation failed: {sim.error[:300]}")
        tx = self.server.prepare_transaction(tx, sim)
        tx.sign(self.kp)
        sent = self.server.send_transaction(tx)
        # An enum, not a string (a string comparison reported a PENDING send as
        # "not accepted" in the first local-network run).
        if sent.status not in (SendTransactionStatus.PENDING, SendTransactionStatus.DUPLICATE):
            raise ChainError(f"{fn} not accepted: {sent.status} {getattr(sent, 'error_result_xdr', '')}")
        deadline = time.time() + self.timeout_s
        while time.time() < deadline:
            r = self.server.get_transaction(sent.hash)
            if r.status == GetTransactionStatus.SUCCESS:
                return {"hash": sent.hash, "ledger": r.ledger}
            if r.status == GetTransactionStatus.FAILED:
                raise ChainError(f"{fn} failed on chain: {sent.hash}")
            time.sleep(1)
        raise ChainError(f"{fn} not confirmed within {self.timeout_s}s: {sent.hash}")

    # ---------------------------------------------------------------- rent
    def live_until(self, keys: List[xdr.LedgerKey]) -> Dict[str, Any]:
        """{key_xdr: (live_until_ledger, entry_xdr)} for entries that exist."""
        out: Dict[str, Any] = {}
        for i in range(0, len(keys), 50):
            resp = self.server.get_ledger_entries(keys[i:i + 50])
            for e in resp.entries or []:
                out[e.key] = (e.live_until_ledger, e.xdr)
        return out

    def wasm_hash(self, instance_entry_xdr: str) -> bytes:
        data = xdr.LedgerEntryData.from_xdr(instance_entry_xdr)
        return bytes(data.contract_data.val.instance.executable.wasm_hash.hash)

    def extend(self, keys: List[xdr.LedgerKey], extend_to: int) -> Dict[str, Any]:
        """ExtendFootprintTTL (permissionless; the keeper pays) for `keys`."""
        acct = self.server.load_account(self.keeper)
        data = SorobanDataBuilder().set_read_only(keys).build()
        tx = (TransactionBuilder(acct, self.passphrase, base_fee=100).append_extend_footprint_ttl_op(extend_to)
              .set_soroban_data(data).set_timeout(60).build())
        sim = self.server.simulate_transaction(tx)
        if sim.error:
            raise ChainError(f"extend simulation failed: {sim.error[:300]}")
        tx = self.server.prepare_transaction(tx, sim)
        tx.sign(self.kp)
        sent = self.server.send_transaction(tx)
        if sent.status not in (SendTransactionStatus.PENDING, SendTransactionStatus.DUPLICATE):
            raise ChainError(f"extend not accepted: {sent.status}")
        deadline = time.time() + self.timeout_s
        while time.time() < deadline:
            r = self.server.get_transaction(sent.hash)
            if r.status == GetTransactionStatus.SUCCESS:
                return {"hash": sent.hash, "ledger": r.ledger}
            if r.status == GetTransactionStatus.FAILED:
                raise ChainError(f"extend failed on chain: {sent.hash}")
            time.sleep(1)
        raise ChainError(f"extend not confirmed: {sent.hash}")

    # ---------------------------------------------------------------- claims
    def claim_events(self, vault: str, start_ledger: Optional[int], cursor: Optional[str]):
        """(holder, asset) pairs from CLAIM_RECORDED events, and the next cursor."""
        topic = scval.to_symbol("CLAIM_RECORDED").to_xdr()
        f = EventFilter(type=EventFilterType.CONTRACT, contractIds=[vault], topics=[[topic, "*", "*"]])
        kw = {"cursor": cursor} if cursor else {"start_ledger": start_ledger}
        resp = self.server.get_events(filters=[f], limit=200, **kw)
        pairs = []
        for e in resp.events:
            t = [scval.to_native(xdr.SCVal.from_xdr(x)) for x in e.topic]
            pairs.append((addr(t[1]), addr(t[2])))
        return pairs, getattr(resp, "cursor", None)


# ------------------------------------------------------------ storage rent (A1)
def instance_key(contract: str) -> xdr.LedgerKey:
    return data_key(contract, xdr.SCVal(xdr.SCValType.SCV_LEDGER_KEY_CONTRACT_INSTANCE))


def data_key(contract: str, key: xdr.SCVal) -> xdr.LedgerKey:
    return xdr.LedgerKey(type=xdr.LedgerEntryType.CONTRACT_DATA, contract_data=xdr.LedgerKeyContractData(
        contract=Address(contract).to_xdr_sc_address(), key=key, durability=xdr.ContractDataDurability.PERSISTENT))


def code_key(wasm_hash: bytes) -> xdr.LedgerKey:
    return xdr.LedgerKey(type=xdr.LedgerEntryType.CONTRACT_CODE,
                         contract_code=xdr.LedgerKeyContractCode(hash=xdr.Hash(wasm_hash)))


def enum_key(variant: str, *fields: xdr.SCVal) -> xdr.SCVal:
    """A #[contracttype] tuple-variant key, e.g. DataKey::PolicyConfig(vault)."""
    return scval.to_vec([scval.to_symbol(variant), *fields])


def scalar_addr(a: str) -> xdr.SCVal:
    return scval.to_address(a)
