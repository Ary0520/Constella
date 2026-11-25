"""Render the services' settings for testnet from testnet/state/config.json
and ~/.config/constella/testnet/keys.json into
~/.config/constella/testnet/services.env (chmod 600, outside the repo). The
same variables are pushed to Railway. Usage: render_env.py [CORS_ORIGIN]"""
import json
import os
import sys
from pathlib import Path

from stellar_sdk import Keypair

ROOT = Path(__file__).resolve().parent.parent
c = json.loads((ROOT / "testnet/state/config.json").read_text())
k = json.loads(Path(os.path.expanduser("~/.config/constella/testnet/keys.json")).read_text())
pub = lambda n: Keypair.from_secret(k[n]).public_key
ct = c["contracts"]
vaults = [v for v in (ct.get("vault_blend"), ct.get("vault_eurc")) if v] or [ct["vault"]]
origin = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:5173"
env = {
    "CONSTELLA_RPC_URL": c["network"]["rpc_url"], "CONSTELLA_NETWORK_PASSPHRASE": c["network"]["passphrase"],
    "CONSTELLA_NETWORK": "testnet",
    # relayer (+ testnet faucet)
    "RELAYER_SECRETS": f'{k["relayer1"]},{k["relayer2"]}', "SPONSOR_CONTRACT_ID": ct["fee_sponsor"],
    "SPONSOR_ALLOWED": ";".join(f"{v}:deposit,redeem,withdraw,redeem_frozen" for v in vaults) + f';{ct["device_registry"]}:add_device'
                       + f';{ct["vault_factory"]}:create_vault_with_policy' + f';{ct["recovery_registry"]}:set_guardians',
    # Step 3: users' own vaults (proven on chain by the trusted factory).
    "SPONSOR_VAULT_FUNCTIONS": "deposit,redeem,withdraw,redeem_frozen",
    # Step 8: recovery calls on users' own accounts (proven by the account factory).
    "SPONSOR_ACCOUNT_FUNCTIONS": "propose_recovery,cancel_recovery",
    "RECOVERY_REGISTRY_ID": ct["recovery_registry"], "RECOVERY_STATE_FILE": "/data/recovery.json",
    # add_device pays its storage (~2.37 XLM); the vault setup deploys a contract
    # (measured 0.11-5.96 XLM on testnet 24 Nov 2026, the higher with restores).
    "RELAYER_FN_MAX_FEE": "add_device:30000000,create_vault_with_policy:70000000,set_guardians:30000000,finish:10000000",
    "ACCOUNT_FACTORY_ID": ct["account_factory"],
    "RELAYER_HOST": "127.0.0.1", "RELAYER_PORT": "8787", "RELAYER_TRUST_PROXY": "1", "RELAYER_CORS_ORIGIN": origin,
    "ACCOUNTS_PER_IP_PER_DAY": "3", "ACCOUNTS_PER_DAY": "200",
    "FAUCET_SECRET": k["lp"], "FAUCET_ASSET": c["assets"]["USDC"], "FAUCET_AMOUNT": "1000000000",
    # keeper
    "KEEPER_SECRET": k["keeper"], "KEEPER_VAULTS": ",".join(vaults), "KEEPER_FACTORIES": ",".join([ct["vault_factory"]] + [v for k, v in sorted(ct.items()) if k.startswith("vault_factory_v")]),
    "KEEPER_INTERVAL_SECONDS": "120",
    "KEEPER_STATE_FILE": "/data/keeper_state.json",
    # ECB feeder (2nd independent EURC source)
    "CONSTELLA_ORACLE_ID": ct["oracle"], "FEEDER_SOURCE_ID": "EURC_ECB", "FEEDER_SECRET": k["feeder1"], "FEEDER_INTERVAL": "21600",
    # watchdog (log-only until the alert email is set up - see reminder)
    "WATCHDOG_NETWORK": "testnet", "WATCHDOG_SOURCE": pub("admin"), "WATCHDOG_VAULTS": ",".join(vaults),
    "WATCHDOG_ACCOUNTS": ",".join(f"{n}:{pub(n)}:50" for n in ("relayer1", "relayer2", "keeper", "feeder1")),
    "RELAYER_HEALTH_URL": "http://127.0.0.1:8787/health", "WATCHDOG_INTERVAL_SECONDS": "300",
    "WATCHDOG_STATE_FILE": "/data/watchdog_state.json", "WATCHDOG_STATUS_FILE": "/data/watchdog_status.json",
    # thesis API
    "REGISTRY_ID": ct["registry"], "ORACLE_ID": ct["oracle"], "READ_SOURCE": pub("admin"),
    "API_HOST": "127.0.0.1", "API_PORT": "8790", "API_CORS_ORIGIN": origin,
}
out = Path(os.path.expanduser("~/.config/constella/testnet/services.env"))
out.write_text("".join(f"{a}={b}\n" for a, b in env.items()))
out.chmod(0o600)
print(f"wrote {out} ({len(env)} variables, CORS origin {origin})")
