"""S2 / F1 end-to-end driver, run by localnet_e2e.sh against a REAL local
Stellar network and a running relayer HTTP server. Everything is real: the
release WASMs, the passkey smart account's __check_auth (WebAuthn P-256), the
fee-sponsor quotas, and the network fees.

Modes:  keygen <pem-path>   -> prints the new passkey's 65-byte public key (hex)
        run                 -> the zero-gas proof
        paused              -> after an on-chain pause: refused, nothing spent
"""
from __future__ import annotations

import base64
import json
import os
import sys
import urllib.error
import urllib.request

from stellar_sdk import Keypair, SorobanServer, TransactionBuilder, scval, xdr

from constella_relayer.passkey import SoftwarePasskey, sign_auth_entry
from constella_relayer.relayer import func_b64

E = os.environ


def fail(msg: str) -> None:
    print(f"E2E FAIL: {msg}", flush=True)
    sys.exit(1)


def post(path: str, body: dict) -> tuple[int, dict]:
    req = urllib.request.Request(E["RELAYER_URL"] + path, data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


class Chain:
    def __init__(self):
        self.server = SorobanServer(E["RPC"])
        self.viewer = E["VIEWER"]  # any funded account, used only as simulation source

    def view(self, contract: str, fn: str, args: list[xdr.SCVal]):
        acct = self.server.load_account(self.viewer)
        tx = (TransactionBuilder(acct, E["PASS"], base_fee=100)
              .append_invoke_contract_function_op(contract, fn, args).set_timeout(30).build())
        sim = self.server.simulate_transaction(tx)
        if sim.error:
            fail(f"view {fn}: {sim.error}")
        return scval.to_native(xdr.SCVal.from_xdr(sim.results[0].xdr))

    def balance(self, token: str, who: str) -> int:
        return int(self.view(token, "balance", [scval.to_address(who)]))


def signed_request(chain: Chain, passkey: SoftwarePasskey, amount: int, acct: str | None = None) -> dict:
    acct = acct or E["ACCOUNT"]
    a = scval.to_address(acct)
    func = func_b64(E["VAULT"], "deposit", [scval.to_int128(amount), a, a, a])
    code, prep = post("/prepare", {"func": func})
    if code != 200:
        fail(f"/prepare: {code} {prep}")
    rule = int(chain.view(acct, "get_owner_rule", []))
    until = prep["latest_ledger"] + 100
    auth = []
    for b64 in prep["auth"]:
        entry = xdr.SorobanAuthorizationEntry.from_xdr(b64)
        signed = sign_auth_entry(entry, passkey, E["VERIFIER"], rule, until, E["PASS"])
        auth.append(signed.to_xdr())
    return {"func": func, "auth": auth}


def run() -> None:
    chain = Chain()
    owner = SoftwarePasskey.from_pem(open(E["PASSKEY_PEM"], "rb").read())
    acct, vault, usdc, native = E["ACCOUNT"], E["VAULT"], E["USDC"], E["NATIVE"]
    relayers = E["RELAYERS"].split(",")
    sponsor = E["SPONSOR"]
    one = 10_000_000

    usdc_before = chain.balance(usdc, acct)
    xlm_user_before = chain.balance(native, acct)
    xlm_relayers_before = {r: chain.balance(native, r) for r in relayers}
    print(f"user {acct}: {usdc_before / one} USDC, {xlm_user_before} stroops XLM; relayer pool {xlm_relayers_before}")
    if xlm_user_before != 0:
        fail("the user must start with zero XLM")

    # --- refusals that must cost nothing -----------------------------------
    stranger_req = signed_request(chain, SoftwarePasskey(), 100 * one)
    code, res = post("/submit", stranger_req)
    print("stranger passkey ->", code, res.get("error"))
    if code != 400 or res.get("error") != "SIMULATION_FAILED":
        fail(f"a stranger's passkey must be refused at simulation: {code} {res}")

    drain = signed_request(chain, owner, 100 * one)
    e0 = xdr.SorobanAuthorizationEntry.from_xdr(drain["auth"][0])
    e0.credentials = xdr.SorobanCredentials(type=xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_SOURCE_ACCOUNT)
    code, res = post("/submit", {"func": drain["func"], "auth": [e0.to_xdr()]})
    print("source-account drain ->", code, res.get("error"))
    if res.get("error") != "SOURCE_ACCOUNT_AUTH_FORBIDDEN":
        fail(f"source-account credentials must be refused: {code} {res}")

    code, res = post("/submit", {"func": func_b64(vault, "set_keeper", [scval.to_address(acct)]), "auth": drain["auth"]})
    print("non-allow-listed function ->", code, res.get("error"))
    if res.get("error") != "FUNCTION_NOT_ALLOWED":
        fail(f"set_keeper must not be sponsored: {code} {res}")

    stats = chain.view(sponsor, "get_user_stats", [scval.to_address(acct)])
    if stats["daily_ops"] != 0:
        fail(f"refused requests consumed quota: {stats}")
    if {r: chain.balance(native, r) for r in relayers} != xlm_relayers_before:
        fail("refused requests cost the relayers XLM")

    # --- the sponsored deposit ---------------------------------------------
    req = signed_request(chain, owner, 500 * one)
    code, res = post("/submit", req)
    print("sponsored deposit ->", code, res)
    if code != 200:
        fail(f"sponsored deposit failed: {code} {res}")

    shares = chain.balance(vault, acct)
    usdc_after = chain.balance(usdc, acct)
    xlm_user_after = chain.balance(native, acct)
    payer = res["relayer"]
    if payer not in relayers:
        fail(f"paid by an account outside the pool: {payer}")
    xlm_relayer_before = xlm_relayers_before[payer]
    xlm_relayer_after = chain.balance(native, payer)
    paid = res["fee_charged"] + res["record_fee_charged"]
    print(f"shares {shares / one}, USDC {usdc_after / one}, user XLM {xlm_user_after}, "
          f"relayer paid {xlm_relayer_before - xlm_relayer_after} stroops (deposit {res['fee_charged']} + record {res['record_fee_charged']})")
    if shares != 500 * one:
        fail(f"expected 500 shares, got {shares}")
    if usdc_after != usdc_before - 500 * one:
        fail("USDC not debited exactly")
    if xlm_user_after != 0:
        fail("the user paid XLM")
    if res["fee_charged"] <= 0:
        fail("no network fee was charged: not a real fee proof")
    if xlm_relayer_before - xlm_relayer_after != paid:
        fail("the relayer's XLM did not drop by exactly the fees charged")
    stats = chain.view(sponsor, "get_user_stats", [scval.to_address(acct)])
    if stats["daily_ops"] != 1:
        fail(f"the sponsored op was not recorded on chain: {stats}")

    print("ZERO-GAS DEPOSIT PASS")


def parallel() -> None:
    """Decision D-I: two users submit at the same time; the pool serves them
    on two different relayer accounts, each paying its own fees."""
    import threading

    chain = Chain()
    users = [(E["ACCOUNT"], SoftwarePasskey.from_pem(open(E["PASSKEY_PEM"], "rb").read())),
             (E["ACCOUNT2"], SoftwarePasskey.from_pem(open(E["PASSKEY2_PEM"], "rb").read()))]
    one = 10_000_000
    shares_before = [chain.balance(E["VAULT"], a) for a, _ in users]
    reqs = [signed_request(chain, pk, 100 * one, acct=a) for a, pk in users]
    results: list = [None, None]

    def go(i: int) -> None:
        results[i] = post("/submit", reqs[i])

    threads = [threading.Thread(target=go, args=(i,)) for i in range(2)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    for i, (code, res) in enumerate(results):
        print(f"parallel user {i + 1} ->", code, res.get("relayer"), res.get("error"))
        if code != 200:
            fail(f"parallel submit {i + 1} failed: {code} {res}")
    payers = {res["relayer"] for _, res in results}
    if len(payers) != 2:
        fail(f"the two requests were not served by two pool accounts: {payers}")
    for i, (a, _) in enumerate(users):
        if chain.balance(E["VAULT"], a) != shares_before[i] + 100 * one:
            fail(f"user {i + 1} shares not minted")
        if chain.balance(E["NATIVE"], a) != 0:
            fail(f"user {i + 1} paid XLM")
    print("PARALLEL PASS")


def paused() -> None:
    """After the admin paused the sponsor on chain: refused, nothing spent."""
    chain = Chain()
    owner = SoftwarePasskey.from_pem(open(E["PASSKEY_PEM"], "rb").read())
    relayers = E["RELAYERS"].split(",")
    before = {r: chain.balance(E["NATIVE"], r) for r in relayers}
    code, res = post("/submit", signed_request(chain, owner, 10 * 10_000_000))
    print("paused sponsor ->", code, res.get("error"))
    if res.get("error") != "SPONSOR_REFUSED":
        fail(f"a paused sponsor must refuse: {code} {res}")
    if {r: chain.balance(E["NATIVE"], r) for r in relayers} != before:
        fail("a refused request cost the relayers XLM")
    print("PAUSED PASS")


if __name__ == "__main__":
    if sys.argv[1] == "keygen":
        pk = SoftwarePasskey()
        open(sys.argv[2], "wb").write(pk.private_pem())
        print(pk.public_key.hex())
    elif sys.argv[1] == "run":
        run()
    elif sys.argv[1] == "parallel":
        parallel()
    elif sys.argv[1] == "paused":
        paused()
    else:
        fail("usage: keygen <pem> | run")
