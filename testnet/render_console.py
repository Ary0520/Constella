"""Write app/config.testnet.json for the hosted app: testnet
addresses and public service URLs ONLY - no secrets (this file is public)."""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
c = json.loads((ROOT / "testnet/state/config.json").read_text())
base = sys.argv[1] if len(sys.argv) > 1 else "https://constella-services-production.up.railway.app"
cfg = {
    "network": {"rpc_url": c["network"]["rpc_url"], "passphrase": c["network"]["passphrase"], "friendbot": c["network"]["friendbot"],
                "name": "testnet", "explorer": "https://stellar.expert/explorer/testnet"},
    "assets": {"USDC": c["assets"]["USDC"], "EURC": c["assets"]["EURC"]},
    "contracts": c["contracts"],
    "accounts": {"admin": c["accounts"]["admin"]},  # public key: simulation source for reads
    "services": {"relayer": f"{base}/relayer", "thesis_api": f"{base}/api"},
}
assert "LOCAL_ONLY_secrets" not in cfg and not any(str(v).startswith("S") and len(str(v)) == 56 for v in cfg["accounts"].values())
(ROOT / "app/config.testnet.json").write_text(json.dumps(cfg, indent=2))
print("wrote app/config.testnet.json (no secrets)")
