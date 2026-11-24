#!/usr/bin/env python3
"""Push ~/.config/constella/testnet/services.env to the Railway service.

Merges (variableCollectionUpsert): variables set only on Railway (e.g.
LLM_ENABLED, OPENROUTER_API_KEY, LLM_RUN_BUDGET_USD) are kept. Secrets are
read from local files and never printed. Railway redeploys the service.

Inputs: ~/.config/constella/railway.env (RAILWAY_API_TOKEN),
~/.config/constella/testnet/railway_ids.json ({project, environment, service}).
"""
import json
import os
import urllib.request
from pathlib import Path


def read_env(path: str) -> dict:
    out = {}
    for line in Path(os.path.expanduser(path)).read_text().splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            k, v = line.split("=", 1)
            out[k.strip()] = v.strip().strip('"')
    return out


def main() -> None:
    token = read_env("~/.config/constella/railway.env")["RAILWAY_API_TOKEN"]
    ids = json.loads(Path(os.path.expanduser("~/.config/constella/testnet/railway_ids.json")).read_text())
    variables = read_env("~/.config/constella/testnet/services.env")
    body = {"query": "mutation($i:VariableCollectionUpsertInput!){variableCollectionUpsert(input:$i)}",
            "variables": {"i": {"projectId": ids["project"], "environmentId": ids["environment"],
                                "serviceId": ids["service"], "variables": variables}}}
    req = urllib.request.Request("https://backboard.railway.com/graphql/v2", json.dumps(body).encode(),
                                 {"Content-Type": "application/json", "Authorization": "Bearer " + token,
                                  "User-Agent": "constella-deploy"})
    with urllib.request.urlopen(req, timeout=60) as r:
        res = json.load(r)
    if res.get("errors"):
        raise SystemExit("railway: " + "; ".join(e.get("message", "?") for e in res["errors"]))
    print(f"pushed {len(variables)} variables (names only): {', '.join(sorted(variables))}")


if __name__ == "__main__":
    main()
