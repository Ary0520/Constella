"""
Deploy Constella to Stellar TESTNET with the REAL testnet ecosystem (no
local copies): Circle testnet USDC/EURC, Soroswap testnet router, Reflector
testnet CEX/DEX feed + the Constella ECB feeder for EURC (two independent
sources), our release WASMs. Refuses any network but testnet.

Keys: ~/.config/constella/testnet/keys.json (chmod 600, outside the repo).
Output: testnet/state/config.json - addresses only, NO secrets (gitignored).
Every external address below was verified live on 2026-11-20.
"""
from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "localnet"))
from deploy import A, I128, ONE, STR, SYM, U32, Net, struct  # noqa: E402
from stellar_sdk import Keypair, Network, scval  # noqa: E402

TESTNET = Network.TESTNET_NETWORK_PASSPHRASE
RPC = os.environ.get("RPC_URL", "https://soroban-testnet.stellar.org")
FRIENDBOT = "https://friendbot.stellar.org"
USDC = "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA"   # Circle testnet USDC SAC
EURC = "CCUUDM434BMZMYWYDITHFXHDMIVTGGD6T2I5UKNX5BSLXLW7HVR4MCGZ"   # Circle testnet EURC SAC
SOROSWAP_ROUTER = "CCJUD55AG6W5HAI5LRVNKAE5WDP5XGZBUDS5WNTIVDU7O264UZZE7BRD"
REFLECTOR_CEX = "CCYOZJCOPG34LLQQ7N24YXBM7LL62R7ONMZ3G6WZAAYPB5OYKOMJRN63"
OTHER = lambda sym: scval.to_vec([SYM("Other"), SYM(sym)])


def main() -> None:
    keys = {k: Keypair.from_secret(v) for k, v in json.loads(Path(os.path.expanduser("~/.config/constella/testnet/keys.json")).read_text()).items()}
    net = Net(RPC, FRIENDBOT, TESTNET, allowed=(TESTNET,))
    out = ROOT / "testnet/state/config.json"
    state = json.loads(out.read_text()) if out.exists() else {}
    c = state.setdefault("contracts", {})
    save = lambda: (out.parent.mkdir(parents=True, exist_ok=True), out.write_text(json.dumps(state, indent=2)))
    log = lambda *a: print(*a, flush=True)
    admin = keys["admin"]
    for name in ("admin", "feeder1", "keeper", "relayer1", "relayer2"):
        try:
            net.server.load_account(keys[name].public_key)
        except Exception:
            net.fund(keys[name])
    W = lambda n: ROOT / f"contracts/target/wasm32v1-none/release/constella_{n}.wasm"
    h = state.setdefault("wasm_hashes", {})
    for n in ["registry", "nav_oracle", "policy", "vault_strategy", "webauthn_verifier", "smart_account", "fee_sponsor",
              "account_factory", "vault_factory", "collateral_oracle", "device_registry"]:
        if n not in h:
            h[n] = net.upload(admin, W(n)).hex(); save(); log("uploaded", n)
    H = lambda n: bytes.fromhex(h[n])

    def once(name, make):  # idempotent: re-running resumes where it stopped
        if name not in c:
            c[name] = make(); save(); log(name, c[name])
        return c[name]

    registry = once("registry", lambda: net.deploy(admin, H("registry"), [A(admin.public_key)]))
    if not state.get("registry_seeded"):
        for aid, addr, srcs in [("USDC", USDC, ["USDC_BASE"]), ("EURC", EURC, ["EURC_REFLECTOR", "EURC_ECB"])]:
            net.invoke(admin, registry, "add_asset", [struct({
                "contract_address": A(addr), "asset_id": SYM(aid), "issuer": SYM("CIRCLE"), "sleeve_type": U32(0),
                "liquidity_tier": U32(0), "permissionless": scval.to_bool(True), "oracle_sources": scval.to_vec([SYM(x) for x in srcs]),
                "eligible_jurisdictions": scval.to_vec([]), "blocked_jurisdictions": scval.to_vec([]),
                "investor_classes_required": scval.to_vec([]), "is_active": scval.to_bool(True), "added_at": scval.to_uint64(0)})])
        state["registry_seeded"] = True; save()

    oracle = once("oracle", lambda: net.deploy(admin, H("nav_oracle"), [A(admin.public_key), A(USDC), U32(7), U32(300)]))
    if not state.get("oracle_sources"):
        net.invoke(admin, oracle, "configure_sleeve", [SYM("CASH"), U32(200), U32(2)])
        src = lambda sid, stale, feeder: struct({
            "source_id": SYM(sid), "asset": A(EURC), "sleeve": SYM("CASH"), "price": I128(0), "timestamp": scval.to_uint64(0),
            "staleness_limit_seconds": scval.to_uint64(stale), "min_liquidity_depth_usd": I128(0),
            "current_liquidity_depth_usd": I128(0), "is_active": scval.to_bool(True), "feeder": A(feeder)})
        # Source 1: Reflector testnet CEX/DEX, EURC cross-rated through its own USDC key (O7b), median of 3 rounds.
        net.invoke(admin, oracle, "register_reflector_source", [src("EURC_REFLECTOR", 1800, admin.public_key), struct({
            "contract": A(REFLECTOR_CEX), "asset": OTHER("EURC"), "quote": scval.to_vec([SYM("ViaUsdc"), OTHER("USDC")]),
            "rounds": U32(3)})])
        # Source 2: Constella ECB feeder (its own key, O7a); ECB publishes on business days -> 4-day limit.
        net.invoke(admin, oracle, "register_source", [src("EURC_ECB", 345_600, keys["feeder1"].public_key)])
        state["oracle_sources"] = True; save()

    policy = once("policy", lambda: net.deploy(admin, H("policy"), [A(admin.public_key), A(oracle)]))
    vault = once("vault", lambda: net.deploy(admin, H("vault_strategy"), [struct({
        "admin": A(admin.public_key), "pause_authority": A(admin.public_key), "numeraire": A(USDC), "policy_contract": A(policy),
        "oracle_contract": A(oracle), "registry_contract": A(registry), "treasury": A(admin.public_key),
        "performance_fee_bps": U32(1000), "tvl_cap": I128(1_000_000 * ONE), "per_user_cap": I128(100_000 * ONE)}),
        STR("Constella Cash Reserve (testnet)"), STR("CCR"), U32(7)]))
    if not state.get("vault_configured"):
        t = lambda aid, tg, lo, hi, kind: struct({"asset_id": SYM(aid), "target_weight_bps": U32(tg), "min_band_bps": U32(lo),
                                                   "max_band_bps": U32(hi), "sleeve_type": SYM(kind)})
        # Cash Reserve (D-A). Slippage 1 %: testnet EURC liquidity is thin (documented, testnet only).
        net.invoke(admin, vault, "bind_policy", [A(admin.public_key), scval.to_bytes(os.urandom(32)), U32(1000), U32(2500),
                                                 U32(3000), U32(100), U32(200),
                                                 scval.to_vec([t("USDC", 8500, 8000, 9000, "numeraire"), t("EURC", 1500, 1000, 2000, "cash")])])
        sl = lambda aid, addr, tg, kind: struct({"asset_id": STR(aid), "contract_address": A(addr), "target_weight_bps": U32(tg),
                                                  "band_bps": U32(200), "current_balance": I128(0), "sleeve_type": U32(kind),
                                                  "liquidity_tier": U32(0)})
        net.invoke(admin, vault, "set_sleeves", [scval.to_vec([sl("USDC", USDC, 8500, 0), sl("EURC", EURC, 1500, 2)])])
        net.invoke(admin, vault, "set_keeper", [A(keys["keeper"].public_key)])
        net.invoke(admin, vault, "propose_router", [U32(1), A(SOROSWAP_ROUTER)])  # fresh vault: immediate (D-K)
        state["vault_configured"] = True; save()

    verifier = once("verifier", lambda: net.deploy(admin, H("webauthn_verifier")))
    account_factory = once("account_factory", lambda: net.deploy(admin, H("account_factory"), [A(admin.public_key), A(verifier), scval.to_bytes(H("smart_account"))]))
    sponsor = once("fee_sponsor", lambda: net.deploy(admin, H("fee_sponsor"), [A(admin.public_key), U32(30), U32(200), scval.to_uint64(2_000 * ONE)]))
    registry_dev = once("device_registry", lambda: net.deploy(admin, H("device_registry"), [A(account_factory), A(verifier)]))
    if not state.get("device_registry_whitelisted"):
        if state.get("sponsor_configured"):
            net.invoke(admin, sponsor, "whitelist_contract", [A(registry_dev)])
        state["device_registry_whitelisted"] = True; save()
    if not state.get("sponsor_configured"):
        for target in (vault, account_factory, registry_dev):
            net.invoke(admin, sponsor, "whitelist_contract", [A(target)])
        for r in ("relayer1", "relayer2"):
            net.invoke(admin, sponsor, "add_relayer", [A(keys[r].public_key)])
        state["sponsor_configured"] = True; save()
    once("vault_factory", lambda: net.deploy(admin, H("vault_factory"), [A(admin.public_key), scval.to_bytes(H("vault_strategy")), A(registry),
                                                                         A(oracle), A(policy), A(admin.public_key), I128(1_000_000 * ONE),
                                                                         U32(1000), A(keys["keeper"].public_key), A(SOROSWAP_ROUTER)]))
    collateral = once("collateral_oracle", lambda: net.deploy(admin, H("collateral_oracle"), [A(admin.public_key), A(oracle)]))
    if not state.get("collateral_listed"):
        net.invoke(admin, collateral, "list_vault", [A(vault)])
        state["collateral_listed"] = True; save()

    state["network"] = {"rpc_url": RPC, "passphrase": TESTNET, "friendbot": FRIENDBOT, "horizon": "https://horizon-testnet.stellar.org"}
    state["assets"] = {"USDC": USDC, "EURC": EURC}
    state["external"] = {"soroswap_router": SOROSWAP_ROUTER, "reflector_cex": REFLECTOR_CEX}
    state["accounts"] = {k: kp.public_key for k, kp in keys.items()}  # public keys only
    save(); log("testnet deployment complete ->", out)


if __name__ == "__main__":
    main()
