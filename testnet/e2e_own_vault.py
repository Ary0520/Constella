#!/usr/bin/env python3
"""Step 3 live proof on Stellar testnet: a thesis becomes a user's own vault.

  1. a fresh account (friendbot, USDC trustline, 100 test USDC from the faucet);
  2. the LIVE thesis API drafts a signable policy with this account as owner;
  3. vault_factory.create_vault_with_policy - one call, one signature;
  4. on chain: policy owner and hash, sleeves = signed targets, keeper, router;
  5. deposit; optionally wait for the keeper to rebalance into Blend (--wait).

The account here is a plain G account paying its own fee (the wallet path);
the passkey + relayer path is exercised from the console. Prints the
transaction fee of the setup call (sizes RELAYER_FN_MAX_FEE).

    python testnet/e2e_own_vault.py [--wait SECONDS] [--thesis "..."]
"""
from __future__ import annotations

import importlib.util
import json
import os
import sys
import time
import urllib.request
from pathlib import Path

from stellar_sdk import Asset, Keypair, Network, scval

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("localnet_deploy", ROOT / "localnet/deploy.py")
ld = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ld)

TESTNET = Network.TESTNET_NETWORK_PASSPHRASE
ONE = 10_000_000


def post(url: str, body: dict) -> dict:
    req = urllib.request.Request(url, json.dumps(body).encode(), {"Content-Type": "application/json", "User-Agent": "constella-e2e"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())


def main() -> None:
    wait = int(sys.argv[sys.argv.index("--wait") + 1]) if "--wait" in sys.argv else 0
    thesis = (sys.argv[sys.argv.index("--thesis") + 1] if "--thesis" in sys.argv
              else "Keep a safe runway in dollars with at least 10% instant cash; lend a little.")
    cfg = json.loads((ROOT / "sandbox/config.testnet.json").read_text())
    st = json.loads((ROOT / "testnet/state/config.json").read_text())
    ct = st["contracts"]
    factory, usdc = ct["vault_factory"], cfg["assets"]["USDC"]
    net = ld.Net(cfg["network"]["rpc_url"], cfg["network"]["friendbot"], TESTNET, allowed=(TESTNET,))
    say = lambda *a: print(*a, flush=True)

    user = Keypair.random()
    net.fund(user)
    issuer = str(net.view(usdc, "name", [], user.public_key)).split(":")[1]
    net.classic(user, lambda b: b.append_change_trust_op(Asset("USDC", issuer)))
    say("account", user.public_key, "faucet:", post(cfg["services"]["relayer"] + "/faucet", {"address": user.public_key}).get("status", "ok"))

    out = post(cfg["services"]["thesis_api"] + "/thesis", {
        "text": thesis,
        "jurisdiction": "DE", "investor_class": "retail", "owner": user.public_key, "use_llm": True})
    assert out["signable"] and out["policy"]["admin"]["admin_address"] == user.public_key, out
    b = out["bind_policy"]
    say("thesis API:", out["parse_source"], "hash", out["policy_hash"][:16], "sleeves",
        [(s["asset_id"], s["target_weight_bps"]) for s in b["sleeves"]])

    S = ld.struct
    params = S({"admin": ld.A(user.public_key), "pause_authority": ld.A(user.public_key), "numeraire": ld.A(out["numeraire_contract"]),
                "performance_fee_bps": ld.U32(int(net.view(factory, "get_settings", [], user.public_key)["default_fee_bps"])),
                "tvl_cap": ld.I128(25_000 * ONE), "per_user_cap": ld.I128(25_000 * ONE),
                "name": ld.STR(out["policy"]["basket_name"]), "symbol": ld.STR("CRUN"), "salt": scval.to_bytes(os.urandom(32))})
    terms = S({"policy_hash": scval.to_bytes(bytes.fromhex(out["policy_hash"])),
               **{k: ld.U32(b[k]) for k in ("min_liquidity_sleeve_bps", "max_single_asset_bps", "max_issuer_bps",
                                            "max_slippage_bps", "rebalance_band_bps")},
               "sleeves": scval.to_vec([S({"asset_id": ld.SYM(s["asset_id"]), "target_weight_bps": ld.U32(s["target_weight_bps"]),
                                           "min_band_bps": ld.U32(s["min_band_bps"]), "max_band_bps": ld.U32(s["max_band_bps"]),
                                           "sleeve_type": ld.SYM(s["sleeve_type"])}) for s in b["sleeves"]])})
    args = [ld.A(user.public_key), params, terms]
    tx = net._builder(user).append_invoke_contract_function_op(factory, "create_vault_with_policy", args).build()
    prepared = net.server.prepare_transaction(tx)
    say("setup call fee (stroops):", prepared.transaction.fee)
    v = net.invoke(user, factory, "create_vault_with_policy", args)
    vault = getattr(v, "address", str(v))
    say("vault", vault)

    pol = net.view(st["contracts"]["policy"], "get_policy_config", [ld.A(vault)], user.public_key)
    addr = lambda a: getattr(a, "address", str(a))
    assert addr(pol["owner"]) == user.public_key and bytes(pol["policy_hash"]).hex() == out["policy_hash"]
    sl = net.view(vault, "get_sleeves", [], user.public_key)
    assert sorted((str(s["asset_id"]), s["target_weight_bps"]) for s in sl) == sorted((s["asset_id"], s["target_weight_bps"]) for s in b["sleeves"])
    keeper = addr(net.view(vault, "get_keeper", [], user.public_key))
    say("on chain: owner + hash match; sleeves = signed targets; keeper", keeper)

    net.invoke(user, vault, "deposit", [ld.I128(100 * ONE), ld.A(user.public_key), ld.A(user.public_key), ld.A(user.public_key)])
    say("deposited 100 USDC; shares", net.view(vault, "balance", [ld.A(user.public_key)], user.public_key))
    t0 = time.time()
    while wait and time.time() - t0 < wait:
        sl = net.view(vault, "get_sleeves", [], user.public_key)
        held = {str(s["asset_id"]): s["current_balance"] for s in sl}
        if any(v > 0 for k, v in held.items() if k != "USDC"):
            say("keeper rebalanced:", held)
            break
        time.sleep(20)
    else:
        if wait:
            say("keeper did not rebalance within", wait, "s:", held)
    say("E2E OK", vault)


if __name__ == "__main__":
    main()
