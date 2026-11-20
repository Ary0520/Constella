"""Feeder entry point: one run = fetch, derive, gate, submit (or skip).

Environment:
  CONSTELLA_NETWORK   testnet | mainnet | local
  CONSTELLA_RPC_URL   optional RPC override
  CONSTELLA_ORACLE_ID oracle contract id (C...)
  FEEDER_SOURCE_ID    oracle source id this feeder owns (e.g. ECB_EURC)
  FEEDER_SECRET       the source's feeder secret key (S...) - env only, never logged
  ECB_URL             optional (file:// for offline replay); default: ECB daily XML
Exit codes: 0 submitted or nothing to do; 2 refused by a safety gate; 1 error.
"""
from __future__ import annotations

import os
import sys
import time

from stellar_sdk import Keypair

from . import ecb
from .chain import NETWORKS, OracleClient
from .pricing import GateRejected, decide, eurc_in_usdc


def run_once(env: dict[str, str]) -> tuple[str, str]:
    network = NETWORKS[env["CONSTELLA_NETWORK"]]
    client = OracleClient(network, env["CONSTELLA_ORACLE_ID"], env.get("CONSTELLA_RPC_URL"))
    feeder = Keypair.from_secret(env["FEEDER_SECRET"])
    source_id = env["FEEDER_SOURCE_ID"]

    rate = ecb.parse_usd_rate(ecb.fetch_daily(env.get("ECB_URL", ecb.ECB_DAILY_URL)))
    update = eurc_in_usdc(rate)
    current = client.get_source(feeder.public_key, source_id)
    action = decide(update, current, int(time.time()))
    if action == "skip":
        return "skip", f"{source_id}: ECB {rate.day} already on-chain ({update.price})"
    tx_hash = client.update_price(feeder, source_id, update)
    return "submit", f"{source_id}: ECB {rate.day} EUR/USD {rate.usd_per_eur} -> {update.price} (tx {tx_hash})"


def cli() -> None:
    try:
        action, msg = run_once(dict(os.environ))
    except GateRejected as e:
        print(f"REFUSED: {e}", file=sys.stderr)
        sys.exit(2)
    except Exception as e:  # noqa: BLE001 - report and exit non-zero
        print(f"ERROR: {type(e).__name__}: {e}", file=sys.stderr)
        sys.exit(1)
    print(f"{action.upper()}: {msg}")


if __name__ == "__main__":
    cli()
