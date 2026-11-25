"""Step 6 live proof: flow netting by the HOSTED keeper on Stellar testnet.

A fresh user deposits into a live vault; the script prints the plan the
netting planner makes for the new state (what should trade and what should
not), then waits for the keeper's rebalance and checks that exactly the
predicted sleeves moved (deposit cash buys only - nothing is sold; sleeves
within half their band are untouched).

  python testnet/e2e_netting.py VAULT AMOUNT_USDC [--wait SECONDS]
"""
from __future__ import annotations

import importlib.util
import json
import os
import sys
import time
import urllib.request
from pathlib import Path

from stellar_sdk import Asset, Keypair, Network

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "services/keeper/src"))
from constella_keeper.chain import Chain  # noqa: E402
from constella_keeper.planner import plan, weights  # noqa: E402

spec = importlib.util.spec_from_file_location("ld", ROOT / "localnet/deploy.py")
ld = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ld)
TESTNET = Network.TESTNET_NETWORK_PASSPHRASE
ONE = 10 ** 7
say = lambda *a: print(*a, flush=True)


def post(url: str, body: dict) -> dict:
    req = urllib.request.Request(url, json.dumps(body).encode(), {"Content-Type": "application/json", "User-Agent": "constella-e2e"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())


def main() -> None:
    vault, amount = sys.argv[1], float(sys.argv[2])
    wait = int(sys.argv[sys.argv.index("--wait") + 1]) if "--wait" in sys.argv else 900
    cfg = json.loads((ROOT / "sandbox/config.testnet.json").read_text())
    usdc = cfg["assets"]["USDC"]
    net = ld.Net(cfg["network"]["rpc_url"], cfg["network"]["friendbot"], TESTNET, allowed=(TESTNET,))
    keeper_secret = json.loads((Path.home() / ".config/constella/testnet/keys.json").read_text())["keeper"]
    ch = Chain(cfg["network"]["rpc_url"], TESTNET, keeper_secret)  # read-only use here
    held = lambda st: {s.asset_id: s.held for s in st["sleeves"]}

    user = Keypair.random()
    net.fund(user)
    issuer = str(net.view(usdc, "name", [], user.public_key)).split(":")[1]
    net.classic(user, lambda b: b.append_change_trust_op(Asset("USDC", issuer)))
    post(cfg["services"]["relayer"] + "/faucet", {"address": user.public_key})
    net.invoke(user, vault, "deposit", [ld.I128(int(amount * ONE)), ld.A(user.public_key), ld.A(user.public_key), ld.A(user.public_key)])
    st = ch.vault_state(vault)
    before = held(st)
    say(f"deposited {amount} USDC; weights now", dict(zip([s.asset_id for s in st["sleeves"]], weights(st["sleeves"], st["scale"]))))

    p = plan(st["sleeves"], st["scale"], st["band_bps"], st["max_slippage_bps"],
             lambda a, b, amt: ch.soroswap_quote(st["router"], a, b, amt) if st["router"] else 0)
    if p is None:
        say("nothing breaches its band: no trade expected (netted in cash)")
        return
    by_addr = {s.address: s.asset_id for s in st["sleeves"]}
    legs = [(by_addr[o.asset_in], by_addr[o.asset_out], o.amount_in) for o in p.operations]
    expect = {a for a, b, _ in legs} | {b for a, b, _ in legs}
    untouched = set(before) - expect
    say("netting plan:", legs, "| untouched:", sorted(untouched))
    cash = next(s.asset_id for s in st["sleeves"] if s.is_cash)
    assert all(a == cash for a, _, _ in legs), "a deposit must not cause a sale"

    t0 = time.time()
    while time.time() - t0 < wait:
        time.sleep(20)
        now = held(ch.vault_state(vault))
        moved = {k for k in now if now[k] != before[k]}
        if moved - {cash}:
            say("keeper rebalanced:", {k: (before[k] / ONE, now[k] / ONE) for k in moved})
            assert moved - {cash} == expect - {cash}, f"moved {moved}, expected {expect}"
            assert all(now[k] >= before[k] for k in moved if k != cash), "nothing was sold"
            say("NETTING OK - untouched:", sorted(untouched))
            return
    raise SystemExit(f"keeper did not rebalance within {wait}s")


if __name__ == "__main__":
    main()
