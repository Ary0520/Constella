"""
Testnet: our own Blend v2 pool + the canonical Cash Reserve vault (D-A:
85 % USDC 'numeraire' / 15 % Blend USDC lending). Resumable (state in
testnet/state/config.json, keys in ~/.config/constella/testnet).

Testnet limits (documented, checked from Blend's source 2026-11-20): the pool
runs in "admin on-ice" (status 2) - supply/withdraw work, borrowing is
blocked - because an ACTIVE pool needs a backstop worth ~75k USDC in
BLND:USDC LP (threshold blnd^4*usdc >= 1e25), and testnet BLND has no
market. So testnet lending yield is 0 % and Collateral Passport borrowing
is "coming online" there (proven on the real Blend code in the tests).
"""
from __future__ import annotations

import hashlib
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "localnet"))
from deploy import A, I128, ONE, STR, SYM, U32, Net, struct  # noqa: E402
from stellar_sdk import Keypair, Network, scval  # noqa: E402

TESTNET = Network.TESTNET_NETWORK_PASSPHRASE
BLEND_FACTORY = "CDV6RX4CGPCOKGTBFS52V3LMWQGZN3LCQTXF5RVPOOCG4XVMHXQ4NTF6"  # official testnet pool factory v2


def reserve_config(c_factor: int) -> object:
    return struct({"index": U32(0), "decimals": U32(7), "c_factor": U32(c_factor), "l_factor": U32(7_500_000),
                   "util": U32(7_500_000), "max_util": U32(9_500_000), "r_base": U32(100_000), "r_one": U32(500_000),
                   "r_two": U32(5_000_000), "r_three": U32(15_000_000), "reactivity": U32(20),
                   "supply_cap": I128(1_000_000_000 * ONE), "enabled": scval.to_bool(True)})


def main() -> None:
    keys = {k: Keypair.from_secret(v) for k, v in json.loads(Path(os.path.expanduser("~/.config/constella/testnet/keys.json")).read_text()).items()}
    out = ROOT / "testnet/state/config.json"
    st = json.loads(out.read_text())
    c, b = st["contracts"], st.setdefault("blend", {})
    save = lambda: out.write_text(json.dumps(st, indent=2))
    log = lambda *a: print(*a, flush=True)
    net = Net(st["network"]["rpc_url"], st["network"]["friendbot"], TESTNET, allowed=(TESTNET,))
    admin = keys["admin"]
    USDC = st["assets"]["USDC"]

    def step(name, fn):
        if not b.get(name):
            b[name] = fn() or True; save(); log(name, b[name])
        return b[name]

    addr = lambda v: v.address if hasattr(v, "address") else str(v)
    pool = step("pool", lambda: addr(net.invoke(admin, BLEND_FACTORY, "deploy", [
        A(admin.public_key), STR("Constella Cash Reserve (testnet)"), scval.to_bytes(hashlib.sha256(b"constella-testnet-pool-1").digest()),
        A(c["collateral_oracle"]), U32(1_000_000), U32(4), I128(ONE)])))
    step("usdc_reserve", lambda: (net.invoke(admin, pool, "queue_set_reserve", [A(USDC), reserve_config(7_500_000)]),
                                  net.invoke(admin, pool, "set_reserve", [A(USDC)]))[1])
    # The oracle's Blend adapter trusts only pools of the official factory (D-H).
    step("oracle_blend_factory", lambda: net.invoke(admin, c["oracle"], "set_blend_factory", [A(BLEND_FACTORY)]))
    step("oracle_blend_position", lambda: net.invoke(admin, c["oracle"], "register_blend_position", [A(pool), A(USDC)]))
    step("registry_blend", lambda: net.invoke(admin, c["registry"], "add_asset", [struct({
        "contract_address": A(pool), "asset_id": SYM("BLEND_USDC_SUPPLY"), "issuer": SYM("BLEND"), "sleeve_type": U32(4),
        "liquidity_tier": U32(1), "permissionless": scval.to_bool(True), "oracle_sources": scval.to_vec([SYM("BLEND_B_RATE")]),
        "eligible_jurisdictions": scval.to_vec([]), "blocked_jurisdictions": scval.to_vec([]),
        "investor_classes_required": scval.to_vec([]), "is_active": scval.to_bool(True), "added_at": scval.to_uint64(0)})]))

    def create_vault():
        v = net.invoke(admin, c["vault_factory"], "create_vault", [A(admin.public_key), struct({
            "admin": A(admin.public_key), "pause_authority": A(admin.public_key), "numeraire": A(USDC), "performance_fee_bps": U32(1000),
            "tvl_cap": I128(1_000_000 * ONE), "per_user_cap": I128(100_000 * ONE), "name": STR("Constella Cash Reserve - Blend (testnet)"),
            "symbol": STR("CCRB"), "salt": scval.to_bytes(hashlib.sha256(b"cash-reserve-blend-1").digest())})])
        return v.address if hasattr(v, "address") else str(v)
    vault = step("vault", create_vault)

    def configure():
        t = lambda aid, tg, lo, hi, kind: struct({"asset_id": SYM(aid), "target_weight_bps": U32(tg), "min_band_bps": U32(lo),
                                                   "max_band_bps": U32(hi), "sleeve_type": SYM(kind)})
        net.invoke(admin, vault, "bind_policy", [A(admin.public_key), scval.to_bytes(os.urandom(32)), U32(1000), U32(2500), U32(3000),
                                                 U32(100), U32(200), scval.to_vec([t("USDC", 8500, 8000, 9000, "numeraire"),
                                                                                  t("BLEND_USDC_SUPPLY", 1500, 1000, 2000, "lending")])])
        sl = lambda aid, addr, tg, kind: struct({"asset_id": STR(aid), "contract_address": A(addr), "target_weight_bps": U32(tg),
                                                  "band_bps": U32(200), "current_balance": I128(0), "sleeve_type": U32(kind),
                                                  "liquidity_tier": U32(0)})
        net.invoke(admin, vault, "set_sleeves", [scval.to_vec([sl("USDC", USDC, 8500, 0), sl("BLEND_USDC_SUPPLY", pool, 1500, 4)])])
        net.invoke(admin, vault, "set_keeper", [A(keys["keeper"].public_key)])
    step("vault_configured", configure)
    step("sponsor_whitelisted", lambda: net.invoke(admin, c["fee_sponsor"], "whitelist_contract", [A(vault)]))
    step("collateral_listed", lambda: net.invoke(admin, c["collateral_oracle"], "list_vault", [A(vault)]))

    def share_reserve():
        kit = net.view(vault, "get_listing_kit", [], admin.public_key)
        net.invoke(admin, pool, "queue_set_reserve", [A(vault), reserve_config(int(kit["recommended_ltv_bps"]) * 1000)])
        return net.invoke(admin, pool, "set_reserve", [A(vault)])
    step("share_reserve", share_reserve)
    step("pool_on_ice", lambda: net.invoke(admin, pool, "set_status", [U32(2)]))  # supply allowed, borrow blocked

    c["vault_eurc"] = c.get("vault_eurc", c["vault"])
    c["vault_blend"] = vault
    c["blend_pool"] = pool
    save(); log("blend setup complete: pool", pool, "vault", vault)


if __name__ == "__main__":
    main()
