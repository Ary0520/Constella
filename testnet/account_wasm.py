#!/usr/bin/env python3
"""Roll a new smart-account WASM out to the testnet account factory.

The factory only switches the code it deploys for NEW accounts after a public
48-hour delay (account-factory WASM_CHANGE_DELAY_SECONDS); existing accounts
keep their WASM forever (accounts have no upgrade path by design).

    python testnet/account_wasm.py status
    python testnet/account_wasm.py propose     # uploads target/.../constella_smart_account.wasm, proposes its hash
    python testnet/account_wasm.py activate    # after the delay; anyone may call it

Keys: ~/.config/constella/testnet/keys.json (admin). State: testnet/state/config.json.
"""
from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
import importlib.util  # noqa: E402
# testnet/deploy.py shares the module name, so load localnet/deploy.py by path.
_spec = importlib.util.spec_from_file_location("localnet_deploy", ROOT / "localnet/deploy.py")
_ld = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_ld)
Net = _ld.Net
from stellar_sdk import Keypair, Network, scval  # noqa: E402

TESTNET = Network.TESTNET_NETWORK_PASSPHRASE
RPC = os.environ.get("RPC_URL", "https://soroban-testnet.stellar.org")
WASM = ROOT / "contracts/target/wasm32v1-none/release/constella_smart_account.wasm"
STATE = ROOT / "testnet/state/config.json"


def main() -> None:
    cmd = sys.argv[1] if len(sys.argv) > 1 else "status"
    keys = {k: Keypair.from_secret(v) for k, v in json.loads(Path(os.path.expanduser("~/.config/constella/testnet/keys.json")).read_text()).items()}
    admin = keys["admin"]
    net = Net(RPC, "https://friendbot.stellar.org", TESTNET, allowed=(TESTNET,))
    cfg = json.loads(STATE.read_text())
    factory = cfg["contracts"]["account_factory"]

    def status() -> None:
        s = net.view(factory, "get_settings", source=admin.public_key)
        p = net.view(factory, "get_pending_account_wasm", source=admin.public_key)
        print("active account wasm:", bytes(s["account_wasm_hash"]).hex() if isinstance(s, dict) else s)
        if p:
            eta = p["effective_at"] if "effective_at" in p else p.get("executable_at")
            print("pending:", bytes(p["wasm_hash"]).hex(), "effective_at:", eta, f"({max(0, eta - int(time.time()))}s from now)")
        else:
            print("pending: none")

    if cmd == "status":
        status()
    elif cmd == "propose":
        h = net.upload(admin, WASM)
        print("uploaded smart-account wasm:", h.hex())
        net.invoke(admin, factory, "propose_account_wasm", [scval.to_bytes(h)])
        cfg.setdefault("pending_wasm", {})["smart_account"] = h.hex()
        STATE.write_text(json.dumps(cfg, indent=1))
        status()
    elif cmd == "activate":
        net.invoke(admin, factory, "activate_account_wasm", [])
        new = cfg.get("pending_wasm", {}).pop("smart_account", None)
        if new:
            cfg["wasm_hashes"]["smart_account"] = new
        STATE.write_text(json.dumps(cfg, indent=1))
        status()
    else:
        sys.exit(f"unknown command {cmd!r}")


if __name__ == "__main__":
    main()
