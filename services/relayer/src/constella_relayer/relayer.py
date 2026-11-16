"""Constella sponsor relayer (audit F1): pays the network fee for a user's
Soroban call, within on-chain quotas.

Decision D-I: a POOL of relayer accounts (all in the fee-sponsor's relayer
set). Each account processes one request at a time under its own lock, so
throughput grows with the pool while every guarantee below holds per account.

Flow for `submit(func, auth)` on a free pool account:
  1. `gateway.validate` - pure request checks (see gateway.py).
  2. The target must be whitelisted on the fee-sponsor contract (on-chain
     source of truth, not local config alone).
  3. The user transaction (source = relayer, sequence S+2) is prepared with
     an ENFORCING simulation of the user's signed auth entries - this runs the
     smart account's real `__check_auth`, so a bad signature is refused
     before anything is spent. Its final fee F must be <= max_fee.
  4. `record_sponsored_op(user, target, F)` (sequence S+1) is submitted
     first: whitelist, pause, per-user quotas and the daily budget are
     enforced on chain before the relayer pays for the user's call.
  5. The prepared user transaction is signed and submitted.

The user never needs XLM: the relayer account is the transaction source and
pays every fee; the user only signs its Soroban authorization entries.
"""
from __future__ import annotations

import base64
import threading
from dataclasses import dataclass

from stellar_sdk import Account, Address, Keypair, SorobanServer, TransactionBuilder, scval, xdr
from stellar_sdk.exceptions import PrepareTransactionException
from stellar_sdk.operation import InvokeHostFunction
from stellar_sdk.soroban_rpc import AuthMode, GetTransactionStatus, SendTransactionStatus

from .gateway import MAX_FUNC_XDR_BYTES, Rejected, _decode, validate


@dataclass(frozen=True)
class Submitted:
    relayer: str
    hash: str
    user: str
    contract: str
    function: str
    fee_charged: int
    record_hash: str
    record_fee_charged: int


class Relayer:
    def __init__(
        self,
        server: SorobanServer,
        network_passphrase: str,
        keypairs: list[Keypair],
        sponsor_contract: str,
        allowed: dict[str, frozenset[str]],
        max_fee_stroops: int,
        inclusion_fee: int = 100,
        timeout_seconds: int = 30,
    ):
        if max_fee_stroops <= inclusion_fee:
            raise ValueError("max_fee_stroops must exceed the inclusion fee")
        if not keypairs or len({k.public_key for k in keypairs}) != len(keypairs):
            raise ValueError("need one or more distinct relayer keypairs")
        self.server = server
        self.passphrase = network_passphrase
        self.keypairs = list(keypairs)
        self.sponsor = sponsor_contract
        self.allowed = allowed
        self.max_fee = max_fee_stroops
        self.inclusion_fee = inclusion_fee
        self.timeout = timeout_seconds
        self.relayer_addresses = frozenset(k.public_key for k in keypairs)
        self._locks = [threading.Lock() for _ in keypairs]
        self._next = 0
        self._pick = threading.Lock()

    # ---------------------------------------------------------------- helpers
    def _builder(self, source: str, sequence: int) -> TransactionBuilder:
        return TransactionBuilder(Account(source, sequence), self.passphrase,
                                  base_fee=self.inclusion_fee).set_timeout(self.timeout)

    def _acquire(self) -> int:
        """A free pool account (its lock held), else wait on the next in turn."""
        for i, lock in enumerate(self._locks):
            if lock.acquire(blocking=False):
                return i
        with self._pick:
            i = self._next
            self._next = (self._next + 1) % len(self._locks)
        self._locks[i].acquire()
        return i

    def _view(self, fn: str, args: list[xdr.SCVal]):
        source = self.keypairs[0].public_key
        seq = self.server.load_account(source).sequence
        tx = self._builder(source, seq).append_invoke_contract_function_op(self.sponsor, fn, args).build()
        sim = self.server.simulate_transaction(tx)
        if sim.error:
            raise Rejected("SPONSOR_UNAVAILABLE", f"{fn}: {sim.error}")
        return scval.to_native(xdr.SCVal.from_xdr(sim.results[0].xdr))

    def _send(self, tx, kp: Keypair) -> tuple[str, int]:
        tx.sign(kp)
        sent = self.server.send_transaction(tx)
        if sent.status not in (SendTransactionStatus.PENDING, SendTransactionStatus.DUPLICATE):
            raise Rejected("SEND_FAILED", f"{sent.status} {sent.error_result_xdr}")
        result = self.server.poll_transaction(sent.hash)
        if result.status != GetTransactionStatus.SUCCESS:
            raise Rejected("ONCHAIN_FAILED", f"{sent.hash} ended {result.status}")
        fee = xdr.TransactionResult.from_xdr(result.result_xdr).fee_charged.int64
        return sent.hash, fee

    # ------------------------------------------------------------- public API
    def prepare(self, func_b64: str) -> dict:
        """Client helper: the unsigned auth entries the user must sign for
        `func` (recording simulation with the relayer as source) and the
        latest ledger, to choose `valid_until_ledger`. Nothing is submitted."""
        try:
            host_fn = xdr.HostFunction.from_xdr_bytes(_decode(func_b64, MAX_FUNC_XDR_BYTES, "func"))
        except Rejected:
            raise
        except Exception as e:
            raise Rejected("INVALID_XDR", f"func: {e}") from None
        if host_fn.type != xdr.HostFunctionType.HOST_FUNCTION_TYPE_INVOKE_CONTRACT:
            raise Rejected("NOT_A_CONTRACT_CALL", str(host_fn.type))
        contract = Address.from_xdr_sc_address(host_fn.invoke_contract.contract_address).address
        function = host_fn.invoke_contract.function_name.sc_symbol.decode()
        if function not in self.allowed.get(contract, frozenset()):
            raise Rejected("FUNCTION_NOT_ALLOWED", f"{contract}.{function}")
        source = self.keypairs[0].public_key
        seq = self.server.load_account(source).sequence
        tx = self._builder(source, seq).append_operation(InvokeHostFunction(host_function=host_fn, auth=[])).build()
        sim = self.server.simulate_transaction(tx, auth_mode=AuthMode.RECORD)
        if sim.error:
            raise Rejected("SIMULATION_FAILED", sim.error)
        return {"auth": list(sim.results[0].auth or []), "latest_ledger": sim.latest_ledger}

    def submit(self, func_b64: str, auth_b64: list[str]) -> Submitted:
        v = validate(func_b64, auth_b64, self.allowed, self.relayer_addresses)
        if self._view("is_whitelisted", [scval.to_address(v.contract)]) is not True:
            raise Rejected("TARGET_NOT_WHITELISTED", v.contract)
        i = self._acquire()
        try:
            kp = self.keypairs[i]
            me = kp.public_key
            seq = self.server.load_account(me).sequence
            user_tx = self._builder(me, seq + 1).append_operation(
                InvokeHostFunction(host_function=v.host_function, auth=v.auth)).build()
            try:
                # Auth entries are present, so the simulation ENFORCES them:
                # the account's real __check_auth runs here.
                prepared = self.server.prepare_transaction(user_tx)
            except PrepareTransactionException as e:
                raise Rejected("SIMULATION_FAILED", str(e.simulate_transaction_response.error)) from None
            fee = prepared.transaction.fee
            if fee > self.max_fee:
                raise Rejected("FEE_TOO_HIGH", f"{fee} > {self.max_fee}")
            record = self._builder(me, seq).append_invoke_contract_function_op(
                self.sponsor, "record_sponsored_op",
                [scval.to_address(me), scval.to_address(v.user), scval.to_address(v.contract), scval.to_uint64(fee)]).build()
            try:
                record = self.server.prepare_transaction(record)
            except PrepareTransactionException as e:
                raise Rejected("SPONSOR_REFUSED", str(e.simulate_transaction_response.error)) from None
            record_hash, record_fee = self._send(record, kp)
            tx_hash, fee_charged = self._send(prepared, kp)
            return Submitted(relayer=me, hash=tx_hash, user=v.user, contract=v.contract, function=v.function,
                             fee_charged=fee_charged, record_hash=record_hash, record_fee_charged=record_fee)
        finally:
            self._locks[i].release()

def func_b64(contract: str, function: str, args: list[xdr.SCVal]) -> str:
    """Encode an InvokeContract host function (what a client sends)."""
    hf = xdr.HostFunction(
        type=xdr.HostFunctionType.HOST_FUNCTION_TYPE_INVOKE_CONTRACT,
        invoke_contract=xdr.InvokeContractArgs(contract_address=Address(contract).to_xdr_sc_address(),
                                               function_name=xdr.SCSymbol(function.encode()), args=args),
    )
    return base64.b64encode(hf.to_xdr_bytes()).decode()
