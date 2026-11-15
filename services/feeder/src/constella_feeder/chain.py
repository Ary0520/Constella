"""Minimal Soroban client for the Constella NAV oracle's feeder calls."""
from __future__ import annotations

from dataclasses import dataclass

from stellar_sdk import Keypair, SorobanServer, TransactionBuilder, scval
from stellar_sdk.soroban_rpc import GetTransactionStatus, SendTransactionStatus
from stellar_sdk.xdr import SCVal

from .pricing import OnChain, Update


class ChainError(RuntimeError):
    pass


@dataclass(frozen=True)
class Network:
    name: str
    rpc_url: str
    passphrase: str


NETWORKS = {
    "testnet": Network("testnet", "https://soroban-testnet.stellar.org", "Test SDF Network ; September 2015"),
    "mainnet": Network("mainnet", "https://mainnet.sorobanrpc.com", "Public Global Stellar Network ; September 2015"),
    "local": Network("local", "http://localhost:8000/soroban/rpc", "Standalone Network ; February 2017"),
}


class OracleClient:
    def __init__(self, network: Network, oracle_id: str, rpc_url: str | None = None):
        self.network = network
        self.oracle_id = oracle_id
        self.server = SorobanServer(rpc_url or network.rpc_url)

    def _tx(self, source: str, fn: str, args: list[SCVal]):
        account = self.server.load_account(source)
        return (
            TransactionBuilder(account, self.network.passphrase, base_fee=100)
            .append_invoke_contract_function_op(self.oracle_id, fn, args)
            .set_timeout(60)
            .build()
        )

    def get_source(self, any_account: str, source_id: str) -> OnChain | None:
        """Read-only simulation of `get_source(source_id)`."""
        sim = self.server.simulate_transaction(self._tx(any_account, "get_source", [scval.to_symbol(source_id)]))
        if sim.error:
            raise ChainError(f"get_source simulation failed: {sim.error}")
        value = scval.to_native(SCVal.from_xdr(sim.results[0].xdr))
        if value is None:
            return None
        return OnChain(price=int(value["price"]), timestamp=int(value["timestamp"]))

    def update_price(self, feeder: Keypair, source_id: str, update: Update) -> str:
        """Submit `update_price` signed by the feeder (tx source = feeder, so
        the contract's `feeder.require_auth()` is satisfied by the source
        account). Returns the transaction hash once it succeeded."""
        args = [
            scval.to_symbol(source_id),
            scval.to_int128(update.price),
            scval.to_int128(update.liquidity_depth_usd),
            scval.to_uint64(update.timestamp),
        ]
        tx = self.server.prepare_transaction(self._tx(feeder.public_key, "update_price", args))
        tx.sign(feeder)
        sent = self.server.send_transaction(tx)
        if sent.status not in (SendTransactionStatus.PENDING, SendTransactionStatus.DUPLICATE):
            raise ChainError(f"send failed: {sent.status} {sent.error_result_xdr}")
        result = self.server.poll_transaction(sent.hash)
        if result.status != GetTransactionStatus.SUCCESS:
            raise ChainError(f"transaction {sent.hash} ended {result.status}")
        return sent.hash
