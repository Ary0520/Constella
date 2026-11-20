"""Render localnet/state/config.json into (1) state/services.env for the
compose services (inside the compose network) and (2) sandbox/public/config.json
for the browser console (localhost URLs). LOCAL ONLY."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
c = json.loads((ROOT / "localnet/state/config.json").read_text())
k, sec, ct = c["accounts"], c["LOCAL_ONLY_secrets"], c["contracts"]
env = {
    "CONSTELLA_RPC_URL": "http://node:8000/soroban/rpc",
    "CONSTELLA_NETWORK_PASSPHRASE": c["network"]["passphrase"],
    # relayer (F1 / D-I / A2)
    "RELAYER_SECRETS": f'{sec["relayer1"]},{sec["relayer2"]}', "SPONSOR_CONTRACT_ID": ct["fee_sponsor"],
    "SPONSOR_ALLOWED": f'{ct["vault"]}:deposit,redeem,withdraw,redeem_frozen;{ct["device_registry"]}:add_device',
    "ACCOUNT_FACTORY_ID": ct["account_factory"],
    "RELAYER_HOST": "0.0.0.0", "RELAYER_PORT": "8787", "RELAYER_CORS_ORIGIN": "http://localhost:5173",
    "ACCOUNTS_PER_IP_PER_DAY": "50",
    # The local quickstart network charges ~35x testnet resource fees once its
    # storage grows (measured: deposit 0.74 vs 0.02 XLM); local cap only.
    "RELAYER_MAX_FEE_STROOPS": "20000000",
    # keeper
    "KEEPER_SECRET": sec["keeper"], "KEEPER_VAULTS": ct["vault"], "KEEPER_INTERVAL_SECONDS": "20",
    "KEEPER_STATE_FILE": "/state/keeper_state.json",
    # watchdog (D-M) -> local mail sink
    "WATCHDOG_NETWORK": "local", "WATCHDOG_SOURCE": k["admin"], "WATCHDOG_VAULTS": ct["vault"],
    "WATCHDOG_ACCOUNTS": f'relayer1:{k["relayer1"]}:100,relayer2:{k["relayer2"]}:100,keeper:{k["keeper"]}:100',
    "RELAYER_HEALTH_URL": "http://relayer:8787/health", "WATCHDOG_INTERVAL_SECONDS": "30",
    "WATCHDOG_STATE_FILE": "/state/watchdog_state.json", "WATCHDOG_STATUS_FILE": "/state/watchdog_status.json",
    "ALERT_SMTP_HOST": "mailsink", "ALERT_SMTP_PORT": "1025", "ALERT_SMTP_TLS": "none",
    "ALERT_FROM": "watchdog@constella.local", "ALERT_TO": "ops@constella.local",
    # thesis API
    "REGISTRY_ID": ct["registry"], "ORACLE_ID": ct["oracle"], "READ_SOURCE": k["admin"],
    "API_HOST": "0.0.0.0", "API_PORT": "8790", "API_CORS_ORIGIN": "http://localhost:5173",
}
(ROOT / "localnet/state/services.env").write_text("".join(f"{a}={b}\n" for a, b in env.items()))
browser = {**c, "network": {**c["network"], "rpc_url": "http://localhost:8000/soroban/rpc",
                            "friendbot": "http://localhost:8000/friendbot"},
           "services": {"relayer": "http://localhost:8787", "thesis_api": "http://localhost:8790"}}
out = ROOT / "sandbox/public/config.json"
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps(browser, indent=2))
print("wrote state/services.env and sandbox/public/config.json")
