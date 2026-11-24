"""
Deploy the whole Constella stack to a LOCAL Stellar network (stellar/quickstart
--local) and write every address to one config file (default
localnet/state/config.json) used by the relayer, keeper, watchdog, AI-engine
API and the sandbox console.

Same contracts and setup as the local-network e2e tests: release WASMs, real
mainnet Soroswap WASMs, a Cash Reserve vault (85 % USDC 'numeraire' / 15 %
EURC, D-A), two independent EURC feeders, the relayer pool, account factory
(A2), vault factory (D-C) and the Collateral Passport feed (D-N).

LOCAL ONLY: the generated keys (secrets included) are throwaway local-network
keys, written to a gitignored file. Never point this at testnet or mainnet.
"""
from __future__ import annotations

import json
import os
import sys
import time
import urllib.request
from pathlib import Path

from stellar_sdk import Address, Asset, Keypair, SorobanServer, TransactionBuilder, scval, xdr
from stellar_sdk.exceptions import PrepareTransactionException
from stellar_sdk.soroban_rpc import GetTransactionStatus, SendTransactionStatus

ROOT = Path(__file__).resolve().parent.parent
ONE = 10 ** 7
STANDALONE = "Standalone Network ; February 2017"


def struct(fields: dict) -> xdr.SCVal:
    return scval.to_map({scval.to_symbol(k): fields[k] for k in sorted(fields)})


class Net:
    def __init__(self, rpc: str, friendbot: str, passphrase: str, allowed: tuple = (STANDALONE,)):
        if passphrase not in allowed:
            sys.exit(f"refusing network {passphrase!r}: this deployer allows only {allowed}")
        self.server = SorobanServer(rpc)
        self.friendbot = friendbot
        self.passphrase = passphrase

    def fund(self, kp: Keypair) -> None:
        for _ in range(30):
            try:
                req = urllib.request.Request(f"{self.friendbot}?addr={kp.public_key}",
                                             headers={"User-Agent": "Mozilla/5.0 (compatible; stellar-deploy)"})
                with urllib.request.urlopen(req, timeout=30):
                    return
            except Exception:
                time.sleep(3)
        sys.exit(f"friendbot could not fund {kp.public_key}")

    def _send(self, tx, signers) -> object:
        for s in signers:
            tx.sign(s)
        sent = self.server.send_transaction(tx)
        if sent.status not in (SendTransactionStatus.PENDING, SendTransactionStatus.DUPLICATE):
            raise RuntimeError(f"send failed: {sent.status} {sent.error_result_xdr}")
        r = self.server.poll_transaction(sent.hash)
        if r.status != GetTransactionStatus.SUCCESS:
            raise RuntimeError(f"tx {sent.hash} ended {r.status}")
        return r

    def _builder(self, source: Keypair) -> TransactionBuilder:
        return TransactionBuilder(self.server.load_account(source.public_key), self.passphrase, base_fee=1000).set_timeout(60)

    def soroban(self, source: Keypair, build, signers=None):
        tx = build(self._builder(source)).build()
        try:
            tx = self.server.prepare_transaction(tx)
        except PrepareTransactionException as e:
            raise RuntimeError(f"simulation failed: {e.simulate_transaction_response.error}") from None
        r = self._send(tx, [source] + (signers or []))
        meta = xdr.TransactionMeta.from_xdr(r.result_meta_xdr)
        rv = None
        for v in (meta.v4, meta.v3):
            if v is not None and getattr(v, "soroban_meta", None) is not None and v.soroban_meta.return_value is not None:
                rv = v.soroban_meta.return_value
        return scval.to_native(rv) if rv is not None else None

    def classic(self, source: Keypair, build, signers=None):
        return self._send(build(self._builder(source)).build(), [source] + (signers or []))

    def upload(self, src: Keypair, wasm: Path) -> bytes:
        h = self.soroban(src, lambda b: b.append_upload_contract_wasm_op(wasm.read_bytes()))
        return bytes(h)

    def deploy(self, src: Keypair, wasm_hash: bytes, args=None) -> str:
        cid = self.soroban(src, lambda b: b.append_create_contract_op(wasm_hash, src.public_key, list(args or []),
                                                                      salt=os.urandom(32)))
        return cid.address if hasattr(cid, "address") else str(cid)

    def invoke(self, src: Keypair, contract: str, fn: str, args=(), signers=None):
        return self.soroban(src, lambda b: b.append_invoke_contract_function_op(contract, fn, list(args)), signers)

    def view(self, contract: str, fn: str, args=(), source: str = None):
        tx = (TransactionBuilder(self.server.load_account(source), self.passphrase, base_fee=100)
              .append_invoke_contract_function_op(contract, fn, list(args)).set_timeout(30).build())
        sim = self.server.simulate_transaction(tx)
        if sim.error:
            raise RuntimeError(f"{fn}: {sim.error}")
        return scval.to_native(xdr.SCVal.from_xdr(sim.results[0].xdr))


A = scval.to_address
U32 = scval.to_uint32
I128 = scval.to_int128
SYM = scval.to_symbol
STR = scval.to_string


def main() -> None:
    rpc = os.environ.get("RPC_URL", "http://localhost:8000/soroban/rpc")
    friendbot = os.environ.get("FRIENDBOT_URL", rpc.replace("/soroban/rpc", "/friendbot"))
    wasm_dir = Path(os.environ.get("WASM_DIR", ROOT / "contracts/target/wasm32v1-none/release"))
    sw_dir = Path(os.environ.get("SOROSWAP_DIR", ROOT / "contracts/vault-strategy/testdata"))
    out = Path(os.environ.get("OUT", ROOT / "localnet/state/config.json"))
    net = Net(rpc, friendbot, os.environ.get("PASSPHRASE", STANDALONE))
    W = lambda n: wasm_dir / f"constella_{n}.wasm"
    log = lambda *a: print(*a, flush=True)

    keys = {k: Keypair.random() for k in ["admin", "issuer", "feeder1", "feeder2", "keeper", "relayer1", "relayer2", "lp"]}
    for kp in keys.values():
        net.fund(kp)
    admin, issuer = keys["admin"], keys["issuer"]
    log("funded", len(keys), "accounts")

    # Assets: classic USDC / EURC (issuer) with their Stellar Asset Contracts.
    sac = {}
    for code in ("USDC", "EURC"):
        asset = Asset(code, issuer.public_key)
        sac[code] = net.soroban(admin, lambda b, a=asset: b.append_create_stellar_asset_contract_from_asset_op(a)).address
        net.classic(keys["lp"], lambda b, a=asset: b.append_change_trust_op(a))
    usdc, eurc = sac["USDC"], sac["EURC"]
    log("assets", sac)

    # Registry, NAV oracle (two independent EURC feeders), policy.
    h = {n: net.upload(admin, W(n)) for n in ["registry", "nav_oracle", "policy", "vault_strategy", "webauthn_verifier",
                                               "smart_account", "fee_sponsor", "account_factory", "vault_factory",
                                               "collateral_oracle", "device_registry"]}
    registry = net.deploy(admin, h["registry"], [A(admin.public_key)])
    for aid, addr, srcs in [("USDC", usdc, ["USDC_BASE"]), ("EURC", eurc, ["EURC_A", "EURC_B"])]:
        net.invoke(admin, registry, "add_asset", [struct({
            "contract_address": A(addr), "asset_id": SYM(aid), "issuer": SYM("CIRCLE"), "sleeve_type": U32(0),
            "liquidity_tier": U32(0), "permissionless": scval.to_bool(True), "oracle_sources": scval.to_vec([SYM(s) for s in srcs]),
            "eligible_jurisdictions": scval.to_vec([]), "blocked_jurisdictions": scval.to_vec([]),
            "investor_classes_required": scval.to_vec([]), "is_active": scval.to_bool(True), "added_at": scval.to_uint64(0)})])
    oracle = net.deploy(admin, h["nav_oracle"], [A(admin.public_key), A(usdc), U32(7), U32(300)])
    net.invoke(admin, oracle, "configure_sleeve", [SYM("CASH"), U32(200), U32(2)])
    for sid, feeder in [("EURC_A", keys["feeder1"]), ("EURC_B", keys["feeder2"])]:
        net.invoke(admin, oracle, "register_source", [struct({
            "source_id": SYM(sid), "asset": A(eurc), "sleeve": SYM("CASH"), "price": I128(0), "timestamp": scval.to_uint64(0),
            "staleness_limit_seconds": scval.to_uint64(432_000), "min_liquidity_depth_usd": I128(0),
            "current_liquidity_depth_usd": I128(0), "is_active": scval.to_bool(True), "feeder": A(feeder.public_key)})])
        net.invoke(feeder, oracle, "update_price", [SYM(sid), I128(ONE * 108 // 100), I128(0), scval.to_uint64(int(time.time()) - 30)])
    policy = net.deploy(admin, h["policy"], [A(admin.public_key), A(oracle)])
    log("registry", registry, "oracle", oracle, "policy", policy)

    # Cash Reserve vault (D-A): 85 % USDC numeraire / 15 % EURC.
    vault_cfg = lambda: struct({"admin": A(admin.public_key), "pause_authority": A(admin.public_key), "numeraire": A(usdc),
                                "policy_contract": A(policy), "oracle_contract": A(oracle), "registry_contract": A(registry),
                                "treasury": A(admin.public_key), "performance_fee_bps": U32(1000),
                                "tvl_cap": I128(10_000_000 * ONE), "per_user_cap": I128(1_000_000 * ONE)})
    vault = net.deploy(admin, h["vault_strategy"], [vault_cfg(), STR("Constella Cash Reserve"), STR("CCR"), U32(7)])
    sleeve_t = lambda aid, t, lo, hi, kind: struct({"asset_id": SYM(aid), "target_weight_bps": U32(t), "min_band_bps": U32(lo),
                                                     "max_band_bps": U32(hi), "sleeve_type": SYM(kind)})
    net.invoke(admin, vault, "bind_policy", [A(admin.public_key), scval.to_bytes(os.urandom(32)), U32(1000), U32(2500), U32(3000),
                                             U32(50), U32(200), scval.to_vec([sleeve_t("USDC", 8500, 8000, 9000, "numeraire"),
                                                                             sleeve_t("EURC", 1500, 1000, 2000, "cash")])])
    sleeve_s = lambda aid, addr, t, kind: struct({"asset_id": STR(aid), "contract_address": A(addr), "target_weight_bps": U32(t),
                                                   "band_bps": U32(200), "current_balance": I128(0), "sleeve_type": U32(kind),
                                                   "liquidity_tier": U32(0)})
    net.invoke(admin, vault, "set_sleeves", [scval.to_vec([sleeve_s("USDC", usdc, 8500, 0), sleeve_s("EURC", eurc, 1500, 2)])])
    net.invoke(admin, vault, "set_keeper", [A(keys["keeper"].public_key)])
    log("vault", vault)

    # Real Soroswap (mainnet WASMs) with liquidity at the oracle price.
    pair_hash = net.upload(admin, sw_dir / "soroswap_pair_mainnet.wasm")
    factory = net.deploy(admin, net.upload(admin, sw_dir / "soroswap_factory_mainnet.wasm"))
    net.invoke(admin, factory, "initialize", [A(admin.public_key), scval.to_bytes(pair_hash)])
    router = net.deploy(admin, net.upload(admin, sw_dir / "soroswap_router_mainnet.wasm"))
    net.invoke(admin, router, "initialize", [A(factory)])
    lp = keys["lp"]
    net.invoke(issuer, usdc, "mint", [A(lp.public_key), I128(20_000_000 * ONE)])
    net.invoke(issuer, eurc, "mint", [A(lp.public_key), I128(10_000_000 * ONE)])
    net.invoke(lp, router, "add_liquidity", [A(usdc), A(eurc), I128(10_800_000 * ONE), I128(10_000_000 * ONE), I128(0), I128(0),
                                             A(lp.public_key), scval.to_uint64(4_102_444_800)])
    net.invoke(admin, vault, "propose_router", [U32(1), A(router)])  # fresh vault: immediate (D-K)
    log("soroswap router", router)

    # Accounts (A2), sponsorship (F1/D-I), vault factory (D-C), Collateral Passport (D-N).
    verifier = net.deploy(admin, h["webauthn_verifier"])
    account_factory = net.deploy(admin, h["account_factory"], [A(admin.public_key), A(verifier), scval.to_bytes(h["smart_account"])])
    sponsor = net.deploy(admin, h["fee_sponsor"], [A(admin.public_key), U32(30), U32(200), scval.to_uint64(5_000 * ONE)])
    device_registry = net.deploy(admin, h["device_registry"], [A(account_factory), A(verifier)])
    for target in (vault, account_factory, device_registry):
        net.invoke(admin, sponsor, "whitelist_contract", [A(target)])
    for r in ("relayer1", "relayer2"):
        net.invoke(admin, sponsor, "add_relayer", [A(keys[r].public_key)])
    vault_factory = net.deploy(admin, h["vault_factory"], [A(admin.public_key), scval.to_bytes(h["vault_strategy"]), A(registry),
                                                           A(oracle), A(policy), A(admin.public_key), I128(1_000_000 * ONE),
                                                                         U32(1000), A(keys["keeper"].public_key), A(router)])
    collateral = net.deploy(admin, h["collateral_oracle"], [A(admin.public_key), A(oracle)])
    net.invoke(admin, collateral, "list_vault", [A(vault)])
    log("account factory", account_factory, "sponsor", sponsor, "vault factory", vault_factory, "collateral feed", collateral)

    config = {
        "network": {"rpc_url": rpc, "passphrase": net.passphrase, "friendbot": friendbot},
        "assets": {"USDC": usdc, "EURC": eurc, "issuer": issuer.public_key},
        "contracts": {"registry": registry, "oracle": oracle, "policy": policy, "vault": vault, "soroswap_router": router,
                      "soroswap_factory": factory, "verifier": verifier, "account_factory": account_factory,
                      "fee_sponsor": sponsor, "vault_factory": vault_factory, "collateral_oracle": collateral,
                      "device_registry": device_registry},
        "wasm_hashes": {k: v.hex() for k, v in h.items()},
        "accounts": {k: kp.public_key for k, kp in keys.items()},
        "LOCAL_ONLY_secrets": {k: kp.secret for k, kp in keys.items()},
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(config, indent=2))
    log("wrote", out)


if __name__ == "__main__":
    main()
