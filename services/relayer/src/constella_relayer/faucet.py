"""Testnet faucet (TEST networks only): a passkey user has no XLM and cannot
use Circle's captcha faucet, so the relayer can send a small amount of test
USDC from a faucet account. Refuses to run on any network but testnet or a
local standalone network. Limits: per address and per client IP per day, and
a global daily cap (sliding 24 h windows)."""
from __future__ import annotations

from stellar_sdk import Address, Keypair, SorobanServer, TransactionBuilder, scval
from stellar_sdk.soroban_rpc import GetTransactionStatus, SendTransactionStatus

from .accounts import RateLimiter
from .gateway import Rejected

TEST_NETWORKS = ("Test SDF Network ; September 2015", "Standalone Network ; February 2017")


def parse_address(value) -> str:
    if not isinstance(value, str):
        raise Rejected("INVALID_ADDRESS", "address required")
    try:
        Address(value)
    except Exception:
        raise Rejected("INVALID_ADDRESS", "not a Stellar address") from None
    return value


class Faucet:
    def __init__(self, server: SorobanServer, passphrase: str, secret: str, asset: str, amount: int,
                 per_address: RateLimiter, per_client: RateLimiter):
        if passphrase not in TEST_NETWORKS:
            raise ValueError("the faucet runs on test networks only")
        self.server, self.passphrase, self.kp = server, passphrase, Keypair.from_secret(secret)
        self.asset, self.amount = asset, amount
        self.per_address, self.per_client = per_address, per_client

    def send(self, address: str, client: str) -> dict:
        self.per_address.check(address)
        self.per_client.check(client)
        acct = self.server.load_account(self.kp.public_key)
        tx = (TransactionBuilder(acct, self.passphrase, base_fee=1000)
              .append_invoke_contract_function_op(self.asset, "transfer",
                  [scval.to_address(self.kp.public_key), scval.to_address(address), scval.to_int128(self.amount)])
              .set_timeout(60).build())
        try:
            tx = self.server.prepare_transaction(tx)
        except Exception as e:
            # e.g. a classic G account without a USDC trustline
            raise Rejected("FAUCET_REFUSED", f"cannot send to this address (a G... account needs a USDC trustline first): {type(e).__name__}") from None
        tx.sign(self.kp)
        sent = self.server.send_transaction(tx)
        if sent.status not in (SendTransactionStatus.PENDING, SendTransactionStatus.DUPLICATE):
            raise Rejected("SEND_FAILED", str(sent.status))
        r = self.server.poll_transaction(sent.hash)
        if r.status != GetTransactionStatus.SUCCESS:
            raise Rejected("ONCHAIN_FAILED", sent.hash)
        self.per_address.count(address)
        self.per_client.count(client)
        return {"to": address, "amount": self.amount, "hash": sent.hash}
