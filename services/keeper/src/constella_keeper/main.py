"""
Keeper loop. Per vault, per tick:
  1. skip unless this key is the vault's keeper and the vault is not paused;
  2. plan (pure) from live state; nothing to do while every sleeve is in band;
  3. pre-validate with the policy contract (read) - its verdict is logged;
  4. submit `rebalance`: simulated first (the vault re-checks its actual
     holdings), nothing is sent if the simulation fails;
  5. settle queued frozen-sleeve claims (`settle_claim` is permissionless
     and pays only the holder); a claim that cannot settle yet is retried.
Config (env): CONSTELLA_RPC_URL, CONSTELLA_NETWORK_PASSPHRASE, KEEPER_SECRET,
KEEPER_VAULTS (comma-separated), KEEPER_INTERVAL_SECONDS (300), KEEPER_ONCE,
KEEPER_DRY_RUN, KEEPER_STATE_FILE, KEEPER_DEADLINE_SECONDS (300).
"""
from __future__ import annotations

import json
import os
import sys
import time
from typing import Dict

from stellar_sdk import scval

from constella_keeper.chain import Chain, ChainError, proposal_scval
from constella_keeper.planner import PlanError, plan


def log(**kw) -> None:
    print(json.dumps(kw, default=str), flush=True)


def load_state(path: str) -> Dict:
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_state(path: str, state: Dict) -> None:
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(state, f)
    os.replace(tmp, path)


def rebalance_vault(chain: Chain, vault: str, dry_run: bool, deadline_s: int) -> Dict:
    st = chain.vault_state(vault)
    if st["keeper"] != chain.keeper:
        return {"action": "skip", "reason": "this key is not the vault's keeper"}
    if st["paused"]:
        return {"action": "skip", "reason": "vault paused"}
    if st["policy_hash"] is None:
        return {"action": "skip", "reason": "no policy bound"}
    router = st["router"]

    def quote(a_in, a_out, amt):
        if not router:
            raise PlanError("no Soroswap router set on the vault")
        return chain.soroswap_quote(router, a_in, a_out, amt)

    try:
        p = plan(st["sleeves"], st["scale"], st["band_bps"], st["max_slippage_bps"], quote)
    except PlanError as e:
        return {"action": "refused", "reason": str(e)}
    if p is None:
        return {"action": "none", "reason": "all sleeves within their bands"}
    deadline = chain.ledger_time() + deadline_s
    prop = proposal_scval(vault, st["policy_hash"], p, deadline, chain.keeper)
    verdict = chain.validate(st["policy"], vault, prop)
    if not verdict["is_valid"]:
        return {"action": "refused", "reason": f"policy rejected: code {verdict['code']} {verdict['message']}"}
    if dry_run:
        return {"action": "dry-run", "drift_bps": p.drift_bps, "operations": len(p.operations)}
    try:
        res = chain.submit(vault, "rebalance", prop)
    except ChainError as e:
        return {"action": "failed", "reason": str(e)}
    return {"action": "rebalanced", "drift_bps": p.drift_bps, "operations": len(p.operations), **res,
            "weights_after": p.weights_after}


def settle_claims(chain: Chain, vault: str, state: Dict, dry_run: bool) -> list:
    vs = state.setdefault(vault, {"cursor": None, "claims": []})
    # First run: from the oldest ledger the RPC still retains (a fixed offset
    # fell outside the retention range on the first local-network run). After
    # that, the saved cursor. Claims older than the RPC's retention must be
    # added to the state file by the operator (the keeper must run at least
    # once per retention window).
    start = None if vs["cursor"] else chain.oldest_ledger()
    pairs, cursor = chain.claim_events(vault, start, vs["cursor"])
    known = {tuple(c) for c in vs["claims"]}
    for p in pairs:
        if p not in known:
            vs["claims"].append(list(p))
            known.add(p)
    if cursor:
        vs["cursor"] = cursor
    out, still = [], []
    for holder, asset in vs["claims"]:
        owed = int(chain.read(vault, "get_claim", scval.to_address(holder), scval.to_address(asset)))
        if owed <= 0:
            continue
        if dry_run:
            still.append([holder, asset]); out.append({"claim": holder, "asset": asset, "action": "dry-run"}); continue
        try:
            res = chain.submit(vault, "settle_claim", scval.to_address(holder), scval.to_address(asset))
            out.append({"claim": holder, "asset": asset, "action": "settled", **res})
        except ChainError as e:  # still unpriced or not enough cash yet: retry next tick
            still.append([holder, asset])
            out.append({"claim": holder, "asset": asset, "action": "pending", "reason": str(e)[:200]})
    vs["claims"] = still
    return out


def tick(chain: Chain, vaults, state: Dict, dry_run: bool, deadline_s: int) -> None:
    for v in vaults:
        try:
            log(vault=v, **rebalance_vault(chain, v, dry_run, deadline_s))
        except Exception as e:  # one vault's failure never stops the others
            log(vault=v, action="error", reason=f"{type(e).__name__}: {str(e)[:300]}")
        try:
            for c in settle_claims(chain, v, state, dry_run):
                log(vault=v, **c)
        except Exception as e:
            log(vault=v, action="claims-error", reason=f"{type(e).__name__}: {str(e)[:300]}")


def cli() -> None:
    env = os.environ
    for k in ("CONSTELLA_RPC_URL", "CONSTELLA_NETWORK_PASSPHRASE", "KEEPER_SECRET", "KEEPER_VAULTS"):
        if not env.get(k):
            sys.exit(f"missing {k}")
    chain = Chain(env["CONSTELLA_RPC_URL"], env["CONSTELLA_NETWORK_PASSPHRASE"], env["KEEPER_SECRET"])
    vaults = [v.strip() for v in env["KEEPER_VAULTS"].split(",") if v.strip()]
    state_file = env.get("KEEPER_STATE_FILE", "keeper_state.json")
    dry = env.get("KEEPER_DRY_RUN") == "1"
    deadline_s = int(env.get("KEEPER_DEADLINE_SECONDS", "300"))
    interval = int(env.get("KEEPER_INTERVAL_SECONDS", "300"))
    log(action="start", keeper=chain.keeper, vaults=vaults, dry_run=dry)
    while True:
        state = load_state(state_file)
        tick(chain, vaults, state, dry, deadline_s)
        save_state(state_file, state)
        if env.get("KEEPER_ONCE") == "1":
            return
        time.sleep(interval)
